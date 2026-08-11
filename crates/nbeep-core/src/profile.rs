//! 프로필 교환(M3-17 · ADR-0008 · DR-22) — **Control 스트림** 위 요청-응답.
//!
//! - **브로드캐스트에 절대 싣지 않는다** — 세션(암호화·신원 확정) 위에서만 오간다.
//! - 응답은 **켠 필드만** 싣는다 — 미공개 필드는 값이 아니라 **필드 자체가 없다**
//!   (fail-closed · FR-S-49 취지의 세션판).
//! - 이미지는 바이트만 나른다 — **디코드는 격리 프로세스(M4-5 imgdec) 몫**(R-5).
//!   수신측은 상한·길이 정합만 검증하고 픽셀을 만들지 않는다.
//!
//! ## 와이어 (`StreamId::Control`)
//!
//! ```text
//! kind 1 = Request   {}                                  ← 상대 프로필 요청
//! kind 2 = Info      { flags u8 · (len u16 BE ‖ UTF-8)×켠 필드 · image_len u32 BE }
//!                      flags: bit0=name · bit1=email · bit2=phone
//! kind 3 = ImageChunk{ offset u32 BE · last u8 · bytes } ← image_len>0일 때만 이어짐
//! ```
//!
//! 미지 kind는 조용히 무시(전방 호환 — Control 스트림의 다른 미래 메시지와 공존).

/// 프로필 이미지 총량 상한(바이트) — 초과 응답은 이미지 생략, 초과 수신은 폐기.
pub const PROFILE_IMAGE_MAX: usize = 256 * 1024;
/// 이미지 청크 크기(Noise 프레임 상한 아래 · 파일 전송과 같은 32KiB).
pub const PROFILE_IMAGE_CHUNK: usize = 32 * 1024;

const K_REQUEST: u8 = 1;
const K_INFO: u8 = 2;
const K_IMAGE: u8 = 3;

/// 프로필 교환 메시지.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileMsg {
    /// 상대 프로필 요청(세션 성립 후 자동 프리페치 — ADR-0008).
    Request,
    /// 프로필 응답 — **켠 필드만**. `image_len` 0 = 이미지 없음/미공개.
    Info {
        /// 표시 이름(기본정보 공개 시).
        name: Option<String>,
        /// 이메일(개별 공개 시).
        email: Option<String>,
        /// 전화번호(개별 공개 시).
        phone: Option<String>,
        /// 뒤따르는 이미지 총 바이트(0 = 없음).
        image_len: u32,
    },
    /// 이미지 조각(Info 직후 순서대로 · `last`가 마지막 표시).
    ImageChunk {
        /// 시작 오프셋(순서·유실 검증).
        offset: u32,
        /// 마지막 조각인가.
        last: bool,
        /// 바이트.
        bytes: Vec<u8>,
    },
}

impl ProfileMsg {
    /// 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ProfileMsg::Request => vec![K_REQUEST],
            ProfileMsg::Info {
                name,
                email,
                phone,
                image_len,
            } => {
                let mut out = Vec::with_capacity(64);
                out.push(K_INFO);
                let mut flags = 0u8;
                if name.is_some() {
                    flags |= 1;
                }
                if email.is_some() {
                    flags |= 2;
                }
                if phone.is_some() {
                    flags |= 4;
                }
                out.push(flags);
                for field in [name, email, phone].into_iter().flatten() {
                    let b = field.as_bytes();
                    let len = u16::try_from(b.len()).unwrap_or(u16::MAX);
                    out.extend_from_slice(&len.to_be_bytes());
                    out.extend_from_slice(&b[..usize::from(len)]);
                }
                out.extend_from_slice(&image_len.to_be_bytes());
                out
            }
            ProfileMsg::ImageChunk {
                offset,
                last,
                bytes,
            } => {
                let mut out = Vec::with_capacity(6 + bytes.len());
                out.push(K_IMAGE);
                out.extend_from_slice(&offset.to_be_bytes());
                out.push(u8::from(*last));
                out.extend_from_slice(bytes);
                out
            }
        }
    }

    /// 해석 — 미지 kind·손상은 `None`(수신측이 조용히 버린다 · 전방 호환).
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (&kind, rest) = bytes.split_first()?;
        match kind {
            K_REQUEST => Some(ProfileMsg::Request),
            K_INFO => {
                let (&flags, mut p) = rest.split_first()?;
                let mut take_str = |present: bool| -> Option<Option<String>> {
                    if !present {
                        return Some(None);
                    }
                    let len = usize::from(u16::from_be_bytes(p.get(..2)?.try_into().ok()?));
                    let s = std::str::from_utf8(p.get(2..2 + len)?).ok()?.to_string();
                    p = &p[2 + len..];
                    Some(Some(s))
                };
                let name = take_str(flags & 1 != 0)?;
                let email = take_str(flags & 2 != 0)?;
                let phone = take_str(flags & 4 != 0)?;
                let image_len = u32::from_be_bytes(p.get(..4)?.try_into().ok()?);
                Some(ProfileMsg::Info {
                    name,
                    email,
                    phone,
                    image_len,
                })
            }
            K_IMAGE => {
                let offset = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?);
                let last = *rest.get(4)? != 0;
                Some(ProfileMsg::ImageChunk {
                    offset,
                    last,
                    bytes: rest.get(5..)?.to_vec(),
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn roundtrip_all_kinds() {
        let msgs = [
            ProfileMsg::Request,
            ProfileMsg::Info {
                name: Some("홍길동".into()),
                email: None,
                phone: Some("010-1234".into()),
                image_len: 1234,
            },
            ProfileMsg::Info {
                name: None,
                email: None,
                phone: None,
                image_len: 0,
            },
            ProfileMsg::ImageChunk {
                offset: 32 * 1024,
                last: true,
                bytes: vec![7u8; 100],
            },
        ];
        for m in msgs {
            assert_eq!(ProfileMsg::decode(&m.encode()).unwrap(), m);
        }
    }

    /// 미공개 필드는 **필드 자체가 없다** — 빈 문자열조차 실리지 않는다(fail-closed).
    #[test]
    fn unshared_fields_are_absent_not_empty() {
        let enc = ProfileMsg::Info {
            name: Some("bob".into()),
            email: None,
            phone: None,
            image_len: 0,
        }
        .encode();
        // kind(1)+flags(1)+len(2)+"bob"(3)+image_len(4) = 11 — 이메일·전화 자리가 없다.
        assert_eq!(enc.len(), 11);
    }

    /// 미지 kind·손상은 None(무시) — Control 스트림 전방 호환.
    #[test]
    fn unknown_or_corrupt_is_ignored() {
        assert_eq!(ProfileMsg::decode(&[99, 1, 2, 3]), None);
        assert_eq!(ProfileMsg::decode(&[]), None);
        assert_eq!(
            ProfileMsg::decode(&[K_INFO, 1, 0, 200]),
            None,
            "길이 초과 주장"
        );
    }
}
