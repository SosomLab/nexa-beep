//! `nbeep-relay` — 릴레이 제어 와이어 + 클라이언트 어댑터(X-1·X-2b 1차 · [docs/32 §12-6·§13]).
//!
//! **서버(`nexa-beepd`)와 클라이언트가 이 크레이트 하나를 같이 쓴다** — 와이어 어긋남을
//! 컴파일 시점에 잡는 한-저장소 구성의 본체다([docs/32 §9] · Q-32-13 확정 08-21).
//!
//! ## 봉투 원리(S-3)
//!
//! 서버가 보는 것 = **회전 RID · 채널 번호 · 바이트 수 · 시각**이 전부다.
//! - 제어 세션은 서버와의 Noise(서버 신원 키 = TOFU 핀 대상 — [docs/32 §2-4])이고,
//! - 그 안의 [`C2s::Data`]/[`S2c::Data`] 페이로드는 **종단 A↔B의 Noise 암호문**이다.
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
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::time::Duration;

/// 릴레이 서버 기본 포트(TCP 제어 + 같은 번호의 UDP 관측) — 발견 47100·세션 47200 다음.
pub const DEFAULT_RELAY_PORT: u16 = 47_300;

/// 랑데부 ID(16B) — 회전 가명([docs/32 §2-3]).
pub type Rid = [u8; 16];

/// UDP 관측 프로브 매직 — ARQ 매직(`NBU1`)과 다른 4B라 같은 소켓에서 섞여도 갈린다.
pub const OBS_MAGIC: [u8; 4] = *b"NBOB";

/// 링크 프레임 상한 — TCP·UDP 링크와 정합(Noise 상한).
pub const MAX_FRAME: usize = nbeep_net::arq::MAX_FRAME;

/// 릴레이로 나르는 조각 상한 — 제어 세션 페이로드(Noise 상한 65519) − 헤더 여유.
/// [`RelayLink`]는 이보다 큰 프레임을 투명하게 분할·조립한다(65535 프레임 수용).
pub const RELAY_CHUNK: usize = 32 * 1024;

/// RID 유도 — 에폭은 UTC 일 번호. 내 공개키를 **이미 아는** 사람만 같은 값을 계산할 수
/// 있다(릴레이는 새로운 만남을 주선하지 않는다 — [docs/32 §2-3]).
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

// ── 와이어 인코딩 ────────────────────────────────────────────────
//
// 제어 세션 프레임 = [kind u8][본문]. 정수는 전부 BE. 미지 kind는 조용히 버린다
// (전방 호환 — sgroup·Info 꼬리와 같은 규약).

/// 클라이언트 → 서버.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum C2s {
    /// 프레즌스 등록 — 회전 RID 목록(≤[`MAX_RIDS`]).
    Register {
        /// 등록할 회전 RID들([`rids_around`]).
        rids: Vec<Rid>,
    },
    /// `dst` RID로 채널 열기 요청. `token`은 응답 대조용.
    Open {
        /// 응답 대조 토큰.
        token: u32,
        /// 대상 RID.
        dst: Rid,
    },
    /// 인바운드 채널 수락 — 이때부터 서버가 중계한다(양방향 성립만 — [docs/32 §2-6]).
    Accept {
        /// 채널 번호.
        ch: u32,
    },
    /// 채널 데이터(종단 암호문 조각). `fin` = 링크 프레임의 마지막 조각.
    Data {
        /// 채널 번호.
        ch: u32,
        /// 링크 프레임의 마지막 조각인가.
        fin: bool,
        /// 조각 바이트(서버는 열 수 없다).
        bytes: Vec<u8>,
    },
    /// 채널 닫기.
    CloseCh {
        /// 채널 번호.
        ch: u32,
    },
    /// 생존 신호(서버 유휴 정리 방지).
    Ping,
}

/// 서버 → 클라이언트.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S2c {
    /// 등록 완료 — UDP 관측 토큰·포트 + 서버가 본 내 TCP 주소.
    RegisterOk {
        /// UDP 프로브에 실어 보낼 토큰(관측 ↔ 등록 연결 고리).
        udp_token: u64,
        /// 서버 UDP 관측 포트.
        udp_port: u16,
        /// 서버가 본 내 공인 TCP 엔드포인트.
        observed: Option<SocketAddr>,
    },
    /// [`C2s::Open`] 결과. `status` 0=성립 · 1=대상 없음 · 2=상한/거절.
    OpenResult {
        /// 요청의 대조 토큰.
        token: u32,
        /// 결과 코드(0=성립).
        status: u8,
        /// 성립한 채널 번호(성립 시).
        ch: u32,
        /// 상대의 관측 UDP 엔드포인트(홀펀칭용 · 미관측이면 None).
        peer_udp: Option<SocketAddr>,
    },
    /// 인바운드 채널 — `src` RID가 나를 찾는다.
    Incoming {
        /// 채널 번호.
        ch: u32,
        /// 여는 쪽의 등록 RID.
        src: Rid,
        /// 여는 쪽의 관측 UDP 엔드포인트(홀펀칭용).
        peer_udp: Option<SocketAddr>,
    },
    /// 채널 데이터(종단 암호문 조각).
    Data {
        /// 채널 번호.
        ch: u32,
        /// 링크 프레임의 마지막 조각인가.
        fin: bool,
        /// 조각 바이트.
        bytes: Vec<u8>,
    },
    /// 채널 종료(상대 이탈·닫음).
    ChClosed {
        /// 채널 번호.
        ch: u32,
    },
    /// [`C2s::Ping`] 응답.
    Pong,
}

/// 연결당 등록 가능한 RID 상한(에폭 3 + 여유).
pub const MAX_RIDS: usize = 8;

const K_REGISTER: u8 = 0x01;
const K_OPEN: u8 = 0x02;
const K_ACCEPT: u8 = 0x03;
const K_DATA: u8 = 0x04;
const K_CLOSECH: u8 = 0x05;
const K_PING: u8 = 0x06;
const K_REGISTER_OK: u8 = 0x81;
const K_OPEN_RESULT: u8 = 0x82;
const K_INCOMING: u8 = 0x83;
const K_S_DATA: u8 = 0x84;
const K_CH_CLOSED: u8 = 0x85;
const K_PONG: u8 = 0x86;

fn put_endpoint(out: &mut Vec<u8>, ep: Option<SocketAddr>) {
    match ep {
        None => out.push(0),
        Some(SocketAddr::V4(a)) => {
            out.push(4);
            out.extend_from_slice(&a.ip().octets());
            out.extend_from_slice(&a.port().to_be_bytes());
        }
        Some(SocketAddr::V6(a)) => {
            out.push(6);
            out.extend_from_slice(&a.ip().octets());
            out.extend_from_slice(&a.port().to_be_bytes());
        }
    }
}

fn take_endpoint(b: &[u8]) -> Option<(Option<SocketAddr>, usize)> {
    match *b.first()? {
        0 => Some((None, 1)),
        4 => {
            if b.len() < 7 {
                return None;
            }
            let ip = Ipv4Addr::new(b[1], b[2], b[3], b[4]);
            let port = u16::from_be_bytes([b[5], b[6]]);
            Some((Some(SocketAddr::new(IpAddr::V4(ip), port)), 7))
        }
        6 => {
            if b.len() < 19 {
                return None;
            }
            let mut oct = [0u8; 16];
            oct.copy_from_slice(&b[1..17]);
            let port = u16::from_be_bytes([b[17], b[18]]);
            Some((
                Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(oct)), port)),
                19,
            ))
        }
        _ => None,
    }
}

impl C2s {
    /// 제어 세션 프레임으로 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            Self::Register { rids } => {
                o.push(K_REGISTER);
                o.push(rids.len().min(MAX_RIDS) as u8);
                for r in rids.iter().take(MAX_RIDS) {
                    o.extend_from_slice(r);
                }
            }
            Self::Open { token, dst } => {
                o.push(K_OPEN);
                o.extend_from_slice(&token.to_be_bytes());
                o.extend_from_slice(dst);
            }
            Self::Accept { ch } => {
                o.push(K_ACCEPT);
                o.extend_from_slice(&ch.to_be_bytes());
            }
            Self::Data { ch, fin, bytes } => {
                o.push(K_DATA);
                o.extend_from_slice(&ch.to_be_bytes());
                o.push(u8::from(*fin));
                o.extend_from_slice(bytes);
            }
            Self::CloseCh { ch } => {
                o.push(K_CLOSECH);
                o.extend_from_slice(&ch.to_be_bytes());
            }
            Self::Ping => o.push(K_PING),
        }
        o
    }

    /// 디코딩 — 형식 오류·미지 kind는 `None`(호출자가 조용히 버린다 · 전방 호환).
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        match *b.first()? {
            K_REGISTER => {
                let n = *b.get(1)? as usize;
                if n > MAX_RIDS || b.len() < 2 + n * 16 {
                    return None;
                }
                let mut rids = Vec::with_capacity(n);
                for i in 0..n {
                    let mut r = [0u8; 16];
                    r.copy_from_slice(&b[2 + i * 16..2 + (i + 1) * 16]);
                    rids.push(r);
                }
                Some(Self::Register { rids })
            }
            K_OPEN => {
                if b.len() < 21 {
                    return None;
                }
                let token = u32::from_be_bytes([b[1], b[2], b[3], b[4]]);
                let mut dst = [0u8; 16];
                dst.copy_from_slice(&b[5..21]);
                Some(Self::Open { token, dst })
            }
            K_ACCEPT => Some(Self::Accept {
                ch: u32::from_be_bytes([*b.get(1)?, *b.get(2)?, *b.get(3)?, *b.get(4)?]),
            }),
            K_DATA => {
                if b.len() < 6 {
                    return None;
                }
                Some(Self::Data {
                    ch: u32::from_be_bytes([b[1], b[2], b[3], b[4]]),
                    fin: b[5] != 0,
                    bytes: b[6..].to_vec(),
                })
            }
            K_CLOSECH => Some(Self::CloseCh {
                ch: u32::from_be_bytes([*b.get(1)?, *b.get(2)?, *b.get(3)?, *b.get(4)?]),
            }),
            K_PING => Some(Self::Ping),
            _ => None,
        }
    }
}

impl S2c {
    /// 제어 세션 프레임으로 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            Self::RegisterOk {
                udp_token,
                udp_port,
                observed,
            } => {
                o.push(K_REGISTER_OK);
                o.extend_from_slice(&udp_token.to_be_bytes());
                o.extend_from_slice(&udp_port.to_be_bytes());
                put_endpoint(&mut o, *observed);
            }
            Self::OpenResult {
                token,
                status,
                ch,
                peer_udp,
            } => {
                o.push(K_OPEN_RESULT);
                o.extend_from_slice(&token.to_be_bytes());
                o.push(*status);
                o.extend_from_slice(&ch.to_be_bytes());
                put_endpoint(&mut o, *peer_udp);
            }
            Self::Incoming { ch, src, peer_udp } => {
                o.push(K_INCOMING);
                o.extend_from_slice(&ch.to_be_bytes());
                o.extend_from_slice(src);
                put_endpoint(&mut o, *peer_udp);
            }
            Self::Data { ch, fin, bytes } => {
                o.push(K_S_DATA);
                o.extend_from_slice(&ch.to_be_bytes());
                o.push(u8::from(*fin));
                o.extend_from_slice(bytes);
            }
            Self::ChClosed { ch } => {
                o.push(K_CH_CLOSED);
                o.extend_from_slice(&ch.to_be_bytes());
            }
            Self::Pong => o.push(K_PONG),
        }
        o
    }

    /// 디코딩 — 형식 오류·미지 kind는 `None`.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        match *b.first()? {
            K_REGISTER_OK => {
                if b.len() < 11 {
                    return None;
                }
                let udp_token =
                    u64::from_be_bytes([b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8]]);
                let udp_port = u16::from_be_bytes([b[9], b[10]]);
                let (observed, _) = take_endpoint(&b[11..])?;
                Some(Self::RegisterOk {
                    udp_token,
                    udp_port,
                    observed,
                })
            }
            K_OPEN_RESULT => {
                if b.len() < 10 {
                    return None;
                }
                let token = u32::from_be_bytes([b[1], b[2], b[3], b[4]]);
                let status = b[5];
                let ch = u32::from_be_bytes([b[6], b[7], b[8], b[9]]);
                let (peer_udp, _) = take_endpoint(&b[10..])?;
                Some(Self::OpenResult {
                    token,
                    status,
                    ch,
                    peer_udp,
                })
            }
            K_INCOMING => {
                if b.len() < 21 {
                    return None;
                }
                let ch = u32::from_be_bytes([b[1], b[2], b[3], b[4]]);
                let mut src = [0u8; 16];
                src.copy_from_slice(&b[5..21]);
                let (peer_udp, _) = take_endpoint(&b[21..])?;
                Some(Self::Incoming { ch, src, peer_udp })
            }
            K_S_DATA => {
                if b.len() < 6 {
                    return None;
                }
                Some(Self::Data {
                    ch: u32::from_be_bytes([b[1], b[2], b[3], b[4]]),
                    fin: b[5] != 0,
                    bytes: b[6..].to_vec(),
                })
            }
            K_CH_CLOSED => Some(Self::ChClosed {
                ch: u32::from_be_bytes([*b.get(1)?, *b.get(2)?, *b.get(3)?, *b.get(4)?]),
            }),
            K_PONG => Some(Self::Pong),
            _ => None,
        }
    }
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
    let mut probe = Vec::with_capacity(12);
    probe.extend_from_slice(&OBS_MAGIC);
    probe.extend_from_slice(&udp_token.to_be_bytes());
    sock.send_to(&probe, server)?;
    sock.set_read_timeout(Some(timeout))?;
    let mut buf = [0u8; 64];
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let (n, from) = sock.recv_from(&mut buf)?;
        // 프로브 소켓엔 펀칭 SYN 등 다른 트래픽이 섞일 수 있다 — 매직·발신원으로 거른다.
        if from == server && n >= 13 && buf[..4] == OBS_MAGIC {
            let echo_token = u64::from_be_bytes([
                buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11],
            ]);
            if echo_token == udp_token {
                if let Some((Some(ep), _)) = take_endpoint(&buf[12..n]) {
                    return Ok(ep);
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

type OpenResp = SyncSender<Result<(RelayLink, Option<SocketAddr>), u8>>;

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
    /// 클라이언트 종료 — 액터가 세션을 내려놓는다(서버가 TCP 종료로 정리).
    Shutdown,
}

enum ChEvent {
    Data { fin: bool, bytes: Vec<u8> },
    Closed,
}

/// 릴레이 서버에 붙은 클라이언트 — 제어 세션(서버와의 Noise)을 액터 스레드가 소유하고,
/// 채널별 [`RelayLink`]가 그 위를 다중화한다.
#[derive(Debug)]
pub struct RelayClient {
    cmd_tx: Sender<Cmd>,
    incoming_rx: Receiver<RelayIncoming>,
    server_peer: PeerId,
    server_addr: SocketAddr,
    reg: RegisterInfo,
}

impl RelayClient {
    /// 서버 접속 → Noise 핸드셰이크 → (핀 검증) → RID 등록 → 액터 가동.
    ///
    /// `expected_server`: 핀된 서버 키. `None` = 첫 접속(TOFU — 반환된
    /// [`Self::server_peer`]를 호출자가 핀에 저장한다). 불일치는 [`RelayError::PinMismatch`].
    ///
    /// # Errors
    /// 연결·핸드셰이크·핀 불일치·등록 실패 시 [`RelayError`].
    pub fn connect(
        server: SocketAddr,
        id: &Identity,
        rids: &[Rid],
        expected_server: Option<PeerId>,
    ) -> Result<Self, RelayError> {
        let stream =
            TcpStream::connect_timeout(&server, Duration::from_secs(10)).map_err(RelayError::Io)?;
        let mut link = TcpLink::new(stream).map_err(RelayError::Io)?;
        // 침묵 서버가 접속을 영구 점유하지 못하게 — 핸드셰이크·등록 왕복 상한.
        let _ = link.set_recv_timeout(Some(Duration::from_secs(10)));
        let mut session = NoiseSession::initiate(link, id).map_err(|_| RelayError::Handshake)?;
        let server_peer = session.peer();
        if let Some(exp) = expected_server {
            if exp != server_peer {
                return Err(RelayError::PinMismatch {
                    expected: exp,
                    got: server_peer,
                });
            }
        }
        // 등록은 동기 왕복 — 액터 가동 전이라 세션을 직접 쓴다.
        session
            .send(
                &C2s::Register {
                    rids: rids.to_vec(),
                }
                .encode(),
            )
            .map_err(|_| RelayError::Protocol)?;
        let reg = loop {
            let frame = match session.recv() {
                Ok(f) => f,
                Err(_) => return Err(RelayError::Protocol),
            };
            match S2c::decode(&frame) {
                Some(S2c::RegisterOk {
                    udp_token,
                    udp_port,
                    observed,
                }) => {
                    break RegisterInfo {
                        udp_token,
                        udp_port,
                        observed_tcp: observed,
                    }
                }
                Some(_) | None => continue, // 미지·이른 프레임은 버린다
            }
        };
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel::<RelayIncoming>();
        let actor_cmd = cmd_tx.clone();
        std::thread::Builder::new()
            .name("relay-client".into())
            .spawn(move || actor(session, &cmd_rx, &actor_cmd, &incoming_tx))
            .map_err(RelayError::Io)?;
        Ok(Self {
            cmd_tx,
            incoming_rx,
            server_peer,
            server_addr: server,
            reg,
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

    /// `dst` RID로 채널을 연다. 성립 시 (링크, 상대 관측 UDP).
    ///
    /// # Errors
    /// 상태 코드 — 1=대상 없음 · 2=상한/거절 · 255=세션 죽음/시간 초과.
    pub fn open(&self, dst: Rid, timeout: Duration) -> Result<(RelayLink, Option<SocketAddr>), u8> {
        let (resp_tx, resp_rx) = std::sync::mpsc::sync_channel(1);
        self.cmd_tx
            .send(Cmd::Open { dst, resp: resp_tx })
            .map_err(|_| 255u8)?;
        match resp_rx.recv_timeout(timeout) {
            Ok(mut r) => {
                if let Ok((link, _)) = &mut r {
                    link.set_server_ip(self.server_addr.ip());
                }
                r
            }
            Err(_) => Err(255),
        }
    }

    /// 인바운드 채널 수신단 — 상대가 나를 열면 여기로 온다(릴레이 층은 자동 수락 —
    /// 진짜 게이트는 그 위의 종단 Noise + 앱 신뢰 정책이다).
    #[must_use]
    pub fn incoming(&self) -> &Receiver<RelayIncoming> {
        &self.incoming_rx
    }

    /// 인바운드 하나를 꺼내며 서버 IP를 배선(경로 등급 판정용).
    #[must_use]
    pub fn accept_incoming(&self, timeout: Duration) -> Option<RelayIncoming> {
        match self.incoming_rx.recv_timeout(timeout) {
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

/// 제어 세션 액터 — 세션의 단일 소유자(수신 폴 + 송신 채널 드레인 교대 · 세션 액터 규약).
fn actor(
    mut session: NoiseSession<TcpLink>,
    cmd_rx: &Receiver<Cmd>,
    cmd_tx: &Sender<Cmd>,
    incoming_tx: &Sender<RelayIncoming>,
) {
    session.set_recv_timeout(Some(Duration::from_millis(15)));
    let mut chans: HashMap<u32, Sender<ChEvent>> = HashMap::new();
    let mut pending: Vec<(u32, OpenResp)> = Vec::new();
    let mut next_token = 1u32;
    let mut last_ping = std::time::Instant::now();
    loop {
        // 1) 명령 드레인.
        let mut dead = false;
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Open { dst, resp } => {
                    let token = next_token;
                    next_token = next_token.wrapping_add(1);
                    if session.send(&C2s::Open { token, dst }.encode()).is_err() {
                        let _ = resp.try_send(Err(255));
                        dead = true;
                        break;
                    }
                    pending.push((token, resp));
                }
                Cmd::Send { ch, frame } => {
                    // 큰 프레임은 조각으로 — 제어 세션 페이로드 상한 안쪽(RELAY_CHUNK).
                    let mut rest = frame.as_slice();
                    loop {
                        let take = rest.len().min(RELAY_CHUNK);
                        let (chunk, tail) = rest.split_at(take);
                        let msg = C2s::Data {
                            ch,
                            fin: tail.is_empty(),
                            bytes: chunk.to_vec(),
                        };
                        if session.send(&msg.encode()).is_err() {
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
                    let _ = session.send(&C2s::CloseCh { ch }.encode());
                }
                Cmd::Shutdown => {
                    dead = true;
                    break;
                }
            }
        }
        if dead {
            break;
        }
        // 2) 생존 신호(20초) — 서버 유휴 정리·NAT 타임아웃 방지.
        if last_ping.elapsed() >= Duration::from_secs(20) {
            last_ping = std::time::Instant::now();
            if session.send(&C2s::Ping.encode()).is_err() {
                break;
            }
        }
        // 3) 세션 수신(15ms 폴 — 명령 드레인과 교대).
        match session.recv() {
            Ok(frame) => match S2c::decode(&frame) {
                Some(S2c::Data { ch, fin, bytes }) => {
                    if let Some(tx) = chans.get(&ch) {
                        if tx.send(ChEvent::Data { fin, bytes }).is_err() {
                            chans.remove(&ch); // 링크가 버려짐 — 서버에도 닫기 통지
                            let _ = session.send(&C2s::CloseCh { ch }.encode());
                        }
                    }
                }
                Some(S2c::Incoming { ch, src, peer_udp }) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    chans.insert(ch, tx);
                    if session.send(&C2s::Accept { ch }.encode()).is_err() {
                        break;
                    }
                    let link = RelayLink::new(ch, cmd_tx.clone(), rx);
                    if incoming_tx
                        .send(RelayIncoming {
                            link,
                            src,
                            peer_udp,
                        })
                        .is_err()
                    {
                        // 소유자가 인바운드 수신단을 버렸다 — 채널만 정리.
                        chans.remove(&ch);
                        let _ = session.send(&C2s::CloseCh { ch }.encode());
                    }
                }
                Some(S2c::OpenResult {
                    token,
                    status,
                    ch,
                    peer_udp,
                }) => {
                    if let Some(pos) = pending.iter().position(|(t, _)| *t == token) {
                        let (_, resp) = pending.swap_remove(pos);
                        if status == 0 {
                            let (tx, rx) = std::sync::mpsc::channel();
                            chans.insert(ch, tx);
                            let link = RelayLink::new(ch, cmd_tx.clone(), rx);
                            let _ = resp.try_send(Ok((link, peer_udp)));
                        } else {
                            let _ = resp.try_send(Err(status));
                        }
                    }
                }
                Some(S2c::ChClosed { ch }) => {
                    if let Some(tx) = chans.remove(&ch) {
                        let _ = tx.send(ChEvent::Closed);
                    }
                }
                Some(S2c::RegisterOk { .. } | S2c::Pong) | None => {}
            },
            Err(SessionError::TimedOut) => {}
            Err(_) => break, // 서버 세션 죽음 — 아래 정리로
        }
    }
    // 세션 죽음 = 전 채널 종료 통지(수신자들이 Closed를 본다).
    for (_, tx) in chans.drain() {
        let _ = tx.send(ChEvent::Closed);
    }
    for (_, resp) in pending.drain(..) {
        let _ = resp.try_send(Err(255));
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
}

impl RelayLink {
    fn new(ch: u32, cmd_tx: Sender<Cmd>, rx: Receiver<ChEvent>) -> Self {
        Self {
            ch,
            cmd_tx,
            rx,
            buf: Vec::new(),
            closed: false,
            recv_timeout: None,
            server_ip: None,
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
        if self.closed || frame.len() > MAX_FRAME {
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
                    if self.buf.len() + bytes.len() > MAX_FRAME {
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
    fn c2s_roundtrip() {
        let msgs = [
            C2s::Register {
                rids: vec![[1u8; 16], [2u8; 16]],
            },
            C2s::Open {
                token: 77,
                dst: [3u8; 16],
            },
            C2s::Accept { ch: 9 },
            C2s::Data {
                ch: 5,
                fin: true,
                bytes: b"cipher".to_vec(),
            },
            C2s::CloseCh { ch: 1 },
            C2s::Ping,
        ];
        for m in msgs {
            assert_eq!(C2s::decode(&m.encode()), Some(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn s2c_roundtrip() {
        let ep4: SocketAddr = "1.2.3.4:5678".parse().unwrap();
        let ep6: SocketAddr = "[2001:db8::1]:443".parse().unwrap();
        let msgs = [
            S2c::RegisterOk {
                udp_token: 0x00de_adbe_efca_fe01,
                udp_port: 47_300,
                observed: Some(ep4),
            },
            S2c::RegisterOk {
                udp_token: 1,
                udp_port: 2,
                observed: Some(ep6),
            },
            S2c::OpenResult {
                token: 3,
                status: 0,
                ch: 4,
                peer_udp: None,
            },
            S2c::Incoming {
                ch: 8,
                src: [9u8; 16],
                peer_udp: Some(ep4),
            },
            S2c::Data {
                ch: 5,
                fin: false,
                bytes: vec![0u8; 100],
            },
            S2c::ChClosed { ch: 6 },
            S2c::Pong,
        ];
        for m in msgs {
            assert_eq!(S2c::decode(&m.encode()), Some(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn unknown_kind_is_none() {
        assert_eq!(
            C2s::decode(&[0x7f, 1, 2]),
            None,
            "미지 kind = 조용히 버림(전방 호환)"
        );
        assert_eq!(S2c::decode(&[0x10]), None);
        assert_eq!(C2s::decode(&[]), None);
    }
}
