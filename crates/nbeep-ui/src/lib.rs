//! `nbeep-ui` — 화면 (컨트롤 · 레이아웃 · 3단계 이벤트).
//!
//! `WidgetBase` 컴포지션 + 트레이트 기본 메서드 전파([docs/14]). 시각=macOS 통일.
//! [`nbeep_core`] 상태를 읽어 [`nbeep_gfx`]로 그린다. 플랫폼 API를 직접 부르지 않는다.
#![forbid(unsafe_op_in_unsafe_fn)]
// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정).
#![cfg_attr(test, allow(clippy::unwrap_used))]

/// 앱 브랜딩 아이콘 — raw RGBA(투명 배경)로 임베드. 창 아이콘·이미지 데모 공용.
/// 원본은 `packaging/branding/icon.svg` → `crates/nbeep-ui/assets/brand-64.rgba`.
pub mod brand {
    /// 64×64 RGBA(straight alpha) 바이트(= 64*64*4).
    pub const ICON_RGBA: &[u8] = include_bytes!("../assets/brand-64.rgba");
    /// 변 크기(px).
    pub const ICON_SIZE: u32 = 64;
}

/// 테마 틴트용 알파 마스크 아이콘(SVG 유래) — **모양만** 담는다. 색은 그리는 쪽이
/// 테마 기준색(다크=밝은 회색·라이트=아주 어두운 회색 = `Theme::text`)으로 입힌다.
pub mod icons {
    /// 새로고침(회전 화살표 2개) 96×96 알파 마스크(1채널 = 96*96).
    pub const REFRESH_ALPHA: &[u8] = include_bytes!("../assets/icon-refresh-96.alpha");
    /// 변 크기(px).
    pub const REFRESH_SIZE: u32 = 96;
    /// 직접 등록(+ · 수동 엔드포인트 DR-19) 96×96 알파 마스크.
    pub const ADD_ALPHA: &[u8] = include_bytes!("../assets/icon-add-96.alpha");
    /// 변 크기(px).
    pub const ADD_SIZE: u32 = 96;
    /// 격리함(방패+체크) 96×96 알파 마스크.
    pub const SHIELD_ALPHA: &[u8] = include_bytes!("../assets/icon-shield-96.alpha");
    /// 변 크기(px).
    pub const SHIELD_SIZE: u32 = 96;
    /// 프로필(사람 실루엣 — 머리 원 + 어깨) 96×96 알파 마스크(M3-17 화면 진입).
    pub const PERSON_ALPHA: &[u8] = include_bytes!("../assets/icon-person-96.alpha");
    /// 변 크기(px).
    pub const PERSON_SIZE: u32 = 96;
}

pub mod about;
pub mod addr_prompt;
pub mod alert;
pub mod avatar;
pub mod chat_view;
pub mod controls;
pub mod draw;
pub mod edit;
pub mod event;
pub mod gallery;
pub mod geom;
pub mod hangul;
pub mod offer_prompt;
pub mod peer_info;
pub mod peer_list;
pub mod profile;
pub mod prompt;
pub mod quarantine_view;
pub mod raster;
pub mod settings;
pub mod theme;
pub mod typeahead;
pub mod widget;

pub use about::{AboutInfo, AboutWidget};
pub use addr_prompt::AddrPromptWidget;
pub use alert::AlertWidget;
pub use chat_view::{
    fmt_hm, update_xfer_ack, update_xfer_in, ChatBody, ChatLine, ChatViewWidget, WallTime,
    XferLine, XferLineState,
};
pub use controls::{
    BorderSpec, Button, ButtonMode, Checkbox, Choose, ChoosePicker, Combo, ComboControl, ComboItem,
    Control, ControlBase, FlatRow, GridColumn, ImageFit, LabelSide, PopupHit, RadioGroup,
    RadioOption, ScrollBars, TextBox, TreeControl, TreeGrid, TreeModel, TreeNode, TreeView,
};
pub use controls::{
    FiredBy, HAlign, MenuBar, MenuDef, MenuEntry, TimeoutButton, ToolIcon, ToolItem, Toolbar,
    VAlign,
};
pub use draw::{DrawCtx, FontSlot};
pub use edit::{EditKey, EditState};
pub use event::{InputEvent, Key, WheelAccum, WHEEL_DELTA};
pub use gallery::GalleryWidget;
pub use geom::{Point, Rect, Size};
pub use offer_prompt::{OfferChoice, OfferInfo, OfferPromptWidget};
pub use peer_info::{PeerInfo, PeerInfoWidget};
pub use peer_list::{
    badge, Activated, GroupAction, GroupRow, HudPos, LinkState, PeerListWidget, PeerRow,
    XferProgress, ROW_H,
};
pub use profile::{ProfileValues, ProfileWidget};
pub use prompt::TextPromptWidget;
pub use quarantine_view::{QAction, QRow, QuarantineWidget};
pub use raster::{FontSet, RasterCtx};
pub use settings::{registry, Entry, SettingKind, SettingsState, SettingsWidget};
pub use theme::{Color, FontPrefs, IconImage, SlotFont, Theme};
pub use typeahead::{Query, TypeAhead, TYPEAHEAD_TIMEOUT_MS};
pub use widget::{Invalidations, Widget};
