//! **목록 필터 바**(08-22 사용자 확정) — 툴바 아래 납작한 칩 3그룹.
//!
//! - 그룹1 경로: 전체·로컬·서버·인터넷·그룹 / 그룹2 상태: 전체·온라인·오프라인 /
//!   그룹3 신뢰: 전체·인증·핀·신규. 각 그룹 **단일 선택**(라디오), 셋은 AND 결합.
//! - 높이는 툴바의 1/3 수준([`FILTER_H`] = 16 논리 px — 사용자 확정).
//! - 선택은 설정 키(`list.filter.*`)로 영속 — 호스트가 [`Self::set_selection`]으로
//!   복원하고 [`Self::take_changed`]로 변경을 받아 저장·재조립한다.
//! - 칩 좌표는 페인트가 실측(글꼴 폭)으로 깔고 캐시에 남긴다 — 히트는 그 캐시
//!   (프로필 복사 버튼과 같은 문법).

use crate::draw::{DrawCtx, FontSlot};
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::{t, Msg};

/// 바 높이(논리 px) — 툴바(48)의 1/3(사용자 확정).
pub const FILTER_H: i32 = 16;

/// 그룹 정의 — (설정 키, 칩들[(저장 값, 라벨)]).
const GROUPS: [(&str, &[(&str, Msg)]); 3] = [
    (
        "list.filter.path",
        &[
            ("all", Msg::FltAll),
            ("local", Msg::FltLocal),
            ("server", Msg::FltServer),
            ("internet", Msg::FltInternet),
            ("group", Msg::FltGroup),
        ],
    ),
    (
        "list.filter.presence",
        &[
            ("all", Msg::FltAll),
            ("online", Msg::FltOnline),
            ("offline", Msg::FltOffline),
        ],
    ),
    (
        "list.filter.trust",
        &[
            ("all", Msg::FltAll),
            ("verified", Msg::FltVerified),
            ("pinned", Msg::FltPinned),
            ("new", Msg::FltNew),
        ],
    ),
];

/// 필터 바 위젯.
#[derive(Debug, Default)]
pub struct FilterBarWidget {
    bounds: Rect,
    scale: f32,
    /// 그룹별 선택 인덱스.
    sel: [usize; 3],
    /// 페인트가 깐 칩 좌표 캐시 — (그룹, 칩, 사각형).
    chips: core::cell::RefCell<Vec<(usize, usize, Rect)>>,
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
                .1
                .iter()
                .position(|(v, _)| v == want)
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

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
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
                    inv.push(self.bounds);
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
                        let (key, chips) = GROUPS[gi];
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
        ctx.select_font(FontSlot::Status, false);
        let lh = ctx.text_height();
        let mut chips = self.chips.borrow_mut();
        chips.clear();
        let mut x = b.x + self.s(8);
        for (gi, (_, group)) in GROUPS.iter().enumerate() {
            if gi > 0 {
                // 그룹 구분 — 세로 1px 실선.
                let dx = x + self.s(5);
                ctx.fill_rect(
                    Rect::new(dx, b.y + self.s(3), 1, b.h - self.s(6) - 1),
                    theme.border,
                );
                x = dx + self.s(6);
            }
            for (ci, (_, label)) in group.iter().enumerate() {
                let text = t(*label);
                let tw = ctx.text_width(text);
                let chip = Rect::new(x, b.y + 1, tw + self.s(12), b.h - 3);
                let selected = self.sel[gi] == ci;
                let hovered = self.hover == Some((gi, ci));
                if selected {
                    ctx.fill_round_rect_alpha(chip, self.s(4), theme.accent, 0.18);
                } else if hovered {
                    ctx.fill_round_rect_alpha(chip, self.s(4), theme.text, 0.08);
                }
                let color = if selected {
                    theme.accent
                } else if hovered {
                    theme.text
                } else {
                    theme.text_dim
                };
                ctx.text(
                    chip.x + self.s(6),
                    chip.y + (chip.h - lh) / 2,
                    b,
                    text,
                    color,
                );
                chips.push((gi, ci, chip));
                x = chip.right() + self.s(4);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    /// 최소 목업 — 폭 = 문자수×8(칩 좌표 캐시를 깔기 위한 것뿐).
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
        w.set_bounds(Rect::new(0, 0, 800, 16), &mut inv);
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
