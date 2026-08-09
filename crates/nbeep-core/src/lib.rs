//! `nbeep-core` — 도메인 + 포트 (허브).
//!
//! 대화·그룹·메시지·피어 상태·시퀀스 + 포트 트레이트. **이 크레이트는 `net`·`gfx`·`plat`을 모른다**
//! (순수 도메인 — I/O 없음). 어댑터가 core에 의존한다(의존성 역전 — [docs/13] §2-4).
//!
//! 횡단 구조([docs/13])의 골격:
//! - [`action::ActionKind`] — 모든 유의미한 행위의 단일 통로.
//! - [`pipeline`] — `Trace→Policy→Validate→Meter→Handler` 인터셉터 파이프라인.
//! - [`ports`] — `Clock`/`Rng`/`Meter`/`Tracer`(도메인이 선언, 어댑터가 구현).
//! - [`identity`] — `PeerId`(기기) · `UserId`(다중 기기, v1은 1:1) · `Recipients`(발신 일반화).
#![forbid(unsafe_code)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod action;
pub mod chat;
pub mod group;
pub mod i18n;
pub mod identity;
pub mod link;
pub mod linkwatch;
pub mod mux;
pub mod name;
pub mod peers;
pub mod pipeline;
pub mod ports;
pub mod rate;
pub mod redact;
pub mod safetext;
pub mod session;
pub mod trust;
pub mod trusted;
pub mod xfer;
pub mod xfer_policy;

#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

// ── 편의 재수출(공개 표면 설계 — 내부 모듈 경로를 그대로 노출하지 않는다) ──
pub use action::{ActionKind, FailCode, Outcome, RejectCode, RiskLevel, ScanOutcome};
pub use chat::{fanout, ChatMessage, DedupIndex, FanoutReport, MessageBody, Sequencer, WireError};
pub use group::{Group, GroupId, GroupStore};
pub use i18n::{current_lang, set_lang, t, tr, Lang, Msg};
pub use identity::{DeviceId, PeerId, Recipients, TrustLevel, UserId};
pub use link::{Link, LinkError};
pub use linkwatch::{Debouncer, LinkEvent};
pub use mux::{MuxSession, StreamId};
pub use name::{DisplayName, NameError};
pub use peers::{DepartReason, PeerEntry, PeerEvent, PeerTable, SourceId};
pub use pipeline::{ActionCtx, ActionId, Interceptor, Pipeline, Reject};
pub use ports::{Actor, Clock, Meter, MeterEvent, MonoInstant, Quantity, Rng, Tracer, WallTime};
pub use rate::{negotiate, Pacer, RateLimit, RateMeter};
pub use safetext::{find_links, sanitize_message, LinkSpan, SafeText};
pub use session::{Session, SessionError};
pub use trust::{MemoryTrustStore, TrustDecision, TrustStore};
pub use trusted::{Established, TrustedSession};
pub use xfer::{
    chunks_of, Received, RejectWhy, XferError, XferId, XferInbox, XferMsg, MAX_CHUNK, MAX_FILE,
};
pub use xfer_policy::{
    check_send_eligibility, judge_offer, ApprovalPolicy, AutoWindow, BasicApproval, DenyReason,
    Exchange, ExchangeLedger, OfferVerdict,
};

#[cfg(test)]
mod integration_tests {
    //! 파이프라인 + 포트 + 신원이 함께 도는 통합 테스트 — **네트워크·화면 없이**.
    //! ([docs/09] "InMemory로 상위 계층을 소켓 없이 테스트"의 씨앗.)

    use super::*;
    use crate::testkit::{CollectingMeter, CollectingTracer, FixedClock};
    use std::sync::Arc;

    /// Trace + Meter 인터셉터가 한 행위를 관측·계측하는 최소 경로.
    struct TraceInterceptor(Arc<CollectingTracer>);
    impl Interceptor for TraceInterceptor {
        fn name(&self) -> &'static str {
            "trace"
        }
        fn before(&self, ctx: &mut ActionCtx) -> Result<(), Reject> {
            self.0.observe(ctx, None);
            Ok(())
        }
        fn after(&self, ctx: &ActionCtx, outcome: &Outcome) {
            self.0.observe(ctx, Some(outcome));
        }
    }

    struct MeterInterceptor {
        meter: Arc<CollectingMeter>,
        clock: Arc<FixedClock>,
    }
    impl Interceptor for MeterInterceptor {
        fn name(&self) -> &'static str {
            "meter"
        }
        fn after(&self, ctx: &ActionCtx, outcome: &Outcome) {
            let ev = MeterEvent {
                wall: self.clock.now_wall(),
                duration_ms: self.clock.now_mono().saturating_ms_since(ctx.started_at),
                actor: ctx.actor,
                kind: ctx.kind,
                quantity: ctx.quantity,
                outcome: *outcome,
            };
            self.meter.record(&ev);
        }
    }

    #[test]
    fn action_flows_through_trace_and_meter_deterministically() {
        let clock = Arc::new(FixedClock::new(1_700_000_000_000, 0));
        let tracer = Arc::new(CollectingTracer::new());
        let meter = Arc::new(CollectingMeter::new());

        let pipe = Pipeline::new()
            .with(Box::new(TraceInterceptor(Arc::clone(&tracer))))
            .with(Box::new(MeterInterceptor {
                meter: Arc::clone(&meter),
                clock: Arc::clone(&clock),
            }));

        let mut ctx = ActionCtx::new(
            ActionId(7),
            ActionKind::MessageSent,
            Actor::Local,
            TrustLevel::Pinned,
            clock.now_mono(),
        );

        // 핸들러가 실제 소요 시간을 만든다(고정 시계를 전진).
        let out = pipe.run(&mut ctx, |c| {
            clock.advance_mono(3_000_000); // 3ms
            c.quantity.count = 1;
            c.quantity.bytes = 42;
            Outcome::Ok
        });

        assert_eq!(out, Outcome::Ok);

        // Trace가 시작·종료 둘 다 관측(거절도 관측되게 하는 최외곽 규약).
        assert_eq!(tracer.observations(), vec![(7, false), (7, true)]);

        // Meter가 정확히 1건 — 결정적 소요 시간 3ms.
        let events = meter.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ActionKind::MessageSent);
        assert_eq!(events[0].duration_ms, 3, "고정 시계로 결정적");
        assert_eq!(events[0].quantity.bytes, 42);
        assert!(events[0].outcome.is_ok());
    }
}
