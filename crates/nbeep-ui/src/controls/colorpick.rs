//! **색상 선택기** — 스와치 + `#RRGGBB` 직접 입력 + 프리셋 팔레트(사용자 요청 08-10).
//!
//! 테마 주요 색을 사용자가 바꾸는 자리(설정 · 모양 ▸ 색상). 값은 항상 `#RRGGBB`
//! 문자열로 오간다(설정 저장 형식과 동일). 적용은 **Enter 확정** 또는 프리셋 클릭 —
//! 글자마다 파싱하지 않는다(Face 글꼴 입력과 같은 규약). 잘못된 형식은 조용히
//! 버리지 않고 **직전 값으로 원복**해 보여 준다.

use super::{Control, ControlBase, TextBox};
use crate::draw::DrawCtx;
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::{color_from_hex, color_to_hex, Color, Theme};
use crate::widget::{Invalidations, Widget};

/// 현재 값 스와치 변 길이(논리 px).
const SWATCH: i32 = 26;
/// 프리셋 스와치 변 길이.
const PRESET: i32 = 18;
/// hex 입력 폭.
const HEX_W: i32 = 88;
/// 간격.
const GAP: i32 = 6;

/// 프리셋 팔레트 — 자주 쓰는 6색(강조·상태색 계열 + 무채 2).
const PRESETS: [u32; 6] = [
    0x003D_8BFF, // 파랑(기본 강조)
    0x002E_A043, // 초록
    0x00B5_7C1E, // 황
    0x00E5_534B, // 빨강
    0x0031_3947, // 어두운 회(다크 수신 풍선)
    0x00E2_E7EE, // 밝은 회(라이트 수신 풍선)
];

/// 색상 선택 컨트롤 — [스와치][#RRGGBB 입력][프리셋 6].
#[derive(Debug)]
pub struct ColorPicker {
    base: ControlBase,
    /// 현재 값(0x00RRGGBB).
    value: u32,
    hex: TextBox,
    changed: bool,
}

impl ColorPicker {
    /// 초기 hex 값으로 만든다(형식 오류면 기본 강조색).
    #[must_use]
    pub fn new(initial: &str) -> Self {
        let value = color_from_hex(initial).map_or(0x003D_8BFF, |c| c.0);
        Self {
            base: ControlBase::default(),
            value,
            hex: TextBox::new("#RRGGBB").with_text(&color_to_hex(Color(value))),
            changed: false,
        }
    }

    /// 현재 값 `#RRGGBB`.
    #[must_use]
    pub fn value_hex(&self) -> String {
        color_to_hex(Color(self.value))
    }

    /// 값 지정(보고 없음 · 형식 오류 = 무시).
    pub fn set_value(&mut self, hex: &str) {
        if let Some(c) = color_from_hex(hex) {
            self.value = c.0;
            self.hex.set_text(&self.value_hex());
        }
    }

    /// 값 변경 1회성 보고(`#RRGGBB`).
    pub fn take_changed(&mut self) -> Option<String> {
        std::mem::take(&mut self.changed).then(|| self.value_hex())
    }

    /// hex 입력이 포커스 상태인가 — 호스트의 타이핑 라우팅 판단용(검색 폴백 차단).
    #[must_use]
    pub fn hex_focused(&self) -> bool {
        self.hex.is_focused()
    }

    /// 권장 폭(물리 px) — 호스트 레이아웃용.
    #[must_use]
    pub fn preferred_width(&self) -> i32 {
        self.s(SWATCH)
            + self.s(GAP)
            + self.s(HEX_W)
            + self.s(GAP)
            + (self.s(PRESET) + self.s(4)) * PRESETS.len() as i32
    }

    /// 포커스 지정 — 내장 hex 입력의 포커스도 함께 정리한다(잔존 방지 · 08-09 교훈).
    pub fn set_focused(&mut self, focused: bool) {
        self.base.focused = focused;
        if !focused {
            self.hex.set_focused(false);
        }
    }

    /// 배율 지정 — 내장 입력에도 전파.
    pub fn set_scale(&mut self, scale: f32) {
        self.base.scale = scale;
        self.hex.set_scale(scale);
    }

    fn swatch_rect(&self) -> Rect {
        let b = self.base.bounds;
        let s = self.s(SWATCH);
        Rect::new(b.x, b.y + (b.h - s) / 2, s, s)
    }

    fn preset_rect(&self, i: usize) -> Rect {
        let b = self.base.bounds;
        let ps = self.s(PRESET);
        let x0 = b.x + self.s(SWATCH) + self.s(GAP) + self.s(HEX_W) + self.s(GAP);
        Rect::new(
            x0 + (ps + self.s(4)) * i as i32,
            b.y + (b.h - ps) / 2,
            ps,
            ps,
        )
    }

    fn commit(&mut self, hex: &str, inv: &mut Invalidations) {
        if let Some(c) = color_from_hex(hex) {
            if c.0 != self.value {
                self.value = c.0;
                self.changed = true;
            }
            self.hex.set_text(&self.value_hex()); // 표기 정규화(#·대문자)
        } else {
            // 형식 오류 — 직전 값으로 원복(조용히 버리지 않는다).
            self.hex.set_text(&self.value_hex());
        }
        inv.push(self.base.bounds);
    }
}

impl Control for ColorPicker {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for ColorPicker {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        let hx = bounds.x + self.s(SWATCH) + self.s(GAP);
        let hh = self.s(26).min(bounds.h);
        self.hex.set_bounds(
            Rect::new(hx, bounds.y + (bounds.h - hh) / 2, self.s(HEX_W), hh),
            inv,
        );
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        if let InputEvent::MouseDown { x, y, .. } = *ev {
            let p = Point { x, y };
            // 프리셋 클릭 = 즉시 적용.
            for (i, &c) in PRESETS.iter().enumerate() {
                if self.preset_rect(i).contains(p) {
                    self.base.focused = true;
                    if c != self.value {
                        self.value = c;
                        self.changed = true;
                    }
                    self.hex.set_text(&self.value_hex());
                    inv.push(self.base.bounds);
                    return;
                }
            }
            self.hex.set_focused(self.hex.bounds().contains(p));
            self.base.focused = self.base.bounds.contains(p);
        }
        // 내장 입력으로 전달(포커스 게이트는 TextBox 자신이 한다).
        self.hex.on_event(ev, inv);
        if let Some(t) = self.hex.take_committed() {
            self.commit(&t, inv);
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        // 현재 값 스와치.
        let sw = self.swatch_rect();
        ctx.fill_round_rect(sw, self.s(4), Color(self.value));
        ctx.stroke_round_rect(sw, self.s(4), theme.border, 1.0);
        self.hex.paint(ctx, theme);
        // 프리셋.
        let accent = self.accent_now(theme);
        for (i, &c) in PRESETS.iter().enumerate() {
            let r = self.preset_rect(i);
            ctx.fill_round_rect(r, self.s(3), Color(c));
            if c == self.value {
                ctx.stroke_round_rect(r, self.s(3), accent, self.s(2).max(2) as f32);
            } else {
                ctx.stroke_round_rect(r, self.s(3), theme.border, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Key;

    fn picker() -> (ColorPicker, Invalidations) {
        let mut p = ColorPicker::new("#3D8BFF");
        let mut inv = Invalidations::default();
        p.set_bounds(Rect::new(0, 0, 340, 30), &mut inv);
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
    fn ch(p: &mut ColorPicker, c: char, inv: &mut Invalidations) {
        p.on_event(&InputEvent::Char { c, now_ms: 0 }, inv);
    }

    #[test]
    fn preset_click_applies_and_reports_once() {
        let (mut p, mut inv) = picker();
        let r = p.preset_rect(1); // 초록
        p.on_event(&click(r.x + 2, r.y + 2), &mut inv);
        assert_eq!(p.take_changed().as_deref(), Some("#2EA043"));
        assert!(p.take_changed().is_none(), "1회성");
        // 같은 프리셋 재클릭 = 변경 없음.
        p.on_event(&click(r.x + 2, r.y + 2), &mut inv);
        assert!(p.take_changed().is_none());
    }

    #[test]
    fn hex_enter_commits_and_normalizes() {
        let (mut p, mut inv) = picker();
        let hb = p.hex.bounds();
        p.on_event(&click(hb.x + 3, hb.y + 3), &mut inv);
        assert!(p.hex_focused());
        // 전체 선택 후 새 값 타이핑(# 없이 소문자).
        p.on_event(&InputEvent::SelectAll, &mut inv);
        for c in "e5534b".chars() {
            ch(&mut p, c, &mut inv);
        }
        p.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(p.take_changed().as_deref(), Some("#E5534B"));
        assert_eq!(p.value_hex(), "#E5534B");
    }

    #[test]
    fn invalid_hex_reverts_without_report() {
        let (mut p, mut inv) = picker();
        let hb = p.hex.bounds();
        p.on_event(&click(hb.x + 3, hb.y + 3), &mut inv);
        p.on_event(&InputEvent::SelectAll, &mut inv);
        for c in "zzz".chars() {
            ch(&mut p, c, &mut inv);
        }
        p.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(p.take_changed().is_none(), "형식 오류 = 보고 없음");
        assert_eq!(p.value_hex(), "#3D8BFF", "직전 값 유지");
    }

    #[test]
    fn set_value_ignores_garbage() {
        let (mut p, _) = picker();
        p.set_value("#12345"); // 5자리 — 무시
        assert_eq!(p.value_hex(), "#3D8BFF");
        p.set_value("2EA043");
        assert_eq!(p.value_hex(), "#2EA043");
    }
}
