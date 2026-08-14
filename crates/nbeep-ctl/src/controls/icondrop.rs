//! **IconDropdown — 이미지 드롭다운**(08-15 사용자 요청 · 19번째 컨트롤).
//!
//! 툴바 한 칸 크기의 버튼에 **현재 선택의 아이콘**을 보여주고, 클릭하면 아래로
//! 아이콘+라벨 목록이 펼쳐진다(목록 정렬 방식 선택이 첫 사용처). 아이콘은
//! [`ToolIcon::Mask`](super::ToolIcon)와 같은 규약의 **알파 마스크**(모양만 —
//! 색은 테마 기준색 틴트 · SVG 원본은 `assets/icons-src/`).
//!
//! 팝업은 [`IconDropdown::paint_popup`]으로 **맨 위 레이어에서 다시 그린다**
//! (콤보·컨텍스트 메뉴와 같은 z순서 규약 — 08-13 실기 계보). 선택은
//! [`IconDropdown::take_changed`] 1회성 보고 — 영속·적용은 호스트 몫(위젯은
//! 저장을 모른다).

use std::cell::RefCell;
use std::rc::Rc;

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::{Color, IconImage, Theme};
use crate::widget::{Invalidations, Widget};

use super::{Control, ControlBase};

/// 드롭다운 항목 — 값 + 알파 마스크 아이콘 + 라벨.
#[derive(Clone)]
pub struct IconDropItem {
    /// 저장·보고되는 값(설정 코드).
    pub value: &'static str,
    /// 라벨(팝업 행에 아이콘 옆 표기).
    pub label: String,
    /// 알파 마스크(1채널 `size×size` — `tools/mkicons.sh` 산출물).
    pub alpha: &'static [u8],
    /// 마스크 변 크기(px).
    pub size: u32,
}

impl std::fmt::Debug for IconDropItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconDropItem")
            .field("value", &self.value)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// 이미지 드롭다운 컨트롤.
#[derive(Debug)]
pub struct IconDropdown {
    base: ControlBase,
    items: Vec<IconDropItem>,
    sel: usize,
    open: bool,
    hover: Option<usize>,
    changed: Option<&'static str>,
    scale: f32,
    /// 틴트 캐시(항목×색) — 페인트가 `&self`라 내부 가변(툴바와 같은 문법).
    tint: RefCell<Vec<TintSlot>>,
}

/// 틴트 캐시 한 칸 — (마지막 색, 그 색으로 구운 이미지).
type TintSlot = Option<(Color, Rc<IconImage>)>;

impl IconDropdown {
    /// 항목과 초기 선택값으로 만든다(미지 값 = 첫 항목).
    #[must_use]
    pub fn new(items: Vec<IconDropItem>, value: &str) -> Self {
        let sel = items.iter().position(|it| it.value == value).unwrap_or(0);
        let n = items.len();
        Self {
            base: ControlBase::default(),
            items,
            sel,
            open: false,
            hover: None,
            changed: None,
            scale: 1.0,
            tint: RefCell::new(vec![None; n]),
        }
    }

    /// 배율 지정.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.max(0.5);
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 선택값 지정(설정 hot-swap 동기화 — 다른 경로가 값을 바꿨을 때 호스트가 부른다).
    pub fn set_value(&mut self, value: &str, inv: &mut Invalidations) {
        if let Some(i) = self.items.iter().position(|it| it.value == value) {
            if self.sel != i {
                self.sel = i;
                inv.push(self.base.bounds);
            }
        }
    }

    /// 현재 선택값.
    #[must_use]
    pub fn value(&self) -> &'static str {
        self.items.get(self.sel).map_or("", |it| it.value)
    }

    /// 선택 변경(1회성) — 호스트가 설정 깔때기로 잇는다.
    pub fn take_changed(&mut self) -> Option<&'static str> {
        self.changed.take()
    }

    /// 팝업이 열려 있는가 — 호스트의 모달 캡처·z순서 판단용.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 팝업 패널 영역(열려 있을 때) — 버튼 바로 아래.
    fn popup_rect(&self) -> Rect {
        let b = self.base.bounds;
        let row_h = self.s(ROW_H);
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let n = self.items.len() as i32;
        // 폭 = 아이콘 + 가장 긴 라벨 근사(ASCII 8 · 그 외 15 — ContextMenu와 같은 근사).
        let label_w = self
            .items
            .iter()
            .map(|it| {
                it.label
                    .chars()
                    .map(|c| if c.is_ascii() { 8 } else { 15 })
                    .sum::<i32>()
            })
            .max()
            .unwrap_or(40);
        let w = self.s(ROW_H + 12 + label_w);
        Rect::new(
            b.x,
            b.bottom() + self.s(2),
            w.max(b.w),
            row_h * n + self.s(8),
        )
    }

    fn popup_row(&self, i: usize) -> Rect {
        let p = self.popup_rect();
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let idx = i as i32;
        Rect::new(
            p.x + self.s(4),
            p.y + self.s(4) + self.s(ROW_H) * idx,
            p.w - self.s(8),
            self.s(ROW_H),
        )
    }

    fn tinted(&self, i: usize, color: Color) -> Rc<IconImage> {
        let mut cache = self.tint.borrow_mut();
        if cache.len() != self.items.len() {
            cache.resize(self.items.len(), None);
        }
        if let Some((c, img)) = &cache[i] {
            if *c == color {
                return Rc::clone(img);
            }
        }
        let it = &self.items[i];
        let img = Rc::new(IconImage::from_alpha_tinted(
            it.size,
            it.size,
            it.alpha,
            color.rgb(),
        ));
        cache[i] = Some((color, Rc::clone(&img)));
        img
    }

    /// 팝업 페인트(맨 위 레이어) — 호스트가 다른 위젯을 다 그린 뒤 부른다.
    pub fn paint_popup(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        if !self.open {
            return;
        }
        let p = self.popup_rect();
        ctx.fill_rect(p, theme.panel_bg_alt);
        ctx.stroke_round_rect(p, self.s(4), theme.border, 1.0);
        ctx.select_font(FontSlot::Base, false);
        for (i, it) in self.items.iter().enumerate() {
            let r = self.popup_row(i);
            if self.hover == Some(i) {
                ctx.fill_round_rect_alpha(r, self.s(4), theme.text, 0.12);
            }
            let icon_d = r.h - self.s(6);
            let icon = Rect::new(r.x + self.s(3), r.y + self.s(3), icon_d, icon_d);
            let color = if i == self.sel {
                theme.accent
            } else {
                theme.text
            };
            let img = self.tinted(i, color);
            ctx.image_scaled(icon, &img, p);
            let ty = r.y + (r.h - ctx.text_height()) / 2;
            ctx.text(icon.right() + self.s(8), ty, p, &it.label, color);
        }
    }
}

/// 팝업 행 높이(논리 px).
const ROW_H: i32 = 26;

impl Control for IconDropdown {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for IconDropdown {
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
                let p = Point { x, y };
                if self.open {
                    // 팝업 우선(모달 캡처) — 행 클릭 = 선택, 밖 = 닫기.
                    if let Some(i) = (0..self.items.len()).find(|&i| self.popup_row(i).contains(p))
                    {
                        if self.sel != i {
                            self.sel = i;
                            self.changed = Some(self.items[i].value);
                        }
                    }
                    self.open = false;
                    self.hover = None;
                    inv.push(self.base.bounds);
                    inv.push(self.popup_rect());
                    return;
                }
                if self.base.bounds.contains(p) {
                    self.open = true;
                    inv.push(self.popup_rect());
                }
            }
            InputEvent::MouseMove { x, y } => {
                if self.open {
                    let p = Point { x, y };
                    let over = (0..self.items.len()).find(|&i| self.popup_row(i).contains(p));
                    if over != self.hover {
                        self.hover = over;
                        inv.push(self.popup_rect());
                    }
                }
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } if self.open => {
                self.open = false;
                self.hover = None;
                inv.push(self.popup_rect());
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        // 버튼 = 현재 선택 아이콘 + 우하단 ▾ 캐럿(툴바 슬롯과 같은 시각 문법).
        if self.open {
            ctx.fill_round_rect_alpha(b, self.s(6), theme.text, 0.16);
        }
        let pad = self.s(4);
        let icon = Rect::new(b.x + pad, b.y + pad, b.w - pad * 2, b.h - pad * 2);
        let color = if self.open { theme.accent } else { theme.text };
        if !self.items.is_empty() {
            let img = self.tinted(self.sel, color);
            ctx.image_scaled(icon, &img, b);
        }
        // ▾ 캐럿(작게 · 우하단) — 드롭다운임을 알리는 표식.
        let cw = self.s(6);
        let cx = b.right() - cw - self.s(2);
        let cy = b.bottom() - self.s(5);
        ctx.fill_rect(Rect::new(cx, cy, cw, self.s(2).max(2)), color);
        ctx.fill_rect(
            Rect::new(cx + self.s(2) / 2 + 1, cy + 2, cw - self.s(2) - 2, 2),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &[u8] = &[255; 16];

    fn drop3() -> (IconDropdown, Invalidations) {
        let items = vec![
            IconDropItem {
                value: "a",
                label: "첫째".into(),
                alpha: M,
                size: 4,
            },
            IconDropItem {
                value: "b",
                label: "둘째".into(),
                alpha: M,
                size: 4,
            },
            IconDropItem {
                value: "c",
                label: "셋째".into(),
                alpha: M,
                size: 4,
            },
        ];
        let mut w = IconDropdown::new(items, "b");
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(10, 10, 32, 32), &mut inv);
        (w, inv)
    }
    fn down(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }

    /// 초기값 매칭 · 클릭 열기 · 행 선택 = 값 보고(1회성) · 닫힘.
    #[test]
    fn open_pick_reports_value_once() {
        let (mut w, mut inv) = drop3();
        assert_eq!(w.value(), "b", "초기 선택 = 값 매칭");
        w.on_event(&down(20, 20), &mut inv);
        assert!(w.is_open());
        let r = w.popup_row(2);
        w.on_event(&down(r.x + 4, r.y + 4), &mut inv);
        assert!(!w.is_open(), "선택 = 닫힘");
        assert_eq!(w.take_changed(), Some("c"));
        assert_eq!(w.take_changed(), None, "1회성");
        assert_eq!(w.value(), "c");
    }

    /// 팝업 밖 클릭 = 취소(변경 없음) · Esc도 닫는다.
    #[test]
    fn outside_click_and_escape_close_without_change() {
        let (mut w, mut inv) = drop3();
        w.on_event(&down(20, 20), &mut inv);
        assert!(w.is_open());
        w.on_event(&down(500, 500), &mut inv); // 밖
        assert!(!w.is_open());
        assert_eq!(w.take_changed(), None, "밖 클릭 = 변경 없음");
        w.on_event(&down(20, 20), &mut inv);
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(!w.is_open(), "Esc = 닫힘");
    }

    /// 같은 항목 재선택은 변경 보고가 없다(무의미한 저장·재정렬 방지).
    #[test]
    fn reselecting_same_item_reports_nothing() {
        let (mut w, mut inv) = drop3();
        w.on_event(&down(20, 20), &mut inv);
        let r = w.popup_row(1); // 현재 선택(b)
        w.on_event(&down(r.x + 4, r.y + 4), &mut inv);
        assert_eq!(w.take_changed(), None);
    }

    /// 외부 동기화(set_value) — 설정 화면이 값을 바꾸면 버튼 아이콘도 따라간다.
    #[test]
    fn set_value_syncs_selection() {
        let (mut w, mut inv) = drop3();
        w.set_value("c", &mut inv);
        assert_eq!(w.value(), "c");
        w.set_value("없는값", &mut inv);
        assert_eq!(w.value(), "c", "미지 값 무시(관용)");
    }
}
