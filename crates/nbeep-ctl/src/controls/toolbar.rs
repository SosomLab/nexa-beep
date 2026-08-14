//! **툴바** — 이미지 버튼 가로 배열(사용자 요청 08-09 · 메뉴바 아래 배치).
//!
//! 아이콘 크기는 설정으로 지정(`ui.toolbar_size` — 16/24/32/48/64 · **기본 32**),
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
    /// **내 프로필 미니 아바타**(08-14 사용자 요청 — 프로필 버튼이 곧 내 얼굴).
    /// 사진·내장 그림·이니셜·빈 원 + 보더 링(소형 2px)을 아바타 문법 그대로 그린다.
    Avatar {
        /// 사진(원형 마스크 완료본) 또는 내장 12간지(투명 배경). 없으면 이니셜/빈 원.
        img: Option<Rc<IconImage>>,
        /// 이니셜(이름 앞 2자 · 빈 문자열 = 빈 원) — `img` 없을 때만 쓰인다.
        initials: String,
        /// 원 배경·색 시드(내 키 지문).
        seed: Vec<u8>,
        /// 아바타 보더 색(소형이라 2px — 사용자 확정).
        border: Option<Color>,
    },
}

/// 툴바 항목 — 액션 id + 아이콘 (+ 오른쪽 정렬 여부).
#[derive(Clone, Debug)]
pub struct ToolItem {
    /// 액션 id(클릭 보고 값).
    pub id: String,
    /// 아이콘.
    pub icon: ToolIcon,
    /// `true`면 툴바 **오른쪽 끝**부터 배치(08-14 — 프로필 버튼).
    pub right: bool,
}

impl ToolItem {
    /// (id, 아이콘)으로 만든다(왼쪽 정렬).
    pub fn new(id: impl Into<String>, icon: ToolIcon) -> Self {
        Self {
            id: id.into(),
            icon,
            right: false,
        }
    }

    /// 오른쪽 끝 배치 항목(체이닝).
    #[must_use]
    pub fn align_right(mut self) -> Self {
        self.right = true;
        self
    }
}

/// 슬롯 안쪽 여백(논리 px).
const SLOT_PAD: i32 = 4;
/// 툴바 상하 여백(논리 px).
const BAR_PAD: i32 = 4;
/// 기본 아이콘 크기(논리 px) — 사용자 확정(08-14 · 24→**32**).
/// 이력: 08-09에 32→24로 내렸다가, 툴바 프로필 버튼이 **내 아바타**를 쓰게 되면서
/// 24px에서는 그림이 뭉개져 다시 32로 올렸다(선택지에 48도 추가).
pub const DEFAULT_ICON: i32 = 32;
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
    /// 아이콘 한 변(논리 px) — 16/24/32/48/64.
    icon_px: i32,
    hover: Option<usize>,
    pressed: Option<usize>,
    clicked: Option<String>,
    /// 마스크 틴트 캐시(항목별 · 같은 색이면 재사용) — 페인트가 `&self`라 내부 가변.
    tint: RefCell<Vec<TintSlot>>,
}

impl Toolbar {
    /// 항목으로 만든다(아이콘 기본 [`DEFAULT_ICON`]).
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
        let y = b.y + (b.h - slot) / 2;
        if self.items[i].right {
            // 오른쪽 끝부터 — 내 뒤의 right 항목 수만큼 왼쪽으로 밀린다.
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let after = self.items[i + 1..].iter().filter(|it| it.right).count() as i32;
            Rect::new(
                b.right() - self.s(6) - slot - (slot + gap) * after,
                y,
                slot,
                slot,
            )
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let before = self.items[..i].iter().filter(|it| !it.right).count() as i32;
            Rect::new(b.x + self.s(6) + (slot + gap) * before, y, slot, slot)
        }
    }

    /// 좌측 정렬 항목들의 **끝 x**(물리 px · 08-15) — 호스트가 툴바 행에 다른
    /// 컨트롤(정렬 드롭다운 등)을 이어 붙일 때의 기준선.
    #[must_use]
    pub fn left_items_end(&self) -> i32 {
        let n = self.items.iter().filter(|it| !it.right).count();
        if n == 0 {
            return self.base.bounds.x + self.s(6);
        }
        self.slot_rect(n - 1).right()
    }

    /// 좌측 슬롯 한 변(물리 px) — 이어 붙일 컨트롤의 크기 맞춤용.
    #[must_use]
    pub fn slot_px(&self) -> i32 {
        self.slot()
    }

    /// 항목 아이콘 교체(08-14 — 프로필 버튼이 내 얼굴을 따라간다). 미지 id는 무시.
    pub fn set_item_icon(&mut self, id: &str, icon: ToolIcon, inv: &mut Invalidations) {
        if let Some(i) = self.items.iter().position(|it| it.id == id) {
            self.items[i].icon = icon;
            if let Some(slot) = self.tint.borrow_mut().get_mut(i) {
                *slot = None; // 틴트 캐시 무효화
            }
            inv.push(self.base.bounds);
        }
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
                ToolIcon::Avatar {
                    img,
                    initials,
                    seed,
                    border,
                } => {
                    // 내 얼굴 미니(08-14) — 목록 행과 같은 시각 문법(원 배경 + 그림/이니셜).
                    if let Some(img) = img {
                        ctx.fill_ellipse(icon_area, crate::avatar::avatar_color(seed));
                        ctx.image_scaled(icon_area, img, slot);
                    } else if initials.is_empty() {
                        ctx.fill_ellipse(icon_area, crate::avatar::avatar_color(seed));
                    // 빈 원
                    } else {
                        crate::avatar::draw_avatar(ctx, icon_area, initials, seed, 0.0);
                    }
                    if let Some(c) = border {
                        ctx.stroke_ellipse(icon_area, *c, self.s(2).max(2) as f32);
                        // 소형 2px
                    }
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
        assert_eq!(t.icon_size(), DEFAULT_ICON, "기본값은 DEFAULT_ICON");
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
