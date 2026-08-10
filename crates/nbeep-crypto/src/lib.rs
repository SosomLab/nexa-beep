//! `nbeep-crypto` — 보안 세션 (Noise_XX · TOFU).
//!
//! Noise 핸드셰이크·AEAD 트랜스포트·TOFU 핀 검증·SAS 지문 파생([docs/08] ADR-0002).
//! **암호는 전송 계층 "위"** — 릴레이는 봉투만 본다(DR-7). 소켓·저장은 모른다.
//!
//! [`nbeep_core::session::Session`]을 구현한다. **M2-1a**: `PlainSession` 스텁(암호화 없음 —
//! 상위 계층을 먼저 검증). **M2-1b**: 실물 `NoiseSession`(Noise_XX) — 암호 라이브러리 의존성 결정 후.
#![forbid(unsafe_op_in_unsafe_fn)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod noise;
pub mod sas;

/// 세션 스텁(암호화 없음) — 릴레이 미포함(feature `testkit` 또는 테스트 빌드).
#[cfg(any(test, feature = "testkit"))]
pub mod plain;

pub use noise::{Identity, NoiseSession};
pub use sas::safety_number;

/// 원본 전체의 SHA-256(FR-X-6 파일 무결성 · [docs/11] `.beepq` `content_sha256`).
///
/// 어댑터 함수 — 도메인(`nbeep-safe`의 `HashPort`)은 이 함수를 모르고,
/// 조립 지점(bin)이 포트에 꽂는다(DR-21).
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}
