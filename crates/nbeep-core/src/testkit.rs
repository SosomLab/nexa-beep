//! 테스트용 포트 구현체 — 결정적 시계·난수·수집 계측/추적([docs/13] §11).
//!
//! `Clock`·`Rng`를 포트로 둔 값이 여기서 나온다: 이걸로 타임아웃·재시도·논스가 **결정적**이 된다.
//! feature `testkit`(또는 이 크레이트의 테스트 빌드)에서만 컴파일 — 릴리스 바이너리에 들어가지 않는다.
//!
//! 테스트 지원 코드이므로 `Mutex` 잠금 `unwrap`을 허용한다(포이즌 시 패닉이 올바른 동작).
#![allow(clippy::unwrap_used)]

use crate::action::Outcome;
use crate::pipeline::ActionCtx;
use crate::ports::{Clock, Meter, MeterEvent, MonoInstant, Rng, Tracer, WallTime};
use std::sync::Mutex;

/// 고정 시계 — 수동으로 시각을 전진시킨다.
#[derive(Debug)]
pub struct FixedClock {
    wall: Mutex<u64>,
    mono: Mutex<u64>,
}

impl FixedClock {
    /// 초기 벽시계·단조 시각으로 생성.
    #[must_use]
    pub fn new(wall_ms: u64, mono_ns: u64) -> Self {
        Self {
            wall: Mutex::new(wall_ms),
            mono: Mutex::new(mono_ns),
        }
    }

    /// 단조 시각을 `ns`만큼 전진(지연 측정 테스트).
    pub fn advance_mono(&self, ns: u64) {
        *self.mono.lock().unwrap() += ns;
    }

    /// 벽시계를 `ms`만큼 전진.
    pub fn advance_wall(&self, ms: u64) {
        *self.wall.lock().unwrap() += ms;
    }
}

impl Clock for FixedClock {
    fn now_wall(&self) -> WallTime {
        WallTime(*self.wall.lock().unwrap())
    }
    fn now_mono(&self) -> MonoInstant {
        MonoInstant(*self.mono.lock().unwrap())
    }
}

/// 시드 난수 — 결정적(SplitMix64). **테스트 전용 · 암호용 아님.**
#[derive(Debug)]
pub struct SeededRng {
    state: Mutex<u64>,
}

impl SeededRng {
    /// 시드로 생성.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: Mutex::new(seed),
        }
    }

    fn next_u64(state: &mut u64) -> u64 {
        // SplitMix64 — 단순·결정적.
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl Rng for SeededRng {
    fn fill_bytes(&self, dst: &mut [u8]) {
        let mut state = self.state.lock().unwrap();
        for chunk in dst.chunks_mut(8) {
            let bytes = Self::next_u64(&mut state).to_le_bytes();
            let n = chunk.len();
            chunk.copy_from_slice(&bytes[..n]);
        }
    }
}

/// 계측 이벤트를 모아 두는 수집기 — 테스트 단언용.
#[derive(Debug, Default)]
pub struct CollectingMeter {
    events: Mutex<Vec<MeterEvent>>,
}

impl CollectingMeter {
    /// 빈 수집기.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// 지금까지 기록된 이벤트 사본.
    #[must_use]
    pub fn events(&self) -> Vec<MeterEvent> {
        self.events.lock().unwrap().clone()
    }
    /// 기록 건수.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }
    /// 비었는가.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Meter for CollectingMeter {
    fn record(&self, event: &MeterEvent) {
        self.events.lock().unwrap().push(*event);
    }
}

/// 아무것도 하지 않는 계측(널 오브젝트) — `Option` 분기 제거([docs/13] §8).
#[derive(Debug, Default)]
pub struct NoopMeter;

impl Meter for NoopMeter {
    fn record(&self, _event: &MeterEvent) {}
}

/// 관측을 (action_id, 종료여부)로 모아 두는 추적기 — 테스트용.
#[derive(Debug, Default)]
pub struct CollectingTracer {
    observations: Mutex<Vec<(u64, bool)>>,
}

impl CollectingTracer {
    /// 빈 추적기.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// (action_id, 종료여부=outcome.is_some()) 목록.
    #[must_use]
    pub fn observations(&self) -> Vec<(u64, bool)> {
        self.observations.lock().unwrap().clone()
    }
}

impl Tracer for CollectingTracer {
    fn observe(&self, ctx: &ActionCtx, outcome: Option<&Outcome>) {
        self.observations
            .lock()
            .unwrap()
            .push((ctx.action_id.0, outcome.is_some()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_advances() {
        let c = FixedClock::new(1000, 0);
        assert_eq!(c.now_wall(), WallTime(1000));
        c.advance_mono(500_000_000);
        assert_eq!(c.now_mono(), MonoInstant(500_000_000));
        c.advance_wall(5);
        assert_eq!(c.now_wall(), WallTime(1005));
    }

    #[test]
    fn seeded_rng_is_deterministic() {
        let a = SeededRng::new(42);
        let b = SeededRng::new(42);
        let mut ba = [0u8; 16];
        let mut bb = [0u8; 16];
        a.fill_bytes(&mut ba);
        b.fill_bytes(&mut bb);
        assert_eq!(ba, bb, "같은 시드 = 같은 바이트");

        let c = SeededRng::new(43);
        let mut bc = [0u8; 16];
        c.fill_bytes(&mut bc);
        assert_ne!(ba, bc, "다른 시드 = 다른 바이트");
    }

    #[test]
    fn seeded_rng_fills_odd_length() {
        let r = SeededRng::new(1);
        let mut buf = [0u8; 5];
        r.fill_bytes(&mut buf);
        assert!(buf.iter().any(|&b| b != 0), "채워짐");
    }

    #[test]
    fn collecting_meter_records() {
        let m = CollectingMeter::new();
        assert!(m.is_empty());
        // 실제 MeterEvent는 상위 테스트에서 — 여기선 빈 상태만 확인.
        assert_eq!(m.len(), 0);
    }
}
