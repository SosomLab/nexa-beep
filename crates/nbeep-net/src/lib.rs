//! `nbeep-net` — 전송·발견 (Transport 어댑터).
//!
//! L1~L4 직접 제어([docs/06]) · 발견 폴백 S1~S6 · TCP `Link` 생성 · 재연결.
//! [`nbeep_core`]가 쓰는 전송을 구현하는 **어댑터**다.
//!
//! **경계 규칙([docs/09] ADR-0003)** — 이 크레이트가 지키는 불변식:
//! 1. `Locator`(IP·포트)는 이 크레이트 **밖으로 나가지 않는다**(`pub(crate)`).
//!    상위 계층은 `PeerId`만 안다.
//! 2. 발견 결과 [`PeerHint`]는 **미검증** — 신원은 세션 핸드셰이크(M2)로만 확정된다.
//! 3. 연결과 대화는 분리 — 대화는 `PeerId`에, 연결(`Link`)은 왔다 갔다 한다.
//!
//! **M1 범위**: `Transport` 경계 + `InMemoryTransport`(fake — 소켓 없이 상위 계층 테스트).
//! 실제 `LocalDirectTransport`(멀티캐스트 발견)는 M1-2~4, `Session`(Noise)은 M2.
#![forbid(unsafe_op_in_unsafe_fn)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod transport;
pub mod udp;
pub mod wire;

/// 소켓 없는 전송 fake — 릴리스 미포함(feature `testkit` 또는 테스트 빌드).
#[cfg(any(test, feature = "testkit"))]
pub mod inmem;

// `Link`/`LinkError`는 core 소유(Session이 net을 몰라도 되게 — [docs/09]). 편의 재수출.
pub use nbeep_core::link::{Link, LinkError};
pub use transport::{Caps, ConnectError, DiscoveryEvent, PeerHint, Transport};
pub use wire::{CloneWatch, Decoded, Packet, PacketKind, MAX_PACKET};
