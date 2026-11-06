//! `Bye` — **정중한 종료** 제어 메시지(M2-4b "Bye 자리" · ADR-0010 §7 · ADR-0013 P-8 "상태 제어는
//! 세션 안" · 11-06).
//!
//! 앱이 스스로 종료할 때(종료 메뉴·OS 종료 신호·CLI `/quit`) 채널을 끊기 **전에** 각 활성 세션에
//! 1프레임을 보낸다. 받은 쪽은 이어지는 끊김을 **의도된 이탈**로 알고 자동 재연결 백오프를 걸지
//! 않는다(상대가 다시 나타나면 발견이 잇는다 · 사람이 클릭하면 즉시). 종전에는 "정중한 종료 신호
//! 부재"로 /quit 뒤 자동 재연결과 상주 accept가 겹쳤다(DEVLOG 08-13).
//!
//! 와이어 = `BYE_TAG(1) ‖ reason(1)`. Control 스트림 태그 [`BYE_TAG`] = 23(다음 빈 번호 · 11은 두
//! 프레임이 길이로 갈려 피한다 · docs/46 §5-3). 구버전 상대는 미지 태그를 조용히 버린다 — 서버
//! `beepd`는 세션 내부 프레임을 보지 않으므로 무영향.

/// Control 스트림 태그 — 정중한 종료.
pub const BYE_TAG: u8 = 23;

/// 종료 사유 — **종류만**(본문·주소 없음 · 봉투 원리).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ByeReason {
    /// 앱 종료(종료 메뉴 · OS 종료 신호 · `/quit`).
    Shutdown,
    /// 모르는 사유 코드(상위 버전) — 의미는 모르나 "의도된 이탈"로는 취급한다.
    Other(u8),
}

impl ByeReason {
    fn code(self) -> u8 {
        match self {
            ByeReason::Shutdown => 0,
            ByeReason::Other(c) => c,
        }
    }

    fn from_code(c: u8) -> Self {
        match c {
            0 => ByeReason::Shutdown,
            other => ByeReason::Other(other),
        }
    }
}

/// 정중한 종료 프레임.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bye {
    /// 사유.
    pub reason: ByeReason,
}

impl Bye {
    /// 앱 종료 사유의 Bye.
    #[must_use]
    pub fn shutdown() -> Self {
        Self {
            reason: ByeReason::Shutdown,
        }
    }

    /// 직렬화 — `BYE_TAG ‖ reason`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        vec![BYE_TAG, self.reason.code()]
    }

    /// 역직렬화 — 태그·길이가 맞을 때만(다른 Control 프레임은 `None`).
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        match b {
            [BYE_TAG, code] => Some(Self {
                reason: ByeReason::from_code(*code),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bye_roundtrips_and_rejects_foreign() {
        let b = Bye::shutdown();
        let w = b.encode();
        assert_eq!(w, [23, 0]);
        assert_eq!(Bye::decode(&w), Some(b));
        // 상위 버전의 새 사유 코드 = Other로 받되 Bye로 인정(의도된 이탈).
        assert_eq!(
            Bye::decode(&[BYE_TAG, 7]),
            Some(Bye {
                reason: ByeReason::Other(7)
            })
        );
        // 길이가 다르면 Bye가 아니다(태그 충돌 방어 · 빈 입력 · ack 프레임).
        assert_eq!(Bye::decode(&[BYE_TAG]), None);
        assert_eq!(Bye::decode(&[BYE_TAG, 0, 0]), None);
        assert_eq!(Bye::decode(&[]), None);
        assert_eq!(
            Bye::decode(
                &crate::ChatAck {
                    target_seq: 1,
                    kind: crate::AckKind::Read
                }
                .encode()
            ),
            None
        );
    }
}
