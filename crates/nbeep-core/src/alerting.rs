//! 알림 강도 판정(M3-8 · ADR-0010 §3-3 "신뢰 등급이 세로축") — **발신자의 주장 × 수신자의 판정**.
//!
//! 와이어를 건너오는 것은 `Importance` 하나뿐이고 [`AlertLevel`]은 전송되지 않는 로컬 값이다 — 그래서
//! 발신자는 내 스피커를 켤 수 없다. 판정은 순수 함수 [`decide_alert`] 한 곳(docs/13 §12-1 "표 하나로
//! 시험 가능"): 신뢰(`TrustLevel`) × 경로(원격) × 팬아웃(그룹·브로드캐스트) × 같은 발신자 `Urgent` 속도.
//! 강등은 **조용히 버리지 않는다** — `demoted`에 사유가 남아 토스트에 표시된다.

use crate::action::{AlertLevel, SuppressReason};
use crate::chat::Importance;
use crate::identity::TrustLevel;

/// 같은 발신자 `Urgent` 속도 창(N분 = 10) — 창 안 1건까지 원 강도, 넘기면 `Toast`, 계속되면 `Badge`.
pub const URGENT_WINDOW_MS: u64 = 10 * 60 * 1000;
/// 창 안에서 원 강도를 허용하는 **이전** 건수(이번 건 제외).
pub const URGENT_WINDOW_LIMIT: u32 = 1;
/// 창 안에서 `Badge`까지 내리는 이전 건수.
pub const URGENT_WINDOW_BADGE: u32 = 3;
/// 24h 표시 창(FR-S-42 — 최근 24시간 `Urgent` 횟수를 수신자 UI에 보인다).
pub const URGENT_DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// 판정 입력.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertInput {
    /// 발신자의 주장.
    pub importance: Importance,
    /// 수신자가 아는 신뢰(형제·내 기기 = FingerprintVerified).
    pub trust: TrustLevel,
    /// 세션이 인터넷 경유인가(FR-S-24 · M5-3c `PathClass::Remote`).
    pub remote: bool,
    /// 그룹·브로드캐스트 팬아웃인가.
    pub fanout: bool,
    /// 같은 발신자의 [`URGENT_WINDOW_MS`] 안 **이전** `Urgent` 건수(이번 건 제외).
    pub urgent_in_window: u32,
    /// 설정 `notify.normal_toast` — `Normal`도 신뢰 상대(검증·핀 · 로컬)면 토스트(종전 08-15 동작 유지 · off = ADR 표 그대로 Badge).
    pub normal_toast: bool,
}

/// 판정 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertDecision {
    /// 실제 강도.
    pub level: AlertLevel,
    /// 미리보기 본문을 숨기고 "새 메시지"만(TOFU 핀 상대 — ADR 표 "내용 숨김").
    pub hide_preview: bool,
    /// 요청보다 내려갔다면 그 사유(표시용 · 조용히 버리지 않는다).
    pub demoted: Option<SuppressReason>,
}

fn one_down(l: AlertLevel) -> AlertLevel {
    match l {
        AlertLevel::Alarm => AlertLevel::Toast,
        AlertLevel::Toast | AlertLevel::Badge => AlertLevel::Badge,
        AlertLevel::Silent => AlertLevel::Silent,
    }
}

/// 기본 매핑(ADR-0010 §3-3 표) + 규칙(팬아웃 1단계 강등 · 속도 강등).
#[must_use]
pub fn decide_alert(i: AlertInput) -> AlertDecision {
    use AlertLevel::{Alarm, Badge, Toast};
    use SuppressReason as S;
    let (mut level, mut demoted, hide_preview) = match (i.importance, i.remote, i.trust) {
        // 원격 경로 — 등급 무관 Badge(위협 모델이 다르다). Normal은 원래 Badge라 강등 아님.
        (Importance::Normal, true, _) => (Badge, None, false),
        (_, true, _) => (Badge, Some(S::Remote), false),
        // Normal — 표는 Badge. normal_toast(기본 on)면 검증·핀 상대만 Toast(핀은 내용 숨김).
        (Importance::Normal, false, TrustLevel::FingerprintVerified) => {
            (if i.normal_toast { Toast } else { Badge }, None, false)
        }
        (Importance::Normal, false, TrustLevel::Pinned) => {
            (if i.normal_toast { Toast } else { Badge }, None, true)
        }
        (Importance::Normal, false, TrustLevel::Unverified) => (Badge, None, false),
        // Notice
        (Importance::Notice, false, TrustLevel::FingerprintVerified) => (Toast, None, false),
        (Importance::Notice, false, TrustLevel::Pinned) => (Toast, None, true),
        (Importance::Notice, false, TrustLevel::Unverified) => (Badge, Some(S::Untrusted), false),
        // Urgent — 소리(Alarm)는 SAS 대조 상대만.
        (Importance::Urgent, false, TrustLevel::FingerprintVerified) => (Alarm, None, false),
        (Importance::Urgent, false, TrustLevel::Pinned) => (Toast, Some(S::Untrusted), true),
        (Importance::Urgent, false, TrustLevel::Unverified) => (Badge, Some(S::Untrusted), false),
    };
    if i.importance == Importance::Urgent {
        // 팬아웃의 Urgent = 기본 1단계 강등.
        if i.fanout && level > Badge {
            level = one_down(level);
            demoted.get_or_insert(S::Fanout);
        }
        // 같은 발신자 속도 — N분 1회를 넘기면 Toast, 계속되면 Badge(강등 사실 표시).
        let cap = if i.urgent_in_window >= URGENT_WINDOW_BADGE {
            Some(Badge)
        } else if i.urgent_in_window > URGENT_WINDOW_LIMIT {
            Some(Toast)
        } else {
            None
        };
        if let Some(c) = cap {
            if level > c {
                level = c;
                demoted.get_or_insert(S::RateLimited);
            }
        }
    }
    AlertDecision {
        level,
        hide_preview,
        demoted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AlertLevel::{Alarm, Badge, Toast};

    fn inp(importance: Importance, trust: TrustLevel) -> AlertInput {
        AlertInput {
            importance,
            trust,
            remote: false,
            fanout: false,
            urgent_in_window: 0,
            normal_toast: true,
        }
    }

    #[test]
    fn table_rows_match_adr_0010() {
        use Importance::{Normal, Notice, Urgent};
        use TrustLevel::{FingerprintVerified as Fv, Pinned, Unverified};
        let d = |i, t| decide_alert(inp(i, t));
        // Normal(normal_toast on) — 신뢰 상대만 Toast · 핀은 내용 숨김 · 미검증 Badge.
        assert_eq!(d(Normal, Fv).level, Toast);
        assert!(d(Normal, Pinned).hide_preview && d(Normal, Pinned).level == Toast);
        assert_eq!(d(Normal, Unverified).level, Badge);
        // normal_toast off = ADR 표 그대로 전부 Badge.
        let strict = AlertInput {
            normal_toast: false,
            ..inp(Normal, Fv)
        };
        assert_eq!(decide_alert(strict).level, Badge);
        // Notice
        assert_eq!(d(Notice, Fv).level, Toast);
        assert!(d(Notice, Pinned).level == Toast && d(Notice, Pinned).hide_preview);
        assert_eq!(
            (d(Notice, Unverified).level, d(Notice, Unverified).demoted),
            (Badge, Some(SuppressReason::Untrusted))
        );
        // Urgent — 소리는 SAS 대조 상대만.
        assert_eq!(d(Urgent, Fv).level, Alarm);
        assert_eq!(
            (d(Urgent, Pinned).level, d(Urgent, Pinned).demoted),
            (Toast, Some(SuppressReason::Untrusted))
        );
        assert_eq!(d(Urgent, Unverified).level, Badge);
    }

    #[test]
    fn remote_path_is_badge_regardless_of_trust() {
        let r = AlertInput {
            remote: true,
            ..inp(Importance::Urgent, TrustLevel::FingerprintVerified)
        };
        let d = decide_alert(r);
        assert_eq!((d.level, d.demoted), (Badge, Some(SuppressReason::Remote)));
        // Normal 원격은 원래 Badge — 강등 표시 없음.
        let n = AlertInput {
            remote: true,
            ..inp(Importance::Normal, TrustLevel::FingerprintVerified)
        };
        assert_eq!(decide_alert(n).demoted, None);
    }

    #[test]
    fn fanout_urgent_steps_down_once() {
        let f = AlertInput {
            fanout: true,
            ..inp(Importance::Urgent, TrustLevel::FingerprintVerified)
        };
        let d = decide_alert(f);
        assert_eq!((d.level, d.demoted), (Toast, Some(SuppressReason::Fanout)));
        // 팬아웃 Notice는 영향 없음.
        let n = AlertInput {
            fanout: true,
            ..inp(Importance::Notice, TrustLevel::FingerprintVerified)
        };
        assert_eq!(decide_alert(n).level, Toast);
    }

    #[test]
    fn urgent_rate_demotes_toast_then_badge() {
        let base = inp(Importance::Urgent, TrustLevel::FingerprintVerified);
        assert_eq!(decide_alert(base).level, Alarm);
        let one = AlertInput {
            urgent_in_window: 1,
            ..base
        };
        assert_eq!(decide_alert(one).level, Alarm, "창 안 1건까지는 원 강도");
        let two = AlertInput {
            urgent_in_window: 2,
            ..base
        };
        let d2 = decide_alert(two);
        assert_eq!(
            (d2.level, d2.demoted),
            (Toast, Some(SuppressReason::RateLimited))
        );
        let many = AlertInput {
            urgent_in_window: 3,
            ..base
        };
        assert_eq!(decide_alert(many).level, Badge);
        // 이미 더 낮은 판정(핀 → Toast)은 속도 사유로 덮어쓰지 않는다(첫 사유 유지).
        let pinned = AlertInput {
            urgent_in_window: 2,
            ..inp(Importance::Urgent, TrustLevel::Pinned)
        };
        assert_eq!(
            decide_alert(pinned).demoted,
            Some(SuppressReason::Untrusted)
        );
    }
}
