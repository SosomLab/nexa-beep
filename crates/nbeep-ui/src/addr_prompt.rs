//! **주소 직접 입력 모달**(DR-19 수동 엔드포인트 · M3-16) — `host:port`/`[v6]:port`를
//! 받아 발견 없이 연결한다. 상태바 인라인 입력을 별도 창으로 승격한 것: 제대로 된 입력
//! 필드(Beam 캐럿·클립보드·선택)와 **형식 검증**, Enter 연결·Esc 취소를 갖춘다.
//!
//! 위젯은 I/O를 모른다 — 유효한 주소 문자열만 [`AddrPromptWidget::take_submit`]으로 내놓고,
//! 실제 연결(`add_endpoint`)·최근 주소 기록은 호스트 몫이다.

use crate::controls::{Button, Control as _, TextBox};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 주소 형식 검증 — `host:port` 또는 `[v6]:port`. 포트 1~65535·호스트 비어있지 않음.
/// 관대하게(호스트 문자 집합은 안 따진다 — 해석은 `to_socket_addrs` 몫) 그러나 **포트는
/// 반드시 있고 숫자여야** 오타를 즉시 잡는다.
#[must_use]
pub fn valid_endpoint(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    // [v6]:port
    if let Some(rest) = s.strip_prefix('[') {
        let Some((host, port)) = rest.split_once("]:") else {
            return false;
        };
        return !host.is_empty() && valid_port(port);
    }
    // host:port — 마지막 ':' 기준(host에 ':'가 없어야 하므로 정확히 하나).
    match s.rsplit_once(':') {
        Some((host, port)) => !host.is_empty() && !host.contains(':') && valid_port(port),
        None => false,
    }
}

fn valid_port(p: &str) -> bool {
    p.parse::<u32>().is_ok_and(|n| (1..=65_535).contains(&n))
}

/// 주소 입력 모달 위젯.
#[derive(Debug)]
pub struct AddrPromptWidget {
    bounds: Rect,
    scale: f32,
    input: TextBox,
    connect: Button,
    cancel: Button,
    submit: Option<String>,
    canceled: bool,
}

impl Default for AddrPromptWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl AddrPromptWidget {
    /// 빈 입력으로 만든다(입력에 포커스).
    #[must_use]
    pub fn new() -> Self {
        let mut input = TextBox::new("host:port · [v6]:port");
        input.set_focused(true);
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            input,
            connect: Button::new(t_connect()),
            cancel: Button::new(t_cancel()),
            submit: None,
            canceled: false,
        }
    }

    /// 확정된 유효 주소(1회성) — 호스트가 `add_endpoint`에 넘긴다.
    pub fn take_submit(&mut self) -> Option<String> {
        self.submit.take()
    }

    /// 취소 요청(1회성 · Esc·취소 버튼).
    pub fn take_cancel(&mut self) -> bool {
        std::mem::take(&mut self.canceled)
    }

    /// 현재 입력(테스트·클립보드 붙여넣기 라우팅 판단).
    #[must_use]
    pub fn input_text(&self) -> String {
        self.input.text()
    }

    /// 배율 지정 — 내부 컨트롤 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.input.set_scale(self.scale);
        self.connect.set_scale(self.scale);
        self.cancel.set_scale(self.scale);
        self.relayout(inv);
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(16);
        let field_h = self.s(30);
        // 입력 필드 — 제목 아래.
        let fy = b.y + self.s(44);
        self.input
            .set_bounds(Rect::new(b.x + pad, fy, b.w - pad * 2, field_h), inv);
        // 하단 버튼 — 우측 정렬 [취소][연결].
        let bw = self.s(88);
        let bh = self.s(28);
        let by = b.bottom() - pad - bh;
        self.connect
            .set_bounds(Rect::new(b.right() - pad - bw, by, bw, bh), inv);
        self.cancel.set_bounds(
            Rect::new(b.right() - pad - bw * 2 - self.s(8), by, bw, bh),
            inv,
        );
    }

    fn try_submit(&mut self, inv: &mut Invalidations) {
        let text = self.input.text();
        if valid_endpoint(&text) {
            self.submit = Some(text.trim().to_string());
        }
        inv.push(self.bounds);
    }
}

/// 연결/취소 라벨 — i18n 표에 별도 키를 두기보다 대화·공통에 이미 있는 어휘를 쓴다.
fn t_connect() -> &'static str {
    "연결"
}
fn t_cancel() -> &'static str {
    "취소"
}

impl Widget for AddrPromptWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.relayout(inv);
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // Enter = 연결 · Esc = 취소(입력 어디에 있든).
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
                // 입력 필드 클릭 = 포커스(그 외엔 유지 — 모달이라 항상 입력이 주인공).
                self.input
                    .set_focused(self.input.bounds().contains(Point { x, y }));
            }
            _ => {}
        }
        self.connect.on_event(ev, inv);
        self.cancel.on_event(ev, inv);
        if self.connect.take_clicked() {
            self.try_submit(inv);
            return;
        }
        if self.cancel.take_clicked() {
            self.canceled = true;
            return;
        }
        // 나머지(문자·캐럿·선택·붙여넣기)는 입력으로. Enter는 위에서 처리(TextBox의
        // take_committed와 이중 확정되지 않게 여기선 전달 안 함).
        self.input.on_event(ev, inv);
        let _ = self.input.take_committed();
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        // 제목.
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            b.x + self.s(16),
            b.y + self.s(14),
            b,
            "주소로 직접 연결 (DR-19)",
            theme.text,
        );
        self.input.paint(ctx, theme);
        // 형식 힌트 — 유효하면 회색, 비어있지 않은데 무효면 위험색으로 즉시 알린다.
        let text = self.input.text();
        ctx.select_font(FontSlot::Status, false);
        let (hint, color) = if text.trim().is_empty() {
            ("예: 192.168.0.5:47300 · [fe80::1]:47300", theme.text_dim)
        } else if valid_endpoint(&text) {
            ("Enter로 연결", theme.ok)
        } else {
            (
                "형식: host:port 또는 [v6]:port (포트 1~65535)",
                theme.danger,
            )
        };
        let sh = ctx.text_height();
        ctx.text(b.x + self.s(16), b.y + self.s(80), b, hint, color);
        let _ = sh;
        self.connect.paint(ctx, theme);
        self.cancel.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (AddrPromptWidget, Invalidations) {
        let mut w = AddrPromptWidget::new();
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 360, 150), &mut inv);
        (w, inv)
    }
    fn ch(w: &mut AddrPromptWidget, c: char, inv: &mut Invalidations) {
        w.on_event(&InputEvent::Char { c, now_ms: 0 }, inv);
    }
    fn enter(w: &mut AddrPromptWidget, inv: &mut Invalidations) {
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
    fn valid_endpoint_accepts_v4_and_v6_rejects_garbage() {
        assert!(valid_endpoint("192.168.0.5:47300"));
        assert!(valid_endpoint("host.example:22"));
        assert!(valid_endpoint("[fe80::1]:47300"));
        assert!(valid_endpoint("[::1]:1"));
        assert!(!valid_endpoint("192.168.0.5"), "포트 없음");
        assert!(!valid_endpoint("192.168.0.5:"), "빈 포트");
        assert!(!valid_endpoint(":47300"), "빈 호스트");
        assert!(!valid_endpoint("192.168.0.5:99999"), "포트 범위 초과");
        assert!(!valid_endpoint("192.168.0.5:0"), "포트 0");
        assert!(!valid_endpoint("fe80::1:47300"), "v6는 대괄호 필요");
        assert!(!valid_endpoint(""), "빈 문자열");
    }

    #[test]
    fn enter_submits_only_valid() {
        let (mut w, mut inv) = widget();
        for c in "10.0.0.1".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert!(w.take_submit().is_none(), "포트 없음 = 확정 안 됨");
        for c in ":47300".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert_eq!(w.take_submit().as_deref(), Some("10.0.0.1:47300"));
        assert!(w.take_submit().is_none(), "1회성");
    }

    #[test]
    fn escape_cancels() {
        let (mut w, mut inv) = widget();
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_cancel());
        assert!(!w.take_cancel(), "1회성");
    }

    #[test]
    fn cancel_button_cancels() {
        let (mut w, mut inv) = widget();
        let b = w.cancel.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: b.x + 3,
                y: b.y + 3,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        w.on_event(
            &InputEvent::MouseUp {
                x: b.x + 3,
                y: b.y + 3,
            },
            &mut inv,
        );
        assert!(w.take_cancel());
    }
}
