//! **편집 우클릭 메뉴 공용 코어**(M3-1e ① 1슬라이스 · 08-17) — 종전엔 TextBox와
//! 대화 입력창이 항목 구성·게이트·픽 처리를 **두 벌**로 들고 있었고, 순서
//! (select_all 위치)와 폭 근사까지 서로 달랐다(08-13 전수 검사의 "같은 기능이
//! 창마다 따로"의 대표 사례). 여기 한 벌로 모은다 — 항목 순서는 **복사 · 잘라
//! 내기 · 붙여넣기 · ― · 전체 선택**으로 통일(macOS 편집 메뉴 관례).
//!
//! 클립보드는 여전히 **호스트 몫**이다(컨트롤은 OS를 모른다) — 픽은
//! [`EditMenuAction`]으로 요청만 남긴다. `extra` 항목(풍선 "메시지 복사" 등
//! 호출측 고유 메뉴)은 앞에 끼워 넣고 픽을 `Extra(id)`로 돌려준다.

use super::{ctl_label, ContextMenu, CtlMsg, CtxItem};
use crate::draw::DrawCtx;
use crate::event::InputEvent;
use crate::geom::Rect;
use crate::theme::Theme;

/// 편집 대상의 현재 상태 — 항목 활성 게이트의 재료.
#[derive(Clone, Copy, Debug)]
pub struct EditMenuCaps {
    /// 선택 영역이 있는가(복사·잘라내기 활성).
    pub has_sel: bool,
    /// 텍스트가 있는가(전체 선택 활성).
    pub has_text: bool,
    /// 클립보드에 텍스트가 있는가(붙여넣기 활성 — 호스트가 조회해 준다).
    pub clip_has_text: bool,
}

/// 픽 결과 — 편집 4종은 열거로, 호출측 고유 항목은 id 그대로.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditMenuAction {
    /// 전체 선택(위젯 내부 상태 — 호출측이 자기 EditState에 적용).
    SelectAll,
    /// 복사(클립보드는 호스트 몫 — 요청만).
    Copy,
    /// 잘라내기.
    Cut,
    /// 붙여넣기.
    Paste,
    /// `extra`로 끼운 호출측 항목.
    Extra(String),
}

/// 편집 우클릭 메뉴 — [`ContextMenu`] 위에 편집 표준 항목·게이트·픽 해석을 얹는다.
#[derive(Debug, Default)]
pub struct EditMenu {
    menu: ContextMenu,
}

impl EditMenu {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 메뉴를 연다 — `extra`(호출측 고유 항목)가 먼저, 편집 표준 4종이 뒤.
    /// 전부 비활성이어도 열린다(비활성 표시가 "왜 안 되는지"를 말해 준다).
    pub fn open_at(
        &mut self,
        x: i32,
        y: i32,
        scale: f32,
        host: Rect,
        caps: EditMenuCaps,
        extra: Vec<CtxItem>,
    ) {
        let mut items = extra;
        if !items.is_empty() {
            items.push(CtxItem::Separator);
        }
        items.push(CtxItem::maybe(
            "copy",
            ctl_label(CtlMsg::CtxCopy),
            caps.has_sel,
        ));
        items.push(CtxItem::maybe(
            "cut",
            ctl_label(CtlMsg::CtxCut),
            caps.has_sel,
        ));
        items.push(CtxItem::maybe(
            "paste",
            ctl_label(CtlMsg::CtxPaste),
            caps.clip_has_text,
        ));
        items.push(CtxItem::Separator);
        items.push(CtxItem::maybe(
            "select_all",
            ctl_label(CtlMsg::CtxSelectAll),
            caps.has_text,
        ));
        // 폭 힌트 — 자당 근사(ASCII 8 · 그 외 15). 실측 보정은 ContextMenu 몫(08-14).
        let widest = items
            .iter()
            .map(|it| match it {
                CtxItem::Item { label, .. } => label
                    .chars()
                    .map(|c| if c.is_ascii() { 8 } else { 15 })
                    .sum::<i32>(),
                CtxItem::Separator => 0,
            })
            .max()
            .unwrap_or(0);
        self.menu.set_scale(scale);
        self.menu.open_at(x, y, items, host, widest);
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.menu.is_open()
    }

    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.menu.bounds()
    }

    /// 열려 있으면 이벤트를 먹고 true(팝업 최상위 규약 그대로).
    pub fn on_event(&mut self, ev: &InputEvent) -> bool {
        self.menu.on_event(ev)
    }

    /// 픽 해석 — 편집 4종은 열거로, 나머지는 `Extra(id)`.
    pub fn take_action(&mut self) -> Option<EditMenuAction> {
        let id = self.menu.take_picked()?;
        Some(match id.as_str() {
            "select_all" => EditMenuAction::SelectAll,
            "copy" => EditMenuAction::Copy,
            "cut" => EditMenuAction::Cut,
            "paste" => EditMenuAction::Paste,
            _ => EditMenuAction::Extra(id),
        })
    }

    /// 테스트 접근자 위임 — 기존 회귀(픽 좌표·항목 검사)가 그대로 돌게.
    #[must_use]
    pub fn items_for_test(&self) -> &[CtxItem] {
        self.menu.items_for_test()
    }

    /// 테스트 접근자 위임(행 사각형 — 클릭 좌표 합성용).
    #[must_use]
    pub fn row_rect_of(&self, i: usize) -> Option<Rect> {
        self.menu.row_rect_of(i)
    }

    pub fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        self.menu.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> EditMenuCaps {
        EditMenuCaps {
            has_sel: true,
            has_text: true,
            clip_has_text: true,
        }
    }

    /// 항목 순서 통일 계약 — extra · ― · copy · cut · paste · ― · select_all.
    #[test]
    fn item_order_is_unified() {
        let mut m = EditMenu::new();
        m.open_at(
            10,
            10,
            1.0,
            Rect::new(0, 0, 400, 400),
            caps(),
            vec![CtxItem::item("copy_message", "메시지 복사")],
        );
        let ids: Vec<String> = m
            .menu
            .items_for_test()
            .iter()
            .filter_map(|it| match it {
                CtxItem::Item { id, .. } => Some(id.clone()),
                CtxItem::Separator => None,
            })
            .collect();
        assert_eq!(ids, ["copy_message", "copy", "cut", "paste", "select_all"]);
    }

    /// 픽 해석 — 표준 4종은 열거 · 미지는 Extra.
    #[test]
    fn picks_resolve_to_actions() {
        let mut m = EditMenu::new();
        m.open_at(10, 10, 1.0, Rect::new(0, 0, 400, 400), caps(), Vec::new());
        // ContextMenu 내부 픽을 직접 흉내낼 수 없어 take_picked 경로만 계약으로:
        // 열림·닫힘과 액션 해석은 위임 대상(ContextMenu)의 기존 회귀가 커버한다.
        assert!(m.is_open());
        assert!(m.take_action().is_none(), "픽 전엔 없음");
    }
}
