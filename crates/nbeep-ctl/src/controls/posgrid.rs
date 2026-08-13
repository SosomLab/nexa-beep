//! **위치 선택기** — 3×3 미니 화면 그리드로 위치를 직관 선택(사용자 요청 08-09).
//!
//! 각 셀은 **4:3 가로 화면**의 축소판이고, 그 안에 해당 위치(좌상단이면 좌상단)에
//! 작은 회색 박스가 그려진다. 선택된 셀은 테두리·박스가 강조색으로 바뀐다.
//! 값 코드는 `HudPos::from_code`(nbeep-ui `peer_list` — 앱 측) 체계(`tl…c…br`)와 동일.
//! 클릭 또는 (포커스 시) 방향키로 고르고, [`PositionPicker::take_changed`] 1회성 보고.

use super::{Control, ControlBase};
use crate::draw::DrawCtx;
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 행우선(상단→하단) 위치 코드 — `HudPos::from_code`와 동일 체계.
const CODES: [&str; 9] = ["tl", "tc", "tr", "ml", "c", "mr", "bl", "bc", "br"];

/// 셀(미니 화면) 크기 — 4:3 가로(논리 px).
const CELL_W: i32 = 36;
const CELL_H: i32 = 27;
/// 셀 간격(논리 px).
const GAP: i32 = 6;
/// 미니 화면 안 박스 크기·여백(논리 px).
const BOX_W: i32 = 9;
const BOX_H: i32 = 7;
const BOX_M: i32 = 4;

/// 3×3 위치 선택 컨트롤.
#[derive(Debug)]
pub struct PositionPicker {
    base: ControlBase,
    /// 선택 인덱스(0..9 · 행우선).
    selected: usize,
    changed: bool,
}

impl Default for PositionPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionPicker {
    /// 기본(좌하 `bl`) 선택으로 만든다.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: ControlBase::default(),
            selected: 6, // "bl"
            changed: false,
        }
    }

    /// 그리드 권장 크기(물리 px) — 호스트 레이아웃용.
    #[must_use]
    pub fn preferred_size(&self) -> (i32, i32) {
        (
            self.s(CELL_W) * 3 + self.s(GAP) * 2,
            self.s(CELL_H) * 3 + self.s(GAP) * 2,
        )
    }

    /// 현재 값 코드.
    #[must_use]
    pub fn value(&self) -> String {
        CODES[self.selected].to_string()
    }

    /// 값 코드로 선택 지정(보고 없음 · 미지 코드 = 무시).
    pub fn select_value(&mut self, code: &str) {
        if let Some(i) = CODES.iter().position(|c| *c == code) {
            self.selected = i;
        }
    }

    /// 값 변경 1회성 보고.
    pub fn take_changed(&mut self) -> Option<String> {
        std::mem::take(&mut self.changed).then(|| self.value())
    }

    fn cell_rect(&self, i: usize) -> Rect {
        let b = self.base.bounds;
        let (cw, ch, gap) = (self.s(CELL_W), self.s(CELL_H), self.s(GAP));
        let (row, col) = (i / 3, i % 3);
        Rect::new(
            b.x + (cw + gap) * col as i32,
            b.y + (ch + gap) * row as i32,
            cw,
            ch,
        )
    }

    fn cell_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..9).find(|&i| self.cell_rect(i).contains(Point { x, y }))
    }

    fn pick(&mut self, i: usize, inv: &mut Invalidations) {
        if i != self.selected {
            self.selected = i;
            self.changed = true;
        }
        inv.push(self.base.bounds);
    }

    /// 미니 화면 안 박스 rect — 셀 i의 위치(좌상단 셀 = 좌상단 박스).
    fn box_rect(&self, cell: Rect, i: usize) -> Rect {
        let (bw, bh, m) = (self.s(BOX_W), self.s(BOX_H), self.s(BOX_M));
        let (row, col) = (i / 3, i % 3);
        let x = match col {
            0 => cell.x + m,
            1 => cell.x + (cell.w - bw) / 2,
            _ => cell.right() - m - bw,
        };
        let y = match row {
            0 => cell.y + m,
            1 => cell.y + (cell.h - bh) / 2,
            _ => cell.bottom() - m - bh,
        };
        Rect::new(x, y, bw, bh)
    }
}

impl Control for PositionPicker {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for PositionPicker {
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
                if let Some(i) = self.cell_at(x, y) {
                    self.base.focused = true;
                    self.pick(i, inv);
                }
            }
            InputEvent::Key { key, .. } if self.base.focused => {
                let (row, col) = (self.selected / 3, self.selected % 3);
                let next = match key {
                    Key::Left if col > 0 => Some(self.selected - 1),
                    Key::Right if col < 2 => Some(self.selected + 1),
                    Key::Up if row > 0 => Some(self.selected - 3),
                    Key::Down if row < 2 => Some(self.selected + 3),
                    _ => None,
                };
                if let Some(i) = next {
                    self.pick(i, inv);
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let accent = self.accent_now(theme);
        for i in 0..9 {
            let cell = self.cell_rect(i);
            let sel = i == self.selected;
            ctx.fill_round_rect(cell, self.s(3), theme.field_bg);
            if sel {
                ctx.stroke_round_rect(cell, self.s(3), accent, self.s(2).max(2) as f32);
            } else {
                ctx.stroke_round_rect(cell, self.s(3), theme.border, 1.0);
            }
            let bx = self.box_rect(cell, i);
            ctx.fill_round_rect(bx, self.s(1), if sel { accent } else { theme.text_dim });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> (PositionPicker, Invalidations) {
        let mut p = PositionPicker::new();
        let mut inv = Invalidations::default();
        let (w, h) = p.preferred_size();
        p.set_bounds(Rect::new(0, 0, w, h), &mut inv);
        (p, inv)
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
    fn default_is_bottom_left_and_click_reports_once() {
        let (mut p, mut inv) = picker();
        assert_eq!(p.value(), "bl", "기본 좌하");
        let tr = p.cell_rect(2);
        p.on_event(&click(tr.x + 5, tr.y + 5), &mut inv);
        assert_eq!(p.take_changed().as_deref(), Some("tr"));
        assert!(p.take_changed().is_none(), "1회성");
        // 같은 셀 재클릭 = 변경 보고 없음.
        p.on_event(&click(tr.x + 5, tr.y + 5), &mut inv);
        assert!(p.take_changed().is_none());
    }

    #[test]
    fn box_follows_cell_position() {
        let (p, _) = picker();
        let c0 = p.cell_rect(0);
        let b0 = p.box_rect(c0, 0);
        assert!(b0.x - c0.x < c0.w / 2 && b0.y - c0.y < c0.h / 2, "좌상단");
        let c8 = p.cell_rect(8);
        let b8 = p.box_rect(c8, 8);
        assert!(
            b8.right() > c8.x + c8.w / 2 && b8.bottom() > c8.y + c8.h / 2,
            "우하단"
        );
        // 4:3 가로 화면.
        assert!(c0.w * 3 == c0.h * 4, "셀 = 4:3");
    }

    #[test]
    fn arrows_move_selection_when_focused() {
        let (mut p, mut inv) = picker();
        p.set_focused(true);
        let key = |key| InputEvent::Key {
            key,
            shift: false,
            primary: false,
        };
        p.on_event(&key(Key::Up), &mut inv); // bl → ml
        assert_eq!(p.take_changed().as_deref(), Some("ml"));
        p.on_event(&key(Key::Right), &mut inv); // ml → c
        assert_eq!(p.take_changed().as_deref(), Some("c"));
        p.on_event(&key(Key::Left), &mut inv);
        p.on_event(&key(Key::Left), &mut inv); // 경계 클램프
        assert_eq!(
            p.take_changed().as_deref(),
            Some("ml"),
            "좌측 경계에서 멈춤"
        );
    }
}
