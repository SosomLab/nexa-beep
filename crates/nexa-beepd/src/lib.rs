//! `nexa-beepd` — 릴레이 서버 MVP(X-1 · [docs/32 §12-6]).
//!
//! **하는 일 네 가지가 전부다**(비목표 = 계정·오프라인 저장·그룹 팬아웃·웹 가입):
//! 1. **프레즌스** — 클라이언트가 붙어 회전 RID를 등록([docs/32 §2-3] — `PeerId` 원본 금지).
//! 2. **랑데부/관측** — TCP·UDP에서 밖에서 본 공인 엔드포인트를 알려 주고(STUN-lite),
//!    열기 요청 시 양쪽에 서로의 관측 엔드포인트를 전달한다(홀펀칭 재료 · X-UDP-c).
//! 3. **blind 릴레이** — 펀치가 안 되는 쌍을 위해 **양방향 성립(Accept) 후에만** 종단
//!    암호문을 중계한다(증폭 방지 [docs/32 §2-6] · 토큰 버킷).
//! 4. **서버 자기 신원** — 서버도 Noise 정적 키를 가진다(클라이언트가 TOFU 핀 — §2-4).
//!
//! **서버가 보는 것** = RID·채널 번호·바이트 수·시각. **저장하지 않는다** — 모드 ①은
//! 버퍼가 아니라 파이프다(양쪽 동시 접속일 때만 흐른다 · [docs/32 §2-6]).
//! 로그에도 같은 원칙(봉투만 — RID 앞 4바이트·건수·바이트).
//!
//! 동시성 모델은 클라이언트와 같은 **세션 액터** 문법이다 — 연결 하나 = 스레드 하나가
//! 세션을 단독 소유하고, 남이 보낼 것은 그 연결의 큐(mpsc)에 넣는다. 큐가 상한(8MiB)을
//! 넘는 느린 소비자는 **정직하게 끊는다**(무한 버퍼·쓰기 교착 둘 다 회피 — 08-16 교훈).
#![forbid(unsafe_code)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

use nbeep_core::session::{Session, SessionError};
use nbeep_core::PeerId;
use nbeep_crypto::{Identity, NoiseSession};
use nbeep_net::TcpLink;
use nbeep_relay::{C2s, Rid, S2c, MAX_RIDS, OBS_MAGIC};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 연결당 릴레이 기본 예산(바이트/초). 0 = 무제한.
pub const DEFAULT_RATE_BPS: u64 = 1024 * 1024;

/// 느린 소비자 판정 — 연결 송신 큐 상한(초과 = 절단).
const QUEUE_CAP: usize = 8 * 1024 * 1024;

/// 연결당 동시 채널 상한.
const MAX_CHANNELS_PER_CONN: usize = 64;

/// 핸드셰이크·미등록 유휴 상한.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 서버 구성.
#[derive(Clone, Debug)]
pub struct Config {
    /// 바인드 주소(기본 0.0.0.0).
    pub bind_ip: IpAddr,
    /// TCP 제어 + UDP 관측 포트(같은 번호). 0 = 임시(테스트 — TCP가 정한 번호로 UDP 재시도).
    pub port: u16,
    /// 서버 신원 키 파일 경로.
    pub key_path: PathBuf,
    /// 연결당 릴레이 예산(바이트/초 · 0 = 무제한).
    pub rate_bps: u64,
    /// 상세 로그.
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_ip: IpAddr::from([0u8, 0, 0, 0]),
            port: nbeep_relay::DEFAULT_RELAY_PORT,
            key_path: PathBuf::from("beepd.key"),
            rate_bps: DEFAULT_RATE_BPS,
            verbose: false,
        }
    }
}

/// 가동 중 서버 핸들 — 테스트·우아한 종료용.
#[derive(Debug)]
pub struct Handle {
    /// 실제 바인드된 TCP 주소.
    pub tcp_addr: SocketAddr,
    /// UDP 관측 포트.
    pub udp_port: u16,
    /// 서버 신원(클라이언트 핀 대상).
    pub server_peer: PeerId,
    shared: Arc<Shared>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Handle {
    /// 서버 정지(스레드 합류까지).
    pub fn shutdown(mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

type ConnId = u64;

#[derive(Debug)]
struct ConnHandle {
    tx: Sender<S2c>,
    /// 대기 중 송신 바이트(느린 소비자 판정) — 넣는 쪽 증가, 보내는 쪽 감소.
    queued: Arc<AtomicUsize>,
    /// 절단 지시(남의 스레드가 세운다 — 본인 루프가 확인).
    kill: Arc<AtomicBool>,
    /// 이 연결의 등록 RID들.
    rids: Vec<Rid>,
    /// UDP 관측 엔드포인트(프로브 도착 시 갱신).
    udp_obs: Option<SocketAddr>,
}

#[derive(Debug)]
struct Chan {
    a: ConnId,
    b: ConnId,
    /// 여는 쪽 대조 토큰(Accept 시 OpenResult에 되돌린다).
    token: u32,
    accepted: bool,
}

#[derive(Debug)]
struct Shared {
    stop: AtomicBool,
    rids: Mutex<HashMap<Rid, ConnId>>,
    conns: Mutex<HashMap<ConnId, ConnHandle>>,
    chans: Mutex<HashMap<u32, Chan>>,
    /// UDP 프로브 토큰 → 연결.
    tokens: Mutex<HashMap<u64, ConnId>>,
    next_ch: AtomicU32,
    next_conn: AtomicU64,
    udp_port: u16,
    rate_bps: u64,
    verbose: bool,
}

impl Shared {
    fn log(&self, msg: &str) {
        if self.verbose {
            eprintln!("[beepd] {msg}");
        }
    }
}

fn rid_short(rid: &Rid) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", rid[0], rid[1], rid[2], rid[3])
}

/// 서버 가동 — 수락·UDP 관측 스레드를 띄우고 즉시 돌아온다.
///
/// # Errors
/// 키 로드·바인드 실패 시 `io::Error`.
pub fn spawn(cfg: &Config) -> std::io::Result<Handle> {
    let (identity, created) = nbeep_crypto::keyfile::load_or_generate(&cfg.key_path)?;
    let server_peer = identity.peer_id();
    if created {
        eprintln!("[beepd] 새 서버 신원 생성 — {}", cfg.key_path.display());
    }

    // TCP·UDP를 같은 번호에 — port 0(테스트)이면 TCP가 정한 번호로 UDP를 몇 번 재시도.
    let (listener, udp) = {
        let mut last_err = None;
        let mut pair = None;
        for _ in 0..10 {
            let l = TcpListener::bind((cfg.bind_ip, cfg.port))?;
            let p = l.local_addr()?.port();
            match UdpSocket::bind((cfg.bind_ip, p)) {
                Ok(u) => {
                    pair = Some((l, u));
                    break;
                }
                Err(e) if cfg.port == 0 => last_err = Some(e), // 다른 임시 번호로 재시도
                Err(e) => return Err(e),
            }
        }
        pair.ok_or_else(|| {
            last_err.unwrap_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::AddrInUse, "TCP/UDP 동번호 확보 실패")
            })
        })?
    };
    let tcp_addr = listener.local_addr()?;
    let udp_port = udp.local_addr()?.port();

    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        rids: Mutex::new(HashMap::new()),
        conns: Mutex::new(HashMap::new()),
        chans: Mutex::new(HashMap::new()),
        tokens: Mutex::new(HashMap::new()),
        next_ch: AtomicU32::new(1),
        next_conn: AtomicU64::new(1),
        udp_port,
        rate_bps: cfg.rate_bps,
        verbose: cfg.verbose,
    });

    let mut threads = Vec::new();

    // 수락 루프 — 논블로킹 + 50ms 폴(정지 신호 확인 가능하게).
    listener.set_nonblocking(true)?;
    {
        let shared = Arc::clone(&shared);
        let id = Arc::new(identity);
        threads.push(
            std::thread::Builder::new()
                .name("beepd-accept".into())
                .spawn(move || accept_loop(&listener, &shared, &id))?,
        );
    }

    // UDP 관측 루프.
    udp.set_read_timeout(Some(Duration::from_millis(100)))?;
    {
        let shared = Arc::clone(&shared);
        threads.push(
            std::thread::Builder::new()
                .name("beepd-udp".into())
                .spawn(move || udp_loop(&udp, &shared))?,
        );
    }

    Ok(Handle {
        tcp_addr,
        udp_port,
        server_peer,
        shared,
        threads,
    })
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>, id: &Arc<Identity>) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                let shared = Arc::clone(shared);
                let id = Arc::clone(id);
                // 연결 스레드는 detach — 종료는 stop 플래그·소켓 타임아웃이 정리한다.
                let _ = std::thread::Builder::new()
                    .name("beepd-conn".into())
                    .spawn(move || conn_thread(stream, peer_addr, &shared, &id));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn udp_loop(udp: &UdpSocket, shared: &Arc<Shared>) {
    let mut buf = [0u8; 64];
    while !shared.stop.load(Ordering::SeqCst) {
        match udp.recv_from(&mut buf) {
            Ok((n, from)) => {
                if n < 12 || buf[..4] != OBS_MAGIC {
                    continue; // 미지 데이터그램은 조용히 버린다(응답 없음 — 증폭 방지)
                }
                let token = u64::from_be_bytes([
                    buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11],
                ]);
                let conn_id = {
                    shared
                        .tokens
                        .lock()
                        .ok()
                        .and_then(|t| t.get(&token).copied())
                };
                let Some(conn_id) = conn_id else {
                    continue; // 등록된 토큰만 응답(스캔에 침묵)
                };
                if let Ok(mut conns) = shared.conns.lock() {
                    if let Some(c) = conns.get_mut(&conn_id) {
                        c.udp_obs = Some(from);
                    }
                }
                // 에코: [magic][token][관측 엔드포인트] — 클라가 자기 매핑을 안다.
                let mut echo = Vec::with_capacity(32);
                echo.extend_from_slice(&OBS_MAGIC);
                echo.extend_from_slice(&token.to_be_bytes());
                match from {
                    SocketAddr::V4(a) => {
                        echo.push(4);
                        echo.extend_from_slice(&a.ip().octets());
                        echo.extend_from_slice(&a.port().to_be_bytes());
                    }
                    SocketAddr::V6(a) => {
                        echo.push(6);
                        echo.extend_from_slice(&a.ip().octets());
                        echo.extend_from_slice(&a.port().to_be_bytes());
                    }
                }
                let _ = udp.send_to(&echo, from);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            // Windows: ICMP Port Unreachable가 ConnReset으로 올라온다 — 소켓은 살아 있다.
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

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

#[allow(clippy::too_many_lines)] // 연결 프로토콜 분기 전체가 한 루프다(상태 공유 최소화)
fn conn_thread(stream: TcpStream, peer_addr: SocketAddr, shared: &Arc<Shared>, id: &Identity) {
    // ⚠ 논블로킹 리스너에서 accept된 스트림은 (Windows 등에서) 논블로킹을 상속한다 —
    // 복원하지 않으면 핸드셰이크 첫 read가 즉시 WouldBlock으로 죽는다(실측).
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    // 핸드셰이크 타임아웃 — 침묵 연결이 스레드를 영구 점유하지 못하게.
    let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
    let link = match TcpLink::new(stream) {
        Ok(l) => l,
        Err(_) => return,
    };
    let mut session = match NoiseSession::accept(link, id) {
        Ok(s) => s,
        Err(_) => return, // 핸드셰이크 실패·비프로토콜 접속 — 조용히 끊는다
    };
    session.set_recv_timeout(Some(Duration::from_millis(15)));

    let conn_id = shared.next_conn.fetch_add(1, Ordering::SeqCst);
    let (tx, rx): (Sender<S2c>, Receiver<S2c>) = std::sync::mpsc::channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let kill = Arc::new(AtomicBool::new(false));
    {
        let mut conns = match shared.conns.lock() {
            Ok(c) => c,
            Err(_) => return,
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
    shared.log(&format!("conn#{conn_id} 접속 {peer_addr}"));

    let mut bucket = Bucket::new(shared.rate_bps);
    let mut registered = false;
    let started = Instant::now();

    loop {
        if shared.stop.load(Ordering::SeqCst) || kill.load(Ordering::SeqCst) {
            break;
        }
        // 미등록 유휴 정리 — 등록 없이 자리만 차지하는 연결.
        if !registered && started.elapsed() > HANDSHAKE_TIMEOUT {
            break;
        }
        // 1) 내 큐 드레인(남이 넣은 중계·통지).
        let mut dead = false;
        while let Ok(msg) = rx.try_recv() {
            let cost = match &msg {
                S2c::Data { bytes, .. } => bytes.len() + 16,
                _ => 16,
            };
            queued.fetch_sub(cost.min(queued.load(Ordering::SeqCst)), Ordering::SeqCst);
            if session.send(&msg.encode()).is_err() {
                dead = true;
                break;
            }
        }
        if dead {
            break;
        }
        // 2) 세션 수신.
        let frame = match session.recv() {
            Ok(f) => f,
            Err(SessionError::TimedOut) => continue,
            Err(_) => break,
        };
        let Some(msg) = C2s::decode(&frame) else {
            continue; // 미지 kind — 전방 호환(조용히 버린다)
        };
        match msg {
            C2s::Register { rids } => {
                if rids.len() > MAX_RIDS {
                    break; // 인코더가 못 만드는 형태 = 프로토콜 위반
                }
                let udp_token = (conn_id << 32)
                    ^ u64::from(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.subsec_nanos()),
                    );
                {
                    let mut map = match shared.rids.lock() {
                        Ok(m) => m,
                        Err(_) => break,
                    };
                    // 최신 등록 우선(재접속 잔재 교체) — 죽은 연결의 RID가 자리를 막지 않게.
                    for r in &rids {
                        map.insert(*r, conn_id);
                    }
                }
                if let Ok(mut conns) = shared.conns.lock() {
                    if let Some(c) = conns.get_mut(&conn_id) {
                        c.rids = rids.clone();
                    }
                }
                if let Ok(mut tokens) = shared.tokens.lock() {
                    tokens.insert(udp_token, conn_id);
                }
                registered = true;
                shared.log(&format!(
                    "conn#{conn_id} 등록 rid={} 외 {}개",
                    rids.first().map(rid_short).unwrap_or_default(),
                    rids.len().saturating_sub(1)
                ));
                let ok = S2c::RegisterOk {
                    udp_token,
                    udp_port: shared.udp_port,
                    observed: Some(peer_addr),
                };
                if session.send(&ok.encode()).is_err() {
                    break;
                }
            }
            C2s::Open { token, dst } => {
                let dst_conn = shared.rids.lock().ok().and_then(|m| m.get(&dst).copied());
                let my_chan_count = shared
                    .chans
                    .lock()
                    .map(|c| {
                        c.values()
                            .filter(|ch| ch.a == conn_id || ch.b == conn_id)
                            .count()
                    })
                    .unwrap_or(usize::MAX);
                let status = match dst_conn {
                    None => 1u8,
                    Some(d) if d == conn_id => 1, // 자기 자신 — 랑데부 무의미
                    _ if my_chan_count >= MAX_CHANNELS_PER_CONN => 2,
                    Some(_) => 0,
                };
                if status != 0 {
                    let _ = session.send(
                        &S2c::OpenResult {
                            token,
                            status,
                            ch: 0,
                            peer_udp: None,
                        }
                        .encode(),
                    );
                    continue;
                }
                let dst_conn = dst_conn.expect("status 0 = 존재 확인됨");
                let ch = shared.next_ch.fetch_add(1, Ordering::SeqCst);
                let my_udp = shared
                    .conns
                    .lock()
                    .ok()
                    .and_then(|c| c.get(&conn_id).and_then(|h| h.udp_obs));
                let src_rid = shared
                    .conns
                    .lock()
                    .ok()
                    .and_then(|c| c.get(&conn_id).and_then(|h| h.rids.first().copied()))
                    .unwrap_or_default();
                if let Ok(mut chans) = shared.chans.lock() {
                    chans.insert(
                        ch,
                        Chan {
                            a: conn_id,
                            b: dst_conn,
                            token,
                            accepted: false,
                        },
                    );
                }
                let delivered = enqueue(
                    shared,
                    dst_conn,
                    S2c::Incoming {
                        ch,
                        src: src_rid,
                        peer_udp: my_udp,
                    },
                );
                if !delivered {
                    if let Ok(mut chans) = shared.chans.lock() {
                        chans.remove(&ch);
                    }
                    let _ = session.send(
                        &S2c::OpenResult {
                            token,
                            status: 2,
                            ch: 0,
                            peer_udp: None,
                        }
                        .encode(),
                    );
                }
                shared.log(&format!("conn#{conn_id} open ch#{ch} → conn#{dst_conn}"));
            }
            C2s::Accept { ch } => {
                let opener = {
                    let mut chans = match shared.chans.lock() {
                        Ok(c) => c,
                        Err(_) => break,
                    };
                    match chans.get_mut(&ch) {
                        Some(c) if c.b == conn_id && !c.accepted => {
                            c.accepted = true;
                            Some((c.a, c.token))
                        }
                        _ => None,
                    }
                };
                if let Some((opener, token)) = opener {
                    let my_udp = shared
                        .conns
                        .lock()
                        .ok()
                        .and_then(|c| c.get(&conn_id).and_then(|h| h.udp_obs));
                    let _ = enqueue(
                        shared,
                        opener,
                        S2c::OpenResult {
                            token,
                            status: 0,
                            ch,
                            peer_udp: my_udp,
                        },
                    );
                    shared.log(&format!("ch#{ch} 성립(양방향)"));
                }
            }
            C2s::Data { ch, fin, bytes } => {
                // ★ 양방향 성립(Accept) 전에는 한 바이트도 나르지 않는다(§2-6 증폭 방지).
                let other = {
                    let chans = match shared.chans.lock() {
                        Ok(c) => c,
                        Err(_) => break,
                    };
                    match chans.get(&ch) {
                        Some(c) if c.accepted && c.a == conn_id => Some(c.b),
                        Some(c) if c.accepted && c.b == conn_id => Some(c.a),
                        _ => None,
                    }
                };
                let Some(other) = other else {
                    continue; // 모르는·미성립 채널 데이터는 버린다
                };
                bucket.take(bytes.len(), &shared.stop); // 예산 — 기다림 = TCP 배압
                if !enqueue(shared, other, S2c::Data { ch, fin, bytes }) {
                    // 느린 소비자로 채널 유지 불가 — 양쪽에 정직하게 닫힘 통지.
                    close_channel(shared, ch, None);
                    let _ = session.send(&S2c::ChClosed { ch }.encode());
                }
            }
            C2s::CloseCh { ch } => {
                // 수락 전에 받는 쪽이 닫았다(지연 수락 drop) — 여는 쪽이 open 타임아웃까지
                // 기다리지 않게 **열기 거절(2)** 로 즉시 알린다.
                let refused = shared.chans.lock().ok().and_then(|c| {
                    c.get(&ch).and_then(|chan| {
                        (!chan.accepted && chan.b == conn_id).then_some((chan.a, chan.token))
                    })
                });
                if let Some((opener, token)) = refused {
                    let _ = enqueue(
                        shared,
                        opener,
                        S2c::OpenResult {
                            token,
                            status: 2,
                            ch: 0,
                            peer_udp: None,
                        },
                    );
                }
                close_channel(shared, ch, Some(conn_id));
            }
            C2s::Ping => {
                if session.send(&S2c::Pong.encode()).is_err() {
                    break;
                }
            }
        }
    }

    // 정리 — 내 RID·토큰·채널 회수, 상대에게 닫힘 통지.
    shared.log(&format!("conn#{conn_id} 종료"));
    if let Ok(mut map) = shared.rids.lock() {
        map.retain(|_, v| *v != conn_id);
    }
    if let Ok(mut tokens) = shared.tokens.lock() {
        tokens.retain(|_, v| *v != conn_id);
    }
    let my_chans: Vec<u32> = shared
        .chans
        .lock()
        .map(|c| {
            c.iter()
                .filter(|(_, ch)| ch.a == conn_id || ch.b == conn_id)
                .map(|(k, _)| *k)
                .collect()
        })
        .unwrap_or_default();
    for ch in my_chans {
        close_channel(shared, ch, Some(conn_id));
    }
    if let Ok(mut conns) = shared.conns.lock() {
        conns.remove(&conn_id);
    }
}

/// 다른 연결의 큐에 넣는다. 상한 초과(느린 소비자)면 그 연결을 절단 지시하고 `false`.
fn enqueue(shared: &Arc<Shared>, conn_id: ConnId, msg: S2c) -> bool {
    let conns = match shared.conns.lock() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let Some(handle) = conns.get(&conn_id) else {
        return false;
    };
    let cost = match &msg {
        S2c::Data { bytes, .. } => bytes.len() + 16,
        _ => 16,
    };
    if handle.queued.load(Ordering::SeqCst) + cost > QUEUE_CAP {
        // 무한 버퍼 대신 정직한 절단(fail-closed) — 쓰기 교착·메모리 폭주 둘 다 회피.
        handle.kill.store(true, Ordering::SeqCst);
        return false;
    }
    handle.queued.fetch_add(cost, Ordering::SeqCst);
    handle.tx.send(msg).is_ok()
}

/// 채널 제거 + 남은 쪽(들)에 닫힘 통지. `except` = 통지 생략 대상(닫은 본인).
fn close_channel(shared: &Arc<Shared>, ch: u32, except: Option<ConnId>) {
    let removed = shared.chans.lock().ok().and_then(|mut c| c.remove(&ch));
    if let Some(chan) = removed {
        for side in [chan.a, chan.b] {
            if Some(side) != except {
                let _ = enqueue(shared, side, S2c::ChClosed { ch });
            }
        }
    }
}
