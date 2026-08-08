//! 대화 화면 위젯 — 스레드 + 한 줄 입력(M3 첫 슬라이스 · M2 게이트 "고르면 대화가 된다"의 UI 절반).
//!
//! 메시지는 [`nbeep_core::safetext`]를 **통과한 것만** 담긴다(타입이 `SafeText` — 무해화 우회
//! 불가). 입력은 임시 한 줄 모델이다 — 캐럿·선택·조합 표시는 `edit` 이식(M3-3)에서 교체하고,
//! 지금은 IME **커밋** 문자가 들어온다(한글 입력 가능·조합 중 표시는 없음).
//!
//! 발신·복귀는 폴링(`take_outgoing`/`take_back`) — 위젯은 부모를 모른다.

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::safetext::{sanitize_message, SafeText};

/// 스레드 한 줄.
#[derive(Clone, Debug)]
pub struct ChatLine {
    /// 내가 보낸 것인가(정렬·색 구분).
    pub mine: bool,
    /// 무해화된 본문.
    pub text: SafeText,
}

/// 대화 화면 위젯.
#[derive(Debug)]
pub struct ChatViewWidget {
    bounds: Rect,
    /// 상대 표시 이름(헤더).
    title: String,
    lines: Vec<ChatLine>,
    input: String,
    scale: f32,
    outgoing: Option<SafeText>,
    back: bool,
}

impl ChatViewWidget {
    /// 상대 이름으로 빈 대화 화면을 연다.
    #[must_use]
    pub fn new(title: String) -> Self {
        Self {
            bounds: Rect::default(),
            title,
            lines: Vec::new(),
            input: String::new(),
            scale: 1.0,
            outgoing: None,
            back: false,
        }
    }

    /// 배율 지정(고DPI).
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let scale = scale.max(0.5);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            inv.push(self.bounds);
        }
    }

    /// 스레드에 줄 추가(수신·발신 확정분 — 이미 무해화된 타입만 받는다).
    pub fn push_line(&mut self, line: ChatLine, inv: &mut Invalidations) {
        self.lines.push(line);
        inv.push(self.bounds);
    }

    /// Enter로 확정된 발신 본문(1회성) — 조립 지점이 시퀀서·팬아웃에 넘긴다.
    pub fn take_outgoing(&mut self) -> Option<SafeText> {
        self.outgoing.take()
    }

    /// Esc 복귀 요청(1회성).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    /// 현재 입력 중 텍스트(테스트·HUD용).
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    fn s(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    fn input_bar(&self) -> Rect {
        let h = self.s(34);
        Rect::new(self.bounds.x, self.bounds.bottom() - h, self.bounds.w, h)
    }
}

impl Widget for ChatViewWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::Key {
                key: Key::Enter, ..
            } => {
                let trimmed = self.input.trim();
                if !trimmed.is_empty() {
                    // 발신 확정 — 무해화를 여기서 통과시킨다(스레드 타입이 SafeText).
                    self.outgoing = Some(sanitize_message(trimmed));
                    self.input.clear();
                    inv.push(self.bounds);
                }
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.back = true;
            }
            InputEvent::Char { c, .. } => {
                if c == '\u{8}' {
                    self.input.pop();
                } else if !c.is_control() {
                    self.input.push(c);
                }
                inv.push(self.input_bar());
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        // 헤더.
        let head_h = self.s(30);
        let head = Rect::new(self.bounds.x, self.bounds.y, self.bounds.w, head_h);
        ctx.select_font(FontSlot::Base, false);
        ctx.text_opaque(
            head.x + self.s(12),
            head.y + self.s(7),
            head,
            &self.title,
            theme.text,
            theme.chrome_bg,
        );

        // 스레드 — 아래부터 최신(마지막 줄이 입력창 위).
        ctx.select_font(FontSlot::Message, false);
        let line_h = self.s(24);
        let input = self.input_bar();
        let mut y = input.y - self.s(6) - line_h;
        for line in self.lines.iter().rev() {
            if y < head.bottom() {
                break; // 화면 밖 — 스크롤은 후속 슬라이스
            }
            let fg = if line.mine { theme.text } else { theme.accent };
            let prefix = if line.mine { "나: " } else { "상대: " };
            let text = format!("{prefix}{}", line.text.as_str());
            let clip = Rect::new(self.bounds.x, y, self.bounds.w, line_h);
            ctx.text(self.bounds.x + self.s(12), y, clip, &text, fg);
            y -= line_h;
        }

        // 입력창.
        ctx.fill_rect(input, theme.field_bg);
        ctx.fill_rect(
            Rect::new(input.x, input.y, input.w, self.s(1)),
            theme.border,
        );
        ctx.select_font(FontSlot::Message, false);
        let shown = if self.input.is_empty() {
            "메시지 입력… (Enter 전송 · Esc 목록)"
        } else {
            &self.input
        };
        let fg = if self.input.is_empty() {
            theme.text_dim
        } else {
            theme.text
        };
        ctx.text(input.x + self.s(10), input.y + self.s(7), input, shown, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (ChatViewWidget, Invalidations) {
        let mut w = ChatViewWidget::new("김철수".into());
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 400, 300), &mut inv);
        (w, inv)
    }
    fn ch(w: &mut ChatViewWidget, c: char, inv: &mut Invalidations) {
        w.on_event(&InputEvent::Char { c, now_ms: 0 }, inv);
    }
    fn enter(w: &mut ChatViewWidget, inv: &mut Invalidations) {
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            inv,
        );
    }

    #[test]
    fn typing_and_enter_produces_sanitized_outgoing() {
        let (mut w, mut inv) = widget();
        for c in "안녕 bob".chars() {
            ch(&mut w, c, &mut inv);
        }
        assert_eq!(w.input(), "안녕 bob");
        enter(&mut w, &mut inv);
        let out = w.take_outgoing().expect("발신 확정");
        assert_eq!(out.as_str(), "안녕 bob");
        assert_eq!(w.input(), "", "전송 후 입력창 비움");
        assert!(w.take_outgoing().is_none(), "1회성");
    }

    #[test]
    fn outgoing_is_sanitized_rlo_stripped() {
        // 발신 경로도 무해화 — RLO를 타이핑(붙여넣기)해도 스레드 타입에 못 들어간다.
        let (mut w, mut inv) = widget();
        for c in "a\u{202E}b".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert_eq!(w.take_outgoing().unwrap().as_str(), "ab");
    }

    #[test]
    fn empty_or_whitespace_input_does_not_send() {
        let (mut w, mut inv) = widget();
        enter(&mut w, &mut inv);
        assert!(w.take_outgoing().is_none());
        ch(&mut w, ' ', &mut inv);
        enter(&mut w, &mut inv);
        assert!(w.take_outgoing().is_none(), "공백만 = 미전송");
    }

    #[test]
    fn backspace_edits_and_escape_requests_back() {
        let (mut w, mut inv) = widget();
        ch(&mut w, 'h', &mut inv);
        ch(&mut w, 'i', &mut inv);
        ch(&mut w, '\u{8}', &mut inv);
        assert_eq!(w.input(), "h");
        assert!(!w.take_back());
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_back(), "Esc = 복귀 요청");
        assert!(!w.take_back(), "1회성");
    }
}
