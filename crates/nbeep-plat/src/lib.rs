//! `nbeep-plat` — 플랫폼 경계 (창·입력·IME·DPI·트레이·격리표식·L1 링크상태·AMSI).
//!
//! 플랫폼 의존 코드를 여기 하나에 격리(NFR-C-3). OS 동작 어댑터(`PlatformConventions`, [docs/14 §6]).
//! `unsafe`는 이 크레이트와 `nbeep-net`의 지정 모듈에만(docs/13 §10).
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod clipboard;
pub mod clock;
pub mod font;
pub mod paths;
pub mod quarantine;
pub mod shutdown;
