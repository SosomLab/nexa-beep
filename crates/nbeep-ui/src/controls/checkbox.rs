//! 체크박스 — 체크만 / 라벨 좌·우 옵션([`LabelSide`]) · 창 활성 색 구분 · 포커스 링 · 도움말.
//!
//! 공통 기능(포커스 링·활성·"?" 도움말)은 [`Control`] 기본 메서드로 상속한다([`super`]).

use super::{draw_checkbox_glyph, Control, ControlBase, LabelSide};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 체크박스 글리프 한 변(논리 px) — 라디오와 동일 크기(사용자 확정: 옵션박스보다 4px 작게).
const BOX: i32 = 12;
/// 글리프 ↔ 라벨 간격(논리 px).
const GAP: i32 = 8;

/// 체크박스 컨트롤.
#[derive(Debug)]
pub struct Checkbox {
    base: ControlBase,
    checked: bool,
    label: String,
    side: LabelSide,
    /// 값 변경 1회성 보고(즉시 적용 폴링).
    toggled: bool,
}

impl Checkbox {
    /// 라벨과 초기 체크 상태로 만든다(라벨 오른쪽이 기본).
    #[must_use]
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            base: ControlBase::default(),
            checked,
            label: label.into(),
            side: LabelSide::Right,
            toggled: false,
        }
    }

    /// 라벨 위치 지정(체크만 = [`LabelSide::None`]).
    #[must_use]
    pub fn with_label_side(mut self, side: LabelSide) -> Self {
        self.side = side;
        self
    }

    /// 현재 체크 여부.
    #[must_use]
    pub fn is_checked(&self) -> bool {
        self.checked
    }

    /// 체크 상태 지정(보고 없음 — 호스트 주도 설정).
    pub fn set_checked(&mut self, on: bool) {
        self.checked = on;
    }

    /// 토글되었으면 새 상태를 꺼낸다(1회성).
    pub fn take_toggled(&mut self) -> Option<bool> {
        std::mem::take(&mut self.toggled).then_some(self.checked)
    }

    fn toggle(&mut self, inv: &mut Invalidations) {
        self.checked = !self.checked;
        self.toggled = true;
        inv.push(self.base.bounds);
    }

    /// 글리프 rect(라벨 위치에 따라 좌/우). 크기 배율(`ui.control_size`) 적용.
    fn box_rect(&self) -> Rect {
        let d = self.s(super::ctl_size(BOX));
        let b = self.base.bounds;
        let y = b.y + (b.h - d) / 2;
        match self.side {
            LabelSide::Left => Rect::new(b.right() - d, y, d, d),
            LabelSide::None | LabelSide::Right => Rect::new(b.x, y, d, d),
        }
    }

    /// 라벨 텍스트 rect.
    fn label_rect(&self) -> Rect {
        let d = self.s(super::ctl_size(BOX));
        let gap = self.s(GAP);
        let b = self.base.bounds;
        match self.side {
            LabelSide::Right => Rect::new(b.x + d + gap, b.y, b.w - d - gap, b.h),
            LabelSide::Left => Rect::new(b.x, b.y, b.w - d - gap, b.h),
            LabelSide::None => Rect::new(b.x, b.y, 0, b.h),
        }
    }

    /// 클릭 히트 영역(글리프 + 라벨).
    fn hit_rect(&self) -> Rect {
        self.base.bounds
    }
}

impl Control for Checkbox {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for Checkbox {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::MouseDown { x, y, .. } => {
                let badge = self.help_badge_rect(self.hit_rect());
                if self.handle_help_click(x, y, badge) {
                    inv.push(self.base.bounds);
                    return;
                }
                if self.hit_rect().contains(Point { x, y }) {
                    self.toggle(inv);
                }
            }
            InputEvent::Key { key, .. } if self.base.focused => {
                if matches!(key, Key::Space | Key::Enter) {
                    self.toggle(inv);
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let box_r = self.box_rect();
        self.draw_focus_ring(ctx, theme, box_r);
        draw_checkbox_glyph(ctx, theme, box_r, self.checked, self.base.active);

        if self.side != LabelSide::None && !self.label.is_empty() {
            let lr = self.label_rect();
            ctx.select_font(FontSlot::Base, false);
            let ty = lr.y + (lr.h - ctx.text_height()) / 2;
            ctx.text(lr.x, ty, lr, &self.label, theme.text);
        }

        let badge = self.help_badge_rect(self.hit_rect());
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cb() -> (Checkbox, Invalidations) {
        let mut c = Checkbox::new("Enable bug reporter", false);
        let mut inv = Invalidations::default();
        c.set_bounds(Rect::new(0, 0, 240, 24), &mut inv);
        (c, inv)
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
    fn click_toggles_and_reports_once() {
        let (mut c, mut inv) = cb();
        assert!(!c.is_checked());
        c.on_event(&click(5, 12), &mut inv);
        assert_eq!(c.take_toggled(), Some(true));
        assert!(c.take_toggled().is_none(), "1회성");
        assert!(c.is_checked());
    }

    #[test]
    fn space_toggles_only_when_focused() {
        let (mut c, mut inv) = cb();
        let space = InputEvent::Key {
            key: Key::Space,
            shift: false,
            primary: false,
        };
        c.on_event(&space, &mut inv);
        assert!(!c.is_checked(), "비포커스 = 무시");
        c.set_focused(true);
        c.on_event(&space, &mut inv);
        assert!(c.is_checked(), "포커스 = 토글");
    }

    #[test]
    fn help_badge_click_opens_tip_not_toggle() {
        let (mut c, mut inv) = cb();
        c.set_help("Sends anonymous crash reports.");
        c.set_show_help(true);
        let badge = c.help_badge_rect(c.hit_rect());
        c.on_event(&click(badge.x + 2, badge.y + 2), &mut inv);
        assert!(c.base().help_open, "툴팁 열림");
        assert!(!c.is_checked(), "도움말 클릭은 토글 아님");
    }

    #[test]
    fn help_badge_reclick_closes_tip() {
        let (mut c, mut inv) = cb();
        c.set_help("x");
        c.set_show_help(true);
        let badge = c.help_badge_rect(c.hit_rect());
        c.on_event(&click(badge.x + 2, badge.y + 2), &mut inv);
        assert!(c.base().help_open, "1클릭 = 열림");
        c.on_event(&click(badge.x + 2, badge.y + 2), &mut inv);
        assert!(!c.base().help_open, "재클릭 = 닫힘(토글)");
    }

    #[test]
    fn label_side_moves_the_box() {
        let (mut c, mut inv) = cb();
        let left_x = c.box_rect().x;
        c.side = LabelSide::Left;
        let _ = &mut inv;
        assert!(c.box_rect().x > left_x, "라벨 왼쪽 = 글리프 오른쪽으로");
    }

    #[test]
    fn hidden_help_has_no_badge_interaction() {
        let (mut c, mut inv) = cb();
        c.set_help("x");
        // show_help = false → 배지 없음, 그 자리 클릭은 토글로 흐르지 않는다(영역 밖).
        let badge = c.help_badge_rect(c.hit_rect());
        c.on_event(&click(badge.x + 2, badge.y + 2), &mut inv);
        assert!(!c.base().help_open);
    }
}
