//! `nbeep-ui` — 화면 (컨트롤 · 레이아웃 · 3단계 이벤트).
//!
//! `WidgetBase` 컴포지션 + 트레이트 기본 메서드 전파([docs/14]). 시각=macOS 통일.
//! [`nbeep_core`] 상태를 읽어 [`nbeep_gfx`]로 그린다. 플랫폼 API를 직접 부르지 않는다.
#![forbid(unsafe_op_in_unsafe_fn)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod chat_view;
pub mod controls;
pub mod draw;
pub mod edit;
pub mod event;
pub mod geom;
pub mod peer_list;
pub mod raster;
pub mod settings;
pub mod theme;
pub mod typeahead;
pub mod widget;

pub use chat_view::{ChatLine, ChatViewWidget};
pub use controls::{
    Checkbox, Combo, ComboControl, ComboItem, Control, ControlBase, ExtendedCombo, FlatRow,
    GridColumn, LabelSide, PopupHit, RadioGroup, RadioOption, TextBox, TreeControl, TreeGrid,
    TreeModel, TreeNode, TreeView,
};
pub use draw::{DrawCtx, FontSlot};
pub use edit::{EditKey, EditState};
pub use event::{InputEvent, Key, WheelAccum, WHEEL_DELTA};
pub use geom::{Point, Rect, Size};
pub use peer_list::{badge, PeerListWidget, PeerRow, ROW_H};
pub use raster::RasterCtx;
pub use settings::{registry, Entry, SettingKind, SettingsState, SettingsWidget};
pub use theme::{Color, FontPrefs, SlotFont, Theme};
pub use typeahead::{Query, TypeAhead, TYPEAHEAD_TIMEOUT_MS};
pub use widget::{Invalidations, Widget};
