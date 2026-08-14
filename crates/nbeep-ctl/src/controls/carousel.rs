//! 캐러셀 — **가로 아이템 띠 + 고정 위치 좌/우 오버레이 버튼**(08-14 사용자 확정 2차).
//!
//! 1차(아이템 단위 창 이동)에서 개정된 규칙:
//! - **버튼 위치는 항상 고정**(좌 = 띠 왼끝 · 우 = 띠 오른끝). 표시/숨김만 토글되고,
//!   보일 때는 언제나 같은 자리다(끝에 닿으면 그쪽 버튼 숨김 · 안 넘치면 둘 다 없음).
//! - 버튼 영역도 **내용 영역이다** — 아이템이 그 밑을 지나가고, 버튼은 **맨 위**에
//!   얹힌다(스크롤 중 반쯤 걸친 아이템 위로 버튼이 보인다).
//! - 스크롤은 **픽셀 단위**(부드러운 이동 — 아이템이 절반만 보여도 된다).
//!
//! **아이템 그리기는 소유자 몫**이다(조합/위임): [`Carousel::item_rect`]로 자리를 받아
//! 그리고, [`Carousel::paint`]가 경계 마스크와 버튼을 맨 위에 얹는다. 소유자 그리기가
//! 띠 밖으로 번진 부분(클립 없는 렌더 경로)은 paint의 **경계 마스크**(배경색)가 덮는다.
//! 클릭은 [`Carousel::take_clicked`](1회성 · 전역 인덱스)로 회수한다.

use super::{Control, ControlBase};
use crate::draw::DrawCtx;
use crate::event::{InputEvent, WheelAccum};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 캐러셀 컨트롤 — 정사각 아이템 가로 띠(픽셀 스크롤).
#[derive(Debug)]
pub struct Carousel {
    base: ControlBase,
    /// 아이템 한 변(논리 px — 배율 전 값).
    item_px: i32,
    /// 아이템 간격(논리 px).
    gap: i32,
    /// 전체 아이템 수(소유자가 갱신).
    count: usize,
    /// 스크롤 오프셋(물리 px · 0 = 왼쪽 끝).
    scroll_px: i32,
    /// 아이템 클릭(전역 인덱스 · 1회성).
    clicked: Option<usize>,
    /// 커서가 띠 위에 있는가 — 가로 휠 스크롤은 이 위에서만.
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
            scroll_px: 0,
            clicked: None,
            hover: false,
            hwheel: WheelAccum::default(),
        }
    }

    /// 전체 아이템 수 갱신 — 줄어들면 스크롤을 안쪽으로 되민다.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        self.clamp();
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

    /// 내용 전체 폭(물리 px).
    fn content_w(&self) -> i32 {
        if self.count == 0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let n = self.count as i32;
        n * self.item_w() + (n - 1) * self.gap_w()
    }

    /// 스크롤 상한(0 = 안 넘침).
    fn max_scroll(&self) -> i32 {
        (self.content_w() - self.base.bounds.w).max(0)
    }

    fn clamp(&mut self) {
        self.scroll_px = self.scroll_px.clamp(0, self.max_scroll());
    }

    /// i번째(전역) 아이템의 자리 — 띠와 겹치지 않으면 `None`(부분 겹침은 준다 —
    /// 픽셀 스크롤이라 반쯤 보이는 아이템이 정상이다).
    #[must_use]
    pub fn item_rect(&self, i: usize) -> Option<Rect> {
        if i >= self.count {
            return None;
        }
        let b = self.base.bounds;
        let d = self.item_w();
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let off = (d + self.gap_w()) * i as i32;
        let x = b.x + off - self.scroll_px;
        let y = b.y + (b.h - d) / 2;
        let r = Rect::new(x, y, d, d);
        r.intersects(&b).then_some(r)
    }

    /// 좌 이동 버튼(고정 위치 — 표시 중일 때만 `Some`).
    #[must_use]
    pub fn left_rect(&self) -> Option<Rect> {
        (self.scroll_px > 0).then(|| {
            let b = self.base.bounds;
            Rect::new(b.x, b.y, self.btn_w(), b.h)
        })
    }

    /// 우 이동 버튼(고정 위치 — 표시 중일 때만 `Some`).
    #[must_use]
    pub fn right_rect(&self) -> Option<Rect> {
        (self.scroll_px < self.max_scroll()).then(|| {
            let b = self.base.bounds;
            Rect::new(b.right() - self.btn_w(), b.y, self.btn_w(), b.h)
        })
    }

    /// 한 페이지 이동(버튼) — 버튼 밑 가려지는 폭을 뺀 가시 폭만큼, 최소 한 아이템.
    fn page(&mut self, dir: i32, inv: &mut Invalidations) {
        let step = (self.base.bounds.w - 2 * (self.btn_w() + self.gap_w()))
            .max(self.item_w() + self.gap_w());
        self.scroll_px += dir * step;
        self.clamp();
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
        self.clamp();
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::MouseMove { x, y } => {
                self.hover = self.base.bounds.contains(Point { x, y });
                return;
            }
            // 트랙패드 가로 스크롤 — 띠 위에서만 · 노치 = 아이템 한 칸(부드러운
            // 픽셀 단위는 delta 자체가 분수 노치라 칸 단위로도 충분히 미끄럽다).
            InputEvent::HWheel { delta } if self.hover => {
                let steps = self.hwheel.add(delta, 1);
                if steps != 0 {
                    self.scroll_px += steps * (self.item_w() + self.gap_w());
                    self.clamp();
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
        // 버튼이 맨 위 — 아이템보다 먼저 판정한다(겹친다).
        if self.left_rect().is_some_and(|r| r.contains(p)) {
            self.page(-1, inv);
            return;
        }
        if self.right_rect().is_some_and(|r| r.contains(p)) {
            self.page(1, inv);
            return;
        }
        if !self.base.bounds.contains(p) {
            return;
        }
        for i in 0..self.count {
            if self.item_rect(i).is_some_and(|r| r.contains(p)) {
                self.clicked = Some(i);
                inv.push(self.base.bounds);
                return;
            }
        }
    }

    /// 경계 마스크 + 버튼을 그린다 — 아이템은 소유자가 [`Carousel::item_rect`]로
    /// 그린 **뒤에** 부른다(버튼 = 맨 위 · 08-14 사용자 확정).
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        // 경계 마스크 — 클립 없는 렌더 경로가 띠 밖으로 번진 부분을 배경색으로 덮는다
        // (한 아이템 폭이면 충분 — 부분 겹침 아이템만 번진다).
        let bleed = self.item_w() + self.gap_w();
        ctx.fill_rect(Rect::new(b.x - bleed, b.y, bleed, b.h), theme.panel_bg);
        ctx.fill_rect(Rect::new(b.right(), b.y, bleed, b.h), theme.panel_bg);
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
                theme.text,
                w,
            );
        };
        // 버튼 = 맨 위 오버레이(고정 위치 · 반쯤 걸친 아이템을 덮는다).
        if let Some(r) = self.left_rect() {
            ctx.fill_round_rect(r, self.s(4), theme.field_bg);
            ctx.stroke_round_rect(r, self.s(4), theme.border, 1.0);
            chevron(ctx, r, 1); // ◀
        }
        if let Some(r) = self.right_rect() {
            ctx.fill_round_rect(r, self.s(4), theme.field_bg);
            ctx.stroke_round_rect(r, self.s(4), theme.border, 1.0);
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

    fn click(c: &mut Carousel, r: Rect) {
        let mut inv = Invalidations::default();
        c.on_event(
            &InputEvent::MouseDown {
                x: r.x + r.w / 2,
                y: r.y + r.h / 2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
    }

    #[test]
    fn no_buttons_when_content_fits() {
        let c = car(400, 5); // 5×36-4 = 176 ≤ 400
        assert!(c.left_rect().is_none(), "왼쪽 끝 = 좌버튼 없음");
        assert!(c.right_rect().is_none(), "안 넘침 = 우버튼 없음");
        assert!(c.item_rect(0).is_some() && c.item_rect(4).is_some());
    }

    #[test]
    fn buttons_are_fixed_at_edges_and_toggle() {
        let mut c = car(200, 16); // 넘친다
        assert!(c.left_rect().is_none(), "왼쪽 끝 = 좌버튼 숨김");
        let r0 = c.right_rect().expect("넘침 = 우버튼");
        assert_eq!(r0.right(), 200, "우버튼 = 오른끝 고정");
        assert_eq!(c.item_rect(0).unwrap().x, 0, "내용은 띠 전체를 쓴다");
        // ▶ 페이지 이동 → 좌버튼 등장(왼끝 고정 위치).
        click(&mut c, r0);
        let l = c.left_rect().expect("이동 후 = 좌버튼");
        assert_eq!(l.x, 0, "좌버튼 = 왼끝 고정");
        assert_eq!(
            c.right_rect().expect("아직 오른쪽 남음"),
            r0,
            "우버튼 위치 불변(고정)"
        );
        assert!(c.item_rect(0).is_none(), "앞 아이템은 화면 밖");
    }

    #[test]
    fn right_edge_hides_right_button_only() {
        let mut c = car(200, 16);
        for _ in 0..12 {
            let Some(r) = c.right_rect() else { break };
            click(&mut c, r);
        }
        assert!(c.right_rect().is_none(), "오른쪽 끝 = 우버튼 숨김");
        assert!(c.left_rect().is_some(), "좌버튼만");
        let last = c.item_rect(15).expect("마지막 아이템이 보인다");
        assert_eq!(last.right(), 200, "오른끝에 정확히 붙는다(픽셀 클램프)");
    }

    #[test]
    fn trackpad_hwheel_scrolls_items_under_cursor() {
        let mut c = car(200, 16);
        let mut inv = Invalidations::default();
        c.on_event(&InputEvent::HWheel { delta: 120 }, &mut inv);
        assert!(c.item_rect(0).is_some(), "밖에서는 안 움직인다");
        c.on_event(&InputEvent::MouseMove { x: 10, y: 10 }, &mut inv);
        c.on_event(&InputEvent::HWheel { delta: 120 }, &mut inv);
        assert!(c.item_rect(0).is_none(), "한 노치 = 한 아이템 전진");
        c.on_event(&InputEvent::HWheel { delta: -120 }, &mut inv);
        assert!(c.item_rect(0).is_some(), "반대 방향 복귀");
    }

    #[test]
    fn item_click_reports_global_index_and_buttons_win_overlap() {
        let mut c = car(200, 16);
        let r1 = c.item_rect(1).unwrap();
        click(&mut c, r1);
        assert_eq!(c.take_clicked(), Some(1));
        assert_eq!(c.take_clicked(), None, "1회성");
        // 우버튼 밑을 지나는 아이템 위 클릭 = 버튼이 이긴다(맨 위 레이어).
        let rb = c.right_rect().unwrap();
        click(&mut c, rb);
        assert_eq!(
            c.take_clicked(),
            None,
            "버튼 영역 클릭은 아이템 클릭이 아니다"
        );
        assert!(c.left_rect().is_some(), "페이지가 넘어갔다");
    }
}
