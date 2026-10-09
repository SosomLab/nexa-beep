//! sender copy 프레임(ADR-0015 S3 · docs/46 §5-1) — Control 스트림 태그 [`SENDER_COPY_TAG`].
//!
//! 내가 상대 C에게 보낸 1:1 메시지를 **내 다른 기기(형제)** 에도 건넨다. 받은 형제는
//! 그 메시지를 C와의 대화방에 **내 말풍선**으로 넣는다(알림·ack 없음).
//!
//! ★ 설계 문서는 "내 기기로 가는 사본 = 같은 `ChatMessage`"라 했지만, 봉투에는 **수신자가
//! 없다** — 그대로 보내면 형제는 사본을 발신 기기(나)와의 대화로 오인한다. 그래서 대화
//! 대상(`to`)을 앞에 붙인 별도 프레임으로 싣는다(10-09 S3 착수 시 발견).
//!
//! 와이어: `TAG(1) ‖ kind(1)=1 ‖ to PeerId(32) ‖ name_len(1) ‖ name(≤64 UTF-8) ‖ ChatMessage`.
//! - `kind`는 대화 대상 종류(1 = 기기 PeerId). 미래의 UserId 대상(2)은 미지 kind = `None`.
//! - `name`은 **표시 힌트**(형제가 처음 보는 상대일 때 목록 이름) — 신뢰 근거가 아니다.
//! - 안쪽 `ChatMessage`는 [`ChatMessage::decode`]의 발신자 검증을 그대로 탄다: 사본은
//!   **원 발신 기기만** 보내므로 `sender_device == 세션 인증 상대`가 성립해야 한다.
//!
//! 봉투 원리: 형제 세션(XXpsk3) 안에서만 오가며, 받는 쪽은 형제 세션이 아니면 버린다(앱 몫).

use crate::chat::{ChatMessage, WireError};
use crate::identity::PeerId;

/// Control 스트림 태그 — 프로필 1~3 · UserHello 4 · (5~7 동기 예약) · 8~18 사용 중 다음.
pub const SENDER_COPY_TAG: u8 = 19;

/// 대상 종류: 기기(PeerId).
const KIND_PEER: u8 = 1;

/// 표시 힌트 이름 상한(바이트).
pub const SENDER_COPY_NAME_MAX: usize = 64;

/// sender copy 한 장.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SenderCopy {
    /// 이 메시지가 속한 대화의 상대 기기.
    pub to: PeerId,
    /// 그 상대의 표시 이름 힌트(빈 문자열 = 없음).
    pub to_name: String,
    /// 원 메시지(발신 기기 = 보낸 형제).
    pub msg: ChatMessage,
}

/// 해석 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyError {
    /// 태그가 아니다(다른 Control 프레임).
    NotCopy,
    /// 미지 대상 종류(전방 호환 — 조용히 버린다).
    Kind(u8),
    /// 길이 부족·이름 UTF-8 오류.
    Malformed,
    /// 안쪽 메시지 오류(발신자 불일치 포함).
    Inner(WireError),
}

/// UTF-8 문자 경계를 지키며 바이트 상한으로 자른다.
fn clip_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl SenderCopy {
    /// 인코딩 — 이름은 64바이트(문자 경계)로 자른다.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let name = clip_utf8(&self.to_name, SENDER_COPY_NAME_MAX);
        let inner = self.msg.encode();
        let mut out = Vec::with_capacity(3 + PeerId::LEN + name.len() + inner.len());
        out.push(SENDER_COPY_TAG);
        out.push(KIND_PEER);
        out.extend_from_slice(self.to.as_bytes());
        #[allow(clippy::cast_possible_truncation)] // ≤ 64
        out.push(name.len() as u8);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&inner);
        out
    }

    /// 태그만 보는 빠른 판별(액터의 Control 분기용).
    #[must_use]
    pub fn is_copy(bytes: &[u8]) -> bool {
        bytes.first() == Some(&SENDER_COPY_TAG)
    }

    /// 디코딩 — `authenticated` = 이 프레임을 실어 온 세션의 인증 상대.
    ///
    /// # Errors
    /// 태그·종류·길이·이름·안쪽 메시지 오류 시 [`CopyError`].
    pub fn decode(bytes: &[u8], authenticated: PeerId) -> Result<Self, CopyError> {
        let (&tag, rest) = bytes.split_first().ok_or(CopyError::NotCopy)?;
        if tag != SENDER_COPY_TAG {
            return Err(CopyError::NotCopy);
        }
        let (&kind, rest) = rest.split_first().ok_or(CopyError::Malformed)?;
        if kind != KIND_PEER {
            return Err(CopyError::Kind(kind));
        }
        if rest.len() < PeerId::LEN + 1 {
            return Err(CopyError::Malformed);
        }
        let mut to = [0u8; PeerId::LEN];
        to.copy_from_slice(&rest[..PeerId::LEN]);
        let rest = &rest[PeerId::LEN..];
        let n = usize::from(rest[0]);
        if n > SENDER_COPY_NAME_MAX || rest.len() < 1 + n {
            return Err(CopyError::Malformed);
        }
        let to_name = std::str::from_utf8(&rest[1..=n])
            .map_err(|_| CopyError::Malformed)?
            .to_string();
        let msg = ChatMessage::decode(&rest[1 + n..], authenticated).map_err(CopyError::Inner)?;
        Ok(Self {
            to: PeerId::from_bytes(to),
            to_name,
            msg,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{Importance, MessageBody};

    fn pid(b: u8) -> PeerId {
        PeerId::from_bytes([b; PeerId::LEN])
    }

    fn msg(sender: PeerId, seq: u64, text: &str) -> ChatMessage {
        ChatMessage {
            sender_device: sender,
            seq,
            body: MessageBody::Text(text.into()),
            importance: Importance::Urgent,
            broadcast: false,
        }
    }

    #[test]
    fn roundtrip_keeps_target_name_and_message() {
        let me = pid(1);
        let c = SenderCopy {
            to: pid(9),
            to_name: "철수@mac".into(),
            msg: msg(me, 42, "안녕"),
        };
        let b = c.encode();
        assert!(SenderCopy::is_copy(&b));
        assert_eq!(SenderCopy::decode(&b, me), Ok(c));
    }

    /// 사본은 원 발신 기기만 보낸다 — 다른 세션에서 온 사본(재전달·위조) = 거절.
    #[test]
    fn rejects_copy_not_from_original_sender() {
        let c = SenderCopy {
            to: pid(9),
            to_name: String::new(),
            msg: msg(pid(1), 1, "x"),
        };
        assert_eq!(
            SenderCopy::decode(&c.encode(), pid(2)),
            Err(CopyError::Inner(WireError::SenderMismatch))
        );
    }

    #[test]
    fn long_name_is_clipped_on_char_boundary() {
        let me = pid(1);
        let c = SenderCopy {
            to: pid(9),
            to_name: "가".repeat(40), // 120바이트
            msg: msg(me, 1, "x"),
        };
        let d = SenderCopy::decode(&c.encode(), me).unwrap();
        assert!(d.to_name.len() <= SENDER_COPY_NAME_MAX);
        assert_eq!(d.to_name, "가".repeat(21)); // 63바이트
    }

    #[test]
    fn unknown_kind_and_truncation_and_other_tags() {
        let me = pid(1);
        let mut b = SenderCopy {
            to: pid(9),
            to_name: "a".into(),
            msg: msg(me, 1, "x"),
        }
        .encode();
        assert_eq!(SenderCopy::decode(&b[..10], me), Err(CopyError::Malformed));
        b[1] = 2; // 미래의 UserId 대상
        assert_eq!(SenderCopy::decode(&b, me), Err(CopyError::Kind(2)));
        assert_eq!(SenderCopy::decode(&[4, 0, 0], me), Err(CopyError::NotCopy));
        assert!(!SenderCopy::is_copy(&[11; 9]));
        // 이름 길이가 상한을 넘으면 손상.
        let mut bad = vec![SENDER_COPY_TAG, 1];
        bad.extend_from_slice(&[9u8; PeerId::LEN]);
        bad.push(65);
        bad.extend_from_slice(&[b'a'; 80]);
        assert_eq!(SenderCopy::decode(&bad, me), Err(CopyError::Malformed));
    }
}
