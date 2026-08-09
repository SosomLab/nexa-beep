//! 옵션 박스(라디오 그룹) — 세로 나열·택일 · 창 활성 색 구분 · 포커스 링 · 도움말.
//!
//! 공통 기능은 [`Control`] 기본 메서드로 상속([`super`]).

use super::{draw_radio_glyph, Control, ControlBase};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 라디오 항목.
#[derive(Clone, Debug)]
pub struct RadioOption {
    /// 값(안정 계약).
    pub value: String,
    /// 표시 라벨.
    pub label: String,
}

impl RadioOption {
    /// (값, 라벨)로 만든다.
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

/// 글리프 지름·행 높이(논리 px). 지름은 체크박스와 동일(구 16 → 12, 4px 축소 — 사용자 확정).
const DOT: i32 = 12;
const OPT_H: i32 = 24;
const GAP: i32 = 8;

/// 라디오 그룹 컨트롤(옵션 박스).
#[derive(Debug)]
pub struct RadioGroup {
    base: ControlBase,
    options: Vec<RadioOption>,
    selected: usize,
    changed: bool,
}

impl RadioGroup {
    /// 옵션 목록과 초기 선택으로 만든다.
    #[must_use]
    pub fn new(options: Vec<RadioOption>, selected: usize) -> Self {
        let selected = selected.min(options.len().saturating_sub(1));
        Self {
            base: ControlBase::default(),
            options,
            selected,
            changed: false,
        }
    }

    /// 선택 인덱스.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// 선택된 값.
    #[must_use]
    pub fn selected_value(&self) -> Option<&str> {
        self.options.get(self.selected).map(|o| o.value.as_str())
    }

    /// 값으로 선택 지정(보고 없음).
    pub fn select_value(&mut self, value: &str) {
        if let Some(i) = self.options.iter().position(|o| o.value == value) {
            self.selected = i;
        }
    }

    /// 선택이 바뀌었으면 새 값을 꺼낸다(1회성).
    pub fn take_changed(&mut self) -> Option<String> {
        if std::mem::take(&mut self.changed) {
            self.selected_value().map(str::to_string)
        } else {
            None
        }
    }

    /// 그룹 자연 높이(논리 → 물리).
    #[must_use]
    pub fn preferred_height(&self) -> i32 {
        self.s(OPT_H) * self.options.len() as i32
    }

    fn select(&mut self, i: usize, inv: &mut Invalidations) {
        if i < self.options.len() && i != self.selected {
            self.selected = i;
            self.changed = true;
            inv.push(self.base.bounds);
        }
    }

    fn opt_at(&self, y: i32) -> Option<usize> {
        let h = self.s(OPT_H).max(1);
        if y < self.base.bounds.y {
            return None;
        }
        let i = ((y - self.base.bounds.y) / h) as usize;
        (i < self.options.len()).then_some(i)
    }

    fn glyph_rect(&self, i: usize) -> Rect {
        let d = self.s(DOT);
        let h = self.s(OPT_H);
        let y = self.base.bounds.y + h * i as i32 + (h - d) / 2;
        Rect::new(self.base.bounds.x, y, d, d)
    }
}

impl Control for RadioGroup {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for RadioGroup {
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
                let badge = self.help_badge_rect(self.base.bounds);
                if self.handle_help_click(x, y, badge) {
                    inv.push(self.base.bounds);
                    return;
                }
                let _ = x;
                if let Some(i) = self.opt_at(y) {
                    self.select(i, inv);
                }
            }
            InputEvent::Key { key, .. } if self.base.focused => match key {
                Key::Up => {
                    let i = self.selected.saturating_sub(1);
                    self.select(i, inv);
                }
                Key::Down => {
                    self.select(self.selected + 1, inv);
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        for (i, opt) in self.options.iter().enumerate() {
            let g = self.glyph_rect(i);
            // 포커스 링은 선택된 항목의 글리프에.
            if i == self.selected {
                self.draw_focus_ring(ctx, theme, g);
            }
            draw_radio_glyph(ctx, theme, g, i == self.selected, self.base.active);
            let lx = g.right() + self.s(GAP);
            ctx.select_font(FontSlot::Base, false);
            let ty = g.y + (g.h - ctx.text_height()) / 2;
            let lr = Rect::new(
                lx,
                self.base.bounds.y,
                self.base.bounds.w,
                self.base.bounds.h,
            );
            ctx.select_font(FontSlot::Base, false);
            ctx.text(lx, ty, lr, &opt.label, theme.text);
        }
        let badge = self.help_badge_rect(self.base.bounds);
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> (RadioGroup, Invalidations) {
        let opts = vec![
            RadioOption::new("icon", "Icon view"),
            RadioOption::new("list", "List view"),
            RadioOption::new("column", "Column view"),
        ];
        let mut g = RadioGroup::new(opts, 1);
        let mut inv = Invalidations::default();
        g.set_bounds(Rect::new(0, 0, 200, 26 * 3), &mut inv);
        (g, inv)
    }
    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }
    fn key(k: Key) -> InputEvent {
        InputEvent::Key {
            key: k,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn starts_at_given_selection() {
        let (g, _) = group();
        assert_eq!(g.selected_value(), Some("list"));
    }

    #[test]
    fn click_selects_and_reports() {
        let (mut g, mut inv) = group();
        g.on_event(&click(5, 26 * 2 + 5), &mut inv); // 3번째 = column
        assert_eq!(g.take_changed().as_deref(), Some("column"));
        assert!(g.take_changed().is_none(), "1회성");
    }

    #[test]
    fn arrows_move_selection_when_focused() {
        let (mut g, mut inv) = group();
        g.on_event(&key(Key::Up), &mut inv);
        assert!(!g.changed, "비포커스 = 무시");
        g.set_focused(true);
        g.on_event(&key(Key::Up), &mut inv); // list → icon
        assert_eq!(g.take_changed().as_deref(), Some("icon"));
        g.on_event(&key(Key::Down), &mut inv); // icon → list
        assert_eq!(g.take_changed().as_deref(), Some("list"));
    }

    #[test]
    fn reselecting_same_does_not_report() {
        let (mut g, mut inv) = group();
        g.on_event(&click(5, 26 + 5), &mut inv); // 이미 list 선택
        assert!(g.take_changed().is_none());
    }
}
