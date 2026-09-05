//! `nbeep-relay` — 릴레이 제어 와이어 + 클라이언트 어댑터(X-1·X-2b 1차 · [docs/32 §12-6·§13]).
//!
//! **서버(`nexa-beepd`)와 클라이언트가 이 크레이트 하나를 같이 쓴다** — 와이어 어긋남을
//! 컴파일 시점에 잡는 한-저장소 구성의 본체다([docs/32 §9] · Q-32-13 확정 08-21).
//!
//! ⚠️ **이 크레이트는 자매 프로젝트 `nexa-clip`과 와이어를 공유한다**([docs/44 §7]).
//! clip은 이 크레이트의 **사본**(`nclip-sync`)을 들고 같은 `nexa-beepd`에 붙는다 — CI 단독
//! 빌드 때문에 path 의존을 못 써 사본을 택했다(clip 09-03). UI 계층 포크와 달리 와이어는
//! 사본이 갈라지는 순간 **통신이 깨진다**. 그러므로 [`C2s`]/[`S2c`] 메시지·kind 번호·상수
//! ([`DEFAULT_RELAY_PORT`]·[`MAX_FRAME`]·[`RELAY_CHUNK`]·[`OBS_MAGIC`]·[`MAX_RIDS`]·
//! [`CARD_MAX`])·핸드셰이크 순서를 바꾸면 **같은 커밋에서 clip에 알린다**(docs/44 §7 절차).
//! 공유 크레이트 승격(clip DR-18)은 별도 결정.
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
    /// ★ 프레즌스 공개 여부(X-2e roster · 08-22 — [docs/32 §12-7] 옵트인):
    /// `true` = 같은 서버의 공개 사용자 목록에 나(공개키)를 싣고, 그 목록을 받는다.
    /// `false` = 내린다. 서버는 **켠 연결만** 메모리에 보관·배포한다(저장 0 불변).
    Announce {
        /// 목록에 실을지 여부.
        listed: bool,
    },
    /// ★ 공개 + **공개 카드**(08-22 — 사용자 확정 "공개 정보 즉시 표시"): 본인이
    /// 공개로 둔 항목(이름·이메일·소개)만 담은 소형 카드를 함께 싣는다. 서버는
    /// 내용 해석 없이 크기만 재고 그대로 되뿌린다(상한 [`CARD_MAX`]). 구서버는
    /// 미지 kind로 버리므로 클라는 [`C2s::Announce`]를 **뒤따라** 보낸다(폴백).
    AnnounceCard {
        /// 목록에 실을지 여부.
        listed: bool,
        /// 공개 카드 바이트([`encode_card`] — 빈 값 = 카드 없음·v2 표식만).
        card: Vec<u8>,
    },
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
    /// ★ 공개 사용자 등장(X-2e roster) — 입장 스냅숏 + 이후 델타. 존재(공개키)만
    /// 싣는다 — 이름·프로필은 성립 후 종단(P2P)이 나른다(DR-22 불변).
    PeerUp {
        /// 등장한 사용자 공개키.
        peer: PeerId,
    },
    /// 공개 사용자 이탈(연결 종료·공개 해제).
    PeerDown {
        /// 이탈한 사용자 공개키.
        peer: PeerId,
    },
    /// ★ 등장 + 공개 카드(08-22) — [`C2s::AnnounceCard`]를 보낸(v2) 연결에만
    /// 온다. 카드가 바뀐 재공지도 같은 kind로(수신은 upsert).
    PeerUpCard {
        /// 등장한 사용자 공개키.
        peer: PeerId,
        /// 공개 카드 바이트(빈 값 = 카드 없음).
        card: Vec<u8>,
    },
}

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

/// 연결당 등록 가능한 RID 상한(에폭 3 + 여유).
pub const MAX_RIDS: usize = 8;

const K_REGISTER: u8 = 0x01;
const K_OPEN: u8 = 0x02;
const K_ACCEPT: u8 = 0x03;
const K_DATA: u8 = 0x04;
const K_CLOSECH: u8 = 0x05;
const K_PING: u8 = 0x06;
const K_ANNOUNCE: u8 = 0x07;
const K_ANNOUNCE_CARD: u8 = 0x09;
const K_REGISTER_OK: u8 = 0x81;
const K_OPEN_RESULT: u8 = 0x82;
const K_INCOMING: u8 = 0x83;
const K_S_DATA: u8 = 0x84;
const K_CH_CLOSED: u8 = 0x85;
const K_PONG: u8 = 0x86;
const K_PEER_UP: u8 = 0x87;
const K_PEER_DOWN: u8 = 0x88;
const K_PEER_UP_CARD: u8 = 0x89;

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
            Self::Announce { listed } => {
                o.push(K_ANNOUNCE);
                o.push(u8::from(*listed));
            }
            Self::AnnounceCard { listed, card } => {
                o.push(K_ANNOUNCE_CARD);
                o.push(u8::from(*listed));
                o.extend_from_slice(card);
            }
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
            K_ANNOUNCE => Some(Self::Announce {
                listed: *b.get(1)? != 0,
            }),
            K_ANNOUNCE_CARD => Some(Self::AnnounceCard {
                listed: *b.get(1)? != 0,
                card: b.get(2..)?.to_vec(),
            }),
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
            Self::PeerUp { peer } => {
                o.push(K_PEER_UP);
                o.extend_from_slice(peer.as_bytes());
            }
            Self::PeerDown { peer } => {
                o.push(K_PEER_DOWN);
                o.extend_from_slice(peer.as_bytes());
            }
            Self::PeerUpCard { peer, card } => {
                o.push(K_PEER_UP_CARD);
                o.extend_from_slice(peer.as_bytes());
                o.extend_from_slice(card);
            }
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
            K_PEER_UP | K_PEER_DOWN => {
                if b.len() < 33 {
                    return None;
                }
                let mut k = [0u8; 32];
                k.copy_from_slice(&b[1..33]);
                let peer = PeerId::from_bytes(k);
                Some(if b[0] == K_PEER_UP {
                    Self::PeerUp { peer }
                } else {
                    Self::PeerDown { peer }
                })
            }
            K_PEER_UP_CARD => {
                if b.len() < 33 {
                    return None;
                }
                let mut k = [0u8; 32];
                k.copy_from_slice(&b[1..33]);
                Some(Self::PeerUpCard {
                    peer: PeerId::from_bytes(k),
                    card: b[33..].to_vec(),
                })
            }
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

impl RelayIncoming {
    /// 채널 수락 — **이때부터** 서버가 중계한다(지연 수락). 홀펀칭을 쓰려면 수락
    /// **전에** UDP 프로브를 마쳐야 상대의 OpenResult에 내 관측 엔드포인트가 실린다
    /// ([`accept_via`]가 이 순서를 지킨다). 수락 없이 drop = 서버가 열기 거절로 정리.
    pub fn accept(&self) {
        let _ = self.link.cmd_tx.send(Cmd::AcceptCh { ch: self.link.ch });
    }
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
        let (roster_tx, roster_rx) = std::sync::mpsc::channel::<RosterEvent>();
        let actor_cmd = cmd_tx.clone();
        std::thread::Builder::new()
            .name("relay-client".into())
            .spawn(move || actor(session, &cmd_rx, &actor_cmd, &incoming_tx, &roster_tx))
            .map_err(RelayError::Io)?;
        Ok(Self {
            cmd_tx,
            incoming_rx: std::sync::Mutex::new(incoming_rx),
            roster_rx: std::sync::Mutex::new(roster_rx),
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

    /// 액터(서버 제어 세션) 생존 여부 — 서버 세션이 죽으면 액터가 종료해 명령 채널이
    /// 닫힌다는 사실을 쓴다. 재접속 판단용(GUI 서버 틱 · 폴링 부담 0).
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.cmd_tx.send(Cmd::Nop).is_ok()
    }

    /// 프레즌스 공개 지정(X-2e roster · 옵트인 — [docs/32 §12-7]). `true` = 같은
    /// 서버의 공개 목록에 실리고 그 목록의 델타를 받기 시작한다(입장 스냅숏 포함).
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

/// 제어 세션 액터 — 세션의 단일 소유자(수신 폴 + 송신 채널 드레인 교대 · 세션 액터 규약).
fn actor(
    mut session: NoiseSession<TcpLink>,
    cmd_rx: &Receiver<Cmd>,
    cmd_tx: &Sender<Cmd>,
    incoming_tx: &Sender<RelayIncoming>,
    roster_tx: &Sender<RosterEvent>,
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
                Cmd::AcceptCh { ch } => {
                    if session.send(&C2s::Accept { ch }.encode()).is_err() {
                        dead = true;
                        break;
                    }
                }
                Cmd::Shutdown => {
                    dead = true;
                    break;
                }
                Cmd::Nop => {} // 생존 확인 — send 성공 자체가 답이다
                Cmd::Announce { listed, card } => {
                    // v2(카드) 먼저 — 신서버는 이걸로 싣고 뒤의 v1은 멱등 no-op,
                    // 구서버는 미지 kind로 버리고 v1로 싣는다(전방 호환 폴백).
                    if session
                        .send(&C2s::AnnounceCard { listed, card }.encode())
                        .is_err()
                        || session.send(&C2s::Announce { listed }.encode()).is_err()
                    {
                        dead = true;
                        break;
                    }
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
                    // ★ 여기서 Accept를 **보내지 않는다**(지연 수락) — 소비자가
                    // [`RelayIncoming::accept`]를 불러야 서버가 중계를 시작한다.
                    // 수락 전에 UDP 프로브를 마치면 서버의 OpenResult가 내 신선한
                    // 관측 엔드포인트를 상대에게 싣는다(홀펀칭 재료의 순서 보장).
                    let (tx, rx) = std::sync::mpsc::channel();
                    chans.insert(ch, tx);
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
                Some(S2c::PeerUp { peer }) => {
                    let _ = roster_tx.send(RosterEvent::Up(peer, Vec::new()));
                }
                Some(S2c::PeerUpCard { peer, card }) => {
                    let _ = roster_tx.send(RosterEvent::Up(peer, card));
                }
                Some(S2c::PeerDown { peer }) => {
                    let _ = roster_tx.send(RosterEvent::Down(peer));
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
/// 프레임이 오나"를 보고 단을 고른 뒤, 그 프레임을 핸드셰이크에 되살린다.
#[derive(Debug)]
struct PrefixedLink<L: Link> {
    first: Option<Vec<u8>>,
    inner: L,
}

impl<L: Link> Link for PrefixedLink<L> {
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
) -> Result<ViaSession, ViaError> {
    let day = current_epoch_day();
    let rids = [
        rid_for(dst, day),
        rid_for(dst, day.saturating_sub(1)),
        rid_for(dst, day + 1),
    ];
    // 프로브가 Open보다 **먼저** — 서버가 내 관측 엔드포인트를 상대의 Incoming에 싣는다.
    let sock = if punch {
        fresh_probed_sock(client).ok() // 실패 = 펀치 없이 릴레이만(사다리 한 단 생략)
    } else {
        None
    };
    for rid in rids {
        let (mut relay, peer_udp) = match client.open(rid, open_timeout) {
            Ok(v) => v,
            Err(1) => continue, // 이 에폭 RID는 미등록 — 다음 에폭
            Err(2) => return Err(ViaError::Limit),
            Err(_) => return Err(ViaError::Dead),
        };
        // 1단: 홀펀칭 UDP — 성립 + 종단 핸드셰이크까지 돼야 이 단을 탄 것이다.
        if let (Some(s), Some(ep)) = (&sock, peer_udp) {
            if let Ok(sc) = s.try_clone() {
                if let Ok(mut udp) = nbeep_net::UdpLink::punch(sc, ep, PUNCH_WINDOW) {
                    let _ = udp.set_recv_timeout(Some(HS_TIMEOUT_UDP));
                    let path = class_of_link(&udp, nbeep_core::PathClass::Remote);
                    let boxed: Box<dyn Link> = Box::new(udp);
                    if let Ok(session) = NoiseSession::initiate(boxed, id) {
                        return finish_via(session, PathTaken::Udp, path, dst);
                    }
                    // 상대가 UDP를 듣지 않았다(창 어긋남) — 릴레이 단으로 내려간다.
                }
            }
        }
        // 2단: 릴레이 폴백 — 서버 미상은 Remote 취급(fail-closed 방향).
        let _ = relay.set_recv_timeout(Some(HS_TIMEOUT_RELAY));
        let path = class_of_link(&relay, nbeep_core::PathClass::Remote);
        let boxed: Box<dyn Link> = Box::new(relay);
        return match NoiseSession::initiate(boxed, id) {
            Ok(session) => finish_via(session, PathTaken::Relay, path, dst),
            Err(_) => Err(ViaError::Handshake),
        };
    }
    Err(ViaError::NotFound)
}

fn finish_via(
    mut session: NoiseSession<Box<dyn Link>>,
    taken: PathTaken,
    path: nbeep_core::PathClass,
    dst: &PeerId,
) -> Result<ViaSession, ViaError> {
    use nbeep_core::Session as _;
    if session.peer() != *dst {
        return Err(ViaError::WrongPeer); // RID 충돌·오지정 — 인증된 키가 근거다
    }
    session.set_recv_timeout(None); // 핸드셰이크용 타임아웃 원복(소비자가 다시 건다)
    Ok(ViaSession {
        session,
        taken,
        path,
    })
}

/// **받는 쪽 사다리** — 인바운드 랑데부에 프로브→수락 순서를 지키고, 펀치를 병행하며
/// **첫 프레임이 온 링크**로 종단 수락을 잇는다(여는 쪽이 어느 단을 골랐든 따라간다).
///
/// # Errors
/// `deadline` 내 어느 링크로도 첫 프레임이 없거나 핸드셰이크 실패 시 [`ViaError`].
pub fn accept_via(
    client: &RelayClient,
    inc: RelayIncoming,
    id: &Identity,
    punch: bool,
    deadline: Duration,
) -> Result<ViaSession, ViaError> {
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
            let u = udp.take().expect("프레임을 준 링크");
            let path = class_of_link(&u, nbeep_core::PathClass::Remote);
            return accept_on(
                PrefixedLink {
                    first: Some(first),
                    inner: u,
                },
                id,
                PathTaken::Udp,
                path,
            );
        }
        // 릴레이 쪽 첫 프레임 — 여는 쪽이 릴레이 단을 골랐다(또는 펀치 실패).
        if relay_alive {
            match link.recv() {
                Ok(first) => {
                    let path = class_of_link(&link, nbeep_core::PathClass::Remote);
                    return accept_on(
                        PrefixedLink {
                            first: Some(first),
                            inner: link,
                        },
                        id,
                        PathTaken::Relay,
                        path,
                    );
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

fn accept_on<L: Link + 'static>(
    mut link: PrefixedLink<L>,
    id: &Identity,
    taken: PathTaken,
    path: nbeep_core::PathClass,
) -> Result<ViaSession, ViaError> {
    use nbeep_core::Session as _;
    let _ = link.set_recv_timeout(Some(HS_TIMEOUT_UDP));
    let boxed: Box<dyn Link> = Box::new(link);
    match NoiseSession::accept(boxed, id) {
        Ok(mut session) => {
            session.set_recv_timeout(None);
            Ok(ViaSession {
                session,
                taken,
                path,
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
) -> Result<Attached, AttachError> {
    let (addr, sa) = resolve_server(raw).ok_or(AttachError::Resolve)?;
    let expected = pinfile::lookup(pin_path, &addr);
    let client = RelayClient::connect(sa, id, &rids_around(&id.peer_id()), expected)
        .map_err(AttachError::Relay)?;
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
