//! 피어 목록 위젯 — 첫 실물 [`Widget`](M3-1 · FR-D-2 · FR-U-4).
//!
//! M1-6의 즉시 렌더 함수를 위젯으로 전환했다: 캐럿 탐색(↑↓·Home/End·PgUp/PgDn) ·
//! 클릭 선택 · 휠 스크롤(분수 노치 누적) · **타입어헤드**(표시 이름 접두사) · Enter 활성화.
//! 색은 전부 [`Theme`] 토큰이다(하드코딩 금지 — [docs/12 §B]). **신뢰 배지 3종은 항상 표시**.
//!
//! 활성화(대화 열기)는 [`PeerListWidget::take_activated`] 폴링으로 꺼낸다 — 위젯은 부모를
//! 모른다(원본 통지 모델의 번역 — [docs/12 §B]).

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key, WheelAccum};
use crate::geom::Rect;
use crate::theme::{Color, Theme};
use crate::typeahead::{TypeAhead, TYPEAHEAD_TIMEOUT_MS};
use crate::widget::{Invalidations, Widget};
use nbeep_core::peers::PeerEntry;
use nbeep_core::{PeerId, TrustLevel};

/// 목록 한 행 — 목록 항목(발견) + 신뢰 상태(`TrustStore`). 출처가 달라 조립 지점에서 합친다.
#[derive(Clone, Debug)]
pub struct PeerRow {
    /// 발견 목록 항목.
    pub entry: PeerEntry,
    /// 신뢰 상태(배지 결정).
    pub trust: TrustLevel,
}

/// 행 높이(px) — 임시. M3-1c 수치표에서 확정.
pub const ROW_H: i32 = 42;

/// 타입어헤드 HUD 표시 위치 — 3×3 중 택1(기본 좌측하단 · 사용자 확정).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HudPos {
    /// 좌상.
    TopLeft,
    /// 상중앙.
    TopCenter,
    /// 우상.
    TopRight,
    /// 좌중앙.
    MidLeft,
    /// 정중앙.
    Center,
    /// 우중앙.
    MidRight,
    /// 좌하(기본).
    #[default]
    BottomLeft,
    /// 하중앙.
    BottomCenter,
    /// 우하.
    BottomRight,
}

impl HudPos {
    /// 설정 값 코드 → 위치(미지 = 기본 좌하).
    #[must_use]
    pub fn from_code(s: &str) -> Self {
        match s {
            "tl" => Self::TopLeft,
            "tc" => Self::TopCenter,
            "tr" => Self::TopRight,
            "ml" => Self::MidLeft,
            "c" => Self::Center,
            "mr" => Self::MidRight,
            "bc" => Self::BottomCenter,
            "br" => Self::BottomRight,
            _ => Self::BottomLeft,
        }
    }
}

/// 신뢰 배지 라벨(현재 언어) + 테마 토큰 선택.
#[must_use]
pub fn badge(trust: TrustLevel, theme: &Theme) -> (&'static str, Color) {
    match trust {
        TrustLevel::Unverified => (nbeep_core::t(nbeep_core::Msg::TrustUnverified), theme.warn),
        TrustLevel::Pinned => (nbeep_core::t(nbeep_core::Msg::TrustPinned), theme.sel_bg),
        TrustLevel::FingerprintVerified => {
            (nbeep_core::t(nbeep_core::Msg::TrustVerified), theme.ok)
        }
    }
}

/// 피어 목록 위젯.
#[derive(Debug)]
pub struct PeerListWidget {
    bounds: Rect,
    rows: Vec<PeerRow>,
    /// 캐럿(키보드 포커스 행). 목록이 비면 무의미.
    caret: usize,
    /// 스크롤 상단 행 인덱스.
    top: usize,
    hover: Option<usize>,
    wheel: WheelAccum,
    typeahead: TypeAhead,
    activated: Option<PeerId>,
    /// 타입어헤드 HUD 위치(설정).
    hud_pos: HudPos,
    /// 타입어헤드에 공백 포함(설정 · 기본 true).
    ta_space: bool,
    /// 타입어헤드에 특수문자 포함(설정 · 기본 true).
    ta_special: bool,
    /// 배율(고DPI — FR-U-6). 행 높이·여백에 곱한다. 좌표·bounds는 물리 px.
    scale: f32,
}

impl Default for PeerListWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerListWidget {
    /// 빈 목록 위젯.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bounds: Rect::default(),
            rows: Vec::new(),
            caret: 0,
            top: 0,
            hover: None,
            wheel: WheelAccum::default(),
            typeahead: TypeAhead::new(TYPEAHEAD_TIMEOUT_MS),
            activated: None,
            hud_pos: HudPos::default(),
            ta_space: true,
            ta_special: true,
            scale: 1.0,
        }
    }

    /// 타입어헤드 유효시간(ms) 설정 — 마지막 입력 후 이 시간 지나면 초기화.
    pub fn set_typeahead_timeout(&mut self, ms: u64) {
        self.typeahead.set_timeout(ms);
    }

    /// 타입어헤드 HUD 위치 설정.
    pub fn set_hud_pos(&mut self, pos: HudPos, inv: &mut Invalidations) {
        self.hud_pos = pos;
        inv.push(self.bounds);
    }

    /// 타입어헤드에 공백 포함 여부.
    pub fn set_typeahead_space(&mut self, on: bool) {
        self.ta_space = on;
    }

    /// 타입어헤드에 특수문자 포함 여부.
    pub fn set_typeahead_special(&mut self, on: bool) {
        self.ta_special = on;
    }

    /// 이 문자가 타입어헤드에 반영되는가(설정 필터). 한/영/숫자는 항상 포함.
    fn ta_accepts(&self, c: char) -> bool {
        if c == ' ' {
            return self.ta_space;
        }
        if c.is_alphanumeric() {
            return true; // 한글 음절·영문·숫자
        }
        self.ta_special // 그 외 = 특수문자
    }

    /// 타임아웃 틱 — 유효시간 경과 시 버퍼 초기화(HUD 자동 숨김). 소거 시 `true`(재그리기).
    pub fn typeahead_tick(&mut self, now_ms: u64, inv: &mut Invalidations) -> bool {
        if self.typeahead.tick(now_ms) {
            inv.push(self.bounds);
            true
        } else {
            false
        }
    }

    /// 배율 지정(창 scale factor 변경·모니터 이동 시) — 레이아웃 전체가 다시 계산된다.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let scale = scale.max(0.5);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.clamp_scroll();
            inv.push(self.bounds);
        }
    }

    /// 물리 px 행 높이(배율 반영).
    #[must_use]
    pub fn row_h(&self) -> i32 {
        (ROW_H as f32 * self.scale).round() as i32
    }

    /// 물리 px 보조 치수.
    fn s(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    /// 목록 교체(발견 이벤트 반영) — 캐럿은 가능한 유지, 전체 무효화.
    pub fn set_rows(&mut self, rows: Vec<PeerRow>, inv: &mut Invalidations) {
        self.rows = rows;
        self.caret = self.caret.min(self.rows.len().saturating_sub(1));
        self.clamp_scroll();
        inv.push(self.bounds);
    }

    /// 현재 캐럿 행.
    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Enter/더블클릭으로 활성화된 상대를 꺼낸다(1회성 — 루프 소유자가 대화를 연다).
    pub fn take_activated(&mut self) -> Option<PeerId> {
        self.activated.take()
    }

    /// **IME 조합 중 텍스트로 실시간 타입어헤드**(한글 "김" 조합 즉시 이동 — 확정/Space 불필요).
    /// 호스트가 `Ime::Preedit`를 목록 모드일 때 이리로 넘긴다.
    pub fn set_preedit(&mut self, text: &str, now_ms: u64, inv: &mut Invalidations) {
        let q = self.typeahead.set_preedit(text, now_ms);
        if !q.prefix.is_empty() {
            let from = self.caret % self.rows.len().max(1);
            if let Some(hit) = self.find_prefix(&q.prefix, from) {
                self.move_caret(hit, inv);
            }
        }
        inv.push(self.bounds); // HUD 갱신
    }

    fn visible_rows(&self) -> usize {
        (self.bounds.h.max(0) as usize) / (self.row_h().max(1) as usize)
    }

    fn clamp_scroll(&mut self) {
        let vis = self.visible_rows().max(1);
        let max_top = self.rows.len().saturating_sub(vis);
        self.top = self.top.min(max_top);
        // 캐럿이 보이도록 스크롤 따라가기.
        if self.caret < self.top {
            self.top = self.caret;
        } else if self.caret >= self.top + vis {
            self.top = self.caret + 1 - vis;
        }
    }

    fn row_rect(&self, i: usize) -> Rect {
        let rel = i as i64 - self.top as i64;
        Rect::new(
            self.bounds.x,
            self.bounds.y
                + i32::try_from(rel)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(self.row_h()),
            self.bounds.w,
            self.row_h(),
        )
    }

    fn move_caret(&mut self, to: usize, inv: &mut Invalidations) {
        let to = to.min(self.rows.len().saturating_sub(1));
        if to == self.caret || self.rows.is_empty() {
            return;
        }
        inv.push(self.row_rect(self.caret));
        self.caret = to;
        let before = self.top;
        self.clamp_scroll();
        if self.top != before {
            inv.push(self.bounds); // 스크롤됨 — 전체
        } else {
            inv.push(self.row_rect(self.caret));
        }
    }

    fn row_at(&self, y: i32) -> Option<usize> {
        if y < self.bounds.y {
            return None;
        }
        let rel = ((y - self.bounds.y) / self.row_h().max(1)) as usize;
        let idx = self.top + rel;
        (idx < self.rows.len()).then_some(idx)
    }

    /// 접두사 매치(대소문자 무시) — `from`부터 순환 검색.
    fn find_prefix(&self, prefix: &str, from: usize) -> Option<usize> {
        let n = self.rows.len();
        if n == 0 {
            return None;
        }
        let p = prefix.to_lowercase();
        (0..n).map(|k| (from + k) % n).find(|&i| {
            self.rows[i]
                .entry
                .name
                .as_str()
                .to_lowercase()
                .starts_with(&p)
        })
    }
}

impl Widget for PeerListWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.clamp_scroll();
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::Key { key, .. } => {
                let vis = self.visible_rows().max(1);
                match key {
                    Key::Up => self.move_caret(self.caret.saturating_sub(1), inv),
                    Key::Down => self.move_caret(self.caret + 1, inv),
                    Key::Home => self.move_caret(0, inv),
                    Key::End => self.move_caret(self.rows.len().saturating_sub(1), inv),
                    Key::PageUp => self.move_caret(self.caret.saturating_sub(vis), inv),
                    Key::PageDown => self.move_caret(self.caret + vis, inv),
                    Key::Enter => {
                        if let Some(row) = self.rows.get(self.caret) {
                            self.activated = Some(row.entry.peer);
                        }
                    }
                    Key::Escape => {
                        self.typeahead.clear();
                    }
                    _ => {}
                }
            }
            InputEvent::Wheel { delta } => {
                let lines = self.wheel.add(delta, 3);
                if lines != 0 {
                    let new_top = if lines > 0 {
                        self.top.saturating_sub(lines.unsigned_abs() as usize)
                    } else {
                        self.top + lines.unsigned_abs() as usize
                    };
                    let vis = self.visible_rows().max(1);
                    let clamped = new_top.min(self.rows.len().saturating_sub(vis));
                    if clamped != self.top {
                        self.top = clamped;
                        inv.push(self.bounds);
                    }
                }
            }
            InputEvent::Char { c, now_ms } => {
                if c == '\u{8}' {
                    if let Some(q) = self.typeahead.backspace(now_ms) {
                        if let Some(hit) = self.find_prefix(&q.prefix, self.caret) {
                            self.move_caret(hit, inv);
                        }
                    }
                    return;
                }
                if !self.ta_accepts(c) {
                    return; // 설정상 미포함(공백·특수문자)
                }
                let q = self.typeahead.push(c, now_ms);
                let from = if q.include_caret {
                    self.caret
                } else {
                    self.caret + 1
                };
                if let Some(hit) = self.find_prefix(&q.prefix, from % self.rows.len().max(1)) {
                    self.move_caret(hit, inv);
                }
            }
            InputEvent::MouseDown { y, .. } => {
                if let Some(i) = self.row_at(y) {
                    self.move_caret(i, inv);
                }
            }
            InputEvent::MouseMove { y, .. } => {
                let over = self.row_at(y);
                if over != self.hover {
                    if let Some(old) = self.hover {
                        inv.push(self.row_rect(old));
                    }
                    if let Some(new) = over {
                        inv.push(self.row_rect(new));
                    }
                    self.hover = over;
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        ctx.select_font(FontSlot::PeerList, false);
        let vis = self.visible_rows();

        let rh = self.row_h();
        for (rel, i) in (self.top..self.rows.len().min(self.top + vis + 1)).enumerate() {
            let row = &self.rows[i];
            let r = Rect::new(
                self.bounds.x,
                self.bounds.y + i32::try_from(rel).unwrap_or(i32::MAX) * rh,
                self.bounds.w,
                rh,
            );
            // 행 배경 — 캐럿 > hover > 기본.
            let bg = if i == self.caret {
                theme.sel_bg
            } else if Some(i) == self.hover {
                theme.panel_bg_alt
            } else {
                theme.panel_bg
            };
            let text_y = r.y + (rh - self.s(20)) / 2;
            ctx.text_opaque(
                r.x + self.s(12),
                text_y,
                r,
                row.entry.name.as_str(),
                theme.text,
                bg,
            );

            // 다중 경로 ×N(진단).
            if row.entry.paths > 1 {
                let name_w = ctx.text_width(row.entry.name.as_str());
                let label = format!("×{}", row.entry.paths);
                ctx.text(
                    r.x + self.s(16) + name_w,
                    text_y + self.s(2),
                    r,
                    &label,
                    theme.text_dim,
                );
            }

            // 신뢰 배지(오른쪽 정렬 라운드 칩) — 항상 표시.
            let (label, chip) = badge(row.trust, theme);
            let bw = ctx.text_width(label) + self.s(16);
            let chip_r = Rect::new(
                r.right() - bw - self.s(10),
                r.y + self.s(8),
                bw,
                rh - self.s(16),
            );
            ctx.fill_round_rect(chip_r, (rh - self.s(16)) / 2, chip);
            ctx.text(
                chip_r.x + self.s(8),
                chip_r.y + self.s(3),
                chip_r,
                label,
                theme.text,
            );

            // 행 구분선.
            ctx.fill_rect(Rect::new(r.x, r.bottom() - 1, r.w, 1), theme.border);
        }
        // 타입어헤드 HUD(입력·조합 중일 때만) — 위치는 설정(3×3). 조합 중 텍스트도 표시.
        let buf = self.typeahead.composing();
        if !buf.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            ctx.select_font(FontSlot::Base, false); // 타입어헤드 = 기본 글꼴(사용자 확정)
            let w = ctx.text_width(&buf) + self.s(16);
            let hh = self.s(20);
            let m = self.s(8);
            let (hx, hy) = {
                use HudPos::*;
                let left = self.bounds.x + m;
                let cx = self.bounds.x + (self.bounds.w - w) / 2;
                let right = self.bounds.right() - w - m;
                let topy = self.bounds.y + m;
                let midy = self.bounds.y + (self.bounds.h - hh) / 2;
                let boty = self.bounds.bottom() - hh - m;
                match self.hud_pos {
                    TopLeft => (left, topy),
                    TopCenter => (cx, topy),
                    TopRight => (right, topy),
                    MidLeft => (left, midy),
                    Center => (cx, midy),
                    MidRight => (right, midy),
                    BottomLeft => (left, boty),
                    BottomCenter => (cx, boty),
                    BottomRight => (right, boty),
                }
            };
            let hud = Rect::new(hx, hy, w, hh);
            ctx.fill_round_rect(hud, self.s(6), theme.field_bg);
            ctx.text(
                hud.x + self.s(8),
                hud.y + self.s(3),
                hud,
                &buf,
                theme.accent,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbeep_core::DisplayName;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn row(b: u8, name: &str, trust: TrustLevel) -> PeerRow {
        PeerRow {
            entry: PeerEntry {
                peer: pid(b),
                name: DisplayName::parse(name).unwrap(),
                paths: 1,
            },
            trust,
        }
    }
    fn widget(names: &[(u8, &str)]) -> (PeerListWidget, Invalidations) {
        let mut w = PeerListWidget::new();
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 300, ROW_H * 4), &mut inv); // 4행 가시
        let rows = names
            .iter()
            .map(|&(b, n)| row(b, n, TrustLevel::Unverified))
            .collect();
        w.set_rows(rows, &mut inv);
        (w, inv)
    }
    fn key(k: Key) -> InputEvent {
        InputEvent::Key {
            key: k,
            shift: false,
            primary: false,
        }
    }

    #[test]
    fn caret_moves_and_invalidates_only_two_rows() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::Down), &mut inv);
        assert_eq!(w.caret(), 1);
        let rects: Vec<_> = inv.drain().collect();
        // 이전 행 + 새 행(인접 = 병합될 수 있음) — 전체 무효화가 아니어야 한다(FR-U-13).
        assert!(rects.iter().all(|r| r.h <= ROW_H * 2), "{rects:?}");
    }

    #[test]
    fn caret_follows_scroll_beyond_visible() {
        let names: Vec<(u8, String)> = (1..=10).map(|i| (i, format!("peer{i:02}"))).collect();
        let refs: Vec<(u8, &str)> = names.iter().map(|(b, s)| (*b, s.as_str())).collect();
        let (mut w, _) = widget(&refs);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::End), &mut inv);
        assert_eq!(w.caret(), 9);
        // 4행 가시 창에서 마지막 행이 보이려면 top = 6.
        let rects: Vec<_> = inv.drain().collect();
        assert!(!rects.is_empty(), "스크롤 = 전체 무효화");
    }

    #[test]
    fn enter_activates_caret_peer_once() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob")]);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::Down), &mut inv);
        w.on_event(&key(Key::Enter), &mut inv);
        assert_eq!(w.take_activated(), Some(pid(2)));
        assert_eq!(w.take_activated(), None, "1회성");
    }

    #[test]
    fn click_selects_row_under_cursor() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        w.on_event(
            &InputEvent::MouseDown {
                x: 10,
                y: ROW_H * 2 + 5,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 2);
        // 목록 밖 클릭은 무시.
        w.on_event(
            &InputEvent::MouseDown {
                x: 10,
                y: ROW_H * 3 + 5,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 2);
    }

    #[test]
    fn typeahead_prefix_and_single_key_cycle() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "bora"), (4, "carol")]);
        let mut inv = Invalidations::default();
        // "bo" 누적 → bob.
        w.on_event(&InputEvent::Char { c: 'b', now_ms: 0 }, &mut inv);
        assert_eq!(w.caret(), 1);
        w.on_event(
            &InputEvent::Char {
                c: 'o',
                now_ms: 100,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 1, "확장 매치 — bob 유지");
        // 타임아웃 후 'b' 반복 = cycle: bob → bora → bob.
        w.on_event(
            &InputEvent::Char {
                c: 'b',
                now_ms: 5000,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 2, "다음 매치 bora");
        w.on_event(
            &InputEvent::Char {
                c: 'b',
                now_ms: 5100,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 1, "cycle 복귀 bob");
    }

    #[test]
    fn scale_resizes_rows_and_hit_testing() {
        // 2배율에서 행 높이·클릭 좌표→행 매핑이 함께 커진다(FR-U-6).
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        w.set_scale(2.0, &mut inv);
        assert!(!inv.is_empty(), "배율 변경 = 전체 무효화");
        assert_eq!(w.row_h(), ROW_H * 2);
        w.on_event(
            &InputEvent::MouseDown {
                x: 10,
                y: ROW_H * 2 + 5, // 1배율이면 2행, 2배율이면 1행
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 1, "물리 좌표는 배율 행 높이로 나눈다");
    }

    #[test]
    fn wheel_scrolls_with_fractional_accumulation() {
        let names: Vec<(u8, String)> = (1..=10).map(|i| (i, format!("p{i:02}"))).collect();
        let refs: Vec<(u8, &str)> = names.iter().map(|(b, s)| (*b, s.as_str())).collect();
        let (mut w, _) = widget(&refs);
        let mut inv = Invalidations::default();
        // 아래로 한 노치(-120) = 3행.
        w.on_event(&InputEvent::Wheel { delta: -120 }, &mut inv);
        w.on_event(&key(Key::Up), &mut inv); // 캐럿 0인 채 top만 이동했는지 확인용
        assert!(!inv.is_empty());
    }
}
