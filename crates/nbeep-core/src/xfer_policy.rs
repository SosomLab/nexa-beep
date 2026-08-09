//! **파일 수신 정책** — 전송 자격(상호 확인) + 승인 방식(자동/기간/수동/거부).
//!
//! 사용자 확정(08-09):
//! - **전송 자격** = TOFU 핀 + **양방향 대화 1회 이상**(내가 보낸 것 ≥1 **그리고** 받은 것 ≥1).
//!   송신은 언제든 시도할 수 있고 **거절은 수신측이 한다** — 정책은 받는 쪽에 있다.
//!   상호 대화 기록이 없으면 **무조건 거부**(사유를 와이어로 알려 준다).
//! - **승인 방식** 4종: 자동 / **기간 자동**(오늘·6시간·1시간) / 수동(기본) / 수신 거부.
//!   기간이 끝나면 **직전 방식으로 자동 복귀**한다(켜 둔 걸 잊어도 원래대로 돌아온다).
//! - 승인은 **오퍼 1건당 1번**이다 — 2번 보내면 2번 승인해야 한다(일괄 승인 없음).
//!
//! ⚠️ 여기서 "승인"은 **수신 수락(격리까지)**이다. 실행 가능한 실체 파일이 되려면
//! 격리함에서 **별도 승인**이 필요하다([docs/11] §7) — 자동 승인이어도 그 문은 닫혀 있다.

use crate::identity::{PeerId, TrustLevel};
use std::collections::HashMap;

/// 상대별 대화 왕래 기록 — 전송 자격의 근거.
///
/// ⚠️ v1 한계: 프로세스 수명 동안만 유지된다(기록 영속은 M2-5 · D-18 대기).
/// 재시작하면 다시 "상호 확인 전"이 되어 파일 수신이 거부된다 — 의도된 보수적 동작이다.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exchange {
    /// 내가 이 상대에게 보낸 메시지 수.
    pub sent: u32,
    /// 이 상대에게서 받은 메시지 수.
    pub recv: u32,
}

impl Exchange {
    /// **양방향** 왕래가 성립했는가(각각 1회 이상).
    #[must_use]
    pub fn is_mutual(self) -> bool {
        self.sent >= 1 && self.recv >= 1
    }
}

/// 상대별 왕래 장부 — 대화 계층이 갱신하고 파일 계층이 조회한다.
#[derive(Clone, Debug, Default)]
pub struct ExchangeLedger {
    map: HashMap<PeerId, Exchange>,
}

impl ExchangeLedger {
    /// 빈 장부.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// 발신 1건 기록.
    pub fn note_sent(&mut self, peer: PeerId) {
        self.map.entry(peer).or_default().sent += 1;
    }
    /// 수신 1건 기록.
    pub fn note_recv(&mut self, peer: PeerId) {
        self.map.entry(peer).or_default().recv += 1;
    }
    /// 현재 왕래 상태.
    #[must_use]
    pub fn get(&self, peer: PeerId) -> Exchange {
        self.map.get(&peer).copied().unwrap_or_default()
    }
}

/// 수신 거절 사유 — 사용자에게 **왜 막혔는지** 그대로 보여 주기 위해 구체적으로 남긴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    /// 신원이 아직 핀되지 않았다(세션 수립 전 등).
    NotPinned,
    /// **상호 확인되지 않은 사용자** — 양방향 대화 기록이 없다(사용자 확정 규칙).
    NoMutualConversation,
    /// 수신 거부 정책.
    Blocked,
}

impl DenyReason {
    /// 사용자·상대에게 보여 줄 사유 문장.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NotPinned => "신원이 확인되지 않은 상대입니다",
            Self::NoMutualConversation => {
                "상호 확인되지 않은 사용자의 파일 수신은 거부합니다(대화를 먼저 나누세요)"
            }
            Self::Blocked => "수신자가 파일 수신을 거부로 설정했습니다",
        }
    }
}

/// 승인 방식(기간 만료 후 복귀 대상이 되는 **기본형** 3종).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BasicApproval {
    /// 자동 수락(격리까지 — 실체화는 여전히 수동).
    Auto,
    /// 수동 승인(기본값).
    Manual,
    /// 수신 거부.
    Block,
}

/// 승인 설정 — 기간 자동 승인은 **직전 방식**을 품고 있다가 만료 시 되돌아간다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// 기본형(자동·수동·거부).
    Basic(BasicApproval),
    /// 기간 한정 자동 승인 — `until_ms`까지 자동, 이후 `revert_to`로 복귀.
    TimedAuto {
        /// 만료 시각(단조 ms — 호스트 시계).
        until_ms: u64,
        /// 만료 후 돌아갈 방식(= 켜기 직전 방식).
        revert_to: BasicApproval,
    },
}

impl Default for ApprovalPolicy {
    /// 기본 = **수동 승인**(사용자 확정).
    fn default() -> Self {
        Self::Basic(BasicApproval::Manual)
    }
}

/// 기간 자동 승인 길이(사용자 확정 3종).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoWindow {
    /// 1시간.
    Hour1,
    /// 6시간.
    Hour6,
    /// 오늘(24시간).
    Today,
}

impl AutoWindow {
    /// 지속 시간(ms).
    #[must_use]
    pub fn millis(self) -> u64 {
        match self {
            Self::Hour1 => 60 * 60 * 1000,
            Self::Hour6 => 6 * 60 * 60 * 1000,
            Self::Today => 24 * 60 * 60 * 1000,
        }
    }
    /// 설정 값 코드 → 창(미지 = `None`).
    #[must_use]
    pub fn from_code(s: &str) -> Option<Self> {
        match s {
            "1h" => Some(Self::Hour1),
            "6h" => Some(Self::Hour6),
            "today" => Some(Self::Today),
            _ => None,
        }
    }
}

impl ApprovalPolicy {
    /// 기간 자동 승인을 켠다 — 지금 방식이 복귀 대상이 된다(중첩해도 원본을 잃지 않는다).
    #[must_use]
    pub fn start_timed(self, window: AutoWindow, now_ms: u64) -> Self {
        let revert_to = match self {
            // 이미 기간 자동이면 **원래 복귀 대상을 유지**한다 — 연장이 복귀 지점을
            // "자동"으로 바꿔 버리면 영영 안 돌아온다.
            Self::TimedAuto { revert_to, .. } => revert_to,
            Self::Basic(b) => b,
        };
        Self::TimedAuto {
            until_ms: now_ms.saturating_add(window.millis()),
            revert_to,
        }
    }

    /// 만료 확인 — 지났으면 복귀한 새 정책을 함께 준다.
    ///
    /// 반환 `(유효 정책, 복귀했는가)` — `true`면 호스트가 설정을 갱신·표시해야 한다.
    #[must_use]
    pub fn tick(self, now_ms: u64) -> (Self, bool) {
        match self {
            Self::TimedAuto {
                until_ms,
                revert_to,
            } if now_ms >= until_ms => (Self::Basic(revert_to), true),
            other => (other, false),
        }
    }

    /// 지금 자동 수락인가(만료 반영).
    #[must_use]
    pub fn is_auto_now(self, now_ms: u64) -> bool {
        matches!(self.tick(now_ms).0, Self::Basic(BasicApproval::Auto))
            || matches!(self, Self::TimedAuto { until_ms, .. } if now_ms < until_ms)
    }

    /// 남은 시간(ms) — 기간 자동일 때만.
    #[must_use]
    pub fn remaining_ms(self, now_ms: u64) -> Option<u64> {
        match self {
            Self::TimedAuto { until_ms, .. } => Some(until_ms.saturating_sub(now_ms)),
            Self::Basic(_) => None,
        }
    }
}

/// 오퍼 하나에 대한 판정 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferVerdict {
    /// 즉시 수락(자동 승인).
    Accept,
    /// 사용자에게 물어본다(수동 승인 — 오퍼 1건당 1번).
    Ask,
    /// 거절 + 사유.
    Deny(DenyReason),
}

/// **수신 오퍼 판정** — 자격(핀·상호 대화) → 정책(거부/자동/수동) 순.
///
/// 자격 검사가 **항상 먼저**다: 수신 거부 설정을 자동으로 바꿔 놔도, 상호 확인이 안 된
/// 상대의 파일은 여전히 거부된다.
#[must_use]
pub fn judge_offer(
    trust: TrustLevel,
    exchange: Exchange,
    policy: ApprovalPolicy,
    now_ms: u64,
) -> OfferVerdict {
    if matches!(trust, TrustLevel::Unverified) {
        return OfferVerdict::Deny(DenyReason::NotPinned);
    }
    if !exchange.is_mutual() {
        return OfferVerdict::Deny(DenyReason::NoMutualConversation);
    }
    match policy.tick(now_ms).0 {
        ApprovalPolicy::Basic(BasicApproval::Block) => OfferVerdict::Deny(DenyReason::Blocked),
        ApprovalPolicy::Basic(BasicApproval::Auto) => OfferVerdict::Accept,
        ApprovalPolicy::Basic(BasicApproval::Manual) => OfferVerdict::Ask,
        // 만료 전 기간 자동.
        ApprovalPolicy::TimedAuto { .. } => OfferVerdict::Accept,
    }
}

/// **발신 자격 확인**(송신측 사전 점검) — 막혀 있어도 시도 자체는 가능하지만,
/// 미리 알려 주면 사용자가 헛되이 기다리지 않는다.
///
/// # Errors
/// [`DenyReason`] — 상대가 거절할 사유.
pub fn check_send_eligibility(trust: TrustLevel, exchange: Exchange) -> Result<(), DenyReason> {
    if matches!(trust, TrustLevel::Unverified) {
        return Err(DenyReason::NotPinned);
    }
    if !exchange.is_mutual() {
        return Err(DenyReason::NoMutualConversation);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    #[test]
    fn mutual_requires_both_directions() {
        let mut led = ExchangeLedger::new();
        let p = pid(1);
        assert!(!led.get(p).is_mutual(), "왕래 없음");
        led.note_sent(p);
        assert!(!led.get(p).is_mutual(), "보내기만 해선 부족");
        led.note_recv(p);
        assert!(led.get(p).is_mutual(), "양방향 성립");
        // 상대가 먼저 말을 걸어온 것만으로는 자격이 생기지 않는다(스팸 차단).
        let q = pid(2);
        led.note_recv(q);
        led.note_recv(q);
        assert!(!led.get(q).is_mutual());
    }

    #[test]
    fn unverified_or_unmutual_is_denied_regardless_of_policy() {
        let mutual = Exchange { sent: 1, recv: 1 };
        // 자동 승인이어도 미핀 상대는 거부.
        assert_eq!(
            judge_offer(
                TrustLevel::Unverified,
                mutual,
                ApprovalPolicy::Basic(BasicApproval::Auto),
                0
            ),
            OfferVerdict::Deny(DenyReason::NotPinned)
        );
        // 핀됐어도 상호 대화가 없으면 거부(사용자 확정 규칙).
        assert_eq!(
            judge_offer(
                TrustLevel::Pinned,
                Exchange { sent: 0, recv: 5 },
                ApprovalPolicy::Basic(BasicApproval::Auto),
                0
            ),
            OfferVerdict::Deny(DenyReason::NoMutualConversation)
        );
    }

    #[test]
    fn policy_decides_after_eligibility() {
        let m = Exchange { sent: 1, recv: 1 };
        let t = TrustLevel::Pinned;
        assert_eq!(
            judge_offer(t, m, ApprovalPolicy::Basic(BasicApproval::Manual), 0),
            OfferVerdict::Ask
        );
        assert_eq!(
            judge_offer(t, m, ApprovalPolicy::Basic(BasicApproval::Auto), 0),
            OfferVerdict::Accept
        );
        assert_eq!(
            judge_offer(t, m, ApprovalPolicy::Basic(BasicApproval::Block), 0),
            OfferVerdict::Deny(DenyReason::Blocked)
        );
    }

    #[test]
    fn timed_auto_expires_and_reverts_to_previous() {
        let start = ApprovalPolicy::Basic(BasicApproval::Manual);
        let p = start.start_timed(AutoWindow::Hour1, 1_000);
        assert!(p.is_auto_now(1_000), "켠 직후 = 자동");
        assert_eq!(p.remaining_ms(1_000), Some(3_600_000));
        // 만료 전.
        let (still, reverted) = p.tick(1_000 + 3_599_999);
        assert_eq!(still, p);
        assert!(!reverted);
        // 만료 후 = 직전 방식으로 복귀.
        let (after, reverted) = p.tick(1_000 + 3_600_000);
        assert_eq!(after, start, "직전 방식(수동)으로 복귀");
        assert!(reverted, "호스트가 설정을 갱신해야 한다");
        assert!(!after.is_auto_now(1_000 + 3_600_000));
    }

    #[test]
    fn extending_timed_keeps_original_revert_target() {
        // 거부 상태에서 1시간 자동 → 연장해도 복귀 대상은 여전히 "거부".
        let base = ApprovalPolicy::Basic(BasicApproval::Block);
        let p1 = base.start_timed(AutoWindow::Hour1, 0);
        let p2 = p1.start_timed(AutoWindow::Hour6, 10_000);
        let (after, _) = p2.tick(10_000 + AutoWindow::Hour6.millis());
        assert_eq!(
            after, base,
            "연장이 복귀 지점을 자동으로 바꾸면 영영 안 돌아온다"
        );
    }

    #[test]
    fn window_codes_and_durations() {
        assert_eq!(AutoWindow::from_code("1h"), Some(AutoWindow::Hour1));
        assert_eq!(AutoWindow::from_code("6h"), Some(AutoWindow::Hour6));
        assert_eq!(AutoWindow::from_code("today"), Some(AutoWindow::Today));
        assert_eq!(AutoWindow::from_code("nope"), None);
        assert!(AutoWindow::Today.millis() > AutoWindow::Hour6.millis());
    }

    #[test]
    fn send_eligibility_mirrors_receiver_rules() {
        assert_eq!(
            check_send_eligibility(TrustLevel::Unverified, Exchange { sent: 9, recv: 9 }),
            Err(DenyReason::NotPinned)
        );
        assert_eq!(
            check_send_eligibility(TrustLevel::Pinned, Exchange { sent: 1, recv: 0 }),
            Err(DenyReason::NoMutualConversation)
        );
        assert!(check_send_eligibility(TrustLevel::Pinned, Exchange { sent: 1, recv: 1 }).is_ok());
    }
}
