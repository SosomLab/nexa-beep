//! **릴레이 와이어 v2 코덱**(ADR-0016 · [docs/56] · 11-07 사용자 결정 "기존 호환 포기 · 이후 확장성 확보").
//!
//! v1(`nbeep-relay` 안의 `C2s`/`S2c` · kind u8 + 고정 오프셋 본문)은 버전·능력·오류 통로가 없어 "모르는 kind는 조용히
//! 버린다"가 유일한 확장 수단이었다. v2는 **봉투 + TLV**로 바꾼다 — 서버·클라·nexa-clip이 같은 바이트 정의를 쓴다.
//!
//! ```text
//! 링크 프레임  = [len u32 BE][Envelope]              (len ≤ limits.max_frame · Welcome이 알린다 · 기본 65535)
//! Envelope     = kind u16 BE ‖ flags u8 ‖ Field*
//! Field(TLV)   = tag u8 ‖ len u32 BE ‖ value          (같은 tag 반복 허용 = 목록)
//! ```
//! - **kind 공간**: `0x0000..=0x3FFF` 핵심(모르면 [`ErrorCode::UnknownKind`]로 **응답**) · `0x4000..=0x7FFF` 확장(모르면
//!   조용히 무시 — flags의 [`FLAG_OPTIONAL`]과 같은 뜻) · `0x8000..` 예약. 방향은 번호로 가른다(C2S `0x00xx` · S2C `0x01xx`).
//! - **tag 규칙**: 모르는 tag는 **건너뛴다**(같은 major 안 전방 호환) · 필수 tag 누락 = [`ErrorCode::Malformed`].
//! - **major**는 Noise prologue([`PROLOGUE`])에 묶여 있다 — major가 다르면 핸드셰이크가 **명시적으로** 실패한다
//!   (v1처럼 "연결은 되는데 기능만 조용히 빠지는" 일이 없다). minor·능력은 [`Hello`]/[`Welcome`]로 교환.
//! - **앱 이름공간**: [`Hello::app`]가 연결을 앱에 묶고 서버는 `(app, rid)`로 라우팅한다 — beep·clip이 같은 서버를 써도
//!   RID 충돌·교차 Open이 구조적으로 없다(v1은 RID 파생 문자열에만 의존).
//! - 봉투 원리는 그대로: 서버는 `kind`·`rid`·`ch`·길이만 본다. `Data`·`Announce.card` 본문은 불투명.

#![forbid(unsafe_code)]

/// Noise prologue — **프로토콜 major**를 묶는다. major가 오르면 이 문자열이 바뀐다(= 구클라 핸드셰이크 실패 · 의도된 비호환).
pub const PROLOGUE: &[u8] = b"nexa-beepd/2";
/// 프로토콜 major(= prologue) · minor(같은 major 안 추가 kind/tag — 모르면 건너뜀).
pub const PROTO_MAJOR: u16 = 2;
/// 현재 minor.
pub const PROTO_MINOR: u16 = 0;
/// 봉투 머리 길이(kind 2 + flags 1).
pub const ENVELOPE_HEAD: usize = 3;
/// TLV 머리 길이(tag 1 + len 4).
pub const TLV_HEAD: usize = 5;
/// 핵심 kind 상한(이하 = 모르면 Error 응답 · 초과 = 확장 = 모르면 무시).
pub const KIND_CORE_MAX: u16 = 0x3FFF;
/// flags bit0 — 받는 쪽이 모르면 조용히 버려도 되는 봉투(확장 kind와 같은 뜻 · 핵심 kind에 붙이면 무시 허용).
pub const FLAG_OPTIONAL: u8 = 1 << 0;

/// 랑데부 RID(16B · v1과 같은 크기 · 파생은 앱 몫).
pub type Rid = [u8; 16];
/// 기기 공개키(= PeerId 바이트 · 명부 PeerUp/PeerDown이 나른다).
pub type PeerKey = [u8; 32];
/// RegisterResult / OpenResult / Incoming의 status "성공".
pub const STATUS_OK: u16 = 0;

/// `Hello.app` 이름공간 상수. 서버는 `(app, rid)`로 라우팅한다 — 교차 Open은 `Forbidden`.
pub mod app {
    /// nexa-beep 클라.
    pub const BEEP: &str = "nexa-beep";
    /// nexa-clip 클라(와이어 사본 · docs/44).
    pub const CLIP: &str = "nexa-clip";
    /// 헬스 프로브 — Welcome만 받고 끊는다(등록 불가 · 명부 비노출 · 같은 포트).
    pub const PROBE: &str = "probe";
}

/// **링크 프리앰블** — Noise보다 앞에 평문으로 오가는 8B. 세대(major) 판별 전용.
///
/// prologue만으로는 "major 불일치"와 "키 변조"가 **구분되지 않는다**(둘 다 Noise msg2 복호 실패) — 클라가 "서버 세대
/// 불일치 · 앱 업데이트 필요"를 **표시**하려면 Noise 밖에서 major를 알아야 한다. 보안 결속은 여전히 prologue가 맡는다
/// (프리앰블을 바꿔치기해도 핸드셰이크는 실패할 뿐 다운그레이드는 안 된다).
///
/// ```text
/// C → S : "NBPD" ‖ major u16 BE ‖ 0u16(예약)   (바로 이어서 Noise msg1 프레임 — RTT 추가 없음)
/// S → C : "NBPD" ‖ major u16 BE ‖ 0u16          (major가 다르면 이것만 보내고 끊는다)
/// ```
/// v1 클라는 프리앰블 없이 `[len u32 = 0x20][e]`로 시작하므로 서버는 첫 4B로 세대를 가를 수 있다(이중 수용 자리).
pub mod preamble {
    /// 매직 4B.
    pub const MAGIC: [u8; 4] = *b"NBPD";
    /// 길이(매직 4 + major 2 + 예약 2).
    pub const LEN: usize = 8;
    /// 프리앰블 바이트.
    #[must_use]
    pub fn encode(major: u16) -> [u8; LEN] {
        let m = major.to_be_bytes();
        [
            MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], m[0], m[1], 0, 0,
        ]
    }
    /// 매직이 맞으면 major. 아니면 `None`(= v1 또는 beepd가 아님).
    #[must_use]
    pub fn parse(b: &[u8]) -> Option<u16> {
        if b.len() < LEN || b[..4] != MAGIC {
            return None;
        }
        Some(u16::from_be_bytes([b[4], b[5]]))
    }
}

/// **UDP 관측 프로브** — Noise 밖 데이터그램(관측 주소를 알아내는 용도라 암호화 대상이 아니다 · 능력 [`caps::UDP_OBSERVE`]).
/// 형식은 v1과 같다(바꿀 이유가 없고 clip 사본이 그대로 맞는다). 정의만 여기로 모은다.
///
/// ```text
/// 요청  = "NBOB" ‖ token u64 BE                       (12B · 서버는 매직·토큰이 틀리면 **침묵** — 증폭 방지)
/// 응답  = "NBOB" ‖ token u64 BE ‖ ep                  (ep = 4‖ip4‖port · 6‖ip16‖port)
/// ```
pub mod udp_obs {
    use super::Ep;
    /// 매직 4B.
    pub const MAGIC: [u8; 4] = *b"NBOB";
    /// 요청 길이.
    pub const PROBE_LEN: usize = 12;
    /// 요청 바이트.
    #[must_use]
    pub fn encode_probe(token: u64) -> [u8; PROBE_LEN] {
        let mut out = [0u8; PROBE_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4..].copy_from_slice(&token.to_be_bytes());
        out
    }
    /// 요청 해석 — 매직·길이가 맞으면 토큰.
    #[must_use]
    pub fn parse_probe(b: &[u8]) -> Option<u64> {
        if b.len() < PROBE_LEN || b[..4] != MAGIC {
            return None;
        }
        Some(u64::from_be_bytes(b[4..12].try_into().ok()?))
    }
    /// 응답 바이트.
    #[must_use]
    pub fn encode_echo(token: u64, ep: Ep) -> Vec<u8> {
        let mut out = Vec::with_capacity(PROBE_LEN + 19);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&token.to_be_bytes());
        ep.encode(&mut out);
        out
    }
    /// 응답 해석 — (토큰, 관측 주소).
    #[must_use]
    pub fn parse_echo(b: &[u8]) -> Option<(u64, Ep)> {
        let token = parse_probe(b)?;
        let ep = Ep::decode(&b[PROBE_LEN..])?;
        if ep == Ep::None {
            return None;
        }
        Some((token, ep))
    }
}

/// kind 번호(v2.0). 번호는 **불변** · 새 kind는 뒤에 append.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    // ── C2S(0x00xx) ──
    /// 첫 프레임 — 앱·버전·능력.
    Hello = 0x0001,
    /// RID 등록(반복 tag) + 바인딩 모드.
    Register = 0x0002,
    /// 채널 열기 요청.
    Open = 0x0003,
    /// 인바운드 수락.
    Accept = 0x0004,
    /// 채널 본문(불투명).
    Data = 0x0005,
    /// 채널 닫기.
    CloseCh = 0x0006,
    /// 생존 신호.
    Ping = 0x0007,
    /// 명부 공개 여부 + 카드(불투명).
    Announce = 0x0008,
    /// 클라의 우아한 종료 예고.
    Goaway = 0x0009,
    // ── S2C(0x01xx) ──
    /// Hello 응답 — 서버 버전·종류·능력·상한·세션.
    Welcome = 0x0101,
    /// Hello 거절(major 불일치·앱 미허용 등) — 뒤에 끊는다.
    Reject = 0x0102,
    /// Register 결과(RID마다 status).
    RegisterResult = 0x0103,
    /// Open 결과.
    OpenResult = 0x0104,
    /// 인바운드 채널 통지.
    Incoming = 0x0105,
    /// 채널 본문(불투명).
    DataS = 0x0106,
    /// 채널 닫힘 통지.
    ChClosed = 0x0107,
    /// Ping 응답.
    Pong = 0x0108,
    /// 명부 입장.
    PeerUp = 0x0109,
    /// 명부 퇴장.
    PeerDown = 0x010A,
    /// 범용 오류(요청 kind 참조 · 코드 · 사유).
    Error = 0x010B,
    /// 서버의 우아한 종료 예고(재접속 지연 힌트).
    GoawayS = 0x010C,
}

impl Kind {
    /// 번호 → kind(모르는 번호 = None — 호출자가 핵심/확장 공간으로 처리).
    #[must_use]
    pub fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            0x0001 => Self::Hello,
            0x0002 => Self::Register,
            0x0003 => Self::Open,
            0x0004 => Self::Accept,
            0x0005 => Self::Data,
            0x0006 => Self::CloseCh,
            0x0007 => Self::Ping,
            0x0008 => Self::Announce,
            0x0009 => Self::Goaway,
            0x0101 => Self::Welcome,
            0x0102 => Self::Reject,
            0x0103 => Self::RegisterResult,
            0x0104 => Self::OpenResult,
            0x0105 => Self::Incoming,
            0x0106 => Self::DataS,
            0x0107 => Self::ChClosed,
            0x0108 => Self::Pong,
            0x0109 => Self::PeerUp,
            0x010A => Self::PeerDown,
            0x010B => Self::Error,
            0x010C => Self::GoawayS,
            _ => return None,
        })
    }
}

/// 범용 오류 코드(u16 · 불변 · append).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorCode {
    /// 핵심 공간의 모르는 kind.
    UnknownKind = 1,
    /// 필수 tag 누락·길이 손상.
    Malformed = 2,
    /// 이 minor/능력에서 지원하지 않는 요청.
    Unsupported = 3,
    /// 상한 초과(프레임·RID 수·채널 수·큐).
    LimitExceeded = 4,
    /// Hello/Register 전 요청.
    NotRegistered = 5,
    /// 대상 RID 없음.
    NoSuchPeer = 6,
    /// 상대가 바쁨(채널 상한).
    Busy = 7,
    /// 허용되지 않음(앱 교차 Open 등).
    Forbidden = 8,
    /// 서버 종료 중.
    Shutdown = 9,
    /// 프로토콜 major 불일치(Reject 전용).
    VersionMismatch = 10,
}

impl ErrorCode {
    #[must_use]
    pub fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            1 => Self::UnknownKind,
            2 => Self::Malformed,
            3 => Self::Unsupported,
            4 => Self::LimitExceeded,
            5 => Self::NotRegistered,
            6 => Self::NoSuchPeer,
            7 => Self::Busy,
            8 => Self::Forbidden,
            9 => Self::Shutdown,
            10 => Self::VersionMismatch,
            _ => return None,
        })
    }
}

/// 서버 종류(ADR-0013 — 릴레이/컨텐츠/관리) — Welcome이 알린다(X-2b "타입 협상" 와이어).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ServerType {
    Relay = 1,
    Content = 2,
    Admin = 3,
}

/// RID 바인딩 모드(docs/44 I-2 "한 RID에 여러 기기").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BindMode {
    /// 나중 등록이 앞 것을 밀어낸다(v1 동작).
    ExclusiveLastWins = 0,
    /// 여러 연결이 같은 RID를 공유 — Incoming은 전부에게(팬아웃).
    Shared = 1,
}

/// 능력 비트(u64 · 불변 · append) — Hello(클라)·Welcome(서버) 양쪽이 보내고 교집합이 세션 능력.
pub mod caps {
    /// 명부(Announce/PeerUp/PeerDown).
    pub const ROSTER: u64 = 1 << 0;
    /// 공유 바인딩(BindMode::Shared).
    pub const SHARED_BIND: u64 = 1 << 1;
    /// UDP 관측 프로브(ep 되비춤).
    pub const UDP_OBSERVE: u64 = 1 << 2;
    /// 컨텐츠 모드(ADR-0013 · S5 — 예약).
    pub const CONTENT: u64 = 1 << 3;
}

/// 서버 상한(Welcome · tag 6 중첩 TLV) — 종전 하드코딩(65535 · RID 8 · 채널 64 · 큐 8MiB · 1MiB/s)을 협상 값으로.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_frame: u32,
    pub max_rids: u16,
    pub max_chans: u16,
    pub queue_bytes: u32,
    pub rate_bps: u64,
    pub ping_interval_s: u16,
    pub card_max: u16,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame: 65_535,
            max_rids: 8,
            max_chans: 64,
            queue_bytes: 8 * 1024 * 1024,
            rate_bps: 1024 * 1024,
            ping_interval_s: 20,
            card_max: 1024,
        }
    }
}

/// 주소(ep) — `[]`(없음) · `4 ‖ ip4 ‖ port` · `6 ‖ ip16 ‖ port`(v1과 같은 배치).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ep {
    None,
    V4([u8; 4], u16),
    V6([u8; 16], u16),
}

impl Ep {
    /// 와이어 배치로 덧붙인다(`None`이면 0B).
    pub fn encode(self, out: &mut Vec<u8>) {
        match self {
            Self::None => {}
            Self::V4(ip, port) => {
                out.push(4);
                out.extend_from_slice(&ip);
                out.extend_from_slice(&port.to_be_bytes());
            }
            Self::V6(ip, port) => {
                out.push(6);
                out.extend_from_slice(&ip);
                out.extend_from_slice(&port.to_be_bytes());
            }
        }
    }
    /// 와이어 배치 해석(빈 슬라이스 = `None` · 길이가 어긋나면 `None` 반환이 아니라 **오류**(`Option::None`)).
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        match b.first() {
            None => Some(Self::None),
            Some(4) if b.len() == 7 => Some(Self::V4(b[1..5].try_into().ok()?, be16(&b[5..7])?)),
            Some(6) if b.len() == 19 => {
                Some(Self::V6(b[1..17].try_into().ok()?, be16(&b[17..19])?))
            }
            _ => None,
        }
    }
}

// ───────────────────────── TLV 원시 ─────────────────────────

/// 봉투 디코드 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// 봉투 머리보다 짧음·TLV 길이가 바깥을 벗어남.
    Truncated,
    /// 필수 tag 누락·값 길이 불일치.
    Malformed,
    /// 핵심 공간의 모르는 kind(응답 Error 대상).
    UnknownCoreKind(u16),
    /// 확장 공간의 모르는 kind(조용히 무시 대상).
    UnknownExtKind(u16),
    /// typed decode에 다른 kind의 봉투가 들어옴(호출자 분기 오류 · Malformed로 응답).
    KindMismatch(u16),
}

impl DecodeError {
    /// 상대에게 돌려줄 오류 코드. 확장 kind는 응답 대상이 아니므로 호출자가 먼저 걸러야 한다(여기선 `Unsupported`).
    #[must_use]
    pub fn error_code(self) -> ErrorCode {
        match self {
            Self::UnknownCoreKind(_) => ErrorCode::UnknownKind,
            Self::UnknownExtKind(_) => ErrorCode::Unsupported,
            Self::Truncated | Self::Malformed | Self::KindMismatch(_) => ErrorCode::Malformed,
        }
    }
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated"),
            Self::Malformed => f.write_str("malformed"),
            Self::UnknownCoreKind(k) => write!(f, "unknown core kind 0x{k:04x}"),
            Self::UnknownExtKind(k) => write!(f, "unknown ext kind 0x{k:04x}"),
            Self::KindMismatch(k) => write!(f, "kind mismatch 0x{k:04x}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// TLV 기록기.
#[derive(Debug, Default)]
pub struct Fields(Vec<u8>);

impl Fields {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub fn bytes(&mut self, tag: u8, v: &[u8]) -> &mut Self {
        self.0.push(tag);
        self.0
            .extend_from_slice(&u32::try_from(v.len()).unwrap_or(u32::MAX).to_be_bytes());
        self.0.extend_from_slice(v);
        self
    }
    pub fn u8(&mut self, tag: u8, v: u8) -> &mut Self {
        self.bytes(tag, &[v])
    }
    pub fn u16(&mut self, tag: u8, v: u16) -> &mut Self {
        self.bytes(tag, &v.to_be_bytes())
    }
    pub fn u32(&mut self, tag: u8, v: u32) -> &mut Self {
        self.bytes(tag, &v.to_be_bytes())
    }
    pub fn u64(&mut self, tag: u8, v: u64) -> &mut Self {
        self.bytes(tag, &v.to_be_bytes())
    }
    pub fn str(&mut self, tag: u8, v: &str) -> &mut Self {
        self.bytes(tag, v.as_bytes())
    }
    pub fn ep(&mut self, tag: u8, v: Ep) -> &mut Self {
        let mut b = Vec::with_capacity(19);
        v.encode(&mut b);
        self.bytes(tag, &b)
    }
    /// 중첩 TLV(값 = 또 다른 Fields).
    pub fn nested(&mut self, tag: u8, v: &Fields) -> &mut Self {
        self.bytes(tag, &v.0)
    }
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// TLV 해석기 — 모르는 tag는 건너뛴다 · 같은 tag 반복은 `all`로.
#[derive(Clone, Copy, Debug)]
pub struct View<'a>(&'a [u8]);

impl<'a> View<'a> {
    /// 전체 TLV 구조 검증(길이가 바깥을 벗어나면 Truncated).
    pub fn parse(b: &'a [u8]) -> Result<Self, DecodeError> {
        let mut i = 0usize;
        while i < b.len() {
            if b.len() - i < TLV_HEAD {
                return Err(DecodeError::Truncated);
            }
            let len = be32(&b[i + 1..i + 5]).ok_or(DecodeError::Truncated)? as usize;
            i = i
                .checked_add(TLV_HEAD + len)
                .ok_or(DecodeError::Truncated)?;
            if i > b.len() {
                return Err(DecodeError::Truncated);
            }
        }
        Ok(Self(b))
    }
    /// 첫 번째 값.
    #[must_use]
    pub fn get(&self, tag: u8) -> Option<&'a [u8]> {
        self.all(tag).next()
    }
    /// 같은 tag 전부(반복 = 목록).
    pub fn all(&self, tag: u8) -> impl Iterator<Item = &'a [u8]> + '_ {
        let b = self.0;
        let mut i = 0usize;
        std::iter::from_fn(move || {
            while i + TLV_HEAD <= b.len() {
                let t = b[i];
                let len = be32(&b[i + 1..i + 5])? as usize;
                let s = i + TLV_HEAD;
                i = s + len;
                if t == tag {
                    return b.get(s..s + len);
                }
            }
            None
        })
    }
    pub fn req(&self, tag: u8) -> Result<&'a [u8], DecodeError> {
        self.get(tag).ok_or(DecodeError::Malformed)
    }
    pub fn req_u8(&self, tag: u8) -> Result<u8, DecodeError> {
        match self.req(tag)? {
            [v] => Ok(*v),
            _ => Err(DecodeError::Malformed),
        }
    }
    pub fn req_u16(&self, tag: u8) -> Result<u16, DecodeError> {
        be16(self.req(tag)?).ok_or(DecodeError::Malformed)
    }
    pub fn req_u32(&self, tag: u8) -> Result<u32, DecodeError> {
        be32(self.req(tag)?).ok_or(DecodeError::Malformed)
    }
    pub fn req_u64(&self, tag: u8) -> Result<u64, DecodeError> {
        be64(self.req(tag)?).ok_or(DecodeError::Malformed)
    }
    pub fn req_rid(&self, tag: u8) -> Result<Rid, DecodeError> {
        self.req(tag)?
            .try_into()
            .map_err(|_| DecodeError::Malformed)
    }
    pub fn opt_u16(&self, tag: u8) -> Result<Option<u16>, DecodeError> {
        self.get(tag)
            .map(|b| be16(b).ok_or(DecodeError::Malformed))
            .transpose()
    }
    pub fn opt_u64(&self, tag: u8) -> Result<Option<u64>, DecodeError> {
        self.get(tag)
            .map(|b| be64(b).ok_or(DecodeError::Malformed))
            .transpose()
    }
    pub fn opt_str(&self, tag: u8) -> Result<Option<String>, DecodeError> {
        self.get(tag)
            .map(|b| {
                std::str::from_utf8(b)
                    .map(str::to_string)
                    .map_err(|_| DecodeError::Malformed)
            })
            .transpose()
    }
    pub fn opt_ep(&self, tag: u8) -> Result<Ep, DecodeError> {
        self.get(tag).map_or(Ok(Ep::None), |b| {
            Ep::decode(b).ok_or(DecodeError::Malformed)
        })
    }
}

fn be16(b: &[u8]) -> Option<u16> {
    Some(u16::from_be_bytes(b.try_into().ok()?))
}
fn be32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.try_into().ok()?))
}
fn be64(b: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(b.try_into().ok()?))
}

// ───────────────────────── 봉투 ─────────────────────────

/// 봉투(kind + flags + TLV 본문) — 링크 프레임의 payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub kind: u16,
    pub flags: u8,
    pub body: Vec<u8>,
}

impl Envelope {
    #[must_use]
    pub fn new(kind: Kind, body: Fields) -> Self {
        Self {
            kind: kind as u16,
            flags: 0,
            body: body.into_bytes(),
        }
    }
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(ENVELOPE_HEAD + self.body.len());
        v.extend_from_slice(&self.kind.to_be_bytes());
        v.push(self.flags);
        v.extend_from_slice(&self.body);
        v
    }
    /// 봉투 머리 해석 + TLV 구조 검증. 모르는 kind는 공간에 따라 오류 종류가 다르다(핵심 = 응답 · 확장 = 무시).
    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        if b.len() < ENVELOPE_HEAD {
            return Err(DecodeError::Truncated);
        }
        let kind = u16::from_be_bytes([b[0], b[1]]);
        let flags = b[2];
        let body = &b[ENVELOPE_HEAD..];
        View::parse(body)?;
        if Kind::from_u16(kind).is_none() {
            return Err(if kind <= KIND_CORE_MAX && flags & FLAG_OPTIONAL == 0 {
                DecodeError::UnknownCoreKind(kind)
            } else {
                DecodeError::UnknownExtKind(kind)
            });
        }
        Ok(Self {
            kind,
            flags,
            body: body.to_vec(),
        })
    }
    #[must_use]
    pub fn kind(&self) -> Option<Kind> {
        Kind::from_u16(self.kind)
    }
    pub fn view(&self) -> Result<View<'_>, DecodeError> {
        View::parse(&self.body)
    }
    /// typed decode 앞 kind 확인 — 허용 목록에 없으면 [`DecodeError::KindMismatch`].
    pub fn expect(&self, allowed: &[Kind]) -> Result<(), DecodeError> {
        match self.kind() {
            Some(k) if allowed.contains(&k) => Ok(()),
            _ => Err(DecodeError::KindMismatch(self.kind)),
        }
    }
}

// ───────────────────────── 핵심 메시지(typed) ─────────────────────────

/// tag 번호(kind별 · 불변 · append). 공통 관례: 1부터 · 반복 가능한 tag는 주석에 명시.
pub mod tag {
    pub const PROTO_MAJOR: u8 = 1;
    pub const PROTO_MINOR: u8 = 2;
    pub const APP: u8 = 3;
    pub const APP_VERSION: u8 = 4;
    pub const CAPS: u8 = 5;
    pub const LIMITS: u8 = 6;
    pub const SESSION_ID: u8 = 7;
    pub const UDP_PORT: u8 = 8;
    pub const EP: u8 = 9;
    pub const SERVER_TYPE: u8 = 10;
    pub const SERVER_VERSION: u8 = 11;
    pub const CODE: u8 = 12;
    pub const REASON: u8 = 13;
    pub const RID: u8 = 14; // 반복
    pub const BIND_MODE: u8 = 15;
    pub const ENTRY: u8 = 16; // 반복(중첩: RID + STATUS)
    pub const STATUS: u8 = 17;
    pub const UDP_TOKEN: u8 = 18;
    pub const TOKEN: u8 = 19;
    pub const CH: u8 = 20;
    pub const FIN: u8 = 21;
    pub const BYTES: u8 = 22;
    pub const LISTED: u8 = 23;
    pub const CARD: u8 = 24;
    pub const NONCE: u8 = 25;
    pub const REF_KIND: u8 = 26;
    pub const RETRY_AFTER_S: u8 = 27;
    pub const MIN_PROTO: u8 = 28;
    pub const PEER: u8 = 29; // 32B 공개키(PeerUp/PeerDown)
    // limits 중첩 tag
    pub const L_MAX_FRAME: u8 = 1;
    pub const L_MAX_RIDS: u8 = 2;
    pub const L_MAX_CHANS: u8 = 3;
    pub const L_QUEUE_BYTES: u8 = 4;
    pub const L_RATE_BPS: u8 = 5;
    pub const L_PING_S: u8 = 6;
    pub const L_CARD_MAX: u8 = 7;
}

/// C2S Hello — 첫 프레임. `app`이 이름공간(`"nexa-beep"` · `"nexa-clip"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub proto_major: u16,
    pub proto_minor: u16,
    pub app: String,
    pub app_version: String,
    pub caps: u64,
}

impl Hello {
    #[must_use]
    pub fn encode(&self) -> Envelope {
        let mut f = Fields::new();
        f.u16(tag::PROTO_MAJOR, self.proto_major)
            .u16(tag::PROTO_MINOR, self.proto_minor)
            .str(tag::APP, &self.app)
            .str(tag::APP_VERSION, &self.app_version)
            .u64(tag::CAPS, self.caps);
        Envelope::new(Kind::Hello, f)
    }
    pub fn decode(e: &Envelope) -> Result<Self, DecodeError> {
        e.expect(&[Kind::Hello])?;
        let v = e.view()?;
        Ok(Self {
            proto_major: v.req_u16(tag::PROTO_MAJOR)?,
            proto_minor: v.req_u16(tag::PROTO_MINOR)?,
            app: v.opt_str(tag::APP)?.ok_or(DecodeError::Malformed)?,
            app_version: v.opt_str(tag::APP_VERSION)?.unwrap_or_default(),
            caps: v.opt_u64(tag::CAPS)?.unwrap_or(0),
        })
    }
}

/// S2C Welcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub proto_major: u16,
    pub proto_minor: u16,
    pub server_version: String,
    pub server_type: ServerType,
    pub caps: u64,
    pub limits: Limits,
    pub session_id: [u8; 16],
    pub udp_port: u16,
    pub observed: Ep,
}

impl Welcome {
    #[must_use]
    pub fn encode(&self) -> Envelope {
        let mut l = Fields::new();
        l.u32(tag::L_MAX_FRAME, self.limits.max_frame)
            .u16(tag::L_MAX_RIDS, self.limits.max_rids)
            .u16(tag::L_MAX_CHANS, self.limits.max_chans)
            .u32(tag::L_QUEUE_BYTES, self.limits.queue_bytes)
            .u64(tag::L_RATE_BPS, self.limits.rate_bps)
            .u16(tag::L_PING_S, self.limits.ping_interval_s)
            .u16(tag::L_CARD_MAX, self.limits.card_max);
        let mut f = Fields::new();
        f.u16(tag::PROTO_MAJOR, self.proto_major)
            .u16(tag::PROTO_MINOR, self.proto_minor)
            .str(tag::SERVER_VERSION, &self.server_version)
            .u8(tag::SERVER_TYPE, self.server_type as u8)
            .u64(tag::CAPS, self.caps)
            .nested(tag::LIMITS, &l)
            .bytes(tag::SESSION_ID, &self.session_id)
            .u16(tag::UDP_PORT, self.udp_port)
            .ep(tag::EP, self.observed);
        Envelope::new(Kind::Welcome, f)
    }
    pub fn decode(e: &Envelope) -> Result<Self, DecodeError> {
        e.expect(&[Kind::Welcome])?;
        let v = e.view()?;
        let d = Limits::default();
        let limits = match v.get(tag::LIMITS) {
            Some(b) => {
                let l = View::parse(b)?;
                Limits {
                    max_frame: l
                        .get(tag::L_MAX_FRAME)
                        .and_then(be32)
                        .unwrap_or(d.max_frame),
                    max_rids: l.get(tag::L_MAX_RIDS).and_then(be16).unwrap_or(d.max_rids),
                    max_chans: l
                        .get(tag::L_MAX_CHANS)
                        .and_then(be16)
                        .unwrap_or(d.max_chans),
                    queue_bytes: l
                        .get(tag::L_QUEUE_BYTES)
                        .and_then(be32)
                        .unwrap_or(d.queue_bytes),
                    rate_bps: l.get(tag::L_RATE_BPS).and_then(be64).unwrap_or(d.rate_bps),
                    ping_interval_s: l
                        .get(tag::L_PING_S)
                        .and_then(be16)
                        .unwrap_or(d.ping_interval_s),
                    card_max: l.get(tag::L_CARD_MAX).and_then(be16).unwrap_or(d.card_max),
                }
            }
            None => d,
        };
        let server_type = match v.req_u8(tag::SERVER_TYPE)? {
            1 => ServerType::Relay,
            2 => ServerType::Content,
            3 => ServerType::Admin,
            _ => return Err(DecodeError::Malformed),
        };
        Ok(Self {
            proto_major: v.req_u16(tag::PROTO_MAJOR)?,
            proto_minor: v.req_u16(tag::PROTO_MINOR)?,
            server_version: v.opt_str(tag::SERVER_VERSION)?.unwrap_or_default(),
            server_type,
            caps: v.opt_u64(tag::CAPS)?.unwrap_or(0),
            limits,
            session_id: v
                .req(tag::SESSION_ID)?
                .try_into()
                .map_err(|_| DecodeError::Malformed)?,
            udp_port: v.opt_u16(tag::UDP_PORT)?.unwrap_or(0),
            observed: v.opt_ep(tag::EP)?,
        })
    }
}

/// S2C Reject / Error / Goaway 공통 본문(code + reason [+ ref_kind / retry_after / min_proto]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub code: u16,
    pub reason: String,
    pub ref_kind: Option<u16>,
    pub retry_after_s: Option<u16>,
    pub min_proto: Option<u16>,
}

impl Problem {
    #[must_use]
    pub fn new(code: ErrorCode, reason: &str) -> Self {
        Self {
            code: code as u16,
            reason: reason.to_string(),
            ref_kind: None,
            retry_after_s: None,
            min_proto: None,
        }
    }
    #[must_use]
    pub fn encode(&self, kind: Kind) -> Envelope {
        let mut f = Fields::new();
        f.u16(tag::CODE, self.code).str(tag::REASON, &self.reason);
        if let Some(k) = self.ref_kind {
            f.u16(tag::REF_KIND, k);
        }
        if let Some(s) = self.retry_after_s {
            f.u16(tag::RETRY_AFTER_S, s);
        }
        if let Some(m) = self.min_proto {
            f.u16(tag::MIN_PROTO, m);
        }
        Envelope::new(kind, f)
    }
    pub fn decode(e: &Envelope) -> Result<Self, DecodeError> {
        e.expect(&[Kind::Reject, Kind::Error, Kind::Goaway, Kind::GoawayS])?;
        let v = e.view()?;
        Ok(Self {
            code: v.req_u16(tag::CODE)?,
            reason: v.opt_str(tag::REASON)?.unwrap_or_default(),
            ref_kind: v.opt_u16(tag::REF_KIND)?,
            retry_after_s: v.opt_u16(tag::RETRY_AFTER_S)?,
            min_proto: v.opt_u16(tag::MIN_PROTO)?,
        })
    }
}

/// C2S Register — RID 목록(반복 tag) + 바인딩 모드.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Register {
    pub rids: Vec<Rid>,
    pub bind: BindMode,
}

impl Register {
    #[must_use]
    pub fn encode(&self) -> Envelope {
        let mut f = Fields::new();
        for r in &self.rids {
            f.bytes(tag::RID, r);
        }
        f.u8(tag::BIND_MODE, self.bind as u8);
        Envelope::new(Kind::Register, f)
    }
    pub fn decode(e: &Envelope) -> Result<Self, DecodeError> {
        e.expect(&[Kind::Register])?;
        let v = e.view()?;
        let rids = v
            .all(tag::RID)
            .map(|b| Rid::try_from(b).map_err(|_| DecodeError::Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        let bind = match v.get(tag::BIND_MODE) {
            None | Some([0]) => BindMode::ExclusiveLastWins,
            Some([1]) => BindMode::Shared,
            _ => return Err(DecodeError::Malformed),
        };
        Ok(Self { rids, bind })
    }
}

/// S2C RegisterResult — RID마다 status(0 = ok · 그 외 = ErrorCode) + UDP 토큰.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterResult {
    pub entries: Vec<(Rid, u16)>,
    pub udp_token: u64,
}

impl RegisterResult {
    #[must_use]
    pub fn encode(&self) -> Envelope {
        let mut f = Fields::new();
        for (rid, st) in &self.entries {
            let mut e = Fields::new();
            e.bytes(tag::RID, rid).u16(tag::STATUS, *st);
            f.nested(tag::ENTRY, &e);
        }
        f.u64(tag::UDP_TOKEN, self.udp_token);
        Envelope::new(Kind::RegisterResult, f)
    }
    pub fn decode(e: &Envelope) -> Result<Self, DecodeError> {
        e.expect(&[Kind::RegisterResult])?;
        let v = e.view()?;
        let mut entries = Vec::new();
        for b in v.all(tag::ENTRY) {
            let x = View::parse(b)?;
            entries.push((x.req_rid(tag::RID)?, x.req_u16(tag::STATUS)?));
        }
        Ok(Self {
            entries,
            udp_token: v.opt_u64(tag::UDP_TOKEN)?.unwrap_or(0),
        })
    }
}

/// 채널 메시지 묶음 — Open/OpenResult/Accept/Incoming/Data/CloseCh/ChClosed(필드가 작아 한 타입으로).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub token: Option<u32>,
    pub ch: Option<u32>,
    pub rid: Option<Rid>,
    pub status: Option<u16>,
    pub ep: Ep,
    pub fin: bool,
    pub bytes: Vec<u8>,
}

impl Channel {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            token: None,
            ch: None,
            rid: None,
            status: None,
            ep: Ep::None,
            fin: false,
            bytes: Vec::new(),
        }
    }
    #[must_use]
    pub fn encode(&self, kind: Kind) -> Envelope {
        let mut f = Fields::new();
        if let Some(t) = self.token {
            f.u32(tag::TOKEN, t);
        }
        if let Some(c) = self.ch {
            f.u32(tag::CH, c);
        }
        if let Some(r) = &self.rid {
            f.bytes(tag::RID, r);
        }
        if let Some(s) = self.status {
            f.u16(tag::STATUS, s);
        }
        if self.ep != Ep::None {
            f.ep(tag::EP, self.ep);
        }
        if self.fin {
            f.u8(tag::FIN, 1);
        }
        if !self.bytes.is_empty() {
            f.bytes(tag::BYTES, &self.bytes);
        }
        Envelope::new(kind, f)
    }
    /// 채널 kind 전부.
    pub const KINDS: [Kind; 8] = [
        Kind::Open,
        Kind::OpenResult,
        Kind::Accept,
        Kind::Incoming,
        Kind::Data,
        Kind::DataS,
        Kind::CloseCh,
        Kind::ChClosed,
    ];
    pub fn decode(e: &Envelope) -> Result<Self, DecodeError> {
        e.expect(&Self::KINDS)?;
        let v = e.view()?;
        Ok(Self {
            token: v
                .get(tag::TOKEN)
                .map(|b| be32(b).ok_or(DecodeError::Malformed))
                .transpose()?,
            ch: v
                .get(tag::CH)
                .map(|b| be32(b).ok_or(DecodeError::Malformed))
                .transpose()?,
            rid: v
                .get(tag::RID)
                .map(|b| Rid::try_from(b).map_err(|_| DecodeError::Malformed))
                .transpose()?,
            status: v.opt_u16(tag::STATUS)?,
            ep: v.opt_ep(tag::EP)?,
            fin: v.get(tag::FIN).is_some_and(|b| b == [1]),
            bytes: v.get(tag::BYTES).map(<[u8]>::to_vec).unwrap_or_default(),
        })
    }
}

/// Announce / PeerUp / PeerDown — listed + 불투명 카드(상한 = limits.card_max) + peer 공개키·rid(PeerUp/Down).
///
/// 명부 항목의 신원은 `peer`(Noise가 증명한 정적 공개키 · 서버가 채운다)다 — 클라는 이 키로 RID를 파생해 Open한다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presence {
    pub listed: bool,
    pub peer: Option<PeerKey>,
    pub rid: Option<Rid>,
    pub card: Vec<u8>,
}

impl Presence {
    /// 명부 kind 전부.
    pub const KINDS: [Kind; 3] = [Kind::Announce, Kind::PeerUp, Kind::PeerDown];
    #[must_use]
    pub fn encode(&self, kind: Kind) -> Envelope {
        let mut f = Fields::new();
        f.u8(tag::LISTED, u8::from(self.listed));
        if let Some(p) = &self.peer {
            f.bytes(tag::PEER, p);
        }
        if let Some(r) = &self.rid {
            f.bytes(tag::RID, r);
        }
        if !self.card.is_empty() {
            f.bytes(tag::CARD, &self.card);
        }
        Envelope::new(kind, f)
    }
    pub fn decode(e: &Envelope) -> Result<Self, DecodeError> {
        e.expect(&Self::KINDS)?;
        let v = e.view()?;
        Ok(Self {
            listed: v.get(tag::LISTED).is_some_and(|b| b == [1]),
            peer: v
                .get(tag::PEER)
                .map(|b| PeerKey::try_from(b).map_err(|_| DecodeError::Malformed))
                .transpose()?,
            rid: v
                .get(tag::RID)
                .map(|b| Rid::try_from(b).map_err(|_| DecodeError::Malformed))
                .transpose()?,
            card: v.get(tag::CARD).map(<[u8]>::to_vec).unwrap_or_default(),
        })
    }
}

/// Ping/Pong — nonce(되비춤).
#[must_use]
pub fn ping(kind: Kind, nonce: u64) -> Envelope {
    let mut f = Fields::new();
    f.u64(tag::NONCE, nonce);
    Envelope::new(kind, f)
}

/// Ping/Pong의 nonce.
pub fn nonce(e: &Envelope) -> Result<u64, DecodeError> {
    e.expect(&[Kind::Ping, Kind::Pong])?;
    e.view()?.req_u64(tag::NONCE)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// 골든 벡터(docs/56 §8 — nexa-clip 사본이 같은 바이트를 내야 한다). 바꾸면 minor가 아니라 **major**다.
    #[test]
    fn golden_vectors_are_stable() {
        let h = Hello {
            proto_major: 2,
            proto_minor: 0,
            app: "nexa-beep".into(),
            app_version: "0.10.0".into(),
            caps: caps::ROSTER | caps::UDP_OBSERVE,
        }
        .encode()
        .encode();
        assert_eq!(
            hex(&h),
            "000100\
             01000000020002\
             02000000020000\
             03000000096e6578612d62656570\
             0400000006302e31302e30\
             05000000080000000000000005"
        );
        let p = ping(Kind::Ping, 0x0102_0304_0506_0708).encode();
        assert_eq!(hex(&p), "00070019000000080102030405060708");
    }

    /// 전 핵심 kind 왕복 + 모르는 tag 건너뜀 + 모르는 kind 공간 판정 + 절단.
    #[test]
    fn roundtrip_skip_unknown_tag_and_kind_spaces() {
        let w = Welcome {
            proto_major: 2,
            proto_minor: 3,
            server_version: "1.0.0".into(),
            server_type: ServerType::Relay,
            caps: caps::ROSTER,
            limits: Limits {
                max_frame: 1 << 20,
                ..Limits::default()
            },
            session_id: [7u8; 16],
            udp_port: 47300,
            observed: Ep::V4([10, 0, 0, 2], 5000),
        };
        let mut e = w.encode();
        assert_eq!(Welcome::decode(&e).unwrap(), w);
        // 미래 minor의 모르는 tag(200)를 끼워 넣어도 같은 결과.
        let mut extra = Fields::new();
        extra.bytes(200, b"future");
        e.body.extend_from_slice(&extra.into_bytes());
        assert_eq!(
            Welcome::decode(&Envelope::decode(&e.encode()).unwrap()).unwrap(),
            w
        );
        let r = Register {
            rids: vec![[1u8; 16], [2u8; 16]],
            bind: BindMode::Shared,
        };
        assert_eq!(Register::decode(&r.encode()).unwrap(), r);
        let rr = RegisterResult {
            entries: vec![([1u8; 16], 0), ([2u8; 16], ErrorCode::LimitExceeded as u16)],
            udp_token: 99,
        };
        assert_eq!(RegisterResult::decode(&rr.encode()).unwrap(), rr);
        let mut c = Channel::empty();
        c.ch = Some(5);
        c.fin = true;
        c.bytes = vec![9u8; 1000];
        assert_eq!(Channel::decode(&c.encode(Kind::Data)).unwrap(), c);
        let pr = Problem::new(ErrorCode::Forbidden, "cross-app open");
        assert_eq!(Problem::decode(&pr.encode(Kind::Error)).unwrap(), pr);
        let ps = Presence {
            listed: true,
            peer: Some([0xAB; 32]),
            rid: Some([3u8; 16]),
            card: b"card".to_vec(),
        };
        assert_eq!(Presence::decode(&ps.encode(Kind::PeerUp)).unwrap(), ps);
        // typed decode는 kind를 확인한다(채널 봉투를 Presence로 풀면 KindMismatch).
        assert_eq!(
            Presence::decode(&c.encode(Kind::Data)),
            Err(DecodeError::KindMismatch(Kind::Data as u16))
        );
        assert_eq!(
            Hello::decode(&ps.encode(Kind::PeerUp)),
            Err(DecodeError::KindMismatch(Kind::PeerUp as u16))
        );
        assert_eq!(nonce(&ping(Kind::Pong, 77)).unwrap(), 77);
        assert_eq!(
            DecodeError::UnknownCoreKind(9).error_code(),
            ErrorCode::UnknownKind
        );
        // kind 공간.
        let core_unknown = Envelope {
            kind: 0x0050,
            flags: 0,
            body: Vec::new(),
        }
        .encode();
        assert_eq!(
            Envelope::decode(&core_unknown),
            Err(DecodeError::UnknownCoreKind(0x0050))
        );
        let ext_unknown = Envelope {
            kind: 0x4001,
            flags: 0,
            body: Vec::new(),
        }
        .encode();
        assert_eq!(
            Envelope::decode(&ext_unknown),
            Err(DecodeError::UnknownExtKind(0x4001))
        );
        let core_optional = Envelope {
            kind: 0x0050,
            flags: FLAG_OPTIONAL,
            body: Vec::new(),
        }
        .encode();
        assert_eq!(
            Envelope::decode(&core_optional),
            Err(DecodeError::UnknownExtKind(0x0050))
        );
        // 절단·누락.
        let full = ping(Kind::Ping, 1).encode();
        assert_eq!(
            Envelope::decode(&full[..full.len() - 1]),
            Err(DecodeError::Truncated)
        );
        assert_eq!(Envelope::decode(&full[..2]), Err(DecodeError::Truncated));
        let no_app = Envelope::new(Kind::Hello, {
            let mut f = Fields::new();
            f.u16(tag::PROTO_MAJOR, 2).u16(tag::PROTO_MINOR, 0);
            f
        });
        assert_eq!(Hello::decode(&no_app), Err(DecodeError::Malformed));
    }

    /// 프리앰블·UDP 프로브는 Noise 밖 바이트 — 형식이 고정이다(clip 사본 검증 대상).
    #[test]
    fn preamble_and_udp_probe_are_stable() {
        let p = preamble::encode(PROTO_MAJOR);
        assert_eq!(hex(&p), "4e42504400020000");
        assert_eq!(preamble::parse(&p), Some(2));
        // v1 첫 프레임([len u32 = 0x20][e])은 프리앰블이 아니다.
        assert_eq!(preamble::parse(&[0, 0, 0, 0x20, 1, 2, 3, 4]), None);
        assert_eq!(preamble::parse(&p[..7]), None);

        let q = udp_obs::encode_probe(0x0102_0304_0506_0708);
        assert_eq!(hex(&q), "4e424f420102030405060708");
        assert_eq!(udp_obs::parse_probe(&q), Some(0x0102_0304_0506_0708));
        assert_eq!(udp_obs::parse_probe(&q[..11]), None);
        let e = udp_obs::encode_echo(5, Ep::V4([127, 0, 0, 1], 47300));
        assert_eq!(hex(&e), "4e424f420000000000000005047f000001b8c4");
        assert_eq!(
            udp_obs::parse_echo(&e),
            Some((5, Ep::V4([127, 0, 0, 1], 47300)))
        );
        // 응답에 ep가 없으면 응답이 아니다.
        assert_eq!(udp_obs::parse_echo(&q), None);
        assert_eq!(udp_obs::parse_echo(b"NBOX00000000"), None);
    }
}
