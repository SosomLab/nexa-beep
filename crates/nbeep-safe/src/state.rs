//! **격리 상태 기계** — 수신 → 격리 → 검사 → 승인 → 실체화(fail-closed).
//!
//! [docs/11 §4] ADR-0004. 불변식:
//! - **자동 승인 경로가 없다** — [`QEvent::Approve`]는 사용자 명시 행위만 발생시킨다(FR-S-9).
//! - **모든 잘못된 전이 = 상태 유지 + 오류**(NFR-S-4) — "잘 모르겠으니 통과"는 없다.
//! - 해시 불일치 = 즉시 폐기(부분 수신물 잔존 금지) · 검사 실패는 `Failed`가 아니라
//!   `Inspected{Unavailable}`(마찰 상승) · 실체화 실패 = `Quarantined` 롤백.
//!
//! 이 모듈은 **순수 도메인**이다 — 파일시스템·OS 표식은 슬라이스 4(어댑터)가 얹는다(DR-21).

use nbeep_core::ScanOutcome;

/// 격리물 상태([docs/11 §4] 그림 그대로).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QState {
    /// 협상 수락 후 수신 중.
    Receiving,
    /// 수신 완료 — 봉인·표식 완료 시점.
    Quarantined,
    /// 검사 끝(결과 포함 — `Unavailable`도 여기다).
    Inspected(ScanOutcome),
    /// 사용자 명시 승인.
    Approved,
    /// 실체화 완료(`.beepq`는 보존 기간까지 유지).
    Materialized,
    /// 사용자 거부 — 삭제 대상.
    Rejected,
    /// 보존 기간 경과 — 삭제 대상.
    Expired,
    /// 수신 실패/취소 또는 해시 불일치 — 즉시 폐기.
    Discarded,
}

/// 상태 전이 사건.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QEvent {
    /// 수신 완료(+SHA-256 대조 결과) — 불일치면 [`QState::Discarded`].
    ReceiveComplete {
        /// 해시 일치 여부.
        hash_ok: bool,
    },
    /// 수신 실패/취소.
    ReceiveFailed,
    /// 검사 완료(못 했으면 `Unavailable` — `Failed` 상태를 만들지 않는다).
    Inspect(ScanOutcome),
    /// **사용자 명시 승인**(자동 경로 없음).
    Approve,
    /// 사용자 거부.
    Reject,
    /// 보존 기간 경과.
    Expire,
    /// 실체화 성공.
    MaterializeOk,
    /// 실체화 실패 — 격리로 롤백(부분 파일은 어댑터가 삭제).
    MaterializeFailed,
}

/// 잘못된 전이 — 상태는 그대로다(fail-closed).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidTransition {
    /// 현재 상태.
    pub state: QState,
    /// 거부된 사건.
    pub event: QEvent,
}

impl core::fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "잘못된 전이: {:?} + {:?} (격리 유지)",
            self.state, self.event
        )
    }
}

impl std::error::Error for InvalidTransition {}

/// 전이 함수 — 허용된 조합만 새 상태를 준다. 그 외 = [`InvalidTransition`](상태 유지).
///
/// # Errors
/// 표에 없는 (상태, 사건) 조합.
pub fn step(state: QState, event: QEvent) -> Result<QState, InvalidTransition> {
    use QEvent as E;
    use QState as S;
    Ok(match (state, event) {
        // 수신 → 격리/폐기. 해시 불일치 = 즉시 폐기(부분 수신물 금지).
        (S::Receiving, E::ReceiveComplete { hash_ok: true }) => S::Quarantined,
        (S::Receiving, E::ReceiveComplete { hash_ok: false }) => S::Discarded,
        (S::Receiving, E::ReceiveFailed) => S::Discarded,
        // 격리 → 검사(실패도 Inspected{Unavailable} — 마찰 상승은 표시 계층 몫).
        (S::Quarantined, E::Inspect(scan)) => S::Inspected(scan),
        // 검사 → 사용자 판정. 승인은 **검사를 거친 뒤에만** 가능하다.
        (S::Inspected(_), E::Approve) => S::Approved,
        (S::Inspected(_), E::Reject) => S::Rejected,
        // 승인 → 실체화. 실패 = 격리 롤백.
        (S::Approved, E::MaterializeOk) => S::Materialized,
        (S::Approved, E::MaterializeFailed) => S::Quarantined,
        // 보존 기간 경과 — 아직 실체화되지 않은 것들만.
        (S::Quarantined | S::Inspected(_) | S::Approved, E::Expire) => S::Expired,
        (s, e) => return Err(InvalidTransition { state: s, event: e }),
    })
}

/// 승인 마찰을 한 단계 올려야 하는가 — 검사 못 한 격리물([docs/11 §4] 전이 규칙).
#[must_use]
pub fn friction_raised(state: QState) -> bool {
    matches!(state, QState::Inspected(ScanOutcome::Unavailable))
}

#[cfg(test)]
mod tests {
    use super::*;
    use QEvent as E;
    use QState as S;

    #[test]
    fn happy_path_to_materialized() {
        let mut s = S::Receiving;
        for e in [
            E::ReceiveComplete { hash_ok: true },
            E::Inspect(ScanOutcome::Clean),
            E::Approve,
            E::MaterializeOk,
        ] {
            s = step(s, e).unwrap();
        }
        assert_eq!(s, S::Materialized);
    }

    #[test]
    fn hash_mismatch_discards_immediately() {
        assert_eq!(
            step(S::Receiving, E::ReceiveComplete { hash_ok: false }).unwrap(),
            S::Discarded,
            "부분 수신물을 남기지 않는다"
        );
    }

    #[test]
    fn scan_failure_is_inspected_unavailable_not_failed() {
        let s = step(S::Quarantined, E::Inspect(ScanOutcome::Unavailable)).unwrap();
        assert_eq!(s, S::Inspected(ScanOutcome::Unavailable));
        assert!(friction_raised(s), "검사 안 됨 = 마찰 한 단계 상승");
        assert!(!friction_raised(S::Inspected(ScanOutcome::Clean)));
    }

    #[test]
    fn approve_requires_inspection_first() {
        // 격리 직후 승인 불가 — 검사를 건너뛰는 경로가 없다.
        let err = step(S::Quarantined, E::Approve).unwrap_err();
        assert_eq!(err.state, S::Quarantined, "상태 유지(fail-closed)");
        // Receiving에서도 불가.
        assert!(step(S::Receiving, E::Approve).is_err());
    }

    #[test]
    fn materialize_failure_rolls_back_to_quarantined() {
        let s = step(S::Approved, E::MaterializeFailed).unwrap();
        assert_eq!(s, S::Quarantined, "롤백 — 재검사·재승인 경로로 복귀");
    }

    #[test]
    fn expire_only_before_materialization() {
        for s in [
            S::Quarantined,
            S::Inspected(ScanOutcome::Clean),
            S::Approved,
        ] {
            assert_eq!(step(s, E::Expire).unwrap(), S::Expired);
        }
        assert!(step(S::Materialized, E::Expire).is_err());
        assert!(step(S::Discarded, E::Expire).is_err());
    }

    #[test]
    fn terminal_states_accept_nothing() {
        for s in [S::Materialized, S::Rejected, S::Expired, S::Discarded] {
            for e in [
                E::ReceiveComplete { hash_ok: true },
                E::Inspect(ScanOutcome::Clean),
                E::Approve,
                E::Reject,
                E::MaterializeOk,
                E::MaterializeFailed,
            ] {
                let err = step(s, e).unwrap_err();
                assert_eq!(err.state, s, "종결 상태는 움직이지 않는다");
            }
        }
    }
}
