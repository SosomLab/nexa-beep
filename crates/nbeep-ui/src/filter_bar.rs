//! **목록 필터 바**(08-22 사용자 확정 · 08-23 아이콘 개편) — 툴바 아래 아이콘 칩 3그룹.
//!
//! - 그룹1 경로: 전체·로컬·서버·인터넷·그룹 / 그룹2 상태: 전체·온라인·오프라인 /
//!   그룹3 신뢰: 전체·인증·핀·신규. 각 그룹 **단일 선택**(라디오), 셋은 AND 결합.
//! - **전부 아이콘**(08-23 사용자 확정 — 높이도 16→[`FILTER_H`]=32 · 텍스트 없음):
//!   기존 자산 재사용(house/waypoints/globe · id 배지 RGBA) + 자산 없는 칩은
//!   원시 도형 작도(온라인=찬 점 · 오프라인=빈 링
//!   — 목록 연결점과 같은 시각 문법). **자산 증가 0**.
//! - hover = **툴팁**(그룹 이름 · 칩 이름 — 팝업 레이어 [`Self::paint_tooltip`]).
//! - 선택은 설정 키(`list.filter.*`)로 영속 — 호스트가 [`Self::set_selection`]으로
//!   복원하고 [`Self::take_changed`]로 변경을 받아 저장·재조립한다.
//! - 칩 좌표는 페인트가 깔고 캐시에 남긴다 — 히트는 그 캐시(복사 버튼 문법).

use crate::draw::{DrawCtx, FontSlot};
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use nbeep_core::{t, Msg};
use std::rc::Rc;

/// 바 높이(논리 px) — 08-23 사용자 확정 2차(아이콘 75% 축소에 맞춰 32→26).
pub const FILTER_H: i32 = 26;

/// 칩 그림 — 자산이 있으면 자산, 없으면 원시 도형 작도.
#[derive(Clone, Copy, Debug)]
enum ChipArt {
    /// 96×96 알파 마스크(상태색 틴트 — 선택 = accent · 평시 = 흐림).
    Mask(&'static [u8]),
    /// 96×96 컬러 RGBA — **알파만 취해** 상태색 틴트(08-23 사용자 확정: 원색
    /// 그대로 두면 비선택 인증(파랑)이 선택 accent와 같은 색으로 오인된다 —
    /// 다른 아이콘과 동일하게 비선택 = 회색).
    Rgba(&'static [u8]),
    /// 온라인 = 찬 점(목록 연결점과 같은 문법).
    DotFilled,
    /// 오프라인 = 빈 링.
    DotRing,
}

/// 칩 하나 — (저장 값, 이름, 그림).
type Chip = (&'static str, Msg, ChipArt);
/// 그룹 정의 — (설정 키, 그룹 이름, 칩들).
const GROUPS: [(&str, Msg, &[Chip]); 3] = [
    (
        "list.filter.path",
        Msg::FltGrpPath,
        &[
            (
                "all",
                Msg::FltAll,
                ChipArt::Mask(crate::icons::path::ALL_ALPHA),
            ),
            (
                "local",
                Msg::FltLocal,
                // Material `lan`(허브 위계 — 08-23 사용자 지정 SVG를 구움).
                ChipArt::Mask(crate::icons::path::LAN_ALPHA),
            ),
            (
                "server",
                Msg::FltServer,
                ChipArt::Mask(crate::icons::path::WAYPOINTS_ALPHA),
            ),
            (
                "internet",
                Msg::FltInternet,
                ChipArt::Mask(crate::icons::path::GLOBE_ALPHA),
            ),
            (
                "group",
                Msg::FltGroup,
                // Material `group`(08-23 사용자 지정 SVG — 원시 도형 폐기).
                ChipArt::Mask(crate::icons::path::GROUP_ALPHA),
            ),
        ],
    ),
    (
        "list.filter.presence",
        Msg::FltGrpPresence,
        &[
            (
                "all",
                Msg::FltAll,
                ChipArt::Mask(crate::icons::path::ALL_ALPHA),
            ),
            ("online", Msg::FltOnline, ChipArt::DotFilled),
            ("offline", Msg::FltOffline, ChipArt::DotRing),
        ],
    ),
    (
        "list.filter.trust",
        Msg::FltGrpTrust,
        &[
            (
                "all",
                Msg::FltAll,
                ChipArt::Mask(crate::icons::path::ALL_ALPHA),
            ),
            (
                "verified",
                Msg::FltVerified,
                ChipArt::Rgba(crate::icons::id::VERIFIED_RGBA),
            ),
            (
                "pinned",
                Msg::FltPinned,
                ChipArt::Rgba(crate::icons::id::PINNED_RGBA),
            ),
            (
                "new",
                Msg::FltNew,
                ChipArt::Rgba(crate::icons::id::FIRSTCONTACT_RGBA),
            ),
        ],
    ),
];

/// 자산 변 크기(전 자산 공통 96).
const ART_SIDE: u32 = 96;

/// 필터 바 위젯.
#[derive(Debug, Default)]
pub struct FilterBarWidget {
    bounds: Rect,
    scale: f32,
    /// 그룹별 선택 인덱스.
    sel: [usize; 3],
    /// 페인트가 깐 칩 좌표 캐시 — (그룹, 칩, 사각형).
    chips: core::cell::RefCell<Vec<(usize, usize, Rect)>>,
    /// 틴트 캐시(마스크) — (평면 인덱스, 색) → 이미지.
    tint: core::cell::RefCell<std::collections::HashMap<(usize, u32), Rc<IconImage>>>,
    hover: Option<(usize, usize)>,
    /// 변경 보고(1회성) — (설정 키, 저장 값).
    changed: Option<(&'static str, &'static str)>,
}

impl FilterBarWidget {
    /// 만든다(전부 "전체").
    #[must_use]
    pub fn new() -> Self {
        Self {
            scale: 1.0,
            ..Self::default()
        }
    }

    /// 설정값 복원 — 미지 값은 "전체"로(관용 파싱).
    pub fn set_selection(
        &mut self,
        path: &str,
        presence: &str,
        trust: &str,
        inv: &mut Invalidations,
    ) {
        for (gi, want) in [path, presence, trust].iter().enumerate() {
            self.sel[gi] = GROUPS[gi]
                .2
                .iter()
                .position(|(v, _, _)| v == want)
                .unwrap_or(0);
        }
        inv.push(self.bounds);
    }

    /// 배율.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        inv.push(self.bounds);
    }

    /// 변경 보고(1회성) — (설정 키, 값).
    pub fn take_changed(&mut self) -> Option<(&'static str, &'static str)> {
        self.changed.take()
    }

    /// hover 칩의 툴팁(08-23) — **팝업 레이어**에서 부른다(아래 목록 위에 얹힘).
    /// 형식 = 칩 이름만(그룹 접두 없음 — 08-23 사용자 확정: 그룹 이름은 바에
    /// 이미 보인다).
    pub fn paint_tooltip(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        if let Some((gi, ci)) = self.hover {
            let (_, _, group) = GROUPS[gi];
            if let Some((_, label, _)) = group.get(ci) {
                let anchor = self
                    .chips
                    .borrow()
                    .iter()
                    .find(|(g, c, _)| (*g, *c) == (gi, ci))
                    .map(|(_, _, r)| *r);
                if let Some(anchor) = anchor {
                    crate::draw::draw_tooltip(
                        ctx,
                        theme,
                        anchor,
                        self.bounds.right(),
                        t(*label),
                        self.scale,
                    );
                }
            }
        }
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 마스크 틴트(캐시) — flat = 그룹×16+칩.
    fn tinted(
        &self,
        flat: usize,
        alpha: &'static [u8],
        color: crate::theme::Color,
    ) -> Rc<IconImage> {
        let key = (flat, color.0);
        if let Some(img) = self.tint.borrow().get(&key) {
            return Rc::clone(img);
        }
        let (r, g, b) = color.rgb();
        let mut rgba = Vec::with_capacity(alpha.len() * 4);
        for &a in alpha {
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        let img = Rc::new(IconImage::from_rgba(ART_SIDE, ART_SIDE, rgba));
        self.tint.borrow_mut().insert(key, Rc::clone(&img));
        img
    }

    /// 컬러 RGBA의 **알파 채널만** 취해 상태색으로 틴트(캐시 — 마스크와 같은 결).
    fn tinted_rgba(
        &self,
        flat: usize,
        bytes: &'static [u8],
        color: crate::theme::Color,
    ) -> Rc<IconImage> {
        let key = (flat, color.0);
        if let Some(img) = self.tint.borrow().get(&key) {
            return Rc::clone(img);
        }
        let (r, g, b) = color.rgb();
        let mut rgba = Vec::with_capacity(bytes.len());
        for px in bytes.chunks_exact(4) {
            rgba.extend_from_slice(&[r, g, b, px[3]]);
        }
        let img = Rc::new(IconImage::from_rgba(ART_SIDE, ART_SIDE, rgba));
        self.tint.borrow_mut().insert(key, Rc::clone(&img));
        img
    }

    /// hover 무효화 띠 — 툴팁이 바 아래로 나간다(재도색 범위 포함).
    fn hover_band(&self) -> Rect {
        let b = self.bounds;
        Rect::new(b.x, b.y, b.w, b.h + self.s(34))
    }
}

impl Widget for FilterBarWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::MouseMove { x, y } => {
                let p = Point { x, y };
                let hit = self
                    .chips
                    .borrow()
                    .iter()
                    .find(|(_, _, r)| r.contains(p))
                    .map(|(g, c, _)| (*g, *c));
                if hit != self.hover {
                    self.hover = hit;
                    inv.push(self.hover_band());
                }
            }
            InputEvent::MouseDown { x, y, .. } => {
                let p = Point { x, y };
                let hit = self
                    .chips
                    .borrow()
                    .iter()
                    .find(|(_, _, r)| r.contains(p))
                    .map(|(g, c, _)| (*g, *c));
                if let Some((gi, ci)) = hit {
                    if self.sel[gi] != ci {
                        self.sel[gi] = ci;
                        let (key, _, chips) = GROUPS[gi];
                        self.changed = Some((key, chips[ci].0));
                        inv.push(self.bounds);
                    }
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        if b.is_empty() {
            return;
        }
        ctx.fill_rect(b, theme.chrome_bg);
        ctx.fill_rect(Rect::new(b.x, b.bottom() - 1, b.w, 1), theme.border);
        let mut chips = self.chips.borrow_mut();
        chips.clear();
        let side = b.h - self.s(6) - 1; // 칩 정사각(위 3 · 아래 3+경계선)
        let icon_d = self.s(16); // 아이콘 실크기(칩 안 중앙 — 08-23 확정 16)
        let mut x = b.x + self.s(8);
        for (gi, (_, gmsg, group)) in GROUPS.iter().enumerate() {
            if gi > 0 {
                // 그룹 구분 — 세로 1px 실선.
                let dx = x + self.s(4);
                ctx.fill_rect(
                    Rect::new(dx, b.y + self.s(6), 1, b.h - self.s(12) - 1),
                    theme.border,
                );
                x = dx + self.s(5);
            }
            // 그룹 이름(08-23 사용자 확정 — Location/Status/Trust 소형 텍스트를
            // 그룹 칩 앞에).
            {
                ctx.select_font(FontSlot::Status, false);
                let gname = t(*gmsg);
                let gh = ctx.text_height();
                ctx.text(x, b.y + (b.h - 1 - gh) / 2, b, gname, theme.text_dim);
                x += ctx.text_width(gname) + self.s(6);
            }
            for (ci, (_, _, art)) in group.iter().enumerate() {
                let chip = Rect::new(x, b.y + self.s(3), side, side);
                let selected = self.sel[gi] == ci;
                let hovered = self.hover == Some((gi, ci));
                if selected {
                    ctx.fill_round_rect_alpha(chip, self.s(6), theme.accent, 0.18);
                } else if hovered {
                    ctx.fill_round_rect_alpha(chip, self.s(6), theme.text, 0.08);
                }
                let color = if selected {
                    theme.accent
                } else if hovered {
                    theme.text
                } else {
                    theme.text_dim
                };
                let icon = Rect::new(
                    chip.x + (chip.w - icon_d) / 2,
                    chip.y + (chip.h - icon_d) / 2,
                    icon_d,
                    icon_d,
                );
                let flat = gi * 16 + ci;
                match art {
                    ChipArt::Mask(alpha) => {
                        let img = self.tinted(flat, alpha, color);
                        ctx.image_scaled(icon, &img, chip);
                    }
                    ChipArt::Rgba(bytes) => {
                        let img = self.tinted_rgba(flat, bytes, color);
                        ctx.image_scaled(icon, &img, chip);
                    }
                    ChipArt::DotFilled => {
                        // 정규화 크기의 95%(08-23 사용자 확정 — 찬 면적이라 커 보임).
                        let d = ((icon_d - self.s(2)) * 95) / 100;
                        ctx.fill_ellipse(
                            Rect::new(icon.x + (icon_d - d) / 2, icon.y + (icon_d - d) / 2, d, d),
                            color,
                        );
                    }
                    ChipArt::DotRing => {
                        // 정규화 크기의 95%(08-23 — DotFilled와 동일 비율).
                        let d = ((icon_d - self.s(2)) * 95) / 100;
                        ctx.stroke_ellipse(
                            Rect::new(icon.x + (icon_d - d) / 2, icon.y + (icon_d - d) / 2, d, d),
                            color,
                            (self.s(2).max(2)) as f32,
                        );
                    }
                }
                chips.push((gi, ci, chip));
                x = chip.right(); // 같은 그룹 = 밀착(08-23 확정 — 세그먼트 문법)
            }
        }
        // 폰트 상태 복원(다음 위젯 대비).
        ctx.select_font(FontSlot::Base, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    /// 최소 목업 — 칩 좌표 캐시를 깔기 위한 것뿐.
    struct FlatCtx;
    impl DrawCtx for FlatCtx {
        fn fill_rect(&mut self, _r: Rect, _c: Color) {}
        fn text_opaque(&mut self, _x: i32, _y: i32, _c: Rect, _t: &str, _f: Color, _b: Color) {}
        fn text(&mut self, _x: i32, _y: i32, _c: Rect, _t: &str, _f: Color) {}
        fn text_width(&mut self, text: &str) -> i32 {
            i32::try_from(text.chars().count()).unwrap_or(0) * 8
        }
    }

    fn bar() -> (FilterBarWidget, Invalidations) {
        let mut w = FilterBarWidget::new();
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 800, FILTER_H), &mut inv);
        (w, inv)
    }

    /// 칩 클릭 = (설정 키, 값) 1회 보고 · 같은 칩 재클릭 = 무보고.
    #[test]
    fn chip_click_reports_key_value_once() {
        let (mut w, mut inv) = bar();
        let mut ctx = FlatCtx;
        w.paint(&mut ctx, &Theme::light()); // 좌표 캐시를 깐다
        let chips: Vec<(usize, usize, Rect)> = w.chips.borrow().clone();
        // 그룹1의 "server"(인덱스 2) 클릭.
        let (_, _, r) = chips.iter().find(|(g, c, _)| *g == 0 && *c == 2).unwrap();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        assert_eq!(w.take_changed(), Some(("list.filter.path", "server")));
        assert!(w.take_changed().is_none(), "1회성");
        // 같은 칩 재클릭 = 변경 없음.
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        assert!(w.take_changed().is_none(), "동일 선택 재클릭 = 무보고");
    }

    /// 설정 복원 — 미지 값은 전체(0)로.
    #[test]
    fn selection_restores_and_tolerates_unknown() {
        let (mut w, mut inv) = bar();
        w.set_selection("internet", "online", "bogus", &mut inv);
        assert_eq!(w.sel, [3, 1, 0]);
    }
}
