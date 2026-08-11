//! 토글 스위치 — mac(iOS) 스타일(사용자 요청 08-11 · 참고 이미지 그대로):
//! 알약형 트랙 + 흰 원형 손잡이, 켜짐 = 초록(#34C759) · 꺼짐 = 회색 트랙.
//!
//! [`Checkbox`](super::Checkbox)와 같은 사용 계약 — 토글만([`LabelSide::None`]) /
//! 라벨 좌·우 선택, `take_toggled` 1회성 보고, Space/Enter 키 토글.
//! 시각은 macOS 규약(DR-15) — 색만이 아니라 **손잡이 위치**가 상태를 말한다
//! (색각 이상에도 좌=꺼짐/우=켜짐이 구분된다 · FR-U 색 단독 구분 금지).

use super::{Control, ControlBase, LabelSide};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::{Color, Theme};
use crate::widget::{Invalidations, Widget};

/// 트랙 크기(논리 px) — iOS 51×31 비례 축소. 초판 40×24는 과대(사용자 지적 08-11
/// "절반으로") → 20×12.
const TRACK_W: i32 = 20;
const TRACK_H: i32 = 12;
/// 손잡이 상하좌우 여백.
const KNOB_PAD: i32 = 1;
/// 트랙 ↔ 라벨 간격(논리 px).
const GAP: i32 = 8;
/// 켜짐 트랙 — iOS 시스템 그린(#34C759) · 다크/라이트 공통(참고 이미지 색).
const ON_GREEN: Color = Color(0x0034_C759);
/// 손잡이 — 흰 원(iOS 공통).
const KNOB_WHITE: Color = Color(0x00FF_FFFF);

/// 토글 스위치 컨트롤.
#[derive(Debug)]
pub struct Switch {
    base: ControlBase,
    on: bool,
    label: String,
    side: LabelSide,
    /// 값 변경 1회성 보고(즉시 적용 폴링) — Checkbox와 같은 계약.
    toggled: bool,
}

impl Switch {
    /// 라벨과 초기 상태로 만든다(라벨 오른쪽이 기본 · 토글만은 [`LabelSide::None`]).
    #[must_use]
    pub fn new(label: impl Into<String>, on: bool) -> Self {
        Self {
            base: ControlBase::default(),
            on,
            label: label.into(),
            side: LabelSide::Right,
            toggled: false,
        }
    }

    /// 라벨 위치 지정(토글만 = [`LabelSide::None`]).
    #[must_use]
    pub fn with_label_side(mut self, side: LabelSide) -> Self {
        self.side = side;
        self
    }

    /// 현재 상태.
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// 상태 지정(보고 없음 — 호스트 주도 설정).
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    /// 토글되었으면 새 상태를 꺼낸다(1회성).
    pub fn take_toggled(&mut self) -> Option<bool> {
        std::mem::take(&mut self.toggled).then_some(self.on)
    }

    fn toggle(&mut self, inv: &mut Invalidations) {
        self.on = !self.on;
        self.toggled = true;
        inv.push(self.base.bounds);
    }

    /// 트랙 rect(라벨 위치에 따라 좌/우 끝 정렬). 크기 배율(`ui.control_size`) 적용.
    fn track_rect(&self) -> Rect {
        let w = self.s(super::ctl_size(TRACK_W));
        let h = self.s(super::ctl_size(TRACK_H));
        let b = self.base.bounds;
        let y = b.y + (b.h - h) / 2;
        match self.side {
            // 라벨이 왼쪽이면 트랙은 오른쪽 끝(설정 행 관례) — 그 외엔 왼쪽.
            LabelSide::Left => Rect::new(b.right() - w, y, w, h),
            LabelSide::None | LabelSide::Right => Rect::new(b.x, y, w, h),
        }
    }

    fn label_rect(&self) -> Rect {
        let w = self.s(super::ctl_size(TRACK_W));
        let gap = self.s(GAP);
        let b = self.base.bounds;
        match self.side {
            LabelSide::Right => Rect::new(b.x + w + gap, b.y, b.w - w - gap, b.h),
            LabelSide::Left => Rect::new(b.x, b.y, b.w - w - gap, b.h),
            LabelSide::None => Rect::new(b.x, b.y, 0, b.h),
        }
    }
}

impl Control for Switch {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for Switch {
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
                let badge = self.help_badge_rect(self.base.bounds);
                if self.handle_help_click(x, y, badge) {
                    inv.push(self.base.bounds);
                    return;
                }
                if self.base.bounds.contains(Point { x, y }) {
                    self.toggle(inv);
                }
            }
            InputEvent::Key { key, .. } if self.base.focused => {
                if matches!(key, Key::Space | Key::Enter) {
                    self.toggle(inv);
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let tr = self.track_rect();
        let radius = tr.h / 2;
        self.draw_focus_ring(ctx, theme, tr);
        // 트랙 — 켜짐 = iOS 그린 · 꺼짐 = 중립 회색(테마 대체 패널색 + 1px 테두리 —
        // 라이트 모드에서 흰 배경과 붙는 것 방지).
        if self.on {
            ctx.fill_round_rect(tr, radius, ON_GREEN);
        } else {
            ctx.fill_round_rect(tr, radius, theme.panel_bg_alt);
            ctx.stroke_round_rect(tr, radius, theme.border, 1.0);
        }
        // 손잡이 — 흰 원. **위치가 상태를 말한다**(좌=꺼짐 · 우=켜짐).
        let pad = self.s(KNOB_PAD);
        let d = tr.h - 2 * pad;
        let kx = if self.on {
            tr.right() - pad - d
        } else {
            tr.x + pad
        };
        let knob = Rect::new(kx, tr.y + pad, d, d);
        ctx.fill_ellipse(knob, KNOB_WHITE);
        // 라벨.
        if self.side != LabelSide::None && !self.label.is_empty() {
            let lr = self.label_rect();
            ctx.select_font(FontSlot::Base, false);
            let ty = lr.y + (lr.h - ctx.text_height()) / 2;
            ctx.text(lr.x, ty, lr, &self.label, theme.text);
        }
        let badge = self.help_badge_rect(self.base.bounds);
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn sw(mut s: Switch) -> (Switch, Invalidations) {
        let mut inv = Invalidations::default();
        s.set_bounds(Rect::new(0, 0, 160, 28), &mut inv);
        (s, inv)
    }
    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: true,
        }
    }

    #[test]
    fn click_toggles_once() {
        let (mut s, mut inv) = sw(Switch::new("알림", false));
        s.on_event(&click(10, 14), &mut inv);
        assert_eq!(s.take_toggled(), Some(true), "클릭 = 켜짐");
        assert_eq!(s.take_toggled(), None, "1회성");
        s.on_event(&click(10, 14), &mut inv);
        assert_eq!(s.take_toggled(), Some(false), "다시 클릭 = 꺼짐");
    }

    #[test]
    fn space_toggles_only_when_focused() {
        let (mut s, mut inv) = sw(Switch::new("", false).with_label_side(LabelSide::None));
        let space = InputEvent::Key {
            key: Key::Space,
            shift: false,
            primary: false,
        };
        s.on_event(&space, &mut inv);
        assert!(!s.is_on(), "비포커스 = 무시");
        s.set_focused(true);
        s.on_event(&space, &mut inv);
        assert!(s.is_on(), "포커스 = 토글");
        s.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(!s.is_on(), "Enter도 토글");
    }

    #[test]
    fn label_side_moves_track() {
        let (l, _) = sw(Switch::new("라벨", false).with_label_side(LabelSide::Left));
        let (r, _) = sw(Switch::new("라벨", false).with_label_side(LabelSide::Right));
        assert!(
            l.track_rect().x > r.track_rect().x,
            "라벨 좌 = 트랙 우측 끝 / 라벨 우 = 트랙 좌측"
        );
        // 토글만 — 라벨 영역 폭 0.
        let (n, _) = sw(Switch::new("무시", false).with_label_side(LabelSide::None));
        assert_eq!(n.label_rect().w, 0);
    }

    #[test]
    fn knob_position_encodes_state() {
        let (mut s, mut inv) = sw(Switch::new("", false).with_label_side(LabelSide::None));
        let tr = s.track_rect();
        let pad = s.s(KNOB_PAD);
        let d = tr.h - 2 * pad;
        let off_x = tr.x + pad;
        let on_x = tr.right() - pad - d;
        assert!(off_x < on_x, "좌=꺼짐 · 우=켜짐 위치 구분");
        s.on_event(&click(tr.x + 2, tr.y + 2), &mut inv);
        assert!(s.is_on());
    }
}
