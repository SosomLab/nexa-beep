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
    Checkbox, Combo, ComboControl, ComboItem, Control, LabelSide, PositionPicker, ScrollBars,
    TextBox, TreeControl, TreeModel, TreeNode, TreeView,
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
    /// 3×3 위치 그리드 — 미니 화면(4:3) 셀로 직관 선택([`PositionPicker`]).
    PositionGrid,
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
    /// 하위 카테고리(없으면 최상위 직속) — 사이드바 계층·필터 근거.
    pub sub: Option<Msg>,
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
            SettingKind::PositionGrid => vec![(self.key, "bl".to_string())],
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
            sub: None,
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
            sub: None,
            label: Msg::Theme,
            desc: Msg::ThemeDesc,
            kind: SettingKind::Radio(&[("dark", Msg::ThemeDark), ("light", Msg::ThemeLight)]),
            key: "ui.theme",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: None,
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
            sub: None,
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
            sub: Some(Msg::CatTypeahead),
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
            sub: Some(Msg::CatTypeahead),
            label: Msg::TypeaheadPos,
            desc: Msg::TypeaheadPosDesc,
            kind: SettingKind::PositionGrid,
            key: "ui.typeahead_pos",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatTypeahead),
            label: Msg::TypeaheadSpace,
            desc: Msg::TypeaheadSpaceDesc,
            kind: SettingKind::Toggle,
            key: "ui.typeahead_space",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatTypeahead),
            label: Msg::TypeaheadSpecial,
            desc: Msg::TypeaheadSpecialDesc,
            kind: SettingKind::Toggle,
            key: "ui.typeahead_special",
        },
        Entry {
            cat: Msg::CatFont,
            sub: None,
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
            sub: None,
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
            sub: None,
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
            sub: None,
            label: Msg::FontStatus,
            desc: Msg::FontStatusDesc,
            kind: SettingKind::FontSection {
                family_key: "font.status.family",
                size_key: "font.status.size",
            },
            key: "font.status.family",
        },
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::XferApproval,
            desc: Msg::XferApprovalDesc,
            kind: SettingKind::Radio(&[
                ("manual", Msg::ApprovalManual),
                ("auto", Msg::ApprovalAuto),
                ("timed", Msg::ApprovalTimed),
                ("block", Msg::ApprovalBlock),
            ]),
            key: "xfer.approval",
        },
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::XferWindow,
            desc: Msg::XferWindowDesc,
            kind: SettingKind::Radio(&[
                ("1h", Msg::Win1h),
                ("6h", Msg::Win6h),
                ("today", Msg::WinToday),
            ]),
            key: "xfer.approval_window",
        },
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::SendRate,
            desc: Msg::SendRateDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("auto", Msg::RateAuto),
                    ("100k", Msg::Rate100k),
                    ("1m", Msg::Rate1m),
                    ("10m", Msg::Rate10m),
                    ("100m", Msg::Rate100m),
                    ("1g", Msg::Rate1g),
                ],
                "B/s",
            ),
            key: "xfer.send_rate",
        },
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::RecvRate,
            desc: Msg::RecvRateDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("auto", Msg::RateAuto),
                    ("100k", Msg::Rate100k),
                    ("1m", Msg::Rate1m),
                    ("10m", Msg::Rate10m),
                    ("100m", Msg::Rate100m),
                    ("1g", Msg::Rate1g),
                ],
                "B/s",
            ),
            key: "xfer.recv_rate",
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
/// 위치 그리드 행 높이(3×3 미니 화면 93 + 여백).
const POS_ROW_H: i32 = 110;
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
    Font {
        family: TextBox,
        size: Combo,
    },
    /// 3×3 위치 그리드.
    Pos(PositionPicker),
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
    /// 사이드바 가시 행 → (cats() 인덱스, 하위 카테고리).
    cat_map: Vec<(usize, Option<Msg>)>,
    /// 선택 카테고리(cats() 인덱스).
    selected_cat: usize,
    /// 선택 하위 카테고리(None = 최상위 — 하위 항목도 함께 보인다).
    selected_sub: Option<Msg>,
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
    /// 사이드바 폭(논리 px) — 스플리터 드래그로 조절(사용자 요청 08-09).
    sidebar_w: i32,
    /// 스플리터 드래그 중.
    split_drag: bool,
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
            search: TextBox::new("Search").with_clearable(),
            query: String::new(),
            tree: TreeView::new(TreeModel::default()),
            cat_map: Vec::new(),
            selected_cat: 0,
            selected_sub: None,
            rows: Vec::new(),
            values,
            changes: Vec::new(),
            back: false,
            scroll: 0,
            content_h: 0,
            bars: ScrollBars::new(),
            sidebar_w: SIDEBAR_W,
            split_drag: false,
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
            self.search.set_scale(s);
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

    /// 카테고리 목록(레지스트리 순서·중복 제거) — (최상위, 하위들).
    fn cats() -> Vec<(Msg, Vec<Msg>)> {
        let mut out: Vec<(Msg, Vec<Msg>)> = Vec::new();
        for e in registry() {
            if !out.iter().any(|(c, _)| *c == e.cat) {
                out.push((e.cat, Vec::new()));
            }
            if let Some(sub) = e.sub {
                if let Some((_, subs)) = out.iter_mut().find(|(c, _)| *c == e.cat) {
                    if !subs.contains(&sub) {
                        subs.push(sub);
                    }
                }
            }
        }
        out
    }

    fn cat_match_count(cat: Msg, sub: Option<Msg>, toks: &[String]) -> usize {
        registry()
            .iter()
            .filter(|e| e.cat == cat && (sub.is_none() || e.sub == sub) && entry_matches(e, toks))
            .count()
    }

    /// 가시 항목(registry 인덱스) — 검색 중=전 카테고리 매치, 아니면 선택 카테고리.
    fn visible_indices(&self) -> Vec<usize> {
        let toks = tokens(&self.query);
        let searching = !toks.is_empty();
        let selected = Self::cats().get(self.selected_cat).map(|(c, _)| *c);
        registry()
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if searching {
                    entry_matches(e, &toks)
                } else if Some(e.cat) != selected {
                    false
                } else {
                    // 최상위 선택 = 하위 포함 전부 · 하위 선택 = 그 하위만(VS Code식).
                    self.selected_sub.is_none() || e.sub == self.selected_sub
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

        // ── 사이드바 트리(계층 카테고리 · 검색 중엔 매치만 + "(N)") ──
        let cats = Self::cats();
        self.cat_map.clear();
        let mut roots = Vec::new();
        for (ci, (cat, subs)) in cats.iter().enumerate() {
            let n = Self::cat_match_count(*cat, None, &toks);
            if searching && n == 0 {
                continue;
            }
            let label = if searching {
                format!("{} ({n})", tr(lang, *cat))
            } else {
                tr(lang, *cat).to_string()
            };
            self.cat_map.push((ci, None));
            let mut children = Vec::new();
            for &sub in subs {
                let sn = Self::cat_match_count(*cat, Some(sub), &toks);
                if searching && sn == 0 {
                    continue;
                }
                let sl = if searching {
                    format!("{} ({sn})", tr(lang, sub))
                } else {
                    tr(lang, sub).to_string()
                };
                children.push(TreeNode::leaf(sl));
                self.cat_map.push((ci, Some(sub)));
            }
            if children.is_empty() {
                roots.push(TreeNode::leaf(label));
            } else {
                roots.push(TreeNode::branch(label, children)); // 기본 펼침
            }
        }
        let mut tree = TreeView::new(TreeModel::new(roots));
        tree.set_scale(self.scale);
        tree.set_focused(true); // 사이드바는 ↑↓ 상시 탐색(트리 자체 포커스 링 없음)
        let sel_row = self
            .cat_map
            .iter()
            .position(|&(c, sub)| c == self.selected_cat && sub == self.selected_sub)
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
                SettingKind::PositionGrid => {
                    let mut p = PositionPicker::new();
                    p.select_value(self.values.get(e.key).map_or("bl", String::as_str));
                    p.set_scale(self.scale);
                    RowCtl::Pos(p)
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
        let sw = self.s(self.sidebar_w);
        let b = self.bounds;
        Rect::new(b.x + sw, b.y, (b.w - sw).max(0), b.h)
    }

    /// 스크롤바 페이드 틱(호스트 ~5Hz) — 표시가 바뀌면 `true`(재그리기).
    pub fn tick(&mut self) -> bool {
        self.bars.tick() || self.tree.tick()
    }

    /// 이 좌표에서 좌우 리사이즈 커서를 보여야 하는가 — 스플리터 hover/드래그
    /// (호스트가 OS 커서로 번역 · 사용자 요청 08-09: 조절 가능함을 직관적으로).
    #[must_use]
    pub fn wants_col_resize_cursor(&self, x: i32, y: i32) -> bool {
        if self.split_drag {
            return true;
        }
        let split_x = self.bounds.x + self.s(self.sidebar_w);
        (x - split_x).abs() <= self.s(4) && y >= self.bounds.y && y < self.bounds.bottom()
    }

    /// 현 bounds에 맞춰 자식 컨트롤 배치.
    fn layout(&mut self, inv: &mut Invalidations) {
        let sw = self.s(self.sidebar_w);
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
        let (hf, he, hp) = (self.s(FONT_SECTION_H), self.s(ENTRY_H), self.s(POS_ROW_H));
        self.content_h = self
            .rows
            .iter()
            .map(|row| match registry()[row.idx].kind {
                SettingKind::FontSection { .. } => hf,
                SettingKind::PositionGrid => hp,
                _ => he,
            })
            .sum();
        self.scroll = self.scroll.clamp(0, (self.content_h - b.h).max(0));
        let mut top = b.y - self.scroll;
        // 차용 분리를 위해 치수 사전 계산.
        let (ctl_h, pad) = (self.s(CTL_H), self.s(PAD));
        let (h_font, h_entry, h_pos) = (self.s(FONT_SECTION_H), self.s(ENTRY_H), self.s(POS_ROW_H));
        let (combo_w, check_w) = (self.s(COMBO_W), self.s(22));
        let (family_w, size_w, gap10, dy32) =
            (self.s(FAMILY_W), self.s(SIZE_W), self.s(10), self.s(32));
        for row in &mut self.rows {
            let e = &registry()[row.idx];
            let h = match e.kind {
                SettingKind::FontSection { .. } => h_font,
                SettingKind::PositionGrid => h_pos,
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
                RowCtl::Pos(p) => {
                    p.set_scale(self.scale);
                    let (pw, ph) = p.preferred_size();
                    p.set_bounds(
                        Rect::new(rx + rw - pw - pad, top + (h - ph) / 2, pw, ph),
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
                RowCtl::Pos(g) => {
                    if let Some(v) = g.take_changed() {
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

        // ── 사이드바 스플리터 드래그(폭 조절) ──
        {
            let bx = self.bounds.x;
            let split_x = bx + self.s(self.sidebar_w);
            match *ev {
                InputEvent::MouseDown { x, y, .. }
                    if (x - split_x).abs() <= self.s(4)
                        && y >= self.bounds.y
                        && y < self.bounds.bottom() =>
                {
                    self.split_drag = true;
                    return;
                }
                InputEvent::MouseMove { x, .. } if self.split_drag => {
                    let logical = ((x - bx) as f32 / self.scale).round() as i32;
                    let clamped = logical.clamp(110, 320);
                    if clamped != self.sidebar_w {
                        self.sidebar_w = clamped;
                        self.layout(inv);
                        inv.push(self.bounds);
                    }
                    return;
                }
                InputEvent::MouseUp { .. } if self.split_drag => {
                    self.split_drag = false;
                    return;
                }
                _ => {}
            }
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
                // ×(지우기) 클릭 처리 — 값이 지워지면 검색 해제 재구성.
                self.search.on_event(ev, inv);
                if self.search.take_changed().is_some() {
                    let q = self.search.text();
                    if q != self.query {
                        self.query = q;
                        self.rebuild(inv);
                        inv.push(self.bounds);
                        return;
                    }
                }
                for row in &mut self.rows {
                    match &mut row.ctl {
                        RowCtl::Font { family, .. } => {
                            family.set_focused(family.bounds().contains(p));
                        }
                        RowCtl::Pos(g) => g.set_focused(g.bounds().contains(p)),
                        _ => {}
                    }
                }
                // 사이드바 트리 — 선택 변경 감지 → 카테고리 전환(검색 해제).
                let before = self.tree.selected_row();
                self.tree.on_event(ev, inv);
                let after = self.tree.selected_row();
                if self.tree.bounds().contains(p) && after != before
                    || (self.tree.bounds().contains(p) && !self.query.is_empty())
                {
                    if let Some(&(ci, sub)) = self.cat_map.get(after) {
                        self.selected_cat = ci;
                        self.selected_sub = sub;
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
                        RowCtl::Pos(g) => g.on_event(ev, inv),
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
                Key::Left | Key::Right | Key::Up | Key::Down
                    if self
                        .rows
                        .iter()
                        .any(|r| matches!(&r.ctl, RowCtl::Pos(g) if g.is_focused())) =>
                {
                    for row in &mut self.rows {
                        if let RowCtl::Pos(g) = &mut row.ctl {
                            if g.is_focused() {
                                g.on_event(ev, inv);
                            }
                        }
                    }
                    self.drain_changes(inv);
                    inv.push(self.bounds);
                }
                Key::Up | Key::Down if self.query.is_empty() => {
                    // 사이드바 카테고리 탐색(검색 중엔 유지).
                    let before = self.tree.selected_row();
                    self.tree.on_event(ev, inv);
                    let after = self.tree.selected_row();
                    if after != before {
                        if let Some(&(ci, sub)) = self.cat_map.get(after) {
                            self.selected_cat = ci;
                            self.selected_sub = sub;
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
        let sw = self.s(self.sidebar_w);

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
                RowCtl::Combo(_) | RowCtl::Check(_) | RowCtl::Pos(_) => {
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
                RowCtl::Pos(g) => g.paint(ctx, theme),
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
            .position(|(c, _)| *c == cat)
            .unwrap();
        w.selected_cat = ci;
        w.selected_sub = None;
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
    fn subcategory_filters_vscode_style() {
        let (mut w, mut inv) = widget();
        // 최상위(모양) 선택 = 하위(타입어헤드) 항목 포함 전부.
        select_cat(&mut w, Msg::CatAppearance);
        let all = w.visible_indices().len();
        let ta = w
            .visible_indices()
            .iter()
            .filter(|&&i| registry()[i].sub == Some(Msg::CatTypeahead))
            .count();
        assert_eq!(ta, 4, "타입어헤드 4건 포함");
        assert!(all > ta, "모양 자체 항목도 함께");
        // 하위(타입어헤드) 선택 = 4건만.
        w.selected_sub = Some(Msg::CatTypeahead);
        w.rebuild(&mut inv);
        assert_eq!(w.visible_indices().len(), 4, "하위 선택 = 그 항목만");
        // 사이드바에 하위 행이 존재(모양 아래).
        assert!(
            w.cat_map.contains(&(1, Some(Msg::CatTypeahead))),
            "사이드바 하위 행"
        );
    }

    #[test]
    fn sidebar_splitter_drags_width() {
        let (mut w, mut inv) = widget();
        let sx = w.bounds.x + w.s(w.sidebar_w);
        let down = InputEvent::MouseDown {
            x: sx,
            y: 100,
            shift: false,
            primary: false,
        };
        w.on_event(&down, &mut inv);
        assert!(w.split_drag, "경계 클릭 = 드래그 시작");
        w.on_event(&InputEvent::MouseMove { x: sx + 60, y: 100 }, &mut inv);
        assert!(w.sidebar_w > SIDEBAR_W, "폭 확장");
        w.on_event(&InputEvent::MouseUp { x: sx + 60, y: 100 }, &mut inv);
        assert!(!w.split_drag);
        // 클램프 하한.
        w.on_event(
            &InputEvent::MouseDown {
                x: w.bounds.x + w.s(w.sidebar_w),
                y: 100,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        w.on_event(
            &InputEvent::MouseMove {
                x: w.bounds.x + 10,
                y: 100,
            },
            &mut inv,
        );
        assert_eq!(w.sidebar_w, 110, "하한 클램프");
        w.on_event(&InputEvent::MouseUp { x: 0, y: 100 }, &mut inv);
    }

    #[test]
    fn search_clear_button_resets_query() {
        let (mut w, mut inv) = widget();
        // 검색어 입력 → 필터.
        w.on_event(&ch('t'), &mut inv);
        assert!(!w.query.is_empty());
        // × 클릭 = 초기화 + 전체 복귀.
        let r = w.search.clear_rect();
        w.on_event(&click(r.x + 3, r.y + 3), &mut inv);
        assert!(w.query.is_empty(), "검색 해제");
    }

    #[test]
    fn escape_requests_close() {
        let (mut w, mut inv) = widget();
        assert!(!w.take_back());
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(w.take_back());
    }
}
