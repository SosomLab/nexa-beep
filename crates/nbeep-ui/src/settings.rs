//! 설정 화면 — **VS Code 방식** · **커스텀 컨트롤 툴킷으로 구성**(DR-24 · [docs/14 §10]).
//!
//! 핵심 발명은 **Entry 레지스트리 단일 원천**이다: 영속 설정 전부가 [`registry`]에 등록되고,
//! 렌더와 검색이 같은 원천을 읽는다 — "화면에 있는데 검색 안 되는 설정"이 구조적으로 불가능하다.
//!
//! ## 컨트롤 구성(사용자 확정 08-09 — 자체 렌더 전면 교체)
//!
//! | 요소 | 컨트롤 |
//! |---|---|
//! | 검색 | [`TextBox`](placeholder·Beam 캐럿) |
//! | 카테고리 사이드바 | [`TreeView`](검색 중 매치 카테고리 + "(N)") |
//! | 택일 설정 | [`Combo`](드롭다운 · 선택 ✓) |
//! | on/off 설정 | [`Checkbox`] |
//! | 글꼴 영역 | [`TextBox`] 글꼴명 + [`Combo`] 크기 |
//!
//! 값 반영은 기존 계약 그대로 — **즉시 적용**([`SettingsWidget::take_changes`] 폴링), 영속은
//! M2-5(Repository 포트). i18n: 라벨은 [`Msg`] 키, 검색은 **전 언어 매치**.

use crate::controls::{
    Checkbox, Combo, ComboControl, ComboItem, Control, LabelSide, ScrollBars, TextBox, TreeControl,
    TreeModel, TreeNode, TreeView,
};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::{current_lang, tr, Lang, Msg};
use std::collections::HashMap;

/// 크기 콤보 후보(전 글꼴 영역 공용) — (값, 라벨 Msg). 작은 것→큰 것 순(사용자 확정 08-09).
const SIZE_OPTS: &[(&str, Msg)] = &[
    ("s", Msg::SizeSmall),
    ("m", Msg::SizeNormal),
    ("l", Msg::SizeLarge),
    ("xl", Msg::SizeExtraLarge),
];

/// 크기 기본값 — 순서와 무관하게 '보통' 고정.
const SIZE_DEFAULT: &str = "m";

/// Radio 기본값 예외 — 표시 순서(오름차순 등)와 기본값이 다른 키만 등록.
/// 미등록 키의 기본은 첫 옵션(기존 규약).
const RADIO_DEFAULTS: &[(&str, &str)] =
    &[("ui.toolbar_size", "24"), ("ui.typeahead_timeout", "2000")];

/// 항목 종류 — 우측 패널이 이 열거를 읽어 컨트롤을 동적 생성한다(새 설정 = Entry 1줄).
#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    /// 값 후보 중 택일 — [`Combo`] 드롭다운.
    Radio(&'static [(&'static str, Msg)]),
    /// 택일 + **직접 입력** — 후보에 없는 값을 인라인 편집으로 넣는다(값, 표시 접미).
    RadioInput(&'static [(&'static str, Msg)], &'static str),
    /// on/off — [`Checkbox`]. 값은 `"on"`/`"off"`(기본 on).
    Toggle,
    /// 글꼴 영역 — 글꼴명 [`TextBox`] + 크기 [`Combo`].
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
    /// 카테고리(사이드바·검색 대상).
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
    /// 레지스트리 기본값(각 값 키 → 기본 문자열).
    fn default_values(&self) -> Vec<(&'static str, String)> {
        match self.kind {
            SettingKind::Radio(opts) | SettingKind::RadioInput(opts, _) => RADIO_DEFAULTS
                .iter()
                .find(|(k, _)| *k == self.key)
                .map(|(_, v)| *v)
                .or_else(|| opts.first().map(|(v, _)| *v))
                .map(|v| (self.key, v.to_string()))
                .into_iter()
                .collect(),
            SettingKind::Toggle => vec![(self.key, "on".to_string())],
            SettingKind::FontSection {
                family_key,
                size_key,
            } => vec![
                (family_key, String::new()), // 빈 문자열 = 시스템 기본 글꼴
                (size_key, SIZE_DEFAULT.to_string()),
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
            cat: Msg::CatAppearance,
            label: Msg::ToolbarSize,
            desc: Msg::ToolbarSizeDesc,
            kind: SettingKind::Radio(&[
                ("16", Msg::Tb16),
                ("24", Msg::Tb24),
                ("32", Msg::Tb32),
                ("64", Msg::Tb64),
            ]),
            key: "ui.toolbar_size",
        },
        Entry {
            cat: Msg::CatAppearance,
            label: Msg::TypeaheadTimeout,
            desc: Msg::TypeaheadTimeoutDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("1000", Msg::TaSec1),
                    ("2000", Msg::TaSec2),
                    ("3000", Msg::TaSec3),
                    ("5000", Msg::TaSec5),
                    ("10000", Msg::TaSec10),
                ],
                "ms",
            ),
            key: "ui.typeahead_timeout",
        },
        Entry {
            cat: Msg::CatAppearance,
            label: Msg::TypeaheadPos,
            desc: Msg::TypeaheadPosDesc,
            kind: SettingKind::Radio(&[
                ("bl", Msg::PosBottomLeft),
                ("bc", Msg::PosBottomCenter),
                ("br", Msg::PosBottomRight),
                ("ml", Msg::PosMidLeft),
                ("c", Msg::PosCenter),
                ("mr", Msg::PosMidRight),
                ("tl", Msg::PosTopLeft),
                ("tc", Msg::PosTopCenter),
                ("tr", Msg::PosTopRight),
            ]),
            key: "ui.typeahead_pos",
        },
        Entry {
            cat: Msg::CatAppearance,
            label: Msg::TypeaheadSpace,
            desc: Msg::TypeaheadSpaceDesc,
            kind: SettingKind::Toggle,
            key: "ui.typeahead_space",
        },
        Entry {
            cat: Msg::CatAppearance,
            label: Msg::TypeaheadSpecial,
            desc: Msg::TypeaheadSpecialDesc,
            kind: SettingKind::Toggle,
            key: "ui.typeahead_special",
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

// 레이아웃(논리 px).
const SIDEBAR_W: i32 = 150;
const SEARCH_H: i32 = 30;
const ENTRY_H: i32 = 52;
const FONT_SECTION_H: i32 = 88;
const CTL_H: i32 = 26;
const COMBO_W: i32 = 170;
const SIZE_W: i32 = 112;
const FAMILY_W: i32 = 180;
const PAD: i32 = 12;

/// 우측 한 행 = 레지스트리 항목 + 실물 컨트롤.
#[derive(Debug)]
enum RowCtl {
    Combo(Combo),
    Check(Checkbox),
    Font { family: TextBox, size: Combo },
}

#[derive(Debug)]
struct RowUi {
    /// registry 인덱스.
    idx: usize,
    /// 행 영역(우측 패널 안 · 물리 px).
    rect: Rect,
    ctl: RowCtl,
}

/// 설정 위젯 — 커스텀 컨트롤 컴포지션.
#[derive(Debug)]
pub struct SettingsWidget {
    bounds: Rect,
    scale: f32,
    /// 검색 입력(TextBox).
    search: TextBox,
    /// 검색어 미러(rebuild 트리거 비교용).
    query: String,
    /// 카테고리 사이드바(TreeView).
    tree: TreeView,
    /// 사이드바 가시 행 → cats() 인덱스.
    cat_map: Vec<usize>,
    /// 선택 카테고리(cats() 인덱스).
    selected_cat: usize,
    /// 우측 행들(가시 항목 + 컨트롤).
    rows: Vec<RowUi>,
    /// 현재 값 스냅숏(컨트롤 초기화·보고 근거).
    values: HashMap<&'static str, String>,
    changes: Vec<(&'static str, String)>,
    back: bool,
    /// 우측 패널 세로 스크롤 오프셋(물리 px).
    scroll: i32,
    /// 우측 패널 콘텐츠 총 높이(물리 px) — layout에서 계산.
    content_h: i32,
    /// 우측 패널 오버레이 스크롤바.
    bars: ScrollBars,
}

impl SettingsWidget {
    /// 현재 값 스냅숏으로 연다.
    #[must_use]
    pub fn new(state: &SettingsState) -> Self {
        let mut values = HashMap::new();
        for e in registry() {
            for (k, _) in e.default_values() {
                values.insert(k, state.get(k).to_string());
            }
        }
        let mut w = Self {
            bounds: Rect::default(),
            scale: 1.0,
            search: TextBox::new("Search"),
            query: String::new(),
            tree: TreeView::new(TreeModel::default()),
            cat_map: Vec::new(),
            selected_cat: 0,
            rows: Vec::new(),
            values,
            changes: Vec::new(),
            back: false,
            scroll: 0,
            content_h: 0,
            bars: ScrollBars::new(),
        };
        let mut inv = Invalidations::default();
        w.rebuild(&mut inv);
        w
    }

    /// 배율 지정(고DPI) — 전 컨트롤 전파 + 재구성.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let s = scale.max(0.5);
        if (s - self.scale).abs() > f32::EPSILON {
            self.scale = s;
            self.rebuild(inv);
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

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 카테고리 목록(레지스트리 순서·중복 제거).
    fn cats() -> Vec<Msg> {
        let mut out: Vec<Msg> = Vec::new();
        for e in registry() {
            if !out.contains(&e.cat) {
                out.push(e.cat);
            }
        }
        out
    }

    fn cat_match_count(cat: Msg, toks: &[String]) -> usize {
        registry()
            .iter()
            .filter(|e| e.cat == cat && entry_matches(e, toks))
            .count()
    }

    /// 가시 항목(registry 인덱스) — 검색 중=전 카테고리 매치, 아니면 선택 카테고리.
    fn visible_indices(&self) -> Vec<usize> {
        let toks = tokens(&self.query);
        let searching = !toks.is_empty();
        let selected = Self::cats().get(self.selected_cat).copied();
        registry()
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if searching {
                    entry_matches(e, &toks)
                } else {
                    Some(e.cat) == selected
                }
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 사이드바·우측 행(컨트롤 포함)을 현재 상태(검색·선택·값)로 다시 만든다.
    fn rebuild(&mut self, inv: &mut Invalidations) {
        let lang = current_lang();
        let toks = tokens(&self.query);
        let searching = !toks.is_empty();

        // ── 사이드바 트리(카테고리 · 검색 중엔 매치만 + "(N)") ──
        let cats = Self::cats();
        self.cat_map.clear();
        let mut roots = Vec::new();
        for (ci, &cat) in cats.iter().enumerate() {
            let n = Self::cat_match_count(cat, &toks);
            if searching && n == 0 {
                continue;
            }
            let label = if searching {
                format!("{} ({n})", tr(lang, cat))
            } else {
                tr(lang, cat).to_string()
            };
            roots.push(TreeNode::leaf(label));
            self.cat_map.push(ci);
        }
        let mut tree = TreeView::new(TreeModel::new(roots));
        tree.set_scale(self.scale);
        tree.set_focused(true); // 사이드바는 ↑↓ 상시 탐색(트리 자체 포커스 링 없음)
        let sel_row = self
            .cat_map
            .iter()
            .position(|&c| c == self.selected_cat)
            .unwrap_or(0);
        tree.set_selected_row(sel_row);
        self.tree = tree;

        // ── 우측 행 + 컨트롤 ──
        self.rows.clear();
        for idx in self.visible_indices() {
            let e = &registry()[idx];
            let ctl = match e.kind {
                SettingKind::Radio(opts) | SettingKind::RadioInput(opts, _) => {
                    let items: Vec<ComboItem> = opts
                        .iter()
                        .map(|(v, m)| ComboItem::new(*v, tr(lang, *m)))
                        .collect();
                    let mut c = Combo::new(items, 0);
                    if let SettingKind::RadioInput(_, suffix) = e.kind {
                        c.set_custom_entry(tr(lang, Msg::CustomInput), suffix);
                    }
                    c.select_value(self.values.get(e.key).map_or("", String::as_str));
                    c.set_scale(self.scale);
                    RowCtl::Combo(c)
                }
                SettingKind::Toggle => {
                    let mut c =
                        Checkbox::new("", self.values.get(e.key).map(String::as_str) == Some("on"))
                            .with_label_side(LabelSide::None);
                    c.set_scale(self.scale);
                    RowCtl::Check(c)
                }
                SettingKind::FontSection {
                    family_key,
                    size_key,
                } => {
                    let mut family = TextBox::new(tr(lang, Msg::SystemDefaultFont))
                        .with_text(self.values.get(family_key).map_or("", String::as_str));
                    family.set_scale(self.scale);
                    let items: Vec<ComboItem> = SIZE_OPTS
                        .iter()
                        .map(|(v, m)| ComboItem::new(*v, tr(lang, *m)))
                        .collect();
                    let mut size = Combo::new(items, 0);
                    size.select_value(self.values.get(size_key).map_or("m", String::as_str));
                    size.set_scale(self.scale);
                    RowCtl::Font { family, size }
                }
            };
            self.rows.push(RowUi {
                idx,
                rect: Rect::default(),
                ctl,
            });
        }
        self.layout(inv);
    }

    /// 우측 패널 뷰포트(사이드바 제외 영역).
    fn right_viewport(&self) -> Rect {
        let sw = self.s(SIDEBAR_W);
        let b = self.bounds;
        Rect::new(b.x + sw, b.y, (b.w - sw).max(0), b.h)
    }

    /// 스크롤바 페이드 틱(호스트 ~5Hz) — 표시가 바뀌면 `true`(재그리기).
    pub fn tick(&mut self) -> bool {
        self.bars.tick() || self.tree.tick()
    }

    /// 현 bounds에 맞춰 자식 컨트롤 배치.
    fn layout(&mut self, inv: &mut Invalidations) {
        let sw = self.s(SIDEBAR_W);
        let b = self.bounds;
        self.search.set_bounds(
            Rect::new(
                b.x + self.s(4),
                b.y + self.s(4),
                sw - self.s(8),
                self.s(SEARCH_H),
            ),
            inv,
        );
        let tree_top = b.y + self.s(SEARCH_H) + self.s(8);
        self.tree.set_bounds(
            Rect::new(b.x, tree_top, sw, (b.bottom() - tree_top).max(0)),
            inv,
        );

        let rx = b.x + sw; // 우측 패널 시작
        let rw = (b.w - sw).max(0);
        // 콘텐츠 총 높이 → 스크롤 클램프(행 추가/검색으로 줄어들면 위로 당긴다).
        let (hf, he) = (self.s(FONT_SECTION_H), self.s(ENTRY_H));
        self.content_h = self
            .rows
            .iter()
            .map(|row| match registry()[row.idx].kind {
                SettingKind::FontSection { .. } => hf,
                _ => he,
            })
            .sum();
        self.scroll = self.scroll.clamp(0, (self.content_h - b.h).max(0));
        let mut top = b.y - self.scroll;
        // 차용 분리를 위해 치수 사전 계산.
        let (ctl_h, pad) = (self.s(CTL_H), self.s(PAD));
        let (h_font, h_entry) = (self.s(FONT_SECTION_H), self.s(ENTRY_H));
        let (combo_w, check_w) = (self.s(COMBO_W), self.s(22));
        let (family_w, size_w, gap10, dy32) =
            (self.s(FAMILY_W), self.s(SIZE_W), self.s(10), self.s(32));
        for row in &mut self.rows {
            let e = &registry()[row.idx];
            let h = match e.kind {
                SettingKind::FontSection { .. } => h_font,
                _ => h_entry,
            };
            row.rect = Rect::new(rx, top, rw, h);
            match &mut row.ctl {
                RowCtl::Combo(c) => {
                    c.set_bounds(
                        Rect::new(
                            rx + rw - combo_w - pad,
                            top + (h - ctl_h) / 2,
                            combo_w,
                            ctl_h,
                        ),
                        inv,
                    );
                }
                RowCtl::Check(c) => {
                    c.set_bounds(
                        Rect::new(
                            rx + rw - check_w - pad,
                            top + (h - ctl_h) / 2,
                            check_w,
                            ctl_h,
                        ),
                        inv,
                    );
                }
                RowCtl::Font { family, size } => {
                    let fy = top + dy32;
                    family.set_bounds(Rect::new(rx + pad, fy, family_w, ctl_h), inv);
                    size.set_bounds(
                        Rect::new(rx + pad + family_w + gap10, fy, size_w, ctl_h),
                        inv,
                    );
                }
            }
            top += h;
        }
        inv.push(self.bounds);
    }

    /// 자식 컨트롤 변경분을 회수해 values/changes에 반영.
    fn drain_changes(&mut self, inv: &mut Invalidations) {
        let mut got = Vec::new();
        for row in &mut self.rows {
            let e = &registry()[row.idx];
            match &mut row.ctl {
                RowCtl::Combo(c) => {
                    if let Some(v) = c.take_changed() {
                        got.push((e.key, v));
                    }
                }
                RowCtl::Check(c) => {
                    if let Some(on) = c.take_toggled() {
                        got.push((e.key, if on { "on" } else { "off" }.to_string()));
                    }
                }
                RowCtl::Font { family, size } => {
                    if let SettingKind::FontSection {
                        family_key,
                        size_key,
                    } = e.kind
                    {
                        if let Some(v) = family.take_changed() {
                            got.push((family_key, v));
                        }
                        if let Some(v) = size.take_changed() {
                            got.push((size_key, v));
                        }
                    }
                }
            }
        }
        if !got.is_empty() {
            for (k, v) in &got {
                self.values.insert(k, v.clone());
            }
            self.changes.extend(got);
            inv.push(self.bounds);
        }
    }

    /// 열린 콤보(모달 캡처 대상)를 찾는다.
    fn open_combo_mut(&mut self) -> Option<&mut Combo> {
        self.rows.iter_mut().find_map(|r| match &mut r.ctl {
            RowCtl::Combo(c) if c.is_open() => Some(c),
            RowCtl::Font { size, .. } if size.is_open() => Some(size),
            _ => None,
        })
    }

    fn any_family_focused(&self) -> bool {
        self.rows.iter().any(|r| match &r.ctl {
            RowCtl::Font { family, .. } => family.is_focused(),
            _ => false,
        })
    }
}

impl Widget for SettingsWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.layout(inv);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // ── 모달 캡처: 열린 콤보가 있으면 그 콤보만 이벤트를 받는다(전파 차단) ──
        if let Some(c) = self.open_combo_mut() {
            c.on_event(ev, inv);
            self.drain_changes(inv);
            inv.push(self.bounds); // 드롭다운 영역 재그리기
            return;
        }

        // ── 인라인 편집(직접 입력) 모달 캡처 — 편집 중 콤보가 모든 입력을 받는다 ──
        if let Some(c) = self.rows.iter_mut().find_map(|r| match &mut r.ctl {
            RowCtl::Combo(c) if c.is_editing() => Some(c),
            _ => None,
        }) {
            c.on_event(ev, inv);
            self.drain_changes(inv);
            inv.push(self.bounds);
            return;
        }

        // ── 우측 패널 오버레이 스크롤(세로 전용) — 콤보 열림 중에는 위 캡처가 우선 ──
        {
            let vp = self.right_viewport();
            let (_, ny, consumed) =
                self.bars
                    .on_event(ev, vp, vp.w, self.content_h, 0, self.scroll, self.scale);
            if ny != self.scroll {
                self.scroll = ny;
                self.layout(inv);
                inv.push(self.bounds);
            }
            if consumed {
                inv.push(self.bounds);
                return;
            }
        }

        match *ev {
            InputEvent::MouseDown { x, y, .. } => {
                let p = Point { x, y };
                // 검색/글꼴명 포커스는 클릭 위치 기준(각 컨트롤이 스스로 잡음 + 여기서 블러).
                self.search.set_focused(self.search.bounds().contains(p));
                for row in &mut self.rows {
                    if let RowCtl::Font { family, .. } = &mut row.ctl {
                        family.set_focused(family.bounds().contains(p));
                    }
                }
                // 사이드바 트리 — 선택 변경 감지 → 카테고리 전환(검색 해제).
                let before = self.tree.selected_row();
                self.tree.on_event(ev, inv);
                let after = self.tree.selected_row();
                if self.tree.bounds().contains(p) && after != before
                    || (self.tree.bounds().contains(p) && !self.query.is_empty())
                {
                    if let Some(&ci) = self.cat_map.get(after) {
                        self.selected_cat = ci;
                    }
                    self.query.clear();
                    self.search.set_text("");
                    self.rebuild(inv);
                    return;
                }
                // 우측 컨트롤들.
                for row in &mut self.rows {
                    match &mut row.ctl {
                        RowCtl::Combo(c) => c.on_event(ev, inv),
                        RowCtl::Check(c) => c.on_event(ev, inv),
                        RowCtl::Font { family, size } => {
                            family.on_event(ev, inv);
                            size.on_event(ev, inv);
                        }
                    }
                }
                self.drain_changes(inv);
                inv.push(self.bounds);
            }
            InputEvent::Char { .. } => {
                if self.any_family_focused() {
                    for row in &mut self.rows {
                        if let RowCtl::Font { family, .. } = &mut row.ctl {
                            if family.is_focused() {
                                family.on_event(ev, inv);
                            }
                        }
                    }
                    self.drain_changes(inv);
                } else {
                    // 기본 타이핑 = 검색(포커스 없어도 검색으로 흐른다 — 기존 UX 유지).
                    self.search.set_focused(true);
                    self.search.on_event(ev, inv);
                    let q = self.search.text();
                    if q != self.query {
                        self.query = q;
                        self.rebuild(inv);
                    }
                }
                inv.push(self.bounds);
            }
            InputEvent::Key { key, .. } => match key {
                Key::Escape => {
                    if self.any_family_focused() {
                        for row in &mut self.rows {
                            if let RowCtl::Font { family, .. } = &mut row.ctl {
                                family.set_focused(false);
                            }
                        }
                        inv.push(self.bounds);
                    } else {
                        self.back = true;
                    }
                }
                Key::Up | Key::Down if self.query.is_empty() => {
                    // 사이드바 카테고리 탐색(검색 중엔 유지).
                    let before = self.tree.selected_row();
                    self.tree.on_event(ev, inv);
                    let after = self.tree.selected_row();
                    if after != before {
                        if let Some(&ci) = self.cat_map.get(after) {
                            self.selected_cat = ci;
                        }
                        self.rebuild(inv);
                    }
                }
                _ => {}
            },
            _ => {
                // 휠 등 — 트리(스크롤바)로.
                self.tree.on_event(ev, inv);
            }
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let lang = current_lang();
        ctx.fill_rect(self.bounds, theme.panel_bg);
        let sw = self.s(SIDEBAR_W);

        // 사이드바 배경 + 검색 + 트리 + 경계선.
        ctx.fill_rect(
            Rect::new(self.bounds.x, self.bounds.y, sw, self.bounds.h),
            theme.chrome_bg,
        );
        self.search.paint(ctx, theme);
        self.tree.paint(ctx, theme);
        ctx.fill_rect(
            Rect::new(self.bounds.x + sw - 1, self.bounds.y, 1, self.bounds.h),
            theme.border,
        );

        // 우측 행: 라벨/설명 + 컨트롤.
        for row in &self.rows {
            let e = &registry()[row.idx];
            let r = row.rect;
            match &row.ctl {
                RowCtl::Combo(_) | RowCtl::Check(_) => {
                    ctx.select_font(FontSlot::Base, false);
                    ctx.text(
                        r.x + self.s(PAD),
                        r.y + self.s(6),
                        r,
                        tr(lang, e.label),
                        theme.text,
                    );
                    ctx.select_font(FontSlot::Status, false);
                    ctx.text(
                        r.x + self.s(PAD),
                        r.y + self.s(30),
                        r,
                        tr(lang, e.desc),
                        theme.text_dim,
                    );
                }
                RowCtl::Font { .. } => {
                    ctx.select_font(FontSlot::Base, true);
                    ctx.text(
                        r.x + self.s(PAD),
                        r.y + self.s(6),
                        r,
                        tr(lang, e.label),
                        theme.text,
                    );
                    ctx.select_font(FontSlot::Status, false);
                    ctx.text(
                        r.x + self.s(PAD),
                        r.y + self.s(64),
                        r,
                        tr(lang, e.desc),
                        theme.text_dim,
                    );
                }
            }
            match &row.ctl {
                RowCtl::Combo(c) => c.paint(ctx, theme),
                RowCtl::Check(c) => c.paint(ctx, theme),
                RowCtl::Font { family, size } => {
                    family.paint(ctx, theme);
                    size.paint(ctx, theme);
                }
            }
        }
        // 열린 콤보 드롭다운은 맨 위에 다시 그린다(아래 행에 가리지 않게).
        for row in &self.rows {
            match &row.ctl {
                RowCtl::Combo(c) if c.is_open() => c.paint(ctx, theme),
                RowCtl::Font { size, .. } if size.is_open() => size.paint(ctx, theme),
                _ => {}
            }
        }
        // 우측 패널 오버레이 스크롤바(맨 위에 겹침 · 세로 전용).
        let vp = self.right_viewport();
        self.bars.paint(
            ctx,
            theme,
            vp,
            vp.w,
            self.content_h,
            0,
            self.scroll,
            self.scale,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (SettingsWidget, Invalidations) {
        let mut w = SettingsWidget::new(&SettingsState::with_defaults());
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 560, 560), &mut inv);
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
    /// 카테고리 강제 선택(테스트 헬퍼).
    fn select_cat(w: &mut SettingsWidget, cat: Msg) {
        let ci = SettingsWidget::cats()
            .iter()
            .position(|&c| c == cat)
            .unwrap();
        w.selected_cat = ci;
        let mut inv = Invalidations::default();
        w.rebuild(&mut inv);
        w.set_bounds(Rect::new(0, 0, 560, 560), &mut inv);
    }

    #[test]
    fn registry_is_single_source() {
        // 전 카테고리 가시 항목 합 == 레지스트리 전체(트리 밖 설정 구조적 불가).
        let (mut w, _) = widget();
        let mut shown = 0;
        for i in 0..SettingsWidget::cats().len() {
            w.selected_cat = i;
            shown += w.visible_indices().len();
        }
        assert_eq!(shown, registry().len());
    }

    #[test]
    fn defaults_include_toggles_on() {
        let s = SettingsState::with_defaults();
        assert_eq!(s.get("ui.typeahead_space"), "on");
        assert_eq!(s.get("ui.typeahead_special"), "on");
        assert_eq!(s.get("chat.window_mode"), "single");
        assert_eq!(s.get("ui.language"), "en");
        assert_eq!(s.get("font.base.size"), "m");
    }

    #[test]
    fn combo_row_selection_reports_change() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatAppearance);
        // 첫 행 = 테마 콤보. 콤보 클릭 → 열림 → 두 번째 항목(light) 클릭.
        let cb = match &w.rows[0].ctl {
            RowCtl::Combo(c) => c.bounds(),
            _ => panic!("첫 행은 콤보"),
        };
        w.on_event(&click(cb.x + 5, cb.y + 5), &mut inv);
        let pop = match &w.rows[0].ctl {
            RowCtl::Combo(c) => {
                assert!(c.is_open(), "클릭 = 드롭다운 열림");
                c.popup_rect()
            }
            _ => unreachable!(),
        };
        let item_h = 26; // combo ROW_H(scale 1)
        w.on_event(&click(pop.x + 30, pop.y + 4 + item_h + 5), &mut inv);
        assert_eq!(w.take_changes(), vec![("ui.theme", "light".to_string())]);
    }

    #[test]
    fn checkbox_row_toggles_off() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatAppearance);
        // Toggle 행(공백 포함) 찾기.
        let (i, cb) = w
            .rows
            .iter()
            .enumerate()
            .find_map(|(i, r)| match &r.ctl {
                RowCtl::Check(c) => Some((i, c.bounds())),
                _ => None,
            })
            .expect("토글 행 존재");
        assert_eq!(registry()[w.rows[i].idx].key, "ui.typeahead_space");
        w.on_event(&click(cb.x + 3, cb.y + cb.h / 2), &mut inv);
        assert_eq!(
            w.take_changes(),
            vec![("ui.typeahead_space", "off".to_string())]
        );
    }

    #[test]
    fn font_family_textbox_types_and_reports() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatFont);
        let fb = match &w.rows[0].ctl {
            RowCtl::Font { family, .. } => family.bounds(),
            _ => panic!("글꼴 행"),
        };
        w.on_event(&click(fb.x + 5, fb.y + 5), &mut inv);
        for c in "Arial".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        let changes = w.take_changes();
        assert!(
            changes
                .iter()
                .any(|(k, v)| *k == "font.base.family" && v == "Arial"),
            "{changes:?}"
        );
        // Esc 1회 = 글꼴명 블러(닫힘 아님), 2회 = 닫기.
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(!w.take_back());
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(w.take_back());
    }

    #[test]
    fn search_filters_across_languages_and_sidebar_counts() {
        let (mut w, mut inv) = widget();
        for c in "테마".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        assert_eq!(w.visible_indices().len(), 1, "테마 1건");
        assert_eq!(registry()[w.visible_indices()[0]].key, "ui.theme");
        assert_eq!(w.cat_map.len(), 1, "매치 있는 카테고리만 사이드바에");
        // 영어로도 매치.
        let (mut w2, mut inv2) = widget();
        for c in "language".chars() {
            w2.on_event(&ch(c), &mut inv2);
        }
        assert!(w2
            .visible_indices()
            .iter()
            .any(|&i| registry()[i].key == "ui.language"));
    }

    #[test]
    fn sidebar_click_switches_category_and_clears_search() {
        let (mut w, mut inv) = widget();
        // 트리 두 번째 행(모양) 클릭.
        let tb = w.tree.bounds();
        w.on_event(&click(tb.x + 10, tb.y + 24 + 5), &mut inv);
        assert_eq!(w.selected_cat, 1, "모양 선택");
        assert!(w.rows.iter().any(|r| registry()[r.idx].key == "ui.theme"));
    }

    #[test]
    fn escape_requests_close() {
        let (mut w, mut inv) = widget();
        assert!(!w.take_back());
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(w.take_back());
    }
}
