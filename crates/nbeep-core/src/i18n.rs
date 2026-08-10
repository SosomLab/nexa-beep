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
    /// 모양 하위: 타입어헤드.
    CatTypeahead,
    /// 카테고리: 파일 전송.
    CatFiles,
    /// 파일 수신 승인 방식.
    XferApproval,
    XferApprovalDesc,
    ApprovalManual,
    ApprovalAuto,
    ApprovalTimed,
    ApprovalBlock,
    /// 기간 자동 승인 길이.
    XferWindow,
    XferWindowDesc,
    Win1h,
    Win6h,
    WinToday,
    /// 전송 속도 상한.
    SendRate,
    SendRateDesc,
    RecvRate,
    RecvRateDesc,
    RateAuto,
    Rate100k,
    Rate1m,
    Rate10m,
    Rate100m,
    Rate1g,
    /// 전송 대기 시간(승인/응답 자동 취소).
    XferTimeout,
    XferTimeoutDesc,
    Sec30,
    Sec60,
    Sec120,
    Sec300,
    /// 고정폭 글꼴(Base UI와 크기 공유).
    FontMono,
    FontMonoDesc,
    // ── 격리함 · 수신 승인 화면 ──
    QuarantineTitle,
    Time24h,
    Time24hDesc,
    DateFormat,
    DateFormatDesc,
    DateFormatIso,
    DateFormatShort,
    QEmpty,
    QApprove,
    QReject,
    QClear,
    QClearConfirm,
    QDoneTag,
    RiskExec,
    RiskActive,
    RiskArchive,
    RiskData,
    RiskExecNote,
    RiskActiveNote,
    RiskArchiveNote,
    RiskDataNote,
    QConfirmExec,
    OfferTitle,
    OfferSender,
    OfferWhen,
    OfferName,
    OfferSize,
    OfferAutoBtn,
    OfferCancel,
    OfferQuarantineNote,
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
    TaSec10,
    /// 콤보 "직접 입력…" 항목.
    CustomInput,
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
    // ── 툴바·메뉴 ──
    MenuLabel,
    MenuGallery,
    /// 메뉴바 '도움말' 라벨.
    MenuHelp,
    ToolbarSize,
    ToolbarSizeDesc,
    Tb16,
    Tb24,
    Tb32,
    Tb64,
    RefreshList,
}

impl Msg {
    /// `[en, ko, zh, ja]` — 이 키의 4언어 번역.
    const fn row(self) -> [&'static str; 4] {
        match self {
            Msg::CatConversation => ["Conversation", "대화", "对话", "会話"],
            Msg::CatAppearance => ["Appearance", "모양", "外观", "外観"],
            Msg::CatFont => ["Font", "글꼴", "字体", "フォント"],
            Msg::CatTypeahead => ["Type-ahead", "타입어헤드", "预输入", "先行入力"],
            Msg::CatFiles => ["Files", "파일", "文件", "ファイル"],
            Msg::XferApproval => [
                "Incoming file approval",
                "파일 수신 승인",
                "文件接收批准",
                "ファイル受信承認",
            ],
            Msg::XferApprovalDesc => [
                "How to handle each incoming file offer — one approval per offer",
                "수신 제안마다 어떻게 처리할지 — 제안 1건당 승인 1번",
                "如何处理每个接收提议 — 每个提议一次批准",
                "受信提案ごとの扱い — 提案1件につき承認1回",
            ],
            Msg::ApprovalManual => ["Ask each time", "매번 확인(기본)", "每次询问", "毎回確認"],
            Msg::ApprovalAuto => ["Always accept", "항상 수락", "始终接受", "常に受諾"],
            Msg::ApprovalTimed => [
                "Accept for a period",
                "기간만 자동 수락",
                "限时自动接受",
                "期間限定で自動受諾",
            ],
            Msg::ApprovalBlock => ["Reject all", "모두 거부", "全部拒绝", "すべて拒否"],
            Msg::XferWindow => [
                "Auto-accept period",
                "자동 수락 기간",
                "自动接受时长",
                "自動受諾期間",
            ],
            Msg::XferWindowDesc => [
                "Reverts to the previous choice when it ends",
                "기간이 끝나면 직전 방식으로 되돌아갑니다",
                "结束后恢复为上一个选项",
                "終了後は直前の方式に戻ります",
            ],
            Msg::Win1h => ["1 hour", "1시간", "1小时", "1時間"],
            Msg::Win6h => ["6 hours", "6시간", "6小时", "6時間"],
            Msg::WinToday => ["Today", "오늘(24시간)", "今天", "今日"],
            Msg::SendRate => [
                "Upload limit",
                "보내기 속도 제한",
                "上传限速",
                "送信速度制限",
            ],
            Msg::SendRateDesc => [
                "Auto = half of the measured peak, so other apps keep bandwidth",
                "자동 = 실측 최고 속도의 절반 — 다른 앱이 쓸 대역을 남깁니다",
                "自动 = 实测峰值的一半，为其他应用保留带宽",
                "自動 = 実測ピークの半分 — 他アプリの帯域を残します",
            ],
            Msg::RecvRate => [
                "Download limit",
                "받기 속도 제한",
                "下载限速",
                "受信速度制限",
            ],
            Msg::RecvRateDesc => [
                "Announced to the sender — the lower of the two is used",
                "발신자에게 알려 **둘 중 낮은 쪽**이 적용됩니다",
                "会告知发送方 — 采用两者中较低者",
                "送信側に通知され、低い方が適用されます",
            ],
            Msg::RateAuto => [
                "Auto (50% of peak)",
                "자동(최고의 50%)",
                "自动(峰值50%)",
                "自動(ピーク50%)",
            ],
            Msg::Rate100k => ["100 KB/s"; 4],
            Msg::Rate1m => ["1 MB/s"; 4],
            Msg::Rate10m => ["10 MB/s"; 4],
            Msg::Rate100m => ["100 MB/s"; 4],
            Msg::Rate1g => ["1 GB/s"; 4],
            Msg::XferTimeout => [
                "Wait timeout",
                "전송 대기 시간",
                "等待超时",
                "待機タイムアウト",
            ],
            Msg::XferTimeoutDesc => [
                "Approval and response windows cancel themselves after this",
                "승인 창과 응답 대기가 이 시간이 지나면 스스로 취소됩니다",
                "批准窗口与响应等待超过此时间后自动取消",
                "承認ウィンドウと応答待ちはこの時間で自動キャンセル",
            ],
            Msg::Sec30 => ["30s", "30초", "30秒", "30秒"],
            Msg::Sec60 => ["60s", "60초", "60秒", "60秒"],
            Msg::Sec120 => ["2m", "2분", "2分钟", "2分"],
            Msg::Sec300 => ["5m", "5분", "5分钟", "5分"],
            Msg::FontMono => [
                "Base UI (monospace)",
                "Base UI (고정폭)",
                "Base UI (等宽)",
                "Base UI (等幅)",
            ],
            Msg::FontMonoDesc => [
                "Face only — size follows Status bar. Used where digits must not jitter",
                "얼굴만 지정 — 크기는 상태 표시줄을 따릅니다. 숫자가 흔들리면 안 되는 곳에 쓰입니다",
                "仅字形 — 大小跟随状态栏，用于数字不能抖动之处",
                "字体のみ — サイズはステータスバーに従う。数字が揺れては困る箇所に使用",
            ],
            Msg::Time24h => [
                "Use 24-hour time",
                "24시간 표시 사용",
                "使用24小时制",
                "24時間表示を使用",
            ],
            Msg::Time24hDesc => [
                "Off shows AM/PM (e.g. PM 7:02)",
                "끄면 오전/오후 표시(예: PM 7:02)",
                "关闭时显示上午/下午（如 PM 7:02）",
                "オフで午前/午後表示（例: PM 7:02）",
            ],
            Msg::DateFormat => ["Date format", "날짜 형식", "日期格式", "日付形式"],
            Msg::DateFormatDesc => [
                "Date shown on day-change pill in chats",
                "대화의 날짜 알약에 쓰는 형식",
                "聊天中日期胶囊的格式",
                "チャットの日付ピルの形式",
            ],
            Msg::DateFormatIso => ["2026-08-10", "2026-08-10", "2026-08-10", "2026-08-10"],
            Msg::DateFormatShort => ["8/10", "8/10", "8/10", "8/10"],
            Msg::QuarantineTitle => ["Quarantine", "격리함", "隔离区", "隔離"],
            Msg::QEmpty => [
                "No quarantined files",
                "격리된 파일이 없습니다",
                "没有被隔离的文件",
                "隔離されたファイルはありません",
            ],
            Msg::QApprove => ["Approve", "승인", "批准", "承認"],
            Msg::QReject => ["Delete", "삭제", "删除", "削除"],
            Msg::QClear => ["Clear All", "비우기", "清空", "空にする"],
            Msg::QClearConfirm => [
                "Press again to delete ALL quarantined files",
                "다시 누르면 격리된 파일을 전부 삭제합니다",
                "再次按下将删除所有被隔离的文件",
                "もう一度押すと隔離ファイルをすべて削除します",
            ],
            Msg::QDoneTag => ["Saved", "실체화됨", "已保存", "実体化済み"],
            Msg::RiskExec => ["Executable", "실행형", "可执行", "実行形式"],
            Msg::RiskActive => ["Active doc", "능동 문서", "活动文档", "能動文書"],
            Msg::RiskArchive => ["Archive", "아카이브", "压缩包", "アーカイブ"],
            Msg::RiskData => ["Data", "데이터", "数据", "データ"],
            Msg::RiskExecNote => [
                "Executable — the app never runs it. An OS protection mark is applied",
                "실행형 — 승인해도 앱이 실행하지 않습니다. OS 보호 표식이 붙습니다",
                "可执行 — 应用不会运行它，会附加系统保护标记",
                "実行形式 — アプリは実行しません。OS 保護マークが付きます",
            ],
            Msg::RiskActiveNote => [
                "Active document — may contain macros or scripts (protected view advised)",
                "능동 문서 — 매크로·스크립트가 있을 수 있습니다(보호된 보기 권장)",
                "活动文档 — 可能含宏或脚本(建议保护视图)",
                "能動文書 — マクロ・スクリプトの可能性(保護ビュー推奨)",
            ],
            Msg::RiskArchiveNote => [
                "Archive — saved only. It is never auto-extracted",
                "아카이브 — 저장만 됩니다. 자동으로 풀지 않습니다",
                "压缩包 — 仅保存，不会自动解压",
                "アーカイブ — 保存のみ。自動展開はしません",
            ],
            Msg::RiskDataNote => [
                "Data — ordinary file",
                "데이터 — 일반 파일",
                "数据 — 普通文件",
                "データ — 通常のファイル",
            ],
            Msg::QConfirmExec => [
                "Danger: executable. Press approve once more to materialize (Esc cancels)",
                "위험: 실행형 파일입니다. 승인을 한 번 더 누르면 실체화합니다(Esc 취소)",
                "危险：可执行文件。再次点击批准以实体化(Esc 取消)",
                "危険: 実行形式です。もう一度承認で実体化します(Esc で取消)",
            ],
            Msg::OfferTitle => [
                "Incoming file",
                "파일 수신 요청",
                "文件接收请求",
                "ファイル受信要求",
            ],
            Msg::OfferSender => ["From", "보낸 사람", "发送者", "送信者"],
            Msg::OfferWhen => ["Received", "받은 시각", "接收时间", "受信時刻"],
            Msg::OfferName => ["File name", "파일 이름", "文件名", "ファイル名"],
            Msg::OfferSize => ["Size", "크기", "大小", "サイズ"],
            Msg::OfferAutoBtn => ["Auto-accept", "자동 승인", "自动接受", "自動承認"],
            Msg::OfferCancel => ["Cancel", "취소", "取消", "キャンセル"],
            Msg::OfferQuarantineNote => [
                "Approving only quarantines it — a separate approval is needed to materialize",
                "승인해도 격리함에 보관됩니다 — 실행 가능한 파일이 되려면 별도 승인이 필요합니다",
                "批准后仅进入隔离区 — 实体化需另行批准",
                "承認しても隔離されます — 実体化には別途承認が必要です",
            ],
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
            Msg::TaSec1 => ["1000ms"; 4],
            Msg::TaSec2 => ["2000ms"; 4],
            Msg::TaSec3 => ["3000ms"; 4],
            Msg::TaSec5 => ["5000ms"; 4],
            Msg::TaSec10 => ["10000ms"; 4],
            Msg::CustomInput => ["Custom…", "직접 입력…", "直接输入…", "直接入力…"],
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
            Msg::MenuLabel => ["Menu", "메뉴", "菜单", "メニュー"],
            Msg::MenuHelp => ["Help", "도움말", "帮助", "ヘルプ"],
            Msg::MenuGallery => [
                "Controls gallery",
                "컨트롤 갤러리",
                "控件库",
                "コントロールギャラリー",
            ],
            Msg::ToolbarSize => [
                "Toolbar icon size",
                "툴바 아이콘 크기",
                "工具栏图标大小",
                "ツールバーアイコンサイズ",
            ],
            Msg::ToolbarSizeDesc => [
                "Size of toolbar image buttons — applies immediately",
                "툴바 이미지 버튼의 크기 — 즉시 적용됩니다",
                "工具栏图像按钮的大小 — 立即生效",
                "ツールバー画像ボタンのサイズ — 即時適用",
            ],
            Msg::Tb16 => ["16×16"; 4],
            Msg::Tb24 => ["24×24"; 4],
            Msg::Tb32 => ["32×32"; 4],
            Msg::Tb64 => ["64×64"; 4],
            Msg::RefreshList => ["Refresh list", "목록 갱신", "刷新列表", "一覧を更新"],
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
