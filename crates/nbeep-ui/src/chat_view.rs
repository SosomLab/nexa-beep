//! 대화 화면 위젯 — 스레드 + 한 줄 입력(M3 첫 슬라이스 · M2 게이트 "고르면 대화가 된다"의 UI 절반).
//!
//! 메시지는 [`nbeep_core::safetext`]를 **통과한 것만** 담긴다(타입이 `SafeText` — 무해화 우회
//! 불가). 입력은 [`crate::edit::EditState`](캐럿·선택·char 단위) + **IME 프리에딧**(조합 중 밑줄
//! 표시 — M3-3). 확정 문자는 `Char`로, 조합 중 텍스트는 [`ChatViewWidget::set_preedit`]로 온다.
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
    input: crate::edit::EditState,
    /// IME 조합 중 텍스트(확정 전 — 밑줄 표시. 확정은 input에 삽입).
    preedit: String,
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
            input: crate::edit::EditState::new(),
            preedit: String::new(),
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
    pub fn input(&self) -> String {
        self.input.text()
    }

    /// 입력 캐럿 위치(문자 인덱스) — 커서 렌더용.
    #[must_use]
    pub fn input_caret(&self) -> usize {
        self.input.caret()
    }

    /// IME 조합 중 텍스트 설정(빈 문자열 = 조합 종료·취소). 확정은 `on_event`의 Char로.
    pub fn set_preedit(&mut self, text: String, inv: &mut Invalidations) {
        self.preedit = text;
        inv.push(self.input_bar());
    }

    /// 조합 중 텍스트(테스트·렌더).
    #[must_use]
    pub fn preedit(&self) -> &str {
        &self.preedit
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
        use crate::edit::EditKey;
        match *ev {
            InputEvent::Key {
                key: Key::Enter, ..
            } => {
                let text = self.input.text();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    // 발신 확정 — 무해화를 여기서 통과시킨다(스레드 타입이 SafeText).
                    self.outgoing = Some(sanitize_message(trimmed));
                    self.input.set_text("");
                    inv.push(self.bounds);
                }
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.back = true;
            }
            InputEvent::Key { key, shift, .. } => {
                // 캐럿 이동·선택(edit 모델). Enter/Esc는 위에서 처리됨.
                let ek = match key {
                    Key::Left => Some(EditKey::Left),
                    Key::Right => Some(EditKey::Right),
                    Key::Home => Some(EditKey::Home),
                    Key::End => Some(EditKey::End),
                    _ => None,
                };
                if let Some(ek) = ek {
                    self.input.key(ek, shift);
                    inv.push(self.input_bar());
                }
            }
            InputEvent::SelectAll => {
                self.input.key(EditKey::SelectAll, false);
                inv.push(self.input_bar());
            }
            InputEvent::Char { c, .. } => {
                self.preedit.clear(); // 확정 문자 도착 = 조합 종료
                if c == '\u{8}' {
                    self.input.backspace();
                } else if !c.is_control() {
                    self.input.insert(c);
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
        let text = self.input.text();
        let tx = input.x + self.s(10);
        let ty = input.y + self.s(7);
        if text.is_empty() && self.preedit.is_empty() {
            ctx.text(
                tx,
                ty,
                input,
                "메시지 입력… (Enter 전송 · Esc 목록)",
                theme.text_dim,
            );
        } else {
            let chars: Vec<char> = text.chars().collect();
            let upto = |ctx: &mut dyn DrawCtx, n: usize| -> i32 {
                let prefix: String = chars[..n].iter().collect();
                ctx.text_width(&prefix)
            };
            // 선택 하이라이트(텍스트 뒤에 먼저).
            if let Some((a, b)) = self.input.selection() {
                let x0 = tx + upto(ctx, a);
                let x1 = tx + upto(ctx, b);
                ctx.fill_rect(
                    Rect::new(
                        x0,
                        input.y + self.s(4),
                        (x1 - x0).max(1),
                        input.h - self.s(8),
                    ),
                    theme.sel_bg,
                );
            }
            ctx.text(tx, ty, input, &text, theme.text);
            let cx = tx + upto(ctx, self.input.caret());
            if self.preedit.is_empty() {
                // 폰트 실측 픽셀 커서 — 캐럿까지 폭만큼 오른쪽에 세로선.
                ctx.fill_rect(
                    Rect::new(
                        cx,
                        input.y + self.s(6),
                        self.s(2).max(1),
                        input.h - self.s(12),
                    ),
                    theme.accent,
                );
            } else {
                // IME 조합 중 — 캐럿 위치에 프리에딧을 accent 색 + 밑줄로(확정 전).
                ctx.text(cx, ty, input, &self.preedit, theme.accent);
                let pw = ctx.text_width(&self.preedit);
                ctx.fill_rect(
                    Rect::new(cx, ty + self.s(16), pw.max(1), self.s(2).max(1)),
                    theme.accent,
                );
            }
        }
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

    fn key(w: &mut ChatViewWidget, k: Key, inv: &mut Invalidations) {
        w.on_event(
            &InputEvent::Key {
                key: k,
                shift: false,
                primary: false,
            },
            inv,
        );
    }

    #[test]
    fn preedit_shown_then_commit_inserts() {
        let (mut w, mut inv) = widget();
        // 조합 중 "한" 프리에딧 표시(확정 아님 — input엔 없음).
        w.set_preedit("한".into(), &mut inv);
        assert_eq!(w.preedit(), "한");
        assert_eq!(w.input(), "", "조합 중엔 확정 텍스트 없음");
        // 확정(Commit) = Char 도착 → 프리에딧 클리어 + input 삽입.
        ch(&mut w, '한', &mut inv);
        assert_eq!(w.preedit(), "", "확정 시 조합 종료");
        assert_eq!(w.input(), "한");
    }

    #[test]
    fn empty_preedit_ends_composition() {
        let (mut w, mut inv) = widget();
        w.set_preedit("ㅎ".into(), &mut inv);
        w.set_preedit(String::new(), &mut inv); // 조합 취소
        assert_eq!(w.preedit(), "");
    }

    #[test]
    fn caret_moves_and_inserts_mid_string() {
        let (mut w, mut inv) = widget();
        for c in "helo".chars() {
            ch(&mut w, c, &mut inv);
        }
        // 캐럿을 'l'과 'o' 사이로: End에서 Left 1회 → "hel|o"에 'l' 삽입 → "hello"
        key(&mut w, Key::Left, &mut inv);
        ch(&mut w, 'l', &mut inv);
        assert_eq!(w.input(), "hello");
        assert_eq!(w.input_caret(), 4);
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
