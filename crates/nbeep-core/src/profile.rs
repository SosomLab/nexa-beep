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
//! kind 2 = Info      { flags u8 · (len u16 BE ‖ UTF-8)×켠 필드 · image_len u32 BE
//!                      · [avatar: len u16 BE ‖ UTF-8]    ← flags bit3일 때만(08-14)
//!                      · [border: len u16 BE ‖ UTF-8]    ← flags bit4일 때만(08-14)
//!                    }  flags: bit0=name · bit1=email · bit2=phone · bit3=avatar · bit4=border
//!                              · bit5=image_keep(값 없음 — M3-21 경량 갱신)
//! kind 3 = ImageChunk{ offset u32 BE · last u8 · bytes } ← image_len>0일 때만 이어짐
//! ```
//!
//! 미지 kind는 조용히 무시(전방 호환 — Control 스트림의 다른 미래 메시지와 공존).
//! ★ `avatar`가 `image_len` **뒤**인 이유(08-14): 구버전 decode는 `image_len`까지 읽고
//! **꼬리를 검사하지 않는다** — 그래서 뒤에 붙이면 구버전이 조용히 무시한다. 필드 사이에
//! 끼우면 구버전의 `image_len` 오프셋이 틀어져 깨진다. 내장 아바타는 **키만** 나른다 —
//! 상대도 같은 자산을 내장하고 있으니 그림 바이트를 실어 나를 이유가 없다.

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
        /// 내장 아바타 키(기본정보 공개 + [`crate::avatar::AvatarChoice::Builtin`]일 때 —
        /// 08-14). 수신측은 자기 [`crate::avatar::ZODIAC`]으로 검증하고 미지 키는 버린다.
        avatar: Option<String>,
        /// 아바타 보더 색 `"#RRGGBB"`(기본정보 공개 시 — 08-14). 수신측은
        /// [`crate::avatar::parse_border`]로 검증하고 무효는 버린다.
        border: Option<String>,
        /// **경량 갱신 마커**(M3-21 — flags bit5 · 값 없음): 참이면 "공유 사진은
        /// 그대로 — 네 캐시를 유지하라". 텍스트·토글만 바뀐 변경이 256KiB 사진을
        /// 매번 다시 실어 나르지 않게 한다. 거짓 + `image_len` 0 = 기존 규칙
        /// 그대로 **사진 철회**. 구버전 수신측은 이 비트를 몰라 철회로 읽는다
        /// (다음 성립 프리페치/Full에서 회복 — 전방 관용의 대가).
        image_keep: bool,
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
                avatar,
                border,
                image_keep,
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
                if avatar.is_some() {
                    flags |= 8;
                }
                if border.is_some() {
                    flags |= 16;
                }
                // bit5 = image_keep(M3-21) — 값 필드가 없어 같은 flags 바이트만 쓴다
                // (구버전은 모르는 비트를 안 보므로 오프셋이 틀어지지 않는다).
                if *image_keep {
                    flags |= 32;
                }
                out.push(flags);
                for field in [name, email, phone].into_iter().flatten() {
                    let b = field.as_bytes();
                    let len = u16::try_from(b.len()).unwrap_or(u16::MAX);
                    out.extend_from_slice(&len.to_be_bytes());
                    out.extend_from_slice(&b[..usize::from(len)]);
                }
                out.extend_from_slice(&image_len.to_be_bytes());
                // 확장 필드는 **맨 뒤에 순서대로**(구버전은 image_len에서 읽기를 멈춘다
                // — 전방 호환. 새 필드는 언제나 이 꼬리 뒤에 붙인다).
                for field in [avatar, border].into_iter().flatten() {
                    let b = field.as_bytes();
                    let len = u16::try_from(b.len()).unwrap_or(u16::MAX);
                    out.extend_from_slice(&len.to_be_bytes());
                    out.extend_from_slice(&b[..usize::from(len)]);
                }
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
                fn take_str(p: &mut &[u8], present: bool) -> Option<Option<String>> {
                    if !present {
                        return Some(None);
                    }
                    let len = usize::from(u16::from_be_bytes(p.get(..2)?.try_into().ok()?));
                    let s = std::str::from_utf8(p.get(2..2 + len)?).ok()?.to_string();
                    *p = &p[2 + len..];
                    Some(Some(s))
                }
                let (&flags, mut p) = rest.split_first()?;
                let name = take_str(&mut p, flags & 1 != 0)?;
                let email = take_str(&mut p, flags & 2 != 0)?;
                let phone = take_str(&mut p, flags & 4 != 0)?;
                let image_len = u32::from_be_bytes(p.get(..4)?.try_into().ok()?);
                p = &p[4..];
                let avatar = take_str(&mut p, flags & 8 != 0)?;
                let border = take_str(&mut p, flags & 16 != 0)?;
                Some(ProfileMsg::Info {
                    name,
                    email,
                    phone,
                    image_len,
                    avatar,
                    border,
                    image_keep: flags & 32 != 0,
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
                avatar: None,
                border: None,
                image_keep: false,
            },
            ProfileMsg::Info {
                name: None,
                email: None,
                phone: None,
                image_len: 0,
                avatar: None,
                border: None,
                image_keep: false,
            },
            ProfileMsg::Info {
                name: Some("bob".into()),
                email: None,
                phone: None,
                image_len: 0,
                avatar: Some("tiger".into()),
                border: Some("#3D8BFF".into()),
                image_keep: false,
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
            avatar: None,
            border: None,
            image_keep: false,
        }
        .encode();
        // kind(1)+flags(1)+len(2)+"bob"(3)+image_len(4) = 11 — 이메일·전화 자리가 없다.
        assert_eq!(enc.len(), 11, "아바타 미포함이면 구버전과 같은 바이트");
    }

    /// ★ 전방 호환(08-14) — 아바타 키는 image_len **뒤**라 ① 구버전 인코딩(bit3 없음)을
    /// 신버전이 avatar=None으로 해석하고 ② 신버전 인코딩의 꼬리를 구버전이 무시한다
    /// (구버전 decode는 image_len에서 읽기를 멈추고 꼬리를 검사하지 않았다 — 그 관용을
    /// 여기 신버전에도 유지해 미래 확장 자리를 남긴다).
    #[test]
    fn avatar_tail_is_forward_compatible() {
        // ① bit3 없는 인코딩 = avatar None으로 해석.
        let old = ProfileMsg::Info {
            name: Some("bob".into()),
            email: None,
            phone: None,
            image_len: 0,
            avatar: None,
            border: None,
            image_keep: false,
        }
        .encode();
        assert!(matches!(
            ProfileMsg::decode(&old),
            Some(ProfileMsg::Info { avatar: None, .. })
        ));
        // ② 미래 확장 — 아바타 뒤에 모르는 꼬리가 더 붙어도 해석은 성공한다.
        let mut newer = ProfileMsg::Info {
            name: None,
            email: None,
            phone: None,
            image_len: 0,
            avatar: Some("ox".into()),
            border: None,
            image_keep: false,
        }
        .encode();
        newer.extend_from_slice(b"future-bytes");
        assert!(matches!(
            ProfileMsg::decode(&newer),
            Some(ProfileMsg::Info { avatar: Some(a), .. }) if a == "ox"
        ));
    }

    /// ★ M3-21 경량 갱신 — `image_keep`은 flags bit5뿐(값 필드 없음)이라
    /// ① 왕복 보존 ② 레이아웃 불변(바이트 수 동일 — 구버전 오프셋 안전)
    /// ③ 구버전 인코딩(bit5 없음) = false 해석.
    #[test]
    fn image_keep_bit_round_trips_without_layout_change() {
        let base = ProfileMsg::Info {
            name: Some("bob".into()),
            email: Some("b@x.y".into()),
            phone: None,
            image_len: 0,
            avatar: None,
            border: Some("#3D8BFF".into()),
            image_keep: false,
        };
        let kept = match &base {
            ProfileMsg::Info {
                name,
                email,
                phone,
                image_len,
                avatar,
                border,
                ..
            } => ProfileMsg::Info {
                name: name.clone(),
                email: email.clone(),
                phone: phone.clone(),
                image_len: *image_len,
                avatar: avatar.clone(),
                border: border.clone(),
                image_keep: true,
            },
            _ => unreachable!(),
        };
        assert_eq!(ProfileMsg::decode(&kept.encode()).unwrap(), kept);
        assert_eq!(
            base.encode().len(),
            kept.encode().len(),
            "bit5는 값 필드가 없다 — 레이아웃 불변(구버전 오프셋 안전)"
        );
        assert!(matches!(
            ProfileMsg::decode(&base.encode()),
            Some(ProfileMsg::Info {
                image_keep: false,
                ..
            })
        ));
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
