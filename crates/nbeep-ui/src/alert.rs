//! **경고 모달** — 제목 + 본문(자동 줄바꿈) + 확인 버튼 하나(08-13 사용자 요청).
//!
//! 상태바 한 줄로는 지나치는 실패(파일 전송 자격 미달 등)를 **사람 눈앞에 세운다**.
//! 위젯은 이유 문자열만 보여 준다 — 판정(정책)은 호스트 몫([`crate::widget`] 통지 모델).
//! 닫힘은 [`AlertWidget::take_closed`] 폴링(Enter·Esc·확인 버튼 전부 같은 길).

use crate::controls::{Button, Checkbox, Control as _};
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
    /// 두 번째 버튼(선택 모드 — 초대 수락/거절 등). None = 확인 하나(기존 경고).
    no: Option<Button>,
    /// 세 번째 버튼(S3 · DR-30 "나중에") — 결정 없이 닫는다(초대는 대기로 남는다). 있으면 Esc도 같은 뜻.
    third: Option<Button>,
    /// 체크박스 한 줄(S3 · "이 그룹 초대 다시 받지 않기" · 기본 꺼짐) — 버튼 줄 위. 값은 [`AlertWidget::is_checked`].
    check: Option<Checkbox>,
    closed: bool,
    /// 선택 결과(1회성 · 선택 모드에서만) — true = 긍정(수락).
    choice: Option<bool>,
    /// ★ M3-4(11-06) 포커스된 버튼(0 = 긍정/확인 · 1 = 부정 · 2 = 나중에) — Tab/Shift+Tab/←/→ 순회 · Enter/Space = 그 버튼.
    /// 포커스 링은 Button이 그린다(docs/14 §3-4 "포커스 링은 항상 보인다").
    focus: usize,
    /// 상태 목록 모드(08-15 — 그룹 구성원): 줄 앞에 **목록과 같은 팔레트**
    /// ([`crate::peer_list::link_color`])의 상태 점을 그린다. 비면 본문 워드랩.
    status_list: Vec<(crate::peer_list::LinkState, String)>,
    /// 상태 목록 행 클릭 허용(G4 — 소유자의 구성원 편집 진입). 기본 꺼짐.
    rows_clickable: bool,
    /// 클릭된 행(1회성 — 호스트 폴 · 모달은 닫지 않는다).
    row_clicked: Option<usize>,
    /// 상태 점 실루엣 구분(M3-19 — 목록 배지와 같은 문법 · `ui.link_badge_shape`).
    badge_shape: bool,
}

impl AlertWidget {
    /// 제목·본문으로 만든다.
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        let mut w = Self {
            bounds: Rect::default(),
            scale: 1.0,
            title: title.into(),
            message: message.into(),
            ok: Button::new(nbeep_core::t(nbeep_core::Msg::ActOk)),
            no: None,
            third: None,
            check: None,
            closed: false,
            choice: None,
            focus: 0,
            status_list: Vec::new(),
            rows_clickable: false,
            row_clicked: None,
            badge_shape: true,
        };
        w.apply_focus(); // 확인 하나짜리도 포커스 링(docs/14 §3-4 "항상 보인다" · 11-06)
        w
    }

    /// 상태 점 실루엣 on/off(M3-19 · `ui.link_badge_shape` — 호스트가 주입).
    pub fn set_badge_shape(&mut self, on: bool) {
        self.badge_shape = on;
    }

    /// 상태 목록 행 클릭 허용 지정(G4 — 소유자만 켠다).
    pub fn set_rows_clickable(&mut self, on: bool) {
        self.rows_clickable = on;
    }

    /// 클릭된 행 인덱스(1회성) — 호스트가 문맥(구성원 순서)으로 해석한다.
    pub fn take_row_clicked(&mut self) -> Option<usize> {
        self.row_clicked.take()
    }

    /// 상태 목록 지정(그룹 구성원 — 08-15). 본문(message) 대신 이 줄들이 그려진다.
    pub fn set_status_list(
        &mut self,
        lines: Vec<(crate::peer_list::LinkState, String)>,
        inv: &mut Invalidations,
    ) {
        self.status_list = lines;
        inv.push(self.bounds);
    }

    /// **선택 모달**(M5-1g — 그룹 초대 수락/거절 등): 긍정/부정 두 버튼.
    /// Enter = 긍정 · Esc = 부정. 결과는 [`AlertWidget::take_choice`].
    #[must_use]
    pub fn with_choice(mut self, yes: &str, no: &str) -> Self {
        self.ok = Button::new(yes);
        self.no = Some(Button::new(no));
        self.apply_focus();
        self
    }

    /// 포커스 가능한 버튼 수(확인/긍정 · 부정 · 나중에).
    fn focus_count(&self) -> usize {
        1 + usize::from(self.no.is_some()) + usize::from(self.third.is_some())
    }

    /// 포커스 인덱스를 버튼 상태에 반영(링은 Button이 그린다).
    fn apply_focus(&mut self) {
        let f = self.focus.min(self.focus_count() - 1);
        self.focus = f;
        self.ok.set_focused(f == 0);
        if let Some(no) = &mut self.no {
            no.set_focused(f == 1);
        }
        if let Some(t) = &mut self.third {
            t.set_focused(f == 2);
        }
    }

    /// 포커스된 버튼을 누른다(Enter/Space) — 0 = 긍정 · 1 = 부정 · 2 = 결정 없음.
    fn activate_focused(&mut self) {
        match self.focus {
            0 => {
                if self.no.is_some() {
                    self.choice = Some(true);
                }
            }
            1 => self.choice = Some(false),
            _ => {}
        }
        self.closed = true;
    }

    /// ★ 위험 동작의 기본 버튼 = **취소**(docs/14 §3-4 · ADR-0004 §7 "Enter 연타로 통과되지 않게" · 11-06): 초기 포커스를
    /// 부정 버튼에 둔다(2택 이상일 때만). Enter/Space는 포커스된 버튼이므로 Enter = 취소 · 긍정은 Tab/→ 뒤 Enter 또는 클릭.
    #[must_use]
    pub fn with_default_no(mut self) -> Self {
        if self.no.is_some() {
            self.focus = 1;
            self.apply_focus();
        }
        self
    }

    /// 세 번째 버튼(S3 · "나중에") — 결정 없이 닫기(= X로 닫기와 같은 결과 · `take_choice` = None). Esc도 이 뜻이 된다.
    #[must_use]
    pub fn with_third(mut self, label: &str) -> Self {
        self.third = Some(Button::new(label));
        self.apply_focus();
        self
    }

    /// 체크박스 한 줄(S3 · 기본 꺼짐) — 버튼 줄 위에 놓인다(호스트는 창 높이를 30 더 준다).
    #[must_use]
    pub fn with_checkbox(mut self, label: &str) -> Self {
        self.check = Some(Checkbox::new(label, false));
        self
    }

    /// 체크박스 상태(없으면 false) — 닫힐 때 호스트가 읽는다.
    #[must_use]
    pub fn is_checked(&self) -> bool {
        self.check.as_ref().is_some_and(Checkbox::is_checked)
    }

    /// 선택 결과(1회성 · 선택 모달에서만) — `true` = 긍정. 닫힘과 함께 소비한다.
    pub fn take_choice(&mut self) -> Option<bool> {
        self.choice.take()
    }

    /// 내용 교체(이미 열려 있는 창 재사용 — 창을 또 띄우지 않는다).
    /// 상태 목록 모드는 해제된다(다음 set_status_list까지 본문 모드).
    pub fn set_content(&mut self, title: &str, message: &str, inv: &mut Invalidations) {
        self.title = title.to_string();
        self.message = message.to_string();
        self.status_list.clear();
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
        if let Some(no) = &mut self.no {
            no.set_scale(self.scale);
        }
        if let Some(t) = &mut self.third {
            t.set_scale(self.scale);
        }
        if let Some(ch) = &mut self.check {
            ch.set_scale(self.scale);
        }
        self.relayout(inv);
    }

    /// 본문·목록이 내려갈 수 있는 바닥(버튼 줄 위 · 체크박스가 있으면 그 위).
    fn content_bottom(&self) -> i32 {
        self.check
            .as_ref()
            .map_or(self.ok.bounds().y, |c| c.bounds().y)
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 상태 목록 행 높이(페인트·히트 공유 — 어긋나면 클릭이 빗나간다).
    fn row_h(&self) -> i32 {
        self.s(24)
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let (bw, bh) = (self.s(88), self.s(28));
        let pad = self.s(16);
        self.ok.set_bounds(
            Rect::new(b.right() - pad - bw, b.bottom() - pad - bh, bw, bh),
            inv,
        );
        let gap = self.s(8);
        if let Some(no) = &mut self.no {
            no.set_bounds(
                Rect::new(
                    b.right() - pad - bw * 2 - gap,
                    b.bottom() - pad - bh,
                    bw,
                    bh,
                ),
                inv,
            );
        }
        if let Some(t) = &mut self.third {
            t.set_bounds(
                Rect::new(
                    b.right() - pad - bw * 3 - gap * 2,
                    b.bottom() - pad - bh,
                    bw,
                    bh,
                ),
                inv,
            );
        }
        let h = self.s(22);
        if let Some(ch) = &mut self.check {
            ch.set_bounds(
                Rect::new(
                    b.x + pad,
                    b.bottom() - pad - bh - gap - h,
                    (b.w - pad * 2).max(0),
                    h,
                ),
                inv,
            );
        }
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
        // 상태 목록 행 클릭(G4 · 08-15) — 확인 버튼보다 먼저 판정(행이 위에 있다).
        if self.rows_clickable && !self.status_list.is_empty() {
            if let InputEvent::MouseDown { x, y, .. } = *ev {
                let p = crate::geom::Point { x, y };
                let lh = self.row_h();
                let top = self.bounds.y + self.s(44);
                for i in 0..self.status_list.len() {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    let r = Rect::new(
                        self.bounds.x + self.s(12),
                        top + lh * i as i32,
                        self.bounds.w - self.s(24),
                        lh,
                    );
                    if r.contains(p) {
                        self.row_clicked = Some(i);
                        inv.push(self.bounds);
                        return;
                    }
                }
            }
        }
        // ★ M3-4(11-06) 키보드 순회: Tab/Shift+Tab · ←/→ = 버튼 포커스 이동 · Enter/Space = 포커스된 버튼(종전 = Enter 항상 긍정).
        if let InputEvent::Key {
            key: Key::Tab,
            shift,
            ..
        } = *ev
        {
            let n = self.focus_count();
            self.focus = if shift {
                (self.focus + n - 1) % n
            } else {
                (self.focus + 1) % n
            };
            self.apply_focus();
            inv.push(self.bounds);
            return;
        }
        if let InputEvent::Key {
            key: Key::Left | Key::Right,
            ..
        } = *ev
        {
            let n = self.focus_count();
            let left = matches!(ev, InputEvent::Key { key: Key::Left, .. });
            self.focus = if left {
                (self.focus + n - 1) % n
            } else {
                (self.focus + 1) % n
            };
            self.apply_focus();
            inv.push(self.bounds);
            return;
        }
        if matches!(
            *ev,
            InputEvent::Key {
                key: Key::Enter,
                ..
            } | InputEvent::Char { c: ' ', .. }
        ) {
            self.activate_focused();
            return;
        }
        if let InputEvent::Key {
            key: Key::Escape, ..
        } = *ev
        {
            // 3택(S3)에서는 Esc = "나중에"(결정 없음) · 2택에서는 종전대로 부정.
            if self.no.is_some() && self.third.is_none() {
                self.choice = Some(false);
            }
            self.closed = true;
            return;
        }
        if let Some(ch) = &mut self.check {
            ch.on_event(ev, inv);
            let _ = ch.take_toggled(); // 상태는 닫힐 때 is_checked로 읽는다
        }
        self.ok.on_event(ev, inv);
        if self.ok.take_clicked() {
            if self.no.is_some() {
                self.choice = Some(true);
            }
            self.closed = true;
            return;
        }
        if let Some(no) = &mut self.no {
            no.on_event(ev, inv);
            if no.take_clicked() {
                self.choice = Some(false);
                self.closed = true;
                return;
            }
        }
        if let Some(t) = &mut self.third {
            t.on_event(ev, inv);
            if t.take_clicked() {
                self.closed = true; // 결정 없음
            }
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let pad = self.s(16);
        // 제목(굵게).
        ctx.select_font(FontSlot::Base, true);
        ctx.text(b.x + pad, b.y + self.s(14), b, &self.title, theme.text);
        // 상태 목록 모드(08-15 — 그룹 구성원): 목록과 같은 팔레트의 점 + 텍스트.
        if !self.status_list.is_empty() {
            ctx.select_font(FontSlot::Base, false);
            let lh = self.row_h();
            let dot_d = self.s(10);
            let mut y = b.y + self.s(44);
            let flush_bottom = self.content_bottom() - lh;
            for (link, text) in &self.status_list {
                if y > flush_bottom {
                    break; // 넘치면 자른다(모달은 요지 전달용)
                }
                let dot = Rect::new(b.x + pad, y + (ctx.text_height() - dot_d) / 2, dot_d, dot_d);
                // 목록 배지와 같은 실루엣 문법(M3-19) — 회전은 없다(모달은 정적 스냅샷).
                crate::peer_list::draw_link_badge(
                    ctx,
                    dot,
                    theme,
                    *link,
                    false,
                    self.badge_shape,
                    0,
                );
                ctx.text(dot.right() + self.s(8), y, b, text, theme.text);
                y += lh;
            }
            self.paint_controls(ctx, theme);
            return;
        }
        // 본문 — 문자 단위 그리디 줄바꿈(CJK는 공백이 없다 · 설정 desc 워드랩과 같은 이유).
        ctx.select_font(FontSlot::Base, false);
        let avail = (b.w - pad * 2).max(self.s(40));
        let lh = ctx.text_height() + self.s(4);
        let mut y = b.y + self.s(44);
        let mut line = String::new();
        let flush_bottom = self.content_bottom() - lh; // 버튼(·체크박스) 위까지만
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
        self.paint_controls(ctx, theme);
    }
}

impl AlertWidget {
    /// 버튼(1~3)·체크박스 그리기(본문/목록 두 경로 공용).
    fn paint_controls(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        self.ok.paint(ctx, theme);
        if let Some(no) = &self.no {
            no.paint(ctx, theme);
        }
        if let Some(t) = &self.third {
            t.paint(ctx, theme);
        }
        if let Some(ch) = &self.check {
            ch.paint(ctx, theme);
        }
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

    /// 선택 모달(M5-1g) — Enter=수락 · Esc=거절 · 버튼도 같은 결과.
    #[test]
    fn choice_mode_reports_yes_no() {
        let mut w = AlertWidget::new("초대", "수락?").with_choice("수락", "거절");
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
        assert_eq!(w.take_choice(), Some(true), "Enter = 수락");
        let mut w = AlertWidget::new("초대", "수락?").with_choice("수락", "거절");
        w.set_bounds(Rect::new(0, 0, 400, 170), &mut inv);
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_closed());
        assert_eq!(w.take_choice(), Some(false), "Esc = 거절");
        // 일반 경고(버튼 1)는 choice가 없다.
        let mut w = AlertWidget::new("경고", "본문");
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
        assert_eq!(w.take_choice(), None);
    }

    /// G4(08-15) — 상태 목록 행 클릭: 허용 시에만 1회성 보고, 모달은 닫히지 않는다.
    #[test]
    fn status_row_click_reports_when_enabled() {
        use crate::peer_list::LinkState;
        let mut w = AlertWidget::new("구성원", "");
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 400, 300), &mut inv);
        w.set_status_list(
            vec![
                (LinkState::Active, "a".into()),
                (LinkState::Idle, "b".into()),
            ],
            &mut inv,
        );
        let click = |x, y| InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        };
        // 기본(비허용) — 무보고.
        w.on_event(&click(50, 44 + 12), &mut inv);
        assert_eq!(w.take_row_clicked(), None);
        // 허용 — 둘째 행 클릭 = 인덱스 1 · 모달 유지.
        w.set_rows_clickable(true);
        w.on_event(&click(50, 44 + 24 + 12), &mut inv);
        assert_eq!(w.take_row_clicked(), Some(1));
        assert_eq!(w.take_row_clicked(), None, "1회성");
        assert!(!w.take_closed(), "행 클릭은 모달을 닫지 않는다");
    }

    /// S3(DR-30): 3택 — "나중에"·Esc = 결정 없이 닫힘(choice None · 초대 유지) · 체크박스는 닫힐 때 읽는다.
    #[test]
    fn third_button_closes_without_choice_and_checkbox_reads() {
        let mut inv = Invalidations::default();
        let mut w = AlertWidget::new("초대", "수락?")
            .with_choice("수락", "거절")
            .with_third("나중에")
            .with_checkbox("다시 받지 않기");
        w.set_bounds(Rect::new(0, 0, 400, 200), &mut inv);
        let t = w.third.as_ref().unwrap().bounds();
        let click = |x: i32, y: i32| {
            [
                InputEvent::MouseDown {
                    x,
                    y,
                    shift: false,
                    primary: false,
                },
                InputEvent::MouseUp { x, y },
            ]
        };
        for e in click(t.x + 3, t.y + 3) {
            w.on_event(&e, &mut inv);
        }
        assert!(w.take_closed());
        assert_eq!(w.take_choice(), None, "나중에 = 결정 없음");
        assert!(!w.is_checked());
        // Esc도 결정 없음(3택) · 체크박스 클릭 뒤 거절 = choice false + checked.
        let mut w = AlertWidget::new("초대", "수락?")
            .with_choice("수락", "거절")
            .with_third("나중에")
            .with_checkbox("다시 받지 않기");
        w.set_bounds(Rect::new(0, 0, 400, 200), &mut inv);
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_closed());
        assert_eq!(w.take_choice(), None, "3택의 Esc = 나중에");
        let mut w = AlertWidget::new("초대", "수락?")
            .with_choice("수락", "거절")
            .with_third("나중에")
            .with_checkbox("다시 받지 않기");
        w.set_bounds(Rect::new(0, 0, 400, 200), &mut inv);
        let cb = w.check.as_ref().unwrap().bounds();
        for e in click(cb.x + 6, cb.y + cb.h / 2) {
            w.on_event(&e, &mut inv);
        }
        assert!(w.is_checked(), "체크 토글");
        let no = w.no.as_ref().unwrap().bounds();
        for e in click(no.x + 3, no.y + 3) {
            w.on_event(&e, &mut inv);
        }
        assert!(w.take_closed());
        assert_eq!(w.take_choice(), Some(false));
        assert!(w.is_checked(), "닫힌 뒤에도 읽힌다(호스트가 hard 플래그로)");
        assert!(t.x < no.x, "버튼 순서 = [나중에][거절][수락]");
    }

    #[test]
    fn content_can_be_replaced() {
        let mut w = AlertWidget::new("a", "b");
        let mut inv = Invalidations::default();
        w.set_content("새 제목", "새 본문", &mut inv);
        assert_eq!(w.title, "새 제목");
        assert_eq!(w.message, "새 본문");
    }

    /// M3-4(11-06): Tab/Shift+Tab/←/→로 버튼을 돌고 Enter는 **포커스된** 버튼을 누른다(종전 = 항상 긍정).
    #[test]
    fn tab_cycles_buttons_and_enter_activates_focused() {
        let mut inv = Invalidations::default();
        let tab = |shift: bool| InputEvent::Key {
            key: Key::Tab,
            shift,
            primary: false,
        };
        let enter = InputEvent::Key {
            key: Key::Enter,
            shift: false,
            primary: false,
        };
        let mut w = AlertWidget::new("t", "m")
            .with_choice("yes", "no")
            .with_third("later");
        assert!(w.ok.is_focused(), "초기 = 긍정");
        w.on_event(&tab(false), &mut inv);
        assert!(w.no.as_ref().is_some_and(Button::is_focused), "Tab = 부정");
        w.on_event(&tab(false), &mut inv);
        assert!(
            w.third.as_ref().is_some_and(Button::is_focused),
            "Tab = 나중에"
        );
        w.on_event(&tab(false), &mut inv);
        assert!(w.ok.is_focused(), "순환");
        w.on_event(&tab(true), &mut inv);
        assert!(
            w.third.as_ref().is_some_and(Button::is_focused),
            "Shift+Tab = 역순"
        );
        w.on_event(
            &InputEvent::Key {
                key: Key::Left,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.no.as_ref().is_some_and(Button::is_focused), "← = 부정");
        w.on_event(&enter, &mut inv);
        assert!(w.take_closed());
        assert_eq!(w.take_choice(), Some(false), "Enter = 포커스된 부정");
        // 2택 · Enter 즉시 = 긍정(종전 동작 유지).
        let mut w2 = AlertWidget::new("t", "m").with_choice("yes", "no");
        w2.on_event(&enter, &mut inv);
        assert!(w2.take_closed());
        assert_eq!(w2.take_choice(), Some(true));
    }

    /// 위험 동작(11-06): 기본 = 취소 — Enter 즉시는 부정 · Tab 뒤 Enter가 긍정.
    #[test]
    fn default_no_makes_enter_cancel() {
        let mut inv = Invalidations::default();
        let enter = InputEvent::Key {
            key: Key::Enter,
            shift: false,
            primary: false,
        };
        let mut w = AlertWidget::new("t", "m")
            .with_choice("reset", "cancel")
            .with_default_no();
        assert!(w.no.as_ref().is_some_and(Button::is_focused), "초기 = 취소");
        w.on_event(&enter, &mut inv);
        assert!(w.take_closed());
        assert_eq!(w.take_choice(), Some(false));
        let mut w2 = AlertWidget::new("t", "m")
            .with_choice("reset", "cancel")
            .with_default_no();
        w2.on_event(
            &InputEvent::Key {
                key: Key::Tab,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        w2.on_event(&enter, &mut inv);
        assert_eq!(w2.take_choice(), Some(true), "Tab 뒤 Enter = 긍정");
        // 확인 하나짜리(no 없음)는 영향 없음.
        let w3 = AlertWidget::new("t", "m").with_default_no();
        assert!(w3.ok.is_focused());
    }
}
