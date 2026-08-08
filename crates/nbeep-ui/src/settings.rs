//! 설정 화면 — **VS Code 방식** 최소 슬라이스(DR-24 · [docs/14 §10]).
//!
//! 핵심 발명은 **Entry 레지스트리 단일 원천**이다: 영속 설정 전부가 [`registry`]에 등록되고,
//! 렌더와 검색이 같은 원천을 읽는다 — "화면에 있는데 검색 안 되는 설정"이 구조적으로 불가능하다.
//! 제품에 실존하는 설정만 등록한다(없는 옵션 미등록 — 원본 규약).
//!
//! 레이아웃(VS Code식 — 사용자 확정 08-08): **좌측 사이드바**(상단 검색 + 카테고리 트리 ·
//! 선택 하이라이트 · 검색 중엔 **매치 있는 카테고리만 + 매치 수 "(N)"** — X-10 ①) + **우측
//! 편집기**(선택 카테고리의 항목 · 검색 중엔 전 카테고리 매치). **즉시 적용**(클릭/Enter = 값
//! 순환 — [`SettingsWidget::take_changes`] 폴링). **값 영속은 M2-5**(Repository 포트).

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use std::collections::HashMap;

/// 항목 종류 — 우측 패널이 이 열거를 읽어 동적 생성한다(새 설정 = Entry 1줄).
#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    /// 값 후보 중 택일 — (값, 표시 라벨) 목록. 클릭/Enter = 다음 값으로 순환.
    Radio(&'static [(&'static str, &'static str)]),
}

/// 설정 항목(레지스트리 최소 단위).
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// 카테고리(헤더 표시·검색 대상).
    pub cat: &'static str,
    /// 제목(검색 대상).
    pub label: &'static str,
    /// 회색 설명 한 줄(검색 대상).
    pub desc: &'static str,
    /// 컨트롤 형태.
    pub kind: SettingKind,
    /// 값 키(안정 계약 — rename 시 마이그레이션).
    pub key: &'static str,
}

/// 설정 레지스트리 — **실존 설정만**. 렌더·검색·기본값이 전부 여기서 나온다.
#[must_use]
pub fn registry() -> &'static [Entry] {
    &[
        Entry {
            cat: "대화",
            label: "대화 창 모드",
            desc: "새 대화를 여는 방식 — 변경은 다음 대화부터 적용됩니다(DR-26)",
            kind: SettingKind::Radio(&[
                ("single", "한 창에서 전환"),
                ("separate", "상대별 별도 창"),
            ]),
            key: "chat.window_mode",
        },
        Entry {
            cat: "모양",
            label: "테마",
            desc: "전체 창의 밝기 팔레트 — 즉시 적용됩니다",
            kind: SettingKind::Radio(&[("dark", "다크"), ("light", "라이트")]),
            key: "ui.theme",
        },
        Entry {
            cat: "글꼴",
            label: "기본 UI 크기",
            desc: "기본 UI 영역의 글자 크기",
            kind: SettingKind::Radio(&[
                ("m", "보통"),
                ("l", "크게"),
                ("xl", "아주 크게"),
                ("s", "작게"),
            ]),
            key: "font.base.size",
        },
        Entry {
            cat: "글꼴",
            label: "기본 UI 굵게",
            desc: "기본 UI 영역을 굵게",
            kind: SettingKind::Radio(&[("off", "보통"), ("on", "굵게")]),
            key: "font.base.bold",
        },
        Entry {
            cat: "글꼴",
            label: "기본 UI 기울임",
            desc: "기본 UI 영역을 기울임꼴로",
            kind: SettingKind::Radio(&[("off", "곧게"), ("on", "기울임")]),
            key: "font.base.italic",
        },
        Entry {
            cat: "글꼴",
            label: "대화 본문 크기",
            desc: "대화 본문 영역의 글자 크기",
            kind: SettingKind::Radio(&[
                ("m", "보통"),
                ("l", "크게"),
                ("xl", "아주 크게"),
                ("s", "작게"),
            ]),
            key: "font.message.size",
        },
        Entry {
            cat: "글꼴",
            label: "대화 본문 굵게",
            desc: "대화 본문 영역을 굵게",
            kind: SettingKind::Radio(&[("off", "보통"), ("on", "굵게")]),
            key: "font.message.bold",
        },
        Entry {
            cat: "글꼴",
            label: "대화 본문 기울임",
            desc: "대화 본문 영역을 기울임꼴로",
            kind: SettingKind::Radio(&[("off", "곧게"), ("on", "기울임")]),
            key: "font.message.italic",
        },
        Entry {
            cat: "글꼴",
            label: "상태바 크기",
            desc: "상태바 영역의 글자 크기",
            kind: SettingKind::Radio(&[
                ("m", "보통"),
                ("l", "크게"),
                ("xl", "아주 크게"),
                ("s", "작게"),
            ]),
            key: "font.status.size",
        },
        Entry {
            cat: "글꼴",
            label: "상태바 굵게",
            desc: "상태바 영역을 굵게",
            kind: SettingKind::Radio(&[("off", "보통"), ("on", "굵게")]),
            key: "font.status.bold",
        },
        Entry {
            cat: "글꼴",
            label: "상태바 기울임",
            desc: "상태바 영역을 기울임꼴로",
            kind: SettingKind::Radio(&[("off", "곧게"), ("on", "기울임")]),
            key: "font.status.italic",
        },
    ]
}

/// 설정 값 저장(런타임) — 영속은 M2-5의 `Repository` 포트로 감싼다.
#[derive(Debug, Default)]
pub struct SettingsState {
    values: HashMap<&'static str, String>,
}

impl SettingsState {
    /// 레지스트리 기본값(각 Radio의 첫 후보)으로 초기화.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut values = HashMap::new();
        for e in registry() {
            let SettingKind::Radio(opts) = e.kind;
            if let Some((v, _)) = opts.first() {
                values.insert(e.key, (*v).to_string());
            }
        }
        Self { values }
    }

    /// 현재 값(미설정 키는 빈 문자열).
    #[must_use]
    pub fn get(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }

    /// 값 지정.
    pub fn set(&mut self, key: &'static str, value: String) {
        self.values.insert(key, value);
    }
}

/// 검색어 → 소문자 토큰(공백 구분 **AND 매칭** — VS Code 규약).
fn tokens(q: &str) -> Vec<String> {
    q.split_whitespace().map(str::to_lowercase).collect()
}

fn entry_matches(e: &Entry, toks: &[String]) -> bool {
    if toks.is_empty() {
        return true;
    }
    let hay = format!("{} {} {}", e.cat, e.label, e.desc).to_lowercase();
    toks.iter().all(|t| hay.contains(t))
}

/// 행 높이(px·논리) — 항목은 2줄(제목+설명).
const ENTRY_H: i32 = 50;
const HEADER_H: i32 = 28;
const SEARCH_H: i32 = 34;
/// 좌측 사이드바 폭(논리 px) — 검색 + 카테고리 트리.
const SIDEBAR_W: i32 = 140;
/// 사이드바 트리 행 높이.
const TREE_ROW_H: i32 = 30;

/// 표시 행(레이아웃 결과).
enum RowKind {
    Header(&'static str),
    Item(usize), // registry 인덱스
}

/// 설정 위젯.
#[derive(Debug)]
pub struct SettingsWidget {
    bounds: Rect,
    scale: f32,
    query: String,
    caret: usize,
    changes: Vec<(&'static str, String)>,
    back: bool,
    /// 사이드바 선택 카테고리(빈 검색일 때 우측 필터).
    selected_cat: usize,
    /// 현재 값 스냅숏(표시용 — 적용 주체는 호스트).
    values: HashMap<&'static str, String>,
}

impl SettingsWidget {
    /// 현재 값 스냅숏으로 연다.
    #[must_use]
    pub fn new(state: &SettingsState) -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            query: String::new(),
            caret: 0,
            changes: Vec::new(),
            back: false,
            selected_cat: 0,
            values: registry()
                .iter()
                .map(|e| (e.key, state.get(e.key).to_string()))
                .collect(),
        }
    }

    /// 배율 지정(고DPI).
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let scale = scale.max(0.5);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            inv.push(self.bounds);
        }
    }

    /// 변경된 (키, 새 값) 목록을 꺼낸다(즉시 적용 — 호스트가 반영).
    pub fn take_changes(&mut self) -> Vec<(&'static str, String)> {
        std::mem::take(&mut self.changes)
    }

    /// Esc 닫기 요청(1회성).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    fn s(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    /// 카테고리 목록(레지스트리 순서·중복 제거) — 사이드바 트리의 원천.
    fn cats() -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for e in registry() {
            if !out.contains(&e.cat) {
                out.push(e.cat);
            }
        }
        out
    }

    /// 검색 중 카테고리별 매치 수(X-10 ① — 트리 행 "(N)" 표기·필터 공용).
    fn cat_match_count(cat: &str, toks: &[String]) -> usize {
        registry()
            .iter()
            .filter(|e| e.cat == cat && entry_matches(e, toks))
            .count()
    }

    /// 사이드바 표시 행 — 검색 중엔 **매치 있는 카테고리만**(X-10 ①), (카테고리, 매치 수).
    fn sidebar_rows(&self) -> Vec<(&'static str, usize)> {
        let toks = tokens(&self.query);
        Self::cats()
            .into_iter()
            .map(|c| (c, Self::cat_match_count(c, &toks)))
            .filter(|&(_, n)| n > 0)
            .collect()
    }

    /// 우측 표시 행 — 빈 검색 = **선택 카테고리만**, 검색 중 = 전 카테고리 매치(헤더 포함).
    fn rows(&self) -> Vec<RowKind> {
        let toks = tokens(&self.query);
        let searching = !toks.is_empty();
        let selected = Self::cats().get(self.selected_cat).copied();
        let mut out = Vec::new();
        let mut last_cat: Option<&str> = None;
        for (i, e) in registry().iter().enumerate() {
            if searching {
                if !entry_matches(e, &toks) {
                    continue;
                }
            } else if Some(e.cat) != selected {
                continue;
            }
            if searching && last_cat != Some(e.cat) {
                out.push(RowKind::Header(e.cat));
                last_cat = Some(e.cat);
            }
            out.push(RowKind::Item(i));
        }
        out
    }

    /// 필터된 항목(registry 인덱스)만.
    fn items(&self) -> Vec<usize> {
        self.rows()
            .into_iter()
            .filter_map(|r| match r {
                RowKind::Item(i) => Some(i),
                RowKind::Header(_) => None,
            })
            .collect()
    }

    /// 항목 값을 다음 후보로 순환한다(즉시 적용).
    fn cycle(&mut self, reg_idx: usize, inv: &mut Invalidations) {
        let e = &registry()[reg_idx];
        let SettingKind::Radio(opts) = e.kind;
        let current = self.values.get(e.key).map_or("", String::as_str);
        let pos = opts.iter().position(|(v, _)| *v == current).unwrap_or(0);
        let (next, _) = opts[(pos + 1) % opts.len()];
        self.values.insert(e.key, (*next).to_string());
        self.changes.push((e.key, (*next).to_string()));
        inv.push(self.bounds);
    }

    fn sidebar_w(&self) -> i32 {
        self.s(SIDEBAR_W)
    }

    /// (x, y) 물리 px → 사이드바 카테고리 행 인덱스.
    fn sidebar_at(&self, x: i32, y: i32) -> Option<usize> {
        if x >= self.bounds.x + self.sidebar_w() {
            return None;
        }
        let top = self.bounds.y + self.s(SEARCH_H);
        if y < top {
            return None;
        }
        let idx = ((y - top) / self.s(TREE_ROW_H).max(1)) as usize;
        (idx < self.sidebar_rows().len()).then_some(idx)
    }

    /// (x, y) 물리 px → 우측 항목의 registry 인덱스.
    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        if x < self.bounds.x + self.sidebar_w() {
            return None;
        }
        let mut top = self.bounds.y;
        for row in self.rows() {
            let h = match row {
                RowKind::Header(_) => self.s(HEADER_H),
                RowKind::Item(_) => self.s(ENTRY_H),
            };
            if y >= top && y < top + h {
                return match row {
                    RowKind::Item(i) => Some(i),
                    RowKind::Header(_) => None,
                };
            }
            top += h;
        }
        None
    }
}

impl Widget for SettingsWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::Key { key, .. } => match key {
                Key::Escape => self.back = true,
                Key::Up => {
                    self.caret = self.caret.saturating_sub(1);
                    inv.push(self.bounds);
                }
                Key::Down => {
                    let n = self.items().len();
                    if n > 0 {
                        self.caret = (self.caret + 1).min(n - 1);
                    }
                    inv.push(self.bounds);
                }
                Key::Enter | Key::Space => {
                    if let Some(&reg_idx) = self.items().get(self.caret) {
                        self.cycle(reg_idx, inv);
                    }
                }
                _ => {}
            },
            InputEvent::Char { c, .. } => {
                if c == '\u{8}' {
                    self.query.pop();
                } else if !c.is_control() {
                    self.query.push(c);
                }
                self.caret = 0;
                inv.push(self.bounds);
            }
            InputEvent::MouseDown { x, y, .. } => {
                if let Some(cat_idx) = self.sidebar_at(x, y) {
                    // 검색 중 사이드바 클릭 = 그 카테고리로 이동 + 검색 해제(문맥 확정).
                    let rows = self.sidebar_rows();
                    if let Some(&(cat, _)) = rows.get(cat_idx) {
                        if let Some(pos) = Self::cats().iter().position(|&c| c == cat) {
                            self.selected_cat = pos;
                        }
                        self.query.clear();
                        self.caret = 0;
                        inv.push(self.bounds);
                    }
                } else if let Some(reg_idx) = self.item_at(x, y) {
                    let items = self.items();
                    if let Some(pos) = items.iter().position(|&i| i == reg_idx) {
                        self.caret = pos;
                    }
                    self.cycle(reg_idx, inv);
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        let sw = self.sidebar_w();

        // ── 좌측 사이드바: 검색창 + 카테고리 트리(선택 하이라이트·매치 수) ──
        let sidebar = Rect::new(self.bounds.x, self.bounds.y, sw, self.bounds.h);
        ctx.fill_rect(sidebar, theme.chrome_bg);
        let search = Rect::new(self.bounds.x, self.bounds.y, sw, self.s(SEARCH_H));
        ctx.fill_rect(search, theme.field_bg);
        ctx.select_font(FontSlot::Status, false);
        let (q, qc) = if self.query.is_empty() {
            ("검색 (공백=AND)".to_string(), theme.text_dim)
        } else {
            (self.query.clone(), theme.text)
        };
        ctx.text(search.x + self.s(8), search.y + self.s(8), search, &q, qc);

        let searching = !self.query.is_empty();
        let selected = Self::cats().get(self.selected_cat).copied();
        let mut ty = self.bounds.y + self.s(SEARCH_H);
        for (cat, n) in self.sidebar_rows() {
            let r = Rect::new(self.bounds.x, ty, sw, self.s(TREE_ROW_H));
            let is_sel = !searching && Some(cat) == selected;
            if is_sel {
                ctx.fill_rect(r, theme.sel_bg);
            }
            ctx.select_font(FontSlot::Base, false);
            let label = if searching {
                format!("{cat} ({n})") // 매치 수(X-10 ①)
            } else {
                cat.to_string()
            };
            ctx.text(
                r.x + self.s(12),
                r.y + self.s(5),
                r,
                &label,
                if is_sel { theme.text } else { theme.text_dim },
            );
            ty += self.s(TREE_ROW_H);
        }
        // 사이드바 경계선.
        ctx.fill_rect(
            Rect::new(self.bounds.x + sw - 1, self.bounds.y, 1, self.bounds.h),
            theme.border,
        );

        // ── 우측 편집기 ──
        let items = self.items();
        let mut top = self.bounds.y;
        let mut item_pos = 0usize;
        for row in self.rows() {
            match row {
                RowKind::Header(cat) => {
                    let r = Rect::new(
                        self.bounds.x + sw,
                        top,
                        self.bounds.w - sw,
                        self.s(HEADER_H),
                    );
                    ctx.select_font(FontSlot::Status, false);
                    ctx.text_opaque(
                        r.x + self.s(10),
                        r.y + self.s(6),
                        r,
                        cat,
                        theme.text_dim,
                        theme.chrome_bg,
                    );
                    top += self.s(HEADER_H);
                }
                RowKind::Item(i) => {
                    let e = &registry()[i];
                    let r = Rect::new(self.bounds.x + sw, top, self.bounds.w - sw, self.s(ENTRY_H));
                    let bg = if items.get(self.caret) == Some(&i) {
                        theme.sel_bg
                    } else {
                        theme.panel_bg
                    };
                    ctx.fill_rect(r, bg);
                    ctx.select_font(FontSlot::Base, false);
                    ctx.text(r.x + self.s(12), r.y + self.s(5), r, e.label, theme.text);
                    ctx.select_font(FontSlot::Status, false);
                    ctx.text(
                        r.x + self.s(12),
                        r.y + self.s(24),
                        r,
                        e.desc,
                        theme.text_dim,
                    );
                    // 현재 값 칩(오른쪽) — 클릭/Enter = 다음 값.
                    let SettingKind::Radio(opts) = e.kind;
                    let current = self.values.get(e.key).map_or("", String::as_str);
                    let label = opts
                        .iter()
                        .find(|(v, _)| *v == current)
                        .map_or(current, |(_, l)| *l);
                    ctx.select_font(FontSlot::Base, false);
                    let bw = ctx.text_width(label) + self.s(16);
                    let chip =
                        Rect::new(r.right() - bw - self.s(12), r.y + self.s(8), bw, self.s(24));
                    ctx.fill_round_rect(chip, self.s(6), theme.accent);
                    ctx.text(
                        chip.x + self.s(8),
                        chip.y + self.s(4),
                        chip,
                        label,
                        theme.panel_bg,
                    );
                    top += self.s(ENTRY_H);
                    item_pos += 1;
                }
            }
        }
        let _ = item_pos;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (SettingsWidget, Invalidations) {
        let mut w = SettingsWidget::new(&SettingsState::with_defaults());
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 400, 400), &mut inv);
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
    fn registry_is_single_source_render_equals_search() {
        // 단일 원천 불변식(사이드바 형태): 사이드바 카테고리 매치 수의 합 == 레지스트리 전체
        // — 어떤 항목도 트리 밖에 있을 수 없다("검색/트리에 안 보이는 설정" 구조적 불가).
        let (w, _) = widget();
        let total: usize = w.sidebar_rows().iter().map(|&(_, n)| n).sum();
        assert_eq!(total, registry().len());
        // 그리고 각 카테고리를 선택하면 그 카테고리 항목이 전부 우측에 나온다.
        let mut shown = 0;
        let mut w2 = w;
        for i in 0..SettingsWidget::cats().len() {
            w2.selected_cat = i;
            shown += w2.items().len();
        }
        assert_eq!(shown, registry().len());
    }

    #[test]
    fn sidebar_shows_match_counts_and_filters_categories() {
        let (mut w, mut inv) = widget();
        for c in "테마".chars() {
            w.on_event(&InputEvent::Char { c, now_ms: 0 }, &mut inv);
        }
        let rows = w.sidebar_rows();
        assert_eq!(rows.len(), 1, "매치 있는 카테고리만(X-10 ①)");
        assert_eq!(rows[0], ("모양", 1), "매치 수 표기 근거");
    }

    #[test]
    fn sidebar_click_selects_category() {
        let (mut w, mut inv) = widget();
        // 사이드바 두 번째 행("모양") 클릭 — 검색창(30) 아래 두 번째 트리 행.
        w.on_event(
            &InputEvent::MouseDown {
                x: 10,
                y: SEARCH_H + TREE_ROW_H + 5, // 둘째 트리 행("모양")
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        let items = w.items();
        assert_eq!(items.len(), 1);
        assert_eq!(
            registry()[items[0]].key,
            "ui.theme",
            "선택 카테고리의 항목만"
        );
    }

    #[test]
    fn search_tokens_and_match() {
        let (mut w, mut inv) = widget();
        for c in "대화 창".chars() {
            w.on_event(&InputEvent::Char { c, now_ms: 0 }, &mut inv);
        }
        let items = w.items();
        assert_eq!(items.len(), 1, "AND 매칭 — '대화'+'창' 모두 포함 항목만");
        assert_eq!(registry()[items[0]].key, "chat.window_mode");
        // 미매치 토큰 추가 → 0건.
        for c in " zzz".chars() {
            w.on_event(&InputEvent::Char { c, now_ms: 0 }, &mut inv);
        }
        assert!(w.items().is_empty());
    }

    #[test]
    fn enter_cycles_value_and_reports_change() {
        let (mut w, mut inv) = widget();
        w.on_event(&key(Key::Enter), &mut inv); // 첫 항목 = chat.window_mode: single → separate
        let ch = w.take_changes();
        assert_eq!(ch, vec![("chat.window_mode", "separate".to_string())]);
        assert!(w.take_changes().is_empty(), "1회성 드레인");
        w.on_event(&key(Key::Enter), &mut inv); // separate → single(순환)
        assert_eq!(
            w.take_changes(),
            vec![("chat.window_mode", "single".to_string())]
        );
    }

    #[test]
    fn escape_requests_close() {
        let (mut w, mut inv) = widget();
        assert!(!w.take_back());
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(w.take_back());
    }

    #[test]
    fn defaults_come_from_registry_first_option() {
        let s = SettingsState::with_defaults();
        assert_eq!(
            s.get("chat.window_mode"),
            "single",
            "v1 기본 = 단일 창(DR-26)"
        );
        assert_eq!(s.get("ui.theme"), "dark");
    }
}
