//! 오버레이 스크롤바 — macOS식 **반투명 오버레이**(사용자 확정 08-08).
//!
//! - 콘텐츠가 넘쳐도 **스크롤 전엔 보이지 않는다**(식별 안 됨).
//! - 스크롤(휠)·바 근처 접근·드래그 중엔 **콘텐츠 위에 겹쳐** 위치·비율을 보여준다(세로·가로).
//! - 바 위에 마우스가 오면 **더 두껍게** + 클릭 드래그 가능.
//! - 항상 **반투명**.
//!
//! 상태(hover/drag/표시)만 보유하고 **오프셋은 호스트가 소유**한다 — [`ScrollBars::on_event`]에
//! 현재 오프셋을 넣으면 갱신된 오프셋을 돌려준다(스크롤 가능한 어떤 뷰에도 재사용: 갤러리·트리·그리드).

use crate::draw::DrawCtx;
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::Theme;

// 레이아웃 상수(논리 px).
const THIN: i32 = 6;
const THICK: i32 = 11;
const MARGIN: i32 = 2;
const MIN_THUMB: i32 = 28;
/// 반투명도 — 항상 은은하게.
const ALPHA_IDLE: f32 = 0.35;
const ALPHA_HOT: f32 = 0.6;
/// 활동 없음 시 숨김까지의 틱 수(호스트가 ~5Hz로 [`ScrollBars::tick`] 호출 → 약 1.2초).
const HIDE_TICKS: u32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
    V,
    H,
}

/// 오버레이 스크롤바(세로+가로).
#[derive(Clone, Debug, Default)]
pub struct ScrollBars {
    hover: Option<Axis>,
    /// 드래그 중: (축, 잡은 지점 오프셋 = 커서 - 썸 시작).
    drag: Option<(Axis, i32)>,
    /// 스크롤/접근/드래그로 활성화되어 보이는가(1·2단계).
    active: bool,
    /// 활동 없음 카운트다운 — 0이 되면 숨긴다(1→0단계). 활동마다 리셋.
    hide_ticks: u32,
}

/// px 헬퍼.
fn sc(v: i32, scale: f32) -> i32 {
    (v as f32 * scale).round() as i32
}

impl ScrollBars {
    /// **프로그램적 표시** — 사용자 입력이 아니라 코드가 스크롤을 옮겼을 때 부른다
    /// (타이핑으로 가로 스크롤이 따라붙는 경우 등). 이걸 부르지 않으면 막대가
    /// `on_event` 전까지 숨어 있어 "스크롤이 생기지 않는다"로 보인다(08-10 지적).
    pub fn show(&mut self) {
        self.active = true;
        self.hide_ticks = HIDE_TICKS;
    }

    /// 새 스크롤바.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn v_needed(vp: Rect, content_h: i32) -> bool {
        content_h > vp.h
    }
    fn h_needed(vp: Rect, content_w: i32) -> bool {
        content_w > vp.w
    }

    /// 세로 썸 rect(현재 오프셋 기준). 불필요하면 `None`. `width`는 두께.
    fn v_thumb(vp: Rect, content_h: i32, off_y: i32, scale: f32, width: i32) -> Option<Rect> {
        if !Self::v_needed(vp, content_h) {
            return None;
        }
        let track = vp.h;
        let thumb = (track * vp.h / content_h)
            .max(sc(MIN_THUMB, scale))
            .min(track);
        let scrollable = (content_h - vp.h).max(1);
        let travel = (track - thumb).max(0);
        let ty = vp.y + off_y.clamp(0, scrollable) * travel / scrollable;
        let x = vp.right() - width - sc(MARGIN, scale);
        Some(Rect::new(x, ty, width, thumb))
    }

    /// 가로 썸 rect. `height`는 두께.
    fn h_thumb(vp: Rect, content_w: i32, off_x: i32, scale: f32, height: i32) -> Option<Rect> {
        if !Self::h_needed(vp, content_w) {
            return None;
        }
        let track = vp.w;
        let thumb = (track * vp.w / content_w)
            .max(sc(MIN_THUMB, scale))
            .min(track);
        let scrollable = (content_w - vp.w).max(1);
        let travel = (track - thumb).max(0);
        let tx = vp.x + off_x.clamp(0, scrollable) * travel / scrollable;
        let y = vp.bottom() - height - sc(MARGIN, scale);
        Some(Rect::new(tx, y, thumb, height))
    }

    fn clamp(off_x: i32, off_y: i32, vp: Rect, content_w: i32, content_h: i32) -> (i32, i32) {
        (
            off_x.clamp(0, (content_w - vp.w).max(0)),
            off_y.clamp(0, (content_h - vp.h).max(0)),
        )
    }

    /// 이벤트 처리 — 갱신된 `(off_x, off_y, consumed)`. `consumed`면 호스트는 그 이벤트를
    /// 자기 콘텐츠에 다시 쓰지 않는다(드래그가 행 선택으로 새지 않도록).
    #[allow(clippy::too_many_arguments)]
    pub fn on_event(
        &mut self,
        ev: &InputEvent,
        vp: Rect,
        content_w: i32,
        content_h: i32,
        off_x: i32,
        off_y: i32,
        scale: f32,
    ) -> (i32, i32, bool) {
        let thick = sc(THICK, scale);
        let (mut ox, mut oy) = (off_x, off_y);
        match *ev {
            InputEvent::Wheel { delta } => {
                oy -= delta / 3;
                self.wake(); // 0/1→1단계 + 카운트다운 리셋
                let (ox, oy) = Self::clamp(ox, oy, vp, content_w, content_h);
                (ox, oy, Self::v_needed(vp, content_h))
            }
            InputEvent::HWheel { delta } => {
                ox += delta / 3;
                self.wake();
                let (ox, oy) = Self::clamp(ox, oy, vp, content_w, content_h);
                (ox, oy, Self::h_needed(vp, content_w))
            }
            InputEvent::MouseDown { x, y, .. } => {
                let p = Point { x, y };
                if let Some(t) = Self::v_thumb(vp, content_h, oy, scale, thick) {
                    if t.contains(p) {
                        self.drag = Some((Axis::V, y - t.y));
                        self.wake();
                        return (ox, oy, true);
                    }
                }
                if let Some(t) = Self::h_thumb(vp, content_w, ox, scale, thick) {
                    if t.contains(p) {
                        self.drag = Some((Axis::H, x - t.x));
                        self.wake();
                        return (ox, oy, true);
                    }
                }
                (ox, oy, false)
            }
            InputEvent::MouseMove { x, y } => {
                let p = Point { x, y };
                // 드래그 중이면 오프셋 갱신.
                if let Some((axis, grab)) = self.drag {
                    match axis {
                        Axis::V => {
                            if let Some(t) = Self::v_thumb(vp, content_h, oy, scale, thick) {
                                let travel = (vp.h - t.h).max(1);
                                let scrollable = (content_h - vp.h).max(0);
                                oy = (y - grab - vp.y) * scrollable / travel;
                            }
                        }
                        Axis::H => {
                            if let Some(t) = Self::h_thumb(vp, content_w, ox, scale, thick) {
                                let travel = (vp.w - t.w).max(1);
                                let scrollable = (content_w - vp.w).max(0);
                                ox = (x - grab - vp.x) * scrollable / travel;
                            }
                        }
                    }
                    self.wake();
                    let (ox, oy) = Self::clamp(ox, oy, vp, content_w, content_h);
                    return (ox, oy, true);
                }
                // 호버 판정(썸 위 = 2단계 두껍게). 바가 보일 때만 판정한다
                // (0단계에선 접근으로 다시 뜨지 않는다 — 스크롤로만 깨어난다).
                self.hover = None;
                if self.active {
                    if let Some(t) = Self::v_thumb(vp, content_h, oy, scale, thick) {
                        if t.contains(p) {
                            self.hover = Some(Axis::V);
                        }
                    }
                    if self.hover.is_none() {
                        if let Some(t) = Self::h_thumb(vp, content_w, ox, scale, thick) {
                            if t.contains(p) {
                                self.hover = Some(Axis::H);
                            }
                        }
                    }
                    // 호버(2단계)면 살려둔다. 아니면 카운트다운은 tick이 진행.
                    if self.hover.is_some() {
                        self.hide_ticks = HIDE_TICKS;
                    }
                }
                (ox, oy, false)
            }
            InputEvent::MouseUp { .. } => {
                let was = self.drag.is_some();
                self.drag = None;
                if was {
                    self.hide_ticks = HIDE_TICKS;
                }
                (ox, oy, was)
            }
            _ => (ox, oy, false),
        }
    }

    /// 스크롤/드래그 활동 → 표시(1단계) + 숨김 카운트다운 리셋.
    fn wake(&mut self) {
        self.active = true;
        self.hide_ticks = HIDE_TICKS;
    }

    /// 호스트가 주기적으로(~5Hz) 호출 — 활동 없으면 카운트다운, 0이면 숨긴다(1→0단계).
    /// 호버/드래그 중(2단계)엔 유지. 표시 상태가 바뀌면 `true`(재그리기 필요).
    pub fn tick(&mut self) -> bool {
        if !self.active || self.hover.is_some() || self.drag.is_some() {
            return false;
        }
        if self.hide_ticks > 0 {
            self.hide_ticks -= 1;
            false
        } else {
            self.active = false;
            true
        }
    }

    /// 오버레이 렌더 — `active`일 때만 그린다(스크롤 전엔 보이지 않는다).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        ctx: &mut dyn DrawCtx,
        theme: &Theme,
        vp: Rect,
        content_w: i32,
        content_h: i32,
        off_x: i32,
        off_y: i32,
        scale: f32,
    ) {
        if !self.active {
            return;
        }
        let thin = sc(THIN, scale);
        let thick = sc(THICK, scale);
        let radius = thin / 2;
        // 세로.
        if let Some(hit) = Self::v_thumb(vp, content_h, off_y, scale, thick) {
            let hot =
                matches!(self.hover, Some(Axis::V)) || matches!(self.drag, Some((Axis::V, _)));
            let w = if hot { thick } else { thin };
            let x = vp.right() - w - sc(MARGIN, scale);
            let thumb = Rect::new(x, hit.y, w, hit.h);
            let a = if hot { ALPHA_HOT } else { ALPHA_IDLE };
            ctx.fill_round_rect_alpha(thumb, radius, theme.text_dim, a);
        }
        // 가로.
        if let Some(hit) = Self::h_thumb(vp, content_w, off_x, scale, thick) {
            let hot =
                matches!(self.hover, Some(Axis::H)) || matches!(self.drag, Some((Axis::H, _)));
            let h = if hot { thick } else { thin };
            let y = vp.bottom() - h - sc(MARGIN, scale);
            let thumb = Rect::new(hit.x, y, hit.w, h);
            let a = if hot { ALPHA_HOT } else { ALPHA_IDLE };
            ctx.fill_round_rect_alpha(thumb, radius, theme.text_dim, a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Rect {
        Rect::new(0, 0, 200, 100)
    }
    fn wheel(d: i32) -> InputEvent {
        InputEvent::Wheel { delta: d }
    }
    fn down(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }
    fn mv(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseMove { x, y }
    }
    fn up() -> InputEvent {
        InputEvent::MouseUp { x: 0, y: 0 }
    }

    #[test]
    fn hidden_until_scrolled() {
        let sb = ScrollBars::new();
        assert!(!sb.active, "스크롤 전엔 비활성(안 보임)");
    }

    #[test]
    fn wheel_scrolls_and_activates_and_clamps() {
        let mut sb = ScrollBars::new();
        let (_ox, oy, consumed) = sb.on_event(&wheel(-300), vp(), 200, 400, 0, 0, 1.0);
        assert!(consumed, "세로 스크롤 소비");
        assert_eq!(oy, 100, "delta/3=100");
        assert!(sb.active, "스크롤 시 표시");
        // 과도 스크롤 클램프(content_h 400 - vp.h 100 = 300).
        let (_ox, oy, _) = sb.on_event(&wheel(-100_000), vp(), 200, 400, oy, 0, 1.0);
        assert_eq!(oy, 300);
    }

    #[test]
    fn drag_thumb_updates_offset() {
        let mut sb = ScrollBars::new();
        // v_thumb at off 0: thumb top = vp.y = 0. 두께 THICK=11. 썸 폭 안 x=200-11-2=187.
        let t = ScrollBars::v_thumb(vp(), 400, 0, 1.0, 11).unwrap();
        let (_ox, _oy, consumed) = sb.on_event(&down(t.x + 2, t.y + 2), vp(), 200, 400, 0, 0, 1.0);
        assert!(consumed && sb.drag.is_some(), "썸 클릭 = 드래그 시작");
        // 아래로 드래그.
        let (_ox, oy, consumed) = sb.on_event(&mv(t.x + 2, t.y + 40), vp(), 200, 400, 0, 0, 1.0);
        assert!(consumed);
        assert!(oy > 0, "드래그로 오프셋 증가: {oy}");
        // 해제.
        let (_ox, _oy, consumed) = sb.on_event(&up(), vp(), 200, 400, oy, 0, 1.0);
        assert!(consumed && sb.drag.is_none(), "해제 = 드래그 종료");
    }

    #[test]
    fn no_bar_when_content_fits() {
        assert!(ScrollBars::v_thumb(vp(), 80, 0, 1.0, 11).is_none());
        assert!(ScrollBars::h_thumb(vp(), 150, 0, 1.0, 11).is_none());
    }

    #[test]
    fn fades_after_inactivity_ticks() {
        // 스크롤(1단계) 후 활동 없으면 tick이 카운트다운해 0단계로.
        let mut sb = ScrollBars::new();
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        assert!(sb.active, "스크롤 = 1단계 표시");
        let mut changed = false;
        for _ in 0..20 {
            changed |= sb.tick();
        }
        assert!(changed && !sb.active, "활동 없음 → 0단계(숨김)");
    }

    #[test]
    fn hover_keeps_visible_until_unhover() {
        let mut sb = ScrollBars::new();
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        // 세로 썸 위로 호버(2단계) — 아무리 tick해도 유지.
        let t = ScrollBars::v_thumb(vp(), 400, 0, 1.0, 11).unwrap();
        sb.on_event(&mv(t.x + 2, t.y + 2), vp(), 200, 400, 0, 0, 1.0);
        for _ in 0..20 {
            sb.tick();
        }
        assert!(sb.active, "호버 중(2단계)엔 유지");
        // 썸 밖으로 이동(1단계) → 이후 tick으로 숨김.
        sb.on_event(&mv(0, 0), vp(), 200, 400, 0, 0, 1.0);
        for _ in 0..20 {
            sb.tick();
        }
        assert!(!sb.active, "언호버 후 무활동 → 숨김");
    }
}
