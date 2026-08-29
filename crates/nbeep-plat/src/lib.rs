//! `nbeep-plat` — 플랫폼 경계 (창·입력·IME·DPI·트레이·격리표식·L1 링크상태·AMSI).
//!
//! 플랫폼 의존 코드를 여기 하나에 격리(NFR-C-3). OS 동작 어댑터(`PlatformConventions`, [docs/14 §6]).
//! `unsafe`는 이 크레이트와 `nbeep-net`의 지정 모듈에만(docs/13 §10).
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod autostart;
pub mod clipboard;
pub mod clock;
pub mod conventions;
pub mod font;
pub mod gui;
pub mod host;
pub mod keytap;
pub mod launch;
pub mod linkwatch;
pub mod notify;
pub mod paths;
pub mod quarantine;
pub mod scan;
pub mod shutdown;
pub mod term;
pub mod theme;
pub mod tray;
pub mod wlactivate;

/// Linux 클립보드 자체 구현(08-29 L-1 — Wayland data-control/data_device · X11 셀렉션).
#[cfg(target_os = "linux")]
pub mod linuxclip;
/// macOS 클립보드 자체 구현(08-30 L-1 mac 판 — NSPasteboard 직접).
#[cfg(target_os = "macos")]
pub(crate) mod macclip;
