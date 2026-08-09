//! **Pull-down 메뉴** — 라벨+∨ 버튼을 누르면 액션 목록이 내려오는 컨트롤(사용자 요청 08-09).
//!
//! [`Combo`](super::Combo)와 달리 **선택 상태가 없다** — 항목은 값이 아니라 **액션**이고,
//! 고르는 즉시 [`PullDown::take_picked`]로 1회성 보고 후 닫힌다(✓ 표시 없음).
//! 공통 기능(포커스 링·활성·도움말)은 [`Control`] 기본 메서드 상속.

use super::{draw_chevron_down, image_fit_contain, ComboItem, Control, ControlBase, LEADING_ICON};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

const ROW_H: i32 = 26;
const CHEV_W: i32 = 16;
const POPUP_PAD: i32 = 4;

/// Pull-down 메뉴 컨트롤.
#[derive(Debug)]
pub struct PullDown {
    base: ControlBase,
    label: String,
    items: Vec<ComboItem>,
    open: bool,
    hover: usize,
    picked: Option<String>,
}

impl PullDown {
    /// 라벨과 액션 항목으로 만든다.
    #[must_use]
    pub fn new(label: impl Into<String>, items: Vec<ComboItem>) -> Self {
        Self {
            base: ControlBase::default(),
            label: label.into(),
            items,
            open: false,
            hover: 0,
            picked: None,
        }
    }

    /// 라벨 교체(i18n 언어 전환 등).
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// 메뉴가 열려 있는가(모달 캡처·최상위 재도색 근거).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 골라진 액션 값(1회성) — 호스트가 실행.
    pub fn take_picked(&mut self) -> Option<String> {
        self.picked.take()
    }

    fn popup_rect(&self) -> Rect {
        if !self.open {
            return Rect::new(0, 0, 0, 0);
        }
        let b = self.base.bounds;
        let h = self.s(POPUP_PAD) * 2 + self.items.len() as i32 * self.s(ROW_H);
        let w = b.w.max(self.s(180));
        Rect::new(b.x, b.bottom() + self.s(2), w, h)
    }

    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        let pop = self.popup_rect();
        if !pop.contains(Point { x, y }) {
            return None;
        }
        let rel = y - pop.y - self.s(POPUP_PAD);
        if rel < 0 {
            return None;
        }
        let i = (rel / self.s(ROW_H).max(1)) as usize;
        (i < self.items.len()).then_some(i)
    }

    fn pick(&mut self, i: usize, inv: &mut Invalidations) {
        if let Some(it) = self.items.get(i) {
            self.picked = Some(it.value.clone());
        }
        self.open = false;
        inv.push(self.base.bounds);
    }
}

impl Control for PullDown {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for PullDown {
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
                if self.open {
                    if let Some(i) = self.item_at(x, y) {
                        self.pick(i, inv);
                    } else {
                        self.open = false; // 바깥 클릭 = 닫기
                        inv.push(self.base.bounds);
                    }
                    return;
                }
                if self.base.bounds.contains(Point { x, y }) {
                    self.base.focused = true;
                    self.open = true;
                    self.hover = 0;
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::MouseMove { x, y } if self.open => {
                if let Some(i) = self.item_at(x, y) {
                    if i != self.hover {
                        self.hover = i;
                        inv.push(self.popup_rect());
                    }
                }
            }
            InputEvent::Key { key, .. } if self.open => match key {
                Key::Escape => {
                    self.open = false;
                    inv.push(self.base.bounds);
                }
                Key::Down => {
                    self.hover = (self.hover + 1).min(self.items.len().saturating_sub(1));
                    inv.push(self.popup_rect());
                }
                Key::Up => {
                    self.hover = self.hover.saturating_sub(1);
                    inv.push(self.popup_rect());
                }
                Key::Enter | Key::Space => {
                    let h = self.hover;
                    self.pick(h, inv);
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        // 버튼 본체(라벨 + ∨).
        ctx.fill_round_rect(b, self.s(6), theme.field_bg);
        ctx.stroke_round_rect(b, self.s(6), theme.border, 1.0);
        self.draw_focus_ring(ctx, theme, b);
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            b.x + self.s(10),
            b.y + (b.h - self.s(16)) / 2,
            b,
            &self.label,
            theme.text,
        );
        let chev = Rect::new(
            b.right() - self.s(CHEV_W) - self.s(2),
            b.y,
            self.s(CHEV_W),
            b.h,
        );
        draw_chevron_down(ctx, chev, theme.text_dim);

        if self.open {
            let pop = self.popup_rect();
            ctx.fill_round_rect(pop, self.s(8), theme.chrome_bg);
            ctx.stroke_round_rect(pop, self.s(8), theme.border, 1.0);
            let rh = self.s(ROW_H);
            let mut y = pop.y + self.s(POPUP_PAD);
            for (i, it) in self.items.iter().enumerate() {
                let row = Rect::new(pop.x + self.s(3), y, pop.w - self.s(6), rh);
                if i == self.hover {
                    ctx.fill_round_rect(row, self.s(5), theme.sel_bg);
                }
                let cy = row.y + rh / 2;
                let mut tx = row.x + self.s(10);
                if let Some(img) = it.image.as_deref() {
                    let isz = self.s(LEADING_ICON);
                    let boxr = Rect::new(tx, cy - isz / 2, isz, isz);
                    let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
                    ctx.image_scaled(fit, img, row);
                    tx += isz + self.s(3);
                }
                ctx.select_font(FontSlot::Base, false);
                ctx.text(tx, cy - self.s(16) / 2, row, &it.label, theme.text);
                y += rh;
            }
        }
        let badge = self.help_badge_rect(b);
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pd() -> (PullDown, Invalidations) {
        let mut p = PullDown::new(
            "메뉴",
            vec![
                ComboItem::new("settings", "설정"),
                ComboItem::new("gallery", "갤러리"),
            ],
        );
        let mut inv = Invalidations::default();
        p.set_bounds(Rect::new(0, 0, 100, 26), &mut inv);
        (p, inv)
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
    fn open_pick_reports_action_once() {
        let (mut p, mut inv) = pd();
        p.on_event(&click(10, 10), &mut inv);
        assert!(p.is_open());
        let pop = p.popup_rect();
        p.on_event(&click(pop.x + 20, pop.y + 4 + 26 + 5), &mut inv); // 두 번째 항목
        assert!(!p.is_open(), "선택 = 닫힘");
        assert_eq!(p.take_picked().as_deref(), Some("gallery"));
        assert!(p.take_picked().is_none(), "1회성");
    }

    #[test]
    fn outside_click_closes_without_pick() {
        let (mut p, mut inv) = pd();
        p.on_event(&click(10, 10), &mut inv);
        p.on_event(&click(500, 500), &mut inv);
        assert!(!p.is_open());
        assert!(p.take_picked().is_none());
    }

    #[test]
    fn keyboard_navigates_and_picks() {
        let (mut p, mut inv) = pd();
        p.on_event(&click(10, 10), &mut inv);
        p.on_event(
            &InputEvent::Key {
                key: Key::Down,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        p.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(p.take_picked().as_deref(), Some("gallery"));
    }
}
