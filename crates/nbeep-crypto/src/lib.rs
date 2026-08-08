//! `nbeep-crypto` — 보안 세션 (Noise_XX · TOFU).
//!
//! Noise 핸드셰이크·AEAD 트랜스포트·TOFU 핀 검증·SAS 지문 파생([docs/08] ADR-0002).
//! **암호는 전송 계층 "위"** — 릴레이는 봉투만 본다(DR-7). 소켓·저장은 모른다.
#![forbid(unsafe_op_in_unsafe_fn)]
