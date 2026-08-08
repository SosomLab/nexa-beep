//! 설정 화면 — **VS Code 방식** 최소 슬라이스(DR-24 · [docs/14 §10]).
//!
//! 핵심 발명은 **Entry 레지스트리 단일 원천**이다: 영속 설정 전부가 [`registry`]에 등록되고,
//! 렌더와 검색이 같은 원천을 읽는다 — "화면에 있는데 검색 안 되는 설정"이 구조적으로 불가능하다.
//! 제품에 실존하는 설정만 등록한다(없는 옵션 미등록 — 원본 규약).
//!
//! 레이아웃(VS Code식): **좌측 사이드바**(검색 + 카테고리 트리 · 검색 중엔 매치 카테고리만 +
//! 매치 수 "(N)") + **우측 편집기**. **즉시 적용**([`SettingsWidget::take_changes`] 폴링).
//!
//! ## i18n(사용자 요청 08-08)
//!
//! 표시 문자열은 전부 [`Msg`] 키로 두고 렌더 시 **현재 언어**([`nbeep_core::current_lang`])로
//! 번역한다(영어 기본 · 한/중/일 팩). 검색은 **전 언어 매치** — 어느 언어로 쳐도 찾힌다.
//! 글꼴 영역 섹션([`SettingKind::FontSection`])은 '제목 + 글꼴명(텍스트박스) + 크기(콤보) + 설명'.

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::{current_lang, tr, Lang, Msg};
use std::collections::HashMap;

/// 크기 콤보 후보(전 글꼴 영역 공용) — (값, 라벨 Msg). 첫 값이 기본(보통).
const SIZE_OPTS: &[(&str, Msg)] = &[
    ("m", Msg::SizeNormal),
    ("l", Msg::SizeLarge),
    ("xl", Msg::SizeExtraLarge),
    ("s", Msg::SizeSmall),
];

/// 항목 종류 — 우측 패널이 이 열거를 읽어 동적 생성한다(새 설정 = Entry 1줄).
#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    /// 값 후보 중 택일 — (값, 라벨 Msg). 클릭/Enter = 다음 값으로 순환.
    Radio(&'static [(&'static str, Msg)]),
    /// 글꼴 영역 — **글꼴명(텍스트박스) + 크기(콤보)** 를 한 섹션으로.
    FontSection {
        /// 글꼴명 값 키(`font.{region}.family`).
        family_key: &'static str,
        /// 크기 값 키(`font.{region}.size`).
        size_key: &'static str,
    },
}

/// 설정 항목(레지스트리 최소 단위).
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// 카테고리(헤더 표시·검색 대상).
    pub cat: Msg,
    /// 제목(검색 대상 · 글꼴 섹션에선 섹션 제목).
    pub label: Msg,
    /// 회색 설명 한 줄(검색 대상).
    pub desc: Msg,
    /// 컨트롤 형태.
    pub kind: SettingKind,
    /// 값 키(안정 계약 — rename 시 마이그레이션). 글꼴 섹션에선 `family_key`와 동일.
    pub key: &'static str,
}

impl Entry {
    /// 이 항목이 소유한 값 키 전부(FontSection은 글꼴명 + 크기 2개).
    fn value_keys(&self) -> Vec<&'static str> {
        match self.kind {
            SettingKind::Radio(_) => vec![self.key],
            SettingKind::FontSection {
                family_key,
                size_key,
            } => vec![family_key, size_key],
        }
    }

    /// 레지스트리 기본값(각 값 키 → 기본 문자열).
    fn default_values(&self) -> Vec<(&'static str, String)> {
        match self.kind {
            SettingKind::Radio(opts) => opts
                .first()
                .map(|(v, _)| (self.key, (*v).to_string()))
                .into_iter()
                .collect(),
            SettingKind::FontSection {
                family_key,
                size_key,
            } => vec![
                (family_key, String::new()), // 빈 문자열 = 시스템 기본 글꼴
                (size_key, SIZE_OPTS[0].0.to_string()),
            ],
        }
    }
}

/// 설정 레지스트리 — **실존 설정만**. 렌더·검색·기본값이 전부 여기서 나온다.
#[must_use]
pub fn registry() -> &'static [Entry] {
    &[
        Entry {
            cat: Msg::CatConversation,
            label: Msg::ChatWindowMode,
            desc: Msg::ChatWindowModeDesc,
            kind: SettingKind::Radio(&[
                ("single", Msg::WindowModeSingle),
                ("separate", Msg::WindowModeSeparate),
            ]),
            key: "chat.window_mode",
        },
        Entry {
            cat: Msg::CatAppearance,
            label: Msg::Theme,
            desc: Msg::ThemeDesc,
            kind: SettingKind::Radio(&[("dark", Msg::ThemeDark), ("light", Msg::ThemeLight)]),
            key: "ui.theme",
        },
        Entry {
            cat: Msg::CatAppearance,
            label: Msg::Language,
            desc: Msg::LanguageDesc,
            kind: SettingKind::Radio(&[
                ("en", Msg::LangEnglish),
                ("ko", Msg::LangKorean),
                ("zh", Msg::LangChinese),
                ("ja", Msg::LangJapanese),
            ]),
            key: "ui.language",
        },
        Entry {
            cat: Msg::CatFont,
            label: Msg::FontBase,
            desc: Msg::FontBaseDesc,
            kind: SettingKind::FontSection {
                family_key: "font.base.family",
                size_key: "font.base.size",
            },
            key: "font.base.family",
        },
        Entry {
            cat: Msg::CatFont,
            label: Msg::FontPeerList,
            desc: Msg::FontPeerListDesc,
            kind: SettingKind::FontSection {
                family_key: "font.peerlist.family",
                size_key: "font.peerlist.size",
            },
            key: "font.peerlist.family",
        },
        Entry {
            cat: Msg::CatFont,
            label: Msg::FontMessage,
            desc: Msg::FontMessageDesc,
            kind: SettingKind::FontSection {
                family_key: "font.message.family",
                size_key: "font.message.size",
            },
            key: "font.message.family",
        },
        Entry {
            cat: Msg::CatFont,
            label: Msg::FontStatus,
            desc: Msg::FontStatusDesc,
            kind: SettingKind::FontSection {
                family_key: "font.status.family",
                size_key: "font.status.size",
            },
            key: "font.status.family",
        },
    ]
}

/// 설정 값 저장(런타임) — 영속은 M2-5의 `Repository` 포트로 감싼다.
#[derive(Debug, Default)]
pub struct SettingsState {
    values: HashMap<&'static str, String>,
}

impl SettingsState {
    /// 레지스트리 기본값으로 초기화.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut values = HashMap::new();
        for e in registry() {
            for (k, v) in e.default_values() {
                values.insert(k, v);
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

/// 전 언어에 걸쳐 매칭한다 — 영어 UI에서도 "테마"로, 한국어 UI에서도 "theme"로 찾힌다.
fn entry_matches(e: &Entry, toks: &[String]) -> bool {
    if toks.is_empty() {
        return true;
    }
    let mut hay = String::new();
    for lang in Lang::ALL {
        hay.push_str(tr(lang, e.cat));
        hay.push(' ');
        hay.push_str(tr(lang, e.label));
        hay.push(' ');
        hay.push_str(tr(lang, e.desc));
        hay.push(' ');
    }
    let hay = hay.to_lowercase();
    toks.iter().all(|t| hay.contains(t))
}

/// 크기 값 → 라벨 Msg.
fn size_msg(value: &str) -> Msg {
    SIZE_OPTS
        .iter()
        .find(|(v, _)| *v == value)
        .map_or(SIZE_OPTS[0].1, |(_, m)| *m)
}

/// 행 높이(px·논리) — Radio 항목은 2줄(제목+설명).
const ENTRY_H: i32 = 50;
/// 글꼴 섹션 높이 — 제목 + (글꼴명·크기) 컨트롤 행 + 설명.
const FONT_SECTION_H: i32 = 88;
const HEADER_H: i32 = 28;
const SEARCH_H: i32 = 34;
/// 좌측 사이드바 폭(논리 px) — 검색 + 카테고리 트리.
const SIDEBAR_W: i32 = 150;
/// 사이드바 트리 행 높이.
const TREE_ROW_H: i32 = 30;

// 글꼴 섹션 내부 컨트롤 레이아웃(논리 px).
const CTRL_DY: i32 = 32; // 섹션 top → 컨트롤 행 y
const CTRL_H: i32 = 28; // 컨트롤 높이
const FAMILY_W: i32 = 180; // 글꼴명 텍스트박스 폭
const SIZE_W: i32 = 112; // 크기 콤보 폭
const CTRL_GAP: i32 = 10; // 글꼴명 ↔ 크기 간격

/// 표시 행(레이아웃 결과).
#[derive(Clone, Copy)]
enum RowKind {
    Header(Msg),
    Item(usize), // registry 인덱스
}

/// 입력 포커스 — 검색창(기본) 또는 특정 글꼴 섹션의 글꼴명 박스.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Search,
    Family(usize), // registry 인덱스
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
    /// 입력 포커스(검색창 vs 글꼴명 박스).
    focus: Focus,
    /// 현재 값 스냅숏(표시용 — 적용 주체는 호스트).
    values: HashMap<&'static str, String>,
}

impl SettingsWidget {
    /// 현재 값 스냅숏으로 연다.
    #[must_use]
    pub fn new(state: &SettingsState) -> Self {
        let mut values = HashMap::new();
        for e in registry() {
            for k in e.value_keys() {
                values.insert(k, state.get(k).to_string());
            }
        }
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            query: String::new(),
            caret: 0,
            changes: Vec::new(),
            back: false,
            selected_cat: 0,
            focus: Focus::Search,
            values,
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

    fn lang(&self) -> Lang {
        current_lang()
    }

    /// 카테고리 목록(레지스트리 순서·중복 제거) — 사이드바 트리의 원천.
    fn cats() -> Vec<Msg> {
        let mut out: Vec<Msg> = Vec::new();
        for e in registry() {
            if !out.contains(&e.cat) {
                out.push(e.cat);
            }
        }
        out
    }

    /// 검색 중 카테고리별 매치 수(트리 행 "(N)" 표기·필터 공용).
    fn cat_match_count(cat: Msg, toks: &[String]) -> usize {
        registry()
            .iter()
            .filter(|e| e.cat == cat && entry_matches(e, toks))
            .count()
    }

    /// 사이드바 표시 행 — 검색 중엔 **매치 있는 카테고리만**, (카테고리, 매치 수).
    fn sidebar_rows(&self) -> Vec<(Msg, usize)> {
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
        let mut last_cat: Option<Msg> = None;
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

    fn row_h(&self, row: &RowKind) -> i32 {
        match row {
            RowKind::Header(_) => self.s(HEADER_H),
            RowKind::Item(i) => match registry()[*i].kind {
                SettingKind::Radio(_) => self.s(ENTRY_H),
                SettingKind::FontSection { .. } => self.s(FONT_SECTION_H),
            },
        }
    }

    /// 표시 행 + 각 행의 top(물리 px) — 렌더·히트테스트 공용(기하 단일 원천).
    fn rows_with_top(&self) -> Vec<(RowKind, i32)> {
        let mut top = self.bounds.y;
        let mut out = Vec::new();
        for row in self.rows() {
            let h = self.row_h(&row);
            out.push((row, top));
            top += h;
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

    /// 값을 후보 목록 안에서 다음으로 순환한다(즉시 적용).
    fn cycle_key(
        &mut self,
        key: &'static str,
        opts: &[(&'static str, Msg)],
        inv: &mut Invalidations,
    ) {
        if opts.is_empty() {
            return;
        }
        let current = self.values.get(key).map_or("", String::as_str);
        let pos = opts.iter().position(|(v, _)| *v == current).unwrap_or(0);
        let (next, _) = opts[(pos + 1) % opts.len()];
        self.values.insert(key, (*next).to_string());
        self.changes.push((key, (*next).to_string()));
        inv.push(self.bounds);
    }

    /// 캐럿 위치 항목을 활성화(Radio = 값 순환 · FontSection = 크기 순환).
    fn activate(&mut self, reg_idx: usize, inv: &mut Invalidations) {
        match registry()[reg_idx].kind {
            SettingKind::Radio(opts) => {
                let key = registry()[reg_idx].key;
                self.cycle_key(key, opts, inv);
            }
            SettingKind::FontSection { size_key, .. } => {
                self.cycle_key(size_key, SIZE_OPTS, inv);
            }
        }
    }

    /// 글꼴명 박스에 문자 편집(끝에 삽입 / 백스페이스). 즉시 적용.
    fn edit_family(&mut self, key: &'static str, c: char, inv: &mut Invalidations) {
        let cur = self.values.entry(key).or_default();
        if c == '\u{8}' {
            cur.pop();
        } else if !c.is_control() {
            cur.push(c);
        }
        let v = cur.clone();
        self.changes.push((key, v));
        inv.push(self.bounds);
    }

    fn sidebar_w(&self) -> i32 {
        self.s(SIDEBAR_W)
    }

    /// 글꼴 섹션의 (글꼴명 박스, 크기 콤보) 물리 rect — `top`은 섹션의 top(물리 px).
    fn section_ctrls(&self, top: i32) -> (Rect, Rect) {
        let x0 = self.bounds.x + self.sidebar_w() + self.s(12);
        let y = top + self.s(CTRL_DY);
        let h = self.s(CTRL_H);
        let fam = Rect::new(x0, y, self.s(FAMILY_W), h);
        let size = Rect::new(fam.right() + self.s(CTRL_GAP), y, self.s(SIZE_W), h);
        (fam, size)
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
                Key::Escape => {
                    if matches!(self.focus, Focus::Family(_)) {
                        self.focus = Focus::Search;
                        inv.push(self.bounds);
                    } else {
                        self.back = true;
                    }
                }
                Key::Up => {
                    self.focus = Focus::Search;
                    self.caret = self.caret.saturating_sub(1);
                    inv.push(self.bounds);
                }
                Key::Down => {
                    self.focus = Focus::Search;
                    let n = self.items().len();
                    if n > 0 {
                        self.caret = (self.caret + 1).min(n - 1);
                    }
                    inv.push(self.bounds);
                }
                Key::Enter | Key::Space => {
                    if let Some(&reg_idx) = self.items().get(self.caret) {
                        self.activate(reg_idx, inv);
                    }
                }
                _ => {}
            },
            InputEvent::Char { c, .. } => {
                if let Focus::Family(reg_idx) = self.focus {
                    if let SettingKind::FontSection { family_key, .. } = registry()[reg_idx].kind {
                        self.edit_family(family_key, c, inv);
                    }
                } else {
                    if c == '\u{8}' {
                        self.query.pop();
                    } else if !c.is_control() {
                        self.query.push(c);
                    }
                    self.caret = 0;
                    inv.push(self.bounds);
                }
            }
            InputEvent::MouseDown { x, y, .. } => {
                if let Some(cat_idx) = self.sidebar_at(x, y) {
                    let rows = self.sidebar_rows();
                    if let Some(&(cat, _)) = rows.get(cat_idx) {
                        if let Some(pos) = Self::cats().iter().position(|&c| c == cat) {
                            self.selected_cat = pos;
                        }
                        self.query.clear();
                        self.caret = 0;
                        self.focus = Focus::Search;
                        inv.push(self.bounds);
                    }
                    return;
                }
                self.focus = Focus::Search; // 기본: 블러(글꼴명 클릭 시 아래서 재설정)
                let p = Point { x, y };
                for (row, top) in self.rows_with_top() {
                    let RowKind::Item(i) = row else { continue };
                    let h = self.row_h(&row);
                    if y < top || y >= top + h {
                        continue;
                    }
                    if let Some(pos) = self.items().iter().position(|&it| it == i) {
                        self.caret = pos;
                    }
                    match registry()[i].kind {
                        SettingKind::Radio(opts) => {
                            let key = registry()[i].key;
                            self.cycle_key(key, opts, inv);
                        }
                        SettingKind::FontSection { size_key, .. } => {
                            let (fam, size) = self.section_ctrls(top);
                            if fam.contains(p) {
                                self.focus = Focus::Family(i);
                                inv.push(self.bounds);
                            } else if size.contains(p) {
                                self.cycle_key(size_key, SIZE_OPTS, inv);
                            } else {
                                inv.push(self.bounds);
                            }
                        }
                    }
                    break;
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let lang = self.lang();
        ctx.fill_rect(self.bounds, theme.panel_bg);
        let sw = self.sidebar_w();

        // ── 좌측 사이드바: 검색창 + 카테고리 트리 ──
        let sidebar = Rect::new(self.bounds.x, self.bounds.y, sw, self.bounds.h);
        ctx.fill_rect(sidebar, theme.chrome_bg);
        let search = Rect::new(self.bounds.x, self.bounds.y, sw, self.s(SEARCH_H));
        ctx.fill_rect(search, theme.field_bg);
        ctx.select_font(FontSlot::Status, false);
        let (q, qc) = if self.query.is_empty() {
            (tr(lang, Msg::SearchPlaceholder).to_string(), theme.text_dim)
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
                format!("{} ({n})", tr(lang, cat))
            } else {
                tr(lang, cat).to_string()
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
        ctx.fill_rect(
            Rect::new(self.bounds.x + sw - 1, self.bounds.y, 1, self.bounds.h),
            theme.border,
        );

        // ── 우측 편집기 ──
        let items = self.items();
        for (row, top) in self.rows_with_top() {
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
                        tr(lang, cat),
                        theme.text_dim,
                        theme.chrome_bg,
                    );
                }
                RowKind::Item(i) => {
                    let e = &registry()[i];
                    let h = self.row_h(&row);
                    let r = Rect::new(self.bounds.x + sw, top, self.bounds.w - sw, h);
                    let is_caret = items.get(self.caret) == Some(&i);
                    match e.kind {
                        SettingKind::Radio(opts) => {
                            self.paint_radio(ctx, theme, lang, e, opts, r, is_caret);
                        }
                        SettingKind::FontSection {
                            family_key,
                            size_key,
                        } => {
                            self.paint_font_section(
                                ctx, theme, lang, e, family_key, size_key, r, top, i,
                            );
                        }
                    }
                }
            }
        }
    }
}

impl SettingsWidget {
    /// Radio 항목 — 제목 + 설명 + 우측 값 콤보(클릭/Enter = 다음 값).
    #[allow(clippy::too_many_arguments)]
    fn paint_radio(
        &self,
        ctx: &mut dyn DrawCtx,
        theme: &Theme,
        lang: Lang,
        e: &Entry,
        opts: &[(&'static str, Msg)],
        r: Rect,
        is_caret: bool,
    ) {
        let bg = if is_caret {
            theme.sel_bg
        } else {
            theme.panel_bg
        };
        ctx.fill_rect(r, bg);
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            r.x + self.s(12),
            r.y + self.s(5),
            r,
            tr(lang, e.label),
            theme.text,
        );
        ctx.select_font(FontSlot::Status, false);
        ctx.text(
            r.x + self.s(12),
            r.y + self.s(24),
            r,
            tr(lang, e.desc),
            theme.text_dim,
        );

        let current = self.values.get(e.key).map_or("", String::as_str);
        let msg = opts
            .iter()
            .find(|(v, _)| *v == current)
            .map_or(SIZE_OPTS[0].1, |(_, m)| *m);
        let combo = format!("{} ▾", tr(lang, msg));
        ctx.select_font(FontSlot::Base, false);
        let bw = ctx.text_width(&combo) + self.s(18);
        let chip = Rect::new(r.right() - bw - self.s(12), r.y + self.s(8), bw, self.s(26));
        ctx.fill_round_rect(chip, self.s(6), theme.accent);
        ctx.text(
            chip.x + self.s(9),
            chip.y + self.s(5),
            chip,
            &combo,
            theme.panel_bg,
        );
    }

    /// 글꼴 섹션 — 제목 + (글꼴명 텍스트박스 · 크기 콤보) + 설명.
    #[allow(clippy::too_many_arguments)]
    fn paint_font_section(
        &self,
        ctx: &mut dyn DrawCtx,
        theme: &Theme,
        lang: Lang,
        e: &Entry,
        family_key: &'static str,
        size_key: &'static str,
        r: Rect,
        top: i32,
        reg_idx: usize,
    ) {
        ctx.fill_rect(r, theme.panel_bg);
        ctx.select_font(FontSlot::Base, true);
        ctx.text(
            r.x + self.s(12),
            r.y + self.s(6),
            r,
            tr(lang, e.label),
            theme.text,
        );

        let (fam, size) = self.section_ctrls(top);

        // 글꼴명 텍스트박스.
        let focused = self.focus == Focus::Family(reg_idx);
        ctx.fill_rect(fam, theme.field_bg);
        if focused {
            ctx.stroke_round_rect(fam, self.s(3), theme.accent, 1.5);
        }
        let family = self.values.get(family_key).map_or("", String::as_str);
        ctx.select_font(FontSlot::Base, false);
        let (ftext, fcolor) = if family.is_empty() {
            (tr(lang, Msg::SystemDefaultFont).to_string(), theme.text_dim)
        } else if focused {
            (format!("{family}|"), theme.text)
        } else {
            (family.to_string(), theme.text)
        };
        ctx.text(fam.x + self.s(8), fam.y + self.s(5), fam, &ftext, fcolor);

        // 크기 콤보(클릭/Enter = 다음 크기).
        ctx.fill_round_rect(size, self.s(6), theme.accent);
        let szval = self.values.get(size_key).map_or("m", String::as_str);
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            size.x + self.s(10),
            size.y + self.s(5),
            size,
            &format!("{} ▾", tr(lang, size_msg(szval))),
            theme.panel_bg,
        );

        // 설명.
        ctx.select_font(FontSlot::Status, false);
        ctx.text(
            r.x + self.s(12),
            r.y + self.s(64),
            r,
            tr(lang, e.desc),
            theme.text_dim,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (SettingsWidget, Invalidations) {
        let mut w = SettingsWidget::new(&SettingsState::with_defaults());
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 500, 500), &mut inv);
        (w, inv)
    }
    fn key(k: Key) -> InputEvent {
        InputEvent::Key {
            key: k,
            shift: false,
            primary: false,
        }
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
    fn font_cat() -> usize {
        SettingsWidget::cats()
            .iter()
            .position(|&c| c == Msg::CatFont)
            .unwrap()
    }
    fn font_first_reg_idx() -> usize {
        registry()
            .iter()
            .position(|e| matches!(e.kind, SettingKind::FontSection { .. }))
            .unwrap()
    }

    #[test]
    fn registry_is_single_source_render_equals_search() {
        let (w, _) = widget();
        let total: usize = w.sidebar_rows().iter().map(|&(_, n)| n).sum();
        assert_eq!(total, registry().len());
        let mut shown = 0;
        let mut w2 = w;
        for i in 0..SettingsWidget::cats().len() {
            w2.selected_cat = i;
            shown += w2.items().len();
        }
        assert_eq!(shown, registry().len());
    }

    #[test]
    fn font_category_has_four_sections() {
        let (mut w, _) = widget();
        w.selected_cat = font_cat();
        let items = w.items();
        assert_eq!(items.len(), 4, "기본/사용자목록/대화본문/상태바");
        for &i in &items {
            assert!(matches!(
                registry()[i].kind,
                SettingKind::FontSection { .. }
            ));
        }
    }

    #[test]
    fn defaults_seed_family_size_and_language() {
        let s = SettingsState::with_defaults();
        assert_eq!(
            s.get("font.base.family"),
            "",
            "글꼴명 기본 = 시스템 기본(빈값)"
        );
        assert_eq!(s.get("font.base.size"), "m", "크기 기본 = 보통");
        assert_eq!(s.get("ui.language"), "en", "언어 기본 = 영어");
        assert_eq!(s.get("ui.theme"), "dark");
        assert_eq!(s.get("chat.window_mode"), "single");
    }

    #[test]
    fn language_option_cycles_through_all_four() {
        let (mut w, mut inv) = widget();
        // 모양 카테고리에서 언어 항목을 찾아 순환.
        let lang_idx = registry()
            .iter()
            .position(|e| e.key == "ui.language")
            .unwrap();
        w.cycle_key(
            "ui.language",
            match registry()[lang_idx].kind {
                SettingKind::Radio(o) => o,
                SettingKind::FontSection { .. } => unreachable!(),
            },
            &mut inv,
        );
        assert_eq!(w.take_changes(), vec![("ui.language", "ko".to_string())]);
    }

    #[test]
    fn search_matches_across_languages() {
        // 영어 UI(기본)에서도 한국어로 검색된다(전 언어 매치).
        let (mut w, mut inv) = widget();
        for c in "테마".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        let rows = w.sidebar_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], (Msg::CatAppearance, 1), "테마 = 모양 1건");
        // 영어로도.
        let (mut w2, mut inv2) = widget();
        for c in "theme".chars() {
            w2.on_event(&ch(c), &mut inv2);
        }
        assert_eq!(w2.items().len(), 1);
        assert_eq!(registry()[w2.items()[0]].key, "ui.theme");
    }

    #[test]
    fn clicking_size_combo_cycles_size() {
        let (mut w, mut inv) = widget();
        w.selected_cat = font_cat();
        let (_fam, size) = w.section_ctrls(0);
        w.on_event(&click(size.x + 4, size.y + 4), &mut inv);
        assert_eq!(
            w.take_changes(),
            vec![("font.base.size", "l".to_string())],
            "m → l 순환"
        );
    }

    #[test]
    fn clicking_family_box_focuses_and_typing_edits_family() {
        let (mut w, mut inv) = widget();
        w.selected_cat = font_cat();
        let (fam, _size) = w.section_ctrls(0);
        w.on_event(&click(fam.x + 4, fam.y + 4), &mut inv);
        assert_eq!(w.focus, Focus::Family(font_first_reg_idx()));
        for c in "Arial".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        assert_eq!(
            w.values.get("font.base.family").map(String::as_str),
            Some("Arial")
        );
        assert!(w.query.is_empty(), "검색창이 아니라 글꼴명으로");
        w.on_event(&ch('\u{8}'), &mut inv);
        assert_eq!(
            w.values.get("font.base.family").map(String::as_str),
            Some("Aria")
        );
        assert!(w
            .take_changes()
            .iter()
            .any(|(k, _)| *k == "font.base.family"));
    }

    #[test]
    fn escape_blurs_family_before_closing() {
        let (mut w, mut inv) = widget();
        w.selected_cat = font_cat();
        let (fam, _s) = w.section_ctrls(0);
        w.on_event(&click(fam.x + 4, fam.y + 4), &mut inv);
        assert!(matches!(w.focus, Focus::Family(_)));
        w.on_event(&key(Key::Escape), &mut inv);
        assert_eq!(w.focus, Focus::Search, "첫 Esc = 블러");
        assert!(!w.take_back());
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(w.take_back(), "둘째 Esc = 닫기");
    }

    #[test]
    fn sidebar_click_selects_category() {
        let (mut w, mut inv) = widget();
        // 둘째 사이드바 행("모양"/Appearance) 클릭 — 항목 2개(테마·언어).
        w.on_event(&click(10, SEARCH_H + TREE_ROW_H + 5), &mut inv);
        let items = w.items();
        assert_eq!(items.len(), 2, "테마 + 언어");
        assert_eq!(registry()[items[0]].key, "ui.theme");
        assert_eq!(registry()[items[1]].key, "ui.language");
    }

    #[test]
    fn enter_cycles_radio_value_and_reports_change() {
        let (mut w, mut inv) = widget();
        w.on_event(&key(Key::Enter), &mut inv); // 첫 항목 = chat.window_mode: single → separate
        let c = w.take_changes();
        assert_eq!(c, vec![("chat.window_mode", "separate".to_string())]);
        assert!(w.take_changes().is_empty(), "1회성 드레인");
        w.on_event(&key(Key::Enter), &mut inv);
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
}
