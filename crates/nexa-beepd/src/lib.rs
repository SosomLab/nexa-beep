//! `nexa-beepd` — 릴레이 서버(X-1 · [docs/32 §12-6]) · **와이어 v2**(ADR-0016 · [docs/56]).
//!
//! **하는 일 네 가지가 전부다**(비목표 = 계정·오프라인 저장·그룹 팬아웃·웹 가입):
//! 1. **프레즌스** — 클라이언트가 붙어 회전 RID를 등록([docs/32 §2-3] — `PeerId` 원본 금지). 라우팅 키는
//!    **`(app, rid)`**(앱 이름공간 — beep·clip이 같은 서버를 써도 교차 Open 불가).
//! 2. **랑데부/관측** — TCP·UDP에서 밖에서 본 공인 엔드포인트를 알려 주고(STUN-lite),
//!    열기 요청 시 양쪽에 서로의 관측 엔드포인트를 전달한다(홀펀칭 재료 · X-UDP-c).
//! 3. **blind 릴레이** — 펀치가 안 되는 쌍을 위해 **양방향 성립(Accept) 후에만** 종단
//!    암호문을 중계한다(증폭 방지 [docs/32 §2-6] · 토큰 버킷).
//! 4. **서버 자기 신원** — 서버도 Noise 정적 키를 가진다(클라이언트가 TOFU 핀 — §2-4).
//!
//! **서버가 보는 것** = kind·RID·채널 번호·바이트 수·시각. **저장하지 않는다** — 모드 ①은
//! 버퍼가 아니라 파이프다(양쪽 동시 접속일 때만 흐른다 · [docs/32 §2-6]).
//! 로그에도 같은 원칙(봉투만 — RID 앞 4바이트·건수·바이트).
//!
//! ## 모듈 경계(ADR-0016 §2-3)
//!
//! ```text
//! lib.rs      설정·바인드·공유 표·Handle        session.rs  프리앰블 → Noise(prologue) → Hello/Welcome → 루프 · 상태기계
//! router.rs   (app, rid) 바인딩 · 채널 표 · 중계   roster.rs   앱별 명부(Announce/PeerUp/PeerDown · 능력 ROSTER)
//! udp_obs.rs  UDP 관측 프로브(능력 UDP_OBSERVE)   metrics.rs  카운터(세션·채널·바이트·오류) — 상태만
//! ```
//!
//! 동시성 모델은 클라이언트와 같은 **세션 액터** 문법이다 — 연결 하나 = 스레드 하나가
//! 세션을 단독 소유하고, 남이 보낼 것은 그 연결의 큐(mpsc)에 넣는다. 큐가 상한(`limits.queue_bytes`)을
//! 넘는 느린 소비자는 **정직하게 끊는다**(무한 버퍼·쓰기 교착 둘 다 회피 — 08-16 교훈).
//!
//! **호환**: v2 전용(사용자 결정 11-07 "호환 포기"). v1 클라(프리앰블 없음)는 첫 4B에서 갈려 **조용히 끊긴다**
//! — 그쪽은 Noise 실패로 보이고, 이쪽은 `rejected_v1` 카운터에 남는다(`session` 프리앰블 단계 = 이중 수용 자리).
#![forbid(unsafe_code)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod metrics;
mod roster;
mod router;
mod session;
mod udp_obs;

use nbeep_core::PeerId;
use nbeep_relay_proto::{caps, Limits, Rid};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use metrics::Snapshot;

/// 릴레이 서버 기본 포트(TCP 제어 + 같은 번호의 UDP 관측) — 발견 47100·세션 47200 다음.
pub const DEFAULT_PORT: u16 = 47_300;

/// 연결당 릴레이 기본 예산(바이트/초). 0 = 무제한.
pub const DEFAULT_RATE_BPS: u64 = 1024 * 1024;

/// 이 서버가 제공하는 능력(Welcome.caps = 이 값 ∩ Hello.caps).
pub const SERVER_CAPS: u64 = caps::ROSTER | caps::SHARED_BIND | caps::UDP_OBSERVE;

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
    /// 연결당 릴레이 예산(바이트/초 · 0 = 무제한) — `Welcome.limits.rate_bps`로도 알린다.
    pub rate_bps: u64,
    /// 상세 로그.
    pub verbose: bool,
    /// 클라에 알리는 상한(프레임·RID·채널·큐·카드·핑 주기). 기본 = [`Limits::default`].
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_ip: IpAddr::from([0u8, 0, 0, 0]),
            port: DEFAULT_PORT,
            key_path: PathBuf::from("beepd.key"),
            rate_bps: DEFAULT_RATE_BPS,
            verbose: false,
            limits: Limits::default(),
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
    /// 서버 정지 — 연결 스레드들이 `Goaway{Shutdown}`을 보내고 내려간다(수락·UDP 스레드 합류까지).
    pub fn shutdown(mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        // 연결 스레드는 detach — 15ms 폴이 stop을 보고 Goaway 뒤 종료한다. 잠깐 기다려 로그가 정돈되게.
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while self.shared.metrics.sessions_live.load(Ordering::SeqCst) > 0
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.shared.verbose {
            eprintln!("[beepd] 종료 — {}", self.shared.metrics.snapshot());
        }
    }

    /// 계측 스냅숏(상태만).
    #[must_use]
    pub fn metrics(&self) -> Snapshot {
        self.shared.metrics.snapshot()
    }
}

pub(crate) type ConnId = u64;

/// 연결 하나의 공유 손잡이 — 남의 스레드가 이 연결에 보낼 때 쓴다.
#[derive(Debug)]
pub(crate) struct ConnHandle {
    /// 인코딩된 봉투(길이 접두 없음 — 세션이 프레임으로 싼다).
    pub(crate) tx: Sender<Vec<u8>>,
    /// 대기 중 송신 바이트(느린 소비자 판정) — 넣는 쪽 증가, 보내는 쪽 감소.
    pub(crate) queued: Arc<AtomicUsize>,
    /// 절단 지시(남의 스레드가 세운다 — 본인 루프가 확인).
    pub(crate) kill: Arc<AtomicBool>,
    /// 이 연결이 바인딩한 RID들(정리용).
    pub(crate) rids: Vec<Rid>,
    /// UDP 관측 엔드포인트(프로브 도착 시 갱신).
    pub(crate) udp_obs: Option<SocketAddr>,
}

/// 공유 상태 — 표마다 잠금 따로 · 중첩 잠금 없음(교착 회피).
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) stop: AtomicBool,
    pub(crate) conns: Mutex<HashMap<ConnId, ConnHandle>>,
    /// `(app, rid)` → 바인딩(Exclusive = 1연결 · Shared = n연결).
    pub(crate) rids: Mutex<HashMap<(String, Rid), router::Binding>>,
    pub(crate) chans: Mutex<HashMap<u32, router::Chan>>,
    /// UDP 프로브 토큰 → 연결.
    pub(crate) tokens: Mutex<HashMap<u64, ConnId>>,
    /// 프레즌스 공개 목록(옵트인 — Announce를 켠 연결만 · 메모리만 · 연결 종료 = 즉시 제거).
    pub(crate) listed: Mutex<HashMap<ConnId, roster::Listed>>,
    pub(crate) next_ch: AtomicU32,
    pub(crate) next_conn: AtomicU64,
    pub(crate) udp_port: u16,
    pub(crate) limits: Limits,
    pub(crate) verbose: bool,
    pub(crate) metrics: metrics::Metrics,
}

impl Shared {
    pub(crate) fn log(&self, msg: &str) {
        if self.verbose {
            eprintln!("[beepd] {msg}");
        }
    }
}

pub(crate) fn rid_short(rid: &Rid) -> String {
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

    // TCP·UDP를 같은 번호에 — port 0(테스트)이면 한쪽이 정한 번호로 다른 쪽을 몇 번 재시도.
    // ★ 10-10 CI(windows-latest) 실측: TCP가 고른 임시 번호로 UDP bind가 **10회 연속** 10013(WSAEACCES) —
    //   Windows는 TCP·UDP의 제외 포트 범위(Hyper-V 예약)가 **서로 달라** 한쪽이 고른 번호가 다른 쪽 제외 범위에
    //   든다. 그래서 **TCP-먼저 / UDP-먼저를 교대**로(각자 OS가 자기 제외 범위를 피해 고른 번호를 상대에게 시도)
    //   40회까지 돈다. 고정 포트(운영)는 종전처럼 1회 — 실패를 숨기지 않는다.
    let (listener, udp) = {
        let mut last_err = None;
        let mut pair = None;
        let attempts = if cfg.port == 0 { 40 } else { 1 };
        for i in 0..attempts {
            if i % 2 == 0 {
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
            } else {
                let u = UdpSocket::bind((cfg.bind_ip, 0))?;
                let p = u.local_addr()?.port();
                match TcpListener::bind((cfg.bind_ip, p)) {
                    Ok(l) => {
                        pair = Some((l, u));
                        break;
                    }
                    Err(e) => last_err = Some(e),
                }
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

    let mut limits = cfg.limits;
    limits.rate_bps = cfg.rate_bps;
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        conns: Mutex::new(HashMap::new()),
        rids: Mutex::new(HashMap::new()),
        chans: Mutex::new(HashMap::new()),
        tokens: Mutex::new(HashMap::new()),
        listed: Mutex::new(HashMap::new()),
        next_ch: AtomicU32::new(1),
        next_conn: AtomicU64::new(1),
        udp_port,
        limits,
        verbose: cfg.verbose,
        metrics: metrics::Metrics::default(),
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
                .spawn(move || udp_obs::udp_loop(&udp, &shared))?,
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

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>, id: &Arc<nbeep_crypto::Identity>) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                let shared = Arc::clone(shared);
                let id = Arc::clone(id);
                // 연결 스레드는 detach — 종료는 stop 플래그·소켓 타임아웃이 정리한다.
                let _ = std::thread::Builder::new()
                    .name("beepd-conn".into())
                    .spawn(move || session::run_conn(stream, peer_addr, &shared, &id));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}
