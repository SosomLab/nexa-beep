//! **한 줄 텍스트 입력 모달**(M5-1 — 그룹 이름 입력·개명. 범용).
//!
//! [`crate::addr_prompt`]와 같은 골격이되 형식 검증이 없다 — 무해화(DisplayName 등)는
//! **호스트가 제출값을 파싱**하며 한다(위젯은 문자열만 안다). Enter/확인 = 제출 ·
//! Esc/취소 = 취소, 둘 다 1회성 폴링.

use crate::controls::{Button, Control as _, TextBox};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 한 줄 입력 모달 위젯.
#[derive(Debug)]
pub struct TextPromptWidget {
    bounds: Rect,
    scale: f32,
    /// 굵은 제목(무엇을 입력하는가).
    title: String,
    input: TextBox,
    ok: Button,
    cancel: Button,
    submit: Option<String>,
    canceled: bool,
}

impl TextPromptWidget {
    /// 제목·placeholder·초기값으로 만든다(입력에 포커스 · 초기값은 전체 선택 대체 입력).
    #[must_use]
    pub fn new(title: impl Into<String>, placeholder: &str, initial: &str) -> Self {
        let mut input = TextBox::new(placeholder).with_text(initial);
        input.set_focused(true);
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            title: title.into(),
            input,
            ok: Button::new("확인"),
            cancel: Button::new("취소"),
            submit: None,
            canceled: false,
        }
    }

    /// 제출된 텍스트(1회성 · 공백 트림 — 빈 값은 제출되지 않는다).
    pub fn take_submit(&mut self) -> Option<String> {
        self.submit.take()
    }

    /// 취소 요청(1회성 · Esc·취소 버튼).
    pub fn take_cancel(&mut self) -> bool {
        std::mem::take(&mut self.canceled)
    }

    /// 배율 지정 — 내부 컨트롤 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.input.set_scale(self.scale);
        self.ok.set_scale(self.scale);
        self.cancel.set_scale(self.scale);
        self.relayout(inv);
    }

    /// 클립보드 위임(① 08-13 — 모든 텍스트 컨트롤 규칙).
    #[must_use]
    pub fn clipboard_copy(&self) -> Option<String> {
        self.input.copy_selection()
    }

    /// 선택 잘라내기.
    pub fn clipboard_cut(&mut self, inv: &mut Invalidations) -> Option<String> {
        self.input.cut_selection(inv)
    }

    /// 붙여넣기.
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        self.input.paste(text, inv);
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(16);
        let field_h = self.s(30);
        self.input.set_bounds(
            Rect::new(b.x + pad, b.y + self.s(44), b.w - pad * 2, field_h),
            inv,
        );
        let (bw, bh) = (self.s(88), self.s(28));
        let by = b.bottom() - pad - bh;
        self.ok
            .set_bounds(Rect::new(b.right() - pad - bw, by, bw, bh), inv);
        self.cancel.set_bounds(
            Rect::new(b.right() - pad - bw * 2 - self.s(8), by, bw, bh),
            inv,
        );
    }

    fn try_submit(&mut self, inv: &mut Invalidations) {
        let text = self.input.text().trim().to_string();
        if !text.is_empty() {
            self.submit = Some(text);
        }
        inv.push(self.bounds);
    }
}

impl Widget for TextPromptWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.relayout(inv);
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::Key {
                key: Key::Enter, ..
            } => {
                self.try_submit(inv);
                return;
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.canceled = true;
                return;
            }
            InputEvent::MouseDown { x, y, .. } => {
                self.input
                    .set_focused(self.input.bounds().contains(Point { x, y }));
            }
            _ => {}
        }
        self.ok.on_event(ev, inv);
        self.cancel.on_event(ev, inv);
        if self.ok.take_clicked() {
            self.try_submit(inv);
            return;
        }
        if self.cancel.take_clicked() {
            self.canceled = true;
            return;
        }
        self.input.on_event(ev, inv);
        let _ = self.input.take_committed(); // Enter는 위에서 처리(이중 확정 방지)
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        ctx.select_font(FontSlot::Base, true);
        ctx.text(
            b.x + self.s(16),
            b.y + self.s(14),
            b,
            &self.title,
            theme.text,
        );
        ctx.select_font(FontSlot::Base, false);
        self.input.paint(ctx, theme);
        self.ok.paint(ctx, theme);
        self.cancel.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enter() -> InputEvent {
        InputEvent::Key {
            key: Key::Enter,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn submit_trims_and_rejects_empty() {
        let mut w = TextPromptWidget::new("그룹 이름", "이름", "");
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 360, 150), &mut inv);
        w.on_event(&enter(), &mut inv);
        assert!(w.take_submit().is_none(), "빈 값은 제출되지 않는다");
        for c in " 개발팀 ".chars() {
            w.on_event(&InputEvent::Char { c, now_ms: 0 }, &mut inv);
        }
        w.on_event(&enter(), &mut inv);
        assert_eq!(w.take_submit().as_deref(), Some("개발팀"), "트림 제출");
        assert!(w.take_submit().is_none(), "1회성");
    }

    #[test]
    fn initial_text_and_escape_cancel() {
        let mut w = TextPromptWidget::new("이름 변경", "이름", "옛이름");
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 360, 150), &mut inv);
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_cancel());
        w.on_event(&enter(), &mut inv);
        assert_eq!(
            w.take_submit().as_deref(),
            Some("옛이름"),
            "초기값 그대로 제출 가능"
        );
    }
}
