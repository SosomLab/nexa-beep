//! 캐러셀 — **가로 아이템 띠 + 넘칠 때만 나타나는 좌/우 이동 버튼**(08-14 사용자 확정).
//!
//! 규칙(사용자 명세 그대로):
//! - 왼쪽 끝: `(아이템)(아이템)…(▶)` — 좌버튼 없음, 아이템부터.
//! - 중간: `(◀)(아이템)…(▶)`.
//! - 오른쪽 끝: `(◀)(아이템)…` — 우버튼 없음.
//! - 안 넘치면 버튼이 아예 없다.
//!
//! **아이템 그리기는 소유자 몫**이다(아바타·색상 등 내용을 컨트롤이 모른다 — 조합/위임).
//! 컨트롤은 창(윈도잉)·버튼·클릭 판정만 책임진다: [`Carousel::item_rect`]로 보이는
//! 아이템의 자리를 받아 소유자가 그리고, [`Carousel::paint`]가 버튼을 얹는다.
//! 클릭은 [`Carousel::take_clicked`](1회성 · 아이템 **전역 인덱스**)로 회수한다.

use super::{Control, ControlBase};
use crate::draw::DrawCtx;
use crate::event::{InputEvent, WheelAccum};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 캐러셀 컨트롤 — 정사각 아이템 가로 띠.
#[derive(Debug)]
pub struct Carousel {
    base: ControlBase,
    /// 아이템 한 변(논리 px — 배율 전 값).
    item_px: i32,
    /// 아이템 간격(논리 px).
    gap: i32,
    /// 전체 아이템 수(소유자가 갱신).
    count: usize,
    /// 첫 표시 아이템 인덱스(스크롤 상태).
    first: usize,
    /// 아이템 클릭(전역 인덱스 · 1회성).
    clicked: Option<usize>,
    /// 커서가 띠 위에 있는가 — 가로 휠(트랙패드) 스크롤은 이 위에서만(08-14).
    hover: bool,
    /// 가로 휠 노치 누적(트랙패드 분수 delta).
    hwheel: WheelAccum,
}

impl Carousel {
    /// 아이템 크기·간격으로 만든다(개수는 [`Carousel::set_count`]).
    #[must_use]
    pub fn new(item_px: i32, gap: i32) -> Self {
        Self {
            base: ControlBase::default(),
            item_px,
            gap,
            count: 0,
            first: 0,
            clicked: None,
            hover: false,
            hwheel: WheelAccum::default(),
        }
    }

    /// 전체 아이템 수 갱신 — 줄어들면 스크롤을 안쪽으로 되민다.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        self.clamp_first();
    }

    /// 아이템 클릭(1회성 · 전역 인덱스).
    pub fn take_clicked(&mut self) -> Option<usize> {
        self.clicked.take()
    }

    fn item_w(&self) -> i32 {
        self.s(self.item_px)
    }
    fn gap_w(&self) -> i32 {
        self.s(self.gap)
    }
    /// 이동 버튼 폭(논리 20 — 32px 아이템 옆에서 눌리는 최소 크기).
    fn btn_w(&self) -> i32 {
        self.s(20)
    }

    /// 이 `first`에서 좌버튼이 보이는가.
    fn left_shown_at(first: usize) -> bool {
        first > 0
    }

    /// 이 `first`에서 (표시 용량, 우버튼 표시 여부). 버튼 표시가 용량을 바꾸고
    /// 용량이 버튼 표시를 바꾸므로 **우버튼 없는 가정 → 필요 판정 → 재계산** 2패스.
    fn cap_at(&self, first: usize) -> (usize, bool) {
        let iw = self.item_w() + self.gap_w();
        if iw <= 0 {
            return (0, false);
        }
        let mut avail = self.base.bounds.w;
        if Self::left_shown_at(first) {
            avail -= self.btn_w() + self.gap_w();
        }
        // 우버튼 없다고 가정한 용량.
        let cap_no_r = ((avail + self.gap_w()).max(0) / iw).max(0) as usize;
        if first + cap_no_r >= self.count {
            return (cap_no_r.min(self.count - first.min(self.count)), false);
        }
        // 넘친다 — 우버튼 자리를 빼고 재계산.
        let cap_r =
            (((avail - self.btn_w() - self.gap_w() + self.gap_w()).max(0)) / iw).max(0) as usize;
        (cap_r, true)
    }

    /// 오른쪽 끝을 채우도록 `first`를 되민다(마지막 페이지가 덜 차 보이지 않게).
    fn clamp_first(&mut self) {
        if self.count == 0 {
            self.first = 0;
            return;
        }
        self.first = self.first.min(self.count - 1);
        while self.first > 0 {
            let (cap, _) = self.cap_at(self.first - 1);
            if self.first - 1 + cap >= self.count {
                self.first -= 1;
            } else {
                break;
            }
        }
    }

    /// 아이템 띠 시작 x(좌버튼 유무 반영).
    fn strip_x(&self) -> i32 {
        let mut x = self.base.bounds.x;
        if Self::left_shown_at(self.first) {
            x += self.btn_w() + self.gap_w();
        }
        x
    }

    /// i번째(전역) 아이템의 자리 — 지금 화면에 없으면 `None`. 소유자가 이 자리에 그린다.
    #[must_use]
    pub fn item_rect(&self, i: usize) -> Option<Rect> {
        let (cap, _) = self.cap_at(self.first);
        if i < self.first || i >= self.first + cap || i >= self.count {
            return None;
        }
        let d = self.item_w();
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let off = (d + self.gap_w()) * (i - self.first) as i32;
        let y = self.base.bounds.y + (self.base.bounds.h - d) / 2;
        Some(Rect::new(self.strip_x() + off, y, d, d))
    }

    /// 좌 이동 버튼 자리(표시 중일 때만).
    #[must_use]
    pub fn left_rect(&self) -> Option<Rect> {
        Self::left_shown_at(self.first).then(|| {
            let b = self.base.bounds;
            Rect::new(b.x, b.y, self.btn_w(), b.h)
        })
    }

    /// 우 이동 버튼 자리(표시 중일 때만).
    #[must_use]
    pub fn right_rect(&self) -> Option<Rect> {
        let (cap, shown) = self.cap_at(self.first);
        shown.then(|| {
            let b = self.base.bounds;
            let x = self.strip_x()
                + (self.item_w() + self.gap_w()) * i32::try_from(cap).unwrap_or(i32::MAX);
            Rect::new(x, b.y, self.btn_w(), b.h)
        })
    }

    /// 한 페이지(현재 표시 용량) 단위로 이동.
    fn page(&mut self, dir: i32, inv: &mut Invalidations) {
        let (cap, _) = self.cap_at(self.first);
        let step = cap.max(1);
        if dir < 0 {
            self.first = self.first.saturating_sub(step);
        } else {
            self.first = (self.first + step).min(self.count.saturating_sub(1));
        }
        self.clamp_first();
        inv.push(self.base.bounds);
    }
}

impl Control for Carousel {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for Carousel {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        self.clamp_first();
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 트랙패드 가로 스크롤(08-14 사용자 요청) — 띠 위에 커서가 있을 때만,
        // 노치 누적으로 **아이템 단위** 이동(버튼은 페이지 단위 — 정밀/도약 분담).
        match *ev {
            InputEvent::MouseMove { x, y } => {
                self.hover = self.base.bounds.contains(Point { x, y });
                return;
            }
            InputEvent::HWheel { delta } if self.hover => {
                let steps = self.hwheel.add(delta, 1);
                if steps != 0 {
                    // 양수 = 오른쪽(다음 아이템) — 목록 가로 스크롤 방향과 일치.
                    if steps > 0 {
                        self.first = (self.first + steps.unsigned_abs() as usize)
                            .min(self.count.saturating_sub(1));
                    } else {
                        self.first = self.first.saturating_sub(steps.unsigned_abs() as usize);
                    }
                    self.clamp_first();
                    inv.push(self.base.bounds);
                }
                return;
            }
            _ => {}
        }
        let InputEvent::MouseDown { x, y, .. } = *ev else {
            return;
        };
        let p = Point { x, y };
        if self.left_rect().is_some_and(|r| r.contains(p)) {
            self.page(-1, inv);
            return;
        }
        if self.right_rect().is_some_and(|r| r.contains(p)) {
            self.page(1, inv);
            return;
        }
        let (cap, _) = self.cap_at(self.first);
        for i in self.first..(self.first + cap).min(self.count) {
            if self.item_rect(i).is_some_and(|r| r.contains(p)) {
                self.clicked = Some(i);
                inv.push(self.base.bounds);
                return;
            }
        }
    }

    /// 버튼만 그린다 — 아이템은 소유자가 [`Carousel::item_rect`]로 그린 뒤 호출.
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let chevron = |ctx: &mut dyn DrawCtx, r: Rect, dir: i32| {
            let cx = r.x + r.w / 2;
            let cy = r.y + r.h / 2;
            let half = (r.w / 4).max(3);
            let w = (r.w as f32 / 8.0).max(1.5);
            ctx.polyline(
                &[
                    (cx + dir * half / 2, cy - half),
                    (cx - dir * half / 2, cy),
                    (cx + dir * half / 2, cy + half),
                ],
                theme.text_dim,
                w,
            );
        };
        if let Some(r) = self.left_rect() {
            ctx.fill_round_rect(r, self.s(4), theme.field_bg);
            chevron(ctx, r, 1); // ◀
        }
        if let Some(r) = self.right_rect() {
            ctx.fill_round_rect(r, self.s(4), theme.field_bg);
            chevron(ctx, r, -1); // ▶
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car(w: i32, count: usize) -> Carousel {
        let mut c = Carousel::new(32, 4);
        let mut inv = Invalidations::default();
        c.set_count(count);
        c.set_bounds(Rect::new(0, 0, w, 36), &mut inv);
        c
    }

    #[test]
    fn no_buttons_when_content_fits() {
        let c = car(400, 5); // 5×36-4 = 176 ≤ 400
        assert!(c.left_rect().is_none(), "왼쪽 끝 = 좌버튼 없음");
        assert!(c.right_rect().is_none(), "안 넘침 = 우버튼 없음");
        assert!(c.item_rect(0).is_some() && c.item_rect(4).is_some());
    }

    #[test]
    fn right_button_appears_on_overflow_and_left_after_paging() {
        let mut c = car(200, 16); // 넘친다
        assert!(c.left_rect().is_none(), "왼쪽 끝 = 아이템부터");
        assert!(c.right_rect().is_some(), "넘침 = 우버튼");
        let first_rect = c.item_rect(0).expect("첫 아이템 표시");
        assert_eq!(first_rect.x, 0, "좌버튼이 없으니 아이템이 맨 앞");
        // ▶ 클릭 = 페이지 이동 → 좌버튼 등장.
        let r = c.right_rect().unwrap();
        let mut inv = Invalidations::default();
        c.on_event(
            &InputEvent::MouseDown {
                x: r.x + 1,
                y: r.y + 1,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(c.left_rect().is_some(), "이동 후 = 좌버튼");
        assert!(c.item_rect(0).is_none(), "앞 아이템은 화면 밖");
    }

    #[test]
    fn right_edge_hides_right_button_and_packs_items() {
        let mut c = car(200, 16);
        let mut inv = Invalidations::default();
        // 끝까지 이동.
        for _ in 0..10 {
            let Some(r) = c.right_rect() else { break };
            c.on_event(
                &InputEvent::MouseDown {
                    x: r.x + 1,
                    y: r.y + 1,
                    shift: false,
                    primary: false,
                },
                &mut inv,
            );
        }
        assert!(c.right_rect().is_none(), "오른쪽 끝 = 우버튼 없음");
        assert!(c.left_rect().is_some(), "오른쪽 끝 = 좌버튼만");
        assert!(c.item_rect(15).is_some(), "마지막 아이템이 보인다");
    }

    #[test]
    fn trackpad_hwheel_scrolls_items_under_cursor() {
        let mut c = car(200, 16);
        let mut inv = Invalidations::default();
        // 커서가 띠 밖 — 무시(다른 스크롤 영역과 경합하지 않게).
        c.on_event(&InputEvent::HWheel { delta: 120 }, &mut inv);
        assert!(c.item_rect(0).is_some(), "밖에서는 안 움직인다");
        // 띠 위에 올리고 한 노치 = 한 아이템 전진.
        c.on_event(&InputEvent::MouseMove { x: 10, y: 10 }, &mut inv);
        c.on_event(&InputEvent::HWheel { delta: 120 }, &mut inv);
        assert!(c.item_rect(0).is_none(), "한 노치 = 한 아이템 전진");
        // 반대로 되돌리기.
        c.on_event(&InputEvent::HWheel { delta: -120 }, &mut inv);
        assert!(c.item_rect(0).is_some(), "반대 방향 복귀");
    }

    #[test]
    fn item_click_reports_global_index() {
        let mut c = car(200, 16);
        let r = c.item_rect(1).unwrap();
        let mut inv = Invalidations::default();
        c.on_event(
            &InputEvent::MouseDown {
                x: r.x + 1,
                y: r.y + 1,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(c.take_clicked(), Some(1));
        assert_eq!(c.take_clicked(), None, "1회성");
    }
}
