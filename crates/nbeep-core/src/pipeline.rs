//! 인터셉터 파이프라인 — 횡단 관심사를 **하나의 통로**로([docs/13] §3·§5).
//!
//! 모든 유의미한 행위([`ActionKind`])는 이 파이프라인을 통과한다:
//! ```text
//! Trace(최외곽) → Policy → Validate → Meter(최내곽) → Handler
//! ```
//! - **Trace 최외곽** — 거절도 관측된다(안쪽에 두면 차단된 행위가 로그에 안 남는다).
//! - **Policy 다음** — 차단 대상은 검증·계측 전에 끊는다(자원 낭비·증폭 공격 방지).
//! - **Meter 최내곽** — 실제로 수행된 것만 계측.
//!
//! `before`는 등록 순, `after`는 역순(중첩). `before`가 거절하면 그 지점 안쪽 핸들러는 실행되지 않고,
//! **이미 통과한 인터셉터의 `after`만 역순으로** 거절 결과를 관측한다.

use crate::action::{ActionKind, Outcome, RejectCode};
use crate::identity::TrustLevel;
use crate::ports::{Actor, MonoInstant, Quantity};

/// 행위 하나의 상관관계 식별자 — 로그·계측·감사가 이 값으로 이어진다([docs/13] §5-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionId(pub u64);

/// 인터셉터가 공유하는 행위 문맥([docs/13] §5-2).
#[derive(Clone, Debug)]
pub struct ActionCtx {
    /// 상관관계 ID.
    pub action_id: ActionId,
    /// 무엇을.
    pub kind: ActionKind,
    /// 누가.
    pub actor: Actor,
    /// 상대 신뢰 상태(정책·UI 마찰 근거).
    pub trust: TrustLevel,
    /// 시작 단조 시각([`crate::ports::Clock`]에서).
    pub started_at: MonoInstant,
    /// 얼마나(핸들러가 채워 넣을 수 있다).
    pub quantity: Quantity,
}

impl ActionCtx {
    /// 최소 문맥 생성. `quantity`는 기본(0), 핸들러·인터셉터가 갱신한다.
    #[must_use]
    pub fn new(
        action_id: ActionId,
        kind: ActionKind,
        actor: Actor,
        trust: TrustLevel,
        started_at: MonoInstant,
    ) -> Self {
        Self {
            action_id,
            kind,
            actor,
            trust,
            started_at,
            quantity: Quantity::default(),
        }
    }
}

/// 인터셉터가 행위를 막을 때 반환하는 거절.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reject {
    /// 거절 코드(안정 — 계측·표시에 쓰인다).
    pub code: RejectCode,
}

impl Reject {
    /// 코드로부터 거절 생성.
    #[must_use]
    pub const fn new(code: RejectCode) -> Self {
        Self { code }
    }
}

/// 파이프라인에 끼어드는 확장점([docs/13] §5-1).
///
/// 기본 메서드를 둔다 — 구현체는 필요한 훅만 덮어쓴다(추상 클래스 템플릿 메서드 효과).
/// 나중에 훅을 추가해도 기존 구현이 깨지지 않는다.
pub trait Interceptor: Send + Sync {
    /// 식별용 이름(로그·진단).
    fn name(&self) -> &'static str;

    /// 실행 전. `Err`를 반환하면 행위는 수행되지 않는다(fail-closed).
    fn before(&self, ctx: &mut ActionCtx) -> Result<(), Reject> {
        let _ = ctx;
        Ok(())
    }

    /// 실행 후. **결과를 바꿀 수 없다** — 관측 전용(관측/제어 분리).
    fn after(&self, ctx: &ActionCtx, outcome: &Outcome) {
        let _ = (ctx, outcome);
    }
}

/// 등록된 인터셉터 체인. 조립 시점에 한 번 구성한다([docs/13] §5-4 — 런타임 동적 추가는 v1 밖).
#[derive(Default)]
pub struct Pipeline {
    interceptors: Vec<Box<dyn Interceptor>>,
}

// Vec<Box<dyn Interceptor>>는 Debug가 없으므로 수동 구현(missing_debug_implementations).
impl core::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let names: Vec<&'static str> = self.interceptors.iter().map(|i| i.name()).collect();
        f.debug_struct("Pipeline")
            .field("interceptors", &names)
            .finish()
    }
}

impl Pipeline {
    /// 빈 파이프라인.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 인터셉터를 **등록 순으로** 추가(빌더). Trace를 먼저, Meter를 마지막에.
    #[must_use]
    pub fn with(mut self, interceptor: Box<dyn Interceptor>) -> Self {
        self.interceptors.push(interceptor);
        self
    }

    /// 등록된 인터셉터 이름(진단·테스트).
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.interceptors.iter().map(|i| i.name()).collect()
    }

    /// 행위를 파이프라인에 태워 실행한다.
    ///
    /// `before`를 등록 순으로 돌리고, 하나라도 거절하면 **거절 지점보다 바깥(이미 `before`가 성공한)
    /// 인터셉터의 `after`만 역순**으로 거절 결과와 함께 관측시킨 뒤 종료한다(핸들러 미실행).
    /// 전부 통과하면 `handler`를 실행하고 모든 `after`를 역순으로 관측시킨다.
    ///
    /// **`after`는 `before`가 성공한 인터셉터에만** 온다(짝 규약):
    /// - 거절한 인터셉터 자신의 `after`는 실행되지 않는다 — 거절을 만든 층은 이미 인라인으로 알고 있고,
    ///   정리가 필요하면 `Err` 반환 전에 하면 된다.
    /// - 거절 지점보다 안쪽 인터셉터는 `before`가 아예 안 돌았으므로 `after`도 없다.
    ///
    /// 그래서 Trace(최외곽)는 어떤 안쪽 거절이든 관측하고, Meter(최내곽)는 **수행된 것만** 계측한다
    /// (정책이 막은 시도는 Meter에 오지 않는다 — [docs/13] §3-2).
    pub fn run<H>(&self, ctx: &mut ActionCtx, handler: H) -> Outcome
    where
        H: FnOnce(&mut ActionCtx) -> Outcome,
    {
        let mut passed = 0usize;
        for interceptor in &self.interceptors {
            match interceptor.before(ctx) {
                Ok(()) => passed += 1,
                Err(reject) => {
                    let outcome = Outcome::Rejected(reject.code);
                    for done in self.interceptors[..passed].iter().rev() {
                        done.after(ctx, &outcome);
                    }
                    return outcome;
                }
            }
        }

        let outcome = handler(ctx);
        for interceptor in self.interceptors.iter().rev() {
            interceptor.after(ctx, &outcome);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionKind;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    fn ctx() -> ActionCtx {
        ActionCtx::new(
            ActionId(1),
            ActionKind::MessageSent,
            Actor::Local,
            TrustLevel::Pinned,
            MonoInstant(0),
        )
    }

    /// 순서를 기록하는 스파이 인터셉터.
    struct Spy {
        name: &'static str,
        log: Arc<Mutex<Vec<String>>>,
        reject: Option<RejectCode>,
    }

    impl Interceptor for Spy {
        fn name(&self) -> &'static str {
            self.name
        }
        fn before(&self, _ctx: &mut ActionCtx) -> Result<(), Reject> {
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:before", self.name));
            match self.reject {
                Some(code) => Err(Reject::new(code)),
                None => Ok(()),
            }
        }
        fn after(&self, _ctx: &ActionCtx, outcome: &Outcome) {
            self.log
                .lock()
                .unwrap()
                .push(format!("{}:after({outcome:?})", self.name));
        }
    }

    fn spy(
        name: &'static str,
        log: &Arc<Mutex<Vec<String>>>,
        reject: Option<RejectCode>,
    ) -> Box<Spy> {
        Box::new(Spy {
            name,
            log: Arc::clone(log),
            reject,
        })
    }

    #[test]
    fn before_in_order_after_in_reverse() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pipe = Pipeline::new()
            .with(spy("trace", &log, None))
            .with(spy("policy", &log, None))
            .with(spy("meter", &log, None));

        let ran = Arc::new(AtomicU32::new(0));
        let ran2 = Arc::clone(&ran);
        let out = pipe.run(&mut ctx(), move |_| {
            ran2.fetch_add(1, Ordering::SeqCst);
            Outcome::Ok
        });

        assert_eq!(out, Outcome::Ok);
        assert_eq!(ran.load(Ordering::SeqCst), 1, "핸들러 1회");
        let seq = log.lock().unwrap().clone();
        assert_eq!(
            seq,
            vec![
                "trace:before",
                "policy:before",
                "meter:before",
                "meter:after(Ok)",
                "policy:after(Ok)",
                "trace:after(Ok)",
            ]
        );
    }

    #[test]
    fn reject_skips_handler_and_only_outer_passed_get_after() {
        let log = Arc::new(Mutex::new(Vec::new()));
        // policy가 거절 → 안쪽 meter.before는 안 돌고, policy 자신의 after도 안 돈다.
        // 바깥에서 이미 before가 성공한 trace의 after만 역순으로.
        let pipe = Pipeline::new()
            .with(spy("trace", &log, None))
            .with(spy("policy", &log, Some(RejectCode::RateLimited)))
            .with(spy("meter", &log, None));

        let ran = Arc::new(AtomicU32::new(0));
        let ran2 = Arc::clone(&ran);
        let out = pipe.run(&mut ctx(), move |_| {
            ran2.fetch_add(1, Ordering::SeqCst);
            Outcome::Ok
        });

        assert_eq!(out, Outcome::Rejected(RejectCode::RateLimited));
        assert_eq!(ran.load(Ordering::SeqCst), 0, "거절 시 핸들러 미실행");
        let seq = log.lock().unwrap().clone();
        assert_eq!(
            seq,
            vec![
                "trace:before",
                "policy:before",
                // meter:before 없음(안쪽) · policy:after 없음(거절한 층 자신)
                "trace:after(Rejected(RateLimited))",
            ]
        );
    }

    #[test]
    fn empty_pipeline_runs_handler() {
        let out = Pipeline::new().run(&mut ctx(), |_| Outcome::Ok);
        assert_eq!(out, Outcome::Ok);
    }

    #[test]
    fn names_reflect_registration_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let pipe = Pipeline::new()
            .with(spy("a", &log, None))
            .with(spy("b", &log, None));
        assert_eq!(pipe.names(), vec!["a", "b"]);
    }
}
