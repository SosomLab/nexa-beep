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
//! | on/off 설정 | [`Checkbox`](crate::controls::Checkbox) |
//! | 글꼴 영역 | [`TextBox`] 글꼴명 + [`Combo`] 크기 |
//!
//! 값 반영은 기존 계약 그대로 — **즉시 적용**([`SettingsWidget::take_changes`] 폴링), 영속은
//! M2-5(Repository 포트). i18n: 라벨은 [`Msg`] 키, 검색은 **전 언어 매치**.

use crate::controls::{
    Button, ColorPicker, Combo, ComboControl, ComboItem, Control, LabelSide, PositionPicker,
    ScrollBars, Switch, TextBox, TreeControl, TreeModel, TreeNode, TreeView,
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

/// [`SIZE_OPTS`]의 `Radio` kind용 정적 참조(컨트롤 크기 항목 재사용).
const SIZE_OPTS_STATIC: &[(&str, Msg)] = SIZE_OPTS;

/// 설정 **화면에는 없지만** 영속되는 키(M3-17 프로필 화면이 쓴다) — 기본 빈 문자열.
/// ⚠ 이메일·전화는 PII다 — 평문 settings.cfg 보관은 잠정이며 M2-5b(암호화 저장)로
/// 이관 후보(journal 08-11 명기).
const HIDDEN_KEYS: &[&str] = &[
    "profile.email",
    "profile.phone",
    "profile.image_path",
    "profile.avatar",        // 아바타 선택(08-14) — 프로필 화면 스와치가 편집한다
    "profile.avatar_border", // 아바타 보더 색(08-14) — 프로필 화면 ColorPick이 편집한다
    // 최근 프로필 이미지(08-14 — 탭 구분 목록). ★ 여기 없으면 저장은 되는데
    // **부팅 로드에서 미지 키로 무시**돼 재시작마다 목록이 증발한다(실기로 잡음).
    "profile.image_recent",
    // 창 위치·크기 기억(08-14) — Moved/Resized가 쓰고 기동이 읽는다.
    "ui.win_x",
    "ui.win_y",
    "ui.win_w",
    "ui.win_h",
];

/// 기본 off 토글 — 프로필 공개(DR-22 **기본 전부 비노출** · 옵트인). 미등록 토글은 on.
const TOGGLE_DEFAULT_OFF: &[&str] = &[
    "profile.share.basic",
    "profile.share.email",
    "profile.share.phone",
];

/// Radio 기본값 예외 — 표시 순서(오름차순 등)와 기본값이 다른 키만 등록.
/// 미등록 키의 기본은 첫 옵션(기존 규약).
const RADIO_DEFAULTS: &[(&str, &str)] = &[
    ("ui.toolbar_size", "32"),
    ("ui.typeahead_timeout", "2000"),
    ("ui.scrollbar_hide", "2000"),
    ("ui.tooltip_ms", "2000"),
    // 목록 갱신 주기 — 기본 1500ms(사용자 확정 08-14).
    ("ui.list_refresh_ms", "1500"),
    // 한글 입력(IME) 기준값 — 기본은 macOS 실측값(H-27 · 08-15).
    ("ime.stale_ms", "250"),
    ("ime.same_key_ms", "40"),
    ("ime.pending_ms", "150"),
    ("ime.echo_ms", "120"),
    ("ime.stash_ms", "300"),
    ("ime.owed_ms", "800"),
    ("ime.pre_clear_ms", "300"),
    ("ime.swallow_ms", "2000"),
    ("ime.selfcommit_ms", "1000"),
    // 컨트롤 글리프 크기 — 기본 "크게"(사용자 확정 08-11 · 설정 Switch가 크게 보이도록).
    ("ui.control_size", "l"),
];

/// 항목 종류 — 우측 패널이 이 열거를 읽어 컨트롤을 동적 생성한다(새 설정 = Entry 1줄).
#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    /// 값 후보 중 택일 — [`Combo`] 드롭다운.
    Radio(&'static [(&'static str, Msg)]),
    /// 택일 + **직접 입력** — 후보에 없는 값을 인라인 편집으로 넣는다(값, 표시 접미).
    RadioInput(&'static [(&'static str, Msg)], &'static str),
    /// 3×3 위치 그리드 — 미니 화면(4:3) 셀로 직관 선택([`PositionPicker`]).
    PositionGrid,
    /// 글꼴 **얼굴만** — 크기는 Base UI를 따른다(고정폭 슬롯).
    FontFace {
        /// 글꼴명 값 키.
        family_key: &'static str,
    },
    /// on/off — [`Checkbox`](crate::controls::Checkbox). 값은 `"on"`/`"off"`(기본 on).
    Toggle,
    /// 색상 — [`ColorPicker`](스와치 + `#RRGGBB` 입력 + 프리셋). 값 = `#RRGGBB`(08-10).
    Color {
        /// 기본 hex(테마 팔레트의 원값).
        default: &'static str,
    },
    /// 글꼴 영역 — 글꼴명 [`TextBox`] + 크기 [`Combo`].
    FontSection {
        /// 글꼴명 값 키(`font.{region}.family`).
        family_key: &'static str,
        /// 크기 값 키(`font.{region}.size`).
        size_key: &'static str,
    },
    /// 실행 버튼 — 값이 아니라 **행위**(백업·복원 등). 클릭 = `(key, "run")` 변경 방출.
    /// 값 키가 없어(`default_values` 빈 목록) 영속 파일에 실리지 않는다.
    Action {
        /// 버튼 라벨.
        verb: Msg,
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
            SettingKind::Toggle => {
                // 프로필 공개는 **기본 비노출**(DR-22 — 옵트인). 그 외 토글은 기본 on.
                let on = !TOGGLE_DEFAULT_OFF.contains(&self.key);
                vec![(self.key, if on { "on" } else { "off" }.to_string())]
            }
            SettingKind::Color { default } => vec![(self.key, default.to_string())],
            SettingKind::PositionGrid => vec![(self.key, "bl".to_string())],
            SettingKind::FontFace { family_key } => vec![(family_key, String::new())],
            SettingKind::FontSection {
                family_key,
                size_key,
            } => vec![
                (family_key, String::new()), // 빈 문자열 = 시스템 기본 글꼴
                (size_key, SIZE_DEFAULT.to_string()),
            ],
            // 행위 항목은 값이 없다 — 영속·검증 대상에서 자연히 빠진다.
            SettingKind::Action { .. } => vec![],
        }
    }
}

/// 설정 레지스트리 — **실존 설정만**. 렌더·검색·기본값이 전부 여기서 나온다.
#[must_use]
pub fn registry() -> &'static [Entry] {
    &[
        // 프로필 — 표시 이름(M1-10 · FR-S-50). "auto" = 정제된 호스트명(실명 제거 ·
        // 실패 시 지문 라벨). 직접 입력 = 옵트인 실명 — desc가 LAN 평문 방송을 고지한다.
        Entry {
            cat: Msg::CatProfile,
            sub: None,
            label: Msg::DisplayNameLabel,
            desc: Msg::DisplayNameDesc,
            kind: SettingKind::RadioInput(&[("auto", Msg::NameAuto)], ""),
            key: "profile.display_name",
        },
        // 프로필 공개(DR-22 옵트인 · ADR-0008) — 기본 전부 off. 값 교환은 세션 경유
        // (브로드캐스트 미포함) — 교환 프로토콜은 프로필 슬라이스(M3-17)에서.
        Entry {
            cat: Msg::CatProfile,
            sub: None,
            label: Msg::ShareBasic,
            desc: Msg::ShareBasicDesc,
            kind: SettingKind::Toggle,
            key: "profile.share.basic",
        },
        Entry {
            cat: Msg::CatProfile,
            sub: None,
            label: Msg::ShareEmail,
            desc: Msg::ShareEmailDesc,
            kind: SettingKind::Toggle,
            key: "profile.share.email",
        },
        Entry {
            cat: Msg::CatProfile,
            sub: None,
            label: Msg::SharePhone,
            desc: Msg::SharePhoneDesc,
            kind: SettingKind::Toggle,
            key: "profile.share.phone",
        },
        // 신원 키 백업·복원(M2-5a · 사용자 요청 08-11) — 값이 아니라 행위.
        Entry {
            cat: Msg::CatProfile,
            sub: None,
            label: Msg::IdBackup,
            desc: Msg::IdBackupDesc,
            kind: SettingKind::Action {
                verb: Msg::ActBackup,
            },
            key: "profile.identity.backup",
        },
        Entry {
            cat: Msg::CatProfile,
            sub: None,
            label: Msg::IdRestore,
            desc: Msg::IdRestoreDesc,
            kind: SettingKind::Action {
                verb: Msg::ActRestore,
            },
            key: "profile.identity.restore",
        },
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
            cat: Msg::CatConversation,
            sub: None,
            label: Msg::Time24h,
            desc: Msg::Time24hDesc,
            kind: SettingKind::Toggle,
            key: "chat.time_24h",
        },
        Entry {
            cat: Msg::CatConversation,
            sub: None,
            label: Msg::DateFormat,
            desc: Msg::DateFormatDesc,
            kind: SettingKind::Radio(&[
                ("iso", Msg::DateFormatIso),
                ("short", Msg::DateFormatShort),
            ]),
            key: "chat.date_format",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: None,
            label: Msg::Theme,
            desc: Msg::ThemeDesc,
            kind: SettingKind::Radio(&[("dark", Msg::ThemeDark), ("light", Msg::ThemeLight)]),
            key: "ui.theme",
        },
        // ── 테마 주요 색(08-10 · 사용자 요청) — 다크/라이트 각각. 즉시 적용(영속은 M3-15). ──
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorAccent,
            desc: Msg::ColorAccentDesc,
            kind: SettingKind::Color { default: "#3D8BFF" },
            key: "theme.dark.accent",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorBubblePeer,
            desc: Msg::ColorBubblePeerDesc,
            kind: SettingKind::Color { default: "#313947" },
            key: "theme.dark.bubble_peer",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorPanelBg,
            desc: Msg::ColorPanelBgDesc,
            kind: SettingKind::Color { default: "#191C21" },
            key: "theme.dark.panel_bg",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorText,
            desc: Msg::ColorTextDesc,
            kind: SettingKind::Color { default: "#D6DAE0" },
            key: "theme.dark.text",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorAccent,
            desc: Msg::ColorAccentDesc,
            kind: SettingKind::Color { default: "#3D8BFF" },
            key: "theme.light.accent",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorBubblePeer,
            desc: Msg::ColorBubblePeerDesc,
            kind: SettingKind::Color { default: "#E2E7EE" },
            key: "theme.light.bubble_peer",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorPanelBg,
            desc: Msg::ColorPanelBgDesc,
            kind: SettingKind::Color { default: "#FFFFFF" },
            key: "theme.light.panel_bg",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorText,
            desc: Msg::ColorTextDesc,
            kind: SettingKind::Color { default: "#1B1F26" },
            key: "theme.light.text",
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
        // 컨트롤 글리프 크기(체크·스위치·옵션박스 — 08-11 사용자 요청 · 기본 "크게").
        Entry {
            cat: Msg::CatAppearance,
            sub: None,
            label: Msg::ControlSize,
            desc: Msg::ControlSizeDesc,
            kind: SettingKind::Radio(SIZE_OPTS_STATIC),
            key: "ui.control_size",
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
                ("48", Msg::Tb48),
                ("64", Msg::Tb64),
            ]),
            key: "ui.toolbar_size",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: None,
            label: Msg::CarouselScroll,
            desc: Msg::CarouselScrollDesc,
            kind: SettingKind::Radio(&[
                ("auto", Msg::ScrollOsDefault),
                ("fwd", Msg::ScrollForward),
                ("rev", Msg::ScrollNatural),
            ]),
            key: "ui.carousel_scroll",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: None,
            label: Msg::TooltipDelay,
            desc: Msg::TooltipDelayDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("1000", Msg::TaSec1),
                    ("2000", Msg::TaSec2),
                    ("3000", Msg::TaSec3),
                    ("5000", Msg::TaSec5),
                ],
                "ms",
            ),
            key: "ui.tooltip_ms",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: None,
            label: Msg::ScrollbarHide,
            desc: Msg::ScrollbarHideDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("0", Msg::ScrollbarHideNever),
                    ("1000", Msg::TaSec1),
                    ("2000", Msg::TaSec2),
                    ("3000", Msg::TaSec3),
                    ("5000", Msg::TaSec5),
                    ("10000", Msg::TaSec10),
                ],
                "ms",
            ),
            key: "ui.scrollbar_hide",
        },
        // ── 목록 보기(08-14 사용자 확정) — 갱신 주기 + 갱신 시 스크롤 동작 ──
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatPeerList),
            label: Msg::ListRefresh,
            desc: Msg::ListRefreshDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("500", Msg::Ms500),
                    ("1000", Msg::TaSec1),
                    ("1500", Msg::Ms1500),
                    ("3000", Msg::TaSec3),
                ],
                "ms",
            ),
            key: "ui.list_refresh_ms",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatPeerList),
            label: Msg::ListSort,
            desc: Msg::ListSortDesc,
            kind: SettingKind::Radio(&[
                ("chat", Msg::SortChat),
                ("name", Msg::SortName),
                ("seen", Msg::SortSeen),
                ("online", Msg::SortOnline),
            ]),
            key: "ui.list_sort",
        },
        Entry {
            cat: Msg::CatAppearance,
            sub: Some(Msg::CatPeerList),
            label: Msg::ListScroll,
            desc: Msg::ListScrollDesc,
            kind: SettingKind::Radio(&[
                ("keep", Msg::ListScrollKeep),
                ("caret", Msg::ListScrollCaret),
                ("top", Msg::ListScrollTop),
            ]),
            key: "ui.list_refresh_scroll",
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
            label: Msg::FontMono,
            desc: Msg::FontMonoDesc,
            kind: SettingKind::FontFace {
                family_key: "font.mono.family",
            },
            key: "font.mono.family",
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
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::XferTimeout,
            desc: Msg::XferTimeoutDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("60", Msg::Sec60),
                    ("30", Msg::Sec30),
                    ("120", Msg::Sec120),
                    ("300", Msg::Sec300),
                ],
                "초",
            ),
            key: "xfer.timeout_sec",
        },
        // 네트워크 — 세션 수신 포트(DR-19 · ADR-0006 §3-1). **듣는 포트이자 주소 입력에서
        // 포트를 생략했을 때 거는 기본 포트**(하나의 값 — 사용자 확정 08-13 ⓐ: 조직이 같은
        // 값을 쓰면 IP만으로 서로 붙는다). 값 검증(1~65535)은 소비처가 관용 파싱 —
        // 무효·범위 밖은 기본 47200으로 본다.
        Entry {
            cat: Msg::CatNetwork,
            sub: None,
            label: Msg::SessionPort,
            desc: Msg::SessionPortDesc,
            kind: SettingKind::RadioInput(&[("47200", Msg::PortDefault)], ""),
            key: "net.session_port",
        },
        // 그룹(M5-1 · ADR-0012) — 재동기 보관 주체 = 송신자(사용자 확정 08-13).
        // 발신자가 구성원별로 미전달 그룹 메시지를 몇 개까지 보관할지(초과 = 오래된 것
        // 폐기 — 큐 상한 필수 NFR-B-6). 소비처(app)가 관용 파싱한다.
        // 구성원 초대 허용(ADR-0012 정책 · 사용자 확정 08-13) — **새 방의 기본값**.
        // 방별 변경은 그룹 행 우클릭(소유자) — 여기 값은 생성 시점에만 복사된다.
        Entry {
            cat: Msg::CatGroup,
            sub: None,
            label: Msg::GroupMemberInvite,
            desc: Msg::GroupMemberInviteDesc,
            kind: SettingKind::Toggle,
            key: "group.member_invite",
        },
        Entry {
            cat: Msg::CatGroup,
            sub: None,
            label: Msg::GroupResyncKeep,
            desc: Msg::GroupResyncKeepDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("200", Msg::Count200),
                    ("50", Msg::Count50),
                    ("1000", Msg::Count1000),
                ],
                "개",
            ),
            key: "group.resync_keep",
        },
        // ── 한글 입력(IME) — 게이트 기준값 일습(08-15 사용자 요청 · H-27) ──
        // 기본값은 macOS 실측으로 굳힌 값. 경합 양상은 기계·IME 버전마다 달라
        // 현장 조정이 필요할 수 있다(입력도 추정 금지·실측 필수 — docs/34).
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeInject,
            desc: Msg::ImeInjectDesc,
            kind: SettingKind::Toggle,
            key: "ime.inject",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeLeak,
            desc: Msg::ImeLeakDesc,
            kind: SettingKind::Toggle,
            key: "ime.leak",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeStale,
            desc: Msg::ImeStaleDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("150", Msg::Ms150),
                    ("250", Msg::Ms250),
                    ("400", Msg::Ms400),
                    ("800", Msg::Ms800),
                ],
                "ms",
            ),
            key: "ime.stale_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeSameKey,
            desc: Msg::ImeSameKeyDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("20", Msg::Ms20),
                    ("40", Msg::Ms40),
                    ("80", Msg::Ms80),
                    ("120", Msg::Ms120),
                ],
                "ms",
            ),
            key: "ime.same_key_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImePending,
            desc: Msg::ImePendingDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("80", Msg::Ms80),
                    ("150", Msg::Ms150),
                    ("250", Msg::Ms250),
                    ("400", Msg::Ms400),
                ],
                "ms",
            ),
            key: "ime.pending_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeEcho,
            desc: Msg::ImeEchoDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("80", Msg::Ms80),
                    ("120", Msg::Ms120),
                    ("200", Msg::Ms200),
                    ("300", Msg::Ms300),
                ],
                "ms",
            ),
            key: "ime.echo_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeStash,
            desc: Msg::ImeStashDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("150", Msg::Ms150),
                    ("300", Msg::Ms300),
                    ("500", Msg::Ms500),
                    ("800", Msg::Ms800),
                ],
                "ms",
            ),
            key: "ime.stash_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeOwed,
            desc: Msg::ImeOwedDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("400", Msg::Ms400),
                    ("800", Msg::Ms800),
                    ("1600", Msg::Ms1600),
                ],
                "ms",
            ),
            key: "ime.owed_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImePreClear,
            desc: Msg::ImePreClearDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("150", Msg::Ms150),
                    ("300", Msg::Ms300),
                    ("500", Msg::Ms500),
                ],
                "ms",
            ),
            key: "ime.pre_clear_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeSwallow,
            desc: Msg::ImeSwallowDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("1000", Msg::TaSec1),
                    ("2000", Msg::TaSec2),
                    ("3000", Msg::TaSec3),
                ],
                "ms",
            ),
            key: "ime.swallow_ms",
        },
        Entry {
            cat: Msg::CatIme,
            sub: None,
            label: Msg::ImeSelfcommit,
            desc: Msg::ImeSelfcommitDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("500", Msg::Ms500),
                    ("1000", Msg::TaSec1),
                    ("2000", Msg::TaSec2),
                ],
                "ms",
            ),
            key: "ime.selfcommit_ms",
        },
        // ── 고급(08-15 사용자 요청) — 설정 백업·복원·초기화(값이 아니라 행위) ──
        Entry {
            cat: Msg::CatAdvanced,
            sub: None,
            label: Msg::SetBackup,
            desc: Msg::SetBackupDesc,
            kind: SettingKind::Action {
                verb: Msg::ActBackup,
            },
            key: "settings.backup",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: None,
            label: Msg::SetRestore,
            desc: Msg::SetRestoreDesc,
            kind: SettingKind::Action {
                verb: Msg::ActRestore,
            },
            key: "settings.restore",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: None,
            label: Msg::SetReset,
            desc: Msg::SetResetDesc,
            kind: SettingKind::Action {
                verb: Msg::ActReset,
            },
            key: "settings.reset",
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
        for k in HIDDEN_KEYS {
            values.insert(*k, String::new());
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

    /// 저장 스냅샷 — 전체 (키, 값) 쌍을 **키 정렬**로(직렬화가 결정적이어야
    /// "직전 저장분과 같으면 쓰지 않는다"(ADR-0011 S-3) 비교가 성립한다).
    #[must_use]
    pub fn known_pairs(&self) -> Vec<(&'static str, &str)> {
        let mut pairs: Vec<_> = self.values.iter().map(|(k, v)| (*k, v.as_str())).collect();
        pairs.sort_unstable_by_key(|(k, _)| *k);
        pairs
    }

    /// 파일에서 읽은 (키, 값)을 적용한다 — **아는 키만**, 값은 Kind별 관용 검증
    /// (ADR-0011 §4-3: 거부·실패가 아니라 무시 = 기본값 유지). 반환 = 아는 키였는가
    /// (거짓이면 호출자가 미지 키로 보존한다 — F-1).
    pub fn set_by_name(&mut self, key: &str, value: &str) -> bool {
        // 화면 밖 영속 키(M3-17 프로필 필드) — 자유 문자열 그대로.
        if let Some(k) = HIDDEN_KEYS.iter().find(|k| **k == key) {
            self.values.insert(k, value.to_string());
            return true;
        }
        // &'static str 키는 레지스트리에서 찾는다(default_values가 파생 키 포함 전부).
        let mut found: Option<&'static str> = None;
        let mut kind: Option<SettingKind> = None;
        'outer: for e in registry() {
            for (k, _) in e.default_values() {
                if k == key {
                    found = Some(k);
                    // FontSection의 size 파생 키는 Radio류가 아니므로 kind 검증에서
                    // family/size를 구분한다 — 아래 검증 참조.
                    kind = Some(e.kind);
                    break 'outer;
                }
            }
        }
        let (Some(k), Some(kind)) = (found, kind) else {
            return false;
        };
        let valid = match kind {
            SettingKind::Radio(opts) => opts.iter().any(|(v, _)| *v == value),
            // 직접 입력 허용 — 빈 값만 거른다(빈 문자열은 기본값 의미가 아니다).
            SettingKind::RadioInput(..) => !value.is_empty(),
            SettingKind::Toggle => value == "on" || value == "off",
            SettingKind::Color { .. } => crate::theme::color_from_hex(value).is_some(),
            // 위치 코드·글꼴명(빈 값 = 시스템 기본)·크기 코드는 소비처가 관용 파싱한다.
            SettingKind::PositionGrid | SettingKind::FontFace { .. } => true,
            SettingKind::FontSection { .. } => true,
            // 행위 항목은 값이 없다 — 파일에서 와도 무시(default_values가 비어 도달 불가).
            SettingKind::Action { .. } => false,
        };
        if valid {
            self.values.insert(k, value.to_string());
        }
        true // 아는 키다 — 값이 무효여도 미지 키로 보존하지 않는다(기본값 유지).
    }
}

/// 검색어 → 소문자 토큰(공백 구분 **AND 매칭** — VS Code 규약).
fn tokens(q: &str) -> Vec<String> {
    q.split_whitespace().map(str::to_lowercase).collect()
}

/// 설명 워드랩(08-11) — `avail`(물리 px) 안에서 그리디 줄바꿈. 공백 없는 긴 조각
/// (CJK 문장 등)은 문자 단위로 쪼갠다. `max_lines` 초과분은 마지막 줄 끝을 `…`로 접는다
/// (예약 줄 수는 레이아웃의 추정 — 실측이 넘치면 자르는 쪽이 침범보다 낫다).
pub(crate) fn wrap_text(
    ctx: &mut dyn DrawCtx,
    text: &str,
    avail: i32,
    max_lines: usize,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let cand = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if ctx.text_width(&cand) <= avail {
            cur = cand;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        if ctx.text_width(word) > avail {
            for ch in word.chars() {
                let cand = format!("{cur}{ch}");
                if cur.is_empty() || ctx.text_width(&cand) <= avail {
                    cur = cand;
                } else {
                    lines.push(std::mem::take(&mut cur));
                    cur = ch.to_string();
                }
            }
        } else {
            cur = word.to_string();
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines.max(1));
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && ctx.text_width(&format!("{last}…")) > avail {
                last.pop();
            }
            last.push('…');
        }
    }
    lines
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
/// 설정 행에 붙는 정보 줄 높이(논리 px).
const NOTE_H: i32 = 22;
/// 설명 워드랩 줄 높이(논리 px — Status 폰트 한 줄 + 행간).
const DESC_LINE_H: i32 = 16;
/// 위치 그리드 행 높이(3×3 미니 화면 93 + 여백).
const POS_ROW_H: i32 = 110;
const CTL_H: i32 = 26;
const COMBO_W: i32 = 170;
const SIZE_W: i32 = 112;
const FAMILY_W: i32 = 180;
const PAD: i32 = 12;
/// 스크롤 영역 안의 하위 섹션 제목 높이 — **위쪽 여백을 크게** 둬서 앞 그룹과 확실히 끊는다
/// (사용자 지적 08-11: 그룹 경계가 눈에 잘 안 띈다). 제목 글자는 이 상자의 **아래쪽**에 붙는다.
const SUB_HEAD_H: i32 = 52;
/// 하위 제목 상자에서 글자 아래 여백 — 제목이 자기 그룹 첫 행에 가깝게 붙게 한다.
const SUB_HEAD_PAD_B: i32 = 8;
/// 상단 고정 밴드 — 상위 제목 줄 + 하위 제목 줄(하위가 없으면 아랫줄은 비워 둔다).
/// **높이를 고정**해야 그룹을 넘나들 때 내용이 위아래로 튀지 않는다.
const CRUMB_CAT_H: i32 = 30;
const CRUMB_SUB_H: i32 = 24;

/// 우측 한 행 = 레지스트리 항목 + 실물 컨트롤.
#[derive(Debug)]
enum RowCtl {
    Combo(Combo),
    /// on/off 토글 — mac(iOS) 스타일 [`Switch`](08-11 · 기존 Checkbox에서 교체).
    Check(Switch),
    /// 실행 버튼(백업·복원 등 행위 항목).
    Act(Button),
    Font {
        family: TextBox,
        size: Combo,
    },
    /// 3×3 위치 그리드.
    Pos(PositionPicker),
    /// 글꼴 **얼굴만**(고정폭 — 크기는 Base UI를 따른다).
    Face(TextBox),
    /// 색상(스와치 + hex + 프리셋 · 08-10).
    Color(ColorPicker),
}

#[derive(Debug)]
struct RowUi {
    /// registry 인덱스.
    idx: usize,
    /// 행 영역(우측 패널 안 · 물리 px).
    rect: Rect,
    ctl: RowCtl,
    /// 이 행이 속한 그룹 `(상위, 하위)` — 상단 고정 밴드가 무엇을 보여줄지 정한다.
    group: (Msg, Option<Msg>),
    /// 이 행 **위에** 그릴 하위 섹션 제목(그룹의 첫 행에만). 상위 직속 구간은 `None`
    /// (상위 제목은 스크롤되지 않는 밴드가 늘 보여주므로 본문에 또 적지 않는다).
    head: Option<Msg>,
    /// 헤더까지 포함한 이 행의 시작 y(레이아웃이 채운다) — 밴드 판정에 쓴다.
    head_h: i32,
    /// 설명에 예약된 줄 수(1~3 · 레이아웃이 추정) — 워드랩이 이 안에서 그린다(08-11).
    desc_lines: i32,
    /// 설명 워드랩 가용 폭(물리 px — 컨트롤 왼쪽까지). 레이아웃·페인트가 같은 값을 쓴다.
    desc_avail: i32,
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
    /// 시스템 기본 폰트 표시 이름(placeholder 식별 — 비면 이름 생략).
    default_base_name: String,
    /// 시스템 고정폭 폰트 표시 이름.
    default_mono_name: String,
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
    /// 비활성 설정 키(호스트가 지정) — 흐리게 그리고 입력을 받지 않는다.
    disabled: std::collections::HashSet<&'static str>,
    /// 특정 설정 행 **바로 아래**에 붙는 한 줄 정보(자리 고정 — 호스트가 채운다).
    notes: HashMap<&'static str, String>,
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
            default_base_name: String::new(),
            default_mono_name: String::new(),
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
            disabled: std::collections::HashSet::new(),
            notes: HashMap::new(),
        };
        let mut inv = Invalidations::default();
        w.rebuild(&mut inv);
        w
    }

    /// 선택 복사(① 08-13) — 포커스된 텍스트 입력(검색·글꼴명)에서만 나온다.
    #[must_use]
    pub fn clipboard_copy(&self) -> Option<String> {
        if let Some(t) = self.search.copy_selection() {
            return Some(t);
        }
        self.rows.iter().find_map(|r| match &r.ctl {
            RowCtl::Font { family, .. } | RowCtl::Face(family) => family.copy_selection(),
            _ => None,
        })
    }

    /// 선택 잘라내기(①).
    pub fn clipboard_cut(&mut self, inv: &mut Invalidations) -> Option<String> {
        if let Some(t) = self.search.cut_selection(inv) {
            self.sync_query(inv);
            return Some(t);
        }
        self.rows.iter_mut().find_map(|r| match &mut r.ctl {
            RowCtl::Font { family, .. } | RowCtl::Face(family) => family.cut_selection(inv),
            _ => None,
        })
    }

    /// 붙여넣기(①) — 포커스된 텍스트 입력만 받는다.
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        self.search.paste(text, inv);
        self.sync_query(inv);
        for r in &mut self.rows {
            if let RowCtl::Font { family, .. } | RowCtl::Face(family) = &mut r.ctl {
                family.paste(text, inv);
            }
        }
    }

    /// 우클릭 편집 메뉴 행동(1회성 — 08-13 전수 검사) — 어느 텍스트 입력에서든.
    pub fn take_edit_ctx(&mut self) -> Option<crate::controls::EditCtxAction> {
        if let Some(a) = self.search.take_edit_ctx() {
            return Some(a);
        }
        self.rows.iter_mut().find_map(|r| match &mut r.ctl {
            RowCtl::Font { family, .. } | RowCtl::Face(family) => family.take_edit_ctx(),
            _ => None,
        })
    }

    /// 클립보드 텍스트 유무 주입(우클릭 시점 — 붙여넣기 항목 활성 근거).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.search.set_clipboard_has_text(yes);
        for r in &mut self.rows {
            if let RowCtl::Font { family, .. } | RowCtl::Face(family) = &mut r.ctl {
                family.set_clipboard_has_text(yes);
            }
        }
    }

    /// 검색 텍스트가 코드 경로(잘라내기·붙여넣기)로 바뀌었으면 결과를 재구성한다.
    fn sync_query(&mut self, inv: &mut Invalidations) {
        let q = self.search.text();
        if q != self.query {
            self.query = q;
            self.rebuild(inv);
        }
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
    ///
    /// **그룹 순서로 정렬해서 돌려준다** — 상위에 직속인 설정이 먼저, 그다음 하위 그룹이
    /// 사이드바에 보이는 순서대로 이어진다(사용자 확정 08-10). registry 순서를 그대로
    /// 쓰면 "다크 색 → 라이트 색 → 언어 → 타입어헤드"처럼 섞여 나와, 지금 보는 값이
    /// 어느 그룹의 것인지 화면만 봐서는 알 수 없다. 그룹 안에서는 registry 순서를 지킨다.
    fn visible_indices(&self) -> Vec<usize> {
        let toks = tokens(&self.query);
        let searching = !toks.is_empty();
        let selected = Self::cats().get(self.selected_cat).map(|(c, _)| *c);
        let mut hits: Vec<usize> = registry()
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
            .collect();
        // 정렬 키 = (상위 순서, 하위 순서). 직속(sub=None)은 하위보다 **먼저**(=0).
        let cats = Self::cats();
        let key = |idx: &usize| -> (usize, usize) {
            let e = &registry()[*idx];
            let ci = cats
                .iter()
                .position(|(c, _)| *c == e.cat)
                .unwrap_or(usize::MAX);
            let si = match e.sub {
                None => 0,
                Some(sub) => cats
                    .get(ci)
                    .and_then(|(_, subs)| subs.iter().position(|s| *s == sub))
                    .map_or(usize::MAX, |p| p + 1),
            };
            (ci, si)
        };
        // 안정 정렬 — 같은 그룹 안에서는 registry 순서가 그대로 남는다.
        hits.sort_by_key(key);
        hits
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
                SettingKind::FontFace { family_key } => {
                    // 기본이 **무엇인지** 보여 준다(사용자 지적 08-10 — "(시스템 기본)"만으로는
                    // 식별 불가). 고정폭 행이므로 고정폭 기본 이름.
                    let ph = if self.default_mono_name.is_empty() {
                        tr(lang, Msg::SystemDefaultFont).to_string()
                    } else {
                        format!(
                            "{} {}",
                            self.default_mono_name,
                            tr(lang, Msg::SystemDefaultFont)
                        )
                    };
                    let mut family = TextBox::new(ph)
                        .with_text(self.values.get(family_key).map_or("", String::as_str));
                    family.set_scale(self.scale);
                    RowCtl::Face(family)
                }
                SettingKind::PositionGrid => {
                    let mut p = PositionPicker::new();
                    p.select_value(self.values.get(e.key).map_or("bl", String::as_str));
                    p.set_scale(self.scale);
                    RowCtl::Pos(p)
                }
                SettingKind::Color { default } => {
                    let mut c =
                        ColorPicker::new(self.values.get(e.key).map_or(default, String::as_str));
                    c.set_scale(self.scale);
                    RowCtl::Color(c)
                }
                SettingKind::Toggle => {
                    // mac(iOS) 스타일 스위치(08-11 사용자 요청) — 라벨은 행 왼쪽 제목이
                    // 이미 있으므로 토글만([`LabelSide::None`]).
                    let mut c =
                        Switch::new("", self.values.get(e.key).map(String::as_str) == Some("on"))
                            .with_label_side(LabelSide::None);
                    c.set_scale(self.scale);
                    RowCtl::Check(c)
                }
                SettingKind::Action { verb } => {
                    let mut b = Button::new(tr(lang, verb));
                    b.set_scale(self.scale);
                    RowCtl::Act(b)
                }
                SettingKind::FontSection {
                    family_key,
                    size_key,
                } => {
                    let ph = if self.default_base_name.is_empty() {
                        tr(lang, Msg::SystemDefaultFont).to_string()
                    } else {
                        format!(
                            "{} {}",
                            self.default_base_name,
                            tr(lang, Msg::SystemDefaultFont)
                        )
                    };
                    let mut family = TextBox::new(ph)
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
            // 그룹이 바뀌는 첫 행에만 하위 섹션 제목을 붙인다(상위 제목은 고정 밴드 몫).
            let group = (e.cat, e.sub);
            let head = match (self.rows.last().map(|r| r.group), e.sub) {
                (_, None) => None,
                (Some(prev), Some(sub)) if prev == group => {
                    let _ = sub;
                    None
                }
                (_, Some(sub)) => Some(sub),
            };
            self.rows.push(RowUi {
                idx,
                rect: Rect::default(),
                ctl,
                group,
                head,
                head_h: 0,
                desc_lines: 1,
                desc_avail: 0,
            });
        }
        self.layout(inv);
    }

    /// 값을 외부에서 갱신한다(예: 기간 만료로 승인 방식이 되돌아갔을 때) —
    /// 화면과 실제가 어긋나지 않게 콤보 표시까지 맞춘다.
    pub fn set_value(&mut self, key: &'static str, value: &str, inv: &mut Invalidations) {
        self.values.insert(key, value.to_string());
        for row in &mut self.rows {
            if registry()[row.idx].key != key {
                continue;
            }
            if let RowCtl::Combo(c) = &mut row.ctl {
                c.select_value(value);
            }
        }
        inv.push(self.bounds);
    }

    /// 비활성 키 지정 — 조건부로만 쓰이는 설정을 흐리게 잠근다(예: 기간은 "기간 자동"일 때만).
    pub fn set_disabled(&mut self, keys: &[&'static str], inv: &mut Invalidations) {
        let next: std::collections::HashSet<&'static str> = keys.iter().copied().collect();
        if next != self.disabled {
            self.disabled = next;
            inv.push(self.bounds);
        }
    }

    /// 설정 행 아래 한 줄 정보 지정 — **자리가 고정**된다(빈 문자열 = 제거).
    /// 값이 바뀔 때만 재배치·무효화하므로 1초 갱신에도 낭비가 없다.
    pub fn set_row_note(&mut self, key: &'static str, text: &str, inv: &mut Invalidations) {
        let had = self.notes.contains_key(key);
        if self.notes.get(key).map(String::as_str) == Some(text) || (text.is_empty() && !had) {
            return;
        }
        if text.is_empty() {
            self.notes.remove(key);
        } else {
            self.notes.insert(key, text.to_string());
        }
        if had != self.notes.contains_key(key) {
            self.layout(inv); // 줄이 생기거나 사라지면 행 높이가 달라진다
        }
        inv.push(self.bounds);
    }

    /// 이 행에 붙은 정보 줄 높이(없으면 0).
    fn note_h(&self, idx: usize) -> i32 {
        if self.notes.contains_key(registry()[idx].key) {
            self.s(NOTE_H)
        } else {
            0
        }
    }

    /// 이 행이 잠겼는가.
    fn is_locked(&self, idx: usize) -> bool {
        self.disabled.contains(registry()[idx].key)
    }

    /// 상단 고정 밴드(상위 + 하위 제목) 높이 — 하위가 없어도 **줄어들지 않는다**.
    /// 그룹 경계를 넘을 때 아래 내용이 위아래로 튀면 읽던 자리를 잃는다.
    fn crumb_h(&self) -> i32 {
        self.s(CRUMB_CAT_H) + self.s(CRUMB_SUB_H)
    }

    /// 우측 패널 뷰포트(사이드바 제외 · **고정 밴드 아래**부터).
    fn right_viewport(&self) -> Rect {
        let sw = self.s(self.sidebar_w);
        let b = self.bounds;
        let top = b.y + self.crumb_h();
        Rect::new(b.x + sw, top, (b.w - sw).max(0), (b.bottom() - top).max(0))
    }

    /// 스크롤 위치 기준으로 지금 보이는 그룹 `(상위, 하위)` — 고정 밴드가 이걸 그린다.
    /// 뷰포트 맨 위에 걸친 행의 그룹을 쓴다(그 행이 곧 사용자가 지금 읽는 것).
    fn current_group(&self) -> Option<(Msg, Option<Msg>)> {
        let vp = self.right_viewport();
        self.rows
            .iter()
            .find(|r| r.rect.bottom() > vp.y)
            .or_else(|| self.rows.last())
            .map(|r| r.group)
    }

    /// 스크롤바 자동숨김 틱 — 표시가 바뀌면 `true`(재그리기). `now_ms`는 호스트 시계.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        // `||`는 단축 평가라 트리 바가 안 돌 수 있다 — 둘 다 재워야 한다.
        self.bars.tick(now_ms) | self.tree.tick(now_ms)
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
        // 차용 분리를 위해 치수 사전 계산.
        let (ctl_h, pad) = (self.s(CTL_H), self.s(PAD));
        let (h_font, h_entry, h_pos) = (self.s(FONT_SECTION_H), self.s(ENTRY_H), self.s(POS_ROW_H));
        // 토글 폭 = Switch 트랙(20) × 컨트롤 크기 배율(ui.control_size).
        let (combo_w, check_w) = (self.s(COMBO_W), self.s(crate::controls::ctl_size(20)));
        let (family_w, size_w, gap10, dy32) =
            (self.s(FAMILY_W), self.s(SIZE_W), self.s(10), self.s(32));
        let note_hs: Vec<i32> = self.rows.iter().map(|r| self.note_h(r.idx)).collect();
        let lang = current_lang();
        let scale = self.scale;
        let desc_line_h = self.s(DESC_LINE_H);
        let min_avail = self.s(60);
        // 콘텐츠 총 높이 → 스크롤 클램프(행 추가/검색으로 줄어들면 위로 당긴다).
        // ★ **설명 워드랩 예약분 포함**(08-15 실기 — 이걸 빼고 합산하면 IME처럼
        // 2줄 설명이 많은 카테고리에서 총높이가 과소평가돼 **끝까지 스크롤이 안 됐다**.
        // 아래 배치 루프와 같은 추정식을 써야 상한이 실제 끝과 일치한다).
        let head_h = self.s(SUB_HEAD_H);
        self.content_h = self
            .rows
            .iter()
            .enumerate()
            .map(|(ri, row)| {
                let e = &registry()[row.idx];
                let base = match e.kind {
                    SettingKind::FontSection { .. } => h_font,
                    SettingKind::PositionGrid => h_pos,
                    _ => h_entry,
                };
                let ctl_w = match &row.ctl {
                    RowCtl::Combo(_) | RowCtl::Act(_) => combo_w,
                    RowCtl::Check(_) => check_w,
                    RowCtl::Face(_) => family_w,
                    RowCtl::Pos(p) => p.preferred_size().0,
                    RowCtl::Color(c) => c.preferred_width().min(rw - pad * 2),
                    RowCtl::Font { .. } => 0,
                };
                let desc_avail = (rw - pad * 2 - ctl_w - gap10).max(min_avail);
                let est_logical: i32 = tr(lang, e.desc)
                    .chars()
                    .map(|c| if c.is_ascii() { 7 } else { 14 })
                    .sum();
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                let est_px = (est_logical as f32 * scale).round() as i32;
                let desc_lines = ((est_px + desc_avail - 1) / desc_avail).clamp(1, 3);
                base + (desc_lines - 1) * desc_line_h
                    + note_hs[ri]
                    + if row.head.is_some() { head_h } else { 0 }
            })
            .sum();
        let vp_h = self.right_viewport().h;
        self.scroll = self.scroll.clamp(0, (self.content_h - vp_h).max(0));
        // 내용은 **밴드 아래**에서 시작한다(밴드가 첫 행을 가리면 못 만진다).
        let mut top = b.y + self.crumb_h() - self.scroll;
        for (ri, row) in self.rows.iter_mut().enumerate() {
            let e = &registry()[row.idx];
            // ── 설명 워드랩 예약(08-11 — 설명이 컨트롤을 침범하지 않게) ──
            // 가용 폭 = 행 폭 − 좌우 여백 − 그 행 컨트롤 폭 − 간격. 줄 수는 문자 폭
            // 추정(ASCII 7·그 외 14 논리px — 실측은 페인트가 하고, 여기는 **예약**이라
            // 약간의 과대/과소는 여백/말줄임으로 흡수된다).
            let ctl_w = match &row.ctl {
                RowCtl::Combo(_) | RowCtl::Act(_) => combo_w,
                RowCtl::Check(_) => check_w,
                RowCtl::Face(_) => family_w,
                RowCtl::Pos(p) => p.preferred_size().0,
                RowCtl::Color(c) => c.preferred_width().min(rw - pad * 2),
                RowCtl::Font { .. } => 0, // 설명이 전폭을 쓴다(컨트롤이 아래 줄)
            };
            row.desc_avail = (rw - pad * 2 - ctl_w - gap10).max(min_avail);
            let est_logical: i32 = tr(lang, e.desc)
                .chars()
                .map(|c| if c.is_ascii() { 7 } else { 14 })
                .sum();
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let est_px = (est_logical as f32 * scale).round() as i32;
            row.desc_lines = ((est_px + row.desc_avail - 1) / row.desc_avail).clamp(1, 3);
            let h = match e.kind {
                SettingKind::FontSection { .. } => h_font,
                SettingKind::PositionGrid => h_pos,
                _ => h_entry,
            } + (row.desc_lines - 1) * desc_line_h
                + note_hs[ri];
            // 하위 섹션 제목 자리를 행 **위에** 비워 둔다.
            row.head_h = if row.head.is_some() { head_h } else { 0 };
            top += row.head_h;
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
                RowCtl::Face(family) => {
                    // 크기 콤보가 없다 — 얼굴만 지정하고 크기는 Base UI를 따른다.
                    family.set_bounds(
                        Rect::new(
                            rx + rw - family_w - pad,
                            top + (h - ctl_h) / 2,
                            family_w,
                            ctl_h,
                        ),
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
                RowCtl::Color(c) => {
                    c.set_scale(self.scale);
                    let cw = c.preferred_width().min(rw - pad * 2);
                    c.set_bounds(
                        Rect::new(rx + rw - cw - pad, top + (h - ctl_h) / 2, cw, ctl_h),
                        inv,
                    );
                }
                RowCtl::Act(b) => {
                    b.set_scale(self.scale);
                    b.set_bounds(
                        Rect::new(
                            rx + rw - combo_w - pad,
                            top + (h - ctl_h) / 2,
                            combo_w,
                            ctl_h,
                        ),
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
                RowCtl::Face(family) => {
                    // ★ 글자마다 폰트를 찾으면 낭비다 — **Enter로 확정할 때만** 보고한다
                    //   (사용자 지적 08-09: 입력해도 적용되지 않는다).
                    if let Some(v) = family.take_committed() {
                        got.push((e.key, v));
                    }
                    let _ = family.take_changed(); // 중간 변경은 버린다
                }
                RowCtl::Color(c) => {
                    if let Some(v) = c.take_changed() {
                        got.push((e.key, v));
                    }
                }
                RowCtl::Check(c) => {
                    if let Some(on) = c.take_toggled() {
                        got.push((e.key, if on { "on" } else { "off" }.to_string()));
                    }
                }
                RowCtl::Act(b) => {
                    // 행위 항목 — 값이 아니라 트리거. 호스트가 key로 분기한다.
                    if b.take_clicked() {
                        got.push((e.key, "run".to_string()));
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

    /// 시스템 기본 폰트의 표시 이름 지정 — "(시스템 기본)"이 무엇인지 placeholder에
    /// 보여 준다(사용자 지적 08-10). 호스트가 plat에서 조회해 넣는다(ui는 OS를 모른다).
    pub fn set_default_font_names(&mut self, base: &str, mono: &str, inv: &mut Invalidations) {
        if self.default_base_name != base || self.default_mono_name != mono {
            self.default_base_name = base.to_string();
            self.default_mono_name = mono.to_string();
            self.rebuild(inv);
        }
    }

    fn any_family_focused(&self) -> bool {
        // ★ Face(얼굴만 지정 — 고정폭)도 글꼴명 입력이다 — 여기서 빠지면 그 입력이
        // "기본 타이핑 = 검색" 폴백으로 새어 검색창에 글자가 들어간다(사용자 지적 08-10).
        // Color의 hex 입력도 같은 부류(같은 사고를 반복하지 않는다).
        self.rows.iter().any(|r| match &r.ctl {
            RowCtl::Font { family, .. } => family.is_focused(),
            RowCtl::Face(f) => f.is_focused(),
            RowCtl::Color(c) => c.hex_focused(),
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

        // ── 상단 고정 밴드는 클릭을 **먹는다** ──
        // 밴드는 스크롤해 올라간 행 위에 덮여 있다. 막지 않으면 제목을 눌렀을 뿐인데
        // 보이지도 않는 행의 콤보가 열린다.
        {
            let sw = self.s(self.sidebar_w);
            let crumb = Rect::new(
                self.bounds.x + sw,
                self.bounds.y,
                (self.bounds.w - sw).max(0),
                self.crumb_h(),
            );
            let inside = match *ev {
                InputEvent::MouseDown { x, y, .. } | InputEvent::MouseUp { x, y } => {
                    crumb.contains(Point { x, y })
                }
                _ => false,
            };
            if inside {
                return;
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
                // ★ 포커스는 **매 클릭마다 전 컨트롤에 다시 계산**한다. 콤보는 자기 클릭에
                // 스스로 포커스를 켜지만 남의 포커스를 끄지는 못해서, 이걸 빼먹으면
                // 눌러 본 콤보마다 파란 테두리가 남는다(카테고리를 나갔다 오면 재생성돼
                // 사라지던 그 증상 — 사용자 지적 08-09).
                for row in &mut self.rows {
                    match &mut row.ctl {
                        RowCtl::Font { family, size } => {
                            family.set_focused(family.bounds().contains(p));
                            size.set_focused(size.bounds().contains(p));
                        }
                        RowCtl::Pos(g) => g.set_focused(g.bounds().contains(p)),
                        RowCtl::Face(f) => f.set_focused(f.bounds().contains(p)),
                        RowCtl::Color(c) => {
                            if !c.bounds().contains(p) {
                                c.set_focused(false); // 내부 hex 포커스는 자신의 클릭 처리로
                            }
                        }
                        RowCtl::Combo(c) => c.set_focused(c.bounds().contains(p)),
                        RowCtl::Check(c) => c.set_focused(c.bounds().contains(p)),
                        RowCtl::Act(b) => b.set_focused(b.bounds().contains(p)),
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
                let locked: Vec<bool> = self.rows.iter().map(|r| self.is_locked(r.idx)).collect();
                for (row, lock) in self.rows.iter_mut().zip(locked) {
                    if lock {
                        continue; // 잠긴 설정 — 조건이 갖춰질 때까지 만질 수 없다
                    }
                    match &mut row.ctl {
                        RowCtl::Combo(c) => c.on_event(ev, inv),
                        RowCtl::Check(c) => c.on_event(ev, inv),
                        RowCtl::Font { family, size } => {
                            family.on_event(ev, inv);
                            size.on_event(ev, inv);
                        }
                        RowCtl::Pos(g) => g.on_event(ev, inv),
                        RowCtl::Face(f) => f.on_event(ev, inv),
                        RowCtl::Color(c) => c.on_event(ev, inv),
                        RowCtl::Act(b) => b.on_event(ev, inv),
                    }
                }
                self.drain_changes(inv);
                inv.push(self.bounds);
            }
            InputEvent::MouseUp { .. } => {
                // 실행 버튼은 "안에서 떼야" 클릭이다(Button 계약) — MouseUp을 전달해야
                // take_clicked가 성립한다(다른 컨트롤은 MouseDown에서 완결).
                for row in &mut self.rows {
                    if let RowCtl::Act(b) = &mut row.ctl {
                        b.on_event(ev, inv);
                    }
                }
                self.drain_changes(inv);
            }
            InputEvent::Char { .. } => {
                if self.any_family_focused() {
                    for row in &mut self.rows {
                        match &mut row.ctl {
                            RowCtl::Font { family, .. } if family.is_focused() => {
                                family.on_event(ev, inv);
                            }
                            RowCtl::Face(f) if f.is_focused() => f.on_event(ev, inv),
                            RowCtl::Color(c) if c.hex_focused() => c.on_event(ev, inv),
                            _ => {}
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
                            match &mut row.ctl {
                                RowCtl::Font { family, .. } => family.set_focused(false),
                                RowCtl::Face(f) => f.set_focused(false),
                                RowCtl::Color(c) => c.set_focused(false),
                                _ => {}
                            }
                        }
                        inv.push(self.bounds);
                    } else {
                        self.back = true;
                    }
                }
                // 글꼴명 입력 중 — Enter(확정)·캐럿 이동을 그 텍스트박스로.
                // (없으면 Face는 take_committed 확정 경로가 영원히 안 밟힌다.)
                Key::Enter | Key::Left | Key::Right | Key::Home | Key::End
                    if self.any_family_focused() =>
                {
                    for row in &mut self.rows {
                        match &mut row.ctl {
                            RowCtl::Font { family, .. } if family.is_focused() => {
                                family.on_event(ev, inv);
                            }
                            RowCtl::Face(f) if f.is_focused() => f.on_event(ev, inv),
                            RowCtl::Color(c) if c.hex_focused() => c.on_event(ev, inv),
                            _ => {}
                        }
                    }
                    self.drain_changes(inv);
                    inv.push(self.bounds);
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

        // 하위 섹션 제목(스크롤과 함께 올라간다 — 고정 밴드가 그 위를 덮는다).
        let vp_clip = self.right_viewport();
        // 하위 제목 = 본문(Base)보다 **+1px · 굵게**(사용자 확정 08-11).
        ctx.select_font_sized(FontSlot::Base, true, 1.0);
        for row in &self.rows {
            let Some(sub) = row.head else { continue };
            let hr = Rect::new(row.rect.x, row.rect.y - row.head_h, row.rect.w, row.head_h);
            if hr.bottom() <= vp_clip.y || hr.y >= vp_clip.bottom() {
                continue; // 화면 밖
            }
            let th = ctx.text_height();
            // 상자 **아래쪽**에 붙인다 — 남는 높이가 곧 위 여백이 되어 앞 그룹과 끊긴다.
            ctx.text(
                hr.x + self.s(PAD),
                hr.bottom() - self.s(SUB_HEAD_PAD_B) - th,
                vp_clip,
                tr(lang, sub),
                theme.text,
            );
        }

        // 우측 행: 라벨/설명 + 컨트롤.
        for row in &self.rows {
            let e = &registry()[row.idx];
            let r = row.rect;
            match &row.ctl {
                RowCtl::Combo(_)
                | RowCtl::Check(_)
                | RowCtl::Act(_)
                | RowCtl::Pos(_)
                | RowCtl::Face(_)
                | RowCtl::Color(_) => {
                    ctx.select_font(FontSlot::Base, false);
                    ctx.text(
                        r.x + self.s(PAD),
                        r.y + self.s(6),
                        r,
                        tr(lang, e.label),
                        theme.text,
                    );
                    // 설명 — 컨트롤을 침범하지 않게 워드랩(08-11 사용자 지적).
                    ctx.select_font(FontSlot::Status, false);
                    #[allow(clippy::cast_sign_loss)]
                    let lines = wrap_text(
                        ctx,
                        tr(lang, e.desc),
                        row.desc_avail,
                        row.desc_lines as usize,
                    );
                    for (i, line) in lines.iter().enumerate() {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                        let dy = self.s(30) + i as i32 * self.s(DESC_LINE_H);
                        ctx.text(r.x + self.s(PAD), r.y + dy, r, line, theme.text_dim);
                    }
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
                    #[allow(clippy::cast_sign_loss)]
                    let lines = wrap_text(
                        ctx,
                        tr(lang, e.desc),
                        row.desc_avail,
                        row.desc_lines as usize,
                    );
                    for (i, line) in lines.iter().enumerate() {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                        let dy = self.s(64) + i as i32 * self.s(DESC_LINE_H);
                        ctx.text(r.x + self.s(PAD), r.y + dy, r, line, theme.text_dim);
                    }
                }
            }
            match &row.ctl {
                RowCtl::Combo(c) => c.paint(ctx, theme),
                RowCtl::Check(c) => c.paint(ctx, theme),
                RowCtl::Act(b) => b.paint(ctx, theme),
                RowCtl::Pos(g) => g.paint(ctx, theme),
                RowCtl::Face(f) => f.paint(ctx, theme),
                RowCtl::Color(c) => c.paint(ctx, theme),
                RowCtl::Font { family, size } => {
                    family.paint(ctx, theme);
                    size.paint(ctx, theme);
                }
            }
        }
        // 잠긴 행은 위에 얇은 가림막을 덮어 "지금은 못 만진다"를 보여 준다.
        for row in &self.rows {
            if self.is_locked(row.idx) {
                ctx.fill_round_rect_alpha(row.rect, 0, theme.panel_bg, 0.55);
            }
        }

        // 행에 붙은 정보 줄 — **행 바로 아래 고정 위치**.
        // 고정폭으로 그린다: 숫자 폭이 변하면 1초마다 글자가 흔들린다(사용자 지적 08-09).
        ctx.select_font(FontSlot::Mono, false);
        for row in &self.rows {
            let Some(note) = self.notes.get(registry()[row.idx].key) else {
                continue;
            };
            let nh = self.s(NOTE_H);
            let r = Rect::new(row.rect.x, row.rect.bottom() - nh, row.rect.w, nh);
            let th = ctx.text_height();
            ctx.text(
                r.x + self.s(PAD),
                r.y + (r.h - th) / 2,
                r,
                note,
                theme.text_dim,
            );
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

        // ── 상단 고정 밴드: 지금 보고 있는 설정의 계층 ──
        // 스크롤해 올라간 섹션 제목이 사라지면, 화면 가운데의 "Accent"가 다크의 것인지
        // 라이트의 것인지 알 수 없다(사용자 지적 08-10). 그래서 **늘 남긴다**.
        // 스크롤 내용을 덮어야 하므로 **맨 마지막에, 불투명하게** 그린다.
        let crumb = Rect::new(
            self.bounds.x + sw,
            self.bounds.y,
            (self.bounds.w - sw).max(0),
            self.crumb_h(),
        );
        ctx.fill_rect(crumb, theme.panel_bg);
        if let Some((cat, sub)) = self.current_group() {
            // 상위 제목 = 본문(Base)보다 **+2px · 굵게**(사용자 확정 08-11).
            ctx.select_font_sized(FontSlot::Base, true, 2.0);
            let th = ctx.text_height();
            let cat_h = self.s(CRUMB_CAT_H);
            ctx.text(
                crumb.x + self.s(PAD),
                crumb.y + (cat_h - th) / 2,
                crumb,
                tr(lang, cat),
                theme.text,
            );
            // 하위 줄 — 직속 설정 구간이면 비워 둔다(자리는 유지).
            if let Some(sub) = sub {
                // 밴드의 하위 줄은 본문 섹션 제목과 **같은 위계** = 같은 모양으로 보인다.
                ctx.select_font_sized(FontSlot::Base, true, 1.0);
                let sth = ctx.text_height();
                let sub_h = self.s(CRUMB_SUB_H);
                // 한 단 들여써서 "상위 아래"임을 보인다.
                ctx.text(
                    crumb.x + self.s(PAD) + self.s(14),
                    crumb.y + cat_h + (sub_h - sth) / 2,
                    crumb,
                    tr(lang, sub),
                    theme.text_dim,
                );
            }
        }
        ctx.fill_rect(
            Rect::new(crumb.x, crumb.bottom() - 1, crumb.w, 1),
            theme.border,
        );

        // 텍스트 필드 우클릭 메뉴 — 진짜 최상위(고정 밴드보다도 위 · 08-13 실기:
        // 프로필에서 형제 위젯이 메뉴를 덮던 것과 같은 z순서 계열).
        self.search.paint_popup(ctx, theme);
        for row in &self.rows {
            match &row.ctl {
                RowCtl::Font { family, .. } => family.paint_popup(ctx, theme),
                RowCtl::Face(f) => f.paint_popup(ctx, theme),
                _ => {}
            }
        }
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

    /// ADR-0011 T-7 — 파일에서 온 값의 관용 검증: 무효 값은 거부가 아니라
    /// **무시(기본값 유지)**, 모르는 키만 거짓(미지 키 보존 대상).
    #[test]
    fn set_by_name_lenient_validation() {
        let mut s = SettingsState::with_defaults();
        // Radio: 후보 밖 값은 무시, 후보 값은 적용.
        assert!(s.set_by_name("chat.window_mode", "쓰레기"));
        assert_eq!(s.get("chat.window_mode"), "single");
        assert!(s.set_by_name("chat.window_mode", "separate"));
        assert_eq!(s.get("chat.window_mode"), "separate");
        // Toggle: on/off 외 무시.
        assert!(s.set_by_name("chat.time_24h", "yes"));
        assert_eq!(s.get("chat.time_24h"), "on");
        assert!(s.set_by_name("chat.time_24h", "off"));
        assert_eq!(s.get("chat.time_24h"), "off");
        // Color: #RRGGBB 아니면 무시.
        let before = s.get("theme.dark.accent").to_string();
        assert!(s.set_by_name("theme.dark.accent", "red"));
        assert_eq!(s.get("theme.dark.accent"), before);
        assert!(s.set_by_name("theme.dark.accent", "#112233"));
        assert_eq!(s.get("theme.dark.accent"), "#112233");
        // FontSection 파생 size 키도 아는 키다.
        assert!(s.set_by_name("font.base.size", "l"));
        // 모르는 키만 거짓.
        assert!(!s.set_by_name("future.key", "x"));
    }

    /// known_pairs는 키 정렬(결정적 직렬화 — S-3 비교 성립 조건).
    #[test]
    fn known_pairs_sorted_and_complete() {
        let s = SettingsState::with_defaults();
        let pairs = s.known_pairs();
        assert!(pairs.windows(2).all(|w| w[0].0 < w[1].0), "정렬·중복 없음");
        assert!(pairs.iter().any(|(k, _)| *k == "chat.window_mode"));
        assert!(pairs.iter().any(|(k, _)| *k == "font.base.size"));
    }

    #[test]
    fn hidden_keys_load_recent_and_window_geometry() {
        // ★ 08-14 실기 회귀 — HIDDEN_KEYS에 없으면 **저장은 되는데 부팅 로드에서
        // 미지 키로 무시**돼 재시작마다 증발한다(최근 이미지 목록이 실제로 당했다).
        let mut s = SettingsState::with_defaults();
        assert!(
            s.set_by_name("profile.image_recent", "/a.png\t/b.png"),
            "아는 키여야 한다"
        );
        assert_eq!(s.get("profile.image_recent"), "/a.png\t/b.png");
        assert!(
            s.set_by_name("ui.win_x", "120"),
            "창 위치 키도 로드돼야 한다"
        );
        assert_eq!(s.get("ui.win_x"), "120");
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
    fn direct_settings_come_first_then_each_sub_group() {
        // 사용자 확정 08-10 — 상위 직속 → 하위1 → 하위2 순서.
        // registry 순서 그대로면 다크 색과 라이트 색 사이에 언어·툴바가 끼어든다.
        let (mut w, _) = widget();
        select_cat(&mut w, Msg::CatAppearance);
        let groups: Vec<Option<Msg>> = w.rows.iter().map(|r| r.group.1).collect();
        assert!(!groups.is_empty());
        // 직속(None)이 앞에 몰려 있어야 한다 — 뒤쪽에 None이 다시 나오면 섞인 것이다.
        let last_direct = groups.iter().rposition(Option::is_none).unwrap();
        let first_sub = groups.iter().position(Option::is_some).unwrap();
        assert!(
            last_direct < first_sub,
            "직속 설정이 하위 그룹 뒤로 흩어졌다: {groups:?}"
        );
        // 같은 하위는 **연속**해야 한다(한 번 끝난 그룹이 다시 나오면 안 된다).
        let mut seen = Vec::new();
        for g in groups.iter().flatten() {
            if seen.last() != Some(g) {
                assert!(!seen.contains(g), "그룹 {g:?}이 두 번 나온다: {groups:?}");
                seen.push(*g);
            }
        }
        assert!(seen.len() >= 2, "하위 그룹이 둘 이상이어야 의미 있는 검증");
    }

    #[test]
    fn each_sub_group_gets_exactly_one_header() {
        let (mut w, _) = widget();
        select_cat(&mut w, Msg::CatAppearance);
        let heads: Vec<Msg> = w.rows.iter().filter_map(|r| r.head).collect();
        let subs: Vec<Msg> = {
            let mut v: Vec<Msg> = w.rows.iter().filter_map(|r| r.group.1).collect();
            v.dedup();
            v
        };
        assert_eq!(heads, subs, "그룹마다 제목 하나 — 빠지거나 겹치지 않는다");
        // 직속 구간에는 제목을 붙이지 않는다(상위 제목은 고정 밴드가 늘 보여준다).
        assert!(
            w.rows
                .iter()
                .all(|r| r.group.1.is_some() || r.head.is_none()),
            "직속 행에 하위 제목이 붙었다"
        );
    }

    #[test]
    fn pinned_band_follows_the_scroll_position() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatAppearance);
        // 맨 위 = 상위 직속 구간이므로 하위 줄은 비어 있다.
        assert_eq!(w.current_group(), Some((Msg::CatAppearance, None)));
        // 첫 하위 그룹의 첫 행까지 스크롤하면 밴드가 그 하위를 가리켜야 한다.
        let (want_sub, y) = w
            .rows
            .iter()
            .find_map(|r| r.group.1.map(|s| (s, r.rect.y)))
            .unwrap();
        w.scroll += y - w.right_viewport().y;
        w.layout(&mut inv);
        assert_eq!(
            w.current_group(),
            Some((Msg::CatAppearance, Some(want_sub))),
            "스크롤한 그룹이 상단에 남아야 한다"
        );
    }

    /// ★ 08-15 실기 — 설명 워드랩(2~3줄)이 많은 카테고리(IME)에서 **끝까지 스크롤이
    /// 안 되던 것**: 총높이 합산이 워드랩 예약분을 빼고 계산돼 스크롤 상한이 실제
    /// 콘텐츠 끝보다 작았다. 상한까지 스크롤하면 마지막 행이 뷰포트 안에 들어와야 한다.
    #[test]
    fn scroll_upper_bound_reaches_last_row() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatIme);
        // 과도한 값 → layout이 상한으로 클램프.
        w.scroll = 1_000_000;
        w.layout(&mut inv);
        let vp = w.right_viewport();
        let last = w.rows.last().unwrap().rect;
        assert!(
            last.bottom() <= vp.bottom() + 2,
            "상한 스크롤에서 마지막 행이 화면 안이어야 한다: bottom {} > vp {}",
            last.bottom(),
            vp.bottom()
        );
        assert!(
            last.y >= vp.y - last.h,
            "마지막 행이 위로 사라질 만큼 과도하게 스크롤되지도 않는다"
        );
    }

    #[test]
    fn content_starts_below_the_pinned_band() {
        // 밴드가 첫 행을 덮으면 그 설정은 영영 못 만진다.
        let (mut w, _) = widget();
        select_cat(&mut w, Msg::CatAppearance);
        let first = w.rows.first().unwrap().rect;
        assert!(
            first.y >= w.bounds.y + w.crumb_h(),
            "첫 행이 밴드 아래에서 시작해야 한다: {} < {}",
            first.y,
            w.bounds.y + w.crumb_h()
        );
    }

    #[test]
    fn band_swallows_clicks_so_hidden_rows_are_not_hit() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatAppearance);
        // 아래로 스크롤해 행들이 밴드 뒤로 올라가게 한다.
        w.scroll = 200;
        w.layout(&mut inv);
        let before: Vec<Rect> = w.rows.iter().map(|r| r.rect).collect();
        let sw = w.s(w.sidebar_w);
        w.on_event(&click(w.bounds.x + sw + 20, w.bounds.y + 4), &mut inv);
        let after: Vec<Rect> = w.rows.iter().map(|r| r.rect).collect();
        assert_eq!(before, after, "밴드 클릭이 뒤 행을 건드리면 안 된다");
        assert!(
            w.rows
                .iter()
                .all(|r| !matches!(&r.ctl, RowCtl::Combo(c) if c.is_open())),
            "보이지 않는 행의 콤보가 열렸다"
        );
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
    fn mono_face_input_stays_out_of_search_and_commits_on_enter() {
        // 사용자 지적 08-10 — Face(고정폭 얼굴) 입력이 "기본 타이핑 = 검색" 폴백으로
        // 새어 검색창에 글자가 들어가고, Enter 확정 경로도 없었다.
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatFont);
        let (i, fb) = w
            .rows
            .iter()
            .enumerate()
            .find_map(|(i, r)| match &r.ctl {
                RowCtl::Face(f) => Some((i, f.bounds())),
                _ => None,
            })
            .expect("Face 행 존재");
        let key_name = registry()[w.rows[i].idx].key;
        w.on_event(&click(fb.x + 5, fb.y + 5), &mut inv);
        for c in "D2".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        assert!(w.query.is_empty(), "글꼴 얼굴 입력이 검색으로 새면 안 된다");
        w.on_event(&key(Key::Enter), &mut inv);
        let changes = w.take_changes();
        assert!(
            changes.iter().any(|(k, v)| *k == key_name && v == "D2"),
            "Enter 확정이 보고돼야 한다: {changes:?}"
        );
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
        // "모양" 행 위치를 찾아 클릭 — 레지스트리에 카테고리가 늘어도 안 깨진다(M1-10에서 학습).
        let row = w
            .cat_map
            .iter()
            .position(|(c, s)| SettingsWidget::cats()[*c].0 == Msg::CatAppearance && s.is_none())
            .expect("모양 행");
        let tb = w.tree.bounds();
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        w.on_event(&click(tb.x + 10, tb.y + 24 * row as i32 + 5), &mut inv);
        assert_eq!(
            SettingsWidget::cats()[w.selected_cat].0,
            Msg::CatAppearance,
            "모양 선택"
        );
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
        // 사이드바에 하위 행이 존재(모양 아래) — 카테고리 인덱스는 위치로 찾는다.
        let ai = SettingsWidget::cats()
            .iter()
            .position(|(c, _)| *c == Msg::CatAppearance)
            .expect("모양 카테고리");
        assert!(
            w.cat_map.contains(&(ai, Some(Msg::CatTypeahead))),
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
