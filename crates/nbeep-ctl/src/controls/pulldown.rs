//! **메뉴바(Pull-down 메뉴)** — 일반적인 메뉴바를 자체 렌더로 그린다(사용자 요청 08-09).
//!
//! **Windows 스타일 + 크로스플랫폼(DR-6)**: OS 네이티브 메뉴를 쓰지 않고 창 안에 직접
//! 그린다 — 상단 바에 평평한 라벨을 나열하고, 클릭하면 아래로 드롭다운이 열린다.
//! 열려 있는 동안 다른 라벨에 hover만 해도 그 메뉴로 전환(표준 메뉴바 동작).
//! 항목은 값이 아니라 **액션** — 고르는 즉시 [`MenuBar::take_picked`] 1회성 보고 후 닫힌다.
//!
//! 라벨/팝업 폭은 페인트 시점에 실제 글꼴로 측정해 캐시한다(`RefCell` — 첫 페인트 전엔
//! 문자폭 추정치 사용). 공통 기능(활성·배율)은 [`Control`] 상속.

use super::{image_fit_contain, ComboItem, Control, ControlBase, LEADING_ICON};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use std::cell::RefCell;

/// 드롭다운 한 줄 — 액션 항목 또는 구분선.
#[derive(Clone, Debug)]
pub enum MenuEntry {
    /// 액션 항목(값 = 보고 id · 라벨 · 선택적 앞 이미지).
    Item(ComboItem),
    /// 구분선.
    Separator,
}

/// 최상위 메뉴 하나 — 바 라벨 + 드롭다운 항목들.
#[derive(Clone, Debug)]
pub struct MenuDef {
    /// 바에 보이는 라벨.
    pub label: String,
    /// 드롭다운 내용.
    pub entries: Vec<MenuEntry>,
}

impl MenuDef {
    /// 라벨과 항목으로 만든다.
    #[must_use]
    pub fn new(label: impl Into<String>, entries: Vec<MenuEntry>) -> Self {
        Self {
            label: label.into(),
            entries,
        }
    }
}

const ITEM_H: i32 = 26;
const SEP_H: i32 = 7;
const LABEL_PAD: i32 = 12;
const POPUP_PAD: i32 = 4;
const POPUP_MIN_W: i32 = 160;

/// 메뉴바 컨트롤.
#[derive(Debug)]
pub struct MenuBar {
    base: ControlBase,
    menus: Vec<MenuDef>,
    /// 열린 최상위 메뉴 index.
    open: Option<usize>,
    hover_top: Option<usize>,
    /// 열린 드롭다운 안의 hover 항목(entries index).
    hover_item: Option<usize>,
    picked: Option<String>,
    /// 페인트 시 측정한 (라벨 폭들, 팝업 내용 폭들) 캐시 — 측정 전엔 추정치.
    measured: RefCell<(Vec<i32>, Vec<i32>)>,
}

impl MenuBar {
    /// 메뉴 목록으로 만든다.
    #[must_use]
    pub fn new(menus: Vec<MenuDef>) -> Self {
        Self {
            base: ControlBase::default(),
            menus,
            open: None,
            hover_top: None,
            hover_item: None,
            picked: None,
            measured: RefCell::new((Vec::new(), Vec::new())),
        }
    }

    /// 메뉴 전체 교체(i18n 언어 전환 등) — 열림 상태·측정 캐시 초기화.
    pub fn set_menus(&mut self, menus: Vec<MenuDef>) {
        self.menus = menus;
        self.open = None;
        self.hover_top = None;
        self.hover_item = None;
        let mut m = self.measured.borrow_mut();
        m.0.clear();
        m.1.clear();
    }

    /// 드롭다운이 열려 있는가(모달 캡처·최상위 재도색 근거).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// 골라진 액션 값(1회성) — 호스트가 실행.
    pub fn take_picked(&mut self) -> Option<String> {
        self.picked.take()
    }

    /// 문자폭 추정(측정 전 폴백) — ASCII 7 · 그 외(CJK 등) 14.
    fn estimate_w(&self, text: &str) -> i32 {
        let units: i32 = text
            .chars()
            .map(|c| if c.is_ascii() { 7 } else { 14 })
            .sum();
        self.s(units)
    }

    fn label_w(&self, i: usize) -> i32 {
        let cached = self.measured.borrow().0.get(i).copied();
        let text_w =
            cached.unwrap_or_else(|| self.menus.get(i).map_or(0, |m| self.estimate_w(&m.label)));
        text_w + self.s(LABEL_PAD) * 2
    }

    fn label_rect(&self, i: usize) -> Rect {
        let b = self.base.bounds;
        let mut x = b.x + self.s(4);
        for j in 0..i {
            x += self.label_w(j);
        }
        Rect::new(x, b.y, self.label_w(i), b.h)
    }

    fn label_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..self.menus.len()).find(|&i| self.label_rect(i).contains(Point { x, y }))
    }

    fn entry_h(&self, e: &MenuEntry) -> i32 {
        match e {
            MenuEntry::Item(_) => self.s(ITEM_H),
            MenuEntry::Separator => self.s(SEP_H),
        }
    }

    fn popup_rect(&self) -> Rect {
        let Some(i) = self.open else {
            return Rect::new(0, 0, 0, 0);
        };
        let lr = self.label_rect(i);
        let m = &self.menus[i];
        let h: i32 = m.entries.iter().map(|e| self.entry_h(e)).sum::<i32>() + self.s(POPUP_PAD) * 2;
        let cached = self.measured.borrow().1.get(i).copied();
        let content_w = cached.unwrap_or_else(|| {
            m.entries
                .iter()
                .map(|e| match e {
                    MenuEntry::Item(it) => self.estimate_w(&it.label),
                    MenuEntry::Separator => 0,
                })
                .max()
                .unwrap_or(0)
        });
        let w = (content_w + self.s(LEADING_ICON) + self.s(34)).max(self.s(POPUP_MIN_W));
        Rect::new(lr.x, self.base.bounds.bottom(), w, h)
    }

    /// 팝업 좌표 → entries index(구분선 포함 — pick에서 항목만 허용).
    fn entry_at(&self, x: i32, y: i32) -> Option<usize> {
        let i = self.open?;
        let pop = self.popup_rect();
        if !pop.contains(Point { x, y }) {
            return None;
        }
        let mut cy = pop.y + self.s(POPUP_PAD);
        for (k, e) in self.menus[i].entries.iter().enumerate() {
            let h = self.entry_h(e);
            if y >= cy && y < cy + h {
                return Some(k);
            }
            cy += h;
        }
        None
    }

    fn pick(&mut self, entry: usize, inv: &mut Invalidations) {
        if let Some(i) = self.open {
            if let Some(MenuEntry::Item(it)) = self.menus[i].entries.get(entry) {
                self.picked = Some(it.value.clone());
                self.close(inv);
            }
        }
    }

    fn close(&mut self, inv: &mut Invalidations) {
        self.open = None;
        self.hover_item = None;
        inv.push(self.base.bounds);
    }

    fn open_menu(&mut self, i: usize, inv: &mut Invalidations) {
        self.open = Some(i);
        self.hover_item = None;
        inv.push(self.base.bounds);
    }

    /// 다음/이전 **항목**(구분선 건너뜀) entries index.
    fn step_item(&self, from: Option<usize>, down: bool) -> Option<usize> {
        let i = self.open?;
        let entries = &self.menus[i].entries;
        let idxs: Vec<usize> = (0..entries.len())
            .filter(|&k| matches!(entries[k], MenuEntry::Item(_)))
            .collect();
        if idxs.is_empty() {
            return None;
        }
        let pos = from.and_then(|f| idxs.iter().position(|&k| k == f));
        Some(match (pos, down) {
            (None, true) => idxs[0],
            (None, false) => *idxs.last()?,
            (Some(p), true) => idxs[(p + 1) % idxs.len()],
            (Some(p), false) => idxs[(p + idxs.len() - 1) % idxs.len()],
        })
    }
}

impl Control for MenuBar {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for MenuBar {
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
                if let Some(i) = self.label_at(x, y) {
                    // 라벨 클릭 = 토글(같은 메뉴 재클릭 = 닫기 — 표준 동작).
                    if self.open == Some(i) {
                        self.close(inv);
                    } else {
                        self.open_menu(i, inv);
                    }
                    return;
                }
                if self.open.is_some() {
                    if let Some(k) = self.entry_at(x, y) {
                        self.pick(k, inv);
                    } else {
                        self.close(inv); // 바깥 클릭 = 닫기
                    }
                }
            }
            InputEvent::MouseMove { x, y } => {
                if self.open.is_some() {
                    // 열림 중 다른 라벨 hover = 그 메뉴로 전환(표준 메뉴바 동작).
                    if let Some(i) = self.label_at(x, y) {
                        if self.open != Some(i) {
                            self.open_menu(i, inv);
                        }
                        return;
                    }
                    let over = self.entry_at(x, y);
                    if over != self.hover_item {
                        self.hover_item = over;
                        inv.push(self.popup_rect());
                    }
                } else {
                    let over = self.label_at(x, y);
                    if over != self.hover_top {
                        self.hover_top = over;
                        inv.push(self.base.bounds);
                    }
                }
            }
            InputEvent::Key { key, .. } if self.open.is_some() => match key {
                Key::Escape => self.close(inv),
                Key::Down => {
                    self.hover_item = self.step_item(self.hover_item, true);
                    inv.push(self.popup_rect());
                }
                Key::Up => {
                    self.hover_item = self.step_item(self.hover_item, false);
                    inv.push(self.popup_rect());
                }
                Key::Left | Key::Right => {
                    if let Some(i) = self.open {
                        let n = self.menus.len();
                        let next = if matches!(key, Key::Right) {
                            (i + 1) % n
                        } else {
                            (i + n - 1) % n
                        };
                        self.open_menu(next, inv);
                    }
                }
                Key::Enter | Key::Space => {
                    if let Some(k) = self.hover_item {
                        self.pick(k, inv);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        // 바 배경 + 아래 경계선.
        ctx.fill_rect(b, theme.chrome_bg);
        ctx.fill_rect(Rect::new(b.x, b.bottom() - 1, b.w, 1), theme.border);

        // 라벨/팝업 폭 실측 캐시 갱신(첫 페인트 이후 hit-test가 정확해진다).
        ctx.select_font(FontSlot::Base, false);
        {
            let mut m = self.measured.borrow_mut();
            m.0 = self
                .menus
                .iter()
                .map(|d| ctx.text_width(&d.label))
                .collect();
            m.1 = self
                .menus
                .iter()
                .map(|d| {
                    d.entries
                        .iter()
                        .map(|e| match e {
                            MenuEntry::Item(it) => ctx.text_width(&it.label),
                            MenuEntry::Separator => 0,
                        })
                        .max()
                        .unwrap_or(0)
                })
                .collect();
        }

        // 텍스트 세로 중앙 = 실측 높이(고정 16 근사 폐기 — 08-09).
        let th = ctx.text_height();
        // 최상위 라벨들 — 평평한 텍스트, 열림 = 선택색 / hover = 옅은 배경(Windows 스타일).
        for (i, d) in self.menus.iter().enumerate() {
            let lr = self.label_rect(i);
            let slot = Rect::new(lr.x, lr.y + 2, lr.w, lr.h - 4);
            if self.open == Some(i) {
                ctx.fill_rect(slot, theme.sel_bg);
            } else if self.hover_top == Some(i) && self.open.is_none() {
                ctx.fill_rect(slot, theme.panel_bg_alt);
            }
            let tw = ctx.text_width(&d.label);
            ctx.text(
                lr.x + (lr.w - tw) / 2,
                lr.y + (lr.h - th) / 2,
                lr,
                &d.label,
                theme.text,
            );
        }

        // 드롭다운 — 직각에 가까운 패널(1px 테두리), 항목 전체폭 하이라이트.
        if let Some(i) = self.open {
            let pop = self.popup_rect();
            ctx.fill_rect(pop, theme.chrome_bg);
            ctx.stroke_round_rect(pop, self.s(2), theme.border, 1.0);
            let mut y = pop.y + self.s(POPUP_PAD);
            for (k, e) in self.menus[i].entries.iter().enumerate() {
                let h = self.entry_h(e);
                match e {
                    MenuEntry::Separator => {
                        ctx.fill_rect(
                            Rect::new(pop.x + self.s(6), y + h / 2, pop.w - self.s(12), 1),
                            theme.border,
                        );
                    }
                    MenuEntry::Item(it) => {
                        let row = Rect::new(pop.x + 1, y, pop.w - 2, h);
                        if self.hover_item == Some(k) {
                            ctx.fill_rect(row, theme.sel_bg);
                        }
                        let cy = row.y + h / 2;
                        let tx = row.x + self.s(10);
                        if let Some(img) = it.image.as_deref() {
                            let isz = self.s(LEADING_ICON);
                            let boxr = Rect::new(tx, cy - isz / 2, isz, isz);
                            let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
                            ctx.image_scaled(fit, img, row);
                        }
                        let tx = tx + self.s(LEADING_ICON) + self.s(6);
                        ctx.text(tx, cy - th / 2, row, &it.label, theme.text);
                    }
                }
                y += h;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> (MenuBar, Invalidations) {
        let mut m = MenuBar::new(vec![
            MenuDef::new(
                "메뉴",
                vec![
                    MenuEntry::Item(ComboItem::new("settings", "설정")),
                    MenuEntry::Item(ComboItem::new("gallery", "갤러리")),
                    MenuEntry::Separator,
                    MenuEntry::Item(ComboItem::new("about", "About")),
                ],
            ),
            MenuDef::new(
                "도움말",
                vec![MenuEntry::Item(ComboItem::new("help", "도움말 보기"))],
            ),
        ]);
        let mut inv = Invalidations::default();
        m.set_bounds(Rect::new(0, 0, 400, 28), &mut inv);
        (m, inv)
    }
    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }
    fn key(key: Key) -> InputEvent {
        InputEvent::Key {
            key,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn click_opens_and_picks_action_once() {
        let (mut m, mut inv) = bar();
        let l0 = m.label_rect(0);
        m.on_event(&click(l0.x + 5, l0.y + 5), &mut inv);
        assert!(m.is_open());
        let pop = m.popup_rect();
        // 두 번째 항목(갤러리) 중앙.
        let y = pop.y + m.s(POPUP_PAD) + m.s(ITEM_H) + m.s(ITEM_H) / 2;
        m.on_event(&click(pop.x + 20, y), &mut inv);
        assert!(!m.is_open(), "선택 = 닫힘");
        assert_eq!(m.take_picked().as_deref(), Some("gallery"));
        assert!(m.take_picked().is_none(), "1회성");
    }

    #[test]
    fn separator_is_not_pickable_and_keyboard_skips_it() {
        let (mut m, mut inv) = bar();
        let l0 = m.label_rect(0);
        m.on_event(&click(l0.x + 5, l0.y + 5), &mut inv);
        // ↓×3 = 설정→갤러리→(구분선 건너뜀)About.
        for _ in 0..3 {
            m.on_event(&key(Key::Down), &mut inv);
        }
        m.on_event(&key(Key::Enter), &mut inv);
        assert_eq!(m.take_picked().as_deref(), Some("about"));
    }

    #[test]
    fn hover_switches_open_menu_and_outside_click_closes() {
        let (mut m, mut inv) = bar();
        let l0 = m.label_rect(0);
        let l1 = m.label_rect(1);
        m.on_event(&click(l0.x + 5, l0.y + 5), &mut inv);
        // 열림 중 두 번째 라벨 hover = 전환.
        m.on_event(
            &InputEvent::MouseMove {
                x: l1.x + 5,
                y: l1.y + 5,
            },
            &mut inv,
        );
        assert!(m.is_open());
        let pop = m.popup_rect();
        assert_eq!(pop.x, l1.x, "팝업이 두 번째 라벨 아래로 이동");
        // 바깥 클릭 = 닫기(선택 없음).
        m.on_event(&click(800, 600), &mut inv);
        assert!(!m.is_open());
        assert!(m.take_picked().is_none());
        // 같은 라벨 재클릭 = 토글.
        m.on_event(&click(l0.x + 5, l0.y + 5), &mut inv);
        m.on_event(&click(l0.x + 5, l0.y + 5), &mut inv);
        assert!(!m.is_open());
    }
}
