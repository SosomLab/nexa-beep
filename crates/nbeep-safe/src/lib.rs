//! `nbeep-safe` — 수신 무해화 (`.beepq` 격리).
//!
//! 협상→격리→검사→승인 4단계 게이트([docs/11] ADR-0004). 위험 등급·매직 대조·상태 기계.
//! **이 크레이트는 수신 파일을 실행하지 않는다** — 실행/열기 API를 두지 않는다(FR-S-9).
#![forbid(unsafe_op_in_unsafe_fn)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod archive;
pub mod container;
pub mod risk;
pub mod sanitize;
pub mod state;
pub mod store;
pub mod zip;

pub use archive::{
    check_archive, check_entry, safe_entry_path, ArchivePolicy, ArchiveReject, EntryDesc,
};
pub use container::{
    head_v2, parse_head_v2, Beepq, Meta, QuarantineError, TailV2, FLAG_SEALED, FORMAT_VER,
    FORMAT_VER_V2, HEAD_V2, MAGIC, MAX_SEAL, TAIL_V2_FIXED,
};
pub use risk::{classify, classify_ext, detect_magic, DetectedKind, Verdict};
pub use sanitize::{sanitize_filename, MAX_NAME_CHARS};
pub use state::{friction_raised, step, InvalidTransition, QEvent, QState};
pub use store::{
    HashPort, MarkOutcome, MarkPort, MaterializeError, Materialized, NoopMark, QuarantineDir,
};
pub use zip::{inspect_zip, looks_like_zip, parse_zip_entries, ZipInspect};
