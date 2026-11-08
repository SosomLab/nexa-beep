//! `nbeep-relay` — 릴레이 **클라이언트 어댑터**(X-1·X-2b · [docs/32 §12-6·§13]). 와이어 v2(ADR-0016 · [docs/56]).
//!
//! **와이어 정의는 [`nbeep_relay_proto`] 한 곳**이다 — 서버(`nexa-beepd`)·이 어댑터·자매 프로젝트 `nexa-clip`
//! (사본 · [docs/44 §7])이 같은 봉투+TLV 바이트를 쓴다. 이 크레이트는 **연결·핸드셰이크·세션 액터·채널·사다리**만 남긴다.
//!
//! ## 핸드셰이크(v2)
//!
//! ```text
//! TCP → 프리앰블 "NBPD"+major(8B 평문 · 세대 판별) ‖ Noise_XX(prologue "nexa-beepd/2" · 서버 키 TOFU 핀)
//!     → Hello{app, app_version, caps} ← Welcome{server_version, server_type, caps, limits, udp_port, observed} | Reject
//!     → Register{rids, bind} ← RegisterResult{(rid,status)×n, udp_token}
//! ```
//! - 서버 major가 다르면 [`RelayError::VersionMismatch`] — "서버 세대 불일치 · 앱 업데이트 필요"로 **표시**한다
//!   (v1처럼 기능만 조용히 빠지는 길이 없다). 프리앰블이 없는 서버(v1·beepd 아님)는 `server_major = 0`.
//! - 상한(프레임·RID 수·카드)은 `Welcome.limits`를 따른다 — 하드코딩 상수([`MAX_RIDS`]·[`CARD_MAX`])는 **기본값**일 뿐이다.
//!
//! ## 봉투 원리(S-3)
//!
//! 서버가 보는 것 = **회전 RID · 채널 번호 · 바이트 수 · 시각**이 전부다.
//! - 제어 세션은 서버와의 Noise(서버 신원 키 = TOFU 핀 대상 — [docs/32 §2-4])이고,
//! - 그 안의 [`proto::Kind::Data`]/[`proto::Kind::DataS`] 페이로드는 **종단 A↔B의 Noise 암호문**이다.
//!   서버는 자기 전송 계층을 벗겨도 종단 암호문만 남는다(릴레이는 MITM이 아니다 —
//!   종단 핸드셰이크는 상대와 직접 한다 · [docs/32 §2-4] 시퀀스).
//!
//! ## 회전 RID (R-18 확대 방지 — [docs/32 §2-3])
//!
//! `RID = SHA-256("nbeep-rid-v1" ‖ 공개키 ‖ epoch_day)[..16]` — 서버에 `PeerId` 원본을
//! 주지 않는다. 에폭은 UTC 일 단위이고, 시계 오차 흡수를 위해 **어제·오늘·내일 셋**을
//! 등록한다([`rids_around`]) — 상대가 자기 시계의 "오늘"로 계산해도 반드시 겹친다.
#![forbid(unsafe_code)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

use nbeep_core::link::{Link, LinkError};
use nbeep_core::session::{Session, SessionError};
use nbeep_core::PeerId;
use nbeep_crypto::{Identity, NoiseSession};
use nbeep_net::TcpLink;
pub use nbeep_relay_proto as proto;
use proto::{Channel, Envelope, Kind, Presence, Problem};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// X-2 Transport 승격(11-06) — [`RelayTransport`]·[`LadderTransport`].
pub mod transport;
pub use transport::{LadderTransport, RelayTransport};

/// 릴레이 서버 기본 포트(TCP 제어 + 같은 번호의 UDP 관측) — 발견 47100·세션 47200 다음.
pub const DEFAULT_RELAY_PORT: u16 = 47_300;

/// 랑데부 ID(16B) — 회전 가명([docs/32 §2-3]).
pub type Rid = [u8; 16];

/// UDP 관측 프로브 매직(= [`proto::udp_obs::MAGIC`]) — ARQ 매직(`NBU1`)과 다른 4B라 같은 소켓에서 섞여도 갈린다.
pub const OBS_MAGIC: [u8; 4] = proto::udp_obs::MAGIC;

/// 이 클라가 쓰는 능력(Hello.caps) — 세션 능력은 서버와의 교집합([`ServerInfo::caps`]).
pub const CLIENT_CAPS: u64 = proto::caps::ROSTER | proto::caps::UDP_OBSERVE;

/// 링크 프레임 상한 — TCP·UDP 링크와 정합(Noise 상한).
pub const MAX_FRAME: usize = nbeep_net::arq::MAX_FRAME;

/// 릴레이로 나르는 조각 상한 — 제어 세션 페이로드(Noise 상한 65519) − 헤더 여유.
/// [`RelayLink`]는 이보다 큰 프레임을 투명하게 분할·조립한다(65535 프레임 수용).
pub const RELAY_CHUNK: usize = 32 * 1024;

/// RID 유도 — 에폭은 UTC 일 번호. 내 공개키를 **이미 아는** 사람만 같은 값을 계산할 수
/// 있다(릴레이는 새로운 만남을 주선하지 않는다 — [docs/32 §2-3]).
///
/// ⚠️ **`"nbeep-rid-v1"` 은 앱 식별자다**([docs/44](../../../docs/44-nexa-clip-liaison.md)).
/// 자매 프로젝트 `nexa-clip`이 **같은 릴레이 서버를 쓰면서** `"nclip-rid-v1"` 을 쓴다 —
/// 도메인 문자열이 다르기 때문에 두 앱 사용자가 **서로를 찾지 않는다**(앱 격리).
/// 이 값을 바꾸면 격리가 깨지거나 앱 내부에서 서로를 못 찾게 되므로, 변경이 필요하면
/// **양쪽을 같은 시점에** 고쳐야 한다. (서버는 RID를 계산하지 않으므로 서버 재배포는 불필요 —
/// [docs/44 §5](../../../docs/44-nexa-clip-liaison.md).)
#[must_use]
pub fn rid_for(peer: &PeerId, epoch_day: u64) -> Rid {
    let mut h = Sha256::new();
    h.update(b"nbeep-rid-v1");
    h.update(peer.as_bytes());
    h.update(epoch_day.to_be_bytes());
    let out = h.finalize();
    let mut rid = [0u8; 16];
    rid.copy_from_slice(&out[..16]);
    rid
}

/// 지금 시각의 에폭 일 번호(UTC).
#[must_use]
pub fn current_epoch_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400)
}

/// 64자리 hex 지문 → `PeerId`(= X25519 공개키 32B). RID 유도·랑데부 대상 지정에 쓴다
/// (짧은 지문 8자리로는 키를 복원할 수 없어 **전체 hex**가 교환 단위다).
#[must_use]
pub fn parse_peer_hex(s: &str) -> Option<PeerId> {
    let s = s.trim();
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(PeerId::from_bytes(out))
}

/// `PeerId` → 64자리 hex(교환·표시용 — [`parse_peer_hex`]의 역).
#[must_use]
pub fn peer_hex(p: &PeerId) -> String {
    p.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// 시계 오차 흡수 등록 세트 — 어제·오늘·내일. 상대가 어느 쪽 "오늘"이어도 겹친다.
#[must_use]
pub fn rids_around(peer: &PeerId) -> [Rid; 3] {
    let day = current_epoch_day();
    [
        rid_for(peer, day.saturating_sub(1)),
        rid_for(peer, day),
        rid_for(peer, day + 1),
    ]
}

// ── 와이어 = `nbeep-relay-proto`(v2) ───────────────────────────
//
// 제어 세션 프레임 = Envelope(kind u16 ‖ flags ‖ TLV). 모르는 핵심 kind는 Error{UnknownKind}로 **응답**하고
// 확장 kind는 무시한다(ADR-0016 §2-1). v1 `C2s`/`S2c`(kind u8 + 고정 오프셋)는 삭제 — 호환 포기(사용자 11-07).

/// 공개 카드 총 바이트 상한(서버·클라 공통 — 초과 = 카드 없는 것으로 취급).
pub const CARD_MAX: usize = 1024;

/// 공개 카드 인코딩(v1) — `[1][이름 u8][..][이메일 u8][..][소개 u16][..]`.
/// 각 필드는 UTF-8 그대로(빈 값 허용) · 총합이 [`CARD_MAX`]를 넘으면 소개부터 자른다.
#[must_use]
pub fn encode_card(name: &str, email: &str, bio: &str) -> Vec<u8> {
    fn cut(s: &str, max: usize) -> &str {
        // UTF-8 경계 보존 절단.
        if s.len() <= max {
            return s;
        }
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
    let name = cut(name, 255);
    let email = cut(email, 255);
    let bio = cut(
        bio,
        CARD_MAX
            .saturating_sub(6 + name.len() + email.len())
            .min(u16::MAX as usize),
    );
    let mut o = Vec::with_capacity(6 + name.len() + email.len() + bio.len());
    o.push(1);
    o.push(name.len() as u8);
    o.extend_from_slice(name.as_bytes());
    o.push(email.len() as u8);
    o.extend_from_slice(email.as_bytes());
    o.extend_from_slice(&(bio.len() as u16).to_be_bytes());
    o.extend_from_slice(bio.as_bytes());
    o
}

/// 공개 카드 디코딩 — 형식·UTF-8 오류·상한 초과는 `None`(fail-closed).
#[must_use]
pub fn decode_card(b: &[u8]) -> Option<(String, String, String)> {
    if b.is_empty() || b.len() > CARD_MAX || b[0] != 1 {
        return None;
    }
    let mut i = 1usize;
    let nlen = *b.get(i)? as usize;
    i += 1;
    let name = std::str::from_utf8(b.get(i..i + nlen)?).ok()?;
    i += nlen;
    let elen = *b.get(i)? as usize;
    i += 1;
    let email = std::str::from_utf8(b.get(i..i + elen)?).ok()?;
    i += elen;
    let blen = u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize;
    i += 2;
    let bio = std::str::from_utf8(b.get(i..i + blen)?).ok()?;
    Some((name.to_string(), email.to_string(), bio.to_string()))
}

/// 연결당 등록 가능한 RID 상한 **기본값**(에폭 3 + 여유) — 실제 상한은 `Welcome.limits.max_rids`(연결이 거기에 맞춰 자른다).
pub const MAX_RIDS: usize = 8;

fn ep_to_sock(ep: proto::Ep) -> Option<SocketAddr> {
    match ep {
        proto::Ep::None => None,
        proto::Ep::V4(ip, port) => Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port)),
        proto::Ep::V6(ip, port) => Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip)), port)),
    }
}

/// 핸드셰이크 뒤 알게 된 서버 정보(Welcome) — X-2b "타입 협상(auto)"의 재료.
#[derive(Clone, Debug)]
pub struct ServerInfo {
    /// 서버 버전 문자열(`CARGO_PKG_VERSION`).
    pub version: String,
    /// 서버 종류(relay/content/admin).
    pub server_type: proto::ServerType,
    /// 세션 능력(= 서버 ∩ 클라).
    pub caps: u64,
    /// 서버가 알린 상한.
    pub limits: proto::Limits,
    /// 세션 id(16B · 로그 대조용).
    pub session_id: [u8; 16],
    /// 프로토콜 minor(서버).
    pub proto_minor: u16,
}

// ── UDP 관측 프로브(STUN-lite) ──────────────────────────────────

/// 관측 프로브 송신 + 에코 수신 — 서버가 밖에서 본 내 UDP 엔드포인트를 돌려준다.
/// 같은 소켓을 홀펀칭에 써야 관측이 유효하다(NAT 매핑 = 로컬 포트·목적지 쌍).
///
/// # Errors
/// 송수신 실패·타임아웃·형식 오류 시 `io::Error`.
pub fn probe_udp(
    sock: &UdpSocket,
    server: SocketAddr,
    udp_token: u64,
    timeout: Duration,
) -> std::io::Result<SocketAddr> {
    sock.send_to(&proto::udp_obs::encode_probe(udp_token), server)?;
    sock.set_read_timeout(Some(timeout))?;
    let mut buf = [0u8; 64];
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let (n, from) = sock.recv_from(&mut buf)?;
        // 프로브 소켓엔 펀칭 SYN 등 다른 트래픽이 섞일 수 있다 — 매직·발신원·토큰으로 거른다.
        if from == server {
            if let Some((echo_token, ep)) = proto::udp_obs::parse_echo(&buf[..n]) {
                if echo_token == udp_token {
                    if let Some(sa) = ep_to_sock(ep) {
                        return Ok(sa);
                    }
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "관측 에코 없음",
            ));
        }
    }
}

// ── 클라이언트 ──────────────────────────────────────────────────

/// [`RelayClient::connect`] 실패.
#[derive(Debug)]
pub enum RelayError {
    /// TCP 연결·소켓 실패.
    Io(std::io::Error),
    /// 서버와의 Noise 핸드셰이크 실패.
    Handshake,
    /// 서버 키가 핀과 다르다(★ 시끄럽게 — DR-28 "신원이 바뀌면 시끄럽게").
    PinMismatch {
        /// 기대한(핀된) 서버 키.
        expected: PeerId,
        /// 실제 제시된 서버 키.
        got: PeerId,
    },
    /// 등록 응답 없음·형식 오류.
    Protocol,
    /// ★ 서버 **세대(major) 불일치**(ADR-0016) — 프리앰블이 알린 서버 major. `0` = 프리앰블 없음(v1 서버 또는 beepd가
    /// 아님). 표시 = "서버가 다른 세대 — 앱 업데이트 필요"(핀 불일치와 다르다 — 키는 그대로).
    VersionMismatch {
        /// 서버가 말한 major(0 = 모름).
        server_major: u16,
    },
    /// 서버가 Hello를 거절했다(`Reject{code, reason}` — 앱 이름공간 금지 등).
    Rejected {
        /// [`proto::ErrorCode`] 값.
        code: u16,
        /// 서버 사유(상태만 · 내용 없음).
        reason: String,
    },
}

impl From<std::io::Error> for RelayError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// 등록 결과 — 관측 정보(홀펀칭 재료).
#[derive(Clone, Copy, Debug)]
pub struct RegisterInfo {
    /// UDP 프로브에 실을 토큰.
    pub udp_token: u64,
    /// 서버 UDP 관측 포트.
    pub udp_port: u16,
    /// 서버가 본 내 공인 TCP 엔드포인트.
    pub observed_tcp: Option<SocketAddr>,
}

/// 인바운드 릴레이 채널 — 상대(`src` RID)가 서버 경유로 나를 열었다.
#[derive(Debug)]
pub struct RelayIncoming {
    /// 성립한 링크(위에 종단 Noise를 얹는다 — 서버는 당사자가 아니다).
    pub link: RelayLink,
    /// 여는 쪽의 회전 RID(누구인지는 내가 아는 공개키들로 역산 — [`rids_around`]).
    pub src: Rid,
    /// 여는 쪽의 관측 UDP 엔드포인트(홀펀칭 시도용 · X-UDP-c).
    pub peer_udp: Option<SocketAddr>,
}

impl RelayIncoming {
    /// 채널 수락 — **이때부터** 서버가 중계한다(지연 수락). 홀펀칭을 쓰려면 수락
    /// **전에** UDP 프로브를 마쳐야 상대의 OpenResult에 내 관측 엔드포인트가 실린다
    /// ([`accept_via`]가 이 순서를 지킨다). 수락 없이 drop = 서버가 열기 거절로 정리.
    pub fn accept(&self) {
        let _ = self.link.cmd_tx.send(Cmd::AcceptCh { ch: self.link.ch });
    }
}

type OpenResp = SyncSender<Result<(RelayLink, Option<SocketAddr>), u16>>;

/// [`RelayClient::open`] 실패 코드 — 서버 세션 죽음·시간 초과(서버 [`proto::ErrorCode`]와 겹치지 않는 값).
pub const OPEN_DEAD: u16 = 0xFFFF;

enum Cmd {
    Open {
        dst: Rid,
        resp: OpenResp,
    },
    Send {
        ch: u32,
        frame: Vec<u8>,
    },
    CloseCh {
        ch: u32,
    },
    /// 인바운드 채널 수락 통지 — [`RelayIncoming::accept`]가 보낸다(지연 수락:
    /// 수락 **전에** UDP 프로브를 마쳐야 서버가 내 관측 엔드포인트를 OpenResult에 싣는다).
    AcceptCh {
        ch: u32,
    },
    /// 클라이언트 종료 — 액터가 세션을 내려놓는다(서버가 TCP 종료로 정리).
    Shutdown,
    /// 생존 확인용 no-op([`RelayClient::is_alive`]) — 액터가 죽었으면 채널 send가
    /// 실패한다는 사실만 쓴다(액터는 아무 것도 하지 않는다).
    Nop,
    /// 프레즌스 공개 지정([`RelayClient::set_announce`] — X-2e roster).
    Announce {
        listed: bool,
        card: Vec<u8>,
    },
}

enum ChEvent {
    Data { fin: bool, bytes: Vec<u8> },
    Closed,
}

/// roster 델타(X-2e — [`RelayClient::poll_roster`]) — 같은 서버 **공개** 사용자의
/// 등장/이탈. 서버는 존재(공개키)만 나른다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RosterEvent {
    /// 공개 사용자 등장(입장 스냅숏 포함) — 둘째 항 = 공개 카드 바이트
    /// (08-22 · 빈 값 = 없음 · 해석은 [`decode_card`] — 카드 갱신도 이 이벤트).
    Up(PeerId, Vec<u8>),
    /// 공개 사용자 이탈(연결 종료·공개 해제).
    Down(PeerId),
}

/// 프리앰블 링크 — **첫 recv 전에** 서버 프리앰블 8B를 원시 스트림에서 읽는다. 클라가 프리앰블+Noise msg1을
/// 파이프라인으로 보냈으므로 서버도 프리앰블+msg2 순서로 답한다(RTT 추가 없음). 판별 결과는 `seen`에 남겨
/// [`RelayClient::connect`]가 Noise 실패를 "세대 불일치"와 "핸드셰이크 실패"로 가른다.
#[derive(Debug)]
struct PreLink {
    inner: TcpLink,
    raw: Option<TcpStream>,
    /// `None` = 아직 · `Some(None)` = 프리앰블 아님/EOF · `Some(Some(major))`.
    seen: Arc<Mutex<Option<Option<u16>>>>,
}

impl Link for PreLink {
    fn peer(&self) -> PeerId {
        self.inner.peer()
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        self.inner.send(frame)
    }
    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        if let Some(mut raw) = self.raw.take() {
            let mut b = [0u8; proto::preamble::LEN];
            let got = raw
                .read_exact(&mut b)
                .ok()
                .and_then(|()| proto::preamble::parse(&b));
            if let Ok(mut s) = self.seen.lock() {
                *s = Some(got);
            }
            if got != Some(proto::PROTO_MAJOR) {
                return Err(LinkError::Closed);
            }
        }
        self.inner.recv()
    }
    fn set_recv_timeout(&mut self, dur: Option<Duration>) -> Result<(), LinkError> {
        self.inner.set_recv_timeout(dur)
    }
    fn remote_ip(&self) -> Option<IpAddr> {
        self.inner.remote_ip()
    }
}

/// 릴레이 서버에 붙은 클라이언트 — 제어 세션(서버와의 Noise)을 액터 스레드가 소유하고,
/// 채널별 [`RelayLink`]가 그 위를 다중화한다.
#[derive(Debug)]
pub struct RelayClient {
    cmd_tx: Sender<Cmd>,
    /// 인바운드 큐 — Mutex는 스레드 공유용(Receiver가 !Sync · 수락 스레드가 Arc로 든다).
    incoming_rx: std::sync::Mutex<Receiver<RelayIncoming>>,
    /// roster 델타 큐(X-2e) — 호스트가 [`RelayClient::poll_roster`]로 드레인.
    roster_rx: std::sync::Mutex<Receiver<RosterEvent>>,
    server_peer: PeerId,
    server_addr: SocketAddr,
    reg: RegisterInfo,
    info: ServerInfo,
    /// 서버 `Goaway.retry_after_s`(초 · 0 = 받은 적 없음) — 액터가 세션을 내려놓기 직전에 적는다.
    goaway_retry_s: Arc<std::sync::atomic::AtomicU32>,
}

impl RelayClient {
    /// 서버 접속 → 프리앰블+Noise(prologue) → (핀 검증) → Hello/Welcome → RID 등록 → 액터 가동.
    /// `app = "nexa-beep"` · `app_version` = 이 크레이트(= 앱) 버전. 다른 앱은 [`Self::connect_as`].
    ///
    /// `expected_server`: 핀된 서버 키. `None` = 첫 접속(TOFU — 반환된
    /// [`Self::server_peer`]를 호출자가 핀에 저장한다). 불일치는 [`RelayError::PinMismatch`].
    ///
    /// # Errors
    /// 연결·핸드셰이크·세대 불일치·핀 불일치·거절·등록 실패 시 [`RelayError`].
    pub fn connect(
        server: SocketAddr,
        id: &Identity,
        rids: &[Rid],
        expected_server: Option<PeerId>,
    ) -> Result<Self, RelayError> {
        Self::connect_as(
            server,
            id,
            rids,
            expected_server,
            proto::app::BEEP,
            env!("CARGO_PKG_VERSION"),
            proto::BindMode::ExclusiveLastWins,
        )
    }

    /// [`Self::connect`]의 앱 이름공간·바인딩 모드 지정판(서버는 `(app, rid)`로 라우팅 — 교차 Open 불가 ·
    /// `Shared` = 같은 RID에 여러 기기 — 세션 능력 `SHARED_BIND`일 때만 성립).
    ///
    /// # Errors
    /// [`Self::connect`]와 같다.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)] // 핸드셰이크 4단계가 순차 왕복 하나다
    pub fn connect_as(
        server: SocketAddr,
        id: &Identity,
        rids: &[Rid],
        expected_server: Option<PeerId>,
        app: &str,
        app_version: &str,
        bind: proto::BindMode,
    ) -> Result<Self, RelayError> {
        let mut stream =
            TcpStream::connect_timeout(&server, Duration::from_secs(10)).map_err(RelayError::Io)?;
        // 프리앰블(평문 8B) — 바로 이어 Noise msg1을 보내 서버가 "프리앰블+msg2"로 답하게 한다.
        stream
            .write_all(&proto::preamble::encode(proto::PROTO_MAJOR))
            .map_err(RelayError::Io)?;
        let raw = stream.try_clone().map_err(RelayError::Io)?;
        let mut raw2 = stream.try_clone().map_err(RelayError::Io)?;
        let mut link = TcpLink::new(stream).map_err(RelayError::Io)?;
        // 침묵 서버가 접속을 영구 점유하지 못하게 — 핸드셰이크·등록 왕복 상한(소켓 옵션이라 raw에도 적용).
        let _ = link.set_recv_timeout(Some(Duration::from_secs(10)));
        let seen = Arc::new(Mutex::new(None));
        let pre = PreLink {
            inner: link,
            raw: Some(raw),
            seen: Arc::clone(&seen),
        };
        let mut session = match NoiseSession::initiate_with(pre, id, Some(proto::PROLOGUE)) {
            Ok(s) => s,
            Err(_) => {
                let mut seen = seen.lock().ok().and_then(|s| *s);
                if seen.is_none() {
                    // msg1 송신 자체가 실패했다(프리앰블을 모르는 서버가 먼저 끊어 RST) — 남은 바이트로 한 번 더 판별.
                    let _ = raw2.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut b = [0u8; proto::preamble::LEN];
                    seen = match raw2.read_exact(&mut b) {
                        Ok(()) => Some(proto::preamble::parse(&b)),
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::UnexpectedEof
                                    | std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::ConnectionAborted
                            ) =>
                        {
                            Some(None)
                        }
                        Err(_) => None,
                    };
                }
                return Err(match seen {
                    Some(Some(m)) if m != proto::PROTO_MAJOR => {
                        RelayError::VersionMismatch { server_major: m }
                    }
                    Some(None) => RelayError::VersionMismatch { server_major: 0 },
                    _ => RelayError::Handshake,
                });
            }
        };
        let server_peer = session.peer();
        if let Some(exp) = expected_server {
            if exp != server_peer {
                return Err(RelayError::PinMismatch {
                    expected: exp,
                    got: server_peer,
                });
            }
        }
        // Hello → Welcome | Reject.
        // 능력 — Shared 바인딩을 원하면 SHARED_BIND도 요청한다(서버가 없으면 RegisterResult가 Unsupported).
        let my_caps = CLIENT_CAPS
            | if bind == proto::BindMode::Shared {
                proto::caps::SHARED_BIND
            } else {
                0
            };
        let hello = proto::Hello {
            proto_major: proto::PROTO_MAJOR,
            proto_minor: proto::PROTO_MINOR,
            app: app.to_string(),
            app_version: app_version.to_string(),
            caps: my_caps,
        };
        session
            .send(&hello.encode().encode())
            .map_err(|_| RelayError::Protocol)?;
        let welcome = loop {
            let frame = session.recv().map_err(|_| RelayError::Protocol)?;
            let env = match Envelope::decode(&frame) {
                Ok(e) => e,
                Err(proto::DecodeError::UnknownExtKind(_)) => continue,
                Err(_) => return Err(RelayError::Protocol),
            };
            match env.kind() {
                Some(Kind::Welcome) => {
                    break proto::Welcome::decode(&env).map_err(|_| RelayError::Protocol)?;
                }
                Some(Kind::Reject) => {
                    let p = Problem::decode(&env).map_err(|_| RelayError::Protocol)?;
                    return Err(RelayError::Rejected {
                        code: p.code,
                        reason: p.reason,
                    });
                }
                _ => return Err(RelayError::Protocol),
            }
        };
        if welcome.proto_major != proto::PROTO_MAJOR {
            return Err(RelayError::VersionMismatch {
                server_major: welcome.proto_major,
            });
        }
        let limits = welcome.limits;
        // 등록 — RID 수는 서버 상한에 맞춰 자른다(하드코딩 제거 · 기기 RID 3이 앞에 있다).
        let mut rids = rids.to_vec();
        rids.truncate(usize::from(limits.max_rids.max(1)));
        session
            .send(&proto::Register { rids, bind }.encode().encode())
            .map_err(|_| RelayError::Protocol)?;
        let rr = loop {
            let frame = session.recv().map_err(|_| RelayError::Protocol)?;
            let env = match Envelope::decode(&frame) {
                Ok(e) => e,
                Err(proto::DecodeError::UnknownExtKind(_)) => continue,
                Err(_) => return Err(RelayError::Protocol),
            };
            match env.kind() {
                Some(Kind::RegisterResult) => {
                    break proto::RegisterResult::decode(&env).map_err(|_| RelayError::Protocol)?;
                }
                Some(Kind::Error | Kind::Reject | Kind::GoawayS) => {
                    return Err(RelayError::Protocol)
                }
                _ => continue, // 이른 명부 델타 등은 버린다(등록 전)
            }
        };
        if !rr.entries.iter().any(|(_, st)| *st == proto::STATUS_OK) {
            return Err(RelayError::Protocol); // 어느 RID도 안 잡혔다 = 랑데부 불가
        }
        let reg = RegisterInfo {
            udp_token: rr.udp_token,
            udp_port: welcome.udp_port,
            observed_tcp: ep_to_sock(welcome.observed),
        };
        let info = ServerInfo {
            version: welcome.server_version,
            server_type: welcome.server_type,
            caps: welcome.caps & my_caps,
            limits,
            session_id: welcome.session_id,
            proto_minor: welcome.proto_minor,
        };
        let max_frame = usize::try_from(limits.max_frame).unwrap_or(MAX_FRAME);
        let ping_every = Duration::from_secs(u64::from(limits.ping_interval_s.clamp(5, 300)));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel::<RelayIncoming>();
        let (roster_tx, roster_rx) = std::sync::mpsc::channel::<RosterEvent>();
        let actor_cmd = cmd_tx.clone();
        let goaway_retry_s = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let goaway_w = Arc::clone(&goaway_retry_s);
        std::thread::Builder::new()
            .name("relay-client".into())
            .spawn(move || {
                actor(
                    session,
                    ActorCfg {
                        max_frame,
                        ping_every,
                    },
                    &cmd_rx,
                    &actor_cmd,
                    &incoming_tx,
                    &roster_tx,
                    &goaway_w,
                );
            })
            .map_err(RelayError::Io)?;
        Ok(Self {
            cmd_tx,
            incoming_rx: std::sync::Mutex::new(incoming_rx),
            roster_rx: std::sync::Mutex::new(roster_rx),
            server_peer,
            server_addr: server,
            reg,
            info,
            goaway_retry_s,
        })
    }

    /// 서버의 신원 키(= TOFU 핀 대상). 첫 접속이면 호출자가 저장한다.
    #[must_use]
    pub fn server_peer(&self) -> PeerId {
        self.server_peer
    }

    /// 서버 주소.
    #[must_use]
    pub fn server_addr(&self) -> SocketAddr {
        self.server_addr
    }

    /// 등록 결과(UDP 토큰·관측 주소).
    #[must_use]
    pub fn register_info(&self) -> RegisterInfo {
        self.reg
    }

    /// Welcome이 알린 서버 정보(버전·종류·능력·상한).
    #[must_use]
    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// 서버가 `Goaway{retry_after_s}`로 내려보낸 재접속 힌트(우아한 종료·재배포). 세션이 죽은 뒤 호스트가
    /// 백오프 첫 단계 대신 이 값을 쓴다. 힌트 없이 죽었으면 `None`.
    #[must_use]
    pub fn goaway_retry_after(&self) -> Option<Duration> {
        match self
            .goaway_retry_s
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            0 => None,
            s => Some(Duration::from_secs(u64::from(s))),
        }
    }

    /// `dst` RID로 채널을 연다. 성립 시 (링크, 상대 관측 UDP).
    ///
    /// # Errors
    /// 서버 상태 코드([`proto::ErrorCode`] 값 — `NoSuchPeer`=대상 없음 · `LimitExceeded`/`Busy`=상한·거절) ·
    /// [`OPEN_DEAD`]=세션 죽음/시간 초과.
    pub fn open(
        &self,
        dst: Rid,
        timeout: Duration,
    ) -> Result<(RelayLink, Option<SocketAddr>), u16> {
        let (resp_tx, resp_rx) = std::sync::mpsc::sync_channel(1);
        self.cmd_tx
            .send(Cmd::Open { dst, resp: resp_tx })
            .map_err(|_| OPEN_DEAD)?;
        match resp_rx.recv_timeout(timeout) {
            Ok(mut r) => {
                if let Ok((link, _)) = &mut r {
                    link.set_server_ip(self.server_addr.ip());
                }
                r
            }
            Err(_) => Err(OPEN_DEAD),
        }
    }

    /// 액터(서버 제어 세션) 생존 여부 — 서버 세션이 죽으면 액터가 종료해 명령 채널이
    /// 닫힌다는 사실을 쓴다. 재접속 판단용(GUI 서버 틱 · 폴링 부담 0).
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.cmd_tx.send(Cmd::Nop).is_ok()
    }

    /// 프레즌스 공개 지정(X-2e roster · 옵트인 — [docs/32 §12-7]). `true` = 같은
    /// 서버·같은 앱의 공개 목록에 실리고 그 목록의 델타를 받기 시작한다(입장 스냅숏 포함).
    pub fn set_announce(&self, listed: bool, card: Vec<u8>) {
        let _ = self.cmd_tx.send(Cmd::Announce { listed, card });
    }

    /// 쌓인 roster 델타를 전부 꺼낸다(논블로킹 — 호스트 틱이 주기 드레인).
    #[must_use]
    pub fn poll_roster(&self) -> Vec<RosterEvent> {
        let mut out = Vec::new();
        if let Ok(rx) = self.roster_rx.lock() {
            while let Ok(ev) = rx.try_recv() {
                out.push(ev);
            }
        }
        out
    }

    /// 인바운드 하나를 꺼낸다(서버 IP 배선 포함 — 경로 등급 판정용). 상대가 서버
    /// 랑데부로 나를 열면 여기로 온다. **지연 수락** — 소비자가 [`RelayIncoming::accept`]
    /// (또는 [`accept_via`])를 불러야 서버가 중계를 시작한다.
    #[must_use]
    pub fn accept_incoming(&self, timeout: Duration) -> Option<RelayIncoming> {
        let rx = self.incoming_rx.lock().ok()?;
        match rx.recv_timeout(timeout) {
            Ok(mut inc) => {
                inc.link.set_server_ip(self.server_addr.ip());
                Some(inc)
            }
            Err(_) => None,
        }
    }
}

impl Drop for RelayClient {
    fn drop(&mut self) {
        // 액터에 종료 지시 — 세션이 내려가면 서버가 내 RID·채널을 정리하고
        // 상대들에게 ChClosed를 전파한다(유령 채널 방지).
        let _ = self.cmd_tx.send(Cmd::Shutdown);
    }
}

/// 액터 매개변수(Welcome.limits에서).
#[derive(Clone, Copy, Debug)]
struct ActorCfg {
    max_frame: usize,
    ping_every: Duration,
}

fn ch_msg(kind: Kind, ch: u32) -> Vec<u8> {
    let mut c = Channel::empty();
    c.ch = Some(ch);
    c.encode(kind).encode()
}

/// 제어 세션 액터 — 세션의 단일 소유자(수신 폴 + 송신 채널 드레인 교대 · 세션 액터 규약).
#[allow(clippy::too_many_lines)] // 제어 프로토콜 분기 전체가 한 루프다
fn actor(
    mut session: NoiseSession<PreLink>,
    cfg: ActorCfg,
    cmd_rx: &Receiver<Cmd>,
    cmd_tx: &Sender<Cmd>,
    incoming_tx: &Sender<RelayIncoming>,
    roster_tx: &Sender<RosterEvent>,
    goaway_retry_s: &std::sync::atomic::AtomicU32,
) {
    session.set_recv_timeout(Some(Duration::from_millis(15)));
    let mut chans: HashMap<u32, Sender<ChEvent>> = HashMap::new();
    let mut pending: Vec<(u32, OpenResp)> = Vec::new();
    let mut next_token = 1u32;
    let mut nonce = 0u64;
    let mut last_ping = std::time::Instant::now();
    // 조각 상한 — 봉투 머리·TLV 여유를 뺀 서버 프레임 상한 안쪽.
    let chunk = RELAY_CHUNK.min(cfg.max_frame.saturating_sub(64).max(1024));
    loop {
        // 1) 명령 드레인.
        let mut dead = false;
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Open { dst, resp } => {
                    let token = next_token;
                    next_token = next_token.wrapping_add(1);
                    let mut c = Channel::empty();
                    c.token = Some(token);
                    c.rid = Some(dst);
                    if session.send(&c.encode(Kind::Open).encode()).is_err() {
                        let _ = resp.try_send(Err(OPEN_DEAD));
                        dead = true;
                        break;
                    }
                    pending.push((token, resp));
                }
                Cmd::Send { ch, frame } => {
                    // 큰 프레임은 조각으로 — 제어 세션 페이로드 상한 안쪽.
                    let mut rest = frame.as_slice();
                    loop {
                        let take = rest.len().min(chunk);
                        let (piece, tail) = rest.split_at(take);
                        let mut c = Channel::empty();
                        c.ch = Some(ch);
                        c.fin = tail.is_empty();
                        c.bytes = piece.to_vec();
                        if session.send(&c.encode(Kind::Data).encode()).is_err() {
                            dead = true;
                            break;
                        }
                        if tail.is_empty() {
                            break;
                        }
                        rest = tail;
                    }
                }
                Cmd::CloseCh { ch } => {
                    chans.remove(&ch);
                    let _ = session.send(&ch_msg(Kind::CloseCh, ch));
                }
                Cmd::AcceptCh { ch } => {
                    if session.send(&ch_msg(Kind::Accept, ch)).is_err() {
                        dead = true;
                        break;
                    }
                }
                Cmd::Shutdown => {
                    // 우아한 종료 통지(서버가 즉시 정리) — 실패해도 TCP 종료가 같은 결과.
                    let _ = session.send(
                        &Problem::new(proto::ErrorCode::Shutdown, "client shutdown")
                            .encode(Kind::Goaway)
                            .encode(),
                    );
                    dead = true;
                    break;
                }
                Cmd::Nop => {} // 생존 확인 — send 성공 자체가 답이다
                Cmd::Announce { listed, card } => {
                    let p = Presence {
                        listed,
                        peer: None,
                        rid: None,
                        card,
                    };
                    if session.send(&p.encode(Kind::Announce).encode()).is_err() {
                        dead = true;
                        break;
                    }
                }
            }
        }
        if dead {
            break;
        }
        // 2) 생존 신호(Welcome.ping_interval_s) — 서버 유휴 정리·NAT 타임아웃 방지.
        if last_ping.elapsed() >= cfg.ping_every {
            last_ping = std::time::Instant::now();
            nonce = nonce.wrapping_add(1);
            if session
                .send(&proto::ping(Kind::Ping, nonce).encode())
                .is_err()
            {
                break;
            }
        }
        // 3) 세션 수신(15ms 폴 — 명령 드레인과 교대).
        let frame = match session.recv() {
            Ok(f) => f,
            Err(SessionError::TimedOut) => continue,
            Err(_) => break, // 서버 세션 죽음 — 아래 정리로
        };
        let env = match Envelope::decode(&frame) {
            Ok(e) => e,
            Err(proto::DecodeError::UnknownCoreKind(k)) => {
                // 모르는 핵심 kind = 응답(조용히 버리지 않는다 · ADR-0016).
                let mut p = Problem::new(proto::ErrorCode::UnknownKind, "unknown kind");
                p.ref_kind = Some(k);
                if session.send(&p.encode(Kind::Error).encode()).is_err() {
                    break;
                }
                continue;
            }
            Err(proto::DecodeError::UnknownExtKind(_)) => continue,
            Err(_) => break, // 서버가 깨진 봉투를 보냈다 — fail-closed(재접속)
        };
        let Some(kind) = env.kind() else { continue };
        match kind {
            Kind::DataS => {
                let Ok(c) = Channel::decode(&env) else { break };
                let Some(ch) = c.ch else { continue };
                if let Some(tx) = chans.get(&ch) {
                    if tx
                        .send(ChEvent::Data {
                            fin: c.fin,
                            bytes: c.bytes,
                        })
                        .is_err()
                    {
                        chans.remove(&ch); // 링크가 버려짐 — 서버에도 닫기 통지
                        let _ = session.send(&ch_msg(Kind::CloseCh, ch));
                    }
                }
            }
            Kind::Incoming => {
                // ★ 여기서 Accept를 **보내지 않는다**(지연 수락) — 소비자가
                // [`RelayIncoming::accept`]를 불러야 서버가 중계를 시작한다.
                // 수락 전에 UDP 프로브를 마치면 서버의 OpenResult가 내 신선한
                // 관측 엔드포인트를 상대에게 싣는다(홀펀칭 재료의 순서 보장).
                let Ok(c) = Channel::decode(&env) else { break };
                let (Some(ch), Some(src)) = (c.ch, c.rid) else {
                    continue;
                };
                let (tx, rx) = std::sync::mpsc::channel();
                chans.insert(ch, tx);
                let link = RelayLink::new(ch, cmd_tx.clone(), rx, cfg.max_frame);
                if incoming_tx
                    .send(RelayIncoming {
                        link,
                        src,
                        peer_udp: ep_to_sock(c.ep),
                    })
                    .is_err()
                {
                    // 소유자가 인바운드 수신단을 버렸다 — 채널만 정리.
                    chans.remove(&ch);
                    let _ = session.send(&ch_msg(Kind::CloseCh, ch));
                }
            }
            Kind::OpenResult => {
                let Ok(c) = Channel::decode(&env) else { break };
                let Some(token) = c.token else { continue };
                if let Some(pos) = pending.iter().position(|(t, _)| *t == token) {
                    let (_, resp) = pending.swap_remove(pos);
                    let status = c.status.unwrap_or(proto::STATUS_OK);
                    match (status, c.ch) {
                        (proto::STATUS_OK, Some(ch)) => {
                            let (tx, rx) = std::sync::mpsc::channel();
                            chans.insert(ch, tx);
                            let link = RelayLink::new(ch, cmd_tx.clone(), rx, cfg.max_frame);
                            let _ = resp.try_send(Ok((link, ep_to_sock(c.ep))));
                        }
                        (proto::STATUS_OK, None) => {
                            let _ = resp.try_send(Err(proto::ErrorCode::Malformed as u16));
                        }
                        (st, _) => {
                            let _ = resp.try_send(Err(st));
                        }
                    }
                }
            }
            Kind::ChClosed => {
                let Ok(c) = Channel::decode(&env) else { break };
                if let Some(tx) = c.ch.and_then(|ch| chans.remove(&ch)) {
                    let _ = tx.send(ChEvent::Closed);
                }
            }
            Kind::PeerUp | Kind::PeerDown => {
                let Ok(p) = Presence::decode(&env) else { break };
                let Some(peer) = p.peer else { continue };
                let peer = PeerId::from_bytes(peer);
                let _ = roster_tx.send(if kind == Kind::PeerUp {
                    RosterEvent::Up(peer, p.card)
                } else {
                    RosterEvent::Down(peer)
                });
            }
            Kind::GoawayS => {
                // 서버 우아한 종료 — 재접속 힌트를 남기고 내려간다(호스트가 백오프 대신 쓴다 · 0초 힌트는 1초로).
                if let Ok(p) = Problem::decode(&env) {
                    if let Some(s) = p.retry_after_s {
                        goaway_retry_s
                            .store(u32::from(s.max(1)), std::sync::atomic::Ordering::Relaxed);
                    }
                }
                break;
            }
            // Pong·늦은 Welcome/RegisterResult·Error(서버 통지 — 상태만)·방향이 틀린 kind는 버린다.
            _ => {}
        }
    }
    // 세션 죽음 = 전 채널 종료 통지(수신자들이 Closed를 본다).
    for (_, tx) in chans.drain() {
        let _ = tx.send(ChEvent::Closed);
    }
    for (_, resp) in pending.drain(..) {
        let _ = resp.try_send(Err(OPEN_DEAD));
    }
}

/// 릴레이 채널 하나 = [`Link`] — 이 위에 **종단** Noise·mux·전송이 코드 무변경으로 얹힌다
/// (DR-21 · [docs/32 §13] C-1). 서버는 이 링크의 내용(조각난 종단 암호문)을 열 수 없다.
#[derive(Debug)]
pub struct RelayLink {
    ch: u32,
    cmd_tx: Sender<Cmd>,
    rx: Receiver<ChEvent>,
    /// 조각 조립 버퍼(fin까지 누적).
    buf: Vec<u8>,
    closed: bool,
    recv_timeout: Option<Duration>,
    /// 경로 등급 판정용 서버 IP(릴레이 경유 = 서버 주소가 실소켓 상대).
    server_ip: Option<IpAddr>,
    /// 조립 프레임 상한(Welcome.limits.max_frame).
    max_frame: usize,
}

impl RelayLink {
    fn new(ch: u32, cmd_tx: Sender<Cmd>, rx: Receiver<ChEvent>, max_frame: usize) -> Self {
        Self {
            ch,
            cmd_tx,
            rx,
            buf: Vec::new(),
            closed: false,
            recv_timeout: None,
            server_ip: None,
            max_frame,
        }
    }

    /// 경로 등급 판정용 서버 IP 지정([`Link::remote_ip`]가 이 값을 낸다).
    pub fn set_server_ip(&mut self, ip: IpAddr) {
        self.server_ip = Some(ip);
    }

    fn next_event(&mut self) -> Result<ChEvent, LinkError> {
        match self.recv_timeout {
            Some(t) => self.rx.recv_timeout(t).map_err(|e| match e {
                std::sync::mpsc::RecvTimeoutError::Timeout => LinkError::TimedOut,
                std::sync::mpsc::RecvTimeoutError::Disconnected => LinkError::Closed,
            }),
            None => self.rx.recv().map_err(|_| LinkError::Closed),
        }
    }
}

impl Link for RelayLink {
    fn peer(&self) -> PeerId {
        // 릴레이 수준에서 상대 신원은 알 수 없다 — 신원은 종단 핸드셰이크가 확정한다.
        PeerId::from_bytes([0u8; 32])
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        if self.closed || frame.len() > self.max_frame {
            return Err(LinkError::Closed);
        }
        self.cmd_tx
            .send(Cmd::Send {
                ch: self.ch,
                frame: frame.to_vec(),
            })
            .map_err(|_| LinkError::Closed)
    }

    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        if self.closed {
            return Err(LinkError::Closed);
        }
        loop {
            match self.next_event()? {
                ChEvent::Data { fin, bytes } => {
                    if self.buf.len() + bytes.len() > self.max_frame {
                        self.closed = true; // 프레임 상한 위반 = 프로토콜 오류(fail-closed)
                        return Err(LinkError::Closed);
                    }
                    self.buf.extend_from_slice(&bytes);
                    if fin {
                        return Ok(core::mem::take(&mut self.buf));
                    }
                }
                ChEvent::Closed => {
                    self.closed = true;
                    return Err(LinkError::Closed);
                }
            }
        }
    }

    fn set_recv_timeout(&mut self, dur: Option<Duration>) -> Result<(), LinkError> {
        self.recv_timeout = dur;
        Ok(())
    }

    fn remote_ip(&self) -> Option<IpAddr> {
        // 릴레이 경유 = 실소켓 상대는 서버다 — 경로 등급이 Local로 오판되지 않게
        // 서버 주소를 낸다(ADR-0006 §5-1-5 · fail-closed 방향).
        self.server_ip
    }
}

impl Drop for RelayLink {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Cmd::CloseCh { ch: self.ch });
    }
}

// ── 경로 사다리(X-UDP-e — 원격 구간) ────────────────────────────
//
// S-1 사다리의 원격 구간: **홀펀칭 UDP 직결 → 실패 시 릴레이 폴백**. LAN 직접은
// 발견(LocalDirect)이 담당하고, 여기는 "서버 랑데부로 닿는 상대"만 다룬다.
// 각 단이 독립 핸드셰이크라 한 단의 불발이 다음 단을 오염시키지 않는다(fail-open
// to next rung · 데이터는 언제나 종단 암호문 = fail-closed on data).

/// 펀치 동시 열기 창 — 양쪽이 랑데부 직후 시작하므로 왕복 몇 번이면 충분하다.
const PUNCH_WINDOW: Duration = Duration::from_secs(3);
/// 종단 핸드셰이크 대기(UDP 단 — 실패 시 릴레이로 내려간다).
const HS_TIMEOUT_UDP: Duration = Duration::from_secs(5);
/// 종단 핸드셰이크 대기(릴레이 단 — 마지막 단이라 넉넉히).
const HS_TIMEOUT_RELAY: Duration = Duration::from_secs(10);

/// 사다리의 어느 단으로 성립했나(표시·계측용 — 정책은 [`nbeep_core::PathClass`]가 담당).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathTaken {
    /// 홀펀칭 UDP 직결(서버는 경로에서 빠졌다).
    Udp,
    /// 릴레이 폴백(서버가 종단 암호문을 나른다).
    Relay,
}

/// [`connect_via`]/[`accept_via`] 결과 — 인증된 종단 세션 + 경로 정보.
pub struct ViaSession {
    /// 성립한 종단 Noise 세션(상대 키 인증됨).
    pub session: NoiseSession<Box<dyn Link>>,
    /// 탄 사다리 단.
    pub taken: PathTaken,
    /// 경로 등급 재료(성립 소켓 실주소 판정 — ADR-0006 §5-1-5).
    pub path: nbeep_core::PathClass,
    /// ★ XXpsk3로 성립했는가(ADR-0015 · 상대가 같은 KP를 안다 = 같은 사용자).
    pub via_psk: bool,
}

impl core::fmt::Debug for ViaSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ViaSession")
            .field("taken", &self.taken)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// 사다리 성립 실패.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViaError {
    /// 어느 에폭 RID로도 대상 없음(상대가 그 서버에 없다).
    NotFound,
    /// 서버 상한·거절.
    Limit,
    /// 서버 세션 죽음·시간 초과.
    Dead,
    /// 종단 핸드셰이크 실패(모든 단에서).
    Handshake,
    /// 인증된 상대가 기대한 키와 다르다 — **연결을 쓰지 않는다**(fail-closed).
    WrongPeer,
}

/// 신선한 펀치 소켓 — 바인드 + 서버 관측 프로브(NAT 매핑 개방 + 서버가 내 공인
/// 엔드포인트를 기록). **시도마다 새로 만든다** — 관측은 (로컬 포트, 목적지) 매핑에
/// 붙으므로 오래된 소켓의 관측은 신뢰할 수 없고, 소켓 공유는 recv 경합을 만든다.
fn fresh_probed_sock(client: &RelayClient) -> std::io::Result<UdpSocket> {
    if client.server_info().caps & proto::caps::UDP_OBSERVE == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "서버가 UDP 관측을 지원하지 않는다",
        ));
    }
    let server = client.server_addr();
    let bind: SocketAddr = if server.is_ipv4() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
    };
    let sock = UdpSocket::bind(bind)?;
    let server_udp = SocketAddr::new(server.ip(), client.register_info().udp_port);
    probe_udp(
        &sock,
        server_udp,
        client.register_info().udp_token,
        Duration::from_secs(2),
    )?;
    Ok(sock)
}

/// 먼저 뽑아 둔 프레임 하나를 되돌려주는 링크 래퍼 — 수락 측이 "어느 링크로 첫
/// 프레임이 오나"를 보고 단을 고른 뒤, 그 프레임을 핸드셰이크에 되살린다. **랑데부 쪽에서만** 쓰이므로
/// [`Link::rendezvous`] = `true`(X-2 승격 11-06 — 호스트가 LAN 인바운드와 같은 펌프에서 배지만 가른다).
struct PrefixedLink {
    first: Option<Vec<u8>>,
    inner: Box<dyn Link>,
}

impl core::fmt::Debug for PrefixedLink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PrefixedLink")
            .field("first", &self.first.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

impl Link for PrefixedLink {
    fn peer(&self) -> PeerId {
        self.inner.peer()
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        self.inner.send(frame)
    }
    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        if let Some(f) = self.first.take() {
            return Ok(f);
        }
        self.inner.recv()
    }
    fn set_recv_timeout(&mut self, dur: Option<Duration>) -> Result<(), LinkError> {
        self.inner.set_recv_timeout(dur)
    }
    fn remote_ip(&self) -> Option<IpAddr> {
        self.inner.remote_ip()
    }
    fn rendezvous(&self) -> bool {
        true
    }
}

fn class_of_link(link: &dyn Link, fallback: nbeep_core::PathClass) -> nbeep_core::PathClass {
    link.remote_ip().map_or(fallback, nbeep_core::class_of_ip)
}

/// **여는 쪽 사다리** — `dst`(상대 공개키)로 서버 랑데부 → 홀펀칭 시도 → 릴레이 폴백.
/// 에폭 RID는 오늘→어제→내일 순으로 시도한다(시계 오차 흡수).
///
/// 성립한 세션의 `peer()`가 `dst`와 다르면 [`ViaError::WrongPeer`] — RID는 힌트일 뿐,
/// **근거는 언제나 암호학적 증거**(핸드셰이크가 확정한 키)다.
///
/// # Errors
/// 대상 없음·상한·서버 죽음·핸드셰이크 실패·상대 불일치 시 [`ViaError`].
pub fn connect_via(
    client: &RelayClient,
    id: &Identity,
    dst: &PeerId,
    punch: bool,
    open_timeout: Duration,
    psk: Option<&[u8; 32]>,
) -> Result<ViaSession, ViaError> {
    let day = current_epoch_day();
    let rids = [
        rid_for(dst, day),
        rid_for(dst, day.saturating_sub(1)),
        rid_for(dst, day + 1),
    ];
    connect_via_rids(client, id, &rids, punch, open_timeout, psk, Some(dst))
}

/// ★ **첫 만남**(ADR-0015 §3-4) — 상대 `PeerId`를 모르는 채 사용자 페어링 RID(`RID_pair` 3개)로
/// 연다. 키 대조 대신 **PSK 성립**이 근거다(형제 기기 = 같은 KP). 세션의 `peer()`가 상대 신원.
///
/// # Errors
/// [`connect_via`]와 같다.
pub fn connect_via_rids_first(
    client: &RelayClient,
    id: &Identity,
    rids: &[Rid],
    punch: bool,
    open_timeout: Duration,
    psk: &[u8; 32],
) -> Result<ViaSession, ViaError> {
    connect_via_rids(client, id, rids, punch, open_timeout, Some(psk), None)
}

/// 종단 개시 — psk가 있으면 XXpsk3(형제), 없으면 XX.
fn hs_initiate(
    boxed: Box<dyn Link>,
    id: &Identity,
    psk: Option<&[u8; 32]>,
) -> Result<NoiseSession<Box<dyn Link>>, nbeep_core::session::SessionError> {
    match psk {
        Some(k) => NoiseSession::initiate_psk(boxed, id, k),
        None => NoiseSession::initiate(boxed, id),
    }
}

fn connect_via_rids(
    client: &RelayClient,
    id: &Identity,
    rids: &[Rid],
    punch: bool,
    open_timeout: Duration,
    psk: Option<&[u8; 32]>,
    expect: Option<&PeerId>,
) -> Result<ViaSession, ViaError> {
    // ★ X-2 승격(11-06): 단 선택은 [`LadderLink`]가 **링크 수준**에서 한다(첫 회신이 온 단에 고정) — 핸드셰이크는 그 위.
    //   종전(08-22)은 "UDP 핸드셰이크 실패 → 릴레이로 새 핸드셰이크"를 여기서 직접 했다. 바깥 동작은 같다
    //   (UDP 상한 5s → 릴레이 상한 10s · 실패 사유 매핑 동일) — `Transport::connect`도 같은 사다리를 쓴다.
    let ladder = open_ladder(client, rids, punch, open_timeout)?;
    let taken = ladder.taken_handle();
    let (udp_ip, server_ip) = ladder.candidate_ips();
    let boxed: Box<dyn Link> = Box::new(ladder);
    let session = hs_initiate(boxed, id, psk).map_err(|_| ViaError::Handshake)?;
    // 성립 뒤 어느 단에 고정됐나 — 경로 등급은 그 단의 실주소(§5-1-5 · 서버 미상은 Remote · fail-closed).
    let (taken, ip) = match taken.load(std::sync::atomic::Ordering::Acquire) {
        LADDER_UDP => (PathTaken::Udp, udp_ip),
        _ => (PathTaken::Relay, Some(server_ip)),
    };
    let path = ip.map_or(nbeep_core::PathClass::Remote, nbeep_core::class_of_ip);
    finish_via(session, taken, path, expect, psk.is_some())
}

/// 사다리 단 표식([`LadderLink`]가 공유 원자값으로 알린다 — 핸드셰이크가 링크를 삼킨 뒤에도 읽을 수 있게).
const LADDER_PENDING: u8 = 0;
const LADDER_UDP: u8 = 1;
const LADDER_RELAY: u8 = 2;

/// **여는 쪽 사다리를 링크 수준에서 연다**(X-2 Transport 승격 · 11-06) — 프로브 → 에폭 RID 순차 Open →
/// (상대 관측이 있으면) 동시 펀치. 결과 [`LadderLink`]는 **핸드셰이크 전**이다 — 단 고정은 첫 회신이 정한다.
///
/// # Errors
/// 어느 에폭 RID도 없음 = [`ViaError::NotFound`] · 서버 상한 = [`ViaError::Limit`] · 서버 세션 죽음/시간 초과 = [`ViaError::Dead`].
pub(crate) fn open_ladder(
    client: &RelayClient,
    rids: &[Rid],
    punch: bool,
    open_timeout: Duration,
) -> Result<LadderLink, ViaError> {
    // 프로브가 Open보다 **먼저** — 서버가 내 관측 엔드포인트를 상대의 Incoming에 싣는다.
    let sock = if punch {
        fresh_probed_sock(client).ok() // 실패 = 펀치 없이 릴레이만(사다리 한 단 생략)
    } else {
        None
    };
    for rid in rids {
        let (relay, peer_udp) = match client.open(*rid, open_timeout) {
            Ok(v) => v,
            Err(c) if c == proto::ErrorCode::NoSuchPeer as u16 => continue, // 이 에폭 RID는 미등록 — 다음 에폭
            Err(c)
                if c == proto::ErrorCode::LimitExceeded as u16
                    || c == proto::ErrorCode::Busy as u16 =>
            {
                return Err(ViaError::Limit)
            }
            Err(_) => return Err(ViaError::Dead),
        };
        // 1단 후보: 홀펀칭 UDP — 펀치 성립(상대 패킷 도착)까지. 실패 = 1단 없음(2단만).
        let udp = match (&sock, peer_udp) {
            (Some(s), Some(ep)) => s
                .try_clone()
                .ok()
                .and_then(|sc| nbeep_net::UdpLink::punch(sc, ep, PUNCH_WINDOW).ok()),
            _ => None,
        };
        return Ok(LadderLink::new(udp, relay, client.server_addr().ip()));
    }
    Err(ViaError::NotFound)
}

/// 여는 쪽 사다리 **링크**(X-2 승격 · 11-06) — 1단(홀펀칭 UDP 직결)과 2단(릴레이 중계)을 하나의 [`Link`]로 묶어
/// **첫 회신이 온 단에 고정**한다(수락 쪽 "첫 프레임 링크 추종"의 거울). 핸드셰이크는 이 위에서 호출자가 한다.
///
/// 규칙: 확정 전 `send`는 1단으로 나가며 사본을 든다(2단 재전송용 — 보통 핸드셰이크 msg1 하나) · 확정 전 `recv`가
/// 1단에서 실패(상한 초과·종료 = 상대가 UDP를 듣지 않았다)하면 사본을 2단으로 다시 보내고 2단에 고정한다 · 1단이
/// 없으면(관측 없음·펀치 실패) 첫 `send`에서 곧장 2단 고정. 고정 뒤 다른 단은 버린다. 확정 전 수신 상한은 단별
/// 핸드셰이크 상한(`HS_TIMEOUT_UDP` 5s / `HS_TIMEOUT_RELAY` 10s)이고, 소비자가 건 타임아웃은 고정 뒤 그 단에 적용된다.
pub struct LadderLink {
    /// 1단 후보(펀치 성립) — 확정 전 실패하거나 2단 고정 뒤 버린다.
    udp: Option<nbeep_net::UdpLink>,
    /// 2단(릴레이 채널) — 1단 고정 뒤 버린다.
    relay: Option<RelayLink>,
    /// 고정된 단(`LADDER_*`) — 핸드셰이크 뒤 호출자가 읽을 수 있게 공유.
    taken: std::sync::Arc<std::sync::atomic::AtomicU8>,
    /// 확정 전 1단으로 보낸 프레임 사본(2단 재전송용).
    pending: Vec<Vec<u8>>,
    /// 소비자가 건 수신 타임아웃(고정 뒤 그 단에 적용).
    consumer_timeout: Option<Duration>,
    /// 경로 등급 판정용 서버 IP — 2단 실소켓 상대 · 확정 전 `remote_ip`의 fail-closed 값.
    server_ip: IpAddr,
}

impl core::fmt::Debug for LadderLink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LadderLink")
            .field("taken", &self.state())
            .field("udp", &self.udp.is_some())
            .field("relay", &self.relay.is_some())
            .finish_non_exhaustive()
    }
}

impl LadderLink {
    fn new(udp: Option<nbeep_net::UdpLink>, relay: RelayLink, server_ip: IpAddr) -> Self {
        Self {
            udp,
            relay: Some(relay),
            taken: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(LADDER_PENDING)),
            pending: Vec::new(),
            consumer_timeout: None,
            server_ip,
        }
    }

    /// 단 표식 공유 핸들 — 핸드셰이크가 링크를 삼킨 뒤 어느 단이었나를 읽는다(`connect_via` 호환 · 계측).
    fn taken_handle(&self) -> std::sync::Arc<std::sync::atomic::AtomicU8> {
        std::sync::Arc::clone(&self.taken)
    }

    /// (1단 상대 IP, 서버 IP) — 고정 뒤 경로 등급을 매기는 재료.
    fn candidate_ips(&self) -> (Option<IpAddr>, IpAddr) {
        (self.udp.as_ref().and_then(Link::remote_ip), self.server_ip)
    }

    fn state(&self) -> u8 {
        self.taken.load(std::sync::atomic::Ordering::Acquire)
    }

    /// 1단 고정 — 2단·사본을 버리고 소비자 타임아웃을 1단에 적용.
    fn commit_udp(&mut self) {
        self.relay = None;
        self.pending.clear();
        if let Some(u) = self.udp.as_mut() {
            let _ = u.set_recv_timeout(self.consumer_timeout);
        }
        self.taken
            .store(LADDER_UDP, std::sync::atomic::Ordering::Release);
    }

    /// 2단 고정 — 1단을 버리고 사본을 2단으로 재전송.
    fn commit_relay(&mut self) -> Result<(), LinkError> {
        self.udp = None;
        let relay = self.relay.as_mut().ok_or(LinkError::Closed)?;
        for f in self.pending.drain(..) {
            relay.send(&f)?;
        }
        let _ = relay.set_recv_timeout(self.consumer_timeout);
        self.taken
            .store(LADDER_RELAY, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}

impl Link for LadderLink {
    fn peer(&self) -> PeerId {
        // 사다리 수준에서 상대 신원은 알 수 없다 — 종단 핸드셰이크가 확정한다(RelayLink와 같다).
        PeerId::from_bytes([0u8; 32])
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        match self.state() {
            LADDER_UDP => self.udp.as_mut().ok_or(LinkError::Closed)?.send(frame),
            LADDER_RELAY => self.relay.as_mut().ok_or(LinkError::Closed)?.send(frame),
            _ => {
                if let Some(u) = self.udp.as_mut() {
                    if u.send(frame).is_ok() {
                        self.pending.push(frame.to_vec());
                        return Ok(());
                    }
                    self.udp = None; // 1단 송신 실패 = 1단 없음
                }
                // 1단이 없으면 기다릴 회신도 없다 — 곧장 2단 고정.
                self.commit_relay()?;
                self.relay.as_mut().ok_or(LinkError::Closed)?.send(frame)
            }
        }
    }

    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        match self.state() {
            LADDER_UDP => self.udp.as_mut().ok_or(LinkError::Closed)?.recv(),
            LADDER_RELAY => self.relay.as_mut().ok_or(LinkError::Closed)?.recv(),
            _ => {
                // 1단: 첫 회신을 UDP 상한 안에 기다린다 — 오면 1단 고정.
                if let Some(u) = self.udp.as_mut() {
                    let _ =
                        u.set_recv_timeout(Some(self.consumer_timeout.unwrap_or(HS_TIMEOUT_UDP)));
                    if let Ok(f) = u.recv() {
                        self.commit_udp();
                        return Ok(f);
                    }
                    // 상대가 UDP를 듣지 않았다(창 어긋남·관측 비대칭) — 2단으로 내려간다.
                }
                // 2단: 사본 재전송 → 첫 회신(릴레이 상한) → 2단 고정.
                self.commit_relay()?;
                let relay = self.relay.as_mut().ok_or(LinkError::Closed)?;
                let _ =
                    relay.set_recv_timeout(Some(self.consumer_timeout.unwrap_or(HS_TIMEOUT_RELAY)));
                let r = relay.recv();
                let _ = relay.set_recv_timeout(self.consumer_timeout);
                r
            }
        }
    }

    fn set_recv_timeout(&mut self, dur: Option<Duration>) -> Result<(), LinkError> {
        self.consumer_timeout = dur;
        match self.state() {
            LADDER_UDP => self
                .udp
                .as_mut()
                .ok_or(LinkError::Closed)?
                .set_recv_timeout(dur),
            LADDER_RELAY => self
                .relay
                .as_mut()
                .ok_or(LinkError::Closed)?
                .set_recv_timeout(dur),
            _ => Ok(()), // 확정 전엔 단별 핸드셰이크 상한을 쓴다 — 고정 때 적용
        }
    }

    fn remote_ip(&self) -> Option<IpAddr> {
        match self.state() {
            LADDER_UDP => self.udp.as_ref().and_then(Link::remote_ip),
            LADDER_RELAY => self.relay.as_ref().and_then(Link::remote_ip),
            // 확정 전 = 서버 주소(공인 = Remote) — 상대 UDP 관측도 공인 엔드포인트라 등급은 같다(fail-closed 방향).
            _ => Some(self.server_ip),
        }
    }

    fn rendezvous(&self) -> bool {
        true
    }
}

fn finish_via(
    mut session: NoiseSession<Box<dyn Link>>,
    taken: PathTaken,
    path: nbeep_core::PathClass,
    expect: Option<&PeerId>,
    via_psk: bool,
) -> Result<ViaSession, ViaError> {
    use nbeep_core::Session as _;
    if let Some(dst) = expect {
        if session.peer() != *dst {
            return Err(ViaError::WrongPeer); // RID 충돌·오지정 — 인증된 키가 근거다
        }
    }
    session.set_recv_timeout(None); // 핸드셰이크용 타임아웃 원복(소비자가 다시 건다)
    Ok(ViaSession {
        session,
        taken,
        path,
        via_psk,
    })
}

/// **받는 쪽 사다리** — 인바운드 랑데부에 프로브→수락 순서를 지키고, 펀치를 병행하며
/// **첫 프레임이 온 링크**로 종단 수락을 잇는다(여는 쪽이 어느 단을 골랐든 따라간다).
/// X-2 승격(11-06) 뒤 = `accept_link_via`(링크 수준 · crate 내부) + 종단 핸드셰이크 — CLI·기존 호출자 호환 층.
///
/// # Errors
/// `deadline` 내 어느 링크로도 첫 프레임이 없거나 핸드셰이크 실패 시 [`ViaError`].
pub fn accept_via(
    client: &RelayClient,
    inc: RelayIncoming,
    id: &Identity,
    punch: bool,
    deadline: Duration,
    psk: Option<&[u8; 32]>,
) -> Result<ViaSession, ViaError> {
    let (link, taken) = accept_link_via(client, inc, punch, deadline)?;
    let path = class_of_link(&*link, nbeep_core::PathClass::Remote);
    accept_on(link, id, taken, path, psk)
}

/// **받는 쪽 사다리를 링크 수준에서 잇는다**(X-2 Transport 승격 · 11-06) — 프로브→수락 순서 · 펀치 병행 · **첫 프레임이
/// 온 링크**를 고르고 그 프레임을 되살린 링크([`PrefixedLink`] · `rendezvous() == true`)를 돌려준다. 핸드셰이크는
/// 호출자 몫(`Transport::incoming` 소비자 = 호스트의 단일 인바운드 펌프).
///
/// # Errors
/// `deadline` 내 어느 링크로도 첫 프레임이 없으면 [`ViaError::Dead`].
pub(crate) fn accept_link_via(
    client: &RelayClient,
    inc: RelayIncoming,
    punch: bool,
    deadline: Duration,
) -> Result<(Box<dyn Link>, PathTaken), ViaError> {
    let RelayIncoming {
        mut link, peer_udp, ..
    } = inc;
    // 프로브(수락 **전**) → 수락 → 펀치 병행. 여는 쪽 펀치는 OpenResult(수락 후) 뒤에
    // 시작하므로, 이 순서면 내 관측이 반드시 실려 간다.
    let punch_rx = match (punch, peer_udp) {
        (true, Some(ep)) => fresh_probed_sock(client).ok().map(|sock| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(nbeep_net::UdpLink::punch(sock, ep, PUNCH_WINDOW));
            });
            rx
        }),
        _ => None,
    };
    inc_accept(&link);
    let _ = link.set_recv_timeout(Some(Duration::from_millis(100)));
    let start = std::time::Instant::now();
    let mut udp: Option<nbeep_net::UdpLink> = None;
    let mut punch_pending = punch_rx.is_some();
    let mut relay_alive = true;
    loop {
        if let Some(rx) = &punch_rx {
            if punch_pending {
                if let Ok(res) = rx.try_recv() {
                    punch_pending = false;
                    if let Ok(mut u) = res {
                        let _ = u.set_recv_timeout(Some(Duration::from_millis(100)));
                        udp = Some(u);
                    }
                }
            }
        }
        // UDP 쪽 첫 프레임 — 여는 쪽이 UDP 단을 골랐다.
        let mut udp_first: Option<Vec<u8>> = None;
        if let Some(u) = udp.as_mut() {
            match u.recv() {
                Ok(f) => udp_first = Some(f),
                Err(LinkError::TimedOut) => {}
                Err(LinkError::Closed) => udp = None,
            }
        }
        if let Some(first) = udp_first {
            let u = udp.take().ok_or(ViaError::Dead)?;
            return Ok((
                Box::new(PrefixedLink {
                    first: Some(first),
                    inner: Box::new(u),
                }),
                PathTaken::Udp,
            ));
        }
        // 릴레이 쪽 첫 프레임 — 여는 쪽이 릴레이 단을 골랐다(또는 펀치 실패).
        if relay_alive {
            match link.recv() {
                Ok(first) => {
                    return Ok((
                        Box::new(PrefixedLink {
                            first: Some(first),
                            inner: Box::new(link),
                        }),
                        PathTaken::Relay,
                    ));
                }
                Err(LinkError::TimedOut) => {}
                Err(LinkError::Closed) => relay_alive = false,
            }
        }
        if start.elapsed() >= deadline || (!relay_alive && udp.is_none() && !punch_pending) {
            return Err(ViaError::Dead);
        }
    }
}

/// 수락 통지(내부 — [`RelayIncoming::accept`]와 동일 경로).
fn inc_accept(link: &RelayLink) {
    let _ = link.cmd_tx.send(Cmd::AcceptCh { ch: link.ch });
}

fn accept_on(
    mut link: Box<dyn Link>,
    id: &Identity,
    taken: PathTaken,
    path: nbeep_core::PathClass,
    psk: Option<&[u8; 32]>,
) -> Result<ViaSession, ViaError> {
    use nbeep_core::Session as _;
    let _ = link.set_recv_timeout(Some(HS_TIMEOUT_UDP));
    // XX/XXpsk3는 첫 메시지로 가른다(ADR-0015 §3-3 · 왕복 0).
    match NoiseSession::accept_any(link, id, psk) {
        Ok((mut session, via_psk)) => {
            session.set_recv_timeout(None);
            Ok(ViaSession {
                session,
                taken,
                path,
                via_psk,
            })
        }
        Err(_) => Err(ViaError::Handshake),
    }
}

// ── 서버 핀 파일 ────────────────────────────────────────────────

/// 서버 TOFU 핀의 영속([docs/32 §2-4·§13-3]) — 주소별 서버 공개키를 기억한다.
/// 형식 = 텍스트 한 줄 `v1 <주소> <64hex>` (읽기 쉬움 · 잘못된 줄은 없는 셈 친다 —
/// 핀 부재는 "첫 접속"으로 흐를 뿐 조용한 수락이 아니다: 불일치는 언제나 시끄럽다).
pub mod pinfile {
    use super::PeerId;
    use std::path::Path;

    /// `addr`에 핀된 서버 키.
    #[must_use]
    pub fn lookup(path: &Path, addr: &str) -> Option<PeerId> {
        let text = std::fs::read_to_string(path).ok()?;
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if it.next() != Some("v1") {
                continue;
            }
            let (Some(a), Some(hex)) = (it.next(), it.next()) else {
                continue;
            };
            if a == addr {
                return super::parse_peer_hex(hex);
            }
        }
        None
    }

    /// `addr`의 핀 저장(기존 항목 교체) — temp 쓰기 후 rename(원자적).
    ///
    /// # Errors
    /// IO 실패 시 `io::Error`.
    pub fn store(path: &Path, addr: &str, peer: &PeerId) -> std::io::Result<()> {
        let mut lines: Vec<String> = std::fs::read_to_string(path)
            .map(|t| {
                t.lines()
                    .filter(|l| {
                        let mut it = l.split_whitespace();
                        !(it.next() == Some("v1") && it.next() == Some(addr))
                    })
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let hex = super::peer_hex(peer);
        lines.push(format!("v1 {addr} {hex}"));
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        {
            // 소유자 전용(0600 · Unix) — 핀은 평문이라 **훼손이 곧 서버 바꿔치기**다(09-05 ·
            // 실측 664 = 같은 그룹 계정이 쓸 수 있었다). rename이 temp 모드를 나른다.
            use std::io::Write as _;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all((lines.join("\n") + "\n").as_bytes())?;
            f.sync_all()?;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn store_lookup_replace_roundtrip() {
            let dir = std::env::temp_dir().join(format!("nb-pin-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let p = dir.join("server.pin");
            let _ = std::fs::remove_file(&p);
            let k1 = PeerId::from_bytes([1u8; 32]);
            let k2 = PeerId::from_bytes([2u8; 32]);
            assert_eq!(lookup(&p, "10.0.0.1:47300"), None, "핀 없음 = 첫 접속");
            store(&p, "10.0.0.1:47300", &k1).unwrap();
            store(&p, "relay.example:47300", &k2).unwrap();
            assert_eq!(lookup(&p, "10.0.0.1:47300"), Some(k1));
            assert_eq!(lookup(&p, "relay.example:47300"), Some(k2));
            store(&p, "10.0.0.1:47300", &k2).unwrap(); // 교체(재핀은 사용자 결정 후)
            assert_eq!(lookup(&p, "10.0.0.1:47300"), Some(k2));
            let _ = std::fs::remove_file(&p);
        }

        #[test]
        fn corrupt_lines_are_skipped() {
            let dir = std::env::temp_dir().join(format!("nb-pin-c-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let p = dir.join("server.pin");
            std::fs::write(&p, "garbage\nv1 a.b:1 zz\nv1 x:1 ").unwrap();
            assert_eq!(
                lookup(&p, "a.b:1"),
                None,
                "손상 줄 = 핀 부재(불일치 조용 수락 아님)"
            );
            let _ = std::fs::remove_file(&p);
        }
    }
}

// ── 접속 단일 정책 (CLI·GUI 공용 — 두 벌 금지) ────────────────────

/// 서버 주소 해석 — 반환 = (핀 키로 쓸 정규 문자열, 소켓 주소).
/// IP 리터럴은 정규화([docs/19] M1-14 공유 · 포트 생략 = [`DEFAULT_RELAY_PORT`]),
/// 호스트명은 DNS로 해석한다(DR-19의 DDNS 경로).
#[must_use]
pub fn resolve_server(raw: &str) -> Option<(String, SocketAddr)> {
    if let Some(n) = nbeep_core::endpoint::normalize_endpoint(raw, DEFAULT_RELAY_PORT) {
        if let Ok(sa) = n.parse::<SocketAddr>() {
            return Some((n, sa));
        }
    }
    let with_port = if raw.contains(':') {
        raw.to_string()
    } else {
        format!("{raw}:{DEFAULT_RELAY_PORT}")
    };
    use std::net::ToSocketAddrs as _;
    let sa = with_port.to_socket_addrs().ok()?.next()?;
    Some((with_port, sa))
}

/// [`attach`] 성공 — 접속·등록 완료된 클라이언트 + 핀 이력.
#[derive(Debug)]
pub struct Attached {
    /// 접속·RID 등록까지 끝난 클라이언트.
    pub client: RelayClient,
    /// 핀 키로 쓴 정규 주소 문자열(상태 표시·재시도용).
    pub addr: String,
    /// 첫 접속(TOFU) — 지금 본 서버 키를 핀했다.
    pub first_pin: bool,
    /// 핀 저장 실패(IO) — 접속은 유효하나 다음 접속이 다시 첫 접속으로 보인다.
    pub pin_write_failed: bool,
}

/// [`attach`] 실패 사유.
#[derive(Debug)]
pub enum AttachError {
    /// 주소 해석 실패(형식 오류·DNS 실패).
    Resolve,
    /// 접속·핸드셰이크·등록 실패 — 핀 불일치([`RelayError::PinMismatch`])는
    /// **시끄럽게**(DR-28 "신원이 바뀌면 시끄럽게" · 조용한 재핀 없음).
    Relay(RelayError),
}

/// 릴레이 서버 접속 **단일 정책**(docs/32 §2-4·§13-2 — CLI·GUI가 같은 함수를 쓴다):
/// 주소 해석 → 접속·Noise → **핀 검증**(첫 접속 = TOFU 저장 · 불일치 = 중단) → RID 등록.
/// 출력·재시도는 호출자 몫(CLI = 즉시 출력 · GUI = 상태바+백오프).
///
/// # Errors
/// 해석 실패 = [`AttachError::Resolve`] · 그 외 = [`AttachError::Relay`].
pub fn attach(
    raw: &str,
    id: &Identity,
    pin_path: &std::path::Path,
    extra_rids: &[Rid],
) -> Result<Attached, AttachError> {
    let (addr, sa) = resolve_server(raw).ok_or(AttachError::Resolve)?;
    let expected = pinfile::lookup(pin_path, &addr);
    // ★ 기기별 RID 3 + 추가(사용자 페어링 RID_pair 3 · ADR-0015 §3-4) — MAX_RIDS 8 안에서 서버 무변경.
    let mut rids: Vec<Rid> = rids_around(&id.peer_id()).to_vec();
    rids.extend(extra_rids.iter().copied().take(MAX_RIDS - 3));
    let client = RelayClient::connect(sa, id, &rids, expected).map_err(AttachError::Relay)?;
    let first_pin = expected.is_none();
    let pin_write_failed =
        first_pin && pinfile::store(pin_path, &addr, &client.server_peer()).is_err();
    Ok(Attached {
        client,
        addr,
        first_pin,
        pin_write_failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rid_is_stable_and_rotates() {
        let p = PeerId::from_bytes([7u8; 32]);
        assert_eq!(rid_for(&p, 100), rid_for(&p, 100), "같은 에폭 = 같은 RID");
        assert_ne!(
            rid_for(&p, 100),
            rid_for(&p, 101),
            "에폭이 바뀌면 RID가 바뀐다"
        );
        let q = PeerId::from_bytes([8u8; 32]);
        assert_ne!(
            rid_for(&p, 100),
            rid_for(&q, 100),
            "키가 다르면 RID가 다르다"
        );
    }

    #[test]
    fn card_roundtrip_and_cap() {
        let c = encode_card("이름", "a@b.c", "소개");
        assert_eq!(
            decode_card(&c),
            Some(("이름".into(), "a@b.c".into(), "소개".into()))
        );
        let big = encode_card("n", "e", &"x".repeat(5000));
        assert!(big.len() <= CARD_MAX, "상한 안으로 자른다");
        assert!(decode_card(&big).is_some());
        assert_eq!(decode_card(&vec![1u8; CARD_MAX + 1]), None);
    }
}
