//! 텍스트 박스 — **placeholder** · char 단위 편집(캐럿·선택 [`EditState`]) · 포커스 링 · 도움말.
//!
//! 공통 기능은 [`Control`] 기본 메서드로 상속([`super`]).

use super::{image_fit_contain, Control, ControlBase};
use crate::draw::{DrawCtx, FontSlot};
use crate::edit::{EditKey, EditState};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

/// 텍스트 박스 컨트롤.
#[derive(Debug)]
pub struct TextBox {
    base: ControlBase,
    edit: EditState,
    placeholder: String,
    /// 선행 이미지 아이콘(옵션 · 투명 배경 RGBA). 있으면 placeholder·캐럿이 그 뒤로 밀린다.
    image: Option<Rc<IconImage>>,
    /// Enter 확정 1회성 보고.
    committed: bool,
    /// 내용 변경 1회성 보고.
    changed: bool,
}

impl TextBox {
    /// placeholder로 만든다(빈 값 시작).
    #[must_use]
    pub fn new(placeholder: impl Into<String>) -> Self {
        Self {
            base: ControlBase::default(),
            edit: EditState::new(),
            placeholder: placeholder.into(),
            image: None,
            committed: false,
            changed: false,
        }
    }

    /// 선행 이미지 아이콘 지정(체이닝) — placeholder·캐럿이 아이콘 뒤로 배치된다.
    #[must_use]
    pub fn with_image(mut self, image: Rc<IconImage>) -> Self {
        self.image = Some(image);
        self
    }

    /// 초기 텍스트 지정.
    #[must_use]
    pub fn with_text(mut self, text: &str) -> Self {
        self.edit = EditState::with_text(text, false);
        self
    }

    /// 현재 텍스트.
    #[must_use]
    pub fn text(&self) -> String {
        self.edit.text()
    }

    /// 텍스트 지정(보고 없음).
    pub fn set_text(&mut self, text: &str) {
        self.edit.set_text(text);
    }

    /// 내용이 바뀌었으면 새 텍스트를 꺼낸다(1회성).
    pub fn take_changed(&mut self) -> Option<String> {
        std::mem::take(&mut self.changed).then(|| self.edit.text())
    }

    /// Enter 확정되었으면 텍스트를 꺼낸다(1회성).
    pub fn take_committed(&mut self) -> Option<String> {
        std::mem::take(&mut self.committed).then(|| self.edit.text())
    }
}

impl Control for TextBox {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for TextBox {
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
                if self.base.bounds.contains(Point { x, y }) {
                    self.base.focused = true;
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::Char { c, .. } if self.base.focused => {
                if c == '\u{8}' {
                    self.edit.backspace();
                } else if !c.is_control() {
                    self.edit.insert(c);
                }
                self.changed = true;
                inv.push(self.base.bounds);
            }
            InputEvent::Key { key, shift, .. } if self.base.focused => match key {
                Key::Enter => {
                    self.committed = true;
                    inv.push(self.base.bounds);
                }
                Key::Left => self.edit.key(EditKey::Left, shift),
                Key::Right => self.edit.key(EditKey::Right, shift),
                Key::Home => self.edit.key(EditKey::Home, shift),
                Key::End => self.edit.key(EditKey::End, shift),
                _ => {}
            },
            InputEvent::SelectAll if self.base.focused => {
                self.edit.key(EditKey::SelectAll, false);
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        ctx.fill_round_rect(b, self.s(6), theme.field_bg);
        ctx.stroke_round_rect(b, self.s(6), theme.border, 1.0);
        self.draw_focus_ring(ctx, theme, b);

        let cy = b.y + b.h / 2;
        let s16 = self.s(16);
        let ty = cy - s16 / 2;
        // 선행 이미지(있으면) — placeholder·텍스트·캐럿의 시작 x를 그 뒤로 민다.
        let mut tx = b.x + self.s(10);
        if let Some(img) = self.image.as_deref() {
            let boxr = Rect::new(tx, cy - s16 / 2, s16, s16);
            let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
            ctx.image_scaled(fit, img, b);
            tx += s16 + self.s(6);
        }

        // 텍스트/placeholder는 **항상 고정 위치**(tx)에 그린다(캐럿으로 밀리지 않게).
        ctx.select_font(FontSlot::Base, false);
        let text = self.edit.text();
        if text.is_empty() {
            ctx.text(tx, ty, b, &self.placeholder, theme.text_dim);
        } else {
            ctx.text(tx, ty, b, &text, theme.text);
        }

        // 캐럿은 **별도 세로 막대**로 그린다(문자열에 '|'를 끼워 넣지 않음 → 위치 고정).
        if self.base.focused {
            let chars: Vec<char> = text.chars().collect();
            let upto: String = chars[..self.edit.caret().min(chars.len())].iter().collect();
            let cx = tx + ctx.text_width(&upto);
            ctx.fill_rect(Rect::new(cx, ty, self.s(2).max(1), s16), theme.text);
        }

        let badge = self.help_badge_rect(b);
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb() -> (TextBox, Invalidations) {
        let mut t = TextBox::new("Run command");
        let mut inv = Invalidations::default();
        t.set_bounds(Rect::new(0, 0, 260, 30), &mut inv);
        (t, inv)
    }
    fn ch(c: char) -> InputEvent {
        InputEvent::Char { c, now_ms: 0 }
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
    fn typing_requires_focus_and_reports_change() {
        let (mut t, mut inv) = tb();
        t.on_event(&ch('a'), &mut inv);
        assert_eq!(t.text(), "", "비포커스 = 무입력");
        t.on_event(&click(5, 15), &mut inv);
        assert!(t.is_focused());
        for c in "git".chars() {
            t.on_event(&ch(c), &mut inv);
        }
        assert_eq!(t.text(), "git");
        assert_eq!(t.take_changed().as_deref(), Some("git"));
    }

    #[test]
    fn enter_commits_once() {
        let (mut t, mut inv) = tb();
        t.on_event(&click(5, 15), &mut inv);
        for c in "hi".chars() {
            t.on_event(&ch(c), &mut inv);
        }
        t.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(t.take_committed().as_deref(), Some("hi"));
        assert!(t.take_committed().is_none(), "1회성");
    }

    #[test]
    fn placeholder_present_until_typed() {
        let (t, _) = tb();
        assert_eq!(t.text(), "");
        // placeholder는 렌더 전용 — 텍스트 값에는 포함되지 않는다.
    }

    #[test]
    fn backspace_edits() {
        let (mut t, mut inv) = tb();
        t.set_text("abc");
        t.base.focused = true;
        t.on_event(&ch('\u{8}'), &mut inv);
        assert_eq!(t.text(), "ab");
    }
}
