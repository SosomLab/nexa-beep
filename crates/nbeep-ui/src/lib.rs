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

    /// 목록 정렬 아이콘 4종(08-15 — [`crate::controls::IconDropdown`] 첫 사용처).
    /// 자체 작도(Lucide 스타일 참고 · 원본 `assets/icons-src/sort-*.svg` · 20px 판독 실측).
    pub mod sort {
        /// 기본(최근 대화 우선) — 말풍선 + 점 3개.
        pub const RECENT_ALPHA: &[u8] = include_bytes!("../assets/icon-sort-recent-96.alpha");
        /// 이름순 — A/Z + 아래 화살표.
        pub const NAME_ALPHA: &[u8] = include_bytes!("../assets/icon-sort-name-96.alpha");
        /// 최근 접속순(온라인 여부 무관) — 시계 + 아래 화살표.
        pub const SEEN_ALPHA: &[u8] = include_bytes!("../assets/icon-sort-seen-96.alpha");
        /// 온라인 우선(그 안은 최근 접속) — 상승 신호 막대.
        pub const ONLINE_ALPHA: &[u8] = include_bytes!("../assets/icon-sort-online-96.alpha");
        /// 변 크기(px) — 네 자산 공통.
        pub const SIZE: u32 = 96;
    }

    /// 연결 상태 아이콘 4종 — **큰 자리 전용**(≥20px · [docs/14 §12-7]).
    ///
    /// 원본 = **Lucide**(ISC · 사용자 확정 08-14 — 한 세트로 통일) ·
    /// 출처 SVG는 `assets/icons-src/`, 굽는 절차는 `tools/mkicons.sh`([docs/18]).
    /// 상태 대응은 `Idle=PLUG` · `Connecting=PLUG_ZAP` · `Active=CABLE` · `Lost=UNPLUG`.
    ///
    /// ⚠️ 목록 행의 11px 배지에는 쓰지 않는다 — 그 자리는 실루엣 파냄(M3-19)이다.
    pub mod link {
        /// 플러그 하나(안 꽂힘) = `Idle`. 96×96 알파 마스크.
        pub const PLUG_ALPHA: &[u8] = include_bytes!("../assets/icon-plug-96.alpha");
        /// 플러그 + 번개 = `Connecting`. 96×96 알파 마스크.
        pub const PLUG_ZAP_ALPHA: &[u8] = include_bytes!("../assets/icon-plug-zap-96.alpha");
        /// 양끝 커넥터가 이어진 케이블 = `Active`. 96×96 알파 마스크.
        pub const CABLE_ALPHA: &[u8] = include_bytes!("../assets/icon-cable-96.alpha");
        /// 뽑히는 두 플러그 = `Lost`. 96×96 알파 마스크.
        pub const UNPLUG_ALPHA: &[u8] = include_bytes!("../assets/icon-unplug-96.alpha");
        /// 변 크기(px) — 네 자산 공통.
        pub const SIZE: u32 = 96;
    }
}

// UI 기반·컨트롤은 별도 라이브러리로 분리(08-14 — `nbeep-ctl` · DR-6/DR-21).
// 기존 경로(`nbeep_ui::controls::…` 등)는 모듈 재수출로 그대로 유지된다(호환 불변).
pub use nbeep_ctl::{avatar, controls, draw, edit, event, geom, raster, theme, widget};

pub mod about;
pub mod addr_prompt;
pub mod alert;
pub mod avatar_assets;
pub mod chat_view;
pub mod gallery;
pub mod hangul;
pub mod offer_prompt;
pub mod peer_info;
pub mod peer_list;
pub mod profile;
pub mod prompt;
pub mod quarantine_view;
pub mod settings;
pub mod typeahead;

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
    FiredBy, HAlign, IconDropItem, IconDropdown, MenuBar, MenuDef, MenuEntry, TimeoutButton,
    ToolIcon, ToolItem, Toolbar, VAlign,
};
pub use draw::{DrawCtx, FontSlot};
pub use edit::{EditKey, EditState};
pub use event::{InputEvent, Key, WheelAccum, WHEEL_DELTA};
pub use gallery::GalleryWidget;
pub use geom::{Point, Rect, Size};
pub use offer_prompt::{OfferChoice, OfferInfo, OfferPromptWidget};
pub use peer_info::{PeerInfo, PeerInfoWidget};
pub use peer_list::{
    badge, draw_link_badge, link_color, Activated, GroupAction, GroupRow, HudPos, LinkState,
    PeerListWidget, PeerRow, RefreshScroll, XferProgress, ROW_H,
};
pub use profile::{ProfileValues, ProfileWidget};
pub use prompt::TextPromptWidget;
pub use quarantine_view::{QAction, QRow, QuarantineWidget};
pub use raster::{FontSet, RasterCtx};
pub use settings::{registry, Entry, SettingKind, SettingsState, SettingsWidget};
pub use theme::{Color, FontPrefs, IconImage, SlotFont, Theme};
pub use typeahead::{Query, TypeAhead, TYPEAHEAD_TIMEOUT_MS};
pub use widget::{Invalidations, Widget};
