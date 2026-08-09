//! i18n — **프로세스 전역 로케일 + 메시지 카탈로그**(영어 기본 · 한/중/일 언어팩).
//!
//! 외부 i18n 크레이트(fluent·gettext)를 쓰지 않는다 — 런타임 의존 0(DR-5)·퍼미시브(DR-12)
//! 원칙에 맞춰 **자체 표**로 둔다. 문자열은 전부 `&'static str`(빌드 타임 상수)이라 힙 할당·
//! 파일 로드가 없다(예산 게이트에 무해).
//!
//! ## 카탈로그 형태 — "한 키 = 한 줄 4언어"
//!
//! [`Msg`]의 각 변형이 `[en, ko, zh, ja]` 4개를 한 줄로 갖는다(`Msg::row`). 한 줄에 4언어가
//! 모여 있어 **누락·불일치를 리뷰에서 바로 본다**. 새 UI 문자열 = 변형 1개 + 줄 1개.
//!
//! ## 현재 언어 = 프로세스 전역
//!
//! 렌더는 단일 스레드(UI 루프)에서 일어나므로 현재 언어를 [`set_lang`]/[`current_lang`]의
//! 원자값 하나로 둔다(로케일은 관례적으로 전역). 상태를 들고 다니는 위젯([`crate::i18n`] 사용처
//! 참고)은 스냅숏 필드를 따로 둘 수 있다(테스트 결정성).

use core::sync::atomic::{AtomicU8, Ordering};

/// 지원 언어 — **영어 기본**(사용자 확정 08-08).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Lang {
    /// 영어(기본·폴백).
    #[default]
    En,
    /// 한국어.
    Ko,
    /// 중국어(간체).
    Zh,
    /// 일본어.
    Ja,
}

impl Lang {
    /// 전 언어(설정 콤보·순회용).
    pub const ALL: [Lang; 4] = [Lang::En, Lang::Ko, Lang::Zh, Lang::Ja];

    /// 값 코드(설정 저장·복원 계약).
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ko => "ko",
            Lang::Zh => "zh",
            Lang::Ja => "ja",
        }
    }

    /// 코드 → 언어(미지 코드는 `None` — 호출자가 기본으로 폴백).
    #[must_use]
    pub fn from_code(s: &str) -> Option<Lang> {
        match s {
            "en" => Some(Lang::En),
            "ko" => Some(Lang::Ko),
            "zh" => Some(Lang::Zh),
            "ja" => Some(Lang::Ja),
            _ => None,
        }
    }

    /// 자기 언어 이름(endonym) — 언어와 무관하게 그 언어의 표기로 보여준다(설정 라벨).
    #[must_use]
    pub fn endonym(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ko => "한국어",
            Lang::Zh => "中文",
            Lang::Ja => "日本語",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// 현재 언어(원자 저장) — 기본 0 = [`Lang::En`].
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// 현재 언어를 지정한다(설정 변경 시 호스트가 호출).
pub fn set_lang(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// 현재 언어.
#[must_use]
pub fn current_lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Ko,
        2 => Lang::Zh,
        3 => Lang::Ja,
        _ => Lang::En,
    }
}

/// 번역 키 — 값 불변(추가는 뒤에 append). 각 변형이 4언어를 `Msg::row`로 갖는다.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Msg {
    // ── 설정: 카테고리 ──
    CatConversation,
    CatAppearance,
    CatFont,
    // ── 설정: 공통 ──
    SearchPlaceholder,
    SystemDefaultFont,
    // ── 설정: 대화 ──
    ChatWindowMode,
    ChatWindowModeDesc,
    WindowModeSingle,
    WindowModeSeparate,
    // ── 설정: 모양 ──
    Theme,
    ThemeDesc,
    ThemeDark,
    ThemeLight,
    Language,
    LanguageDesc,
    // 언어 이름(endonym — 전 언어 동일 표기).
    LangEnglish,
    LangKorean,
    LangChinese,
    LangJapanese,
    // ── 설정: 글꼴 영역 ──
    FontBase,
    FontBaseDesc,
    FontPeerList,
    FontPeerListDesc,
    FontMessage,
    FontMessageDesc,
    FontStatus,
    FontStatusDesc,
    // 글꼴 크기.
    SizeNormal,
    SizeLarge,
    SizeExtraLarge,
    SizeSmall,
    // ── 대화 화면 ──
    ChatPrefixMe,
    ChatPrefixPeer,
    ChatInputPlaceholder,
    // ── 사용자 목록: 신뢰 등급 ──
    TrustUnverified,
    TrustPinned,
    TrustVerified,
    // ── 창 제목 ──
    SettingsTitle,
    // ── 타입어헤드 설정 ──
    TypeaheadTimeout,
    TypeaheadTimeoutDesc,
    TypeaheadPos,
    TypeaheadPosDesc,
    TaSec1,
    TaSec2,
    TaSec3,
    TaSec5,
    PosTopLeft,
    PosTopCenter,
    PosTopRight,
    PosMidLeft,
    PosCenter,
    PosMidRight,
    PosBottomLeft,
    PosBottomCenter,
    PosBottomRight,
    TypeaheadSpace,
    TypeaheadSpaceDesc,
    TypeaheadSpecial,
    TypeaheadSpecialDesc,
    ToggleApply,
    ToggleIgnore,
}

impl Msg {
    /// `[en, ko, zh, ja]` — 이 키의 4언어 번역.
    const fn row(self) -> [&'static str; 4] {
        match self {
            Msg::CatConversation => ["Conversation", "대화", "对话", "会話"],
            Msg::CatAppearance => ["Appearance", "모양", "外观", "外観"],
            Msg::CatFont => ["Font", "글꼴", "字体", "フォント"],
            Msg::SearchPlaceholder => [
                "Search (space = AND)",
                "검색 (공백=AND)",
                "搜索（空格=AND）",
                "検索（スペース=AND）",
            ],
            Msg::SystemDefaultFont => [
                "(system default)",
                "(시스템 기본)",
                "(系统默认)",
                "(システム既定)",
            ],
            Msg::ChatWindowMode => [
                "Chat window mode",
                "대화 창 모드",
                "对话窗口模式",
                "会話ウィンドウモード",
            ],
            Msg::ChatWindowModeDesc => [
                "How new conversations open — applies from the next conversation",
                "새 대화를 여는 방식 — 변경은 다음 대화부터 적용됩니다",
                "新对话的打开方式 — 从下次对话起生效",
                "新しい会話の開き方 — 次の会話から適用されます",
            ],
            Msg::WindowModeSingle => [
                "Single window",
                "한 창에서 전환",
                "单窗口切换",
                "単一ウィンドウ",
            ],
            Msg::WindowModeSeparate => [
                "Separate windows",
                "상대별 별도 창",
                "每人独立窗口",
                "相手ごとに別ウィンドウ",
            ],
            Msg::Theme => ["Theme", "테마", "主题", "テーマ"],
            Msg::ThemeDesc => [
                "Overall brightness palette — applies immediately",
                "전체 창의 밝기 팔레트 — 즉시 적용됩니다",
                "整体明暗配色 — 立即生效",
                "全体の明暗パレット — 即時適用",
            ],
            Msg::ThemeDark => ["Dark", "다크", "深色", "ダーク"],
            Msg::ThemeLight => ["Light", "라이트", "浅色", "ライト"],
            Msg::Language => ["Language", "언어", "语言", "言語"],
            Msg::LanguageDesc => [
                "Display language — applies immediately",
                "표시 언어 — 즉시 적용됩니다",
                "显示语言 — 立即生效",
                "表示言語 — 即時適用",
            ],
            Msg::LangEnglish => ["English", "English", "English", "English"],
            Msg::LangKorean => ["한국어", "한국어", "한국어", "한국어"],
            Msg::LangChinese => ["中文", "中文", "中文", "中文"],
            Msg::LangJapanese => ["日本語", "日本語", "日本語", "日本語"],
            Msg::FontBase => ["Base UI", "기본 UI", "基本界面", "基本UI"],
            Msg::FontBaseDesc => [
                "Font for buttons, headers, settings and other base UI",
                "버튼·헤더·설정 등 기본 UI 영역의 글꼴",
                "按钮、标题、设置等基本界面的字体",
                "ボタン・見出し・設定など基本UIのフォント",
            ],
            Msg::FontPeerList => ["Peer list", "사용자 목록", "用户列表", "ユーザー一覧"],
            Msg::FontPeerListDesc => [
                "Font for the discovered peer list",
                "발견된 사용자(피어) 목록의 글꼴",
                "已发现用户（对端）列表的字体",
                "発見したユーザー（ピア）一覧のフォント",
            ],
            Msg::FontMessage => ["Message", "대화 본문", "消息正文", "メッセージ本文"],
            Msg::FontMessageDesc => [
                "Font for conversation thread messages",
                "대화 스레드 메시지의 글꼴",
                "对话消息的字体",
                "会話スレッドのメッセージのフォント",
            ],
            Msg::FontStatus => ["Status bar", "상태바", "状态栏", "ステータスバー"],
            Msg::FontStatusDesc => [
                "Font for the bottom status bar and secondary text",
                "하단 상태바·보조 텍스트의 글꼴",
                "底部状态栏及辅助文字的字体",
                "下部ステータスバー・補助テキストのフォント",
            ],
            Msg::SizeNormal => ["Normal", "보통", "标准", "標準"],
            Msg::SizeLarge => ["Large", "크게", "大", "大"],
            Msg::SizeExtraLarge => ["Extra large", "아주 크게", "超大", "特大"],
            Msg::SizeSmall => ["Small", "작게", "小", "小"],
            Msg::ChatPrefixMe => ["Me: ", "나: ", "我：", "自分: "],
            Msg::ChatPrefixPeer => ["Peer: ", "상대: ", "对方：", "相手: "],
            Msg::ChatInputPlaceholder => [
                "Type a message… (Enter to send · Esc for list)",
                "메시지 입력… (Enter 전송 · Esc 목록)",
                "输入消息…（Enter 发送 · Esc 返回列表）",
                "メッセージ入力…（Enter 送信・Esc 一覧）",
            ],
            Msg::TrustUnverified => ["Unverified", "미검증", "未验证", "未検証"],
            Msg::TrustPinned => ["Pinned", "핀 고정", "已固定", "ピン留め"],
            Msg::TrustVerified => ["Verified", "대조 완료", "已核对", "照合済み"],
            Msg::SettingsTitle => ["Settings", "설정", "设置", "設定"],
            Msg::TypeaheadTimeout => [
                "Type-ahead reset (ms)",
                "타입어헤드 초기화(ms)",
                "预输入重置(ms)",
                "先行入力リセット(ms)",
            ],
            Msg::TypeaheadTimeoutDesc => [
                "Clear the type-ahead buffer this long after the last keystroke",
                "마지막 입력 후 이 시간이 지나면 타입어헤드 버퍼를 초기화",
                "上次按键后经过此时间清除预输入缓冲",
                "最後の入力からこの時間で先行入力バッファを消去",
            ],
            Msg::TypeaheadPos => [
                "Type-ahead HUD position",
                "타입어헤드 HUD 위치",
                "预输入提示位置",
                "先行入力HUD位置",
            ],
            Msg::TypeaheadPosDesc => [
                "Where the type-ahead indicator appears (3×3)",
                "타입어헤드 표시가 나타나는 위치(3×3)",
                "预输入提示出现的位置(3×3)",
                "先行入力表示の位置(3×3)",
            ],
            Msg::TaSec1 => ["1.0s"; 4],
            Msg::TaSec2 => ["2.0s"; 4],
            Msg::TaSec3 => ["3.0s"; 4],
            Msg::TaSec5 => ["5.0s"; 4],
            Msg::PosTopLeft => ["↖"; 4],
            Msg::PosTopCenter => ["↑"; 4],
            Msg::PosTopRight => ["↗"; 4],
            Msg::PosMidLeft => ["←"; 4],
            Msg::PosCenter => ["·"; 4],
            Msg::PosMidRight => ["→"; 4],
            Msg::PosBottomLeft => ["↙"; 4],
            Msg::PosBottomCenter => ["↓"; 4],
            Msg::PosBottomRight => ["↘"; 4],
            Msg::TypeaheadSpace => ["Include spaces", "공백 포함", "包含空格", "スペースを含む"],
            Msg::TypeaheadSpaceDesc => [
                "Count the space key in the type-ahead buffer",
                "공백 키를 타입어헤드 버퍼에 포함",
                "空格键计入预输入缓冲",
                "スペースキーを先行入力に含める",
            ],
            Msg::TypeaheadSpecial => ["Include symbols", "특수문자 포함", "包含符号", "記号を含む"],
            Msg::TypeaheadSpecialDesc => [
                "Count symbol keys in the type-ahead buffer",
                "특수문자를 타입어헤드 버퍼에 포함",
                "符号键计入预输入缓冲",
                "記号キーを先行入力に含める",
            ],
            Msg::ToggleApply => ["On", "적용", "开", "オン"],
            Msg::ToggleIgnore => ["Off", "미적용", "关", "オフ"],
        }
    }
}

/// 지정 언어로 번역한다.
#[must_use]
pub fn tr(lang: Lang, msg: Msg) -> &'static str {
    msg.row()[lang.index()]
}

/// 현재 언어로 번역한다(위젯 렌더 편의).
#[must_use]
pub fn t(msg: Msg) -> &'static str {
    tr(current_lang(), msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_default_and_all_langs_resolve() {
        // 기본 언어 = 영어.
        assert_eq!(Lang::default(), Lang::En);
        // 모든 언어에서 모든 키가 비지 않는다(누락 방지).
        let keys = [
            Msg::CatConversation,
            Msg::Theme,
            Msg::Language,
            Msg::FontBase,
            Msg::SizeNormal,
            Msg::TrustPinned,
            Msg::SettingsTitle,
        ];
        for lang in Lang::ALL {
            for &k in &keys {
                assert!(!tr(lang, k).is_empty(), "{lang:?}/{k:?} 비어 있음");
            }
        }
    }

    #[test]
    fn code_roundtrips() {
        for lang in Lang::ALL {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
        assert_eq!(Lang::from_code("xx"), None);
    }

    #[test]
    fn translations_differ_by_language() {
        assert_eq!(tr(Lang::En, Msg::Theme), "Theme");
        assert_eq!(tr(Lang::Ko, Msg::Theme), "테마");
        assert_eq!(tr(Lang::Zh, Msg::Theme), "主题");
        assert_eq!(tr(Lang::Ja, Msg::Theme), "テーマ");
    }

    #[test]
    fn endonyms_are_language_neutral() {
        // 언어 이름은 현재 UI 언어와 무관하게 그 언어 표기.
        assert_eq!(tr(Lang::En, Msg::LangKorean), "한국어");
        assert_eq!(tr(Lang::Ja, Msg::LangKorean), "한국어");
        assert_eq!(Lang::Ko.endonym(), "한국어");
    }
}
