//! 텍스트 박스 — **placeholder** · char 단위 편집(캐럿·선택 [`EditState`]) · 포커스 링 · 도움말.
//!
//! 공통 기능은 [`Control`] 기본 메서드로 상속([`super`]).

use super::{image_fit_contain, Control, ControlBase};
use crate::draw::{DrawCtx, FontSlot};
use crate::edit::{EditKey, EditState};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

/// 텍스트 박스 컨트롤.
#[derive(Debug)]
pub struct TextBox {
    base: ControlBase,
    edit: EditState,
    placeholder: String,
    /// 선행 이미지 아이콘(옵션 · 투명 배경 RGBA). 있으면 placeholder·캐럿이 그 뒤로 밀린다.
    image: Option<Rc<IconImage>>,
    /// Enter 확정 1회성 보고.
    committed: bool,
    /// 내용 변경 1회성 보고.
    changed: bool,
    /// 값이 있으면 우측에 ×(지우기) 버튼 표시(클릭 = 초기화 · 사용자 요청 08-09).
    clearable: bool,
    /// 텍스트 시작 x(페인트가 기록 — 클릭 좌표를 글자 위치로 바꾸는 근거).
    text_x: std::cell::Cell<i32>,
    /// 각 문자 경계의 누적 폭(페인트가 실측해 기록 · 폰트를 모르는 이벤트 경로가 쓴다).
    caret_xs: std::cell::RefCell<Vec<i32>>,
    /// 드래그 선택 중.
    dragging: bool,
    /// 마지막 클릭 (캐럿 인덱스, 연속 횟수) — 더블·트리플 판정.
    /// `MouseDown`에는 시각이 없어 **같은 위치 + 무개입**(사이에 키 입력 없음)으로
    /// 연속을 판정한다(시각 주입은 M3-1e). 위치가 다르면 새 체인 = 캐럿 이동만.
    last_click: (usize, u8),
    /// 가로 스크롤(px · ① 08-13) — 텍스트가 폭을 넘으면 **캐럿이 항상 보이게**
    /// 페인트가 조정한다(셀: 페인트는 &self).
    hscroll: std::cell::Cell<i32>,
    /// IME 조합 중 문자열(08-13) — 캐럿 자리에 밑줄로 끼워 그린다. 확정 전에도
    /// 지금 치는 글자가 보여야 한다(대화 입력과 동일한 경험 — 호스트가 배선).
    preedit: String,
    /// 우클릭 편집 메뉴(08-13 전수 검사 — 대화 입력에만 있고 일반 필드엔 없었다).
    ctx_menu: super::ContextMenu,
    /// 메뉴에서 고른 클립보드 행동(1회성) — OS 클립보드는 호스트 몫이라 요청만 남긴다.
    edit_ctx: Option<EditCtxAction>,
    /// 붙여넣기 항목 활성 근거(호스트가 우클릭 시점에 1회 주입 — 대화 입력과 동일).
    clip_has_text: bool,
}

/// 우클릭 편집 메뉴에서 고른 행동 — 실행(클립보드 접근)은 호스트가 한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCtxAction {
    /// 복사(⌘/Ctrl+C와 같은 경로).
    Copy,
    /// 잘라내기.
    Cut,
    /// 붙여넣기.
    Paste,
}

impl TextBox {
    /// placeholder로 만든다(빈 값 시작).
    #[must_use]
    pub fn new(placeholder: impl Into<String>) -> Self {
        Self {
            base: ControlBase::default(),
            edit: EditState::new(),
            placeholder: placeholder.into(),
            image: None,
            committed: false,
            changed: false,
            clearable: false,
            text_x: std::cell::Cell::new(0),
            caret_xs: std::cell::RefCell::new(Vec::new()),
            dragging: false,
            last_click: (0, 0),
            hscroll: std::cell::Cell::new(0),
            preedit: String::new(),
            ctx_menu: super::ContextMenu::new(),
            edit_ctx: None,
            clip_has_text: true,
        }
    }

    /// 메뉴에서 고른 클립보드 행동(1회성) — 호스트가 ⌘C/X/V와 같은 경로로 잇는다.
    pub fn take_edit_ctx(&mut self) -> Option<EditCtxAction> {
        self.edit_ctx.take()
    }

    /// 클립보드에 텍스트가 있는가(호스트가 우클릭 시점에 1회 주입 — 붙여넣기 활성 근거).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.clip_has_text = yes;
    }

    /// IME 조합 중 문자열 갱신(빈 문자열 = 소거). 포커스 없는 박스는 무시한다 —
    /// 호스트가 창 단위로 보내므로 초점 필드만 받아야 이중 표시가 없다.
    pub fn set_preedit(&mut self, text: &str, inv: &mut Invalidations) {
        if !self.base.focused && !text.is_empty() {
            return;
        }
        if self.preedit != text {
            self.preedit = text.to_string();
            inv.push(self.base.bounds);
        }
    }

    /// ×(지우기) 버튼 사용(체이닝) — 값이 있을 때만 표시, 클릭 = 즉시 초기화.
    #[must_use]
    pub fn with_clearable(mut self) -> Self {
        self.clearable = true;
        self
    }

    /// 클릭 x → 캐럿 인덱스(페인트가 남긴 실측 폭을 쓴다 — 가장 가까운 경계).
    fn caret_at_x(&self, x: i32) -> usize {
        let xs = self.caret_xs.borrow();
        if xs.is_empty() {
            return 0;
        }
        let mut best = 0usize;
        let mut best_d = i32::MAX;
        for (i, cx) in xs.iter().enumerate() {
            let d = (x - cx).abs();
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        best
    }

    /// 단어 경계로 선택(더블클릭).
    fn select_word_at(&mut self, idx: usize) {
        let chars: Vec<char> = self.edit.text().chars().collect();
        if chars.is_empty() {
            return;
        }
        let i = idx.min(chars.len().saturating_sub(1));
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut a = i;
        while a > 0 && is_word(chars[a - 1]) {
            a -= 1;
        }
        let mut b = i;
        while b < chars.len() && is_word(chars[b]) {
            b += 1;
        }
        self.edit.set_selection(a, b);
    }

    /// ×(지우기) 버튼 영역(값이 있을 때만 유효) — 호스트 테스트용 공개.
    #[must_use]
    pub fn clear_rect(&self) -> Rect {
        let b = self.base.bounds;
        let d = self.s(16);
        Rect::new(b.right() - d - self.s(6), b.y + (b.h - d) / 2, d, d)
    }

    /// 선행 이미지 아이콘 지정(체이닝) — placeholder·캐럿이 아이콘 뒤로 배치된다.
    #[must_use]
    pub fn with_image(mut self, image: Rc<IconImage>) -> Self {
        self.image = Some(image);
        self
    }

    /// 초기 텍스트 지정.
    #[must_use]
    pub fn with_text(mut self, text: &str) -> Self {
        self.edit = EditState::with_text(text, false);
        self
    }

    /// 현재 텍스트.
    #[must_use]
    pub fn text(&self) -> String {
        self.edit.text()
    }

    /// 텍스트 지정(보고 없음).
    pub fn set_text(&mut self, text: &str) {
        self.edit.set_text(text);
    }

    /// 내용이 바뀌었으면 새 텍스트를 꺼낸다(1회성).
    pub fn take_changed(&mut self) -> Option<String> {
        std::mem::take(&mut self.changed).then(|| self.edit.text())
    }

    /// 선택 텍스트(복사 — ① 08-13). 위젯은 OS 클립보드를 모른다 — 호스트가 잇는다.
    #[must_use]
    pub fn copy_selection(&self) -> Option<String> {
        self.base.focused.then(|| self.edit.selected_text())?
    }

    /// 선택 텍스트를 잘라낸다(① — 반환 텍스트를 호스트가 클립보드에 쓴다).
    pub fn cut_selection(&mut self, inv: &mut Invalidations) -> Option<String> {
        if !self.base.focused {
            return None;
        }
        let t = self.edit.cut()?;
        self.changed = true;
        inv.push(self.base.bounds);
        Some(t)
    }

    /// 붙여넣기(① — 호스트가 읽은 클립보드 텍스트). 단일 행 컨트롤이라
    /// 개행·제어문자는 공백 하나로 접는다(주소·이름·검색 어디서든 안전).
    pub fn paste(&mut self, text: &str, inv: &mut Invalidations) {
        if !self.base.focused || text.is_empty() {
            return;
        }
        let mut cleaned = String::with_capacity(text.len());
        let mut ws = false;
        for c in text.chars() {
            if c.is_control() {
                ws = true;
                continue;
            }
            if ws {
                cleaned.push(' ');
                ws = false;
            }
            cleaned.push(c);
        }
        if cleaned.is_empty() {
            return;
        }
        self.edit.insert_str(&cleaned);
        self.changed = true;
        inv.push(self.base.bounds);
    }

    /// Enter 확정되었으면 텍스트를 꺼낸다(1회성).
    pub fn take_committed(&mut self) -> Option<String> {
        std::mem::take(&mut self.committed).then(|| self.edit.text())
    }

    /// 우클릭 편집 메뉴가 열려 있는가 — 컨테이너의 Esc 가드용(08-13 실기:
    /// 메뉴가 열려 있는데 Esc가 창 닫기로 새면 메뉴를 키보드로 못 닫는다).
    #[must_use]
    pub fn popup_open(&self) -> bool {
        self.ctx_menu.is_open()
    }

    /// 조합 중(preedit) 문자열까지 캐럿 자리에 끼운 **표시용** 텍스트(편집 상태 불변).
    /// 아바타 이니셜 미리보기 등 "지금 화면에 보이는 그대로"가 필요한 곳이 쓴다
    /// (08-13 실기: 필드엔 "나다"가 보이는데 아바타는 "나"라 미입력처럼 보였다).
    #[must_use]
    pub fn display_text(&self) -> String {
        if self.preedit.is_empty() {
            return self.edit.text();
        }
        let chars: Vec<char> = self.edit.text().chars().collect();
        let caret = self.edit.caret().min(chars.len());
        let before: String = chars[..caret].iter().collect();
        let after: String = chars[caret..].iter().collect();
        format!("{before}{}{after}", self.preedit)
    }

    /// 우클릭 메뉴를 **최상위 레이어로** 다시 그린다(08-13 실기: 프로필에서 아래
    /// 필드가 메뉴를 덮었다 — z순서는 그리는 순서가 전부다). `paint`도 그리지만
    /// (단독 사용 안전망), 컨테이너는 **모든 자식을 그린 뒤** 이걸 한 번 더 불러
    /// 팝업을 맨 위로 올린다.
    pub fn paint_popup(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        if self.ctx_menu.is_open() {
            self.ctx_menu.paint(ctx, theme);
        }
    }
}

impl Control for TextBox {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for TextBox {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // 우클릭 편집 메뉴가 열려 있으면 가장 먼저 먹는다(팝업 최상위).
        if self.ctx_menu.is_open() {
            let menu_rect = self.ctx_menu.bounds();
            if self.ctx_menu.on_event(ev) {
                inv.push(menu_rect);
                inv.push(self.base.bounds);
                if let Some(id) = self.ctx_menu.take_picked() {
                    match id.as_str() {
                        // 전체 선택은 위젯 내부 상태 — 즉시 실행.
                        "select_all" => self.edit.key(EditKey::SelectAll, false),
                        // 클립보드는 호스트 몫 — 요청만 남긴다(⌘C/X/V와 같은 경로).
                        "copy" => self.edit_ctx = Some(EditCtxAction::Copy),
                        "cut" => self.edit_ctx = Some(EditCtxAction::Cut),
                        "paste" => self.edit_ctx = Some(EditCtxAction::Paste),
                        _ => {}
                    }
                }
                return;
            }
        }
        if let InputEvent::RightDown { x, y } = *ev {
            if self.base.bounds.contains(Point { x, y }) {
                // 라벨은 주입 공급자에서(08-14 라이브러리화 — 컨트롤은 앱 i18n을 모른다).
                use super::{ctl_label, CtlMsg};
                self.base.focused = true; // 우클릭도 포커스(메뉴 행동의 대상이 된다)
                let has_sel = self.edit.selected_text().is_some();
                let has_text = !self.edit.text().is_empty();
                let items = vec![
                    super::CtxItem::maybe("select_all", ctl_label(CtlMsg::CtxSelectAll), has_text),
                    super::CtxItem::maybe("copy", ctl_label(CtlMsg::CtxCopy), has_sel),
                    super::CtxItem::maybe("cut", ctl_label(CtlMsg::CtxCut), has_sel),
                    super::CtxItem::maybe("paste", ctl_label(CtlMsg::CtxPaste), self.clip_has_text),
                ];
                self.ctx_menu.set_scale(self.base.scale);
                // 팝업이 박스 밖(아래)으로 펼쳐질 공간 — 박스 사각형만 주면 안에 구겨진다.
                let host = Rect::new(
                    self.base.bounds.x,
                    self.base.bounds.y,
                    self.base.bounds.w.max(self.s(200)),
                    self.base.bounds.h + self.s(140),
                );
                self.ctx_menu.open_at(x, y, items, host, 8 * 15);
                inv.push(self.base.bounds);
                inv.push(self.ctx_menu.bounds());
            }
            return;
        }
        match *ev {
            InputEvent::MouseDown { x, y, shift, .. } => {
                let badge = self.help_badge_rect(self.base.bounds);
                if self.handle_help_click(x, y, badge) {
                    inv.push(self.base.bounds);
                    return;
                }
                // ×(지우기) — 값이 있을 때만. 클릭 = 초기화 + 변경 보고.
                if self.clearable
                    && !self.edit.text().is_empty()
                    && self.clear_rect().contains(Point { x, y })
                {
                    self.edit.set_text("");
                    self.changed = true;
                    inv.push(self.base.bounds);
                    return;
                }
                if self.base.bounds.contains(Point { x, y }) {
                    self.base.focused = true;
                    // 클릭 지점으로 캐럿 이동 + 드래그 선택 시작(기본 텍스트 동작).
                    let idx = self.caret_at_x(x);
                    // 연속 클릭은 **같은 캐럿 위치일 때만** 잇는다(08-13 실기 — 위치 무관
                    // 누적이라 두 번째 단일 클릭이 단어 선택이 돼 캐럿 재배치가 불가능했다).
                    // Shift+클릭은 언제나 선택 확장 — 더블클릭 체인에 넣지 않는다.
                    self.last_click.1 = if shift {
                        0
                    } else if self.last_click.0 == idx && self.last_click.1 > 0 {
                        if self.last_click.1 >= 3 {
                            1
                        } else {
                            self.last_click.1 + 1
                        }
                    } else {
                        1
                    };
                    self.last_click.0 = idx;
                    match self.last_click.1 {
                        2 => self.select_word_at(idx),                 // 더블 = 단어
                        3 => self.edit.key(EditKey::SelectAll, false), // 트리플 = 전체
                        _ => {
                            self.edit.set_caret(idx, shift);
                            self.dragging = true;
                        }
                    }
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::MouseMove { x, .. } if self.dragging => {
                // 영역 밖으로 끌면 **한 글자씩 자동 진행**(① 08-13) — 페인트가 캐럿을
                // 따라 스크롤하므로, 마우스를 밖에 둔 채 움직이면 계속 밀린다.
                let idx = if x > self.base.bounds.right() {
                    self.edit.caret().saturating_add(1)
                } else if x < self.base.bounds.x {
                    self.edit.caret().saturating_sub(1)
                } else {
                    self.caret_at_x(x)
                };
                self.edit.set_caret(idx, true); // 앵커 유지 = 범위 확장
                inv.push(self.base.bounds);
            }
            InputEvent::MouseUp { .. } => {
                self.dragging = false;
            }
            InputEvent::Char { c, .. } if self.base.focused => {
                self.last_click.1 = 0; // 타이핑 = 클릭 체인 끊김(클릭-타이핑-클릭 ≠ 더블클릭)
                if c == '\u{8}' {
                    self.edit.backspace();
                } else if !c.is_control() {
                    self.edit.insert(c);
                }
                self.changed = true;
                inv.push(self.base.bounds);
            }
            InputEvent::Key {
                key,
                shift,
                primary,
            } if self.base.focused => {
                self.last_click.1 = 0; // 키 개입 = 클릭 체인 끊김
                match key {
                    Key::Enter => {
                        self.committed = true;
                        inv.push(self.base.bounds);
                    }
                    // ⌘/Ctrl+←/→ = 줄 처음/끝(mac 관례 · DR-16 — 08-13 전수 검사).
                    // 이동·선택 키는 **다시 그리기를 요청해야** 캐럿·선택 반전이 보인다
                    // (08-13 실기 — 프로필 필드에서 ←/→·Shift+←·⌘A가 무반응으로 보였다).
                    Key::Left if primary => {
                        self.edit.key(EditKey::Home, shift);
                        inv.push(self.base.bounds);
                    }
                    Key::Right if primary => {
                        self.edit.key(EditKey::End, shift);
                        inv.push(self.base.bounds);
                    }
                    Key::Left => {
                        self.edit.key(EditKey::Left, shift);
                        inv.push(self.base.bounds);
                    }
                    Key::Right => {
                        self.edit.key(EditKey::Right, shift);
                        inv.push(self.base.bounds);
                    }
                    Key::Home => {
                        self.edit.key(EditKey::Home, shift);
                        inv.push(self.base.bounds);
                    }
                    Key::End => {
                        self.edit.key(EditKey::End, shift);
                        inv.push(self.base.bounds);
                    }
                    Key::Delete => {
                        self.edit.key(EditKey::DeleteForward, false);
                        self.changed = true;
                        inv.push(self.base.bounds);
                    }
                    _ => {}
                }
            }
            InputEvent::SelectAll if self.base.focused => {
                self.edit.key(EditKey::SelectAll, false);
                inv.push(self.base.bounds); // 선택 반전이 즉시 보여야 한다
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        ctx.fill_round_rect(b, self.s(6), theme.field_bg);
        ctx.stroke_round_rect(b, self.s(6), theme.border, 1.0);
        self.draw_focus_ring(ctx, theme, b);

        let cy = b.y + b.h / 2;
        let s16 = self.s(16);
        ctx.select_font(FontSlot::Base, false);
        let ty = cy - ctx.text_height() / 2;
        // 선행 이미지(있으면) — placeholder·텍스트·캐럿의 시작 x를 그 뒤로 민다.
        let mut tx = b.x + self.s(10);
        if let Some(img) = self.image.as_deref() {
            let boxr = Rect::new(tx, cy - s16 / 2, s16, s16);
            let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
            ctx.image_scaled(fit, img, b);
            tx += s16 + self.s(6);
        }

        // 텍스트/placeholder는 고정 시작점(tx)에서 그리되, 폭을 넘으면 **가로 스크롤**로
        // 캐럿을 따라간다(① 08-13 — 그전엔 긴 텍스트에서 캐럿이 화면 밖으로 사라졌다).
        ctx.select_font(FontSlot::Base, false);
        let text = self.edit.text();
        let chars: Vec<char> = text.chars().collect();
        let caret_i = self.edit.caret().min(chars.len());
        let before: String = chars[..caret_i].iter().collect();
        // 조합 중 문자열(preedit)은 캐럿 자리에 끼워 **표시만** 한다(편집 상태 불변).
        let shown = if self.preedit.is_empty() {
            text.clone()
        } else {
            let after: String = chars[caret_i..].iter().collect();
            format!("{before}{}{after}", self.preedit)
        };
        // 문자 경계 누적 폭 — 단일 패스(08-14 성능 · 값은 접두사 재측정과 동일 계약.
        // 캐럿 깜빡임이 포커스 창을 상시 리페인트해 매 프레임 O(n²) 측정이 비쌌다).
        let mut w = Vec::new();
        ctx.text_prefix_widths(&text, &mut w);
        let pre_start_px = w.get(caret_i).copied().unwrap_or(0);
        let caret_px = pre_start_px + ctx.text_width(&self.preedit); // 조합 뒤가 캐럿
        let total_px = if self.preedit.is_empty() {
            w.last().copied().unwrap_or(0) // shown == text — 같은 값(계약)
        } else {
            ctx.text_width(&shown) // 조합 중 한정 — 종전 그대로
        };
        // 가용 폭 — 우측 여백(×·도움말 배지 자리)을 뺀다.
        let avail = (b.right() - self.s(24) - tx).max(self.s(20));
        let mut hs = self.hscroll.get();
        if total_px <= avail {
            hs = 0; // 다 들어가면 스크롤 없음
        } else {
            hs = hs.clamp(0, total_px - avail); // 텍스트가 줄면 빈 공간이 남지 않게
            if caret_px - hs > avail {
                hs = caret_px - avail; // 캐럿이 오른쪽 밖 → 따라간다
            }
            if caret_px - hs < 0 {
                hs = caret_px; // 캐럿이 왼쪽 밖
            }
        }
        self.hscroll.set(hs);
        // 텍스트 뷰포트(스크롤 전 시작 ~ 가용 폭 끝) — fill_rect는 클립을 모르므로
        // 선택 반전·밑줄은 이 범위로 **직접 잘라** 그린다(08-13 실기: 하이라이트가
        // 좌우로 삐져나왔다). ★ 글자도 **같은 뷰포트로** 잘라야 한다(08-14 실기:
        // 글자는 상자 전체(b)로 잘라 우측 여백(×·배지 자리) 아래까지 보이는데
        // 하이라이트만 뷰포트에서 멈춰 "오른쪽 2글자 반전 누락"으로 보였다 —
        // 둘의 클립이 다르면 어느 쪽이 맞아도 어긋나 보인다).
        let (view_x0, view_x1) = (tx, tx + avail);
        let view = Rect::new(view_x0, b.y, avail, b.h);
        let tx = tx - hs;
        // 문자 경계 x(화면 좌표 — 스크롤 반영)를 남긴다 — 클릭→캐럿 변환 근거.
        {
            self.text_x.set(tx);
            let mut xs = self.caret_xs.borrow_mut();
            xs.clear();
            xs.extend(w.iter().map(|px| tx + px));
        }
        // 선택 반전(08-13 전수 검사: 선택은 되는데 하이라이트가 안 보였다) —
        // 텍스트보다 먼저 채워야 글자가 위에 얹힌다. preedit 중엔 선택이 없다.
        if let Some((a, b_end)) = self.edit.selection() {
            let mid: String = chars[a.min(chars.len())..b_end.min(chars.len())]
                .iter()
                .collect();
            let wp = w.get(a.min(chars.len())).copied().unwrap_or(0); // 접두사 폭(누적 재사용)
            let x0 = (tx + wp).max(view_x0);
            let x1 = (tx + wp + ctx.text_width(&mid)).min(view_x1);
            let th = ctx.text_height();
            if x1 > x0 {
                ctx.fill_rect(
                    Rect::new(x0, ty, x1 - x0, th),
                    if self.base.focused {
                        theme.sel_bg
                    } else {
                        theme.sel_bg_inactive
                    },
                );
            }
        }
        if shown.is_empty() {
            ctx.text(tx, ty, view, &self.placeholder, theme.text_dim);
        } else {
            ctx.text(tx, ty, view, &shown, theme.text);
        }
        // 조합 구간 밑줄 — "여기가 아직 확정 전"임을 대화 입력과 같은 문법으로 표시.
        if !self.preedit.is_empty() {
            let th = ctx.text_height();
            let ux0 = (tx + pre_start_px).max(view_x0);
            let ux1 = (tx + caret_px).min(view_x1).max(ux0 + self.s(4));
            ctx.fill_rect(
                Rect::new(ux0, ty + th, ux1 - ux0, self.s(1).max(1)),
                theme.text_dim,
            );
        }

        // 캐럿은 **별도 세로 막대**로 그린다(문자열에 '|'를 끼워 넣지 않음 → 위치 고정).
        // 깜빡임 위상(caret_on)은 호스트가 주입한다(08-13 — 포커스 창에서만 점멸).
        if self.base.focused && ctx.caret_on() {
            let cx = tx + caret_px;
            // 캐럿 높이 = 실측 텍스트 높이(고정 16 근사는 고배율에서 반토막으로 보였다).
            let th = ctx.text_height();
            ctx.fill_rect(Rect::new(cx, ty, self.s(2).max(2), th), theme.text);
        }

        // ×(지우기) — 값이 있을 때만(원 배경 없이 × 두 획 · text_dim).
        if self.clearable && !text.is_empty() {
            let r = self.clear_rect();
            let m = self.s(4);
            let (x0, y0, x1, y1) = (r.x + m, r.y + m, r.right() - m, r.bottom() - m);
            ctx.polyline(
                &[(x0, y0), (x1, y1)],
                theme.text_dim,
                self.s(1).max(1) as f32 + 0.5,
            );
            ctx.polyline(
                &[(x0, y1), (x1, y0)],
                theme.text_dim,
                self.s(1).max(1) as f32 + 0.5,
            );
        }

        let badge = self.help_badge_rect(b);
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);

        // 우클릭 편집 메뉴 — 이 위젯 안에서는 최상위. 형제 위젯이 뒤에 그려지는
        // 컨테이너에선 부족하다 — 컨테이너가 `paint_popup`을 끝에 한 번 더 부른다.
        self.paint_popup(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb() -> (TextBox, Invalidations) {
        let mut t = TextBox::new("Run command");
        let mut inv = Invalidations::default();
        t.set_bounds(Rect::new(0, 0, 260, 30), &mut inv);
        (t, inv)
    }
    fn ch(c: char) -> InputEvent {
        InputEvent::Char { c, now_ms: 0 }
    }
    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn primary_arrows_jump_line_edges() {
        let (mut t, mut inv) = tb();
        t.on_event(&click(5, 15), &mut inv);
        for c in "abc".chars() {
            t.on_event(&ch(c), &mut inv);
        }
        let key = |key, shift, primary| InputEvent::Key {
            key,
            shift,
            primary,
        };
        t.on_event(&key(Key::Left, false, true), &mut inv); // ⌘← = 줄 처음
        t.on_event(&ch('x'), &mut inv);
        assert_eq!(t.text(), "xabc", "⌘/Ctrl+← = 줄 처음(삽입 위치로 검증)");
        t.on_event(&key(Key::Right, false, true), &mut inv); // ⌘→ = 줄 끝
        t.on_event(&ch('z'), &mut inv);
        assert_eq!(t.text(), "xabcz", "⌘/Ctrl+→ = 줄 끝");
        t.on_event(&key(Key::Left, false, true), &mut inv);
        t.on_event(&key(Key::Right, true, true), &mut inv); // ⌘⇧→ = 끝까지 선택
        assert_eq!(
            t.copy_selection().as_deref(),
            Some("xabcz"),
            "⇧ 조합 = 범위 선택"
        );
    }

    #[test]
    fn typing_requires_focus_and_reports_change() {
        let (mut t, mut inv) = tb();
        t.on_event(&ch('a'), &mut inv);
        assert_eq!(t.text(), "", "비포커스 = 무입력");
        t.on_event(&click(5, 15), &mut inv);
        assert!(t.is_focused());
        for c in "git".chars() {
            t.on_event(&ch(c), &mut inv);
        }
        assert_eq!(t.text(), "git");
        assert_eq!(t.take_changed().as_deref(), Some("git"));
    }

    #[test]
    fn enter_commits_once() {
        let (mut t, mut inv) = tb();
        t.on_event(&click(5, 15), &mut inv);
        for c in "hi".chars() {
            t.on_event(&ch(c), &mut inv);
        }
        t.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(t.take_committed().as_deref(), Some("hi"));
        assert!(t.take_committed().is_none(), "1회성");
    }

    #[test]
    fn placeholder_present_until_typed() {
        let (t, _) = tb();
        assert_eq!(t.text(), "");
        // placeholder는 렌더 전용 — 텍스트 값에는 포함되지 않는다.
    }

    /// 페인트를 한 번 태워 문자 경계 실측을 채운다(클릭→캐럿 변환의 전제).
    fn measure(t: &TextBox) {
        use crate::controls::ProbeCtx;
        let mut probe = ProbeCtx;
        let theme = crate::theme::Theme::dark();
        t.paint(&mut probe, &theme);
    }

    #[test]
    fn select_all_selects_everything() {
        let (mut t, mut inv) = tb();
        t.set_text("hello world");
        t.base.focused = true;
        t.on_event(&InputEvent::SelectAll, &mut inv);
        assert_eq!(t.edit.selected_text().as_deref(), Some("hello world"));
    }

    #[test]
    fn double_click_selects_word_triple_selects_all() {
        let (mut t, mut inv) = tb();
        t.set_text("alpha beta");
        measure(&t);
        // 같은 자리 두 번 = 단어.
        t.on_event(&click(5, 15), &mut inv);
        t.on_event(&click(5, 15), &mut inv);
        assert_eq!(t.edit.selected_text().as_deref(), Some("alpha"));
        // 세 번째 = 전체.
        t.on_event(&click(5, 15), &mut inv);
        assert_eq!(t.edit.selected_text().as_deref(), Some("alpha beta"));
    }

    #[test]
    fn second_click_elsewhere_moves_caret_without_selecting() {
        // 08-13 실기 — 위치 무관 클릭 누적이라 두 번째 단일 클릭이 단어 선택이 됐다.
        let (mut t, mut inv) = tb();
        t.set_text("alpha beta");
        measure(&t);
        t.on_event(&click(5, 15), &mut inv); // 앞쪽
        let far = *t.caret_xs.borrow().last().unwrap(); // 맨 끝 경계
        t.on_event(&click(far, 15), &mut inv); // 다른 위치 = 새 체인
        assert!(
            t.edit.selected_text().is_none(),
            "다른 위치의 두 번째 클릭은 캐럿 이동이지 단어 선택이 아니다"
        );
        assert_eq!(t.edit.caret(), 10, "캐럿은 클릭 지점으로");
    }

    #[test]
    fn shift_click_extends_selection_not_word_select() {
        let (mut t, mut inv) = tb();
        t.set_text("alpha beta");
        measure(&t);
        t.on_event(&click(5, 15), &mut inv); // 캐럿 0
        let far = *t.caret_xs.borrow().last().unwrap();
        t.on_event(
            &InputEvent::MouseDown {
                x: far,
                y: 15,
                shift: true,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(
            t.edit.selected_text().as_deref(),
            Some("alpha beta"),
            "Shift+클릭 = 앵커에서 클릭 지점까지 확장"
        );
    }

    #[test]
    fn typing_breaks_click_chain() {
        let (mut t, mut inv) = tb();
        t.set_text("alpha");
        measure(&t);
        t.on_event(&click(5, 15), &mut inv);
        t.on_event(&ch('x'), &mut inv); // 키 개입 = 체인 끊김
        measure(&t);
        t.on_event(&click(5, 15), &mut inv);
        assert!(
            t.edit.selected_text().is_none(),
            "클릭-타이핑-클릭은 더블클릭이 아니다"
        );
    }

    #[test]
    fn movement_and_select_keys_request_repaint() {
        // 08-13 실기 — 이동·선택 키가 무효화를 안 밀어 프로필 필드에서
        // ←/→·Shift+←·⌘A가 무반응(화면 불변)으로 보였다.
        let (mut t, _) = tb();
        t.set_text("abc");
        t.base.focused = true;
        let key = |key, shift, primary| InputEvent::Key {
            key,
            shift,
            primary,
        };
        let mut inv = Invalidations::default();
        t.on_event(&key(Key::Left, true, false), &mut inv);
        assert!(!inv.is_empty(), "Shift+← = 다시 그리기 요청");
        let mut inv = Invalidations::default();
        t.on_event(&InputEvent::SelectAll, &mut inv);
        assert!(!inv.is_empty(), "⌘/Ctrl+A = 다시 그리기 요청");
        assert_eq!(t.edit.selected_text().as_deref(), Some("abc"));
    }

    #[test]
    fn display_text_includes_preedit_at_caret() {
        // 아바타 미리보기 등은 "화면에 보이는 그대로"를 써야 한다(08-13 실기).
        let (mut t, mut inv) = tb();
        t.set_text("나");
        t.base.focused = true;
        t.set_preedit("다", &mut inv);
        assert_eq!(t.display_text(), "나다", "조합 중 글자 포함");
        assert_eq!(t.text(), "나", "편집 상태(버퍼)는 불변");
        t.set_preedit("", &mut inv);
        assert_eq!(t.display_text(), "나", "소거 후엔 버퍼 그대로");
    }

    #[test]
    fn delete_key_removes_forward() {
        let (mut t, mut inv) = tb();
        t.set_text("abc");
        t.base.focused = true;
        t.edit.set_caret(0, false);
        t.on_event(
            &InputEvent::Key {
                key: Key::Delete,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(t.text(), "bc");
    }

    #[test]
    fn clear_button_resets_and_reports() {
        let mut t = TextBox::new("Search").with_clearable();
        let mut inv = Invalidations::default();
        t.set_bounds(Rect::new(0, 0, 260, 30), &mut inv);
        t.set_text("abc");
        let r = t.clear_rect();
        t.on_event(&click(r.x + 3, r.y + 3), &mut inv);
        assert_eq!(t.text(), "", "× = 초기화");
        assert_eq!(t.take_changed().as_deref(), Some(""), "변경 보고");
        // 값이 없으면 × 영역 클릭은 일반 포커스 클릭.
        t.on_event(&click(r.x + 3, r.y + 3), &mut inv);
        assert_eq!(t.text(), "");
        assert!(t.is_focused());
    }

    #[test]
    fn backspace_edits() {
        let (mut t, mut inv) = tb();
        t.set_text("abc");
        t.base.focused = true;
        t.on_event(&ch('\u{8}'), &mut inv);
        assert_eq!(t.text(), "ab");
    }

    #[test]
    fn clipboard_copy_cut_paste_roundtrip() {
        // ① 08-13 — 모든 텍스트 컨트롤의 기본 에디터 기능(클립보드는 호스트가 잇는다).
        let (mut t, mut inv) = tb();
        t.set_text("hello world");
        t.base.focused = true;
        t.edit.set_selection(0, 5);
        assert_eq!(t.copy_selection().as_deref(), Some("hello"), "복사");
        assert_eq!(t.text(), "hello world", "복사는 내용 불변");
        assert_eq!(
            t.cut_selection(&mut inv).as_deref(),
            Some("hello"),
            "잘라내기"
        );
        assert_eq!(t.text(), " world");
        // 붙여넣기 — 개행·제어문자는 공백 하나로 접는다(단일 행 컨트롤).
        t.edit.set_caret(0, false);
        t.paste("multi\nline\ttext", &mut inv);
        assert_eq!(t.text(), "multi line text world");
        // 비포커스면 전부 무시(다른 컨트롤의 단축키를 삼키지 않는다).
        t.base.focused = false;
        assert!(t.copy_selection().is_none());
        assert!(t.cut_selection(&mut inv).is_none());
        let before = t.text();
        t.paste("x", &mut inv);
        assert_eq!(t.text(), before);
    }

    #[test]
    fn drag_beyond_edges_advances_selection() {
        // ① — 영역 밖 드래그 = 한 글자씩 자동 진행(페인트가 캐럿을 따라 스크롤).
        let (mut t, mut inv) = tb();
        t.set_text("abcdef");
        measure(&t);
        t.on_event(&click(5, 15), &mut inv); // 앞쪽 클릭 → 드래그 시작
        let start = t.edit.caret();
        let b = t.bounds();
        for _ in 0..3 {
            t.on_event(
                &InputEvent::MouseMove {
                    x: b.right() + 20,
                    y: 15,
                },
                &mut inv,
            );
        }
        assert_eq!(t.edit.caret(), (start + 3).min(6), "오른쪽 밖 = +1씩");
        assert!(t.edit.selected_text().is_some(), "앵커 유지 = 선택 확장");
        for _ in 0..10 {
            t.on_event(&InputEvent::MouseMove { x: b.x - 20, y: 15 }, &mut inv);
        }
        assert_eq!(t.edit.caret(), 0, "왼쪽 밖 = -1씩(0에서 멈춤)");
    }

    /// fill_rect만 기록하는 캔버스 — 선택 반전·밑줄·캐럿이 상자를 넘는지 검증.
    struct RecCtx(Vec<Rect>);
    impl crate::draw::DrawCtx for RecCtx {
        fn fill_rect(&mut self, r: Rect, _c: crate::theme::Color) {
            self.0.push(r);
        }
        fn text_opaque(
            &mut self,
            _x: i32,
            _y: i32,
            _clip: Rect,
            _t: &str,
            _f: crate::theme::Color,
            _b: crate::theme::Color,
        ) {
        }
        fn text(&mut self, _x: i32, _y: i32, _clip: Rect, _t: &str, _f: crate::theme::Color) {}
        fn text_width(&mut self, text: &str) -> i32 {
            text.chars().count() as i32 * 7
        }
    }

    #[test]
    fn selection_highlight_clipped_to_box() {
        // 08-13 실기 — 가로 스크롤 상태의 전체 선택 하이라이트가 컨트롤 좌우로
        // 삐져나왔다(fill_rect는 클립을 모른다 — 뷰포트로 직접 잘라야 한다).
        let (mut t, _inv) = tb();
        t.set_text(&"가나다라마바사아자차".repeat(10)); // 260px 상자를 확실히 넘긴다
        t.base.focused = true;
        t.edit.key(EditKey::SelectAll, false); // 선택 끝(=캐럿)이 오른쪽 밖
        let mut rec = RecCtx(Vec::new());
        let theme = crate::theme::Theme::dark();
        t.paint(&mut rec, &theme);
        let b = t.bounds();
        assert!(!rec.0.is_empty(), "선택 반전이 그려져야 한다");
        for r in &rec.0 {
            assert!(
                r.x >= b.x && r.right() <= b.right(),
                "채움이 상자 밖으로 나갔다: {r:?} vs 상자 {b:?}"
            );
        }
        // ★ 08-14 실기 — 하이라이트가 **뷰포트를 정확히 채워야** 한다(글자 클립과
        // 동일 범위). 좌 10px·우 24px 어긋남이 "좌우 글자 반전 누락"으로 보였다.
        let (vx0, vx1) = (b.x + 10, b.right() - 24); // scale 1.0 기준 s(10)·s(24)
        assert!(
            rec.0.iter().any(|r| r.x == vx0 && r.right() == vx1),
            "전체 선택(스크롤 중) 하이라이트가 뷰포트 전폭이어야 한다: {:?}",
            rec.0
        );
        // 왼쪽 밖 케이스 — 캐럿을 앞으로 옮겨 스크롤을 왼쪽 끝으로 되돌린 뒤
        // 다시 전체 선택(앵커 끝 유지 → 선택이 왼쪽 밖까지 걸치는 상태).
        t.edit.set_caret(0, true);
        let mut rec = RecCtx(Vec::new());
        t.paint(&mut rec, &theme);
        for r in &rec.0 {
            assert!(
                r.x >= b.x && r.right() <= b.right(),
                "왼쪽 케이스 — 채움이 상자 밖: {r:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn hscroll_follows_caret_and_resets_when_fits() {
        // ① — 긴 텍스트에서 캐럿이 항상 보인다(그전엔 오른쪽 밖으로 사라졌다).
        let (mut t, _inv) = tb();
        t.set_text(&"m".repeat(200)); // 260px 상자를 확실히 넘긴다
        t.base.focused = true;
        t.edit.set_caret(200, false);
        measure(&t); // 페인트가 스크롤을 조정한다
        assert!(t.hscroll.get() > 0, "캐럿(끝)이 보이려면 스크롤돼야 한다");
        t.set_text("short");
        measure(&t);
        assert_eq!(t.hscroll.get(), 0, "다 들어가면 스크롤 없음");
    }
}
