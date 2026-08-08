//! L1 링크 상태 — 이벤트 모델 + **디바운서**(M1-2 · FR-D-5).
//!
//! 링크 변화(도킹·Wi-Fi 전환·VPN 토글)는 **구독**한다 — 폴링 금지. OS 구독은 `nbeep-plat`
//! (`PF_ROUTE`/netlink/`NotifyIpInterfaceChange`)이 하고, core는 이벤트 타입과 **디바운스
//! 판정**(순수 로직 — 시각 주입으로 결정적 테스트)만 갖는다.
//!
//! 디바운스가 필수인 이유: 도킹 한 번에 OS는 수십 개의 인터페이스/주소 이벤트를 쏟아낸다.
//! 그때마다 재발견을 돌리면 발견 트래픽이 폭주한다(NFR-B). **조용해진 뒤 1회**로 접는다
//! (trailing debounce — 마지막 이벤트 후 `quiet_ms` 경과 시 발화).

use crate::ports::MonoInstant;

/// 링크 변화 이벤트 — v1 최소는 "무언가 변했다"(재발견 트리거로 충분).
/// 인터페이스별 세분화(Up/Down·주소 변경)는 S1~S6 인터페이스별 바인딩(M1-3)과 함께.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkEvent {
    /// 인터페이스/주소 구성이 변했다 — 디바운스 후 재발견을 돌려라.
    Changed,
}

/// trailing 디바운서 — 이벤트 폭주를 "조용해진 뒤 1회"로 접는다.
///
/// `quiet_ms`는 D-8b 실측 후 확정값을 주입한다(하드코딩 금지 — [docs/08 §8]).
#[derive(Debug)]
pub struct Debouncer {
    quiet_ms: u32,
    deadline: Option<MonoInstant>,
}

impl Debouncer {
    /// `quiet_ms` 동안 조용하면 발화하는 디바운서.
    #[must_use]
    pub fn new(quiet_ms: u32) -> Self {
        Self {
            quiet_ms,
            deadline: None,
        }
    }

    /// 이벤트 관측 — 마감을 `now + quiet_ms`로 민다(연속 이벤트 = 마감 연장).
    pub fn observe(&mut self, now: MonoInstant) {
        self.deadline = Some(MonoInstant(
            now.0.saturating_add(u64::from(self.quiet_ms) * 1_000_000),
        ));
    }

    /// 주기 점검 — 마감이 지났으면 `true`(재발견 1회) 후 리셋.
    pub fn fire(&mut self, now: MonoInstant) -> bool {
        match self.deadline {
            Some(d) if now.0 >= d.0 => {
                self.deadline = None;
                true
            }
            _ => false,
        }
    }

    /// 대기 중인가(발화 예약 존재).
    #[must_use]
    pub fn pending(&self) -> bool {
        self.deadline.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> MonoInstant {
        MonoInstant(ms * 1_000_000)
    }

    #[test]
    fn fires_once_after_quiet_period() {
        let mut d = Debouncer::new(500);
        d.observe(at(0));
        assert!(!d.fire(at(499)), "조용 기간 전");
        assert!(d.fire(at(500)), "조용 기간 경과 = 발화");
        assert!(!d.fire(at(501)), "리셋 — 재발화 없음");
        assert!(!d.pending());
    }

    #[test]
    fn burst_extends_deadline_to_single_fire() {
        // 도킹 폭주: 0·100·200ms 이벤트 3연발 → 마지막 기준 1회만.
        let mut d = Debouncer::new(500);
        d.observe(at(0));
        d.observe(at(100));
        d.observe(at(200));
        assert!(!d.fire(at(500)), "첫 이벤트 기준 아님");
        assert!(!d.fire(at(699)));
        assert!(d.fire(at(700)), "마지막 이벤트 + quiet");
    }

    #[test]
    fn no_event_no_fire() {
        let mut d = Debouncer::new(500);
        assert!(!d.fire(at(10_000)));
    }

    #[test]
    fn new_burst_after_fire_works_again() {
        let mut d = Debouncer::new(500);
        d.observe(at(0));
        assert!(d.fire(at(500)));
        d.observe(at(1_000));
        assert!(d.fire(at(1_500)), "발화 후 새 폭주도 동작");
    }
}
