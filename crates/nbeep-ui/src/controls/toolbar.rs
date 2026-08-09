//! **툴바** — 이미지 버튼 가로 배열(사용자 요청 08-09 · 메뉴바 아래 배치).
//!
//! 아이콘 크기는 설정으로 지정(`ui.toolbar_size` — 16/24/32/64 · **기본 24**),
//! [`Toolbar::set_icon_size`]로 즉시 반영된다. 아이콘 소스 규약(사용자 확정 08-09):
//! - [`ToolIcon::Image`](PNG 등 RGBA) = **원본 색 그대로**(슬롯에 contain 맞춤).
//! - [`ToolIcon::Mask`](SVG 유래 알파 마스크) = **모양만** — 색은 테마 기준색으로 틴트
//!   (다크 = 밝은 회색 · 라이트 = 아주 어두운 회색 = `Theme::text`) · **hover = 선색 변경**(accent).
//!
//! 상호작용 UX(사용자 요청 08-09): hover = 기준색 반투명 배경(다크/라이트 공용) ·
//! pressed = 더 진한 배경 + 아이콘 1px 내림(눌림 식별). 클릭은 [`Toolbar::take_clicked`] 1회성.

use super::{image_fit_contain, Control, ControlBase};
use crate::draw::DrawCtx;
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::{Color, IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::cell::RefCell;
use std::rc::Rc;

/// 항목 아이콘 종류.
#[derive(Clone, Debug)]
pub enum ToolIcon {
    /// 이미지(투명 배경 RGBA) — **원본 색 그대로**.
    Image(Rc<IconImage>),
    /// SVG 유래 알파 마스크 — **테마 기준색 틴트**(hover/pressed = accent).
    Mask {
        /// 폭(px).
        w: u32,
        /// 높이(px).
        h: u32,
        /// `w*h` 길이의 1채널 커버리지.
        alpha: &'static [u8],
    },
}

/// 툴바 항목 — 액션 id + 아이콘.
#[derive(Clone, Debug)]
pub struct ToolItem {
    /// 액션 id(클릭 보고 값).
    pub id: String,
    /// 아이콘.
    pub icon: ToolIcon,
}

impl ToolItem {
    /// (id, 아이콘)으로 만든다.
    pub fn new(id: impl Into<String>, icon: ToolIcon) -> Self {
        Self {
            id: id.into(),
            icon,
        }
    }
}

/// 슬롯 안쪽 여백(논리 px).
const SLOT_PAD: i32 = 4;
/// 툴바 상하 여백(논리 px).
const BAR_PAD: i32 = 4;
/// 기본 아이콘 크기(논리 px) — 사용자 확정(08-09 · 32→24).
pub const DEFAULT_ICON: i32 = 24;
/// hover 배경 불투명도(기준색 틴트 — 다크/라이트 공용).
const HOVER_BG_ALPHA: f32 = 0.10;
/// pressed 배경 불투명도 — hover보다 진해 눌림이 식별된다.
const PRESS_BG_ALPHA: f32 = 0.20;

/// 항목별 틴트 캐시 슬롯 — (틴트 색, 생성된 이미지).
type TintSlot = Option<(Color, Rc<IconImage>)>;

/// 툴바 컨트롤.
#[derive(Debug)]
pub struct Toolbar {
    base: ControlBase,
    items: Vec<ToolItem>,
    /// 아이콘 한 변(논리 px) — 16/24/32/64.
    icon_px: i32,
    hover: Option<usize>,
    pressed: Option<usize>,
    clicked: Option<String>,
    /// 마스크 틴트 캐시(항목별 · 같은 색이면 재사용) — 페인트가 `&self`라 내부 가변.
    tint: RefCell<Vec<TintSlot>>,
}

impl Toolbar {
    /// 항목으로 만든다(아이콘 기본 24).
    #[must_use]
    pub fn new(items: Vec<ToolItem>) -> Self {
        let n = items.len();
        Self {
            base: ControlBase::default(),
            items,
            icon_px: DEFAULT_ICON,
            hover: None,
            pressed: None,
            clicked: None,
            tint: RefCell::new(vec![None; n]),
        }
    }

    /// 아이콘 크기(논리 px) 지정 — 설정 `ui.toolbar_size` 즉시 적용.
    pub fn set_icon_size(&mut self, px: i32) {
        self.icon_px = px.clamp(12, 128);
    }

    /// 현재 아이콘 크기(논리 px).
    #[must_use]
    pub fn icon_size(&self) -> i32 {
        self.icon_px
    }

    /// 이 아이콘 크기에서의 툴바 권장 높이(논리 px) — 호스트 레이아웃용.
    #[must_use]
    pub fn preferred_height(&self) -> i32 {
        self.icon_px + (SLOT_PAD + BAR_PAD) * 2
    }

    /// 클릭된 액션 id(1회성).
    pub fn take_clicked(&mut self) -> Option<String> {
        self.clicked.take()
    }

    fn slot(&self) -> i32 {
        self.s(self.icon_px + SLOT_PAD * 2)
    }

    fn slot_rect(&self, i: usize) -> Rect {
        let b = self.base.bounds;
        let slot = self.slot();
        let gap = self.s(4);
        Rect::new(
            b.x + self.s(6) + (slot + gap) * i as i32,
            b.y + (b.h - slot) / 2,
            slot,
            slot,
        )
    }

    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..self.items.len()).find(|&i| self.slot_rect(i).contains(Point { x, y }))
    }

    /// 마스크 항목의 틴트 이미지(캐시) — 색이 바뀌면(테마·hover) 다시 만든다.
    fn tinted(
        &self,
        i: usize,
        w: u32,
        h: u32,
        alpha: &'static [u8],
        color: Color,
    ) -> Rc<IconImage> {
        let mut cache = self.tint.borrow_mut();
        if cache.len() != self.items.len() {
            cache.resize(self.items.len(), None);
        }
        if let Some((c, img)) = &cache[i] {
            if *c == color {
                return Rc::clone(img);
            }
        }
        let img = Rc::new(IconImage::from_alpha_tinted(w, h, alpha, color.rgb()));
        cache[i] = Some((color, Rc::clone(&img)));
        img
    }
}

impl Control for Toolbar {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for Toolbar {
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
                if let Some(i) = self.item_at(x, y) {
                    self.pressed = Some(i);
                    inv.push(self.slot_rect(i));
                }
            }
            InputEvent::MouseUp { x, y } => {
                if let Some(i) = self.pressed.take() {
                    if self.item_at(x, y) == Some(i) {
                        self.clicked = Some(self.items[i].id.clone());
                    }
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::MouseMove { x, y } => {
                let over = self.item_at(x, y);
                if over != self.hover {
                    self.hover = over;
                    inv.push(self.base.bounds);
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        ctx.fill_rect(b, theme.chrome_bg);
        ctx.fill_rect(Rect::new(b.x, b.bottom() - 1, b.w, 1), theme.border);
        for (i, it) in self.items.iter().enumerate() {
            let slot = self.slot_rect(i);
            let is_pressed = self.pressed == Some(i);
            let is_hover = self.hover == Some(i);
            // 기준색 반투명 배경 — 다크/라이트 모두 "선택된 느낌".
            if is_pressed {
                ctx.fill_round_rect_alpha(slot, self.s(6), theme.text, PRESS_BG_ALPHA);
            } else if is_hover {
                ctx.fill_round_rect_alpha(slot, self.s(6), theme.text, HOVER_BG_ALPHA);
            }
            let pad = self.s(SLOT_PAD);
            // 눌림 식별 — 아이콘을 1px 내려 그린다.
            let dy = i32::from(is_pressed);
            let icon_area = Rect::new(
                slot.x + pad,
                slot.y + pad + dy,
                slot.w - pad * 2,
                slot.h - pad * 2,
            );
            match &it.icon {
                ToolIcon::Image(img) => {
                    let fit = image_fit_contain(icon_area, img.w as i32, img.h as i32);
                    ctx.image_scaled(fit, img, slot);
                }
                ToolIcon::Mask { w, h, alpha } => {
                    // SVG 유래 = 테마 기준색 · hover/pressed = 선색 변경(accent).
                    let color = if is_hover || is_pressed {
                        theme.accent
                    } else {
                        theme.text
                    };
                    let img = self.tinted(i, *w, *h, alpha, color);
                    let fit = image_fit_contain(icon_area, img.w as i32, img.h as i32);
                    ctx.image_scaled(fit, &img, slot);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::InputEvent;

    /// 4×4 더미 마스크.
    const MASK4: &[u8] = &[255; 16];

    fn bar() -> (Toolbar, Invalidations) {
        let mut t = Toolbar::new(vec![
            ToolItem::new(
                "refresh",
                ToolIcon::Mask {
                    w: 4,
                    h: 4,
                    alpha: MASK4,
                },
            ),
            ToolItem::new(
                "gallery",
                ToolIcon::Image(Rc::new(IconImage::swatch(16, (0, 0, 255)))),
            ),
        ]);
        let mut inv = Invalidations::default();
        t.set_bounds(Rect::new(0, 0, 300, t.preferred_height()), &mut inv);
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
    fn up(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseUp { x, y }
    }

    #[test]
    fn click_reports_action_id_once() {
        let (mut t, mut inv) = bar();
        let s0 = t.slot_rect(0);
        t.on_event(&down(s0.x + 5, s0.y + 5), &mut inv);
        t.on_event(&up(s0.x + 5, s0.y + 5), &mut inv);
        assert_eq!(t.take_clicked().as_deref(), Some("refresh"));
        assert!(t.take_clicked().is_none(), "1회성");
    }

    #[test]
    fn release_outside_cancels() {
        let (mut t, mut inv) = bar();
        let s1 = t.slot_rect(1);
        t.on_event(&down(s1.x + 5, s1.y + 5), &mut inv);
        t.on_event(&up(999, 999), &mut inv);
        assert!(t.take_clicked().is_none());
    }

    #[test]
    fn icon_size_drives_preferred_height_and_slots() {
        let (mut t, _) = bar();
        assert_eq!(t.icon_size(), DEFAULT_ICON, "기본 24");
        let h24 = t.preferred_height();
        t.set_icon_size(64);
        assert!(t.preferred_height() > h24, "64는 24보다 높다");
        t.set_icon_size(16);
        assert_eq!(t.icon_size(), 16);
        assert!(t.preferred_height() < h24);
    }

    #[test]
    fn mask_tint_cache_rebuilds_on_color_change() {
        let (t, _) = bar();
        let a = t.tinted(0, 4, 4, MASK4, Color::from_rgb(200, 200, 200));
        let b = t.tinted(0, 4, 4, MASK4, Color::from_rgb(200, 200, 200));
        assert!(Rc::ptr_eq(&a, &b), "같은 색 = 캐시 재사용");
        let c = t.tinted(0, 4, 4, MASK4, Color::from_rgb(61, 139, 255));
        assert!(!Rc::ptr_eq(&a, &c), "색 변경 = 재생성");
        assert_eq!(c.rgba[0], 61, "틴트 색 반영");
        assert_eq!(c.rgba[3], 255, "마스크 알파 유지");
    }
}
