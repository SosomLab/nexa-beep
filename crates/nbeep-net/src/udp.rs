//! S2 — IPv4 멀티캐스트 발견(**첫 실물 소켓** · M1-4 슬라이스 1 · [docs/06 §4]).
//!
//! [`wire`](crate::wire) 패킷을 **S2(IPv4 멀티캐스트) + S3(IPv4 브로드캐스트) 동시**로 쏘고 받는다.
//! 멀티캐스트를 막는 기업 Wi-Fi에서도 브로드캐스트로 발견되게 하는 폴백([docs/06 §4]). 수신
//! 소켓은 `0.0.0.0:PORT` 바인딩이라 **둘 다 같은 소켓으로 받는다**(같은 peer 이중 관측은 PeerTable이
//! 병합 — FR-D-6). **S1(IPv6 멀티캐스트)** 도 best-effort 병행(IPv6 미지원 환경은 조용히 IPv4만).
//! S4(유니캐스트 · M1-8)는 ARP 이웃 프로브(HELLO) + 수신측 유니캐스트 Announce 응답 + **송신 소켓 응답 수신**(11-07).
//!
//! - **자기 패킷 필터** — 멀티캐스트는 루프백된다. **키(`PeerId`)로 거른다**([docs/08 §5] —
//!   주소·포트가 아니라 신원 기준. 같은 호스트의 다른 인스턴스는 걸러지지 않아야 한다).
//! - **`SO_REUSEADDR`(+unix `SO_REUSEPORT`)** — 같은 호스트에서 여러 인스턴스가 같은 발견
//!   포트에 바인딩(개발·테스트 필수, socket2 — std 미노출 옵션).
//! - **타이밍은 전부 주입**(announce 주기) 또는 **잠정 상수**(그룹·포트·TTL — ⚠️ D-8b 실측 후
//!   확정, [docs/08 §8]). 잠정치는 상수 주석에 명시한다.
//! - 종료 시 **GOODBYE 2회**(FR-D-8 명시적 이탈 — 유실 대비 중복. 수신 측 중복은 무해).

use crate::wire::{Decoded, Packet, PacketKind};
use nbeep_core::{DisplayName, PeerId};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// 발견 멀티캐스트 그룹(잠정 — D-8b 실측 후 확정). 239.255/16 = 조직 로컬 범위.
pub const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 77, 77);
/// 발견 포트(잠정 — D-8b).
pub const DISCOVERY_PORT: u16 = 47_100;
/// S3 브로드캐스트 목적지(제한 브로드캐스트 — 서브넷 무관 · 라우터는 넘지 않음).
const BROADCAST: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 255);
/// S1 IPv6 멀티캐스트 그룹(링크로컬 ff02::/16 — 관리자 지정 잠정, D-8b 확정).
const MULTICAST_GROUP_V6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x0beb);
/// 멀티캐스트 TTL(잠정 — 링크 로컬 1홉. 라우팅 확장은 S6/릴레이 몫).
const TTL: u32 = 1;
/// 수신 폴링 간격(정지 플래그 확인용).
const RECV_POLL: Duration = Duration::from_millis(300);
/// S4(M1-8) 라운드 주기 — 광고 몇 주기마다 이웃 유니캐스트 프로브를 도는가
/// (기본 광고 800ms면 ≈12.8초 · macOS는 라운드마다 `arp -an` 스폰이라 성기게).
const S4_EVERY: u32 = 16;
/// S4 라운드당 프로브 상한(큐 상한 필수 규칙 — NFR-B-6 결).
const S4_MAX: usize = 64;
/// ★ 적응형 S4(R-22 ③ · 11-07 · 사용자 결정 11-06) — S1~S3(멀티캐스트/브로드캐스트)가 **살아 있다고 확인되면**
/// S4 라운드를 이 주기로 성기게 한다(16→64 · 800ms 기준 ≈51초). 근거: IDS는 관측 창 안의 고유 목적지 수를 세므로
/// 버스트 분산이 아니라 **라운드 수 축소**만 판정을 바꾼다(journal 09-10). 혼합망(일부 상대만 격리)의 지연은
/// [`S4Pacer`]의 "S4로만 보이는 상대" 신호가 다시 촘촘하게 되돌려 막는다.
const S4_EVERY_RELAXED: u32 = 64;
/// 이 시간 안에 남의 Announce가 47100/v6 소켓에 왔으면 "S1~S3 생존"(≈ 촘촘한 라운드 2회분 + 여유).
const S13_FRESH_MS: u64 = 30_000;
/// S4로만 보이는 상대를 본 뒤 이만큼은 촘촘한 주기를 유지(생존 타임아웃 60초 × 3 — 한 라운드 유실에도 목록이
/// 깜빡이지 않게).
const S4_ONLY_HOLD_MS: u64 = 180_000;
/// 상대별 "최근 S1~S3로 보였다" 판정 창(주기 광고 800ms × 6 — 이 안에 Announce가 없으면 그 상대는 S4 경로뿐).
const PEER_S13_FRESH_MS: u64 = 5_000;

/// 적응형 S4 페이서(R-22 ③ · **순수** — 시각은 ms 인자 · 결정적 테스트). 두 신호를 본다:
/// - **S1~S3 생존**: 47100/v6 수신 소켓에 남의 Announce(주기 광고 — S4 응답은 송신 소켓으로 가므로 섞이지 않는다).
/// - **S4로만 보이는 상대**: 내 프로브의 응답(송신 소켓)이나 47100에 온 HELLO를 보낸 상대가 최근 S1~S3로 안 보였다.
///
/// 판정: S1~S3 생존 ∧ S4-only 상대 없음 → 64주기 · 그 밖(못 듣는 수신 강등 포함) → 16주기(종전 동작 = 안전 기본).
#[derive(Debug)]
pub(crate) struct S4Pacer {
    /// 47100/v6에 남의 Announce가 마지막으로 온 시각.
    last_s13_ms: Option<u64>,
    /// 상대별 마지막 S1~S3 관측(상한에서 비움 — 상주 상한).
    s13_by_peer: HashMap<PeerId, u64>,
    /// S4 경로로만 보이는 상대를 마지막으로 확인한 시각.
    last_s4_only_ms: Option<u64>,
}

impl S4Pacer {
    fn new() -> Self {
        Self {
            last_s13_ms: None,
            s13_by_peer: HashMap::new(),
            last_s4_only_ms: None,
        }
    }

    /// 47100/v6 소켓에 남의 Announce — S1~S3 생존 증거.
    fn on_s13_announce(&mut self, peer: PeerId, now_ms: u64) {
        self.last_s13_ms = Some(now_ms);
        if self.s13_by_peer.len() > 4096 {
            self.s13_by_peer.clear();
        }
        self.s13_by_peer.insert(peer, now_ms);
    }

    fn peer_s13_fresh(&self, peer: PeerId, now_ms: u64) -> bool {
        self.s13_by_peer
            .get(&peer)
            .is_some_and(|&t| now_ms.saturating_sub(t) <= PEER_S13_FRESH_MS)
    }

    /// 내 S4 프로브의 응답(송신 소켓으로 온 Announce) — 그 상대가 최근 S1~S3로 안 보였으면 **S4-only**.
    fn on_s4_reply(&mut self, peer: PeerId, now_ms: u64) {
        if !self.peer_s13_fresh(peer, now_ms) {
            self.last_s4_only_ms = Some(now_ms);
        }
    }

    /// 47100에 온 HELLO — **전에 S1~S3로 보였던** 상대가 이제 안 보이면(격리 구간으로 이동) S4-only. 처음 보는
    /// 상대의 HELLO는 증거로 안 센다(기동 직후 방송 HELLO는 Announce보다 먼저 와서 오판이 된다).
    fn on_hello(&mut self, peer: PeerId, now_ms: u64) {
        if self.s13_by_peer.contains_key(&peer) && !self.peer_s13_fresh(peer, now_ms) {
            self.last_s4_only_ms = Some(now_ms);
        }
    }

    /// 지금 S4 라운드 주기(광고 틱 단위).
    fn every(&self, now_ms: u64) -> u32 {
        let s13_alive = self
            .last_s13_ms
            .is_some_and(|t| now_ms.saturating_sub(t) <= S13_FRESH_MS);
        let s4_only_recent = self
            .last_s4_only_ms
            .is_some_and(|t| now_ms.saturating_sub(t) <= S4_ONLY_HOLD_MS);
        if s13_alive && !s4_only_recent {
            S4_EVERY_RELAXED
        } else {
            S4_EVERY
        }
    }

    /// 링크 변화([`UdpDiscovery::kick`]) — 다시 배운다(촘촘한 주기부터).
    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// 페이서 공유 핸들 — 시계(기동 기준 ms)와 잠금을 묶는다(수신 스레드 2~3개 + 애넌서).
#[derive(Debug)]
struct PacerShared {
    start: std::time::Instant,
    inner: std::sync::Mutex<S4Pacer>,
}

impl PacerShared {
    fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
            inner: std::sync::Mutex::new(S4Pacer::new()),
        }
    }
    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn with<R>(&self, f: impl FnOnce(&mut S4Pacer, u64) -> R) -> R {
        let now = self.now_ms();
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        f(&mut g, now)
    }
}

/// 수신 스레드의 역할 — 어느 소켓을 읽는가(페이서 신호의 의미가 다르다).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RxRole {
    /// 47100 v4 / v6 멀티캐스트 수신 소켓 — Announce = S1~S3 생존.
    Broadcast,
    /// 송신 소켓(임의 포트) — 여기 오는 Announce = 내 S4 프로브의 응답.
    SendSock,
}

/// 수신 스레드 공통 문맥(인자 묶음 — clippy too_many_arguments 회피가 아니라 역할이 같은 값들).
struct RxCtx {
    me: PeerId,
    tx: Sender<Observation>,
    stop: Arc<AtomicBool>,
    quiet: Arc<AtomicBool>,
    pacer: Arc<PacerShared>,
    role: RxRole,
}

impl RxCtx {
    fn fork(&self, role: RxRole) -> Self {
        Self {
            me: self.me,
            tx: self.tx.clone(),
            stop: Arc::clone(&self.stop),
            quiet: Arc::clone(&self.quiet),
            pacer: Arc::clone(&self.pacer),
            role,
        }
    }
}

/// 수신된 발견 관측 — 해석된 패킷 + 발신 주소(연결 힌트).
#[derive(Debug)]
pub struct Observation {
    /// 해석된 패킷(자기 자신은 이미 걸러짐 · **미검증 힌트**).
    pub packet: Packet,
    /// 발신 소켓 주소(세션 연결의 주소 힌트 — 신원 아님).
    pub from: SocketAddr,
    /// 세션 세대(M1-13 ⓒ) — 패킷 `flags` 하위 4비트 · 미래 와이어 = [`nbeep_core::FOREIGN_GEN`].
    pub gen: u8,
}

/// S2 발견 노드 — 주기 광고 + 수신. 드롭 시 GOODBYE.
#[derive(Debug)]
pub struct UdpDiscovery {
    events: std::sync::Mutex<Option<Receiver<Observation>>>,
    stop: Arc<AtomicBool>,
    /// 광고 스레드 정지 신호(RL-10 · 11-04) — 드롭 = 끊김 = 즉시 깸. 루프는 `recv_timeout(announce)`로 **통잠**(종전 100ms
    /// 쪼갬 = 10Hz 웨이크업 · 필요 주기의 8배).
    announce_stop: Option<std::sync::mpsc::Sender<()>>,
    /// ★ 조용한 모드(내 상태 = 오프라인 · 10-19) — 광고·HELLO 응답·관측 전부 멈춤(소켓·스레드는 유지).
    quiet: Arc<AtomicBool>,
    /// ★ 발견 광고(R-18 ⓑ · 11-07) — `false` = **조용한 노드**: 주기 광고·기동/링크 HELLO·S4 프로브·이름/힌트
    /// 즉시 송출·GOODBYE를 **보내지 않는다**. 수신·HELLO 응답·관측은 그대로(남이 나를 먼저 찾지는 못하지만,
    /// 남의 광고를 듣고 내가 걸면 대화된다 · 수동 등록 S6는 영향 없음). `quiet`와 독립(둘 다 참이어야 보낸다).
    advertise: Arc<AtomicBool>,
    /// 적응형 S4 페이서(R-22 ③) — 수신 스레드가 신호를 넣고 애넌서가 주기를 읽는다.
    pacer: Arc<PacerShared>,
    send_sock: UdpSocket,
    /// 내 광고 원본(seq는 전송 시 갱신) — 애넌서와 공유. 이름 변경(M1-10 즉시
    /// 재공지)이 여기를 갱신하면 다음 주기부터 새 이름으로 나간다.
    template: Arc<std::sync::Mutex<Packet>>,
    seq: Arc<AtomicU32>,
    /// ★ 사용자 힌트 태그(ADR-0015) — `Some`이면 광고마다 `UserHint` 패킷을 덧붙인다.
    user_tag: Arc<std::sync::Mutex<Option<[u8; 16]>>>,
    /// 수신 강등(M1-13ⓔ) — 발견 포트를 다른 프로세스가 배타 점유해 **듣지 못한다**.
    /// 발신은 임의 포트라 정상(상대 목록에 나는 뜬다) — 호스트가 상태바에 고지한다.
    recv_degraded: bool,
    /// 수신 소켓의 조인용 복제(M1-2 [`Self::kick`] — 링크 변화 시 새 인터페이스
    /// 그룹 재조인). 소켓 옵션은 어느 핸들로든 같은 소켓에 적용된다.
    recv_join: Option<UdpSocket>,
}

impl UdpDiscovery {
    /// 발견을 시작한다 — 광고 스레드(주기 `announce_ms`) + 수신 스레드.
    ///
    /// # Errors
    /// **송신** 소켓 생성 실패 시 `io::Error`(방화벽·권한·인터페이스 부재).
    /// 수신 소켓 실패(발견 포트 배타 점유)는 오류가 아니라 **발신 전용 강등**이다
    /// (M1-13ⓔ — 종전엔 기동 패닉 · [`Self::recv_degraded`]로 조회).
    pub fn spawn(
        me: PeerId,
        instance: [u8; 16],
        name: DisplayName,
        tcp_port: u16,
        epoch: u64,
        announce_ms: u32,
        advertise: bool,
    ) -> std::io::Result<Self> {
        // ── 수신 소켓: 발견 포트에 재사용 바인딩 + 그룹 가입 ──
        // ★ 실패 = 치명 아님(M1-13ⓔ): 포트 47100은 프로토콜 헌법이라 바꿀 수 없고,
        // 다른 앱이 배타 점유하면 들을 수만 없다. 발신·세션은 멀쩡하므로 수신만
        // 포기한다 — 상대 화면에 나는 뜨고, 상대가 걸어오면 인바운드로 대화 성립.
        let recv: Option<UdpSocket> = (|| -> std::io::Result<UdpSocket> {
            let recv = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            recv.set_reuse_address(true)?;
            #[cfg(unix)]
            recv.set_reuse_port(true)?; // 같은 호스트 다중 인스턴스(개발·테스트)
            recv.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).into())?;
            recv.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
            // 인터페이스별 그룹 가입(M1-9) — 기본 경로 인터페이스만 조인되면
            // 링크로컬 직결(169.254)·보조 NIC의 광고를 못 듣는다. 실패는 무시
            // (이미 조인된 인터페이스는 EADDRINUSE류 — best-effort).
            for i in crate::netif::eligible_v4() {
                let _ = recv.join_multicast_v4(&MULTICAST_GROUP, &i.v4);
            }
            recv.set_multicast_loop_v4(true)?; // 같은 호스트 인스턴스 간 도달
            let recv: UdpSocket = recv.into();
            recv.set_read_timeout(Some(RECV_POLL))?;
            Ok(recv)
        })()
        .ok();
        let recv_degraded = recv.is_none();

        // ── 송신 소켓: 임의 포트(발신 주소가 연결 힌트가 된다) ──
        let send_sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        send_sock.set_multicast_ttl_v4(TTL)?;
        send_sock.set_multicast_loop_v4(true)?;
        send_sock.set_broadcast(true)?; // S3 — 멀티캐스트 차단 폴백
        send_sock.set_read_timeout(Some(RECV_POLL))?; // 응답 수신 스레드의 정지 폴링(발신에는 영향 없음)

        // ── S1: IPv6 멀티캐스트(best-effort — 실패해도 IPv4로 계속) ──
        let v6 = Self::setup_ipv6().ok();

        let template = Arc::new(std::sync::Mutex::new(Packet {
            kind: PacketKind::Announce,
            flags: u16::from(nbeep_core::SESSION_GEN), // 세대 비트(M1-13 ⓒ)
            peer: me,
            tcp_port,
            epoch,
            seq: 0,
            instance,
            name,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let quiet = Arc::new(AtomicBool::new(false));
        let advertise = Arc::new(AtomicBool::new(advertise)); // 기동 HELLO보다 먼저 정해져야 한다(setter로는 늦다)
        let seq = Arc::new(AtomicU32::new(0));
        let user_tag: Arc<std::sync::Mutex<Option<[u8; 16]>>> =
            Arc::new(std::sync::Mutex::new(None));
        let (tx, events) = channel::<Observation>();
        let pacer = Arc::new(PacerShared::new());
        let ctx = RxCtx {
            me,
            tx,
            stop: Arc::clone(&stop),
            quiet: Arc::clone(&quiet),
            pacer: Arc::clone(&pacer),
            role: RxRole::Broadcast,
        };

        let recv_join = recv.as_ref().and_then(|r| r.try_clone().ok());
        if let Some(recv) = recv {
            // v4 수신자는 HELLO 유니캐스트 응답까지(S4 왕복 — 송신 소켓 복제 실패 시
            // 응답 생략 = 종전 동작).
            let reply = send_sock
                .try_clone()
                .ok()
                .map(|s| (s, Arc::clone(&template), Arc::clone(&seq)));
            Self::spawn_receiver(recv, ctx.fork(RxRole::Broadcast), reply);
        }
        // ★ S4 왕복의 **발신측 절반**(11-07 결함 수정 · M1-8): 상대는 내 HELLO의 발신 주소(= 이 송신 소켓의 임의
        //   포트)로 유니캐스트 Announce를 되쏘는데, 종전엔 이 소켓을 읽는 스레드가 없어 그 응답이 **전부 버려졌다**
        //   (47100 수신 소켓은 임의 포트로 온 패킷을 받지 못한다). 막힌 망에선 양쪽이 각자 S4 라운드를 돌아야만
        //   발견됐다(≤12.8초 × 2). 같은 디코드·자기 필터·관측 경로를 재사용(응답은 하지 않는다 — Announce는 비유도).
        if let Ok(rx) = send_sock.try_clone() {
            Self::spawn_receiver(rx, ctx.fork(RxRole::SendSock), None);
        }
        let v6_send = if let Some((v6_recv, v6_send)) = v6 {
            if let Ok(rx) = v6_recv.try_clone() {
                Self::spawn_receiver(rx, ctx.fork(RxRole::Broadcast), None);
            }
            v6_send.try_clone().ok()
        } else {
            None
        };
        let (announce_tx, announce_rx) = std::sync::mpsc::channel::<()>();
        Self::spawn_announcer(
            send_sock.try_clone()?,
            v6_send,
            Arc::clone(&template),
            announce_ms,
            Arc::clone(&stop),
            Arc::clone(&seq),
            Arc::clone(&user_tag),
            Arc::clone(&quiet),
            Arc::clone(&advertise),
            Arc::clone(&pacer),
            announce_rx,
        );

        Ok(Self {
            events: std::sync::Mutex::new(Some(events)),
            announce_stop: Some(announce_tx),
            stop,
            quiet,
            advertise,
            pacer,
            send_sock,
            template,
            seq,
            user_tag,
            recv_degraded,
            recv_join,
        })
    }

    /// 수신 강등 여부(M1-13ⓔ) — true = 발견 포트 점유로 **듣지 못하는** 발신 전용.
    #[must_use]
    pub fn recv_degraded(&self) -> bool {
        self.recv_degraded
    }

    /// 링크 변화 후 재발견(M1-2) — 새로 생긴 인터페이스에 그룹 재조인(best-effort ·
    /// 이미 조인된 곳은 오류 무시) + 즉시 HELLO(응답 유도) + S4 1라운드. 주기 광고를
    /// 기다리지 않고 새 링크의 이웃을 당긴다. 호출 주체 = 호스트의 디바운서(폭주는
    /// 거기서 접힌다 — 여기서는 접지 않는다).
    pub fn kick(&self) {
        if let Some(j) = &self.recv_join {
            let sref = socket2::SockRef::from(j);
            for i in crate::netif::eligible_v4() {
                let _ = sref.join_multicast_v4(&MULTICAST_GROUP, &i.v4);
            }
        }
        self.pacer.with(|p, _| p.reset()); // 새 링크 — 적응 상태를 버리고 촘촘한 S4부터 다시 배운다(R-22 ③)
        if !self.may_send() {
            return; // 조용한 노드·오프라인 — 재조인만(새 링크의 광고를 듣는다) · HELLO·S4는 내지 않는다
        }
        let hello = {
            let mut p = match self.template.lock() {
                Ok(g) => g.clone(),
                Err(e) => e.into_inner().clone(),
            };
            p.kind = PacketKind::Hello;
            p.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            p
        };
        Self::send_pkt(&self.send_sock, None, &hello);
        Self::send_s4_pkt(&self.send_sock, &hello);
    }

    /// 관측 수신단의 **소유권**을 가져간다(1회 — InMemory `discovery()`와 같은 계약).
    ///
    /// # Panics
    /// 두 번 호출하면 패닉(구성 오류).
    #[must_use]
    pub fn take_events(&self) -> Receiver<Observation> {
        self.events
            .lock()
            .expect("잠금")
            .take()
            .expect("take_events는 1회만")
    }

    fn dest() -> SocketAddrV4 {
        SocketAddrV4::new(MULTICAST_GROUP, DISCOVERY_PORT)
    }

    fn broadcast_dest() -> SocketAddrV4 {
        SocketAddrV4::new(BROADCAST, DISCOVERY_PORT)
    }

    fn dest_v6() -> SocketAddrV6 {
        SocketAddrV6::new(MULTICAST_GROUP_V6, DISCOVERY_PORT, 0, 0)
    }

    /// S1 IPv6 멀티캐스트 소켓 쌍(recv, send) — best-effort. 링크로컬 그룹 가입은
    /// 기본 인터페이스(0)로 시도한다(정밀 인터페이스 선택은 M1-3 인터페이스 바인딩·D-8b).
    fn setup_ipv6() -> std::io::Result<(UdpSocket, UdpSocket)> {
        let recv = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        recv.set_only_v6(true)?; // IPv4는 별도 소켓 — 이중 바인딩 회피
        recv.set_reuse_address(true)?;
        #[cfg(unix)]
        recv.set_reuse_port(true)?;
        recv.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, DISCOVERY_PORT)).into())?;
        recv.join_multicast_v6(&MULTICAST_GROUP_V6, 0)?;
        recv.set_multicast_loop_v6(true)?;
        let recv: UdpSocket = recv.into();
        recv.set_read_timeout(Some(RECV_POLL))?;

        let send = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        send.set_only_v6(true)?;
        send.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)).into())?;
        send.set_multicast_loop_v6(true)?;
        let send: UdpSocket = send.into();
        Ok((recv, send))
    }

    /// S2 멀티캐스트 + S3 브로드캐스트 (+ 있으면 S1 IPv6) 동시 발신 — 하나만 통과해도 발견 성립.
    /// 패킷을 발신 정책의 **모든 버전으로**(M1-13 ⓑ · [`crate::wire::TX_VERS`]) — 지금은 1장.
    fn send_pkt(v4: &UdpSocket, v6: Option<&UdpSocket>, p: &Packet) {
        for b in p.encode_all() {
            Self::send_all(v4, v6, &b);
        }
    }

    fn send_s4_pkt(sock: &UdpSocket, p: &Packet) {
        for b in p.encode_all() {
            Self::send_s4(sock, &b);
        }
    }

    fn send_all(v4: &UdpSocket, v6: Option<&UdpSocket>, bytes: &[u8]) {
        // 기본 경로 발신(종전 동작) — 인터페이스 열거가 비는 환경(Windows 폴백 등)의
        // 안전망. 멀티캐스트 IF를 바꾼 뒤에도 마지막에 기본으로 되돌린다.
        let mut sent: u64 = 2; // netmon — 발신 데이터그램 계수(횟수만 · 봉투 원리)
        let _ = v4.send_to(bytes, Self::dest());
        let _ = v4.send_to(bytes, Self::broadcast_dest());
        // ★ 인터페이스별 명시 발신(M1-9 · FR-D-7) — 기본 경로가 없는 링크로컬 직결
        // (자기 배정 169.254)·다중 NIC에서도 상대 링크로 나간다. 가상·터널 제외는
        // netif가 맡는다(VPN으로 존재 방송 금지 — R-18 결).
        let sref = socket2::SockRef::from(v4);
        // RL-16 ⓐ(11-06): 같은 서브넷 브로드캐스트 주소는 1회(IF 하나에 IPv4 여럿 · 같은 링크의 NIC 둘이면 종전엔
        //   사본이 그만큼 늘었다). IF별 멀티캐스트는 주소마다 1회(인덱스가 없어 주소가 곧 키).
        let mut bcasts: HashSet<Ipv4Addr> = HashSet::new();
        for i in crate::netif::eligible_v4() {
            if sref.set_multicast_if_v4(&i.v4).is_ok() {
                let _ = v4.send_to(bytes, Self::dest());
                sent += 1;
            }
            // S3 지향 브로드캐스트 — 서브넷 브로드캐스트는 IF 지정과 무관하게
            // 목적지 주소로 경로가 정해진다(255.255.255.255와 달리 링크가 특정된다).
            if let Some(b) = i.subnet_broadcast() {
                if bcasts.insert(b) {
                    let _ = v4.send_to(bytes, SocketAddr::from((b, DISCOVERY_PORT)));
                    sent += 1;
                }
            }
        }
        let _ = sref.set_multicast_if_v4(&Ipv4Addr::UNSPECIFIED);
        if let Some(v6) = v6 {
            let _ = v6.send_to(bytes, Self::dest_v6());
            sent += 1;
        }
        crate::netmon::on_disco_tx(sent);
    }

    #[allow(clippy::too_many_arguments)] // quiet(10-19)·advertise(R-18 ⓑ)·pacer(R-22 ③) — 스레드 인자 묶음
    fn spawn_announcer(
        sock: UdpSocket,
        v6: Option<UdpSocket>,
        template: Arc<std::sync::Mutex<Packet>>,
        announce_ms: u32,
        stop: Arc<AtomicBool>,
        seq: Arc<AtomicU32>,
        user_tag: Arc<std::sync::Mutex<Option<[u8; 16]>>>,
        quiet: Arc<AtomicBool>,
        advertise: Arc<AtomicBool>,
        pacer: Arc<PacerShared>,
        stop_rx: Receiver<()>,
    ) {
        std::thread::spawn(move || {
            // 스냅샷 발신 — 잠금은 복사 순간만(소켓 I/O 중 잠금 유지 금지).
            let snapshot = |kind: PacketKind| {
                let mut p = match template.lock() {
                    Ok(g) => g.clone(),
                    Err(e) => e.into_inner().clone(),
                };
                p.kind = kind;
                p.seq = seq.fetch_add(1, Ordering::Relaxed);
                p
            };
            // 기동 직후 HELLO(응답 유도) 1회, 이후 주기 ANNOUNCE — 조용한 노드(R-18 ⓑ)는 기동 HELLO도 없다.
            if advertise.load(Ordering::Relaxed) && !quiet.load(Ordering::Relaxed) {
                Self::send_pkt(&sock, v6.as_ref(), &snapshot(PacketKind::Hello));
                // ★ S4(M1-8) — 기동 직후 이웃 유니캐스트 1회(막힌 망에서 첫 발견을 당긴다).
                Self::send_s4_pkt(&sock, &snapshot(PacketKind::Hello));
            }
            let period = Duration::from_millis(u64::from(announce_ms.max(1)));
            let mut cycles: u32 = 0;
            loop {
                // 통잠(RL-10): 주기만큼 자되 정지 신호(송신 끝 드롭·명시 send)면 즉시 깬다.
                match stop_rx.recv_timeout(period) {
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                {
                    if quiet.load(Ordering::Relaxed) || !advertise.load(Ordering::Relaxed) {
                        continue; // 조용한 모드(오프라인 상태)·조용한 노드(광고 끄기) — 이번 주기 광고 전부 생략
                    }
                    Self::send_pkt(&sock, v6.as_ref(), &snapshot(PacketKind::Announce));
                    // ★ 사용자 힌트(ADR-0015 §3-4) — 태그가 있으면 광고마다 한 장 더(≈70B).
                    let tag = match user_tag.lock() {
                        Ok(g) => *g,
                        Err(e) => *e.into_inner(),
                    };
                    if let Some(tag) = tag {
                        Self::send_pkt(
                            &sock,
                            v6.as_ref(),
                            &Self::hint_packet(snapshot(PacketKind::UserHint), tag),
                        );
                    }
                    cycles = cycles.wrapping_add(1);
                    // S4 주기 라운드 — 광고 틱 재사용(새 타이머 0 · [13 §12-1]):
                    // 16주기(기본 800ms면 ≈12.8초)마다 이웃 테이블을 다시 읽어 프로브.
                    // 멀티캐스트가 통하는 망에선 중복 관측일 뿐이고(무해), 막힌 망에선
                    // 이것이 유일한 발견 경로다(S1~S3 폴백 사다리의 다음 단 — 06 §4).
                    // ★ 적응형(R-22 ③): S1~S3가 살아 있고 S4로만 보이는 상대가 없으면 64주기(IDS 스윕 판정 완화).
                    if cycles % pacer.with(|p, now| p.every(now)) == 0 {
                        Self::send_s4_pkt(&sock, &snapshot(PacketKind::Hello));
                    }
                }
            }
        });
    }

    /// S4 유니캐스트 프로브(M1-8) — OS 이웃 테이블(ARP)의 주소 중 **자격 인터페이스
    /// 서브넷 안**의 것들에게 1:1 HELLO. 봉투 원리: 프로브 내용은 방송과 동일하다
    /// (이웃에게만 더 보내는 것 — 새 정보 없음). 상한 [`S4_MAX`](S4_MAX)로 폭주 방지.
    fn send_s4(sock: &UdpSocket, bytes: &[u8]) {
        for n in s4_targets(
            &crate::netif::eligible_v4(),
            crate::neigh::neighbors_v4(),
            S4_MAX,
        ) {
            let _ = sock.send_to(bytes, SocketAddr::from((n, DISCOVERY_PORT)));
            crate::netmon::on_disco_tx(1);
        }
    }

    /// 표시 이름 교체 + **즉시 재공지**(M1-10 · 사용자 확정 08-11) — 상대 목록은
    /// `PeerTable::observe`의 `Renamed` 경로로 갱신된다. 즉시 발신은 v4(멀티캐스트+
    /// 브로드캐스트)로 하고, v6은 다음 주기 광고에 실린다(GOODBYE와 같은 정책).
    /// 힌트 패킷 — `name` 자리에 태그 hex(32자).
    fn hint_packet(mut p: Packet, tag: [u8; 16]) -> Packet {
        let hex: String = tag.iter().map(|b| format!("{b:02x}")).collect();
        p.kind = PacketKind::UserHint;
        if let Ok(n) = DisplayName::parse(&hex) {
            p.name = n;
        }
        p
    }

    /// ★ 사용자 힌트 태그 설정(ADR-0015) — `Some`이면 즉시 1회 송출 + 이후 광고마다 동행.
    pub fn set_user_tag(&self, tag: Option<[u8; 16]>) {
        {
            let mut g = match self.user_tag.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            *g = tag;
        }
        if let (Some(tag), true) = (tag, self.may_send()) {
            let mut p = match self.template.lock() {
                Ok(g) => g.clone(),
                Err(e) => e.into_inner().clone(),
            };
            p.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            Self::send_pkt(&self.send_sock, None, &Self::hint_packet(p, tag));
        }
    }

    /// 지금 광고를 내도 되는가 — 광고 켜짐(R-18 ⓑ) **그리고** 조용한 모드 아님(10-19). 즉시 송출 경로(kick ·
    /// 이름·힌트 변경 · GOODBYE)가 전부 이 판정을 거친다(종전엔 오프라인 모드에서도 링크 변화·이름 변경 때 샜다).
    fn may_send(&self) -> bool {
        self.advertise.load(Ordering::Relaxed) && !self.quiet.load(Ordering::Relaxed)
    }

    /// 발견 광고 여부(R-18 ⓑ) — 상태바·툴팁 고지용.
    #[must_use]
    pub fn advertising(&self) -> bool {
        self.advertise.load(Ordering::Relaxed)
    }

    /// ★ 발견 광고 켜기/끄기(R-18 ⓑ · 11-07 · 설정 `net.discovery_advertise`): 끄면 GOODBYE(상대 목록에서 즉시
    /// 이탈 — 이후 내 광고가 없으니 타임아웃으로 사라질 뿐인데 명시 이탈이 더 빠르고 정직하다) · 켜면 즉시 HELLO
    /// ([`Self::kick`] — 재발견 당김). 수신·응답·관측은 양쪽 다 그대로.
    pub fn set_advertise(&self, on: bool) {
        let was = self.advertise.swap(on, Ordering::Relaxed);
        if was && !on {
            if !self.quiet.load(Ordering::Relaxed) {
                self.send_goodbye();
            }
        } else if !was && on {
            self.kick();
        }
    }

    pub fn set_name(&self, name: DisplayName) {
        let announce = {
            let mut t = match self.template.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            t.name = name;
            let mut p = t.clone();
            p.kind = PacketKind::Announce;
            p.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            p
        };
        if self.may_send() {
            Self::send_pkt(&self.send_sock, None, &announce);
        }
    }

    /// ★ 조용한 모드(내 상태 = 오프라인 · 10-19): `true` = 주기 광고·HELLO 응답·관측을 전부 멈추고
    /// GOODBYE 2회(상대 목록에서 즉시 이탈 · FR-D-8 문법) · `false` = 즉시 HELLO(재발견 당김 · [`Self::kick`]).
    /// 소켓·스레드는 유지한다(포트 점유 그대로 — 되돌릴 때 재바인딩 실패 없음).
    pub fn set_quiet(&self, quiet: bool) {
        let was = self.quiet.swap(quiet, Ordering::Relaxed);
        if quiet && !was {
            if self.advertise.load(Ordering::Relaxed) {
                self.send_goodbye(); // 광고한 적 없는 조용한 노드는 이탈할 명부도 없다
            }
        } else if was && !quiet {
            self.kick();
        }
    }

    /// 명시적 이탈(FR-D-8) — 유실 대비 2회(수신 중복은 무해). Drop·조용한 모드 진입이 공유.
    fn send_goodbye(&self) {
        let mut bye = match self.template.lock() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        };
        bye.kind = PacketKind::Goodbye;
        for _ in 0..2 {
            bye.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            // v4로만(v6 send 핸들은 애넌서 스레드 소유 — GOODBYE는 v4로 충분).
            Self::send_pkt(&self.send_sock, None, &bye);
        }
    }

    fn spawn_receiver(
        sock: UdpSocket,
        ctx: RxCtx,
        reply: Option<(UdpSocket, Arc<std::sync::Mutex<Packet>>, Arc<AtomicU32>)>,
    ) {
        let RxCtx {
            me,
            tx,
            stop,
            quiet,
            pacer,
            role,
        } = ctx;
        std::thread::spawn(move || {
            let mut buf = [0u8; crate::wire::MAX_PACKET];
            // RL-16 ⓐ(11-06): 발신자가 같은 HELLO를 사다리 전 단(기본 mcast·전역 bcast·IF mcast·서브넷 bcast)으로 보내므로
            //   같은 (peer, seq)가 3~4부 도착한다 — 응답은 1회(종전 = 사본마다 유니캐스트 응답 = 상대마다 3~4배 증폭).
            let mut hello_seen: HashMap<PeerId, u32> = HashMap::new();
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                match sock.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        crate::netmon::on_disco_rx();
                        if quiet.load(Ordering::Relaxed) {
                            continue; // 조용한 모드 — HELLO 응답도 관측도 없다(상대는 없는 것처럼)
                        }
                        // ★ 미래 와이어 버전(M1-13 ⓒ)은 헌법 접두의 키만 읽힌다 — 포트·이름 없이 "존재"만 올린다(비호환 표시).
                        let (packet, gen) = match Packet::decode(&buf[..n]) {
                            Decoded::Packet(p) => {
                                let g = p.gen();
                                (p, g)
                            }
                            Decoded::Foreign { peer, ver } => (
                                Packet {
                                    kind: PacketKind::Announce,
                                    flags: 0,
                                    peer,
                                    tcp_port: 0,
                                    epoch: 0,
                                    seq: 0,
                                    instance: [0u8; 16],
                                    name: foreign_label(peer, ver),
                                },
                                nbeep_core::FOREIGN_GEN,
                            ),
                            Decoded::Ignore | Decoded::Invalid => continue,
                        };
                        {
                            // 자기 패킷 필터 — 주소가 아니라 키 기준(docs/08 §5).
                            if packet.peer == me {
                                crate::netmon::on_disco_rx_self();
                                continue;
                            }
                            // 적응형 S4 신호(R-22 ③) — 소켓 역할 × 패킷 종류.
                            match (role, packet.kind) {
                                (RxRole::Broadcast, PacketKind::Announce) => {
                                    pacer.with(|p, now| p.on_s13_announce(packet.peer, now));
                                }
                                (RxRole::Broadcast, PacketKind::Hello) => {
                                    pacer.with(|p, now| p.on_hello(packet.peer, now));
                                }
                                (RxRole::SendSock, PacketKind::Announce) => {
                                    pacer.with(|p, now| p.on_s4_reply(packet.peer, now));
                                }
                                _ => {}
                            }
                            if packet.kind == PacketKind::Hello
                                && hello_reply_due(&mut hello_seen, packet.peer, packet.seq)
                            {
                                if let Some((rs, template, seq)) = &reply {
                                    let mut p = match template.lock() {
                                        Ok(g) => g.clone(),
                                        Err(e) => e.into_inner().clone(),
                                    };
                                    p.kind = PacketKind::Announce;
                                    p.seq = seq.fetch_add(1, Ordering::Relaxed);
                                    for b in p.encode_all() {
                                        let _ = rs.send_to(&b, from);
                                    }
                                    crate::netmon::on_disco_tx(1);
                                }
                            }
                            if tx.send(Observation { packet, from, gen }).is_err() {
                                return; // 수신자 소멸
                            }
                        }
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(_) => return,
                }
            }
        });
    }
}

/// 미래 버전 상대의 자리표시 이름 — `v<ver> <지문>`(이름은 그 판의 배치를 몰라 못 읽는다).
fn foreign_label(peer: PeerId, ver: u8) -> DisplayName {
    DisplayName::parse(&format!("v{ver} {}", peer.short()))
        .or_else(|_| DisplayName::parse(&peer.short()))
        .unwrap_or_else(|_| DisplayName::parse("peer").expect("고정 문자열"))
}

/// S4 프로브 대상 선별(M1-8 · 순수 — 회귀 대상): 이웃 중 **자격 인터페이스 서브넷
/// 안**의 주소만, 내 주소 제외, `cap`개까지. 서브넷 밖 이웃(라우터 너머 잔존 항목)에
/// 발견 패킷을 쏘지 않는다 — 링크 밖 유출 금지(FR-S-49 결).
fn s4_targets(ifs: &[crate::netif::NetIf], neighbors: Vec<Ipv4Addr>, cap: usize) -> Vec<Ipv4Addr> {
    neighbors
        .into_iter()
        .filter(|n| {
            ifs.iter().any(|i| {
                i.mask.is_some_and(|m| {
                    let m = u32::from(m);
                    m != 0 && (u32::from(*n) & m) == (u32::from(i.v4) & m) && *n != i.v4
                })
            })
        })
        .take(cap)
        .collect()
}

impl Drop for UdpDiscovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        drop(self.announce_stop.take()); // 광고 스레드 즉시 깸(통잠 중이어도)
        if self.may_send() {
            self.send_goodbye();
        }
    }
}

/// HELLO 응답 여부(RL-16 ⓐ) — 같은 발신자의 같은 `seq`는 사본이라 한 번만 응답한다. 표는 상한(4096)에서 비운다
/// (발신자가 많아도 상주 상한 · 비운 직후 사본 하나가 더 응답받는 정도는 무해).
fn hello_reply_due(seen: &mut HashMap<PeerId, u32>, peer: PeerId, seq: u32) -> bool {
    if seen.len() > 4096 {
        seen.clear();
    }
    seen.insert(peer, seq) != Some(seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RL-16 ⓐ: 같은 (peer, seq) 사본은 첫 1회만 응답 · 다음 seq·다른 peer는 응답 · 상한에서 표를 비운다.
    #[test]
    fn hello_reply_once_per_sender_seq() {
        let mut seen = HashMap::new();
        let a = pid(1);
        let b = pid(2);
        assert!(hello_reply_due(&mut seen, a, 7));
        assert!(!hello_reply_due(&mut seen, a, 7), "사본 = 무응답");
        assert!(!hello_reply_due(&mut seen, a, 7));
        assert!(hello_reply_due(&mut seen, b, 7), "다른 발신자");
        assert!(hello_reply_due(&mut seen, a, 8), "다음 HELLO");
        for i in 0..5000u16 {
            let mut k = [0u8; 32];
            k[..2].copy_from_slice(&i.to_le_bytes());
            k[2] = 9;
            hello_reply_due(&mut seen, PeerId::from_bytes(k), 1);
        }
        assert!(seen.len() <= 4097, "상한에서 비운다: {}", seen.len());
    }

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn name(s: &str) -> DisplayName {
        DisplayName::parse(s).unwrap()
    }

    /// 적응형 S4 페이서(R-22 ③ · 11-07) 판정표 — 순수·결정적.
    #[test]
    fn s4_pacer_relaxes_only_when_s13_alive_and_no_s4_only_peer() {
        let (a, b) = (pid(1), pid(2));
        let mut p = S4Pacer::new();
        assert_eq!(
            p.every(0),
            S4_EVERY,
            "아무것도 못 들었으면 촘촘(수신 강등 포함)"
        );
        p.on_s13_announce(a, 1_000);
        assert_eq!(p.every(2_000), S4_EVERY_RELAXED, "S1~S3 생존 → 64주기");
        assert_eq!(
            p.every(1_000 + S13_FRESH_MS + 1),
            S4_EVERY,
            "생존 증거가 낡으면 다시 촘촘"
        );
        // 내 프로브 응답을 보낸 b는 S1~S3로 본 적이 없다 → S4-only → 촘촘(보유 시간 동안).
        p.on_s13_announce(a, 10_000);
        p.on_s4_reply(b, 10_500);
        assert_eq!(p.every(11_000), S4_EVERY);
        assert_eq!(
            p.every(10_500 + S4_ONLY_HOLD_MS + 1),
            S4_EVERY,
            "보유 끝 + S1~S3도 낡음"
        );
        p.on_s13_announce(a, 10_500 + S4_ONLY_HOLD_MS + 2);
        assert_eq!(
            p.every(10_500 + S4_ONLY_HOLD_MS + 3),
            S4_EVERY_RELAXED,
            "보유 끝 + S1~S3 생존 → 다시 64"
        );
        // S1~S3로 방금 보인 상대의 응답은 S4-only 증거가 아니다.
        let mut p = S4Pacer::new();
        p.on_s13_announce(b, 100);
        p.on_s4_reply(b, 200);
        assert_eq!(p.every(300), S4_EVERY_RELAXED);
        // 처음 보는 상대의 HELLO(기동 방송)는 증거가 아니고, 전에 보였다가 5초 넘게 안 보이는 상대의 HELLO는 증거다.
        let mut p = S4Pacer::new();
        p.on_s13_announce(a, 0);
        p.on_hello(b, 10);
        assert_eq!(
            p.every(20),
            S4_EVERY_RELAXED,
            "처음 보는 상대의 HELLO는 무시"
        );
        p.on_hello(a, PEER_S13_FRESH_MS + 100);
        assert_eq!(
            p.every(PEER_S13_FRESH_MS + 200),
            S4_EVERY,
            "S1~S3로 보였던 상대가 HELLO로만 → 격리 이동"
        );
        p.reset();
        assert_eq!(
            p.every(PEER_S13_FRESH_MS + 300),
            S4_EVERY,
            "링크 변화 = 처음부터"
        );
    }

    /// S4 대상 선별(M1-8) — 서브넷 안만·내 주소 제외·상한. 라우터 너머 잔존
    /// ARP 항목으로 발견 패킷이 새면 링크 밖 유출이다(FR-S-49 결).
    #[test]
    fn s4_targets_stay_inside_subnet_and_cap() {
        use crate::netif::NetIf;
        use std::net::Ipv4Addr;
        let ifs = vec![NetIf {
            name: "en0".into(),
            v4: Ipv4Addr::new(192, 168, 45, 84),
            mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            up: true,
            loopback: false,
        }];
        let neigh = vec![
            Ipv4Addr::new(192, 168, 45, 1),   // 같은 서브넷 ✓
            Ipv4Addr::new(192, 168, 45, 84),  // 나 자신 ✗
            Ipv4Addr::new(10, 0, 0, 5),       // 다른 서브넷 ✗(잔존 항목)
            Ipv4Addr::new(192, 168, 45, 200), // 같은 서브넷 ✓
        ];
        let t = s4_targets(&ifs, neigh.clone(), 64);
        assert_eq!(
            t,
            vec![
                Ipv4Addr::new(192, 168, 45, 1),
                Ipv4Addr::new(192, 168, 45, 200)
            ]
        );
        assert_eq!(s4_targets(&ifs, neigh, 1).len(), 1, "상한 준수");
        assert!(s4_targets(&[], vec![Ipv4Addr::new(1, 2, 3, 4)], 64).is_empty());
    }

    /// M1-13ⓔ — 발견 포트를 다른 프로세스가 **배타 점유**해도 spawn은 성공하고
    /// 발신 전용으로 강등된다(종전엔 기동 패닉 = 제로 컨피그 정체성 파탄).
    ///
    /// Windows는 SO_REUSEADDR 의미가 달라(비재사용 소켓의 포트도 빼앗을 수 있다)
    /// 점유 시뮬레이션이 성립하지 않는다 — unix 한정("네트워크는 실측" · R-7 결).
    #[cfg(unix)]
    #[test]
    fn occupied_discovery_port_degrades_to_send_only() {
        // 재사용 플래그 없는 배타 점유자 — 우리 수신 바인딩(reuse여도)은 실패한다.
        let Ok(_hog) = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)) else {
            return; // 이 호스트에서 진짜 점유 중(다른 인스턴스) — 환경 의존이라 건너뜀
        };
        let d = UdpDiscovery::spawn(pid(9), [9; 16], name("hog"), 9000, 1, 60_000, true)
            .expect("점유 시에도 spawn은 성공(발신 전용)");
        assert!(d.recv_degraded(), "수신 강등이 표시돼야 호스트가 고지한다");
    }

    /// S4 왕복의 발신측 절반(11-07 결함 수정): 임의 포트의 **송신 소켓**으로 되돌아온 유니캐스트 Announce가
    /// 관측으로 올라온다. 멀티캐스트 없이 루프백 유니캐스트만 쓰므로 CI에서도 돈다.
    #[test]
    fn unicast_reply_to_send_socket_is_observed() {
        let a = UdpDiscovery::spawn(pid(3), [3; 16], name("asker"), 1000, 1, 60_000, true).unwrap();
        let ev = a.take_events();
        let me_port = a.send_sock.local_addr().unwrap().port();
        // 상대 역할: a의 송신 소켓 주소로 Announce 1장(상대가 HELLO에 되쏘는 것과 같은 바이트).
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = Packet {
            kind: PacketKind::Announce,
            flags: u16::from(nbeep_core::SESSION_GEN),
            peer: pid(4),
            tcp_port: 4000,
            epoch: 1,
            seq: 7,
            instance: [4; 16],
            name: name("replier"),
        };
        for b in p.encode_all() {
            peer.send_to(&b, (Ipv4Addr::LOCALHOST, me_port)).unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while std::time::Instant::now() < deadline && !seen {
            if let Ok(o) = ev.recv_timeout(Duration::from_millis(200)) {
                seen = o.packet.peer == pid(4) && o.packet.tcp_port == 4000;
            }
        }
        assert!(
            seen,
            "송신 소켓으로 온 유니캐스트 응답이 관측돼야 한다(종전 = 버려짐)"
        );
    }

    /// 같은 호스트 두 인스턴스의 상호 발견 — 실물 UDP 멀티캐스트.
    ///
    /// CI 러너의 멀티캐스트 지원이 보장되지 않아 `#[ignore]` — 로컬·Docker(D-8a)에서
    /// `cargo test -- --ignored`로 실행한다([docs/18] 절차).
    #[test]
    #[ignore = "실네트워크 멀티캐스트 필요 — 로컬/Docker에서 --ignored로 실행(D-8a)"]
    fn two_instances_discover_each_other_via_real_multicast() {
        let a = UdpDiscovery::spawn(pid(1), [1; 16], name("alpha"), 1000, 1, 300, true).unwrap();
        let b = UdpDiscovery::spawn(pid(2), [2; 16], name("beta"), 2000, 1, 300, true).unwrap();
        let (a_ev, b_ev) = (a.take_events(), b.take_events());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (mut a_saw, mut b_saw) = (false, false);
        while std::time::Instant::now() < deadline && !(a_saw && b_saw) {
            if let Ok(o) = a_ev.recv_timeout(Duration::from_millis(200)) {
                assert_ne!(o.packet.peer, pid(1), "자기 패킷은 키로 걸러진다");
                if o.packet.peer == pid(2) {
                    assert_eq!(o.packet.name.as_str(), "beta");
                    assert_eq!(o.packet.tcp_port, 2000);
                    a_saw = true;
                }
            }
            if let Ok(o) = b_ev.recv_timeout(Duration::from_millis(200)) {
                if o.packet.peer == pid(1) {
                    b_saw = true;
                }
            }
        }
        assert!(
            a_saw && b_saw,
            "상호 발견 실패(a_saw={a_saw} b_saw={b_saw})"
        );
    }

    /// R-18 ⓑ(11-07) — 조용한 노드(광고 끄기)는 기동 HELLO·주기 광고를 내지 않아 상대 목록에 뜨지 않지만,
    /// 상대의 광고는 듣는다(비대칭 발견). 켜면 즉시 HELLO로 나타난다.
    #[test]
    #[ignore = "실네트워크 멀티캐스트 필요 — 로컬/Docker에서 --ignored로 실행(D-8a)"]
    fn silent_node_hears_but_is_not_seen_until_enabled() {
        let loud = UdpDiscovery::spawn(pid(5), [5; 16], name("loud"), 1000, 1, 300, true).unwrap();
        let silent =
            UdpDiscovery::spawn(pid(6), [6; 16], name("silent"), 2000, 1, 300, false).unwrap();
        assert!(!silent.advertising());
        let (l_ev, s_ev) = (loud.take_events(), silent.take_events());
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let (mut loud_saw_silent, mut silent_saw_loud) = (false, false);
        while std::time::Instant::now() < deadline {
            if let Ok(o) = l_ev.recv_timeout(Duration::from_millis(100)) {
                loud_saw_silent |= o.packet.peer == pid(6);
            }
            if let Ok(o) = s_ev.recv_timeout(Duration::from_millis(100)) {
                silent_saw_loud |= o.packet.peer == pid(5);
            }
        }
        assert!(silent_saw_loud, "조용한 노드도 남의 광고는 듣는다");
        assert!(
            !loud_saw_silent,
            "조용한 노드는 3초 동안 한 번도 보이지 않아야 한다(기동 HELLO·주기 광고 0)"
        );
        silent.set_advertise(true);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && !loud_saw_silent {
            if let Ok(o) = l_ev.recv_timeout(Duration::from_millis(100)) {
                loud_saw_silent |= o.packet.peer == pid(6);
            }
        }
        assert!(loud_saw_silent, "광고를 켜면 즉시 HELLO로 나타난다");
    }

    /// M1-10 — `set_name`이 **즉시 재공지**하고, 이후 주기 광고에도 새 이름이 실린다.
    #[test]
    #[ignore = "실네트워크 멀티캐스트 필요 — 로컬/Docker에서 --ignored로 실행(D-8a)"]
    fn rename_reannounces_immediately() {
        let a =
            UdpDiscovery::spawn(pid(1), [1; 16], name("before"), 1000, 1, 60_000, true).unwrap();
        let b =
            UdpDiscovery::spawn(pid(2), [2; 16], name("watcher"), 2000, 1, 60_000, true).unwrap();
        let b_ev = b.take_events();
        // a의 기동 HELLO가 b에 닿을 때까지 대기(채널 정착).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen_before = false;
        while std::time::Instant::now() < deadline && !seen_before {
            if let Ok(o) = b_ev.recv_timeout(Duration::from_millis(200)) {
                seen_before = o.packet.peer == pid(1) && o.packet.name.as_str() == "before";
            }
        }
        assert!(seen_before, "기동 광고 미도달");
        // 주기(60s)가 오기 한참 전에 — set_name의 즉시 발신만으로 새 이름이 닿아야 한다.
        a.set_name(name("after"));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen_after = false;
        while std::time::Instant::now() < deadline && !seen_after {
            if let Ok(o) = b_ev.recv_timeout(Duration::from_millis(200)) {
                seen_after = o.packet.peer == pid(1) && o.packet.name.as_str() == "after";
            }
        }
        assert!(seen_after, "즉시 재공지 미도달(주기 60s 내 도달 = 실패)");
    }
}
