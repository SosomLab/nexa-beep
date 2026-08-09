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
    /// 마지막 클릭 (시각 ms, 연속 횟수) — 더블·트리플 판정.
    last_click: (u64, u8),
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

    /// Enter 확정되었으면 텍스트를 꺼낸다(1회성).
    pub fn take_committed(&mut self) -> Option<String> {
        std::mem::take(&mut self.committed).then(|| self.edit.text())
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
                    self.last_click.1 = if self.last_click.1 >= 3 {
                        1
                    } else {
                        self.last_click.1 + 1
                    };
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
                let idx = self.caret_at_x(x);
                self.edit.set_caret(idx, true); // 앵커 유지 = 범위 확장
                inv.push(self.base.bounds);
            }
            InputEvent::MouseUp { .. } => {
                self.dragging = false;
            }
            InputEvent::Char { c, .. } if self.base.focused => {
                if c == '\u{8}' {
                    self.edit.backspace();
                } else if !c.is_control() {
                    self.edit.insert(c);
                }
                self.changed = true;
                inv.push(self.base.bounds);
            }
            InputEvent::Key { key, shift, .. } if self.base.focused => match key {
                Key::Enter => {
                    self.committed = true;
                    inv.push(self.base.bounds);
                }
                Key::Left => self.edit.key(EditKey::Left, shift),
                Key::Right => self.edit.key(EditKey::Right, shift),
                Key::Home => self.edit.key(EditKey::Home, shift),
                Key::End => self.edit.key(EditKey::End, shift),
                Key::Delete => {
                    self.edit.key(EditKey::DeleteForward, false);
                    self.changed = true;
                    inv.push(self.base.bounds);
                }
                _ => {}
            },
            InputEvent::SelectAll if self.base.focused => {
                self.edit.key(EditKey::SelectAll, false);
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

        // 텍스트/placeholder는 **항상 고정 위치**(tx)에 그린다(캐럿으로 밀리지 않게).
        ctx.select_font(FontSlot::Base, false);
        let text = self.edit.text();
        // 문자 경계 x를 실측해 남긴다 — 이벤트 경로는 폰트를 모른다(클릭→캐럿 변환 근거).
        {
            self.text_x.set(tx);
            let mut xs = self.caret_xs.borrow_mut();
            xs.clear();
            xs.push(tx);
            let mut acc = String::new();
            for ch in text.chars() {
                acc.push(ch);
                xs.push(tx + ctx.text_width(&acc));
            }
        }
        if text.is_empty() {
            ctx.text(tx, ty, b, &self.placeholder, theme.text_dim);
        } else {
            ctx.text(tx, ty, b, &text, theme.text);
        }

        // 캐럿은 **별도 세로 막대**로 그린다(문자열에 '|'를 끼워 넣지 않음 → 위치 고정).
        if self.base.focused {
            let chars: Vec<char> = text.chars().collect();
            let upto: String = chars[..self.edit.caret().min(chars.len())].iter().collect();
            let cx = tx + ctx.text_width(&upto);
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
}
