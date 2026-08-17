//! 대화 수신 확인(N-2 · ADR-0010 §5) — **세 사실을 절대 섞지 않는다**.
//!
//! - **`Delivered`** = 기술적 전달(자동) — 메시지가 상대 세션에 닿아 파싱됐다.
//!   사람의 의사와 무관하며 액터가 자동으로 되쏜다("보냈다"와 "닿았다"의 분리 ·
//!   파일 전송의 [`crate::XferMsg::Received`]와 같은 결).
//! - **`Acknowledged`** = 사람이 **확인 버튼**을 눌렀다(수동 · M3-9). 창이 열린
//!   것은 의사가 아니므로 "읽음"은 만들지 않는다(ADR-0010 §5 — *"읽고 씹었네"*
//!   가 성립하는 근거를 주지 않는다).
//!
//! Control 스트림에 실린다(같은 세션 = 대상 발신자 암묵). 대상은 **내가 보낸
//! 메시지의 `seq`** 하나면 된다 — 그 세션에서 내 seq는 유일하다.

/// 확인 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckKind {
    /// 기술적 전달(자동).
    Delivered,
    /// 사람이 확인 버튼을 누름(수동).
    Acknowledged,
}

/// 대화 확인 메시지 — 대상 seq + 종류. Control 스트림 태그 [`ACK_TAG`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatAck {
    /// 대상 메시지의 발신 seq(수신자가 그대로 되쏜다).
    pub target_seq: u64,
    /// 종류.
    pub kind: AckKind,
}

/// Control 스트림에서 ack를 가르는 태그 — ProfileMsg(1~3)와 겹치지 않게 10.
pub const ACK_TAG: u8 = 10;

impl ChatAck {
    /// 와이어: `ACK_TAG(1) ‖ kind(1) ‖ target_seq(8 BE)`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(10);
        out.push(ACK_TAG);
        out.push(match self.kind {
            AckKind::Delivered => 0,
            AckKind::Acknowledged => 1,
        });
        out.extend_from_slice(&self.target_seq.to_be_bytes());
        out
    }

    /// 해석 — 태그·길이·미지 종류는 `None`(전방 호환 · 수신측이 조용히 버린다).
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 10 || bytes[0] != ACK_TAG {
            return None;
        }
        let kind = match bytes[1] {
            0 => AckKind::Delivered,
            1 => AckKind::Acknowledged,
            _ => return None, // 미지 종류 — 버린다
        };
        let target_seq = u64::from_be_bytes(bytes[2..10].try_into().ok()?);
        Some(Self { target_seq, kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_roundtrips_and_rejects_foreign() {
        for kind in [AckKind::Delivered, AckKind::Acknowledged] {
            let a = ChatAck {
                target_seq: 0x0102_0304_0506_0708,
                kind,
            };
            assert_eq!(ChatAck::decode(&a.encode()), Some(a));
        }
        // ProfileMsg 태그(1~3)는 ack가 아니다(스트림 다중화 구분).
        assert_eq!(ChatAck::decode(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        // 미지 종류(2)는 버린다(전방 호환).
        assert_eq!(ChatAck::decode(&[ACK_TAG, 2, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(ChatAck::decode(&[ACK_TAG, 0]), None, "짧으면 None");
    }
}
