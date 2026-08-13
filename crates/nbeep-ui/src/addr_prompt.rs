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

/// 세션 기본 포트 — 포트를 생략한 입력에 붙인다(`nbeep_net::DEFAULT_SESSION_PORT`와 같은 값).
///
/// ⚠️ UI 크레이트는 `net`에 의존하지 않으므로([docs/01] 단방향 의존) 값을 여기 다시 둔다.
/// 두 값이 갈리면 안 되므로 테스트가 문서화된 값(47200)을 고정한다.
/// 설정 `net.session_port`(듣는 포트 = 거는 기본 포트 — 08-13 ⓐ)를 바꾸면 호스트가
/// [`AddrPromptWidget::new`]에 그 값을 넘긴다 — 이 상수는 미배선 경로의 폴백.
pub const DEFAULT_PORT: u16 = 47_200;

/// 주소를 정규화한다 — **포트를 생략하면 `default_port`를 붙인다.**
///
/// 사용자가 `10.60.218.157`만 넣어도 연결되게 하는 것이 목적이다(2026-08-13 사용자 요구).
/// 포트를 적었으면 그대로 두고, 형식이 틀렸으면 `None`.
///
/// ```
/// # use nbeep_ui::addr_prompt::{normalize_endpoint, DEFAULT_PORT};
/// assert_eq!(normalize_endpoint("10.0.0.1", DEFAULT_PORT).as_deref(), Some("10.0.0.1:47200"));
/// assert_eq!(normalize_endpoint("10.0.0.1:9000", DEFAULT_PORT).as_deref(), Some("10.0.0.1:9000"));
/// assert_eq!(normalize_endpoint("[fe80::1]", 48000).as_deref(), Some("[fe80::1]:48000"));
/// assert_eq!(normalize_endpoint("", DEFAULT_PORT), None);
/// ```
#[must_use]
pub fn normalize_endpoint(s: &str, default_port: u16) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // [v6] 또는 [v6]:port
    if let Some(rest) = s.strip_prefix('[') {
        if let Some((host, port)) = rest.split_once("]:") {
            return (!host.is_empty() && valid_port(port)).then(|| s.to_string());
        }
        // 포트 없는 [v6]
        let host = rest.strip_suffix(']')?;
        return (!host.is_empty()).then(|| format!("[{host}]:{default_port}"));
    }
    // 대괄호 없는 v6(':'가 둘 이상) — 포트를 붙이려면 대괄호가 필요하다.
    if s.matches(':').count() >= 2 {
        return None;
    }
    match s.rsplit_once(':') {
        // host:port
        Some((host, port)) => (!host.is_empty() && valid_port(port)).then(|| s.to_string()),
        // 포트 생략 — 기본 포트를 붙인다.
        None => Some(format!("{s}:{default_port}")),
    }
}

/// 주소 형식 검증 — `host[:port]` 또는 `[v6][:port]`. **포트는 생략 가능**(기본 포트 사용).
/// 호스트 문자 집합은 안 따진다(해석은 `to_socket_addrs` 몫) — 그러나 포트를 **적었다면**
/// 숫자·1~65535여야 오타를 즉시 잡는다.
#[must_use]
pub fn valid_endpoint(s: &str) -> bool {
    // 유효성은 기본 포트 값과 무관하다(붙는 값만 달라진다).
    normalize_endpoint(s, DEFAULT_PORT).is_some()
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
    /// 포트 생략 시 붙일 기본 포트 — 설정 `net.session_port`(듣는 포트와 같은 값 · ⓐ).
    default_port: u16,
}

impl Default for AddrPromptWidget {
    fn default() -> Self {
        Self::new(DEFAULT_PORT)
    }
}

impl AddrPromptWidget {
    /// 빈 입력으로 만든다(입력에 포커스). `default_port` = 포트 생략 시 붙일 값
    /// (호스트가 설정 `net.session_port`를 넘긴다 — 듣는 포트 = 거는 기본 포트).
    #[must_use]
    pub fn new(default_port: u16) -> Self {
        let mut input = TextBox::new("host 또는 host:port · [v6]:port");
        input.set_focused(true);
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            input,
            connect: Button::new(t_connect()),
            cancel: Button::new(t_cancel()),
            submit: None,
            canceled: false,
            default_port,
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

    /// IME 조합 중 문자열(08-13 — 모든 텍스트 컨트롤 규칙: 확정 전에도 보인다).
    pub fn set_preedit(&mut self, text: &str, inv: &mut Invalidations) {
        self.input.set_preedit(text, inv);
    }

    /// 우클릭 편집 메뉴 행동(1회성 — 08-13 전수 검사).
    pub fn take_edit_ctx(&mut self) -> Option<crate::controls::EditCtxAction> {
        self.input.take_edit_ctx()
    }

    /// 클립보드 텍스트 유무 주입(우클릭 시점 — 붙여넣기 항목 활성 근거).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.input.set_clipboard_has_text(yes);
    }

    /// 선택 복사(① 08-13) — OS 클립보드 쓰기는 호스트 몫.
    #[must_use]
    pub fn clipboard_copy(&self) -> Option<String> {
        self.input.copy_selection()
    }

    /// 선택 잘라내기(①).
    pub fn clipboard_cut(&mut self, inv: &mut Invalidations) -> Option<String> {
        self.input.cut_selection(inv)
    }

    /// 붙여넣기(① — 호스트가 읽은 텍스트).
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        self.input.paste(text, inv);
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
        // ★ 포트를 생략했으면 여기서 기본 포트가 붙는다 — 호스트는 완성된 주소만 본다.
        // 형식이 틀리면 `None`이라 제출되지 않는다(검증과 정규화가 한 함수 = 판정이 갈리지 않는다).
        if let Some(addr) = normalize_endpoint(&text, self.default_port) {
            self.submit = Some(addr);
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
        // 단 우클릭 메뉴가 열려 있으면 Enter/Esc는 메뉴 몫 — 입력 전달로 흘린다.
        match *ev {
            InputEvent::Key {
                key: Key::Enter, ..
            } if !self.input.popup_open() => {
                self.try_submit(inv);
                return;
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } if !self.input.popup_open() => {
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
        let p = self.default_port;
        let (hint, color) = if text.trim().is_empty() {
            (
                format!("예: 10.0.0.5 (포트 생략 시 {p}) · 10.0.0.5:{p} · [fe80::1]:{p}"),
                theme.text_dim,
            )
        } else if valid_endpoint(&text) {
            ("Enter로 연결".to_string(), theme.ok)
        } else {
            (
                "형식: host:port 또는 [v6]:port (포트 1~65535)".to_string(),
                theme.danger,
            )
        };
        ctx.text(b.x + self.s(16), b.y + self.s(80), b, &hint, color);
        self.connect.paint(ctx, theme);
        self.cancel.paint(ctx, theme);
        self.input.paint_popup(ctx, theme); // 우클릭 메뉴 — 힌트·버튼 위로(최상위)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (AddrPromptWidget, Invalidations) {
        let mut w = AddrPromptWidget::new(DEFAULT_PORT);
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
        assert!(valid_endpoint("192.168.0.5"), "포트 생략 = 기본 포트");
        assert!(!valid_endpoint("192.168.0.5:"), "빈 포트");
        assert!(!valid_endpoint(":47300"), "빈 호스트");
        assert!(!valid_endpoint("192.168.0.5:99999"), "포트 범위 초과");
        assert!(!valid_endpoint("192.168.0.5:0"), "포트 0");
        assert!(!valid_endpoint("fe80::1:47300"), "v6는 대괄호 필요");
        assert!(!valid_endpoint(""), "빈 문자열");
    }

    #[test]
    fn omitted_port_gets_default() {
        // 사용자 요구(08-13) — IP만 넣어도 연결되어야 한다.
        assert_eq!(
            normalize_endpoint("10.60.218.157", DEFAULT_PORT).as_deref(),
            Some("10.60.218.157:47200")
        );
        assert_eq!(
            normalize_endpoint("host.example", DEFAULT_PORT).as_deref(),
            Some("host.example:47200")
        );
        assert_eq!(
            normalize_endpoint("[fe80::1]", DEFAULT_PORT).as_deref(),
            Some("[fe80::1]:47200")
        );
    }

    #[test]
    fn explicit_port_is_kept() {
        assert_eq!(
            normalize_endpoint("10.0.0.1:9000", DEFAULT_PORT).as_deref(),
            Some("10.0.0.1:9000")
        );
        assert_eq!(
            normalize_endpoint("[fe80::1]:9000", DEFAULT_PORT).as_deref(),
            Some("[fe80::1]:9000")
        );
    }

    #[test]
    fn bare_v6_without_brackets_is_rejected() {
        // 대괄호가 없으면 마지막 ':'가 포트인지 v6 구분자인지 알 수 없다 — 추측하지 않는다.
        assert_eq!(normalize_endpoint("fe80::1", DEFAULT_PORT), None);
        assert_eq!(normalize_endpoint("::1", DEFAULT_PORT), None);
    }

    #[test]
    fn custom_default_port_is_used_on_omission() {
        // 설정 `net.session_port`를 바꾸면(ⓐ — 듣는 포트 = 거는 기본 포트) 생략 입력에
        // 그 값이 붙어야 한다. 명시 포트는 설정과 무관하게 유지.
        assert_eq!(
            normalize_endpoint("10.0.0.1", 48123).as_deref(),
            Some("10.0.0.1:48123")
        );
        assert_eq!(
            normalize_endpoint("10.0.0.1:9000", 48123).as_deref(),
            Some("10.0.0.1:9000")
        );
        let mut w = AddrPromptWidget::new(48123);
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 360, 150), &mut inv);
        for c in "10.0.0.7".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert_eq!(w.take_submit().as_deref(), Some("10.0.0.7:48123"));
    }

    #[test]
    fn default_port_matches_documented_value() {
        // net 크레이트의 DEFAULT_SESSION_PORT와 같아야 한다(의존 방향상 상수를 공유할 수 없다).
        assert_eq!(DEFAULT_PORT, 47_200);
    }

    #[test]
    fn submit_carries_normalized_address() {
        let (mut w, mut inv) = widget();
        for c in "10.0.0.1".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert_eq!(w.take_submit().as_deref(), Some("10.0.0.1:47200"));
    }

    #[test]
    fn enter_submits_only_valid() {
        // ⚠️ 정정(08-13) — "포트 없음"은 이제 **유효**하다(기본 포트가 붙는다).
        // 이 테스트가 지키는 것은 "형식이 틀린 것은 확정되지 않는다"로 좁혀졌다.
        let (mut w, mut inv) = widget();
        for c in "10.0.0.1:99999".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert!(w.take_submit().is_none(), "포트 범위 초과 = 확정 안 됨");
        // 새 위젯으로 유효한 값 확인(지우기 키는 이 테스트의 관심사가 아니다).
        let (mut w, mut inv) = widget();
        for c in "10.0.0.1:47200".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert_eq!(w.take_submit().as_deref(), Some("10.0.0.1:47200"));
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
