//! **컨텍스트 메뉴**(우클릭 팝업) — 커서 자리에 뜨는 작은 목록.
//!
//! 우클릭이 **곧바로 동작을 실행하면 되돌릴 방법이 없다**. 예전 대화 창은 풍선을
//! 우클릭하는 순간 클립보드를 덮어써서, 사용자가 "복사가 됐는지, 뭐가 됐는지" 알 수 없었다
//! (08-10 지적). 메뉴는 **무엇이 일어날지 먼저 보여주고 고르게** 한다.
//!
//! 이 컨트롤은 **무엇을 할지 모른다** — 항목 id를 돌려줄 뿐이고, 실행은 호스트 몫이다
//! (클립보드 접근은 `nbeep-plat`에 있고 UI 크레이트는 그걸 모른다 — 이음새 유지).
//!
//! 화면 밖으로 나가지 않도록 **경계 안으로 접어 넣는다**(오른쪽·아래에서 열면 위/왼쪽으로).

use crate::draw::DrawCtx;
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::FontSlot;

// 레이아웃 상수(논리 px).
const PAD_H: i32 = 12;
const PAD_V: i32 = 5;
const ROW_EXTRA: i32 = 10;
const SEP_H: i32 = 7;
const MIN_W: i32 = 120;
const RADIUS: i32 = 6;

/// 메뉴 한 줄.
#[derive(Clone, Debug)]
pub enum CtxItem {
    /// 고를 수 있는 항목 — `(id, 라벨, 활성)`. 비활성은 흐리게 표시되고 골라지지 않는다.
    Item {
        /// 호스트가 받을 식별자.
        id: String,
        /// 표시 문구(i18n 적용 후).
        label: String,
        /// `false`면 흐리게 · 선택 불가(선택할 게 없는데 "복사"가 멀쩡히 보이면 거짓말이다).
        enabled: bool,
    },
    /// 구분선.
    Separator,
}

impl CtxItem {
    /// 활성 항목.
    #[must_use]
    pub fn item(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::Item {
            id: id.into(),
            label: label.into(),
            enabled: true,
        }
    }
    /// 활성 여부를 지정한 항목.
    #[must_use]
    pub fn maybe(id: impl Into<String>, label: impl Into<String>, enabled: bool) -> Self {
        Self::Item {
            id: id.into(),
            label: label.into(),
            enabled,
        }
    }
}

/// 커서 자리에 뜨는 팝업 메뉴.
#[derive(Clone, Debug, Default)]
pub struct ContextMenu {
    items: Vec<CtxItem>,
    /// 열려 있으면 좌상단 좌표(경계 보정 완료).
    at: Option<Point>,
    hover: Option<usize>,
    picked: Option<String>,
    scale: f32,
    /// 마지막으로 계산한 팝업 rect(히트 판정용) — paint가 아니라 열 때 정한다.
    rect: Rect,
    /// 항목 라벨의 최대 폭(px) — 열 때 측정해 둔다(paint에서만 잴 수 있어서).
    measured_w: i32,
}

impl ContextMenu {
    /// 새 메뉴(닫힌 상태).
    #[must_use]
    pub fn new() -> Self {
        Self {
            scale: 1.0,
            ..Self::default()
        }
    }

    /// 배율(고DPI).
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 열려 있는가.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.at.is_some()
    }

    /// 현재 팝업 영역(닫혀 있으면 빈 rect) — 호스트의 무효화 범위 계산용.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        if self.is_open() {
            self.rect
        } else {
            Rect::new(0, 0, 0, 0)
        }
    }

    /// 닫는다.
    pub fn close(&mut self) {
        self.at = None;
        self.hover = None;
    }

    /// **`(x, y)`에 연다** — `host`는 팝업이 넘어가면 안 되는 영역(보통 창 전체).
    /// `text_w`는 라벨 최대 폭(호스트가 [`DrawCtx::text_width`]로 잰 값): 이 컨트롤은
    /// paint 밖에서 글자를 잴 수 없어, 폭 측정만 호출 측에서 받는다.
    pub fn open_at(&mut self, x: i32, y: i32, items: Vec<CtxItem>, host: Rect, text_w: i32) {
        if items.is_empty() {
            self.close();
            return;
        }
        self.items = items;
        self.measured_w = text_w;
        self.hover = None;
        self.picked = None;
        let (w, h) = self.size_px();
        // 경계 접기 — 오른쪽/아래로 넘치면 커서 반대쪽으로 편다.
        let px = if x + w > host.right() {
            (x - w).max(host.x)
        } else {
            x
        };
        let py = if y + h > host.bottom() {
            (y - h).max(host.y)
        } else {
            y
        };
        self.at = Some(Point { x: px, y: py });
        self.rect = Rect::new(px, py, w, h);
    }

    fn row_h(&self) -> i32 {
        // 글꼴 높이를 모르는 자리라 논리 px 기준으로 잡는다(paint에서 실제 글자를 세로 중앙에 둔다).
        self.s(16 + ROW_EXTRA)
    }

    fn size_px(&self) -> (i32, i32) {
        let mut h = self.s(PAD_V) * 2;
        for it in &self.items {
            h += match it {
                CtxItem::Item { .. } => self.row_h(),
                CtxItem::Separator => self.s(SEP_H),
            };
        }
        let w = (self.measured_w + self.s(PAD_H) * 2).max(self.s(MIN_W));
        (w, h)
    }

    /// 인덱스 → 그 행의 rect.
    fn row_rect(&self, idx: usize) -> Option<Rect> {
        let at = self.at?;
        let mut y = at.y + self.s(PAD_V);
        for (i, it) in self.items.iter().enumerate() {
            let h = match it {
                CtxItem::Item { .. } => self.row_h(),
                CtxItem::Separator => self.s(SEP_H),
            };
            if i == idx {
                return Some(Rect::new(at.x, y, self.rect.w, h));
            }
            y += h;
        }
        None
    }

    fn hit(&self, p: Point) -> Option<usize> {
        (0..self.items.len()).find(|&i| {
            matches!(self.items[i], CtxItem::Item { enabled: true, .. })
                && self.row_rect(i).is_some_and(|r| r.contains(p))
        })
    }

    /// 키보드 탐색(08-13 실기: "메뉴를 키보드로 이동 못 한다") — 활성 항목 사이를
    /// 순환한다(비활성·구분선 건너뜀 · 처음 ↓ = 첫 항목, 처음 ↑ = 마지막).
    fn move_hover(&mut self, down: bool) {
        let sel: Vec<usize> = (0..self.items.len())
            .filter(|&i| matches!(self.items[i], CtxItem::Item { enabled: true, .. }))
            .collect();
        if sel.is_empty() {
            return;
        }
        let cur = self.hover.and_then(|h| sel.iter().position(|&i| i == h));
        let next = match cur {
            None => {
                if down {
                    0
                } else {
                    sel.len() - 1
                }
            }
            Some(p) => {
                if down {
                    (p + 1) % sel.len()
                } else {
                    (p + sel.len() - 1) % sel.len()
                }
            }
        };
        self.hover = Some(sel[next]);
    }

    /// 이벤트 처리 — `true`면 **소비**(호스트는 그 이벤트를 아래 콘텐츠에 쓰지 않는다).
    ///
    /// 열려 있는 동안은 바깥 클릭·Esc로 닫히며, 그 클릭도 소비한다
    /// (메뉴를 닫으려던 클릭이 뒤 컨텐츠의 선택을 바꾸면 놀란다).
    pub fn on_event(&mut self, ev: &InputEvent) -> bool {
        if !self.is_open() {
            return false;
        }
        match *ev {
            InputEvent::MouseMove { x, y } => {
                let h = self.hit(Point { x, y });
                let changed = h != self.hover;
                self.hover = h;
                changed
            }
            InputEvent::MouseDown { x, y, .. } => {
                let p = Point { x, y };
                if let Some(i) = self.hit(p) {
                    if let CtxItem::Item { id, .. } = &self.items[i] {
                        self.picked = Some(id.clone());
                    }
                    self.close();
                } else {
                    // 팝업 안의 비활성 행/여백이면 그냥 무시, 바깥이면 닫는다.
                    if !self.rect.contains(p) {
                        self.close();
                    }
                }
                true
            }
            // 팝업이 열린 동안의 우클릭·휠·키는 모두 팝업이 먹고 닫는다.
            InputEvent::RightDown { x, y } => {
                if !self.rect.contains(Point { x, y }) {
                    self.close();
                }
                true
            }
            InputEvent::MouseUp { .. } | InputEvent::Wheel { .. } | InputEvent::HWheel { .. } => {
                true
            }
            // 키보드 — ↑/↓ 이동 · Enter 선택 · 그 외(Esc 포함)는 메뉴만 닫는다.
            InputEvent::Key { key, .. } => {
                match key {
                    Key::Down => self.move_hover(true),
                    Key::Up => self.move_hover(false),
                    Key::Enter => {
                        if let Some(CtxItem::Item {
                            id, enabled: true, ..
                        }) = self.hover.map(|i| &self.items[i])
                        {
                            self.picked = Some(id.clone());
                        }
                        self.close();
                    }
                    _ => self.close(),
                }
                true
            }
            InputEvent::Char { .. } => {
                self.close();
                true
            }
            _ => false,
        }
    }

    /// 현재 항목 목록(테스트·검증용) — 활성 여부까지 그대로 본다.
    #[must_use]
    pub fn items_for_test(&self) -> &[CtxItem] {
        &self.items
    }

    /// 인덱스 행의 rect(테스트·호스트 히트 검증용).
    #[must_use]
    pub fn row_rect_of(&self, idx: usize) -> Option<Rect> {
        self.row_rect(idx)
    }

    /// 고른 항목 id를 **가져간다**(한 번만).
    pub fn take_picked(&mut self) -> Option<String> {
        self.picked.take()
    }

    /// 팝업 렌더 — 다른 것들을 다 그린 **뒤에** 불러야 위에 뜬다.
    pub fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let Some(at) = self.at else { return };
        let r = self.rect;
        // 바탕 + 테두리(그림자 대신 테두리로 층을 만든다 — 렌더러에 블러가 없다).
        ctx.fill_round_rect(r, self.s(RADIUS), theme.panel_bg);
        ctx.stroke_round_rect(r, self.s(RADIUS), theme.border, 1.0);
        ctx.select_font(FontSlot::Base, false);
        let th = ctx.text_height();
        let mut y = at.y + self.s(PAD_V);
        for it in &self.items {
            match it {
                CtxItem::Item {
                    id: _,
                    label,
                    enabled,
                } => {
                    let h = self.row_h();
                    let row = Rect::new(r.x, y, r.w, h);
                    let hot = *enabled
                        && self
                            .hover
                            .and_then(|i| self.row_rect(i))
                            .is_some_and(|hr| hr.y == y);
                    if hot {
                        ctx.fill_rect(
                            Rect::new(r.x + self.s(2), y, r.w - self.s(4), h),
                            theme.accent,
                        );
                    }
                    let fg = if !*enabled {
                        theme.text_dim
                    } else if hot {
                        theme.window_bg
                    } else {
                        theme.text
                    };
                    // 세로 정확히 가운데 — 글자 높이를 재서 놓는다(눈대중 상수 금지 · 08-09).
                    ctx.text(r.x + self.s(PAD_H), y + (h - th) / 2, row, label, fg);
                    y += h;
                }
                CtxItem::Separator => {
                    let h = self.s(SEP_H);
                    ctx.fill_rect(
                        Rect::new(r.x + self.s(6), y + h / 2, r.w - self.s(12), 1),
                        theme.border,
                    );
                    y += h;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Rect {
        Rect::new(0, 0, 400, 300)
    }
    fn items() -> Vec<CtxItem> {
        vec![
            CtxItem::item("copy", "복사"),
            CtxItem::maybe("cut", "잘라내기", false),
            CtxItem::Separator,
            CtxItem::item("paste", "붙여넣기"),
        ]
    }
    fn down(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: true,
        }
    }

    #[test]
    fn opens_at_cursor_and_picks_an_item() {
        let mut m = ContextMenu::new();
        assert!(!m.is_open());
        m.open_at(10, 10, items(), host(), 60);
        assert!(m.is_open());
        let first = m.row_rect(0).unwrap();
        assert!(m.on_event(&down(first.x + 5, first.y + 2)), "클릭 소비");
        assert_eq!(m.take_picked().as_deref(), Some("copy"));
        assert!(!m.is_open(), "고르면 닫힌다");
        assert_eq!(m.take_picked(), None, "결과는 한 번만 가져간다");
    }

    #[test]
    fn disabled_item_cannot_be_picked() {
        let mut m = ContextMenu::new();
        m.open_at(10, 10, items(), host(), 60);
        let cut = m.row_rect(1).unwrap();
        m.on_event(&down(cut.x + 5, cut.y + 2));
        assert_eq!(m.take_picked(), None, "비활성은 골라지지 않는다");
        assert!(m.is_open(), "비활성 클릭으로 닫히지도 않는다");
    }

    #[test]
    fn separator_is_not_selectable() {
        let mut m = ContextMenu::new();
        m.open_at(10, 10, items(), host(), 60);
        let sep = m.row_rect(2).unwrap();
        m.on_event(&down(sep.x + 5, sep.y));
        assert_eq!(m.take_picked(), None);
    }

    #[test]
    fn outside_click_closes_and_is_consumed() {
        // 닫으려는 클릭이 뒤 콘텐츠까지 가면 선택이 엉뚱하게 바뀐다.
        let mut m = ContextMenu::new();
        m.open_at(10, 10, items(), host(), 60);
        assert!(m.on_event(&down(390, 290)), "바깥 클릭도 소비");
        assert!(!m.is_open());
        assert_eq!(m.take_picked(), None);
    }

    #[test]
    fn folds_inside_host_bounds_near_edges() {
        let mut m = ContextMenu::new();
        // 우하단 모서리에서 열면 위/왼쪽으로 펴야 화면 밖으로 안 나간다.
        m.open_at(398, 298, items(), host(), 60);
        let r = m.bounds();
        assert!(r.right() <= host().right(), "오른쪽 경계 안: {r:?}");
        assert!(r.bottom() <= host().bottom(), "아래 경계 안: {r:?}");
        assert!(r.x >= 0 && r.y >= 0);
    }

    fn key(k: Key) -> InputEvent {
        InputEvent::Key {
            key: k,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn keyboard_navigates_skipping_disabled_and_picks_with_enter() {
        // 08-13 실기 — 키보드로 메뉴를 이동·선택할 수 없었다(모든 키가 닫기였다).
        let mut m = ContextMenu::new();
        m.open_at(10, 10, items(), host(), 60);
        assert!(m.on_event(&key(Key::Down)), "키 소비");
        assert!(m.is_open(), "↓는 메뉴를 닫지 않는다");
        m.on_event(&key(Key::Down)); // copy → paste(비활성 cut·구분선 건너뜀)
        m.on_event(&key(Key::Enter));
        assert_eq!(m.take_picked().as_deref(), Some("paste"));
        assert!(!m.is_open(), "Enter 선택 = 닫힘");
    }

    #[test]
    fn up_from_nothing_starts_at_last_enabled() {
        let mut m = ContextMenu::new();
        m.open_at(10, 10, items(), host(), 60);
        m.on_event(&key(Key::Up));
        m.on_event(&key(Key::Enter));
        assert_eq!(m.take_picked().as_deref(), Some("paste"), "↑ 시작 = 마지막 활성");
    }

    #[test]
    fn escape_closes_without_pick_and_enter_without_hover_closes() {
        let mut m = ContextMenu::new();
        m.open_at(10, 10, items(), host(), 60);
        assert!(m.on_event(&key(Key::Escape)), "Esc 소비(창 닫기로 새면 안 된다)");
        assert!(!m.is_open());
        assert_eq!(m.take_picked(), None);
        m.open_at(10, 10, items(), host(), 60);
        m.on_event(&key(Key::Enter)); // 아무것도 고르지 않은 Enter = 그냥 닫기
        assert!(!m.is_open());
        assert_eq!(m.take_picked(), None);
    }

    #[test]
    fn closed_menu_ignores_events() {
        let mut m = ContextMenu::new();
        assert!(!m.on_event(&down(5, 5)), "닫혀 있으면 소비하지 않는다");
    }

    #[test]
    fn empty_items_do_not_open() {
        // 줄 게 없으면 빈 상자를 띄우지 않는다.
        let mut m = ContextMenu::new();
        m.open_at(10, 10, vec![], host(), 60);
        assert!(!m.is_open());
    }
}
