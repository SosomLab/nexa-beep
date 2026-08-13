//! 콤보박스 — **일반 콤보(∨)** 와 **확장 콤보(⇕)** 의 상속 계층(사용자 요청 08-08).
//!
//! ## 상속 설계 (ControlBase → Combo → Choose)
//!
//! Rust엔 클래스 상속이 없어 **트레이트 계층 + 공유 상태 컴포지션**으로 같은 개념을 만든다:
//!
//! - [`super::Control`] — 모든 컨트롤의 **루트 인터페이스**(포커스 링·활성·도움말).
//! - [`ComboControl`] — 그 위에 얹는 **콤보 계층 인터페이스**. [`ComboCore`](공유 상태)를 여는
//!   접근자 둘만 구현하면 열기/닫기·후버 이동·선택·드롭다운 렌더·히트테스트를 **기본 메서드로 상속**.
//! - [`Combo`] — 일반 콤보(**∨** · 목록 택일). `ComboControl`의 기본 동작 그대로.
//! - [`Choose`] — 확장 콤보(**⇕** · 값 직접 편집 + 구분자 아래 **"Choose…"** 로 커스텀 값).
//!   `Combo`의 모델을 물려받아(같은 `ComboCore`) 편집 필드와 Choose 항목만 **덧붙인다/재정의**.
//!
//! 시각: ⇕ = "값을 사용자 정의할 수 있다"(확장) · ∨ = "목록에서 고른다"(일반) — 사용자 정의.

use super::{
    draw_check_mark, draw_chevron_down, draw_updown_chevrons, image_fit_contain, Control,
    ControlBase, ScrollBars,
};
use crate::draw::{DrawCtx, FontSlot};
use crate::edit::EditState;
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

/// **찾기 창 어댑터**(인터페이스 · Adapter 패턴) — [`Choose`]의 "Choose…"가 열 화면을 갈아끼운다.
///
/// 어떤 화면이든 이 트레이트만 구현하면 Choose에 꽂을 수 있다([`Choose::set_picker`]). Choose는
/// 후보를 [`ChoosePicker::items`]로 받아 오버레이 목록으로 띄우고, 고른 항목의 라벨을 값으로 반영한다.
/// 샘플 구현(단일 파일 선택기)은 파일 접근이 가능한 상위 계층(bin)에 둔다 — UI 계층은 I/O를 모른다.
pub trait ChoosePicker: std::fmt::Debug {
    /// 찾기 창 제목.
    fn title(&self) -> String;
    /// 선택 후보(오버레이에 표시) — 아이콘 포함 가능.
    fn items(&self) -> Vec<ComboItem>;
}

/// 콤보 항목 — 값·라벨 + **선택적 선행 아이콘**. 아이콘은 **이미지**([`ComboItem::image`] · 투명
/// 배경 RGBA)가 우선이고, 없으면 글리프 문자([`ComboItem::icon`])로 폴백한다.
#[derive(Clone, Debug, Default)]
pub struct ComboItem {
    /// 값(안정 계약).
    pub value: String,
    /// 표시 라벨.
    pub label: String,
    /// 선행 글리프(이미지가 없을 때 폴백 · 기본 없음).
    pub icon: Option<String>,
    /// 선행 이미지 아이콘(투명 배경 RGBA · 우선). 공유를 위해 `Rc`.
    pub image: Option<Rc<IconImage>>,
}

impl ComboItem {
    /// (값, 라벨) — 아이콘 없음(텍스트 기본).
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            icon: None,
            image: None,
        }
    }
    /// 선행 글리프 지정(이미지 없을 때 표시).
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
    /// 선행 이미지 아이콘 지정(투명 배경 RGBA · 글리프보다 우선).
    #[must_use]
    pub fn with_image(mut self, image: Rc<IconImage>) -> Self {
        self.image = Some(image);
        self
    }
}

/// 선행 아이콘(이미지 우선 · 없으면 글리프)을 그리고 다음 x를 돌려준다. `cy` = 세로 중심.
/// `icon_sz` = 이미지 아이콘 정사각 변(글자 높이에 맞춤) · `s16` = 글리프 폴백 세로 기준.
#[allow(clippy::too_many_arguments)]
fn draw_leading(
    ctx: &mut dyn DrawCtx,
    it: &ComboItem,
    x: i32,
    cy: i32,
    icon_sz: i32,
    s16: i32,
    gap: i32,
    clip: Rect,
    glyph_color: crate::theme::Color,
) -> i32 {
    if let Some(img) = it.image.as_deref() {
        // icon_sz 정사각 박스에 비율 유지로 맞춘다(큰 이미지는 자동 축소).
        let boxr = Rect::new(x, cy - icon_sz / 2, icon_sz, icon_sz);
        let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
        ctx.image_scaled(fit, img, clip);
        x + icon_sz + gap
    } else if let Some(g) = it.icon.as_deref() {
        ctx.select_font(FontSlot::Base, false);
        ctx.text(x, cy - s16 / 2, clip, g, glyph_color);
        x + ctx.text_width(g) + gap
    } else {
        x
    }
}

/// 콤보 아이콘 변 크기 — 공용 단일 원천([`super::LEADING_ICON`] · 크기 드리프트 방지).
const ICON_SZ: i32 = super::LEADING_ICON;

/// 콤보 공유 상태(추상 콤보의 상태 계층). `Combo`·`Choose`가 함께 쓴다.
#[derive(Debug)]
pub struct ComboCore {
    items: Vec<ComboItem>,
    selected: usize,
    open: bool,
    /// 드롭다운 후버(키보드/마우스 하이라이트).
    hover: usize,
    changed: bool,
}

impl ComboCore {
    fn new(items: Vec<ComboItem>, selected: usize) -> Self {
        let selected = selected.min(items.len().saturating_sub(1));
        Self {
            items,
            selected,
            open: false,
            hover: selected,
            changed: false,
        }
    }
}

/// 드롭다운 히트 결과.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupHit {
    /// 일반 항목 i.
    Item(usize),
    /// 구분자 아래 확장 항목 j(예: "Choose…").
    Extra(usize),
}

// 레이아웃 상수(논리 px).
const ROW_H: i32 = 26;
// 셰브론 영역 폭 — 트리 셰브론과 같은 글리프 크기(구 22 → 16 · 사용자 확정).
const CHEV_W: i32 = 16;
const SEP_H: i32 = 9;
const POPUP_PAD: i32 = 4;
// 찾기 오버레이(Choose 어댑터).
const PICK_ROW_H: i32 = 24;
const PICK_TITLE_H: i32 = 26;

/// 콤보 계층 인터페이스 — [`Control`] 위에 콤보 공통 동작을 **기본 메서드로 상속**시킨다.
///
/// 구현체는 [`ComboControl::core`]/[`core_mut`](Self::core_mut)와, 재정의 훅
/// [`is_editable`](Self::is_editable)/[`extra_rows`](Self::extra_rows)만 제공하면 된다.
pub trait ComboControl: Control {
    /// 공유 콤보 상태(구현 필수).
    fn core(&self) -> &ComboCore;
    /// 공유 콤보 상태 가변(구현 필수).
    fn core_mut(&mut self) -> &mut ComboCore;

    /// ⇕(편집 가능=확장) vs ∨(일반) — `Choose`가 `true`로 재정의.
    fn is_editable(&self) -> bool {
        false
    }
    /// 구분자 아래 확장 항목(예: "Choose…") — 아이콘 옵션 포함. 기본 없음.
    fn extra_rows(&self) -> Vec<ComboItem> {
        Vec::new()
    }
    /// 목록에서 ✓를 그릴 항목(기본 = 선택 index). 커스텀 값 활성 시 `None`(Combo 재정의).
    fn checked_index(&self) -> Option<usize> {
        Some(self.core().selected)
    }
    /// 확장 항목 `j`에 ✓ 표시 여부(기본 없음) — 직접 입력 값 활성 시 true(Combo 재정의).
    fn extra_checked(&self, j: usize) -> bool {
        let _ = j;
        false
    }

    // ── 상태 조작(상속 기본 메서드) ──
    /// 드롭다운 열림 여부.
    fn is_open(&self) -> bool {
        self.core().open
    }
    /// 드롭다운 토글.
    fn toggle_open(&mut self, inv: &mut Invalidations) {
        let b = self.bounds();
        let c = self.core_mut();
        c.open = !c.open;
        c.hover = c.selected;
        inv.push(popup_or_box(b, self));
    }
    /// 드롭다운 닫기.
    fn close(&mut self, inv: &mut Invalidations) {
        if self.core().open {
            self.core_mut().open = false;
            inv.push(self.bounds());
        }
    }
    /// 후버를 delta만큼 이동(열려 있을 때).
    fn move_hover(&mut self, delta: i32) {
        let n = self.core().items.len() as i32;
        if n == 0 {
            return;
        }
        let h = (self.core().hover as i32 + delta).clamp(0, n - 1);
        self.core_mut().hover = h as usize;
    }
    /// 현재 선택 값.
    fn value(&self) -> String {
        self.core()
            .items
            .get(self.core().selected)
            .map(|it| it.value.clone())
            .unwrap_or_default()
    }
    /// 값 변경 1회성 보고.
    fn take_changed(&mut self) -> Option<String> {
        if std::mem::take(&mut self.core_mut().changed) {
            Some(self.value())
        } else {
            None
        }
    }
    /// 인덱스로 선택 확정(닫고 보고).
    fn choose_index(&mut self, i: usize, inv: &mut Invalidations) {
        if i < self.core().items.len() {
            let c = self.core_mut();
            let changed = c.selected != i;
            c.selected = i;
            c.open = false;
            c.changed |= changed;
            inv.push(self.bounds());
        }
    }

    // ── 기하 ──
    /// 셰브론 영역(오른쪽) — 우측 여백 2px(사용자 확정).
    fn chevron_rect(&self) -> Rect {
        let b = self.bounds();
        let w = self.s(CHEV_W);
        Rect::new(b.right() - w - self.s(2), b.y, w, b.h)
    }
    /// 드롭다운 팝업 rect(닫혀 있으면 빈 rect).
    fn popup_rect(&self) -> Rect {
        if !self.core().open {
            return Rect::new(0, 0, 0, 0);
        }
        let b = self.bounds();
        let rows = self.core().items.len() as i32;
        let extra = self.extra_rows().len() as i32;
        let mut h = self.s(POPUP_PAD) * 2 + rows * self.s(ROW_H);
        if extra > 0 {
            h += self.s(SEP_H) + extra * self.s(ROW_H);
        }
        Rect::new(b.x, b.bottom() + self.s(2), b.w, h)
    }
    /// (x,y) → 드롭다운 히트(항목/확장).
    fn popup_hit(&self, x: i32, y: i32) -> Option<PopupHit> {
        let pop = self.popup_rect();
        if !pop.contains(Point { x, y }) {
            return None;
        }
        let rh = self.s(ROW_H).max(1);
        let top = pop.y + self.s(POPUP_PAD);
        let items = self.core().items.len() as i32;
        let rel = y - top;
        if rel < 0 {
            return None;
        }
        let idx = rel / rh;
        if idx < items {
            return Some(PopupHit::Item(idx as usize));
        }
        // 구분자 아래 확장 항목.
        let extra_top = top + items * rh + self.s(SEP_H);
        if y >= extra_top {
            let j = (y - extra_top) / rh;
            if (j as usize) < self.extra_rows().len() {
                return Some(PopupHit::Extra(j as usize));
            }
        }
        None
    }

    /// 확장 항목(Choose… 등) 선택 처리 — 재정의(기본은 그냥 닫기).
    fn on_extra(&mut self, _j: usize, inv: &mut Invalidations) {
        self.close(inv);
    }

    // ── 렌더 ──
    /// 닫힌 박스 + 셰브론 + (열렸으면) 드롭다운을 그린다. 편집 텍스트는 구현체가 `box_text`로 제공.
    fn paint_combo(&self, ctx: &mut dyn DrawCtx, theme: &Theme, box_text: &str) {
        let b = self.bounds();
        ctx.fill_round_rect(b, self.s(6), theme.field_bg);
        ctx.stroke_round_rect(b, self.s(6), theme.border, 1.0);
        self.draw_focus_ring(ctx, theme, b);

        // 선택 항목 아이콘(이미지 우선 · 없으면 글리프) + 텍스트.
        let cy = b.y + b.h / 2;
        let mut tx = b.x + self.s(10);
        if let Some(it) = self.core().items.get(self.core().selected) {
            tx = draw_leading(
                ctx,
                it,
                tx,
                cy,
                self.s(ICON_SZ),
                self.s(16),
                self.s(3),
                b,
                theme.text,
            );
        }
        ctx.select_font(FontSlot::Base, false);
        let th = ctx.text_height();
        ctx.text(tx, cy - th / 2, b, box_text, theme.text);

        // 셰브론(⇕ 확장 / ∨ 일반) — 트리와 동일한 회색 계열(text_dim).
        let chev = self.chevron_rect();
        let color = theme.text_dim;
        if self.is_editable() {
            draw_updown_chevrons(ctx, theme, chev, color);
        } else {
            draw_chevron_down(ctx, chev, color);
        }

        if self.core().open {
            self.paint_dropdown(ctx, theme);
        }

        // 도움말(맨 끝 — 위에 겹침).
        let badge = self.help_badge_rect(b);
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);
    }

    /// 드롭다운 패널 — 항목(선택 ✓·후버 하이라이트) + (있으면) 구분자 + 확장 항목.
    fn paint_dropdown(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let pop = self.popup_rect();
        ctx.fill_round_rect(pop, self.s(8), theme.chrome_bg);
        ctx.stroke_round_rect(pop, self.s(8), theme.border, 1.0);
        let rh = self.s(ROW_H);
        let mut y = pop.y + self.s(POPUP_PAD);
        for (i, it) in self.core().items.iter().enumerate() {
            let row = Rect::new(pop.x + self.s(3), y, pop.w - self.s(6), rh);
            if i == self.core().hover {
                ctx.fill_round_rect(row, self.s(5), theme.sel_bg);
            }
            // 선택 ✓(크기 70% — 사용자 확정 · 16→11).
            let cs = self.s(11);
            let check = Rect::new(row.x + self.s(6), row.y + (rh - cs) / 2, cs, cs);
            if Some(i) == self.checked_index() {
                draw_check_mark(ctx, check, self.accent_now(theme));
            }
            let cy = row.y + rh / 2;
            let tx = draw_leading(
                ctx,
                it,
                check.right() + self.s(6),
                cy,
                self.s(ICON_SZ),
                self.s(16),
                self.s(3),
                row,
                theme.text,
            );
            ctx.select_font(FontSlot::Base, false);
            let th = ctx.text_height();
            ctx.text(tx, cy - th / 2, row, &it.label, theme.text);
            y += rh;
        }
        // 구분자(Horizon) + 확장 항목("Choose…" 등 · 아이콘 옵션).
        let extras = self.extra_rows();
        if !extras.is_empty() {
            let sep_y = y + self.s(SEP_H) / 2;
            ctx.fill_rect(
                Rect::new(pop.x + self.s(8), sep_y, pop.w - self.s(16), 1),
                theme.border,
            );
            y += self.s(SEP_H);
            for (j, it) in extras.iter().enumerate() {
                let row = Rect::new(pop.x + self.s(3), y, pop.w - self.s(6), rh);
                let cy = row.y + rh / 2;
                // 항목과 같은 ✓ 슬롯(정렬 통일) — 직접 입력 값 활성 시 "직접 입력…"에 ✓.
                let cs = self.s(11);
                let check = Rect::new(row.x + self.s(6), row.y + (rh - cs) / 2, cs, cs);
                if self.extra_checked(j) {
                    draw_check_mark(ctx, check, self.accent_now(theme));
                }
                let tx = draw_leading(
                    ctx,
                    it,
                    check.right() + self.s(6),
                    cy,
                    self.s(ICON_SZ),
                    self.s(16),
                    self.s(3),
                    row,
                    theme.accent,
                );
                ctx.select_font(FontSlot::Base, false);
                let th = ctx.text_height();
                ctx.text(tx, cy - th / 2, row, &it.label, theme.accent);
                y += rh;
            }
        }
    }
}

/// 팝업이 열렸으면 팝업 영역, 아니면 박스 — 무효화 대상.
fn popup_or_box<C: ComboControl + ?Sized>(box_b: Rect, c: &C) -> Rect {
    let pop = c.popup_rect();
    if pop.is_empty() {
        box_b
    } else {
        box_b.union(&pop)
    }
}

// ───────────────────────────── 일반 콤보(∨) ─────────────────────────────

/// 일반 콤보박스 — 목록에서 택일(∨).
///
/// **직접 입력(옵션)**: [`Combo::set_custom_entry`]를 켜면 구분자 아래 "직접 입력…" 항목이
/// 생기고, 고르면 읽기 전용이던 박스가 **인라인 숫자 편집**으로 바뀐다(사용자 요청 08-09).
/// Enter/바깥 클릭 = 확정(값 보고) · Esc = 취소. 목록에 없는 값은 `값+접미`로 표시된다.
#[derive(Debug)]
pub struct Combo {
    base: ControlBase,
    core: ComboCore,
    /// 직접 입력 항목 라벨(None = 기능 꺼짐).
    custom_label: Option<String>,
    /// 커스텀 값 표시 접미(예: "ms").
    custom_suffix: String,
    /// 목록에 없는 커스텀 값(직접 입력 결과 · 원시 문자열).
    custom_value: Option<String>,
    /// 인라인 편집 중.
    editing: bool,
    /// 편집 버퍼(숫자 전용).
    buf: String,
}

impl Combo {
    /// 항목과 초기 선택으로 만든다.
    #[must_use]
    pub fn new(items: Vec<ComboItem>, selected: usize) -> Self {
        Self {
            base: ControlBase::default(),
            core: ComboCore::new(items, selected),
            custom_label: None,
            custom_suffix: String::new(),
            custom_value: None,
            editing: false,
            buf: String::new(),
        }
    }
    /// 선택된 값.
    #[must_use]
    pub fn selected_value(&self) -> String {
        self.value()
    }
    /// 값으로 선택 지정(보고 없음). 목록에 없으면(직접 입력 켜짐) 커스텀 값으로 표시.
    pub fn select_value(&mut self, value: &str) {
        if let Some(i) = self.core.items.iter().position(|it| it.value == value) {
            self.core.selected = i;
            self.core.hover = i;
            self.custom_value = None;
        } else if self.custom_label.is_some() && !value.is_empty() {
            self.custom_value = Some(value.to_string());
        }
    }
    /// 직접 입력 항목을 켠다 — (드롭다운 라벨, 표시 접미).
    pub fn set_custom_entry(&mut self, label: impl Into<String>, suffix: impl Into<String>) {
        self.custom_label = Some(label.into());
        self.custom_suffix = suffix.into();
    }
    /// 인라인 편집 중인가(호스트 라우팅 근거 — 편집은 모달).
    #[must_use]
    pub fn is_editing(&self) -> bool {
        self.editing
    }
    /// 편집 확정 — 버퍼가 비지 않으면 값 보고(목록과 일치하면 그 항목 선택으로 환원).
    fn commit_edit(&mut self, inv: &mut Invalidations) {
        self.editing = false;
        if !self.buf.is_empty() {
            let v = std::mem::take(&mut self.buf);
            if let Some(i) = self.core.items.iter().position(|it| it.value == v) {
                self.core.selected = i;
                self.custom_value = None;
            } else {
                self.custom_value = Some(v);
            }
            self.core.changed = true;
        }
        inv.push(self.base.bounds);
    }
}

impl Control for Combo {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}
impl ComboControl for Combo {
    fn core(&self) -> &ComboCore {
        &self.core
    }
    fn core_mut(&mut self) -> &mut ComboCore {
        &mut self.core
    }
    fn extra_rows(&self) -> Vec<ComboItem> {
        self.custom_label
            .as_deref()
            .map(|l| vec![ComboItem::new("__custom__", l)])
            .unwrap_or_default()
    }
    fn value(&self) -> String {
        // 커스텀 값이 살아 있으면 그것이 현재 값.
        self.custom_value.clone().unwrap_or_else(|| {
            self.core
                .items
                .get(self.core.selected)
                .map(|it| it.value.clone())
                .unwrap_or_default()
        })
    }
    fn choose_index(&mut self, i: usize, inv: &mut Invalidations) {
        if i < self.core.items.len() {
            let changed = self.core.selected != i || self.custom_value.is_some();
            self.core.selected = i;
            self.core.open = false;
            self.core.changed |= changed;
            self.custom_value = None; // 목록 선택 = 커스텀 해제
            inv.push(self.base.bounds);
        }
    }
    fn on_extra(&mut self, _j: usize, inv: &mut Invalidations) {
        // "직접 입력…" — 박스를 인라인 편집으로 전환(현재 값의 숫자만 남겨 시작).
        self.core.open = false;
        self.buf = self.value().chars().filter(char::is_ascii_digit).collect();
        self.editing = true;
        inv.push(self.base.bounds);
    }
    fn checked_index(&self) -> Option<usize> {
        // 커스텀 값이 살아 있으면 목록 어디에도 ✓를 찍지 않는다(표시 착오 방지).
        if self.custom_value.is_some() {
            None
        } else {
            Some(self.core.selected)
        }
    }
    fn extra_checked(&self, _j: usize) -> bool {
        self.custom_value.is_some()
    }
}

impl Widget for Combo {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }
    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        inv.push(bounds);
    }
    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 인라인 편집 중 = 모달 — 숫자만 받고 Enter/바깥 클릭 확정 · Esc 취소.
        if self.editing {
            match *ev {
                InputEvent::Char { c, .. } => {
                    if c == '\u{8}' {
                        self.buf.pop();
                    } else if c.is_ascii_digit() && self.buf.len() < 6 {
                        self.buf.push(c);
                    }
                    inv.push(self.base.bounds);
                }
                InputEvent::Key { key, .. } => match key {
                    Key::Enter => self.commit_edit(inv),
                    Key::Escape => {
                        self.editing = false;
                        self.buf.clear();
                        inv.push(self.base.bounds);
                    }
                    _ => {}
                },
                InputEvent::MouseDown { x, y, .. }
                    if !self.base.bounds.contains(Point { x, y }) =>
                {
                    self.commit_edit(inv); // 바깥 클릭 = 확정
                }
                _ => {}
            }
            return;
        }
        combo_event(self, ev, inv);
    }
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        if self.editing {
            // 편집 모드 — 버퍼 + 캐럿 + accent 테두리(입력 가능 상태 식별).
            self.paint_combo(ctx, theme, &self.buf);
            let b = self.base.bounds;
            ctx.select_font(FontSlot::Base, false);
            let tw = ctx.text_width(&self.buf);
            let th = ctx.text_height();
            if ctx.caret_on() {
                // 깜빡임 위상은 호스트 주입(08-13).
                ctx.fill_rect(
                    Rect::new(
                        b.x + self.s(10) + tw + 1,
                        b.y + (b.h - th) / 2,
                        self.s(1).max(1),
                        th,
                    ),
                    theme.text,
                );
            }
            ctx.stroke_round_rect(b, self.s(6), self.accent_now(theme), 1.0);
            return;
        }
        let text = self.custom_value.as_ref().map_or_else(
            || {
                self.core
                    .items
                    .get(self.core.selected)
                    .map_or("", |it| it.label.as_str())
                    .to_string()
            },
            |v| format!("{v}{}", self.custom_suffix),
        );
        self.paint_combo(ctx, theme, &text);
    }
}

// ─────────────────────────── 확장 콤보(⇕) ───────────────────────────

/// 확장 콤보박스 — 값 직접 편집(⇕) + 구분자 아래 **"Choose…"** 로 커스텀 값 선택.
/// [`Combo`]의 모델([`ComboCore`])을 물려받아 편집 필드와 Choose 항목을 덧붙인다.
#[derive(Debug)]
pub struct Choose {
    base: ControlBase,
    core: ComboCore,
    /// 직접 편집 텍스트(커스텀 값).
    edit: EditState,
    /// "Choose…" 항목 라벨.
    choose_label: String,
    /// "Choose…" 항목 선행 아이콘(옵션).
    choose_icon: Option<String>,
    /// Choose… 활성화 1회성(어댑터가 없을 때만 — 호스트가 직접 처리).
    chose: bool,
    /// 찾기 창 어댑터(Adapter 패턴 · 있으면 인라인 오버레이로 후보 표시).
    picker: Option<Box<dyn ChoosePicker>>,
    /// 찾기 오버레이가 열려 있는가.
    picking: bool,
    /// 찾기 오버레이 세로 스크롤(물리 px).
    pick_scroll: i32,
    /// 찾기 오버레이 스크롤바.
    pick_bars: ScrollBars,
}

impl Choose {
    /// 항목·초기 선택·Choose 라벨로 만든다. 편집 텍스트는 선택 항목 라벨로 시작.
    #[must_use]
    pub fn new(items: Vec<ComboItem>, selected: usize, choose_label: impl Into<String>) -> Self {
        let core = ComboCore::new(items, selected);
        let init = core
            .items
            .get(core.selected)
            .map_or(String::new(), |it| it.label.clone());
        Self {
            base: ControlBase::default(),
            core,
            edit: EditState::with_text(&init, false),
            choose_label: choose_label.into(),
            choose_icon: None,
            chose: false,
            picker: None,
            picking: false,
            pick_scroll: 0,
            pick_bars: ScrollBars::new(),
        }
    }

    /// "Choose…" 항목에 선행 아이콘 지정(체이닝).
    #[must_use]
    pub fn with_choose_icon(mut self, icon: impl Into<String>) -> Self {
        self.choose_icon = Some(icon.into());
        self
    }

    /// 찾기 창 어댑터를 꽂는다(Adapter 패턴). 이후 "Choose…"가 인라인 오버레이를 연다.
    pub fn set_picker(&mut self, picker: Box<dyn ChoosePicker>) {
        self.picker = Some(picker);
    }

    /// 찾기 오버레이가 열려 있는가.
    #[must_use]
    pub fn is_picking(&self) -> bool {
        self.picking
    }

    /// 찾기 오버레이 rect(닫혀 있으면 빈 rect). 제목행 + 목록(최대 8행) + 여백.
    fn picker_rect(&self) -> Rect {
        if !self.picking {
            return Rect::new(0, 0, 0, 0);
        }
        let b = self.base.bounds;
        let n = self.picker_items().len().min(8) as i32;
        let rh = self.s(PICK_ROW_H);
        let w = b.w.max(self.s(280));
        let h = self.s(PICK_TITLE_H) + n * rh + self.s(POPUP_PAD) * 2;
        Rect::new(b.x, b.bottom() + self.s(2), w, h)
    }

    /// 찾기 오버레이 목록 뷰포트(제목 아래).
    fn picker_viewport(&self) -> Rect {
        let p = self.picker_rect();
        let top = p.y + self.s(PICK_TITLE_H);
        Rect::new(p.x, top, p.w, (p.bottom() - top - self.s(POPUP_PAD)).max(0))
    }

    fn picker_items(&self) -> Vec<ComboItem> {
        self.picker.as_ref().map_or_else(Vec::new, |p| p.items())
    }

    /// 찾기 오버레이의 (x,y) → 항목 인덱스.
    fn picker_hit(&self, x: i32, y: i32) -> Option<usize> {
        let vp = self.picker_viewport();
        if !vp.contains(Point { x, y }) {
            return None;
        }
        let rh = self.s(PICK_ROW_H).max(1);
        let i = ((y - vp.y + self.pick_scroll) / rh) as usize;
        (i < self.picker_items().len()).then_some(i)
    }

    /// 찾기 오버레이에서 항목 선택 — 라벨을 값으로 반영하고 닫는다.
    fn pick(&mut self, i: usize, inv: &mut Invalidations) {
        if let Some(it) = self.picker_items().get(i) {
            self.edit.set_text(&it.label);
            self.core.changed = true;
            self.picking = false;
            inv.push(self.base.bounds);
        }
    }

    /// 현재 텍스트(커스텀 편집 값).
    #[must_use]
    pub fn text(&self) -> String {
        self.edit.text()
    }
    /// 텍스트 지정.
    pub fn set_text(&mut self, text: &str) {
        self.edit.set_text(text);
    }
    /// "Choose…"가 눌렸으면 `true`(1회성) — 호스트가 커스텀 값 선택 UI를 연다.
    pub fn take_chose(&mut self) -> bool {
        std::mem::take(&mut self.chose)
    }
    /// 편집 텍스트 변경 1회성.
    pub fn take_text_changed(&mut self) -> Option<String> {
        // core.changed는 항목 선택, 편집 변경도 여기로 합친다.
        self.take_changed()
    }
}

impl Control for Choose {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}
impl ComboControl for Choose {
    fn core(&self) -> &ComboCore {
        &self.core
    }
    fn core_mut(&mut self) -> &mut ComboCore {
        &mut self.core
    }
    fn is_editable(&self) -> bool {
        true
    }
    fn extra_rows(&self) -> Vec<ComboItem> {
        let mut it = ComboItem::new("__choose__", &self.choose_label);
        if let Some(icon) = &self.choose_icon {
            it = it.with_icon(icon.clone());
        }
        vec![it]
    }
    fn value(&self) -> String {
        // 확장 콤보의 값 = 편집 텍스트(커스텀 우선).
        self.edit.text()
    }
    fn choose_index(&mut self, i: usize, inv: &mut Invalidations) {
        // 확장 콤보는 목록 선택 시 **편집 텍스트도 갱신**해야 박스 표시가 바뀐다
        // (박스 텍스트 = edit.text() · 기본 구현은 selected만 바꿔 표시가 안 바뀌던 버그).
        if let Some(it) = self.core.items.get(i) {
            self.edit.set_text(&it.label);
        }
        if i < self.core.items.len() {
            self.core.selected = i;
            self.core.open = false;
            self.core.changed = true;
            inv.push(self.base.bounds);
        }
    }
    fn on_extra(&mut self, _j: usize, inv: &mut Invalidations) {
        self.core.open = false;
        if self.picker.is_some() {
            // 어댑터가 있으면 인라인 찾기 오버레이를 연다.
            self.picking = true;
            self.pick_scroll = 0;
        } else {
            // 없으면 호스트가 직접 처리하도록 신호만 남긴다.
            self.chose = true;
        }
        inv.push(self.base.bounds);
    }
}

impl Widget for Choose {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }
    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        inv.push(bounds);
    }
    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 찾기 오버레이가 열려 있으면 그쪽이 먼저 처리한다.
        if self.picking {
            let vp = self.picker_viewport();
            let ch = self.picker_items().len() as i32 * self.s(PICK_ROW_H);
            let (_nx, ny, consumed) =
                self.pick_bars
                    .on_event(ev, vp, vp.w, ch, 0, self.pick_scroll, self.base.scale);
            self.pick_scroll = ny;
            match *ev {
                InputEvent::MouseDown { x, y, .. } if !consumed => {
                    if let Some(i) = self.picker_hit(x, y) {
                        self.pick(i, inv);
                    } else if !self.picker_rect().contains(Point { x, y }) {
                        self.picking = false; // 바깥 클릭 = 취소
                        inv.push(self.base.bounds);
                    }
                }
                InputEvent::Key {
                    key: Key::Escape, ..
                } => {
                    self.picking = false;
                    inv.push(self.base.bounds);
                }
                _ => inv.push(self.base.bounds),
            }
            return;
        }
        // 편집 입력(포커스 상태의 타이핑)은 확장 콤보 고유.
        if let InputEvent::Char { c, .. } = *ev {
            if self.base.focused {
                if c == '\u{8}' {
                    self.edit.backspace();
                } else if !c.is_control() {
                    self.edit.insert(c);
                }
                self.core.changed = true;
                inv.push(self.base.bounds);
            }
            return;
        }
        combo_event(self, ev, inv);
    }
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let text = self.edit.text();
        self.paint_combo(ctx, theme, &text);
        if self.picking {
            self.paint_picker(ctx, theme);
        }
    }
}

impl Choose {
    /// 찾기 오버레이 렌더 — 제목 + 후보 목록(스크롤) + 오버레이 스크롤바.
    fn paint_picker(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let p = self.picker_rect();
        ctx.fill_round_rect(p, self.s(8), theme.chrome_bg);
        ctx.stroke_round_rect(p, self.s(8), theme.border, 1.0);
        // 제목.
        ctx.select_font(FontSlot::Status, false);
        let title = self.picker.as_ref().map_or_else(String::new, |p| p.title());
        ctx.text(p.x + self.s(10), p.y + self.s(6), p, &title, theme.text_dim);
        ctx.fill_rect(
            Rect::new(
                p.x + self.s(6),
                p.y + self.s(PICK_TITLE_H) - 1,
                p.w - self.s(12),
                1,
            ),
            theme.border,
        );
        // 목록(뷰포트 안 온전한 행만 · 세로 스크롤).
        let vp = self.picker_viewport();
        let rh = self.s(PICK_ROW_H);
        let items = self.picker_items();
        for (i, it) in items.iter().enumerate() {
            let y = vp.y - self.pick_scroll + rh * i as i32;
            if y < vp.y || y + rh > vp.bottom() {
                continue;
            }
            let row = Rect::new(vp.x + self.s(3), y, vp.w - self.s(6), rh);
            let cy = y + rh / 2;
            let tx = draw_leading(
                ctx,
                it,
                row.x + self.s(8),
                cy,
                self.s(ICON_SZ),
                self.s(16),
                self.s(3),
                row,
                theme.text,
            );
            ctx.select_font(FontSlot::Base, false);
            let th = ctx.text_height();
            ctx.text(tx, cy - th / 2, row, &it.label, theme.text);
        }
        let ch = items.len() as i32 * rh;
        self.pick_bars.paint(
            ctx,
            theme,
            vp,
            vp.w,
            ch,
            0,
            self.pick_scroll,
            self.base.scale,
        );
    }
}

/// 콤보 공통 이벤트 처리(일반·확장 공용) — 마우스/키보드로 열기·후버·선택.
fn combo_event<C: ComboControl + ?Sized>(c: &mut C, ev: &InputEvent, inv: &mut Invalidations) {
    match *ev {
        InputEvent::MouseDown { x, y, .. } => {
            let badge = c.help_badge_rect(c.bounds());
            if c.handle_help_click(x, y, badge) {
                inv.push(c.bounds());
                return;
            }
            if c.is_open() {
                match c.popup_hit(x, y) {
                    Some(PopupHit::Item(i)) => c.choose_index(i, inv),
                    Some(PopupHit::Extra(j)) => c.on_extra(j, inv),
                    None => c.close(inv), // 바깥 클릭 = 닫기
                }
                return;
            }
            if c.bounds().contains(Point { x, y }) {
                c.set_focused(true);
                c.toggle_open(inv);
            }
        }
        InputEvent::Key { key, .. } if c.is_focused() => match key {
            Key::Escape => c.close(inv),
            Key::Enter | Key::Space => {
                if c.is_open() {
                    let h = c.core().hover;
                    c.choose_index(h, inv);
                } else {
                    c.toggle_open(inv);
                }
            }
            Key::Down => {
                if c.is_open() {
                    c.move_hover(1);
                    inv.push(c.popup_rect());
                } else {
                    c.toggle_open(inv);
                }
            }
            Key::Up if c.is_open() => {
                c.move_hover(-1);
                inv.push(c.popup_rect());
            }
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<ComboItem> {
        vec![
            ComboItem::new("home", "Home"),
            ComboItem::new("recents", "Recents"),
            ComboItem::new("desktop", "Desktop"),
        ]
    }
    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn custom_entry_inline_edit_commits_value() {
        let mut c = Combo::new(items(), 0);
        c.set_custom_entry("직접 입력…", "ms");
        let mut inv = Invalidations::default();
        c.set_bounds(Rect::new(0, 0, 160, 26), &mut inv);
        // 열고 → 확장 항목("직접 입력…") 클릭 → 편집 모드.
        c.on_event(&click(10, 10), &mut inv);
        assert!(c.is_open());
        let pop = c.popup_rect();
        c.on_event(&click(pop.x + 10, pop.bottom() - 8), &mut inv); // 마지막 행 = 확장
        assert!(c.is_editing(), "직접 입력 = 인라인 편집");
        // 숫자만 받는다 + Enter 확정.
        for d in ['a', '3', '5', '0', '0'] {
            c.on_event(&ch(d), &mut inv);
        }
        c.on_event(&key(Key::Enter), &mut inv);
        assert!(!c.is_editing());
        assert_eq!(
            c.take_changed().as_deref(),
            Some("3500"),
            "비숫자 시작값은 비워진다"
        );
    }

    #[test]
    fn custom_edit_starts_from_current_and_esc_cancels() {
        let mut c = Combo::new(
            vec![
                ComboItem::new("1000", "1000ms"),
                ComboItem::new("2000", "2000ms"),
            ],
            1,
        );
        c.set_custom_entry("직접 입력…", "ms");
        let mut inv = Invalidations::default();
        c.set_bounds(Rect::new(0, 0, 160, 26), &mut inv);
        c.on_event(&click(10, 10), &mut inv);
        let pop = c.popup_rect();
        c.on_event(&click(pop.x + 10, pop.bottom() - 8), &mut inv);
        assert!(c.is_editing());
        // Esc = 취소(보고 없음).
        c.on_event(&key(Key::Escape), &mut inv);
        assert!(!c.is_editing());
        assert!(c.take_changed().is_none());
        // 백스페이스로 지우고 새 값 입력 → 목록에 없는 값 = 커스텀, 있으면 그 항목.
        c.on_event(&click(10, 10), &mut inv);
        let pop = c.popup_rect();
        c.on_event(&click(pop.x + 10, pop.bottom() - 8), &mut inv);
        for _ in 0..4 {
            c.on_event(&ch('\u{8}'), &mut inv);
        }
        for d in ['7', '5', '0', '0'] {
            c.on_event(&ch(d), &mut inv);
        }
        c.on_event(&key(Key::Enter), &mut inv);
        assert_eq!(c.take_changed().as_deref(), Some("7500"), "커스텀 값 보고");
        // 커스텀 활성 = ✓가 목록이 아니라 "직접 입력…"에 붙는다(사용자 지적 08-09).
        assert_eq!(c.checked_index(), None, "목록 ✓ 없음");
        assert!(c.extra_checked(0), "직접 입력 ✓");
        // 목록 재선택 = 커스텀 해제.
        c.on_event(&click(10, 10), &mut inv);
        let pop = c.popup_rect();
        c.on_event(&click(pop.x + 10, pop.y + 6), &mut inv); // 첫 항목
        assert_eq!(c.take_changed().as_deref(), Some("1000"));
        assert_eq!(c.checked_index(), Some(0), "재선택 = 목록 ✓ 복귀");
        assert!(!c.extra_checked(0));
    }
    fn key(k: Key) -> InputEvent {
        InputEvent::Key {
            key: k,
            shift: false,
            primary: false,
        }
    }
    fn ch(c: char) -> InputEvent {
        InputEvent::Char { c, now_ms: 0 }
    }

    fn combo() -> (Combo, Invalidations) {
        let mut c = Combo::new(items(), 0);
        let mut inv = Invalidations::default();
        c.set_bounds(Rect::new(0, 0, 160, 28), &mut inv);
        (c, inv)
    }

    #[test]
    fn click_opens_then_select_reports_and_closes() {
        let (mut c, mut inv) = combo();
        c.on_event(&click(20, 14), &mut inv); // 박스 클릭 → 열림
        assert!(c.is_open());
        // 팝업 2번째 항목(Recents) 클릭.
        let pop = c.popup_rect();
        let rh = 28;
        let y = pop.y + 4 + rh + 5;
        c.on_event(&click(pop.x + 30, y), &mut inv);
        assert!(!c.is_open(), "선택 후 닫힘");
        assert_eq!(c.take_changed().as_deref(), Some("recents"));
    }

    #[test]
    fn keyboard_navigates_dropdown() {
        let (mut c, mut inv) = combo();
        c.set_focused(true);
        c.on_event(&key(Key::Down), &mut inv); // 열림
        assert!(c.is_open());
        c.on_event(&key(Key::Down), &mut inv); // hover 0→1
        c.on_event(&key(Key::Enter), &mut inv); // 선택
        assert_eq!(c.take_changed().as_deref(), Some("recents"));
        assert!(!c.is_open());
    }

    #[test]
    fn outside_click_closes_without_change() {
        let (mut c, mut inv) = combo();
        c.on_event(&click(20, 14), &mut inv);
        assert!(c.is_open());
        c.on_event(&click(500, 500), &mut inv);
        assert!(!c.is_open());
        assert!(c.take_changed().is_none());
    }

    #[test]
    fn normal_combo_has_no_extra_rows() {
        let (c, _) = combo();
        assert!(c.extra_rows().is_empty());
        assert!(!c.is_editable(), "일반 = ∨");
    }

    fn ext() -> (Choose, Invalidations) {
        let mut c = Choose::new(items(), 0, "Choose…");
        let mut inv = Invalidations::default();
        c.set_bounds(Rect::new(0, 0, 200, 28), &mut inv);
        (c, inv)
    }

    #[test]
    fn extended_is_editable_with_choose_row() {
        let (c, _) = ext();
        assert!(c.is_editable(), "확장 = ⇕");
        let extras = c.extra_rows();
        assert_eq!(extras.len(), 1);
        assert_eq!(extras[0].label, "Choose…");
        assert_eq!(c.text(), "Home", "편집 텍스트 = 선택 라벨로 시작");
    }

    #[test]
    fn extended_typing_sets_custom_value() {
        let (mut c, mut inv) = ext();
        c.set_focused(true);
        c.set_text("");
        for ch_ in "MyApp".chars() {
            c.on_event(&ch(ch_), &mut inv);
        }
        assert_eq!(c.text(), "MyApp");
        assert_eq!(
            c.take_text_changed().as_deref(),
            Some("MyApp"),
            "값 = 커스텀 텍스트"
        );
    }

    #[test]
    fn extended_choose_row_fires_action() {
        let (mut c, mut inv) = ext();
        c.on_event(&click(20, 14), &mut inv); // 열림
        assert!(c.is_open());
        // Choose… 행 = 항목 3개 + 구분자 아래.
        let pop = c.popup_rect();
        let y = pop.y + 4 + 3 * 28 + 9 + 5; // pad + 3항목 + sep + 확장행 안쪽
        c.on_event(&click(pop.x + 20, y), &mut inv);
        assert!(c.take_chose(), "Choose… 발동");
        assert!(!c.is_open());
    }
}
