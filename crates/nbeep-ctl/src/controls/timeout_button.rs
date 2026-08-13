//! **타임아웃 버튼** — 지정 시간이 지나면 **스스로 눌리는** 버튼(사용자 요청 08-09).
//!
//! 쓰임: 송신측이 상대 승인을 기다리는 동안 "취소" 버튼을 띄우고, 60초가 지나면
//! 사용자가 아무것도 하지 않아도 **자동으로 눌려** 전송을 취소하고 창을 닫는다.
//! 즉 *"기다림에는 끝이 있다"* 를 UI가 보장한다 — 응답 없는 상대 때문에 창이 영원히
//! 남지 않는다.
//!
//! 남은 시간은 **숫자와 게이지 두 가지로** 보여 준다(진행형 테두리 채움 + "45초").
//! 호스트가 [`TimeoutButton::tick`]을 주기적으로 호출해 시각을 주입한다 —
//! 컨트롤이 시계를 직접 읽지 않아 **테스트가 결정적**이다([`crate::event`] 규약과 동일).
//!
//! 클릭이든 만료든 결과는 [`TimeoutButton::take_fired`] 하나로 보고하고,
//! **어느 쪽이었는지**는 [`TimeoutButton::fired_by_timeout`]로 구분한다.

use super::{Control, ControlBase};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 발화 원인.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FiredBy {
    /// 사용자가 눌렀다.
    Click,
    /// 시간이 다 됐다(자동).
    Timeout,
}

/// 지정 시간이 지나면 자동으로 눌리는 버튼.
#[derive(Debug)]
pub struct TimeoutButton {
    base: ControlBase,
    label: String,
    /// 총 대기 시간(ms).
    total_ms: u64,
    /// 시작 시각(ms · 호스트 주입) — `None`이면 아직 시작 전.
    started_ms: Option<u64>,
    /// 마지막으로 주입된 시각.
    now_ms: u64,
    pressed: bool,
    hover: bool,
    fired: Option<FiredBy>,
    /// **한 번 발화하면 끝**. `take_fired`로 꺼낸 뒤에도 만료가 두 번째 발화를 만들지
    /// 않게 잠근다(호스트가 창을 닫기 전 tick이 한 번 더 도는 경우가 실제로 있다).
    done: bool,
}

impl TimeoutButton {
    /// 라벨과 제한 시간(ms)으로 만든다. [`TimeoutButton::start`] 전에는 카운트다운이 없다.
    #[must_use]
    pub fn new(label: impl Into<String>, total_ms: u64) -> Self {
        Self {
            base: ControlBase::default(),
            label: label.into(),
            total_ms,
            started_ms: None,
            now_ms: 0,
            pressed: false,
            hover: false,
            fired: None,
            done: false,
        }
    }

    /// 카운트다운 시작(호스트가 현재 시각을 준다).
    pub fn start(&mut self, now_ms: u64) {
        self.started_ms = Some(now_ms);
        self.now_ms = now_ms;
        self.fired = None;
        self.done = false;
    }

    /// 시각 주입 — 만료되면 **자동 발화**하고 `true`(재그리기 필요)를 준다.
    /// 남은 시간 표시가 바뀌어도 `true`.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let before = self.remaining_secs();
        self.now_ms = now_ms;
        if !self.done && self.expired() {
            self.fired = Some(FiredBy::Timeout);
            self.done = true;
            return true;
        }
        before != self.remaining_secs()
    }

    /// 남은 시간(ms · 시작 전이면 총 시간).
    #[must_use]
    pub fn remaining_ms(&self) -> u64 {
        match self.started_ms {
            None => self.total_ms,
            Some(s) => self.total_ms.saturating_sub(self.now_ms.saturating_sub(s)),
        }
    }

    /// 남은 시간(초 · 올림) — 표시용.
    #[must_use]
    pub fn remaining_secs(&self) -> u64 {
        self.remaining_ms().div_ceil(1000)
    }

    /// 만료 여부.
    #[must_use]
    pub fn expired(&self) -> bool {
        self.started_ms.is_some() && self.remaining_ms() == 0
    }

    /// 발화했으면 원인을 꺼낸다(1회성).
    pub fn take_fired(&mut self) -> Option<FiredBy> {
        self.fired.take()
    }

    /// 마지막 발화가 타임아웃이었는가(꺼내지 않고 확인).
    #[must_use]
    pub fn fired_by_timeout(&self) -> bool {
        self.fired == Some(FiredBy::Timeout)
    }

    /// 진행 비율 0.0~1.0(경과분) — 게이지 렌더용.
    fn elapsed_ratio(&self) -> f32 {
        if self.total_ms == 0 {
            return 1.0;
        }
        let done = self.total_ms.saturating_sub(self.remaining_ms());
        (done as f32 / self.total_ms as f32).clamp(0.0, 1.0)
    }
}

impl Control for TimeoutButton {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for TimeoutButton {
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
                if self.base.bounds.contains(Point { x, y }) {
                    self.pressed = true;
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::MouseUp { x, y } => {
                if self.pressed {
                    self.pressed = false;
                    if self.base.bounds.contains(Point { x, y }) && !self.done {
                        self.fired = Some(FiredBy::Click);
                        self.done = true;
                    }
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::MouseMove { x, y } => {
                let over = self.base.bounds.contains(Point { x, y });
                if over != self.hover {
                    self.hover = over;
                    inv.push(self.base.bounds);
                }
            }
            // Esc = 즉시 취소(= 클릭과 같다). 기다리는 창에서 가장 자연스러운 손이다.
            InputEvent::Key {
                key: Key::Escape, ..
            } if !self.done => {
                self.fired = Some(FiredBy::Click);
                self.done = true;
                inv.push(self.base.bounds);
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        let radius = self.s(6);
        let bg = if self.pressed {
            theme.sel_bg
        } else if self.hover {
            theme.panel_bg_alt
        } else {
            theme.field_bg
        };
        ctx.fill_round_rect(b, radius, bg);

        // 경과 게이지 — 왼쪽부터 차오른다(남은 시간이 줄어드는 게 눈에 보인다).
        let ratio = self.elapsed_ratio();
        if ratio > 0.0 {
            let w = (b.w as f32 * ratio).round() as i32;
            if w > 0 {
                // 막판(20% 미만 남음)은 위험색으로 — 곧 취소된다는 신호.
                let fill = if self.remaining_ms() * 5 <= self.total_ms {
                    theme.danger
                } else {
                    theme.accent
                };
                ctx.fill_round_rect_alpha(Rect::new(b.x, b.y, w.min(b.w), b.h), radius, fill, 0.28);
            }
        }
        ctx.stroke_round_rect(b, radius, theme.border, 1.0);
        self.draw_focus_ring(ctx, theme, b);

        // 라벨 + 남은 초.
        ctx.select_font(FontSlot::Base, false);
        let text = if self.started_ms.is_some() && !self.expired() {
            format!("{} ({}초)", self.label, self.remaining_secs())
        } else {
            self.label.clone()
        };
        let tw = ctx.text_width(&text);
        let th = ctx.text_height();
        ctx.text(
            b.x + (b.w - tw) / 2,
            b.y + (b.h - th) / 2,
            b,
            &text,
            theme.text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn btn() -> (TimeoutButton, Invalidations) {
        let mut t = TimeoutButton::new("전송 취소", 60_000);
        let mut inv = Invalidations::default();
        t.set_bounds(Rect::new(0, 0, 160, 28), &mut inv);
        (t, inv)
    }
    fn down(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn fires_itself_when_time_runs_out() {
        let (mut t, _) = btn();
        t.start(1_000);
        t.tick(30_000); // 초 표시가 바뀌어 true지만 발화는 아니다
        assert_eq!(t.take_fired(), None, "중간 — 아직 발화 없음");
        assert!(t.tick(61_000), "만료 시각 = 발화");
        assert_eq!(t.take_fired(), Some(FiredBy::Timeout));
        assert!(t.take_fired().is_none(), "1회성");
    }

    #[test]
    fn click_fires_early_and_blocks_later_timeout() {
        let (mut t, mut inv) = btn();
        t.start(0);
        t.on_event(&down(5, 5), &mut inv);
        t.on_event(&InputEvent::MouseUp { x: 5, y: 5 }, &mut inv);
        assert_eq!(t.take_fired(), Some(FiredBy::Click));
        // 이미 발화한 뒤에는 만료가 두 번째 발화를 만들지 않는다.
        t.tick(999_999);
        assert!(t.take_fired().is_none(), "중복 발화 없음");
    }

    #[test]
    fn escape_counts_as_cancel() {
        let (mut t, mut inv) = btn();
        t.start(0);
        t.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(t.take_fired(), Some(FiredBy::Click));
    }

    #[test]
    fn release_outside_does_not_fire() {
        let (mut t, mut inv) = btn();
        t.start(0);
        t.on_event(&down(5, 5), &mut inv);
        t.on_event(&InputEvent::MouseUp { x: 900, y: 900 }, &mut inv);
        assert!(t.take_fired().is_none());
    }

    #[test]
    fn countdown_reports_remaining_and_redraw_on_second_change() {
        let (mut t, _) = btn();
        t.start(0);
        assert_eq!(t.remaining_secs(), 60);
        assert!(t.tick(1_000), "초 단위가 바뀌면 재그리기");
        assert_eq!(t.remaining_secs(), 59);
        assert!(!t.tick(1_100), "같은 초 안에서는 재그리기 불필요");
        assert!(!t.expired());
    }

    #[test]
    fn not_started_means_no_countdown() {
        let (mut t, _) = btn();
        assert_eq!(t.remaining_ms(), 60_000, "시작 전엔 총 시간 유지");
        assert!(!t.tick(10_000_000), "시작하지 않으면 만료도 없다");
        assert!(t.take_fired().is_none());
        assert!(!t.expired());
    }
}
