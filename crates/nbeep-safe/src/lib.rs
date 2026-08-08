//! `nbeep-safe` — 수신 무해화 (`.beepq` 격리).
//!
//! 협상→격리→검사→승인 4단계 게이트([docs/11] ADR-0004). 위험 등급·매직 대조·상태 기계.
//! **이 크레이트는 수신 파일을 실행하지 않는다** — 실행/열기 API를 두지 않는다(FR-S-9).
#![forbid(unsafe_op_in_unsafe_fn)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod container;

pub use container::{Beepq, Meta, QuarantineError, FLAG_SEALED, FORMAT_VER, MAGIC, MAX_SEAL};
