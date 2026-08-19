//! **파일 전송 와이어 + 수신 조립기** — 협상→수락→청크→완료(M4-2 슬라이스 1 · FR-X-3/4/6).
//!
//! ## 와이어 (`StreamId::File` 스트림)
//!
//! `[ver 1B][kind 1B][xfer_id 16B][이하 kind별]`
//!
//! | kind | 값 | 본문 |
//! |---|---|---|
//! | Offer | 1 | `[size 8B][sha256 32B][name_len 2B][name bytes(원본)]` |
//! | Accept | 2 | `[rate_cap 8B]` — 수신측 속도 상한(B/s · 0 = 무제한) |
//! | Reject | 3 | `[reason 1B][limit 8B]` — `limit` = 수신측 상한 공지(0 = 미공개) |
//! | Chunk | 4 | `[offset 8B][data …]` |
//! | Done | 5 | — |
//! | Cancel | 6 | — |
//!
//! 원칙: **수락 전에는 데이터가 오지 않는다**(협상 — 게이트 1단계 · [docs/04]). 수신 조립기는
//! 크기 상한·순서(연속 오프셋)·선언 크기 초과를 전부 거부하고(fail-closed), 완료 시 **선언
//! SHA-256과 실측 해시 대조는 호출자(조립 지점)의 해시 포트** 몫이다 — core는 크립토를 모른다
//! (DR-21). 통과한 원본은 `.beepq` 봉인(`nbeep-safe`)으로 넘어간다 — **실행 파일이 평문으로
//! 디스크에 닿는 경로가 없다.**
//!
//! 미지 `ver`·`kind`·잘림은 전부 [`XferError`] — 조용히 넘어가지 않는다.

use std::collections::HashMap;

/// 와이어 버전(미지 버전 거부).
const WIRE_VER: u8 = 1;

/// 전송 식별자(발신자가 생성 — 세션 내 유일).
pub type XferId = [u8; 16];

/// 청크 최대 크기(32 KiB) — Noise 프레임 상한(65535 − AEAD 태그 16 − mux/xfer 헤더 27)
/// 안에 **여유 있게** 들어가야 한다. 64KiB로 잡으면 헤더+태그를 더한 뒤 상한을 넘겨
/// 전송 자체가 실패한다(실측: 128KiB 전송이 첫 청크에서 세션 끊김 — 08-09).
pub const MAX_CHUNK: usize = 32 * 1024;

/// 수신 상한 **기본값**(256 MiB) — 실제 상한은 **수신측이 설정**한다
/// ([`XferInbox::with_max_file`] — 사용자 확정 08-09: 핸드셰이크에서 수신측 제약).
pub const MAX_FILE: u64 = 256 * 1024 * 1024;

/// 거부 사유 와이어 값.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectWhy {
    /// 수신자가 거절.
    Declined,
    /// 크기 상한 초과.
    TooLarge,
    /// 동시 전송 상한 등 일시 불가.
    Busy,
    /// **상호 확인되지 않은 사용자** — 양방향 대화 기록 없음(사용자 확정 규칙 08-09).
    Unverified,
    /// 수신자가 파일 수신을 거부로 설정.
    Blocked,
}

impl RejectWhy {
    fn to_byte(self) -> u8 {
        match self {
            Self::Declined => 0,
            Self::TooLarge => 1,
            Self::Busy => 2,
            Self::Unverified => 3,
            Self::Blocked => 4,
        }
    }
    fn from_byte(b: u8) -> Self {
        match b {
            1 => Self::TooLarge,
            2 => Self::Busy,
            3 => Self::Unverified,
            4 => Self::Blocked,
            _ => Self::Declined, // 미지 사유 = 일반 거절(전방 호환)
        }
    }
}

/// 파일 스트림 메시지.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XferMsg {
    /// 전송 제안 — 이름은 **원본 바이트**(정규화는 실체화 직전 · [docs/11 §4]).
    Offer {
        /// 전송 id.
        id: XferId,
        /// 원본 전체 크기.
        size: u64,
        /// 원본 전체의 SHA-256(발신 선언 — 수신이 재검증).
        sha256: [u8; 32],
        /// 원본 파일명 바이트.
        name: Vec<u8>,
    },
    /// 수락(이때부터 청크 허용) — **수신측 속도 상한을 함께 알린다**(쌍방 협상).
    Accept {
        /// 전송 id.
        id: XferId,
        /// 수신측이 감당할 상한(B/s · 0 = 무제한). 발신측은 자기 상한과 **낮은 쪽**을 쓴다.
        rate_cap: u64,
        /// **재개 오프셋**(M4-10 · D-31) — "이 오프셋부터 보내라". 0 = 처음부터
        /// (구버전 동작과 동일 · 와이어 꼬리 확장이라 구버전은 무시하고 0부터 보낸다).
        resume_offset: u64,
        /// 보존 부분의 SHA-256(M4-10) — 발신측이 자기 원본 같은 구간과 대조해
        /// **일치할 때만** 이어 보낸다(불일치 = 처음부터 · fail-closed).
        /// `resume_offset == 0`이면 무의미(0 배열).
        prefix_sha: [u8; 32],
    },
    /// 거절 — `TooLarge`면 수신측 상한을 공지해 발신자가 재시도 판단을 할 수 있다.
    Reject {
        /// 전송 id.
        id: XferId,
        /// 사유.
        why: RejectWhy,
        /// 수신측 상한 공지(바이트 · 0 = 미공개/무관).
        limit: u64,
    },
    /// 데이터 청크(연속 오프셋 강제).
    Chunk {
        /// 전송 id.
        id: XferId,
        /// 시작 오프셋.
        offset: u64,
        /// 데이터(≤ [`MAX_CHUNK`]).
        data: Vec<u8>,
    },
    /// 발신 완료 선언 — **최종 해시 동봉**(08-18 지연 해시: Offer의 sha가 0이면
    /// 여기 실린 해시가 선언이다. 발신이 전송하며 증분 계산 — 오퍼 전 전체
    /// 해시(1.5GB ≈ 2~3초)가 사라져 수신 확인 창이 즉시 뜬다).
    Done {
        /// 전송 id.
        id: XferId,
        /// 원본 전체 SHA-256(0 = 구버전/기송신 Offer 선언 사용).
        sha256: [u8; 32],
    },
    /// 취소(양쪽 어디서든).
    Cancel {
        /// 전송 id.
        id: XferId,
    },
    /// **수신 종단 확인**(M4-9) — 수신측이 해시 재검증·격리까지 마쳤음을 발신자에게 알린다.
    /// 이걸 받아야 발신 "완료"가 참이 된다("보냈다"≠"닿았다" — 그전엔 "확인 대기").
    Received {
        /// 전송 id.
        id: XferId,
    },
    /// **수신 종단 실패**(M4-9) — 수신측 해시 불일치·저장 실패. 발신자가 거짓 완료를
    /// 남기지 않게 실패를 되돌려 알린다.
    Failed {
        /// 전송 id.
        id: XferId,
    },
}

/// 와이어 오류 — 전부 명시 거부(fail-closed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XferError {
    /// 잘린 메시지.
    Truncated,
    /// 미지 버전.
    Version(u8),
    /// 미지 kind.
    Kind(u8),
    /// 청크 크기 초과.
    ChunkTooBig,
    /// 파일 크기 상한 초과(오퍼 거부 사유).
    FileTooBig,
    /// 미지 전송 id.
    UnknownXfer,
    /// 수락 전 청크 도착(프로토콜 위반).
    NotAccepted,
    /// 오프셋 불연속(순서 위반).
    OutOfOrder,
    /// 선언 크기 초과 수신.
    Overflow,
    /// 완료 시점 크기 불일치.
    SizeMismatch,
}

impl core::fmt::Display for XferError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for XferError {}

const K_OFFER: u8 = 1;
const K_ACCEPT: u8 = 2;
const K_REJECT: u8 = 3;
const K_CHUNK: u8 = 4;
const K_DONE: u8 = 5;
const K_CANCEL: u8 = 6;
const K_RECEIVED: u8 = 7;
const K_FAILED: u8 = 8;

impl XferMsg {
    /// 와이어 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.push(WIRE_VER);
        match self {
            Self::Offer {
                id,
                size,
                sha256,
                name,
            } => {
                out.push(K_OFFER);
                out.extend_from_slice(id);
                out.extend_from_slice(&size.to_le_bytes());
                out.extend_from_slice(sha256);
                let nlen = u16::try_from(name.len()).unwrap_or(u16::MAX);
                out.extend_from_slice(&nlen.to_le_bytes());
                out.extend_from_slice(&name[..nlen as usize]);
            }
            Self::Accept {
                id,
                rate_cap,
                resume_offset,
                prefix_sha,
            } => {
                out.push(K_ACCEPT);
                out.extend_from_slice(id);
                out.extend_from_slice(&rate_cap.to_le_bytes());
                // 재개 꼬리(M4-10) — 재개일 때만 싣는다: 0이면 구버전과 **바이트
                // 동일**(하위 호환 검증이 쉽고, 미지 꼬리를 안 좋아하는 중간자
                // 검사기와의 마찰도 없다). 구버전 수신자는 꼬리를 무시한다.
                if *resume_offset > 0 {
                    out.extend_from_slice(&resume_offset.to_le_bytes());
                    out.extend_from_slice(prefix_sha);
                }
            }
            Self::Reject { id, why, limit } => {
                out.push(K_REJECT);
                out.extend_from_slice(id);
                out.push(why.to_byte());
                out.extend_from_slice(&limit.to_le_bytes());
            }
            Self::Chunk { id, offset, data } => {
                out.push(K_CHUNK);
                out.extend_from_slice(id);
                out.extend_from_slice(&offset.to_le_bytes());
                out.extend_from_slice(data);
            }
            Self::Done { id, sha256 } => {
                out.push(K_DONE);
                out.extend_from_slice(id);
                out.extend_from_slice(sha256); // 구버전 수신자는 꼬리 무시(전방 호환)
            }
            Self::Cancel { id } => {
                out.push(K_CANCEL);
                out.extend_from_slice(id);
            }
            Self::Received { id } => {
                out.push(K_RECEIVED);
                out.extend_from_slice(id);
            }
            Self::Failed { id } => {
                out.push(K_FAILED);
                out.extend_from_slice(id);
            }
        }
        out
    }

    /// 와이어 해석 — 미지 버전/kind·잘림은 오류(fail-closed).
    ///
    /// # Errors
    /// [`XferError::Truncated`]·[`XferError::Version`]·[`XferError::Kind`]·[`XferError::ChunkTooBig`].
    pub fn decode(bytes: &[u8]) -> Result<Self, XferError> {
        if bytes.len() < 2 + 16 {
            return Err(XferError::Truncated);
        }
        if bytes[0] != WIRE_VER {
            return Err(XferError::Version(bytes[0]));
        }
        let kind = bytes[1];
        let mut id = [0u8; 16];
        id.copy_from_slice(&bytes[2..18]);
        let rest = &bytes[18..];
        Ok(match kind {
            K_OFFER => {
                if rest.len() < 8 + 32 + 2 {
                    return Err(XferError::Truncated);
                }
                let size = u64::from_le_bytes(rest[0..8].try_into().expect("8B"));
                let mut sha256 = [0u8; 32];
                sha256.copy_from_slice(&rest[8..40]);
                let nlen = u16::from_le_bytes([rest[40], rest[41]]) as usize;
                if rest.len() < 42 + nlen {
                    return Err(XferError::Truncated);
                }
                Self::Offer {
                    id,
                    size,
                    sha256,
                    name: rest[42..42 + nlen].to_vec(),
                }
            }
            K_ACCEPT => {
                // 구버전(상한 없음)도 받아 준다 — 없으면 무제한으로 본다(전방 호환).
                let rate_cap = if rest.len() >= 8 {
                    u64::from_le_bytes(rest[0..8].try_into().expect("8B"))
                } else {
                    0
                };
                // 재개 꼬리(M4-10 — 없으면 0 = 처음부터 · 잘린 꼬리도 0 취급:
                // 재개는 최적화라 관용이 안전하다. 청크 검증이 최종 방어선).
                let (resume_offset, prefix_sha) = if rest.len() >= 8 + 8 + 32 {
                    let off = u64::from_le_bytes(rest[8..16].try_into().expect("8B"));
                    let mut sha = [0u8; 32];
                    sha.copy_from_slice(&rest[16..48]);
                    (off, sha)
                } else {
                    (0, [0u8; 32])
                };
                Self::Accept {
                    id,
                    rate_cap,
                    resume_offset,
                    prefix_sha,
                }
            }
            K_REJECT => {
                if rest.len() < 9 {
                    return Err(XferError::Truncated);
                }
                let why = RejectWhy::from_byte(rest[0]);
                let limit = u64::from_le_bytes(rest[1..9].try_into().expect("8B"));
                Self::Reject { id, why, limit }
            }
            K_CHUNK => {
                if rest.len() < 8 {
                    return Err(XferError::Truncated);
                }
                let offset = u64::from_le_bytes(rest[0..8].try_into().expect("8B"));
                let data = rest[8..].to_vec();
                if data.len() > MAX_CHUNK {
                    return Err(XferError::ChunkTooBig);
                }
                Self::Chunk { id, offset, data }
            }
            K_DONE => {
                let mut sha256 = [0u8; 32];
                if rest.len() >= 32 {
                    sha256.copy_from_slice(&rest[..32]);
                }
                Self::Done { id, sha256 }
            }
            K_CANCEL => Self::Cancel { id },
            K_RECEIVED => Self::Received { id },
            K_FAILED => Self::Failed { id },
            k => return Err(XferError::Kind(k)),
        })
    }
}

/// 수신 파일 상한 공지 태그(Control 스트림 · 08-18 — ChatAck 10·ProfileMsg 1~3과
/// 불충돌). 세션 성립·설정 변경 시 상대에게 알린다: 발신자가 **해시 전에**
/// 초과를 즉시 차단한다(10GB 전체 해시 뒤에야 TooLarge 거절을 받던 낭비 제거).
pub const CAP_TAG: u8 = 11;

/// 상한 공지 인코딩 — `CAP_TAG(1) ‖ recv_max(8 LE)`. `u64::MAX` = 무제한.
#[must_use]
pub fn encode_cap_advert(recv_max: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(9);
    out.push(CAP_TAG);
    out.extend_from_slice(&recv_max.to_le_bytes());
    out
}

/// 상한 공지 해석 — 태그·길이 불일치 = None(다른 Control 프레임).
#[must_use]
pub fn decode_cap_advert(bytes: &[u8]) -> Option<u64> {
    if bytes.len() != 9 || bytes[0] != CAP_TAG {
        return None;
    }
    Some(u64::from_le_bytes(bytes[1..9].try_into().ok()?))
}

/// 상한 **질의** 태그(08-18 — 송신 직전 신선도 확인: 상대 설정이 그새 바뀌었을
/// 수 있다 · 사용자 요청). 응답은 [`encode_cap_advert`] 그대로.
pub const CAP_REQ_TAG: u8 = 12;

/// 상한 질의 인코딩 — 본문 없음.
#[must_use]
pub fn encode_cap_request() -> Vec<u8> {
    vec![CAP_REQ_TAG]
}

/// 상한 질의인가.
#[must_use]
pub fn is_cap_request(bytes: &[u8]) -> bool {
    bytes == [CAP_REQ_TAG]
}

/// 배치 목록(manifest) 태그(M4-2e · 08-19) — 발신자가 **요청 단위 승인**을 위해
/// 배치의 전체 파일 목록(이름·크기·제외 여부)을 첫 오퍼 전에 알린다. 수신측은
/// 목록 전체를 한 승인 창에 보이고, 승인 1회로 이후 오퍼를 **이름+크기 대조 후**
/// 자동 수락한다(불일치 = 수동 폴백 · fail-closed). 구버전은 미지 Control 태그
/// 무시 = 파일 단위 승인으로 자연 강등(전방 호환 — File 스트림과 달리 Control
/// 미지 태그는 조용히 버려진다).
pub const MANIFEST_TAG: u8 = 13;
/// manifest 항목 상한 — 비정상 대량 목록 방어(한 배치가 이걸 넘을 일은 없다).
pub const MANIFEST_MAX: usize = 4096;

/// 배치 목록 인코딩 — `MANIFEST_TAG ‖ count(2 LE) ‖ [name_len(2 LE) ‖ name ‖
/// size(8 LE) ‖ excluded(1)]…`. `excluded` = 용량 초과 등으로 **전송하지 않지만
/// 목록에는 보이는** 파일(사용자 확정 — "제외 상태로 목록에 보이도록").
#[must_use]
pub fn encode_batch_manifest(entries: &[(String, u64, bool)]) -> Vec<u8> {
    let n = entries.len().min(MANIFEST_MAX);
    let mut out = Vec::with_capacity(3 + n * 32);
    out.push(MANIFEST_TAG);
    out.extend_from_slice(&(u16::try_from(n).unwrap_or(u16::MAX)).to_le_bytes());
    for (name, size, excluded) in &entries[..n] {
        let nb = name.as_bytes();
        let nlen = u16::try_from(nb.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&nb[..usize::from(nlen)]);
        out.extend_from_slice(&size.to_le_bytes());
        out.push(u8::from(*excluded));
    }
    out
}

/// 배치 목록 해석 — 태그 불일치·손상·상한 초과 = None(다른 Control 프레임/폐기).
#[must_use]
pub fn decode_batch_manifest(bytes: &[u8]) -> Option<Vec<(String, u64, bool)>> {
    if bytes.first() != Some(&MANIFEST_TAG) {
        return None;
    }
    let count = usize::from(u16::from_le_bytes(bytes.get(1..3)?.try_into().ok()?));
    if count > MANIFEST_MAX {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut p = 3usize;
    for _ in 0..count {
        let nlen = usize::from(u16::from_le_bytes(bytes.get(p..p + 2)?.try_into().ok()?));
        p += 2;
        let name = String::from_utf8_lossy(bytes.get(p..p + nlen)?).into_owned();
        p += nlen;
        let size = u64::from_le_bytes(bytes.get(p..p + 8)?.try_into().ok()?);
        p += 8;
        let excluded = *bytes.get(p)? != 0;
        p += 1;
        out.push((name, size, excluded));
    }
    if p != bytes.len() {
        return None; // 꼬리 쓰레기 = 손상(fail-closed)
    }
    Some(out)
}

/// 수신측 **일시정지 요청** 태그(M4-2e · 08-19) — 수신자가 진행 중 전송을 멈춰
/// 달라고 발신자에게 요청한다(발신 액터의 `paused_xid` 게이트 재사용 · 세션
/// 유지). ★ **Control 스트림인 이유**: File 스트림은 미지 kind가 디코드 오류로
/// 전송 실패를 만들지만(`xfer_step` fail — 구버전에 유해), Control 미지 태그는
/// 조용히 무시된다 — 구버전 발신자 상대로는 자연 무시(전방 호환).
pub const PAUSE_REQ_TAG: u8 = 14;
/// 수신측 **재개 요청** 태그(M4-2e) — [`PAUSE_REQ_TAG`]의 짝.
pub const RESUME_REQ_TAG: u8 = 15;

/// 일시정지/재개 요청 인코딩 — `TAG ‖ xid(16)`.
#[must_use]
pub fn encode_pause_req(id: XferId, pause: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(17);
    out.push(if pause { PAUSE_REQ_TAG } else { RESUME_REQ_TAG });
    out.extend_from_slice(&id);
    out
}

/// 일시정지/재개 요청 해석 — `Some((xid, pause))`. 태그·길이 불일치 = None.
#[must_use]
pub fn decode_pause_req(bytes: &[u8]) -> Option<(XferId, bool)> {
    if bytes.len() != 17 {
        return None;
    }
    let pause = match bytes[0] {
        PAUSE_REQ_TAG => true,
        RESUME_REQ_TAG => false,
        _ => return None,
    };
    let mut id = [0u8; 16];
    id.copy_from_slice(&bytes[1..17]);
    Some((id, pause))
}

/// 발신 청커 — 원본을 [`MAX_CHUNK`] 단위 [`XferMsg::Chunk`]로 자른다.
#[must_use]
pub fn chunks_of(id: XferId, original: &[u8]) -> Vec<XferMsg> {
    original
        .chunks(MAX_CHUNK)
        .enumerate()
        .map(|(i, c)| XferMsg::Chunk {
            id,
            offset: (i * MAX_CHUNK) as u64,
            data: c.to_vec(),
        })
        .collect()
}

/// 수신 중 전송 하나의 상태.
#[derive(Debug)]
struct Incoming {
    size: u64,
    sha256: [u8; 32],
    name: Vec<u8>,
    accepted: bool,
    buf: Vec<u8>,
}

/// 완료된 수신물 — 해시 대조·`.beepq` 봉인은 조립 지점 몫(core는 크립토를 모른다).
#[derive(Debug, PartialEq, Eq)]
pub struct Received {
    /// 전송 id.
    pub id: XferId,
    /// 원본 파일명 바이트.
    pub name: Vec<u8>,
    /// 발신이 선언한 SHA-256 — **호출자가 `bytes` 실측 해시와 대조해야 한다**(FR-X-6).
    pub declared_sha256: [u8; 32],
    /// 수신 원본.
    pub bytes: Vec<u8>,
}

/// 수신 조립기 — 세션당 하나. 오퍼 수신→(호스트 결정) 수락→청크 연속 조립→완료.
///
/// **크기 상한은 수신측 정책**이다(사용자 확정 08-09) — 발신측 상수가 아니라 수신자가
/// [`XferInbox::with_max_file`]/[`XferInbox::set_max_file`]로 정하고, 초과 오퍼는
/// [`XferError::FileTooBig`]으로 거부하며 거절 와이어에 상한을 공지한다.
#[derive(Debug)]
pub struct XferInbox {
    inflight: HashMap<XferId, Incoming>,
    /// 수신 허용 상한(바이트).
    max_file: u64,
}

impl Default for XferInbox {
    fn default() -> Self {
        Self::new()
    }
}

impl XferInbox {
    /// 새 조립기(상한 = [`MAX_FILE`] 기본값).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inflight: HashMap::new(),
            max_file: MAX_FILE,
        }
    }

    /// 수신 상한을 정해 만든다(수신측 정책 — 설정/호스트가 주입).
    #[must_use]
    pub fn with_max_file(max_file: u64) -> Self {
        Self {
            inflight: HashMap::new(),
            max_file,
        }
    }

    /// 수신 상한 변경(설정 hot-swap 경로).
    pub fn set_max_file(&mut self, max_file: u64) {
        self.max_file = max_file;
    }

    /// 현재 수신 상한(바이트) — 거절 공지·UI 표시용.
    #[must_use]
    pub fn max_file(&self) -> u64 {
        self.max_file
    }

    /// 오퍼 접수 — 상한 검사만 하고 **대기**(수락은 호스트/사용자 결정 · FR-X-3).
    ///
    /// # Errors
    /// [`XferError::FileTooBig`] — 오퍼 즉시 거부 사유.
    pub fn offer(&mut self, msg: &XferMsg) -> Result<(), XferError> {
        let XferMsg::Offer {
            id,
            size,
            sha256,
            name,
        } = msg
        else {
            return Ok(()); // 오퍼 아님 — 무시(호출 편의)
        };
        if *size > self.max_file {
            return Err(XferError::FileTooBig);
        }
        self.inflight.insert(
            *id,
            Incoming {
                size: *size,
                sha256: *sha256,
                name: name.clone(),
                accepted: false,
                buf: Vec::new(),
            },
        );
        Ok(())
    }

    /// 호스트가 수락 확정 — 이후 청크가 허용된다.
    ///
    /// # Errors
    /// [`XferError::UnknownXfer`].
    pub fn accept(&mut self, id: &XferId) -> Result<(), XferError> {
        self.inflight
            .get_mut(id)
            .map(|x| x.accepted = true)
            .ok_or(XferError::UnknownXfer)
    }

    /// 수락 + **보존 부분 선적재**(M4-10 재개) — 이후 청크는 `prefix.len()`부터
    /// 이어진다(연속 오프셋 검사가 buf 길이 기준이라 그대로 성립). 구버전 발신자가
    /// 0부터 다시 보내도 [`Self::chunk`]의 중복 프리픽스 관용이 흡수한다.
    ///
    /// # Errors
    /// [`XferError::UnknownXfer`] · [`XferError::Overflow`](prefix가 선언 크기 초과).
    pub fn accept_with_prefix(&mut self, id: &XferId, prefix: Vec<u8>) -> Result<(), XferError> {
        let x = self.inflight.get_mut(id).ok_or(XferError::UnknownXfer)?;
        if prefix.len() as u64 > x.size {
            return Err(XferError::Overflow);
        }
        x.accepted = true;
        x.buf = prefix;
        Ok(())
    }

    /// 거절/취소 — 부분 수신물 즉시 폐기(잔존 금지).
    pub fn drop_xfer(&mut self, id: &XferId) {
        self.inflight.remove(id);
    }

    /// 청크 조립 — 수락 전·불연속·선언 크기 초과 전부 거부(fail-closed · 해당 전송 폐기).
    ///
    /// # Errors
    /// [`XferError`] — 오류 시 해당 전송은 폐기된다(부분물 잔존 금지).
    pub fn chunk(&mut self, id: &XferId, offset: u64, data: &[u8]) -> Result<(), XferError> {
        let r = self.chunk_inner(id, offset, data);
        if r.is_err() {
            self.inflight.remove(id);
        }
        r
    }

    fn chunk_inner(&mut self, id: &XferId, offset: u64, data: &[u8]) -> Result<(), XferError> {
        if data.len() > MAX_CHUNK {
            return Err(XferError::ChunkTooBig);
        }
        let x = self.inflight.get_mut(id).ok_or(XferError::UnknownXfer)?;
        if !x.accepted {
            return Err(XferError::NotAccepted);
        }
        // 중복 프리픽스 관용(M4-10 — 재개 Accept를 무시한 구버전 발신자가 0부터
        // 다시 보내는 경우): 이미 가진 구간과 **바이트가 일치할 때만** 버리고,
        // 어긋나면 즉시 폐기(조용히 이어붙여 손상 파일을 만들지 않는다).
        if offset < x.buf.len() as u64 {
            let have = usize::try_from(x.buf.len() as u64 - offset).unwrap_or(usize::MAX);
            let dup = have.min(data.len());
            let start = usize::try_from(offset).map_err(|_| XferError::OutOfOrder)?;
            if x.buf[start..start + dup] != data[..dup] {
                return Err(XferError::OutOfOrder);
            }
            let tail = &data[dup..];
            if x.buf.len() as u64 + tail.len() as u64 > x.size {
                return Err(XferError::Overflow);
            }
            x.buf.extend_from_slice(tail);
            return Ok(());
        }
        if offset != x.buf.len() as u64 {
            return Err(XferError::OutOfOrder);
        }
        if x.buf.len() as u64 + data.len() as u64 > x.size {
            return Err(XferError::Overflow);
        }
        x.buf.extend_from_slice(data);
        Ok(())
    }

    /// 완료 처리 — 크기 대조 후 수신물 반환(해시 대조는 호출자 몫 — 문서 참조).
    ///
    /// # Errors
    /// [`XferError::UnknownXfer`]·[`XferError::SizeMismatch`] — 둘 다 해당 전송을 폐기한다.
    pub fn done(&mut self, id: &XferId) -> Result<Received, XferError> {
        let x = self.inflight.remove(id).ok_or(XferError::UnknownXfer)?;
        if x.buf.len() as u64 != x.size {
            return Err(XferError::SizeMismatch);
        }
        Ok(Received {
            id: *id,
            name: x.name,
            declared_sha256: x.sha256,
            bytes: x.buf,
        })
    }

    /// 진행률(수신 바이트, 선언 크기) — UI 표시용.
    #[must_use]
    pub fn progress(&self, id: &XferId) -> Option<(u64, u64)> {
        self.inflight.get(id).map(|x| (x.buf.len() as u64, x.size))
    }

    /// 세션 종료 시 **수락된 부분 수신물 회수**(M4-10a) — 보존은 호스트 몫
    /// (`.part` 봉인 저장). 미수락(오퍼만) 항목은 보존 가치가 없어 버린다.
    pub fn take_partials(&mut self) -> Vec<PartialXfer> {
        let ids: Vec<XferId> = self
            .inflight
            .iter()
            .filter(|(_, x)| x.accepted && !x.buf.is_empty() && (x.buf.len() as u64) < x.size)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .filter_map(|id| {
                self.inflight.remove(&id).map(|x| PartialXfer {
                    id,
                    size: x.size,
                    sha256: x.sha256,
                    name: x.name,
                    bytes: x.buf,
                })
            })
            .collect()
    }
}

/// 중단된 수신 전송의 부분 상태(M4-10a) — 세션 액터가 회수해 호스트가 보존한다.
#[derive(Debug)]
pub struct PartialXfer {
    /// 전송 id(원 Offer의 것 — 재개 협상은 sha·size로 하므로 참고용).
    pub id: XferId,
    /// 선언 전체 크기.
    pub size: u64,
    /// 선언 전체 SHA-256 — **재개 후보 판정의 앵커**(같은 sha+size Offer = 같은 파일).
    pub sha256: [u8; 32],
    /// 원본 파일명 바이트.
    pub name: Vec<u8>,
    /// 수신 완료 구간(연속 · 0부터).
    pub bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xid(b: u8) -> XferId {
        [b; 16]
    }

    /// CapAdvert(08-18) — 왕복 + 태그/길이 불일치 거부.
    #[test]
    fn cap_advert_roundtrip() {
        assert_eq!(
            decode_cap_advert(&encode_cap_advert(512 * 1024 * 1024)),
            Some(512 * 1024 * 1024)
        );
        assert_eq!(
            decode_cap_advert(&encode_cap_advert(u64::MAX)),
            Some(u64::MAX)
        );
        assert_eq!(decode_cap_advert(&[CAP_TAG, 1, 2]), None, "짧으면 None");
        assert_eq!(decode_cap_advert(&encode_cap_advert(1)[..8]), None);
        let mut w = encode_cap_advert(7);
        w[0] = 12;
        assert_eq!(decode_cap_advert(&w), None, "다른 태그 = 남의 프레임");
    }

    /// ★ M4-2e(08-19) — 배치 manifest 왕복(이름·크기·제외 보존) + 손상 거부.
    #[test]
    fn batch_manifest_roundtrip() {
        let entries = vec![
            ("사진 모음.zip".to_string(), 12_345_678u64, false),
            ("VMware-Fusion-25H2.dmg".to_string(), 4_512_000_000u64, true), // 용량 초과 = 제외
            ("메모.txt".to_string(), 42u64, false),
        ];
        let enc = encode_batch_manifest(&entries);
        assert_eq!(enc[0], MANIFEST_TAG);
        assert_eq!(decode_batch_manifest(&enc).unwrap(), entries);
        // 손상: 잘림 = None · 꼬리 쓰레기 = None · 다른 태그 = None.
        assert_eq!(decode_batch_manifest(&enc[..enc.len() - 1]), None);
        let mut tail = enc.clone();
        tail.push(0);
        assert_eq!(decode_batch_manifest(&tail), None);
        let mut w = enc;
        w[0] = CAP_TAG;
        assert_eq!(decode_batch_manifest(&w), None);
        // 빈 배치도 유효(0건 — 발신 전 취소 시 목록 철회 신호로 쓸 수 있다).
        assert_eq!(
            decode_batch_manifest(&encode_batch_manifest(&[])),
            Some(vec![])
        );
    }

    /// ★ M4-2e(08-19) — 수신측 일시정지/재개 요청 왕복 + 불일치 거부.
    #[test]
    fn pause_req_roundtrip() {
        assert_eq!(
            decode_pause_req(&encode_pause_req(xid(7), true)),
            Some((xid(7), true))
        );
        assert_eq!(
            decode_pause_req(&encode_pause_req(xid(9), false)),
            Some((xid(9), false))
        );
        assert_eq!(decode_pause_req(&[PAUSE_REQ_TAG; 5]), None, "길이 불일치");
        let mut w = encode_pause_req(xid(1), true);
        w[0] = CAP_TAG;
        assert_eq!(decode_pause_req(&w), None, "다른 태그 = 남의 프레임");
    }

    #[test]
    fn wire_roundtrip_all_kinds() {
        let msgs = [
            XferMsg::Offer {
                id: xid(1),
                size: 3,
                sha256: [9u8; 32],
                name: "보고서.pdf".as_bytes().to_vec(),
            },
            XferMsg::Accept {
                id: xid(1),
                rate_cap: 1_048_576,
                resume_offset: 0,
                prefix_sha: [0u8; 32],
            },
            XferMsg::Reject {
                id: xid(1),
                why: RejectWhy::TooLarge,
                limit: 8 * 1024 * 1024,
            },
            XferMsg::Chunk {
                id: xid(1),
                offset: 0,
                data: vec![1, 2, 3],
            },
            XferMsg::Done {
                id: xid(1),
                sha256: [5u8; 32],
            },
            XferMsg::Cancel { id: xid(1) },
            XferMsg::Received { id: xid(1) },
            XferMsg::Failed { id: xid(1) },
        ];
        for m in msgs {
            assert_eq!(XferMsg::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn wire_rejects_unknown_and_truncated() {
        assert_eq!(XferMsg::decode(&[]), Err(XferError::Truncated));
        let mut v = XferMsg::Accept {
            id: xid(1),
            rate_cap: 0,
            resume_offset: 0,
            prefix_sha: [0u8; 32],
        }
        .encode();
        v[0] = 9;
        assert_eq!(XferMsg::decode(&v), Err(XferError::Version(9)));
        let mut v = XferMsg::Accept {
            id: xid(1),
            rate_cap: 0,
            resume_offset: 0,
            prefix_sha: [0u8; 32],
        }
        .encode();
        v[1] = 99;
        assert_eq!(XferMsg::decode(&v), Err(XferError::Kind(99)));
    }

    #[test]
    fn full_receive_flow_and_progress() {
        let mut inbox = XferInbox::new();
        let original: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let offer = XferMsg::Offer {
            id: xid(2),
            size: original.len() as u64,
            sha256: [7u8; 32],
            name: b"big.bin".to_vec(),
        };
        inbox.offer(&offer).unwrap();
        inbox.accept(&xid(2)).unwrap();
        for m in chunks_of(xid(2), &original) {
            let XferMsg::Chunk { id, offset, data } = m else {
                unreachable!()
            };
            inbox.chunk(&id, offset, &data).unwrap();
        }
        assert_eq!(
            inbox.progress(&xid(2)),
            Some((original.len() as u64, original.len() as u64))
        );
        let got = inbox.done(&xid(2)).unwrap();
        assert_eq!(got.bytes, original);
        assert_eq!(got.declared_sha256, [7u8; 32], "해시 대조는 호출자 몫");
    }

    #[test]
    fn chunk_before_accept_is_violation_and_drops() {
        let mut inbox = XferInbox::new();
        inbox
            .offer(&XferMsg::Offer {
                id: xid(3),
                size: 10,
                sha256: [0u8; 32],
                name: b"x".to_vec(),
            })
            .unwrap();
        assert_eq!(
            inbox.chunk(&xid(3), 0, b"data"),
            Err(XferError::NotAccepted),
            "수락 전 데이터 = 프로토콜 위반"
        );
        assert!(inbox.progress(&xid(3)).is_none(), "위반 = 즉시 폐기");
    }

    #[test]
    fn out_of_order_and_overflow_drop_partials() {
        let mut inbox = XferInbox::new();
        let offer = |id| XferMsg::Offer {
            id,
            size: 8,
            sha256: [0u8; 32],
            name: b"x".to_vec(),
        };
        inbox.offer(&offer(xid(4))).unwrap();
        inbox.accept(&xid(4)).unwrap();
        assert_eq!(inbox.chunk(&xid(4), 4, b"late"), Err(XferError::OutOfOrder));
        assert!(inbox.progress(&xid(4)).is_none());

        inbox.offer(&offer(xid(5))).unwrap();
        inbox.accept(&xid(5)).unwrap();
        assert_eq!(
            inbox.chunk(&xid(5), 0, b"way too much data"),
            Err(XferError::Overflow)
        );
    }

    #[test]
    fn done_size_mismatch_discards() {
        let mut inbox = XferInbox::new();
        inbox
            .offer(&XferMsg::Offer {
                id: xid(6),
                size: 10,
                sha256: [0u8; 32],
                name: b"x".to_vec(),
            })
            .unwrap();
        inbox.accept(&xid(6)).unwrap();
        inbox.chunk(&xid(6), 0, b"short").unwrap();
        assert_eq!(inbox.done(&xid(6)), Err(XferError::SizeMismatch));
        assert!(inbox.progress(&xid(6)).is_none(), "부분 수신물 잔존 금지");
    }

    #[test]
    fn oversized_offer_rejected() {
        let mut inbox = XferInbox::new();
        let r = inbox.offer(&XferMsg::Offer {
            id: xid(7),
            size: MAX_FILE + 1,
            sha256: [0u8; 32],
            name: b"huge".to_vec(),
        });
        assert_eq!(r, Err(XferError::FileTooBig));
    }

    #[test]
    fn accept_carries_receiver_rate_cap() {
        let m = XferMsg::Accept {
            id: xid(9),
            rate_cap: 512_000,
            resume_offset: 0,
            prefix_sha: [0u8; 32],
        };
        let back = XferMsg::decode(&m.encode()).unwrap();
        assert_eq!(back, m, "상한이 왕복해야 발신측이 협상할 수 있다");
    }

    /// M4-10 재개 꼬리 — ① 왕복 보존 ② 재개 0 = 구버전과 바이트 동일(하위 호환)
    /// ③ 꼬리 없는 구버전 프레임 = 0/영배열 해석.
    #[test]
    fn accept_resume_tail_roundtrip_and_backcompat() {
        let m = XferMsg::Accept {
            id: xid(9),
            rate_cap: 0,
            resume_offset: 8 * 1024 * 1024,
            prefix_sha: [7u8; 32],
        };
        assert_eq!(XferMsg::decode(&m.encode()).unwrap(), m, "재개 꼬리 왕복");
        let zero = XferMsg::Accept {
            id: xid(9),
            rate_cap: 44,
            resume_offset: 0,
            prefix_sha: [9u8; 32], // 0 오프셋이면 안 실린다 — 수신은 영배열
        };
        let bytes = zero.encode();
        assert_eq!(bytes.len(), 2 + 16 + 8, "재개 0 = 구버전과 같은 길이");
        let back = XferMsg::decode(&bytes).unwrap();
        assert_eq!(
            back,
            XferMsg::Accept {
                id: xid(9),
                rate_cap: 44,
                resume_offset: 0,
                prefix_sha: [0u8; 32],
            }
        );
    }

    /// M4-10a·b — 프리픽스 선적재 재개: 이어받기 → 완료. 구버전 발신자가 0부터
    /// 다시 보내는 중복 프리픽스도 흡수하고, **어긋난 프리픽스는 폐기**한다.
    #[test]
    fn resume_with_prefix_and_duplicate_prefix_tolerance() {
        let original: Vec<u8> = (0..100_000u32).map(|i| (i % 249) as u8).collect();
        let offer = |id| XferMsg::Offer {
            id,
            size: original.len() as u64,
            sha256: [7u8; 32],
            name: b"big.bin".to_vec(),
        };
        // ① 프리픽스 선적재 후 그 지점부터 이어받아 완료.
        let mut inbox = XferInbox::new();
        inbox.offer(&offer(xid(1))).unwrap();
        let cut = 40_000usize;
        inbox
            .accept_with_prefix(&xid(1), original[..cut].to_vec())
            .unwrap();
        let mut off = cut as u64;
        for c in original[cut..].chunks(MAX_CHUNK) {
            inbox.chunk(&xid(1), off, c).unwrap();
            off += c.len() as u64;
        }
        assert_eq!(inbox.done(&xid(1)).unwrap().bytes, original);
        // ② 구버전 발신자 — 재개 요청을 무시하고 0부터 전량 재송신해도 조립된다.
        let mut inbox = XferInbox::new();
        inbox.offer(&offer(xid(2))).unwrap();
        inbox
            .accept_with_prefix(&xid(2), original[..cut].to_vec())
            .unwrap();
        for m in chunks_of(xid(2), &original) {
            let XferMsg::Chunk { id, offset, data } = m else {
                unreachable!()
            };
            inbox.chunk(&id, offset, &data).unwrap();
        }
        assert_eq!(inbox.done(&xid(2)).unwrap().bytes, original);
        // ③ 어긋난 프리픽스(다른 파일) — 중복 구간 대조 실패 = 즉시 폐기.
        let mut inbox = XferInbox::new();
        inbox.offer(&offer(xid(3))).unwrap();
        inbox.accept_with_prefix(&xid(3), vec![0xEE; cut]).unwrap();
        let r = inbox.chunk(&xid(3), 0, &original[..MAX_CHUNK]);
        assert_eq!(
            r,
            Err(XferError::OutOfOrder),
            "어긋난 중복 = 손상 방지 폐기"
        );
        assert!(inbox.progress(&xid(3)).is_none());
    }

    /// M4-10a — 세션 종료 시 수락·부분 수신분만 회수된다(오퍼만·완료분 제외).
    #[test]
    fn take_partials_returns_accepted_incomplete_only() {
        let mut inbox = XferInbox::new();
        let offer = |id, size| XferMsg::Offer {
            id,
            size,
            sha256: [3u8; 32],
            name: b"f".to_vec(),
        };
        inbox.offer(&offer(xid(1), 100)).unwrap(); // 미수락 — 제외
        inbox.offer(&offer(xid(2), 100)).unwrap(); // 수락+부분 — 회수
        inbox.accept(&xid(2)).unwrap();
        inbox.chunk(&xid(2), 0, &[1u8; 40]).unwrap();
        inbox.offer(&offer(xid(3), 100)).unwrap(); // 수락+0바이트 — 제외
        inbox.accept(&xid(3)).unwrap();
        let parts = inbox.take_partials();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].id, xid(2));
        assert_eq!(parts[0].bytes.len(), 40);
        assert_eq!(parts[0].size, 100);
    }

    #[test]
    fn receiver_sets_its_own_limit() {
        // 수신측 정책 — 발신 상수가 아니라 수신자가 정한다(사용자 확정 08-09).
        let mut inbox = XferInbox::with_max_file(1024);
        assert_eq!(inbox.max_file(), 1024);
        let offer = |size| XferMsg::Offer {
            id: xid(8),
            size,
            sha256: [0u8; 32],
            name: b"f".to_vec(),
        };
        assert_eq!(inbox.offer(&offer(1025)), Err(XferError::FileTooBig));
        assert!(inbox.offer(&offer(1024)).is_ok(), "상한 이하 = 접수");
        // hot-swap 축소 후 새 오퍼부터 적용.
        inbox.set_max_file(10);
        assert_eq!(inbox.offer(&offer(11)), Err(XferError::FileTooBig));
    }
}
