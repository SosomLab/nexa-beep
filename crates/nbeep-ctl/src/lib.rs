//! # nbeep-ctl — 커스텀 컨트롤 라이브러리 (08-14 분리 · DR-6)
//!
//! 앱(nexa-beep)에서 **재사용 가능한 UI 기반**만 모았다: 기하·이벤트·그리기 어휘·
//! 위젯 계약·컨트롤 17종·이니셜 아바타·소프트웨어 래스터라이저. **앱 도메인
//! (nbeep-core) 의존 0**이 이 크레이트의 불변식이다 — 문자열(i18n)·크기 배율 같은
//! 앱 정책은 전부 주입으로 받는다([`controls::set_ctl_labels`] ·
//! [`controls::set_control_size_mult`]).
//!
//! ## 추상화 구조 — Rust식 Interface · Abstract · 상속
//!
//! | 고전 OOP | 이 크레이트 | 실체 |
//! | --- | --- | --- |
//! | **Interface** | [`widget::Widget`] · [`draw::DrawCtx`] | 계약만 — 렌더 백엔드·컨테이너가 구현을 갈아끼운다(테스트 = [`controls::ProbeCtx`] 같은 스텁) |
//! | **Abstract class** | [`controls::Control`] + [`controls::ControlBase`] | 공통 상태(bounds·focus·scale·help)와 **기본 메서드**(`s()`·`set_focused`·포커스 링·도움말 배지)를 제공 — 구체 컨트롤은 `base()/base_mut()` 두 개만 구현하면 나머지를 물려받는다 |
//! | **상속** | **조합 + 위임**(Rust에 구현 상속 없음) | 컨트롤이 [`controls::ControlBase`]를 필드로 품고, 복합 컨트롤(ColorPicker의 TextBox 등)은 하위 컨트롤에 위임한다 |
//!
//! 전파 규약: 이벤트는 [`widget::Widget::on_event`] 단일 진입, 무효화는
//! [`widget::Invalidations`] 수집(그리는 쪽이 아니라 **바뀐 쪽이 신고**),
//! 페인트는 불변(`&self`) — 상태 변경 없는 렌더가 계약이다.
//!
//! ## 경계 규칙 (docs/13 §2-4 규칙 7 — 이음새)
//!
//! - 이 크레이트는 **호스트(창·OS·클립보드·i18n)를 모른다** — 컨트롤은 요청만
//!   남기고([`controls::EditCtxAction`] 등) 실행은 호스트 몫.
//! - 외부 크레이트 타입이 공개 시그니처에 나오지 않는다(렌더 기반 `nbeep-gfx`는
//!   [`raster`] 어댑터 안에만).

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod avatar;
pub mod controls;
pub mod draw;
pub mod edit;
pub mod event;
pub mod geom;
pub mod raster;
pub mod theme;
pub mod widget;

pub use controls::{
    BorderSpec, Button, ButtonMode, Checkbox, Choose, ChoosePicker, ColorPicker, Combo,
    ComboControl, ComboItem, Control, ControlBase, CtlMsg, EditCtxAction, FlatRow, GridColumn,
    ImageFit, LabelSide, PopupHit, RadioGroup, RadioOption, ScrollBars, TextBox, TreeControl,
    TreeGrid, TreeModel, TreeNode, TreeView,
};
pub use controls::{
    FiredBy, HAlign, MenuBar, MenuDef, MenuEntry, TimeoutButton, ToolIcon, ToolItem, Toolbar,
    VAlign,
};
pub use draw::{DrawCtx, FontSlot};
pub use edit::{EditKey, EditState};
pub use event::{InputEvent, Key, WheelAccum, WHEEL_DELTA};
pub use geom::{Point, Rect, Size};
pub use raster::{FontSet, RasterCtx};
pub use theme::{Color, FontPrefs, IconImage, SlotFont, Theme};
pub use widget::{Invalidations, Widget};
