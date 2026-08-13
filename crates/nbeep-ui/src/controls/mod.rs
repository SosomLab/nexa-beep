//! 커스텀 컨트롤 툴킷 — **공통 베이스 + 개별 컨트롤**(DR-6 · 사용자 요청 08-08).
//!
//! OS 위젯을 쓰지 않고([DR-6]) 전 플랫폼 동일 UI를 그린다. 모든 컨트롤이 [`ControlBase`]를
//! **컴포지션**하고 [`Control`] 트레이트의 **기본 메서드**로 공통 기능을 상속한다(Rust엔 상속이
//! 없으므로 lib.rs가 말하는 `WidgetBase` 방식 — 베이스 필드 + 트레이트 기본 구현 전파).
//!
//! ## 모든 컨트롤이 공통으로 얻는 것
//!
//! - **포커스 링** — 선택(키보드 포커스) 시 **밝은 반투명 테두리**([`Theme::focus_ring`])로 식별.
//! - **창 활성 상태**(`active`) — 창이 포커스를 잃으면 강조색을 무채로 낮춘다(macOS 관례).
//! - **도움말** — `help` 상세 설명 + `show_help`(Y) → 컨트롤 옆 **"?" 배지**, 누르면 **툴팁**.
//!
//! 새 컨트롤 = [`ControlBase`] 필드 1개 + [`Control`] 구현 2줄(`base`/`base_mut`)이면 이 전부를
//! 물려받는다(확장 용이 — 사용자 요청).

pub mod button;
pub mod checkbox;
pub mod colorpick;
pub mod combo;
pub mod ctxmenu;
pub mod posgrid;
pub mod pulldown;
pub mod radio;
pub mod scroll;
pub mod switch;
pub mod textbox;
pub mod timeout_button;
pub mod toolbar;
pub mod tree;

pub use button::{Button, ButtonMode, ImageFit};
pub use checkbox::Checkbox;
pub use colorpick::ColorPicker;
pub use combo::{Choose, ChoosePicker, Combo, ComboControl, ComboItem, PopupHit};
pub use ctxmenu::{ContextMenu, CtxItem};
pub use posgrid::PositionPicker;
pub use pulldown::{MenuBar, MenuDef, MenuEntry};
pub use radio::{RadioGroup, RadioOption};
pub use scroll::ScrollBars;
pub use switch::Switch;
pub use textbox::{EditCtxAction, TextBox};
pub use timeout_button::{FiredBy, TimeoutButton};
pub use toolbar::{ToolIcon, ToolItem, Toolbar, DEFAULT_ICON};
pub use tree::{FlatRow, GridColumn, TreeControl, TreeGrid, TreeModel, TreeNode, TreeView};

use crate::draw::{DrawCtx, FontSlot};
use crate::geom::{Point, Rect};

/// 컨트롤 크기 배율(체크·스위치·옵션박스 글리프 — 08-11 사용자 요청) —
/// f32 비트를 원자적으로 보관(스크롤바 숨김 지연과 같은 전역 설정 문법).
/// 기본 1.0(= 현재 크기) · 설정 `ui.control_size` s/m/l/xl → 0.8/1.0/1.3/1.6.
static CTL_SIZE_MULT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000); // 1.0f32 비트

/// 배율 지정(호스트 — 설정 적용·부팅 반영).
pub fn set_control_size_mult(m: f32) {
    CTL_SIZE_MULT.store(m.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// 현재 배율.
#[must_use]
pub fn control_size_mult() -> f32 {
    f32::from_bits(CTL_SIZE_MULT.load(std::sync::atomic::Ordering::Relaxed))
}

/// 설정 코드(s/m/l/xl) → 배율. 미지 값은 기본 1.0(관용).
#[must_use]
pub fn control_size_mult_from_code(code: &str) -> f32 {
    match code {
        "s" => 0.8,
        "l" => 1.3,
        "xl" => 1.6,
        _ => 1.0,
    }
}

/// 논리 px에 컨트롤 크기 배율 적용(글리프 치수 전용 — 행 높이·여백은 그대로).
#[must_use]
pub fn ctl_size(logical: i32) -> i32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let v = (logical as f32 * control_size_mult()).round() as i32;
    v.max(1)
}
use crate::theme::{Color, Theme};

/// 선행 아이콘 변 크기(논리 px) — **콤보/Choose/트리/버튼 공용 단일 원천**(크기 드리프트 방지).
pub const LEADING_ICON: i32 = 13;

/// 외곽 테두리 설정 — 두께(논리 px · 소수 가능 예: 0.5)·색·투명도(0..=1). 두께 ≤0 = 없음(기본).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderSpec {
    /// 두께(논리 px). 0 이하면 그리지 않는다. 0.5 같은 소수도 허용(SDF AA 얇은 선).
    pub width: f32,
    /// 색.
    pub color: Color,
    /// 불투명도(0.0~1.0).
    pub alpha: f32,
}

impl Default for BorderSpec {
    fn default() -> Self {
        Self {
            width: 0.0,
            color: Color::from_rgb(0x36, 0x3C, 0x46),
            alpha: 1.0,
        }
    }
}

impl BorderSpec {
    /// (두께, 색, 투명도)로 만든다.
    #[must_use]
    pub fn new(width: f32, color: Color, alpha: f32) -> Self {
        Self {
            width,
            color,
            alpha: alpha.clamp(0.0, 1.0),
        }
    }
}

/// 가로 정렬 — **전 컨트롤 공통**([`ControlBase`] 상속 · 콘텐츠 배치 기준).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HAlign {
    /// 왼쪽.
    Left,
    /// 가운데(기본 — 현행 시각 유지).
    #[default]
    Center,
    /// 오른쪽.
    Right,
}

/// 세로 정렬 — **전 컨트롤 공통**([`ControlBase`] 상속).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VAlign {
    /// 위.
    Top,
    /// 세로 중앙(기본).
    #[default]
    Center,
    /// 아래.
    Bottom,
}

/// 라벨 위치 옵션 — 컨트롤 본체 기준(사용자 요청: 체크만/좌/우).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LabelSide {
    /// 라벨 없음(컨트롤만).
    None,
    /// 왼쪽.
    Left,
    /// 오른쪽(기본 — 체크박스 관례).
    #[default]
    Right,
}

/// 모든 커스텀 컨트롤이 컴포지션하는 공통 상태.
#[derive(Clone, Debug)]
pub struct ControlBase {
    /// 컨트롤 경계(창 클라이언트 좌표).
    pub bounds: Rect,
    /// 배율(고DPI). 논리 → 물리 변환에 곱한다.
    pub scale: f32,
    /// 키보드 포커스(→ 포커스 링).
    pub focused: bool,
    /// 창 활성(→ 강조색 강도 · macOS 관례).
    pub active: bool,
    /// 상세 설명(도움말 툴팁 내용).
    pub help: Option<String>,
    /// 도움말 기능 사용 여부(Y = "?" 배지 표시).
    pub show_help: bool,
    /// 툴팁이 현재 열려 있는가(런타임).
    pub help_open: bool,
    /// 콘텐츠 가로 정렬(전 컨트롤 상속 — 각 컨트롤이 배치에 반영).
    pub halign: HAlign,
    /// 콘텐츠 세로 정렬(전 컨트롤 상속).
    pub valign: VAlign,
}

impl Default for ControlBase {
    fn default() -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            focused: false,
            active: true,
            help: None,
            show_help: false,
            help_open: false,
            halign: HAlign::default(),
            valign: VAlign::default(),
        }
    }
}

/// 커스텀 컨트롤 공통 트레이트 — 기본 메서드로 포커스/활성/도움말을 **상속**시킨다.
///
/// 구현체는 [`Control::base`]/[`Control::base_mut`] 둘만 제공하면 나머지를 전부 물려받는다.
pub trait Control: crate::widget::Widget {
    /// 공통 베이스 참조(구현 필수).
    fn base(&self) -> &ControlBase;
    /// 공통 베이스 가변 참조(구현 필수).
    fn base_mut(&mut self) -> &mut ControlBase;

    /// 논리 → 물리 px(배율 적용).
    fn s(&self, logical: i32) -> i32 {
        (logical as f32 * self.base().scale).round() as i32
    }

    /// 키보드 포커스 지정.
    ///
    /// ⚠️ 도움말 툴팁은 여기서 닫지 않는다 — 바깥 클릭 닫기는 [`Control::handle_help_click`]이
    /// 담당한다. 호스트가 클릭마다 포커스를 재계산하는 구조에서 "?" 배지는 컨트롤 bounds
    /// 밖이라, 여기서 닫으면 "닫기→토글" 순서가 되어 재클릭 닫기가 영원히 무효화된다(08-09).
    fn set_focused(&mut self, on: bool) {
        self.base_mut().focused = on;
    }
    /// 키보드 포커스 여부.
    fn is_focused(&self) -> bool {
        self.base().focused
    }
    /// 창 활성 지정(비활성 시 강조 무채화).
    fn set_active(&mut self, on: bool) {
        self.base_mut().active = on;
    }
    /// 창 활성 여부.
    fn is_active(&self) -> bool {
        self.base().active
    }
    /// 배율 지정.
    fn set_scale(&mut self, scale: f32) {
        self.base_mut().scale = scale.max(0.5);
    }

    /// 가로 정렬 지정(전 컨트롤 공통).
    fn set_halign(&mut self, a: HAlign) {
        self.base_mut().halign = a;
    }
    /// 가로 정렬.
    fn halign(&self) -> HAlign {
        self.base().halign
    }
    /// 세로 정렬 지정(전 컨트롤 공통).
    fn set_valign(&mut self, a: VAlign) {
        self.base_mut().valign = a;
    }
    /// 세로 정렬.
    fn valign(&self) -> VAlign {
        self.base().valign
    }

    /// 세로 정렬 y 계산 — `area` 안에 높이 `item_h` 항목을 [`VAlign`]대로 놓는다(여백 `pad`).
    fn align_y(&self, area: Rect, item_h: i32, pad: i32) -> i32 {
        match self.valign() {
            VAlign::Top => area.y + pad,
            VAlign::Center => area.y + (area.h - item_h) / 2,
            VAlign::Bottom => area.bottom() - pad - item_h,
        }
    }

    /// 도움말 내용 지정.
    fn set_help(&mut self, help: impl Into<String>) {
        self.base_mut().help = Some(help.into());
    }
    /// 도움말 기능 사용 여부(Y/N).
    fn set_show_help(&mut self, on: bool) {
        self.base_mut().show_help = on;
        if !on {
            self.base_mut().help_open = false;
        }
    }
    /// "?" 배지가 지금 그려지는가(도움말 사용 + 내용 있음).
    fn has_help_badge(&self) -> bool {
        self.base().show_help && self.base().help.is_some()
    }
    /// 툴팁 열림/닫힘 토글.
    fn toggle_help(&mut self) {
        let b = self.base_mut();
        b.help_open = !b.help_open;
    }

    /// 강조색(창 활성 = accent · 비활성 = 무채) — 컨트롤 공통 색 규칙.
    fn accent_now(&self, theme: &Theme) -> Color {
        if self.is_active() {
            theme.accent
        } else {
            theme.text_dim
        }
    }

    /// **포커스 링** — `around`(컨트롤 본체) 바깥에 밝은 반투명 테두리를 그린다(선택 식별).
    fn draw_focus_ring(&self, ctx: &mut dyn DrawCtx, theme: &Theme, around: Rect) {
        if !self.is_focused() {
            return;
        }
        let pad = self.s(2).max(2);
        let ring = Rect::new(
            around.x - pad,
            around.y - pad,
            around.w + pad * 2,
            around.h + pad * 2,
        );
        let radius = self.s(6);
        // 2px 테두리 · 50% 반투명(사용자 확정) — 선택 식별용 밝은 헤일로.
        ctx.stroke_round_rect_alpha(ring, radius, theme.focus_ring, self.s(2).max(2) as f32, 0.5);
    }

    /// "?" 배지 rect — `after`(컨트롤/라벨) 오른쪽에 붙인다.
    fn help_badge_rect(&self, after: Rect) -> Rect {
        let d = self.s(18);
        let gap = self.s(8);
        let cy = after.y + (after.h - d) / 2;
        Rect::new(after.right() + gap, cy, d, d)
    }

    /// "?" 배지를 그린다(도움말 사용 시). 툴팁이 열려 있으면 **눌린 상태로 반전** —
    /// 체크박스와 같은 밝은 파랑 채움 + 흰 "?"(직관적 식별 · 사용자 확정 08-09).
    fn draw_help_badge(&self, ctx: &mut dyn DrawCtx, theme: &Theme, badge: Rect) {
        if !self.has_help_badge() {
            return;
        }
        let open = self.base().help_open;
        if open {
            ctx.fill_ellipse(badge, self.accent_now(theme));
        } else {
            ctx.fill_ellipse(badge, theme.field_bg);
            ctx.stroke_round_rect(badge, badge.w / 2, theme.border, 1.0);
        }
        ctx.select_font(FontSlot::Status, false);
        let qw = ctx.text_width("?");
        let th = ctx.text_height();
        let fg = if open {
            Color::from_rgb(255, 255, 255)
        } else {
            theme.text_dim
        };
        // "?"를 배지 정중앙에(가로·세로) — 상태 글꼴 높이 기준으로 세로 중앙 정렬.
        ctx.text(
            badge.x + (badge.w - qw) / 2,
            badge.y + (badge.h - th) / 2,
            badge,
            "?",
            fg,
        );
    }

    /// "?" 클릭 처리 — 배지를 눌렀으면 툴팁 토글 후 `true`(소비). 배지 밖을 누르면 **열린 툴팁을
    /// 닫는다**(다른 영역·다른 컨트롤 클릭 시 자동 숨김) — 이벤트는 계속 흐르도록 `false`.
    fn handle_help_click(&mut self, x: i32, y: i32, badge: Rect) -> bool {
        if self.has_help_badge() && badge.contains(Point { x, y }) {
            self.toggle_help();
            true
        } else {
            if self.base().help_open {
                self.base_mut().help_open = false;
            }
            false
        }
    }

    /// 툴팁(말풍선)을 그린다 — **컨트롤 paint 맨 끝에** 호출(다른 내용 위에 겹침). `anchor` = "?" 배지.
    fn draw_help_tip(&self, ctx: &mut dyn DrawCtx, theme: &Theme, anchor: Rect) {
        if !(self.base().help_open && self.has_help_badge()) {
            return;
        }
        let Some(text) = self.base().help.as_deref() else {
            return;
        };
        let pad = self.s(10);
        let max_w = self.s(280);
        ctx.select_font(FontSlot::Status, false);
        let lines = wrap_text(ctx, text, max_w);
        let line_gap = self.s(18); // 줄 간격(줄 top 사이)
        let glyph_h = self.s(14); // 글자 시각 높이(상·하 여백 균등화 기준)
                                  // 텍스트 블록 높이 = (줄-1)*간격 + 마지막 줄 글자 높이 → 상·하 pad가 동일해진다.
        let block_h = (lines.len() as i32 - 1).max(0) * line_gap + glyph_h;
        let tip_h = block_h + pad * 2;
        let tip_w = max_w + pad * 2;
        let tip = Rect::new(anchor.x, anchor.bottom() + self.s(4), tip_w, tip_h);
        ctx.fill_round_rect(tip, self.s(8), theme.chrome_bg);
        ctx.stroke_round_rect(tip, self.s(8), theme.border, 1.0);
        ctx.select_font(FontSlot::Status, false);
        let mut ty = tip.y + pad;
        for line in &lines {
            ctx.text(tip.x + pad, ty, tip, line, theme.text);
            ty += line_gap;
        }
    }
}

/// 그리디 줄바꿈 — `max_w`(물리 px) 넘지 않게 공백 단위로 접는다(툴팁·설명 공용).
pub fn wrap_text(ctx: &mut dyn DrawCtx, text: &str, max_w: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let trial = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if ctx.text_width(&trial) > max_w && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = trial;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// 체크박스 글리프 — `box_r`에 그린다. `checked`/`active`(창 활성)로 모양이 갈린다.
pub fn draw_checkbox_glyph(
    ctx: &mut dyn DrawCtx,
    theme: &Theme,
    box_r: Rect,
    checked: bool,
    active: bool,
) {
    let radius = (box_r.w / 4).max(2);
    if !checked {
        ctx.fill_round_rect(box_r, radius, theme.field_bg);
        ctx.stroke_round_rect(box_r, radius, theme.border, 1.0);
        return;
    }
    // 체크: 창 활성 = accent 채움 + 흰 체크, 비활성 = 무채 체크(채움 없음 — macOS 관례).
    let (fill, tick) = if active {
        (theme.accent, Color::from_rgb(255, 255, 255))
    } else {
        (theme.field_bg, theme.text)
    };
    ctx.fill_round_rect(box_r, radius, fill);
    if !active {
        ctx.stroke_round_rect(box_r, radius, theme.border, 1.0);
    }
    let x = box_r.x;
    let y = box_r.y;
    let (w, h) = (box_r.w, box_r.h);
    let pts = [
        (x + w * 26 / 100, y + h * 52 / 100),
        (x + w * 43 / 100, y + h * 70 / 100),
        (x + w * 76 / 100, y + h * 30 / 100),
    ];
    ctx.polyline(&pts, tick, (box_r.w as f32 / 9.0).max(1.5));
}

/// 라디오 글리프 — 원 + (선택 시) 안쪽 점.
pub fn draw_radio_glyph(
    ctx: &mut dyn DrawCtx,
    theme: &Theme,
    r: Rect,
    selected: bool,
    active: bool,
) {
    if selected {
        let fill = if active { theme.accent } else { theme.field_bg };
        ctx.fill_ellipse(r, fill);
        if !active {
            ctx.stroke_round_rect(r, r.w / 2, theme.border, 1.0);
        }
        // 안쪽 점.
        let inset = r.w * 30 / 100;
        let dot = Rect::new(r.x + inset, r.y + inset, r.w - inset * 2, r.h - inset * 2);
        let dc = if active {
            Color::from_rgb(255, 255, 255)
        } else {
            theme.text
        };
        ctx.fill_ellipse(dot, dc);
    } else {
        ctx.fill_ellipse(r, theme.field_bg);
        ctx.stroke_round_rect(r, r.w / 2, theme.border, 1.0);
    }
}

/// 위/아래 이중 셰브론(⇕) — 콤보박스 오른쪽 표식.
pub fn draw_updown_chevrons(ctx: &mut dyn DrawCtx, theme: &Theme, area: Rect, color: Color) {
    let cx = area.x + area.w / 2;
    // 좌우로 조금 짧은 형태(사용자 확정 · w/5).
    let half = (area.w / 5).max(2);
    // 두 셰브론을 2px 더 가깝게(각 방향 1px씩 · 사용자 확정).
    let gap = (area.h / 8 - 1).max(1);
    let midy = area.y + area.h / 2;
    let up_tip = midy - gap - half;
    let dn_tip = midy + gap + half;
    let w = (area.w as f32 / 10.0).max(1.5);
    let _ = theme;
    // ▲
    ctx.polyline(
        &[
            (cx - half, up_tip + half),
            (cx, up_tip),
            (cx + half, up_tip + half),
        ],
        color,
        w,
    );
    // ▼
    ctx.polyline(
        &[
            (cx - half, dn_tip - half),
            (cx, dn_tip),
            (cx + half, dn_tip - half),
        ],
        color,
        w,
    );
}

/// 이미지를 `area` 안에 **비율 유지로 맞춘**(contain) 목적 rect — 여백은 남기고 잘리지 않는다.
/// 큰 이미지는 축소, 작은 이미지는 확대되어 중앙 정렬된다.
#[must_use]
pub fn image_fit_contain(area: Rect, iw: i32, ih: i32) -> Rect {
    if iw <= 0 || ih <= 0 {
        return area;
    }
    let s = (area.w as f32 / iw as f32).min(area.h as f32 / ih as f32);
    let (w, h) = ((iw as f32 * s) as i32, (ih as f32 * s) as i32);
    Rect::new(
        area.x + (area.w - w) / 2,
        area.y + (area.h - h) / 2,
        w.max(1),
        h.max(1),
    )
}

/// 이미지를 `area`를 **가득 채우도록**(cover) 비율 유지 확대한 목적 rect — 넘치는 부분은
/// 호출자가 `area`로 클립한다(이미지 버튼: 버튼 크기 유지 + 이미지 비율 + 잘림).
#[must_use]
pub fn image_fit_cover(area: Rect, iw: i32, ih: i32) -> Rect {
    if iw <= 0 || ih <= 0 {
        return area;
    }
    let s = (area.w as f32 / iw as f32).max(area.h as f32 / ih as f32);
    let (w, h) = ((iw as f32 * s) as i32, (ih as f32 * s) as i32);
    Rect::new(
        area.x + (area.w - w) / 2,
        area.y + (area.h - h) / 2,
        w.max(1),
        h.max(1),
    )
}

/// 체크 표식(✓) — `area` 안에 그린다(드롭다운 선택 행 등).
pub fn draw_check_mark(ctx: &mut dyn DrawCtx, area: Rect, color: Color) {
    let x = area.x;
    let y = area.y;
    let (w, h) = (area.w, area.h);
    let pts = [
        (x + w * 20 / 100, y + h * 52 / 100),
        (x + w * 42 / 100, y + h * 72 / 100),
        (x + w * 80 / 100, y + h * 28 / 100),
    ];
    ctx.polyline(&pts, color, (area.w as f32 / 9.0).max(1.5));
}

/// 아래 셰브론(∨) — 드롭다운/트리 접힘 표식. 좌우로 조금 짧은 형태(사용자 확정 · w/5).
pub fn draw_chevron_down(ctx: &mut dyn DrawCtx, area: Rect, color: Color) {
    let cx = area.x + area.w / 2;
    let cy = area.y + area.h / 2;
    let half = (area.w / 5).max(2);
    let w = (area.w as f32 / 10.0).max(1.5);
    ctx.polyline(
        &[
            (cx - half, cy - half / 2),
            (cx, cy + half / 2),
            (cx + half, cy - half / 2),
        ],
        color,
        w,
    );
}

/// 오른쪽 셰브론(›) — 트리 접힘 표식. 아래 셰브론(∨)과 같은 세트 크기(h/5 · 사용자 확정).
pub fn draw_chevron_right(ctx: &mut dyn DrawCtx, area: Rect, color: Color) {
    let cx = area.x + area.w / 2;
    let cy = area.y + area.h / 2;
    let half = (area.h / 5).max(2);
    let w = (area.w as f32 / 10.0).max(1.5);
    ctx.polyline(
        &[
            (cx - half / 2, cy - half),
            (cx + half / 2, cy),
            (cx - half / 2, cy + half),
        ],
        color,
        w,
    );
}

/// 테스트용 측정 전용 [`DrawCtx`] — 그리기는 no-op, `text_width` = 문자수 × 7(결정적).
#[cfg(test)]
pub(crate) struct ProbeCtx;

#[cfg(test)]
impl DrawCtx for ProbeCtx {
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn text_opaque(&mut self, _x: i32, _y: i32, _clip: Rect, _t: &str, _f: Color, _b: Color) {}
    fn text(&mut self, _x: i32, _y: i32, _clip: Rect, _t: &str, _f: Color) {}
    fn text_width(&mut self, text: &str) -> i32 {
        text.chars().count() as i32 * 7
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_respects_max_width() {
        let mut ctx = ProbeCtx;
        let lines = wrap_text(&mut ctx, "aa bb cc dd ee ff", 21);
        assert!(lines.len() > 1, "여러 줄로 접힘: {lines:?}");
        for l in &lines {
            // 단어 하나로는 넘칠 수 있으나, 공백을 포함한 줄은 상한 이내.
            assert!(ctx.text_width(l) <= 21 || !l.contains(' '));
        }
    }

    #[test]
    fn label_side_default_is_right() {
        assert_eq!(LabelSide::default(), LabelSide::Right);
    }
}
