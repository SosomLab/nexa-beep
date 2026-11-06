//! `.beepq`(NXBQ) 격리 컨테이너 코덱 — **M4-1 슬라이스 1**([docs/11] ADR-0004 §2).
//!
//! ```text
//! [magic 4B "NXBQ"][format_ver 2B][flags 2B][content_sha256 32B][original_size 8B]
//! [sealed_len N 2B][sealed_prefix N][meta_len 4B][meta TLV][body = 원본[N..]]
//! ```
//!
//! ## 봉인(seal) — 이 컨테이너의 존재 이유
//!
//! 원본 선두 **N = min(512, 원본크기)** 바이트를 **본문에서 잘라 헤더로 옮긴다**. 512B면
//! PE/ELF/Mach-O 헤더·스크립트 셔뱅·아카이브 로컬 헤더를 전부 덮으므로, **`body`만으로는
//! 어떤 로더도 원본을 해석·실행할 수 없다**(ADR-0004 결정 ②). 승인 전까지 원본은 파일시스템에
//! 형태로 존재하지 않는다(결정 ①).
//!
//! ## fail-closed
//!
//! 발견 와이어([nbeep-net] `wire.rs`)는 미지 버전을 **무시**하지만, 격리 컨테이너는 정반대다 —
//! **미지 `format_ver`는 열지 않는다**(ADR-0004 §2 · NFR-S-4). 잘못 해석해 여는 것이 위험하므로
//! 손상·미지 구조는 전부 [`QuarantineError`]로 수렴한다("모르겠으니 통과"는 없다).
//!
//! ## 봉투만 본다
//!
//! 이 계층의 봉투 = **컨테이너 구조와 메타뿐**. 해시는 **불투명 32바이트**로 저장·비교만 하고
//! 이 모듈은 계산하지 않는다(해시 계산은 상위 포트 — 다음 슬라이스). TLV 파싱도 값의 의미가
//! 아니라 형태만 검증한다.

use nbeep_core::{PeerId, RiskLevel, ScanOutcome};

/// 컨테이너 매직.
pub const MAGIC: [u8; 4] = *b"NXBQ";
/// 현재 컨테이너 버전 — **미지 버전은 열지 않는다**(fail-closed).
pub const FORMAT_VER: u16 = 1;
/// 봉인 선두 상한(바이트). 실제 N = `min(MAX_SEAL, original_size)`.
pub const MAX_SEAL: usize = 512;

/// ★ NXBQ **v2 — 꼬리 메타**(M4-12 · 11-07 · 사용자 결정 11-06). v1은 sha·size·메타가 머리에 있어 **쓰기 시작 전에**
/// 해시·검사 결과가 확정돼야 했다(= 원본 전체가 메모리에). v2는 머리를 8B로 줄이고 본문을 받는 대로 쓴 뒤 꼬리에
/// 둔다. **반드시 NBSE v2 스트림([`nbeep_store::sealed_stream`]) 안에만 존재**한다 — 그래서 v1의 "선두 N 분리"(본문만으론
/// 로드 불가)는 두지 않는다(AEAD 스트림이 전체를 덮고, 게이트는 v2를 평문으로 쓰지 않는다).
///
/// ```text
/// head = "NXBQ" ‖ ver(2B LE)=2 ‖ flags(2B LE)                            (8B)
/// body = 원본 전체
/// tail = sha256(32) ‖ original_size(8 LE) ‖ meta_len(4 LE) ‖ meta TLV ‖ tail_len(4 LE)
///        (tail_len = 32+8+4+meta_len · 파일 끝 4B를 읽어 꼬리 시작을 찾는다)
/// ```
pub const FORMAT_VER_V2: u16 = 2;
/// v2 머리 길이.
pub const HEAD_V2: usize = 8;
/// v2 꼬리 고정부 길이(sha32 + size8 + meta_len4) — 트레일러(tail_len 4B) 제외.
pub const TAIL_V2_FIXED: usize = 32 + 8 + 4;

/// v2 머리 바이트.
#[must_use]
pub fn head_v2(flags: u16) -> [u8; HEAD_V2] {
    let mut h = [0u8; HEAD_V2];
    h[..4].copy_from_slice(&MAGIC);
    h[4..6].copy_from_slice(&FORMAT_VER_V2.to_le_bytes());
    h[6..8].copy_from_slice(&flags.to_le_bytes());
    h
}

/// v2 머리 해석 → flags. v1 머리면 `UnknownVersion(1)`(호출자가 v1 경로로 돌린다).
///
/// # Errors
/// [`QuarantineError`] — 매직·버전·잘림.
pub fn parse_head_v2(bytes: &[u8]) -> Result<u16, QuarantineError> {
    if bytes.len() < HEAD_V2 {
        return Err(QuarantineError::Truncated);
    }
    if bytes[..4] != MAGIC {
        return Err(QuarantineError::BadMagic);
    }
    let ver = u16::from_le_bytes([bytes[4], bytes[5]]);
    if ver != FORMAT_VER_V2 {
        return Err(QuarantineError::UnknownVersion(ver));
    }
    Ok(u16::from_le_bytes([bytes[6], bytes[7]]))
}

/// v2 꼬리(sha·size·메타) — 본문을 다 쓴 뒤 붙인다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TailV2 {
    /// 원본 전체 SHA-256(수신 중 증분 계산 — 호출자).
    pub content_sha256: [u8; 32],
    /// 원본 전체 크기.
    pub original_size: u64,
    /// 메타 블록(검사 결과 포함 — 본문 뒤에 확정).
    pub meta: Meta,
}

impl TailV2 {
    /// 꼬리 바이트(트레일러 tail_len 포함).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let meta = self.meta.encode();
        let tail_len = u32::try_from(TAIL_V2_FIXED + meta.len()).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(TAIL_V2_FIXED + meta.len() + 4);
        out.extend_from_slice(&self.content_sha256);
        out.extend_from_slice(&self.original_size.to_le_bytes());
        out.extend_from_slice(&(meta.len() as u32).to_le_bytes());
        out.extend_from_slice(&meta);
        out.extend_from_slice(&tail_len.to_le_bytes());
        out
    }

    /// 트레일러(파일 끝 4B) → 꼬리 길이(트레일러 제외). 손상은 None.
    #[must_use]
    pub fn tail_len(trailer: &[u8]) -> Option<usize> {
        let t: [u8; 4] = trailer.try_into().ok()?;
        let n = u32::from_le_bytes(t) as usize;
        (n >= TAIL_V2_FIXED).then_some(n)
    }

    /// 꼬리 해석(트레일러 제외 바이트 · `tail_len` 길이만큼) — 미지 TLV는 건너뛴다(v1과 같은 전방 호환).
    ///
    /// # Errors
    /// [`QuarantineError`] — 잘림·메타 손상·길이 불일치.
    pub fn decode(tail: &[u8]) -> Result<Self, QuarantineError> {
        if tail.len() < TAIL_V2_FIXED {
            return Err(QuarantineError::Truncated);
        }
        let content_sha256: [u8; 32] = tail[..32].try_into().expect("32B");
        let original_size = u64::from_le_bytes(tail[32..40].try_into().expect("8B"));
        let meta_len = u32::from_le_bytes(tail[40..44].try_into().expect("4B")) as usize;
        if tail.len() != TAIL_V2_FIXED + meta_len {
            return Err(QuarantineError::BadMeta);
        }
        let meta = Meta::decode(&tail[44..])?;
        Ok(Self {
            content_sha256,
            original_size,
            meta,
        })
    }
}

/// `sealed_prefix` 앞 고정 헤더 길이 = magic4 + ver2 + flags2 + sha32 + size8 + sealed_len2.
const HEADER: usize = 4 + 2 + 2 + 32 + 8 + 2;

/// flags 비트 — **봉인 완료**(v1은 항상 1). 이후 비트는 뒤에 append(값 불변).
pub const FLAG_SEALED: u16 = 1 << 0;

/// 컨테이너 파싱·구조 오류 — 전부 **격리 유지**로 수렴(fail-closed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuarantineError {
    /// 매직 불일치 — 우리 컨테이너가 아니다.
    BadMagic,
    /// 미지 `format_ver` — **열지 않는다**(미래 포맷).
    UnknownVersion(u16),
    /// 선언 길이가 실제 바이트를 벗어남(잘림·손상).
    Truncated,
    /// 메타 TLV 손상(길이 초과·미지 필수 값).
    BadMeta,
    /// `sealed_prefix + body` 길이가 `original_size`와 어긋남.
    SizeMismatch,
}

/// 격리 메타([docs/11] §2 메타 블록) — **자체 TLV로 직렬화**(런타임 의존 0).
///
/// `orig_name`은 **정규화하지 않은 원본 바이트**로 보존한다(RLO 공격 사후 감사용 — ADR-0004 §2).
/// UI로 넘기는 경로는 실체화 시점에 별도 무해화한다(FR-S-13 — 다음 슬라이스).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// 원본 파일명 **원본 바이트**(정규화 전).
    pub orig_name: Vec<u8>,
    /// 송신자가 주장한 확장자.
    pub declared_ext: String,
    /// 송신자가 주장한 MIME.
    pub declared_mime: String,
    /// 매직바이트로 판정한 형식.
    pub detected_kind: String,
    /// 위험 등급([docs/11] §3).
    pub risk: RiskLevel,
    /// 발신자 `PeerId`(키 지문).
    pub sender: PeerId,
    /// 수신 시각(Unix 초).
    pub received_at: u64,
    /// 자동 삭제 예정(Unix 초).
    pub expires_at: u64,
    /// 검사 결과.
    pub scan: ScanOutcome,
    /// 전송 세션 참조(대화 스레드 연결용) — 없으면 빈 문자열.
    pub xfer: String,
}

/// TLV 키(값 불변 · 추가는 뒤에). 미지 키는 읽을 때 **건너뛴다**(같은 major 내 전방 호환).
mod tag {
    pub(super) const ORIG_NAME: u8 = 1;
    pub(super) const DECLARED_EXT: u8 = 2;
    pub(super) const DECLARED_MIME: u8 = 3;
    pub(super) const DETECTED_KIND: u8 = 4;
    pub(super) const RISK: u8 = 5;
    pub(super) const SENDER: u8 = 6;
    pub(super) const RECEIVED_AT: u8 = 7;
    pub(super) const EXPIRES_AT: u8 = 8;
    pub(super) const SCAN: u8 = 9;
    pub(super) const XFER: u8 = 10;
}

fn risk_byte(r: RiskLevel) -> u8 {
    match r {
        RiskLevel::Executable => 0,
        RiskLevel::ActiveDocument => 1,
        RiskLevel::Archive => 2,
        RiskLevel::Data => 3,
    }
}

fn risk_from_byte(b: u8) -> Option<RiskLevel> {
    match b {
        0 => Some(RiskLevel::Executable),
        1 => Some(RiskLevel::ActiveDocument),
        2 => Some(RiskLevel::Archive),
        3 => Some(RiskLevel::Data),
        _ => None,
    }
}

fn scan_byte(s: ScanOutcome) -> u8 {
    match s {
        ScanOutcome::Unavailable => 0,
        ScanOutcome::Clean => 1,
        ScanOutcome::Detected => 2,
        ScanOutcome::TooLarge => 3, // M4-12(11-07) — 구 앱은 BadMeta(fail-closed · 하향 호환 포기는 v2와 같은 결)
    }
}

fn scan_from_byte(b: u8) -> Option<ScanOutcome> {
    match b {
        0 => Some(ScanOutcome::Unavailable),
        1 => Some(ScanOutcome::Clean),
        2 => Some(ScanOutcome::Detected),
        3 => Some(ScanOutcome::TooLarge),
        _ => None,
    }
}

/// 하나의 TLV 필드를 `out`에 쓴다: `[tag 1B][len u32 LE][value]`.
fn put_tlv(out: &mut Vec<u8>, tag: u8, value: &[u8]) {
    out.push(tag);
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value);
}

impl Meta {
    /// 메타 블록을 TLV로 직렬화한다.
    fn encode(&self) -> Vec<u8> {
        let mut m = Vec::new();
        put_tlv(&mut m, tag::ORIG_NAME, &self.orig_name);
        put_tlv(&mut m, tag::DECLARED_EXT, self.declared_ext.as_bytes());
        put_tlv(&mut m, tag::DECLARED_MIME, self.declared_mime.as_bytes());
        put_tlv(&mut m, tag::DETECTED_KIND, self.detected_kind.as_bytes());
        put_tlv(&mut m, tag::RISK, &[risk_byte(self.risk)]);
        put_tlv(&mut m, tag::SENDER, self.sender.as_bytes());
        put_tlv(&mut m, tag::RECEIVED_AT, &self.received_at.to_le_bytes());
        put_tlv(&mut m, tag::EXPIRES_AT, &self.expires_at.to_le_bytes());
        put_tlv(&mut m, tag::SCAN, &[scan_byte(self.scan)]);
        put_tlv(&mut m, tag::XFER, self.xfer.as_bytes());
        m
    }

    /// TLV 블록을 파싱한다. **미지 태그는 건너뛴다**(전방 호환). 필수 필드가 없으면 [`QuarantineError::BadMeta`].
    fn decode(mut buf: &[u8]) -> Result<Self, QuarantineError> {
        let (mut orig_name, mut sender) = (None, None);
        let (mut ext, mut mime, mut kind, mut xfer) = (None, None, None, None);
        let (mut risk, mut scan, mut recv, mut exp) = (None, None, None, None);

        while !buf.is_empty() {
            let tag = buf[0];
            if buf.len() < 5 {
                return Err(QuarantineError::BadMeta);
            }
            let len = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
            let val = buf.get(5..5 + len).ok_or(QuarantineError::BadMeta)?;
            match tag {
                tag::ORIG_NAME => orig_name = Some(val.to_vec()),
                tag::DECLARED_EXT => ext = Some(str_field(val)?),
                tag::DECLARED_MIME => mime = Some(str_field(val)?),
                tag::DETECTED_KIND => kind = Some(str_field(val)?),
                tag::RISK => {
                    risk = Some(risk_from_byte(one(val)?).ok_or(QuarantineError::BadMeta)?);
                }
                tag::SENDER => {
                    let b: [u8; 32] = val.try_into().map_err(|_| QuarantineError::BadMeta)?;
                    sender = Some(PeerId::from_bytes(b));
                }
                tag::RECEIVED_AT => recv = Some(u64_field(val)?),
                tag::EXPIRES_AT => exp = Some(u64_field(val)?),
                tag::SCAN => {
                    scan = Some(scan_from_byte(one(val)?).ok_or(QuarantineError::BadMeta)?);
                }
                tag::XFER => xfer = Some(str_field(val)?),
                _ => {} // 미지 태그 — 건너뛴다(전방 호환)
            }
            buf = &buf[5 + len..];
        }

        Ok(Meta {
            orig_name: orig_name.ok_or(QuarantineError::BadMeta)?,
            declared_ext: ext.unwrap_or_default(),
            declared_mime: mime.unwrap_or_default(),
            detected_kind: kind.unwrap_or_default(),
            risk: risk.ok_or(QuarantineError::BadMeta)?,
            sender: sender.ok_or(QuarantineError::BadMeta)?,
            received_at: recv.ok_or(QuarantineError::BadMeta)?,
            expires_at: exp.ok_or(QuarantineError::BadMeta)?,
            scan: scan.ok_or(QuarantineError::BadMeta)?,
            xfer: xfer.unwrap_or_default(),
        })
    }
}

fn one(val: &[u8]) -> Result<u8, QuarantineError> {
    match val {
        [b] => Ok(*b),
        _ => Err(QuarantineError::BadMeta),
    }
}

fn str_field(val: &[u8]) -> Result<String, QuarantineError> {
    String::from_utf8(val.to_vec()).map_err(|_| QuarantineError::BadMeta)
}

fn u64_field(val: &[u8]) -> Result<u64, QuarantineError> {
    let b: [u8; 8] = val.try_into().map_err(|_| QuarantineError::BadMeta)?;
    Ok(u64::from_le_bytes(b))
}

/// 파싱된 `.beepq` 컨테이너 — 봉인 프리픽스와 본문은 **분리 보관**한다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beepq {
    /// flags 비트.
    pub flags: u16,
    /// 원본 전체의 SHA-256(**봉인 전 기준** · 불투명 — 이 모듈은 계산하지 않는다).
    pub content_sha256: [u8; 32],
    /// 원본 전체 크기.
    pub original_size: u64,
    /// 잘라낸 원본 선두 N바이트.
    pub sealed_prefix: Vec<u8>,
    /// 메타 블록.
    pub meta: Meta,
    /// 원본[N..] — **연속된 원본이 아니다**(선두가 빠져 있다).
    pub body: Vec<u8>,
}

impl Beepq {
    /// 원본 선두를 잘라 `.beepq` 바이트를 만든다(봉인). `content_sha256`는 호출자가 계산해 넘긴다.
    #[must_use]
    pub fn seal(original: &[u8], content_sha256: [u8; 32], meta: &Meta) -> Vec<u8> {
        let n = original.len().min(MAX_SEAL);
        let (prefix, body) = original.split_at(n);
        let meta_bytes = meta.encode();

        let mut out = Vec::with_capacity(HEADER + n + 4 + meta_bytes.len() + body.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VER.to_le_bytes());
        out.extend_from_slice(&FLAG_SEALED.to_le_bytes());
        out.extend_from_slice(&content_sha256);
        out.extend_from_slice(&(original.len() as u64).to_le_bytes());
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(prefix);
        out.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&meta_bytes);
        out.extend_from_slice(body);
        out
    }

    /// ★ M4-12 0단계(11-06) — 원본 버퍼를 **소유권째** 받아 그 자리에 봉인(헤더·선두·메타를 앞에 끼워 넣는 memmove 1회 ·
    /// 여분 S 버퍼 없음). 결과 바이트는 [`Self::seal`]과 동일.
    #[must_use]
    pub fn seal_owned(mut original: Vec<u8>, content_sha256: [u8; 32], meta: &Meta) -> Vec<u8> {
        let n = original.len().min(MAX_SEAL);
        let meta_bytes = meta.encode();
        let mut head = Vec::with_capacity(HEADER + n + 4 + meta_bytes.len());
        head.extend_from_slice(&MAGIC);
        head.extend_from_slice(&FORMAT_VER.to_le_bytes());
        head.extend_from_slice(&FLAG_SEALED.to_le_bytes());
        head.extend_from_slice(&content_sha256);
        head.extend_from_slice(&(original.len() as u64).to_le_bytes());
        head.extend_from_slice(&(n as u16).to_le_bytes());
        head.extend_from_slice(&original[..n]);
        head.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
        head.extend_from_slice(&meta_bytes);
        // 선두 n바이트 자리를 머리로 바꿔 끼운다 — 본문은 그대로 뒤로 밀린다.
        original.splice(0..n, head);
        original
    }

    /// `.beepq` 바이트를 파싱한다. 손상·미지 버전은 전부 오류(**fail-closed**).
    ///
    /// # Errors
    /// [`QuarantineError`] — 매직 불일치·미지 버전·잘림·메타 손상·크기 불일치.
    pub fn open(bytes: &[u8]) -> Result<Self, QuarantineError> {
        let p = Parsed::from(bytes)?;
        let body = bytes[p.body_start..].to_vec();
        Ok(p.into_beepq(body))
    }

    /// ★ M4-12 0단계 — 파일 바이트를 소유권째 받아 머리만 잘라내고 **본문은 그 버퍼 그대로**(복사 없음 · memmove 1회).
    ///
    /// # Errors
    /// [`Self::open`]과 같다.
    pub fn open_owned(mut bytes: Vec<u8>) -> Result<Self, QuarantineError> {
        let p = Parsed::from(&bytes)?;
        bytes.drain(..p.body_start);
        Ok(p.into_beepq(bytes))
    }

    /// 봉인 프리픽스와 본문을 다시 붙여 **원본 바이트를 복원**한다.
    ///
    /// ⚠️ SHA-256 재검증은 하지 않는다 — 실체화 절차(다음 슬라이스)가 해시 포트로 대조한 뒤
    /// 파일로 내보낸다(ADR-0004 §4). 이 함수는 순수 재결합만 한다.
    #[must_use]
    pub fn unseal(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.sealed_prefix.len() + self.body.len());
        out.extend_from_slice(&self.sealed_prefix);
        out.extend_from_slice(&self.body);
        out
    }

    /// 소유권째 재결합(M4-12 0단계) — 본문 버퍼 앞에 선두(≤512B)를 끼워 넣는 memmove 1회 · 여분 S 버퍼 없음.
    #[must_use]
    pub fn unseal_owned(self) -> Vec<u8> {
        let mut body = self.body;
        body.splice(0..0, self.sealed_prefix);
        body
    }
}

/// 헤더 해석 결과(본문 제외) — `open`/`open_owned`가 공유한다(M4-12 0단계).
struct Parsed {
    flags: u16,
    content_sha256: [u8; 32],
    original_size: u64,
    sealed_prefix: Vec<u8>,
    meta: Meta,
    body_start: usize,
}

impl Parsed {
    fn from(bytes: &[u8]) -> Result<Self, QuarantineError> {
        if bytes.len() < HEADER {
            return Err(QuarantineError::Truncated);
        }
        if bytes[0..4] != MAGIC {
            return Err(QuarantineError::BadMagic);
        }
        let ver = u16::from_le_bytes([bytes[4], bytes[5]]);
        if ver != FORMAT_VER {
            return Err(QuarantineError::UnknownVersion(ver));
        }
        let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        let content_sha256: [u8; 32] = bytes[8..40].try_into().expect("32B slice");
        let original_size = u64::from_le_bytes(bytes[40..48].try_into().expect("8B slice"));
        let n = u16::from_le_bytes([bytes[48], bytes[49]]) as usize;
        let prefix_end = HEADER + n;
        let meta_len_at = prefix_end;
        if bytes.len() < meta_len_at + 4 {
            return Err(QuarantineError::Truncated);
        }
        let sealed_prefix = bytes[HEADER..prefix_end].to_vec();
        let meta_len = u32::from_le_bytes(
            bytes[meta_len_at..meta_len_at + 4]
                .try_into()
                .expect("4B slice"),
        ) as usize;
        let meta_start = meta_len_at + 4;
        let body_start = meta_start + meta_len;
        if bytes.len() < body_start {
            return Err(QuarantineError::Truncated);
        }
        let meta = Meta::decode(&bytes[meta_start..body_start])?;
        let body_len = bytes.len() - body_start;
        if sealed_prefix.len() as u64 + body_len as u64 != original_size {
            return Err(QuarantineError::SizeMismatch);
        }
        Ok(Self {
            flags,
            content_sha256,
            original_size,
            sealed_prefix,
            meta,
            body_start,
        })
    }

    fn into_beepq(self, body: Vec<u8>) -> Beepq {
        Beepq {
            flags: self.flags,
            content_sha256: self.content_sha256,
            original_size: self.original_size,
            sealed_prefix: self.sealed_prefix,
            meta: self.meta,
            body,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    fn sample_meta() -> Meta {
        Meta {
            orig_name: b"invoice.pdf".to_vec(),
            declared_ext: "pdf".into(),
            declared_mime: "application/pdf".into(),
            detected_kind: "pe-executable".into(),
            risk: RiskLevel::Executable,
            sender: pid(7),
            received_at: 1_700_000_000,
            expires_at: 1_700_600_000,
            scan: ScanOutcome::Unavailable,
            xfer: "xfer-42".into(),
        }
    }

    #[test]
    fn seal_open_roundtrip_reconstructs_original() {
        let original: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
        let sha = [0xABu8; 32];
        let bytes = Beepq::seal(&original, sha, &sample_meta());

        let q = Beepq::open(&bytes).unwrap();
        assert_eq!(q.content_sha256, sha);
        assert_eq!(q.original_size, 2000);
        assert_eq!(q.meta, sample_meta());
        assert_eq!(q.unseal(), original, "봉인 프리픽스+본문 = 원본");
    }

    #[test]
    fn prefix_is_cut_from_body_so_body_alone_cannot_load() {
        // 원본 선두 512B는 본문에 없다 — 헤더가 사라져 로더가 해석 불가(봉인의 목적).
        let original: Vec<u8> = (0..1500u32).map(|i| (i % 251) as u8).collect();
        let bytes = Beepq::seal(&original, [0; 32], &sample_meta());
        let q = Beepq::open(&bytes).unwrap();
        assert_eq!(q.sealed_prefix.len(), MAX_SEAL);
        assert_eq!(q.body.len(), 1500 - MAX_SEAL);
        assert_eq!(&q.sealed_prefix[..], &original[..MAX_SEAL]);
        assert_ne!(q.body.first(), original.first(), "본문 선두 ≠ 원본 선두");
    }

    #[test]
    fn small_file_seals_entirely_into_prefix() {
        let original = b"tiny".to_vec();
        let bytes = Beepq::seal(&original, [0; 32], &sample_meta());
        let q = Beepq::open(&bytes).unwrap();
        assert_eq!(q.sealed_prefix, original);
        assert!(q.body.is_empty(), "N=min(512,4)=4 — 전부 프리픽스로");
        assert_eq!(q.unseal(), original);
    }

    #[test]
    fn empty_original_is_valid() {
        let bytes = Beepq::seal(&[], [0; 32], &sample_meta());
        let q = Beepq::open(&bytes).unwrap();
        assert_eq!(q.original_size, 0);
        assert!(q.sealed_prefix.is_empty() && q.body.is_empty());
        assert_eq!(q.unseal(), Vec::<u8>::new());
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = Beepq::seal(b"x", [0; 32], &sample_meta());
        bytes[0] = b'X';
        assert_eq!(Beepq::open(&bytes), Err(QuarantineError::BadMagic));
    }

    #[test]
    fn unknown_version_is_not_opened() {
        // 발견 와이어는 미지 버전을 무시하지만, 격리는 fail-closed로 거부한다.
        let mut bytes = Beepq::seal(b"x", [0; 32], &sample_meta());
        bytes[4] = 99;
        assert_eq!(
            Beepq::open(&bytes),
            Err(QuarantineError::UnknownVersion(99))
        );
    }

    #[test]
    fn truncated_is_rejected() {
        let bytes = Beepq::seal(b"hello world", [0; 32], &sample_meta());
        for cut in [0, HEADER - 1, HEADER + 2] {
            assert_eq!(
                Beepq::open(&bytes[..cut]).err(),
                Some(QuarantineError::Truncated),
                "cut={cut}"
            );
        }
    }

    #[test]
    fn corrupt_size_field_is_caught() {
        let mut bytes = Beepq::seal(b"hello", [0; 32], &sample_meta());
        bytes[40] ^= 0xFF; // original_size 훼손
        assert_eq!(Beepq::open(&bytes), Err(QuarantineError::SizeMismatch));
    }

    #[test]
    fn unknown_meta_tag_is_skipped_forward_compat() {
        // v1.x가 메타에 태그 200을 추가해도 v1 파서는 죽지 않고 건너뛴다.
        let mut bytes = Beepq::seal(b"data", [0; 32], &sample_meta());
        // meta_len 위치: HEADER + n(=4). 태그 하나(9바이트: tag1+len4+val4)를 메타에 주입.
        let meta_len_at = HEADER + 4;
        let old = u32::from_le_bytes(bytes[meta_len_at..meta_len_at + 4].try_into().unwrap());
        let extra = {
            let mut v = vec![200u8];
            v.extend_from_slice(&4u32.to_le_bytes());
            v.extend_from_slice(b"noop");
            v
        };
        // 메타 끝(body 시작 전)에 삽입 + meta_len 갱신.
        let body_start = meta_len_at + 4 + old as usize;
        bytes.splice(body_start..body_start, extra.iter().copied());
        bytes[meta_len_at..meta_len_at + 4]
            .copy_from_slice(&(old + extra.len() as u32).to_le_bytes());

        let q = Beepq::open(&bytes).unwrap();
        assert_eq!(q.meta, sample_meta(), "미지 태그는 무시, 나머지는 온전");
    }

    /// NXBQ v2(M4-12 · 11-07): 머리 8B · 꼬리 왕복(트레일러로 길이 찾기) · v1 머리는 UnknownVersion(1) · 꼬리 손상 fail-closed.
    #[test]
    fn v2_head_and_tail_roundtrip_and_fail_closed() {
        let h = head_v2(FLAG_SEALED);
        assert_eq!(parse_head_v2(&h), Ok(FLAG_SEALED));
        assert_eq!(parse_head_v2(&h[..5]), Err(QuarantineError::Truncated));
        let v1 = Beepq::seal(b"abc", [1u8; 32], &sample_meta());
        assert_eq!(parse_head_v2(&v1), Err(QuarantineError::UnknownVersion(1)));
        let tail = TailV2 {
            content_sha256: [7u8; 32],
            original_size: 123_456,
            meta: sample_meta(),
        };
        let bytes = tail.encode();
        let n = TailV2::tail_len(&bytes[bytes.len() - 4..]).unwrap();
        assert_eq!(n + 4, bytes.len(), "트레일러 = 꼬리 길이");
        let back = TailV2::decode(&bytes[..n]).unwrap();
        assert_eq!(back, tail);
        assert_eq!(
            TailV2::decode(&bytes[..n - 1]),
            Err(QuarantineError::BadMeta),
            "길이 불일치"
        );
        assert_eq!(
            TailV2::decode(&bytes[..10]),
            Err(QuarantineError::Truncated)
        );
        assert!(
            TailV2::tail_len(&[1, 0, 0, 0]).is_none(),
            "고정부보다 짧은 꼬리 길이 = 손상"
        );
    }

    #[test]
    fn every_risk_and_scan_roundtrips() {
        for risk in [
            RiskLevel::Executable,
            RiskLevel::ActiveDocument,
            RiskLevel::Archive,
            RiskLevel::Data,
        ] {
            for scan in [
                ScanOutcome::Unavailable,
                ScanOutcome::Clean,
                ScanOutcome::Detected,
                ScanOutcome::TooLarge,
            ] {
                let mut m = sample_meta();
                m.risk = risk;
                m.scan = scan;
                let bytes = Beepq::seal(b"z", [0; 32], &m);
                let q = Beepq::open(&bytes).unwrap();
                assert_eq!((q.meta.risk, q.meta.scan), (risk, scan));
            }
        }
    }

    #[test]
    fn orig_name_preserves_raw_bytes_unnormalized() {
        // RLO(U+202E)·비UTF8스러운 원본 이름을 정규화 없이 보존해야 사후 감사가 된다.
        let mut m = sample_meta();
        m.orig_name = vec![0xE2, 0x80, 0xAE, b'g', b'p', b'j', b'.', b'e', b'x', b'e'];
        let bytes = Beepq::seal(b"z", [0; 32], &m);
        let q = Beepq::open(&bytes).unwrap();
        assert_eq!(q.meta.orig_name, m.orig_name);
    }

    /// M4-12 0단계: 소유권 경로(seal_owned · open_owned · unseal_owned)는 복사 경로와 바이트가 같다.
    #[test]
    fn owned_paths_match_copying_paths() {
        let original: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        let sha = [7u8; 32];
        let meta = Meta {
            orig_name: b"x.bin".to_vec(),
            declared_ext: "bin".into(),
            declared_mime: String::new(),
            detected_kind: "unknown".into(),
            risk: RiskLevel::Data,
            sender: nbeep_core::PeerId::from_bytes([1u8; 32]),
            received_at: 1,
            expires_at: 2,
            scan: ScanOutcome::Unavailable,
            xfer: String::new(),
        };
        let a = Beepq::seal(&original, sha, &meta);
        let b = Beepq::seal_owned(original.clone(), sha, &meta);
        assert_eq!(a, b);
        let oa = Beepq::open(&a).unwrap();
        let ob = Beepq::open_owned(b).unwrap();
        assert_eq!(oa.body, ob.body);
        assert_eq!(oa.sealed_prefix, ob.sealed_prefix);
        assert_eq!(oa.unseal(), original);
        assert_eq!(ob.unseal_owned(), original);
        assert!(Beepq::open_owned(vec![0u8; 10]).is_err());
    }
}
