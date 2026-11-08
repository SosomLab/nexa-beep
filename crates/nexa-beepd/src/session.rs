//! 연결 세션 — 프리앰블 → Noise(prologue) → Hello/Welcome → 등록·루프 → 정리. 상태기계 Pre(핸드셰이크) / Ready / Closing.
//!
//! 한 연결 = 한 스레드가 세션을 단독 소유한다(세션 액터). 남이 보낼 것은 `ConnHandle.tx` 큐로 들어와 여기서 드레인된다.
//! 모르는 **핵심** kind는 `Error{UnknownKind, ref_kind}`로 응답하고(조용히 버리지 않는다) 확장 kind는 무시한다.

use crate::{
    metrics, roster, router, udp_obs, ConnHandle, ConnId, Shared, HANDSHAKE_TIMEOUT, SERVER_CAPS,
};
use nbeep_core::session::{Session, SessionError};
use nbeep_core::PeerId;
use nbeep_crypto::{Identity, NoiseSession};
use nbeep_net::TcpLink;
use nbeep_relay_proto::{
    app, preamble, BindMode, Channel, DecodeError, Envelope, ErrorCode, Hello, Kind, Presence,
    Problem, Register, RegisterResult, ServerType, Welcome, PROLOGUE, PROTO_MAJOR, PROTO_MINOR,
};
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 토큰 버킷 — 연결당 릴레이 예산. 부족하면 **자면서 기다린다**(이 스레드가 곧 그 연결의
/// 소비자라, 기다림 = TCP 배압으로 발신자를 페이싱한다. 드롭은 세션을 죽이므로 안 쓴다).
#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
    rate: f64,
    cap: f64,
}

impl Bucket {
    fn new(rate_bps: u64) -> Self {
        let rate = rate_bps as f64;
        Self {
            tokens: rate.max(1.0),
            last: Instant::now(),
            rate,
            cap: (rate * 2.0).max(1.0),
        }
    }

    fn refill(&mut self) {
        let dt = self.last.elapsed().as_secs_f64();
        self.last = Instant::now();
        self.tokens = (self.tokens + dt * self.rate).min(self.cap);
    }

    /// `n`바이트만큼 소비(부족하면 대기 · stop 시 중단).
    fn take(&mut self, n: usize, stop: &AtomicBool) {
        if self.rate <= 0.0 {
            return; // 0 = 무제한
        }
        loop {
            self.refill();
            if self.tokens >= n as f64 {
                self.tokens -= n as f64;
                return;
            }
            if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn problem(kind: Kind, code: ErrorCode, reason: &str, ref_kind: Option<u16>) -> Vec<u8> {
    let mut p = Problem::new(code, reason);
    p.ref_kind = ref_kind;
    p.encode(kind).encode()
}

/// 세션 id — 대조용 16B(conn 번호 ‖ 시각 나노 ‖ 0). 비밀이 아니다.
fn session_id(conn_id: ConnId) -> [u8; 16] {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&conn_id.to_be_bytes());
    id[8..12].copy_from_slice(&nanos.to_be_bytes());
    id
}

/// 프리앰블 → Noise → Hello → (Welcome | Reject). `None` = 끝났다(프로브·거절·실패 — 필요한 응답은 이미 보냈다).
fn handshake(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    shared: &Arc<Shared>,
    id: &Identity,
    conn_id: ConnId,
) -> Option<(NoiseSession<TcpLink>, Hello)> {
    let m = &shared.metrics;
    // ① 프리앰블(평문 8B) — 세대 판별. v1 클라([len=0x20][e])·비프로토콜은 여기서 갈린다(★ 이중 수용 자리).
    let mut pre = [0u8; preamble::LEN];
    if stream.read_exact(&mut pre).is_err() {
        return None;
    }
    match preamble::parse(&pre) {
        Some(major) if major == PROTO_MAJOR => {
            if stream.write_all(&preamble::encode(PROTO_MAJOR)).is_err() {
                return None;
            }
        }
        Some(major) => {
            // 다른 세대 — 내 major만 알리고 끊는다(클라가 "앱 업데이트 필요"를 표시한다).
            let _ = stream.write_all(&preamble::encode(PROTO_MAJOR));
            m.inc(&m.rejected_version);
            shared.log(&format!(
                "conn#{conn_id} {peer_addr} 세대 불일치(major {major}) — 거절"
            ));
            return None;
        }
        None => {
            m.inc(&m.rejected_v1);
            shared.log(&format!(
                "conn#{conn_id} {peer_addr} 프리앰블 없음(v1·비프로토콜) — 거절"
            ));
            return None;
        }
    }
    // ② Noise_XX + prologue(major 결속).
    let link = match TcpLink::new(stream) {
        Ok(l) => l,
        Err(_) => return None,
    };
    let mut session = match NoiseSession::accept_with(link, id, PROLOGUE) {
        Ok(s) => s,
        Err(_) => {
            m.inc(&m.handshake_failed);
            return None; // 키·prologue 실패 — 조용히 끊는다(상대는 핸드셰이크 실패를 본다)
        }
    };
    // ③ Hello(첫 봉투 · 10초 안).
    let frame = match session.recv() {
        Ok(f) => f,
        Err(_) => return None,
    };
    let env = match Envelope::decode(&frame) {
        Ok(e) => e,
        Err(e) => {
            let _ = session.send(&problem(
                Kind::Error,
                e.error_code(),
                "expected Hello",
                None,
            ));
            m.inc(&m.errors_sent);
            return None;
        }
    };
    let hello = match env.kind() {
        Some(Kind::Hello) => match Hello::decode(&env) {
            Ok(h) => h,
            Err(_) => {
                let _ = session.send(&problem(
                    Kind::Error,
                    ErrorCode::Malformed,
                    "bad Hello",
                    Some(env.kind),
                ));
                m.inc(&m.errors_sent);
                return None;
            }
        },
        _ => {
            // Hello 전 어떤 요청도 NotRegistered.
            let _ = session.send(&problem(
                Kind::Error,
                ErrorCode::NotRegistered,
                "Hello first",
                Some(env.kind),
            ));
            m.inc(&m.errors_sent);
            return None;
        }
    };
    if hello.proto_major != PROTO_MAJOR {
        let mut p = Problem::new(ErrorCode::VersionMismatch, "proto major");
        p.min_proto = Some(PROTO_MAJOR);
        let _ = session.send(&p.encode(Kind::Reject).encode());
        m.inc(&m.rejected_hello);
        return None;
    }
    if hello.app.is_empty()
        || hello.app.len() > 32
        || !hello.app.bytes().all(|b| b.is_ascii_graphic())
    {
        let _ = session.send(
            &Problem::new(ErrorCode::Forbidden, "app")
                .encode(Kind::Reject)
                .encode(),
        );
        m.inc(&m.rejected_hello);
        return None;
    }
    // ④ Welcome.
    let welcome = Welcome {
        proto_major: PROTO_MAJOR,
        proto_minor: PROTO_MINOR,
        server_version: SERVER_VERSION.to_string(),
        server_type: ServerType::Relay,
        caps: hello.caps & SERVER_CAPS,
        limits: shared.limits,
        session_id: session_id(conn_id),
        udp_port: shared.udp_port,
        observed: udp_obs::sock_to_ep(peer_addr),
    };
    if session.send(&welcome.encode().encode()).is_err() {
        return None;
    }
    if hello.app == app::PROBE {
        // 헬스 프로브 — Welcome만 받고 끝(등록·명부 없음).
        m.inc(&m.probes);
        shared.log(&format!("conn#{conn_id} probe"));
        return None;
    }
    Some((session, hello))
}

/// 연결 스레드 본체.
#[allow(clippy::too_many_lines)] // 연결 프로토콜 분기 전체가 한 루프다(상태 공유 최소화)
pub(crate) fn run_conn(
    stream: TcpStream,
    peer_addr: SocketAddr,
    shared: &Arc<Shared>,
    id: &Identity,
) {
    let m: &metrics::Metrics = &shared.metrics;
    m.inc(&m.accepted);
    // ⚠ 논블로킹 리스너에서 accept된 스트림은 (Windows 등에서) 논블로킹을 상속한다 —
    // 복원하지 않으면 핸드셰이크 첫 read가 즉시 WouldBlock으로 죽는다(실측).
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    // 핸드셰이크 타임아웃 — 침묵 연결이 스레드를 영구 점유하지 못하게.
    let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
    let conn_id = shared.next_conn.fetch_add(1, Ordering::SeqCst);
    let Some((mut session, hello)) = handshake(stream, peer_addr, shared, id, conn_id) else {
        return;
    };
    // 상호 인증(Noise_XX)이라 클라 공개키는 여기서 이미 보인다 — 그러나 **담는 것은
    // Announce를 켠 경우뿐**(§2-3 비기록 원칙 · roster는 옵트인 예외 — §12-7).
    let client_peer: PeerId = session.peer();
    let caps = hello.caps & SERVER_CAPS;
    let app_name = hello.app;
    session.set_recv_timeout(Some(Duration::from_millis(15)));

    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = std::sync::mpsc::channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let kill = Arc::new(AtomicBool::new(false));
    {
        let Ok(mut conns) = shared.conns.lock() else {
            return;
        };
        conns.insert(
            conn_id,
            ConnHandle {
                tx,
                queued: Arc::clone(&queued),
                kill: Arc::clone(&kill),
                rids: Vec::new(),
                udp_obs: None,
            },
        );
    }
    m.inc(&m.sessions_total);
    m.inc(&m.sessions_live);
    shared.log(&format!(
        "conn#{conn_id} 접속 {peer_addr} app={app_name} v{} caps={caps:#x}",
        hello.app_version
    ));

    let mut bucket = Bucket::new(shared.limits.rate_bps);
    let mut registered = false;
    let started = Instant::now();
    let mut goaway_sent = false;

    'conn: loop {
        if shared.stop.load(Ordering::SeqCst) {
            // 우아한 종료 — 재접속 힌트와 함께 알린다.
            let mut p = Problem::new(ErrorCode::Shutdown, "server shutdown");
            p.retry_after_s = Some(5);
            let _ = session.send(&p.encode(Kind::GoawayS).encode());
            goaway_sent = true;
            break;
        }
        if kill.load(Ordering::SeqCst) {
            let _ = session.send(&problem(
                Kind::GoawayS,
                ErrorCode::LimitExceeded,
                "slow consumer",
                None,
            ));
            goaway_sent = true;
            break;
        }
        // 미등록 유휴 정리 — 등록 없이 자리만 차지하는 연결.
        if !registered && started.elapsed() > HANDSHAKE_TIMEOUT {
            let _ = session.send(&problem(
                Kind::GoawayS,
                ErrorCode::NotRegistered,
                "register timeout",
                None,
            ));
            goaway_sent = true;
            break;
        }
        // 1) 내 큐 드레인(남이 넣은 중계·통지).
        while let Ok(frame) = rx.try_recv() {
            let cost = frame.len() + 16;
            queued.fetch_sub(cost.min(queued.load(Ordering::SeqCst)), Ordering::SeqCst);
            if session.send(&frame).is_err() {
                break 'conn;
            }
        }
        // 2) 세션 수신.
        let frame = match session.recv() {
            Ok(f) => f,
            Err(SessionError::TimedOut) => continue,
            Err(_) => break,
        };
        let env = match Envelope::decode(&frame) {
            Ok(e) => e,
            Err(DecodeError::UnknownCoreKind(k)) => {
                // 모르는 핵심 kind = 응답(ADR-0016 — 조용히 버리지 않는다).
                if session
                    .send(&problem(
                        Kind::Error,
                        ErrorCode::UnknownKind,
                        "unknown kind",
                        Some(k),
                    ))
                    .is_err()
                {
                    break;
                }
                m.inc(&m.errors_sent);
                continue;
            }
            Err(DecodeError::UnknownExtKind(_)) => continue, // 확장 kind — 무시
            Err(e) => {
                // 깨진 봉투 — 알리고 끊는다(fail-closed).
                let _ = session.send(&problem(Kind::Error, e.error_code(), "malformed", None));
                m.inc(&m.errors_sent);
                break;
            }
        };
        let Some(kind) = env.kind() else { continue };
        // 등록 전 허용 = Register · Ping · Goaway · Announce(명부는 등록과 무관).
        if !registered && matches!(kind, Kind::Open | Kind::Accept | Kind::Data | Kind::CloseCh) {
            if session
                .send(&problem(
                    Kind::Error,
                    ErrorCode::NotRegistered,
                    "register first",
                    Some(env.kind),
                ))
                .is_err()
            {
                break;
            }
            m.inc(&m.errors_sent);
            continue;
        }
        match kind {
            Kind::Register => {
                let Ok(reg) = Register::decode(&env) else {
                    let _ = session.send(&problem(
                        Kind::Error,
                        ErrorCode::Malformed,
                        "bad Register",
                        Some(env.kind),
                    ));
                    m.inc(&m.errors_sent);
                    break;
                };
                let entries = if reg.bind == BindMode::Shared
                    && caps & nbeep_relay_proto::caps::SHARED_BIND == 0
                {
                    // 세션 능력에 없는 모드 — RID마다 Unsupported(등록되지 않음 · 클라 버그를 숨기지 않는다).
                    reg.rids
                        .iter()
                        .map(|r| (*r, ErrorCode::Unsupported as u16))
                        .collect()
                } else {
                    router::bind(shared, conn_id, &app_name, &reg.rids, reg.bind)
                };
                let ok = entries
                    .iter()
                    .any(|(_, st)| *st == nbeep_relay_proto::STATUS_OK);
                let udp_token = if ok {
                    udp_obs::issue_token(shared, conn_id)
                } else {
                    0
                };
                registered |= ok;
                shared.log(&format!(
                    "conn#{conn_id} 등록 rid={} 외 {}개 bind={:?} ok={ok}",
                    reg.rids.first().map(crate::rid_short).unwrap_or_default(),
                    reg.rids.len().saturating_sub(1),
                    reg.bind
                ));
                if session
                    .send(&RegisterResult { entries, udp_token }.encode().encode())
                    .is_err()
                {
                    break;
                }
            }
            Kind::Open => {
                let Ok(c) = Channel::decode(&env) else { break };
                let (Some(token), Some(dst)) = (c.token, c.rid) else {
                    let _ = session.send(&problem(
                        Kind::Error,
                        ErrorCode::Malformed,
                        "Open needs token+rid",
                        Some(env.kind),
                    ));
                    m.inc(&m.errors_sent);
                    continue;
                };
                if let Some(reply) = router::open(shared, conn_id, &app_name, token, dst) {
                    if session.send(&reply).is_err() {
                        break;
                    }
                }
            }
            Kind::Accept => {
                let Ok(c) = Channel::decode(&env) else { break };
                if let Some(ch) = c.ch {
                    router::accept(shared, conn_id, ch);
                }
            }
            Kind::Data => {
                let Ok(c) = Channel::decode(&env) else { break };
                let Some(ch) = c.ch else { continue };
                // ★ 양방향 성립(Accept) 전에는 한 바이트도 나르지 않는다(§2-6 증폭 방지).
                let Some(other) = router::other_party(shared, conn_id, ch) else {
                    continue; // 모르는·미성립 채널 데이터는 버린다
                };
                let n = c.bytes.len();
                bucket.take(n, &shared.stop); // 예산 — 기다림 = TCP 배압
                m.add(&m.bytes_relayed, n as u64);
                if !router::enqueue(shared, other, c.encode(Kind::DataS).encode()) {
                    // 느린 소비자로 채널 유지 불가 — 양쪽에 정직하게 닫힘 통지.
                    router::close_channel(shared, ch, None);
                }
            }
            Kind::CloseCh => {
                let Ok(c) = Channel::decode(&env) else { break };
                if let Some(ch) = c.ch {
                    router::close_by(shared, conn_id, ch);
                }
            }
            Kind::Ping => {
                let nonce = nbeep_relay_proto::nonce(&env).unwrap_or(0);
                if session
                    .send(&nbeep_relay_proto::ping(Kind::Pong, nonce).encode())
                    .is_err()
                {
                    break;
                }
            }
            Kind::Announce => {
                if caps & nbeep_relay_proto::caps::ROSTER == 0 {
                    let _ = session.send(&problem(
                        Kind::Error,
                        ErrorCode::Unsupported,
                        "no ROSTER cap",
                        Some(env.kind),
                    ));
                    m.inc(&m.errors_sent);
                    continue;
                }
                let Ok(p) = Presence::decode(&env) else { break };
                for f in roster::announce(shared, conn_id, &app_name, client_peer, p.listed, p.card)
                {
                    if session.send(&f).is_err() {
                        break 'conn;
                    }
                }
            }
            Kind::Goaway => break, // 클라 우아한 종료
            Kind::Hello => {
                let _ = session.send(&problem(
                    Kind::Error,
                    ErrorCode::Malformed,
                    "Hello twice",
                    Some(env.kind),
                ));
                m.inc(&m.errors_sent);
            }
            // S2C 방향 kind를 클라가 보냈다 — 알리고 버린다.
            _ => {
                let _ = session.send(&problem(
                    Kind::Error,
                    ErrorCode::Malformed,
                    "wrong direction",
                    Some(env.kind),
                ));
                m.inc(&m.errors_sent);
            }
        }
    }

    // 정리 — 내 RID·토큰·채널 회수, 상대에게 닫힘 통지, 명부 이탈.
    if goaway_sent {
        m.inc(&m.goaway_sent);
    }
    shared.log(&format!("conn#{conn_id} 종료"));
    router::unbind_all(shared, conn_id);
    udp_obs::forget(shared, conn_id);
    router::close_all_of(shared, conn_id);
    if let Ok(mut conns) = shared.conns.lock() {
        conns.remove(&conn_id);
    }
    roster::leave(shared, conn_id, client_peer);
    m.sessions_live.fetch_sub(1, Ordering::Relaxed);
}
