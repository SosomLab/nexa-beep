//! `nbeep-core` — 도메인 + 포트 (허브).
//!
//! 대화·그룹·메시지·피어 상태·시퀀스 + 포트 트레이트(`Transport`/`Clock`/`Rng`/`Meter`/`Tracer` 등).
//! **이 크레이트는 `net`·`gfx`·`plat`을 모른다**(순수 도메인 — I/O 없음). 그래서 네트워크·화면 없이 테스트된다.
//! 어댑터가 core에 의존한다(의존성 역전 — docs/13 §2-4). core는 nbeep 크레이트에 의존하지 않는다.
//!
//! 횡단 구조([docs/13])는 M0-1b에서 채운다 — `ActionKind`·`ActionCtx`·`Interceptor`·포트.
#![forbid(unsafe_code)]
