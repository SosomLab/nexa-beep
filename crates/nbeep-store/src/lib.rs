//! `nbeep-store` — 영속 (암호화 기록 · 포터블/폴백 경로).
//!
//! at-rest 암호화·블라인드 인덱스·크립토 셰레딩([docs/17] ADR-0005). 데이터 경로 결정(DR-4).
//! **네트워크로 나가는 경로를 두지 않는다**(NFR-O-5) — 기록은 외부로 노출되지 않는다.
//!
//! ⚠️ **D-21 v1 제약(M2-5 구현 시 반드시 지킬 것)**: 저장 마스터 키를 기기 키에서 **직접 파생하지 않는다.**
//! 마스터 키는 **무작위 생성 → 래핑 키로 한 겹 감싼다**([docs/17] §3 · DR-20 V1-4). 직접 파생이면 보호 수준을
//! 바꿀 때마다 전 기록 재암호화가 필요하고 [docs/20] 다중 기기가 불가능해진다. 지금은 간접 한 겹 비용뿐.
//! (M0-1b에 이 자리를 세운 이유 — 나중에 넣으면 저장 포맷 마이그레이션이 된다.)
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod groupfile;
pub mod sealed;
pub mod trustfile;

pub use groupfile::{FileGroupStore, GroupLoad, MineState, SharedGroup};
pub use trustfile::{FileTrustStore, TrustLoad};
