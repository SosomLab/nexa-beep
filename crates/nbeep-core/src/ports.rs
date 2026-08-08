//! 포트 — 도메인이 선언하고 어댑터가 구현하는 경계([docs/13] §2).
//!
//! **`Clock`·`Rng`를 포트로 두는 것이 가장 값이 크다** — 없으면 타임아웃·재시도·논스·세션 만료
//! 테스트가 전부 비결정적이 된다. **도메인 코드에서 `Instant::now()`·시스템 난수를 직접 부르는 것은 금지**.
//!
//! 테스트 구현체는 `crate::testkit`(feature `testkit` 또는 테스트 빌드).

use crate::action::{ActionKind, Outcome};
use crate::identity::PeerId;

/// 집계용 벽시계(유닉스 epoch 밀리초). 서로 다른 기기 간 비교·정렬에 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallTime(pub u64);

/// 지연 측정용 단조 시계(나노초). **되감기지 않는다** — 소요 시간 계산 전용.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonoInstant(pub u64);

impl MonoInstant {
    /// `self`가 `earlier` 이후일 때 경과 밀리초. 되감김(음수)은 0으로 포화.
    #[must_use]
    pub fn saturating_ms_since(self, earlier: MonoInstant) -> u32 {
        let ns = self.0.saturating_sub(earlier.0);
        u32::try_from(ns / 1_000_000).unwrap_or(u32::MAX)
    }
}

/// 시각 포트 — 벽시계(집계)와 단조 시계(지연)를 분리해 제공.
pub trait Clock: Send + Sync {
    /// 현재 벽시계(유닉스 epoch ms).
    fn now_wall(&self) -> WallTime;
    /// 현재 단조 시각(ns).
    fn now_mono(&self) -> MonoInstant;
}

/// 난수 포트 — 키 생성·논스·백오프 지터. 도메인은 시스템 난수를 직접 부르지 않는다.
pub trait Rng: Send + Sync {
    /// `dst`를 난수로 채운다.
    fn fill_bytes(&self, dst: &mut [u8]);
}

/// 계측 대상 5W([docs/13] §6-2) — 얼마나(양).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Quantity {
    /// 횟수.
    pub count: u32,
    /// 바이트 수.
    pub bytes: u64,
    /// 관여한 고유 상대 수(팬아웃 폭 등).
    pub peers: u32,
    /// 소요 시간(ms).
    pub duration_ms: u32,
}

/// 계측 레저에 기록되는 한 건([docs/13] §6). **내용은 담지 않는다** — 봉투만.
///
/// 상대 `actor`는 이 계층에서는 원시 `PeerId`지만, **레저 직렬화 시 솔티드 해시로** 낮춘다(§6-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterEvent {
    /// 벽시계(집계).
    pub wall: WallTime,
    /// 소요 시간(단조 기반, ms).
    pub duration_ms: u32,
    /// 행위 주체.
    pub actor: Actor,
    /// 무엇을(안정 코드 매핑은 M0-1c).
    pub kind: ActionKind,
    /// 얼마나.
    pub quantity: Quantity,
    /// 결과.
    pub outcome: Outcome,
}

/// 계측 포트 — 사용 기록 레저([docs/13] §6). **로컬 전용 · 자동 전송 없음.**
pub trait Meter: Send + Sync {
    /// 한 건 기록. 실패해도 본 기능을 막지 않는다(§6-6).
    fn record(&self, event: &MeterEvent);
}

/// 추적 포트 — 구조적 로그·스팬([docs/13] §7). **내용·경로·키를 담지 않는다.**
pub trait Tracer: Send + Sync {
    /// 행위 하나의 관측. `outcome`이 `Some`이면 종료(after), `None`이면 시작(before).
    fn observe(&self, ctx: &crate::pipeline::ActionCtx, outcome: Option<&Outcome>);
}

/// 행위 주체 — 자기 자신 또는 상대([docs/13] §6-2 "누가").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Actor {
    /// 자기 자신.
    Local,
    /// 상대 기기(계측 레저에서는 솔티드 해시로 낮춘다).
    Peer(PeerId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_elapsed_ms() {
        let a = MonoInstant(1_000_000_000); // 1s
        let b = MonoInstant(1_500_000_000); // 1.5s
        assert_eq!(b.saturating_ms_since(a), 500);
    }

    #[test]
    fn mono_backwards_saturates_to_zero() {
        let a = MonoInstant(2_000_000_000);
        let b = MonoInstant(1_000_000_000);
        assert_eq!(b.saturating_ms_since(a), 0, "되감김은 0으로 포화");
    }

    #[test]
    fn wall_is_ordered() {
        assert!(WallTime(10) < WallTime(20));
    }
}
