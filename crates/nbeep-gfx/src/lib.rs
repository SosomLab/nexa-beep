//! `nbeep-gfx` — 렌더 코어 (CPU 래스터라이저 · 텍스트 스택).
//!
//! 픽셀 버퍼에 직접 그린다(ADR-0001 B안 — GPU·OS 위젯 미사용). 무효화 사각형·폰트 셰이핑.
//! 플랫폼 중립 — 창·입력을 모른다(그건 `nbeep-plat`).
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod surface;
pub mod text;

pub use surface::{Color, Surface};
pub use text::{Font, FontError, TextStyle};
