//! **툴바** — 이미지 버튼 가로 배열(사용자 요청 08-09 · Pull-down 메뉴 아래 배치).
//!
//! 아이콘 크기는 설정으로 지정(`ui.toolbar_size` — 16/24/32/64 · **기본 32**),
//! [`Toolbar::set_icon_size`]로 즉시 반영된다. 항목 아이콘은 이미지([`IconImage`]) 또는
//! 내장 글리프([`ToolIcon::Refresh`] — 폰트 글리프 의존 없이 폴리라인으로 직접 그림).
//! 클릭은 [`Toolbar::take_clicked`] 1회성 보고(액션 id).

use super::{image_fit_contain, Control, ControlBase};
use crate::draw::DrawCtx;
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

/// 항목 아이콘 종류.
#[derive(Clone, Debug)]
pub enum ToolIcon {
    /// 이미지(투명 배경 RGBA · 슬롯에 contain 맞춤).
    Image(Rc<IconImage>),
    /// 내장 새로고침 글리프(원호+화살촉 — 폰트 의존 없음).
    Refresh,
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
/// 기본 아이콘 크기(논리 px) — 사용자 확정.
pub const DEFAULT_ICON: i32 = 32;

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
}

impl Toolbar {
    /// 항목으로 만든다(아이콘 기본 32).
    #[must_use]
    pub fn new(items: Vec<ToolItem>) -> Self {
        Self {
            base: ControlBase::default(),
            items,
            icon_px: DEFAULT_ICON,
            hover: None,
            pressed: None,
            clicked: None,
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
}

/// 새로고침 글리프(↻) — 원호(폴리라인 근사) + 화살촉. 폰트 글리프 의존 없음.
fn draw_refresh(ctx: &mut dyn DrawCtx, area: Rect, color: crate::theme::Color, width: f32) {
    let cx = area.x as f32 + area.w as f32 / 2.0;
    let cy = area.y as f32 + area.h as f32 / 2.0;
    let r = (area.w.min(area.h) as f32 / 2.0) - width - 1.0;
    // 30°→300° 원호를 12분할 폴리라인으로.
    let mut pts = Vec::with_capacity(13);
    for k in 0..=12 {
        let a = (30.0 + 270.0 * (k as f32) / 12.0).to_radians();
        pts.push((cx + r * a.cos(), cy - r * a.sin()));
    }
    let ipts: Vec<(i32, i32)> = pts.iter().map(|&(x, y)| (x as i32, y as i32)).collect();
    ctx.polyline(&ipts, color, width);
    // 화살촉 — 원호 끝(300° = -60°) 접선 방향.
    let end = *pts.last().unwrap_or(&(cx, cy));
    let a = 300.0_f32.to_radians();
    // 접선(시계 반대 진행이므로 -sin, -cos 방향 회전) 기준 두 날개.
    let (tx, ty) = (a.sin(), a.cos()); // 근사 접선
    let l = r * 0.55;
    let head = |ang: f32| {
        let (s, c) = ang.sin_cos();
        (
            (end.0 + l * (tx * c - ty * s)) as i32,
            (end.1 + l * (tx * s + ty * c)) as i32,
        )
    };
    let e = (end.0 as i32, end.1 as i32);
    ctx.polyline(&[head(0.5), e, head(-0.9)], color, width);
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
            if self.pressed == Some(i) {
                ctx.fill_round_rect(slot, self.s(6), theme.sel_bg);
            } else if self.hover == Some(i) {
                ctx.fill_round_rect(slot, self.s(6), theme.panel_bg_alt);
            }
            let pad = self.s(SLOT_PAD);
            let icon_area = Rect::new(
                slot.x + pad,
                slot.y + pad,
                slot.w - pad * 2,
                slot.h - pad * 2,
            );
            match &it.icon {
                ToolIcon::Image(img) => {
                    let fit = image_fit_contain(icon_area, img.w as i32, img.h as i32);
                    ctx.image_scaled(fit, img, slot);
                }
                ToolIcon::Refresh => {
                    let w = (icon_area.w as f32 / 12.0).max(1.6);
                    draw_refresh(ctx, icon_area, theme.text, w);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::InputEvent;

    fn bar() -> (Toolbar, Invalidations) {
        let mut t = Toolbar::new(vec![
            ToolItem::new("refresh", ToolIcon::Refresh),
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
        assert_eq!(t.icon_size(), DEFAULT_ICON);
        let h32 = t.preferred_height();
        t.set_icon_size(64);
        assert!(t.preferred_height() > h32, "64는 32보다 높다");
        t.set_icon_size(16);
        assert_eq!(t.icon_size(), 16);
        assert!(t.preferred_height() < h32);
    }
}
