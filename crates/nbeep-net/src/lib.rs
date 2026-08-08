//! `nbeep-net` — 전송·발견 (Transport 어댑터).
//!
//! L1~L4 직접 제어([docs/06]) · 발견 폴백 S1~S6 · TCP `Link` 생성 · 재연결.
//! [`nbeep_core`]가 선언한 포트(`Transport` 등)를 구현하는 **어댑터**다.
//!
//! 경계 규칙([docs/09] ADR-0003): `Locator`(IP·포트)는 이 크레이트 밖으로 나가지 않는다.
//! 상위 계층은 `PeerId`만 안다.
#![forbid(unsafe_op_in_unsafe_fn)]
