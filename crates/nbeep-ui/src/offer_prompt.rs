//! **파일 수신 승인 화면** — 누가·언제·무엇을·얼마나 보내는지 보여 주고 결정을 받는다.
//!
//! 사용자 확정(08-09):
//! - 정보 4종(**보낸 사람 · 받은 시각 · 파일 이름 · 크기**)을 먼저 보여 준다.
//! - 하단 기본 버튼 **승인 · 취소** — 취소는 [`TimeoutButton`]이라 지정 시간이 지나면
//!   **스스로 눌려** 창을 닫는다(기다림에 끝이 있다 · 시간은 설정에서 지정).
//! - 그 위에 **자동 승인 기간 콤보(1/6/24시간) + [자동 승인]** — 설정 화면까지 가지 않고
//!   여기서 바로 기간 자동 수락을 켠다. 켜면 이 건도 함께 수락된다.
//!
//! ⚠️ 여기서 "승인"은 **수신 수락(격리까지)**이다 — 실행 가능한 실체 파일이 되려면
//! 격리함에서 별도 승인이 필요하다([docs/11] §7). 자동 승인이어도 그 문은 닫혀 있다.

use crate::controls::{Button, Combo, ComboControl as _, ComboItem, Control as _, TimeoutButton};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::InputEvent;
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::{t, tf, Msg};

/// 화면에 띄울 제안 정보(호스트가 채운다).
#[derive(Clone, Debug)]
pub struct OfferInfo {
    /// 보낸 사람(표시 이름 + 지문 앞자리 등 — 호스트가 만든 문자열).
    pub sender: String,
    /// 받은 시각(지역 시각 문자열).
    pub when: String,
    /// 파일 이름(원본 그대로 — 실체화 시 정규화된다).
    pub name: String,
    /// 크기(바이트).
    pub size: u64,
    /// 같은 상대에게서 대기 중인 제안 수(1이면 표시하지 않는다).
    pub queued: usize,
    /// 자동 승인 강등 안내(08-16 — 자동 승인 중인데 이 창이 뜬 이유를 설명:
    /// 빈 값 = 표시 없음). 실기에서 "자동인데 왜 묻지?"가 버그로 오인됐다.
    pub downgrade_note: String,
    /// 이어받기 보존율(M4-10c · 0~100) — Some이면 승인이 2택이 된다:
    /// [이어받기 N%](기본) · [처음부터]. None = 종전 단일 승인.
    pub resume_pct: Option<u8>,
}

/// 사용자 결정.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfferChoice {
    /// 이 건만 수락(보존분이 있으면 **이어받기** — 기본).
    Approve,
    /// 보존분을 버리고 **처음부터** 수락(M4-10c — 이어받기 2택의 다른 쪽).
    ApproveFresh,
    /// 거절(취소 버튼·타임아웃) — 타임아웃이면 `by_timeout = true`.
    Cancel {
        /// 시간이 다 되어 자동으로 닫혔는가.
        by_timeout: bool,
    },
    /// 기간 자동 수락을 켠다(이 건도 함께 수락) — 값은 설정 코드(`1h`/`6h`/`today`).
    AutoFor(&'static str),
}

/// 기간 선택지(설정 화면과 **같은 코드**를 쓴다 — 두 곳이 어긋나지 않게).
const WINDOWS: [(&str, &str); 3] = [("1h", "1시간"), ("6h", "6시간"), ("today", "24시간")];

/// 사람이 읽는 크기.
fn human(bytes: u64) -> String {
    const K: u64 = 1024;
    match bytes {
        b if b >= K * K => format!("{:.1}MiB", b as f64 / (K * K) as f64),
        b if b >= K => format!("{:.1}KiB", b as f64 / K as f64),
        b => format!("{b}B"),
    }
}

/// 승인 화면 위젯.
#[derive(Debug)]
pub struct OfferPromptWidget {
    bounds: Rect,
    scale: f32,
    info: OfferInfo,
    window: Combo,
    auto_btn: Button,
    approve: Button,
    /// 처음부터(M4-10c) — 이어받기가 가능할 때만 화면에 존재한다.
    fresh: Option<Button>,
    cancel: TimeoutButton,
    choice: Option<OfferChoice>,
}

impl OfferPromptWidget {
    /// 정보와 **취소 자동 실행 시간(초)**으로 만든다.
    #[must_use]
    pub fn new(info: OfferInfo, timeout_secs: u64) -> Self {
        let items = WINDOWS
            .iter()
            .map(|(v, l)| ComboItem::new(*v, *l))
            .collect();
        let approve = if let Some(pct) = info.resume_pct {
            Button::new(tf(Msg::OfferResumeBtn, &[&pct.to_string()]))
        } else {
            Button::new(t(Msg::QApprove))
        };
        let fresh = info
            .resume_pct
            .is_some()
            .then(|| Button::new(t(Msg::OfferFreshBtn)));
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            info,
            window: Combo::new(items, 0),
            auto_btn: Button::new(t(Msg::OfferAutoBtn)),
            approve,
            fresh,
            cancel: TimeoutButton::new(t(Msg::OfferCancel), timeout_secs.saturating_mul(1000)),
            choice: None,
        }
    }

    /// 카운트다운 시작(호스트가 현재 시각을 준다).
    pub fn start(&mut self, now_ms: u64) {
        self.cancel.start(now_ms);
    }

    /// 시각 주입 — 만료되면 자동 취소가 발생하고 `true`(재그리기).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let redraw = self.cancel.tick(now_ms);
        if let Some(f) = self.cancel.take_fired() {
            self.choice = Some(OfferChoice::Cancel {
                by_timeout: f == crate::controls::FiredBy::Timeout,
            });
        }
        redraw
    }

    /// 배율 지정.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.window.set_scale(self.scale);
        self.auto_btn.set_scale(self.scale);
        self.approve.set_scale(self.scale);
        if let Some(f) = &mut self.fresh {
            f.set_scale(self.scale);
        }
        self.cancel.set_scale(self.scale);
        inv.push(self.bounds);
    }

    /// 결정(1회성).
    pub fn take_choice(&mut self) -> Option<OfferChoice> {
        self.choice.take()
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }
}

impl Widget for OfferPromptWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        let pad = self.s(16);
        let bh = self.s(28);
        // 하단 줄: [승인] [취소] — 오른쪽 정렬.
        let bw = self.s(96);
        let by = bounds.bottom() - bh - pad;
        self.cancel
            .set_bounds(Rect::new(bounds.right() - bw - pad, by, bw, bh), inv);
        // 이어받기 2택(M4-10c): [이어받기 N%][처음부터][취소] — 오른쪽 정렬 유지.
        let (g8, g16) = (self.s(8), self.s(16));
        if let Some(f) = &mut self.fresh {
            f.set_bounds(
                Rect::new(bounds.right() - bw * 2 - pad - g8, by, bw, bh),
                inv,
            );
            self.approve.set_bounds(
                Rect::new(bounds.right() - bw * 3 - pad - g16, by, bw, bh),
                inv,
            );
        } else {
            self.approve.set_bounds(
                Rect::new(bounds.right() - bw * 2 - pad - self.s(8), by, bw, bh),
                inv,
            );
        }
        // 그 위 줄: [기간 콤보] [자동 승인] — 왼쪽 정렬.
        let ay = by - bh - self.s(12);
        self.window
            .set_bounds(Rect::new(bounds.x + pad, ay, self.s(110), bh), inv);
        self.auto_btn.set_bounds(
            Rect::new(bounds.x + pad + self.s(118), ay, self.s(104), bh),
            inv,
        );
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 콤보가 열려 있으면 모달 캡처(뒤 버튼으로 클릭이 새지 않게).
        if self.window.is_open() {
            self.window.on_event(ev, inv);
            inv.push(self.bounds);
            return;
        }
        self.window.on_event(ev, inv);
        self.auto_btn.on_event(ev, inv);
        self.approve.on_event(ev, inv);
        if let Some(f) = &mut self.fresh {
            f.on_event(ev, inv);
        }
        self.cancel.on_event(ev, inv);

        if self.approve.take_clicked() {
            self.choice = Some(OfferChoice::Approve);
        } else if self.fresh.as_mut().is_some_and(Button::take_clicked) {
            self.choice = Some(OfferChoice::ApproveFresh);
        } else if self.auto_btn.take_clicked() {
            // 콤보 값 → 설정과 같은 코드로 되돌려 준다.
            let v = self.window.selected_value();
            let code = WINDOWS
                .iter()
                .find(|(c, _)| *c == v)
                .map_or("1h", |(c, _)| *c);
            self.choice = Some(OfferChoice::AutoFor(code));
        } else if let Some(f) = self.cancel.take_fired() {
            self.choice = Some(OfferChoice::Cancel {
                by_timeout: f == crate::controls::FiredBy::Timeout,
            });
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let pad = self.s(16);

        // 제목.
        ctx.select_font(FontSlot::Base, true);
        let th = ctx.text_height();
        let title = if self.info.queued > 1 {
            format!("{} ({})", t(Msg::OfferTitle), self.info.queued)
        } else {
            t(Msg::OfferTitle).to_string()
        };
        ctx.text(b.x + pad, b.y + pad, b, &title, theme.text);

        // 정보 4종 — 라벨/값 2열.
        let mut y = b.y + pad + th + self.s(12);
        let label_w = self.s(88);
        let rows = [
            (t(Msg::OfferSender), self.info.sender.clone()),
            (t(Msg::OfferWhen), self.info.when.clone()),
            (t(Msg::OfferName), self.info.name.clone()),
            (t(Msg::OfferSize), human(self.info.size)),
        ];
        for (k, v) in rows {
            ctx.select_font(FontSlot::Status, false);
            let sh = ctx.text_height();
            ctx.text(b.x + pad, y, b, k, theme.text_dim);
            ctx.select_font(FontSlot::Base, false);
            ctx.text(b.x + pad + label_w, y - self.s(2), b, &v, theme.text);
            y += sh + self.s(10);
        }

        // 강등 안내(08-16) — 자동 승인 중인데 이 창이 뜬 이유(첫 왕래 전 강등).
        // 설정과 실제 동작의 괴리가 버그로 오인되는 것을 막는다(실기).
        if !self.info.downgrade_note.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            let sh = ctx.text_height();
            ctx.text(b.x + pad, y, b, &self.info.downgrade_note, theme.warn);
            y += sh + self.s(8);
        }

        // 구분선.
        ctx.fill_rect(Rect::new(b.x + pad, y, b.w - pad * 2, 1), theme.border);

        // 안내 — 승인해도 바로 실행되지 않는다는 사실을 여기서 밝힌다.
        ctx.select_font(FontSlot::Status, false);
        let sh = ctx.text_height();
        ctx.text(
            b.x + pad,
            y + self.s(8),
            b,
            t(Msg::OfferQuarantineNote),
            theme.text_dim,
        );
        let _ = sh;

        self.window.paint(ctx, theme);
        self.auto_btn.paint(ctx, theme);
        self.approve.paint(ctx, theme);
        if let Some(f) = &self.fresh {
            f.paint(ctx, theme);
        }
        self.cancel.paint(ctx, theme);
        // 열린 드롭다운은 맨 위에.
        if self.window.is_open() {
            self.window.paint(ctx, theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> OfferInfo {
        OfferInfo {
            sender: "김철수 (a1b2c3d4)".into(),
            when: "22:14:03".into(),
            name: "보고서.pdf".into(),
            size: 1024 * 1024,
            queued: 1,
            downgrade_note: String::new(),
            resume_pct: None,
        }
    }

    fn prompt() -> (OfferPromptWidget, Invalidations) {
        let mut w = OfferPromptWidget::new(info(), 60);
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 420, 280), &mut inv);
        w.start(0);
        (w, inv)
    }
    fn click(w: &mut OfferPromptWidget, r: Rect, inv: &mut Invalidations) {
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 5,
                y: r.y + 5,
                shift: false,
                primary: false,
            },
            inv,
        );
        w.on_event(
            &InputEvent::MouseUp {
                x: r.x + 5,
                y: r.y + 5,
            },
            inv,
        );
    }

    #[test]
    fn approve_reports_once() {
        let (mut w, mut inv) = prompt();
        let r = w.approve.bounds();
        click(&mut w, r, &mut inv);
        assert_eq!(w.take_choice(), Some(OfferChoice::Approve));
        assert!(w.take_choice().is_none(), "1회성");
    }

    #[test]
    fn cancel_button_and_timeout_both_report_cancel() {
        let (mut w, mut inv) = prompt();
        let r = w.cancel.bounds();
        click(&mut w, r, &mut inv);
        assert_eq!(
            w.take_choice(),
            Some(OfferChoice::Cancel { by_timeout: false })
        );

        // 새 화면에서 시간을 흘리면 스스로 취소된다.
        let (mut w2, _) = prompt();
        w2.tick(59_000);
        assert!(w2.take_choice().is_none(), "아직");
        w2.tick(60_000);
        assert_eq!(
            w2.take_choice(),
            Some(OfferChoice::Cancel { by_timeout: true }),
            "지정 시간 경과 = 자동 취소"
        );
    }

    #[test]
    fn auto_button_returns_selected_window_code() {
        let (mut w, mut inv) = prompt();
        // 기본 = 1시간.
        let r = w.auto_btn.bounds();
        click(&mut w, r, &mut inv);
        assert_eq!(w.take_choice(), Some(OfferChoice::AutoFor("1h")));

        // 콤보에서 6시간을 고르면 그 코드가 나온다.
        let (mut w2, mut inv2) = prompt();
        w2.window.select_value("6h");
        let r2 = w2.auto_btn.bounds();
        click(&mut w2, r2, &mut inv2);
        assert_eq!(w2.take_choice(), Some(OfferChoice::AutoFor("6h")));
    }

    /// M4-10c — 보존분이 있으면 [이어받기 N%]·[처음부터] 2택. 각 버튼이 제
    /// 선택을 보고하고, 보존분이 없으면 처음부터 버튼 자체가 없다.
    #[test]
    fn resume_two_choice_buttons_report() {
        let mut i = info();
        i.resume_pct = Some(37);
        let mut w = OfferPromptWidget::new(i, 60);
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 420, 280), &mut inv);
        w.start(0);
        let r = w.approve.bounds();
        click(&mut w, r, &mut inv);
        assert_eq!(
            w.take_choice(),
            Some(OfferChoice::Approve),
            "기본 = 이어받기"
        );
        let rf = w.fresh.as_ref().unwrap().bounds();
        click(&mut w, rf, &mut inv);
        assert_eq!(w.take_choice(), Some(OfferChoice::ApproveFresh));
        let (w2, _) = prompt();
        assert!(w2.fresh.is_none(), "보존분 없음 = 단일 승인");
    }

    #[test]
    fn custom_timeout_is_respected() {
        let mut w = OfferPromptWidget::new(info(), 5);
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 420, 280), &mut inv);
        w.start(0);
        w.tick(4_000);
        assert!(w.take_choice().is_none());
        w.tick(5_000);
        assert_eq!(
            w.take_choice(),
            Some(OfferChoice::Cancel { by_timeout: true }),
            "설정한 5초에 맞춰 닫힌다"
        );
    }
}
