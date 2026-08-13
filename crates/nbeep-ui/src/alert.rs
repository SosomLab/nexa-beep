//! **경고 모달** — 제목 + 본문(자동 줄바꿈) + 확인 버튼 하나(08-13 사용자 요청).
//!
//! 상태바 한 줄로는 지나치는 실패(파일 전송 자격 미달 등)를 **사람 눈앞에 세운다**.
//! 위젯은 이유 문자열만 보여 준다 — 판정(정책)은 호스트 몫([`crate::widget`] 통지 모델).
//! 닫힘은 [`AlertWidget::take_closed`] 폴링(Enter·Esc·확인 버튼 전부 같은 길).

use crate::controls::{Button, Control as _};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 경고 모달 위젯.
#[derive(Debug)]
pub struct AlertWidget {
    bounds: Rect,
    scale: f32,
    /// 굵은 제목 한 줄.
    title: String,
    /// 본문 — 폭에 맞춰 문자 단위로 접는다(한국어는 공백 워드랩이 안 통한다).
    message: String,
    ok: Button,
    closed: bool,
}

impl AlertWidget {
    /// 제목·본문으로 만든다.
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            title: title.into(),
            message: message.into(),
            ok: Button::new("확인"),
            closed: false,
        }
    }

    /// 내용 교체(이미 열려 있는 창 재사용 — 창을 또 띄우지 않는다).
    pub fn set_content(&mut self, title: &str, message: &str, inv: &mut Invalidations) {
        self.title = title.to_string();
        self.message = message.to_string();
        inv.push(self.bounds);
    }

    /// 닫힘 요청(1회성 · Enter·Esc·확인).
    pub fn take_closed(&mut self) -> bool {
        std::mem::take(&mut self.closed)
    }

    /// 배율 지정 — 내부 컨트롤 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.ok.set_scale(self.scale);
        self.relayout(inv);
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let (bw, bh) = (self.s(88), self.s(28));
        let pad = self.s(16);
        self.ok.set_bounds(
            Rect::new(b.right() - pad - bw, b.bottom() - pad - bh, bw, bh),
            inv,
        );
    }
}

impl Widget for AlertWidget {
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
                key: Key::Enter | Key::Escape,
                ..
            } => {
                self.closed = true;
                return;
            }
            _ => {}
        }
        self.ok.on_event(ev, inv);
        if self.ok.take_clicked() {
            self.closed = true;
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let pad = self.s(16);
        // 제목(굵게).
        ctx.select_font(FontSlot::Base, true);
        ctx.text(b.x + pad, b.y + self.s(14), b, &self.title, theme.text);
        // 본문 — 문자 단위 그리디 줄바꿈(CJK는 공백이 없다 · 설정 desc 워드랩과 같은 이유).
        ctx.select_font(FontSlot::Base, false);
        let avail = (b.w - pad * 2).max(self.s(40));
        let lh = ctx.text_height() + self.s(4);
        let mut y = b.y + self.s(44);
        let mut line = String::new();
        let flush_bottom = self.ok.bounds().y - lh; // 버튼 위까지만
        for c in self.message.chars() {
            if c == '\n' || ctx.text_width(&format!("{line}{c}")) > avail {
                ctx.text(b.x + pad, y, b, &line, theme.text);
                y += lh;
                line.clear();
                if y > flush_bottom {
                    break; // 넘치면 자른다 — 모달은 요지 전달용(전문은 상태바·로그)
                }
            }
            if c != '\n' {
                line.push(c);
            }
        }
        if !line.is_empty() && y <= flush_bottom {
            ctx.text(b.x + pad, y, b, &line, theme.text);
        }
        self.ok.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_escape_and_button_all_close_once() {
        let mut w = AlertWidget::new("제목", "본문");
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 400, 170), &mut inv);
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_closed());
        assert!(!w.take_closed(), "1회성");
        // 확인 버튼 클릭도 같은 길.
        let r = w.ok.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 3,
                y: r.y + 3,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        w.on_event(
            &InputEvent::MouseUp {
                x: r.x + 3,
                y: r.y + 3,
            },
            &mut inv,
        );
        assert!(w.take_closed());
    }

    #[test]
    fn content_can_be_replaced() {
        let mut w = AlertWidget::new("a", "b");
        let mut inv = Invalidations::default();
        w.set_content("새 제목", "새 본문", &mut inv);
        assert_eq!(w.title, "새 제목");
        assert_eq!(w.message, "새 본문");
    }
}
