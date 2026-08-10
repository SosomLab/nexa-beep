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

/// 자동 숨김까지의 기본 지연(ms) — 사용자 확정 08-10. 설정에서 바꾼다.
pub const DEFAULT_HIDE_MS: u64 = 2000;

/// 전역 자동 숨김 지연 — 설정 변경이 **모든 스크롤 영역에 즉시** 반영되도록 프로세스 전역에 둔다
/// (스크롤바는 목록·트리·갤러리·대화·설정에 흩어져 있어, 값을 일일이 들고 다니면
/// 한 군데만 옛 값으로 남는다 — 핫스왑 원칙).
static HIDE_MS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(DEFAULT_HIDE_MS);

/// 자동 숨김 지연을 바꾼다(설정 즉시 적용). 0이면 **숨기지 않는다**(항상 표시).
pub fn set_hide_delay_ms(ms: u64) {
    HIDE_MS.store(ms, core::sync::atomic::Ordering::Relaxed);
}

/// 현재 자동 숨김 지연(ms).
#[must_use]
pub fn hide_delay_ms() -> u64 {
    HIDE_MS.load(core::sync::atomic::Ordering::Relaxed)
}

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
    /// 이 시각(ms)이 지나면 숨긴다(1→0단계). 활동마다 뒤로 민다.
    hide_at_ms: u64,
    /// 활동이 있었다 — 다음 [`ScrollBars::tick`]에서 마감 시각을 다시 잡는다.
    /// (`on_event`는 시계를 모른다. 시각 주입은 호스트가 하는 `tick` 한 곳으로 모은다.)
    bumped: bool,
    /// 모습이 바뀌어 다시 그려야 한다(표시 전환·호버 두께) — `tick`이 호스트에 알린다.
    dirty: bool,
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
        self.wake();
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
                let was_hover = self.hover;
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
                    // 호버가 바뀌면 두께가 바뀐다 — 다시 그려야 보인다.
                    if self.hover != was_hover {
                        self.dirty = true;
                    }
                }
                (ox, oy, false)
            }
            InputEvent::MouseUp { .. } => {
                let was = self.drag.is_some();
                self.drag = None;
                if was {
                    self.wake(); // 놓는 순간부터 다시 카운트 — 곧바로 사라지지 않는다
                }
                (ox, oy, was)
            }
            _ => (ox, oy, false),
        }
    }

    /// 스크롤/드래그 활동 → 표시(1단계) + 숨김 마감 연기.
    fn wake(&mut self) {
        if !self.active {
            self.dirty = true; // 숨김 → 표시 전환은 다시 그려야 보인다
        }
        self.active = true;
        self.bumped = true;
    }

    /// 호스트가 호출 — `now_ms`가 마감을 넘겼고 호버/드래그가 아니면 숨긴다(1→0단계).
    /// 표시 상태가 바뀌면 `true`(재그리기 필요).
    ///
    /// ★ **시간 기반이어야 한다** — 예전에는 호출 횟수를 셌는데, 호스트는 유휴 시 5Hz지만
    /// **이벤트가 들어오면 그때마다** 부른다. 그래서 드래그 중에는 초당 수십 번 깎여
    /// 막대가 0.2초 만에 사라졌다(08-10 지적: "드래그하면 잠깐 보였다 금방 사라짐").
    /// 벽시계로 재면 호출 빈도와 무관하게 항상 설정된 시간만큼 보인다.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let delay = hide_delay_ms();
        let mut redraw = core::mem::take(&mut self.dirty);
        // 활동이 있었거나 호버/드래그 중(2단계)이면 마감을 계속 뒤로 민다.
        if self.bumped || self.hover.is_some() || self.drag.is_some() {
            self.bumped = false;
            self.hide_at_ms = now_ms.saturating_add(delay);
            return redraw;
        }
        // delay 0 = 자동 숨김 안 함(사용자가 항상 보이길 택한 경우).
        if self.active && delay != 0 && now_ms >= self.hide_at_ms {
            self.active = false;
            redraw = true;
        }
        redraw
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

    /// 숨김 지연은 **프로세스 전역**이라, 값을 바꾸는 테스트와 시간에 의존하는 테스트가
    /// 동시에 돌면 서로를 흔든다. 그 테스트들만 이 잠금을 잡는다.
    static DELAY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn lock_delay() -> std::sync::MutexGuard<'static, ()> {
        DELAY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
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
    fn fades_after_the_configured_delay_not_before() {
        let _g = lock_delay();
        let mut sb = ScrollBars::new();
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        assert!(sb.active, "스크롤 = 1단계 표시");
        sb.tick(0); // 마감 = 0 + 2000ms
        assert!(!sb.tick(1999) && sb.active, "지연 이전엔 유지");
        assert!(sb.tick(2000) && !sb.active, "지연이 지나면 숨김");
    }

    #[test]
    fn frequent_ticks_do_not_shorten_the_delay() {
        let _g = lock_delay();
        // ★ 회귀: 예전 구현은 tick **횟수**를 셌다. 호스트는 이벤트마다 tick을 부르므로
        //   드래그 중 초당 수십 번 불려 막대가 0.2초 만에 사라졌다(08-10 지적).
        let mut sb = ScrollBars::new();
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        sb.tick(0);
        for i in 0..500 {
            // 500번 불러도 시계가 1.5초 안이면 살아 있어야 한다.
            assert!(!sb.tick(i * 3), "호출 횟수로 사라지면 안 된다(t={})", i * 3);
        }
        assert!(sb.active);
    }

    #[test]
    fn hide_delay_is_configurable_and_zero_means_always_on() {
        let _g = lock_delay();
        let mut sb = ScrollBars::new();
        set_hide_delay_ms(500);
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        sb.tick(0);
        assert!(!sb.tick(499));
        assert!(sb.tick(500) && !sb.active, "설정한 500ms에 숨는다");
        // 0 = 자동 숨김 없음.
        set_hide_delay_ms(0);
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        sb.tick(0);
        assert!(!sb.tick(u64::MAX) && sb.active, "0이면 숨기지 않는다");
        set_hide_delay_ms(DEFAULT_HIDE_MS); // 전역이라 되돌린다
    }

    #[test]
    fn hover_keeps_visible_until_unhover_and_is_thicker() {
        let _g = lock_delay();
        let mut sb = ScrollBars::new();
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        // 세로 썸 위로 호버(2단계) — 시간이 아무리 흘러도 유지.
        let t = ScrollBars::v_thumb(vp(), 400, 0, 1.0, 11).unwrap();
        sb.on_event(&mv(t.x + 2, t.y + 2), vp(), 200, 400, 0, 0, 1.0);
        assert_eq!(sb.hover, Some(Axis::V), "썸 위 = 호버");
        for i in 0..50 {
            sb.tick(i * 1000);
        }
        assert!(sb.active, "호버 중(2단계)엔 유지 — 사라지지 않는다");
        // 마지막 호버 틱이 t=49_000이었으니 마감은 51_000.
        // 썸 밖으로 이동(1단계) → 남은 지연을 채운 뒤에야 숨는다(즉시 사라지지 않는다).
        sb.on_event(&mv(0, 0), vp(), 200, 400, 0, 0, 1.0);
        assert_eq!(sb.hover, None);
        sb.tick(50_000);
        assert!(sb.active, "언호버 직후엔 아직 지연이 남아 있다");
        assert!(sb.tick(51_000) && !sb.active, "언호버 후 지연 경과 → 숨김");
    }

    #[test]
    fn hover_change_requests_a_redraw() {
        let _g = lock_delay();
        // 두께가 바뀌는데 다시 그리지 않으면 사용자 눈엔 아무 일도 안 일어난다.
        let mut sb = ScrollBars::new();
        sb.on_event(&wheel(-100), vp(), 200, 400, 0, 0, 1.0);
        sb.tick(0);
        let t = ScrollBars::v_thumb(vp(), 400, 0, 1.0, 11).unwrap();
        sb.on_event(&mv(t.x + 2, t.y + 2), vp(), 200, 400, 0, 0, 1.0);
        assert!(sb.tick(1), "호버 진입 = 재그리기 요청");
        sb.on_event(&mv(0, 0), vp(), 200, 400, 0, 0, 1.0);
        assert!(sb.tick(2), "호버 이탈 = 재그리기 요청");
    }
}
