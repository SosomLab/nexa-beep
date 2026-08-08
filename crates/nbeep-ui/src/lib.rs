//! `nbeep-ui` — 화면 (컨트롤 · 레이아웃 · 3단계 이벤트).
//!
//! `WidgetBase` 컴포지션 + 트레이트 기본 메서드 전파([docs/14]). 시각=macOS 통일.
//! [`nbeep_core`] 상태를 읽어 [`nbeep_gfx`]로 그린다. 플랫폼 API를 직접 부르지 않는다.
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod draw;
pub mod event;
pub mod geom;
pub mod peer_list;
pub mod raster;
pub mod theme;
pub mod widget;

pub use draw::{DrawCtx, FontSlot};
pub use event::{InputEvent, Key, WheelAccum, WHEEL_DELTA};
pub use geom::{Point, Rect, Size};
pub use peer_list::{badge, render, PeerRow, ROW_H};
pub use raster::RasterCtx;
pub use theme::{Color, Theme};
pub use widget::{Invalidations, Widget};
