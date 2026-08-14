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
    CatColorsDark,
    CatColorsLight,
    ColorAccent,
    ColorAccentDesc,
    ColorBubblePeer,
    ColorBubblePeerDesc,
    ColorPanelBg,
    ColorPanelBgDesc,
    ColorText,
    ColorTextDesc,
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
    // ── 컨텍스트 메뉴(우클릭) ──
    CtxCopy,
    CtxCut,
    CtxPaste,
    CtxSelectAll,
    /// 말풍선 우클릭 — 메시지 본문 전체 복사.
    CtxCopyMessage,
    // ── 스크롤바 설정 ──
    ScrollbarHide,
    ScrollbarHideDesc,
    TooltipDelay,
    TooltipDelayDesc,
    CarouselScroll,
    CarouselScrollDesc,
    ScrollOsDefault,
    ScrollForward,
    ScrollNatural,
    /// 자동 숨김 없음(항상 표시).
    ScrollbarHideNever,
    TaSec1,
    TaSec2,
    TaSec3,
    TaSec5,
    TaSec10,
    Ms500,
    Ms1500,
    Ms20,
    Ms40,
    Ms80,
    Ms120,
    Ms150,
    Ms200,
    Ms250,
    Ms300,
    Ms400,
    Ms800,
    Ms1600,
    // ── 한글 입력(IME) 기준값(08-15 · H-27) ──
    CatIme,
    ImeInject,
    ImeInjectDesc,
    ImeLeak,
    ImeLeakDesc,
    ImeStale,
    ImeStaleDesc,
    ImeSameKey,
    ImeSameKeyDesc,
    ImePending,
    ImePendingDesc,
    ImeEcho,
    ImeEchoDesc,
    ImeStash,
    ImeStashDesc,
    ImeOwed,
    ImeOwedDesc,
    ImePreClear,
    ImePreClearDesc,
    ImeSwallow,
    ImeSwallowDesc,
    ImeSelfcommit,
    ImeSelfcommitDesc,
    // ── 고급(설정 백업·복원·초기화 — 08-15) ──
    CatAdvanced,
    SetBackup,
    SetBackupDesc,
    SetRestore,
    SetRestoreDesc,
    SetReset,
    SetResetDesc,
    ActReset,
    // ── 목록 보기(08-14) — 갱신 주기 + 갱신 시 스크롤 동작 ──
    CatPeerList,
    ListRefresh,
    ListRefreshDesc,
    ListScroll,
    ListScrollDesc,
    ListScrollKeep,
    ListScrollCaret,
    ListScrollTop,
    // ── 목록 정렬(08-15) — 고정 구획 + 속성 사슬 ──
    ListSort,
    ListSortDesc,
    SortSeen,
    SortChat,
    SortOnline,
    SortName,
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
    Tb48,
    Tb64,
    RefreshList,
    // ── 프로필 (M1-10 · FR-S-50) ──
    /// 카테고리: 프로필.
    CatProfile,
    /// 표시 이름 항목.
    DisplayNameLabel,
    /// 표시 이름 설명 — **LAN 평문 방송 고지**(R-19 옵트인 조건).
    DisplayNameDesc,
    /// 기본값(정제된 호스트명·지문 라벨) 선택지.
    NameAuto,
    /// 신원 키 백업(M2-5a).
    IdBackup,
    IdBackupDesc,
    /// 백업 버튼 라벨.
    ActBackup,
    /// 신원 키 복원.
    IdRestore,
    IdRestoreDesc,
    ActRestore,
    /// 프로필 공개(DR-22 옵트인) — 기본정보(사진·표시 이름).
    ShareBasic,
    ShareBasicDesc,
    /// 추가정보 개별 공개 — 이메일.
    ShareEmail,
    ShareEmailDesc,
    /// 추가정보 개별 공개 — 전화번호.
    SharePhone,
    SharePhoneDesc,
    /// 컨트롤 크기(체크·스위치·옵션박스 글리프 배율).
    ControlSize,
    ControlSizeDesc,
    /// 프로필 화면(M3-17).
    ProfileTitle,
    /// 프로필 이미지 필드.
    ProfileImage,
    /// 아바타 보더 색 필드(08-14).
    AvatarBorderLabel,
    /// 파일 선택 버튼.
    ActChoose,
    /// 이메일 필드.
    FieldEmail,
    /// 전화번호 필드.
    FieldPhone,
    /// "LAN에 방송되지 않음 · 연결된 상대의 요청에만" 안내.
    ProfileShareNote,
    // ── 네트워크 (DR-19 · ADR-0006 — 수동 등록이 성립하려면 포트가 예측 가능해야) ──
    /// 카테고리: 네트워크.
    CatNetwork,
    /// 세션 수신 포트 항목 — **듣는 포트 = 주소 입력에서 포트 생략 시 거는 포트**(사용자 확정 08-13 ⓐ).
    SessionPort,
    SessionPortDesc,
    /// 기본 포트(47200) 선택지 라벨.
    PortDefault,
    // ── 그룹 (M5-1 · ADR-0012) ──
    /// 카테고리: 그룹.
    CatGroup,
    /// 발신자 보관 상한 — 미전달 그룹 메시지를 발신자가 몇 개까지 보관하는가
    /// (재동기 주체 = 송신자 · 사용자 확정 08-13).
    GroupResyncKeep,
    GroupResyncKeepDesc,
    /// 보관 상한 선택지.
    Count50,
    Count200,
    Count1000,
    /// 구성원 초대 허용(새 방 기본값 · 방별로 소유자가 변경 — ADR-0012 정책).
    GroupMemberInvite,
    GroupMemberInviteDesc,
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
            Msg::CatColorsDark => [
                "Colors (Dark)",
                "색상 (다크)",
                "颜色（深色）",
                "カラー（ダーク）",
            ],
            Msg::CatColorsLight => [
                "Colors (Light)",
                "색상 (라이트)",
                "颜色（浅色）",
                "カラー（ライト）",
            ],
            Msg::ColorAccent => ["Accent", "강조색", "强调色", "アクセント"],
            Msg::ColorAccentDesc => [
                "Selection, links, my message bubble",
                "선택·링크·내 말풍선에 쓰는 색",
                "选中、链接与我的气泡颜色",
                "選択・リンク・自分の吹き出しの色",
            ],
            Msg::ColorBubblePeer => [
                "Received bubble",
                "받은 말풍선",
                "接收气泡",
                "受信の吹き出し",
            ],
            Msg::ColorBubblePeerDesc => [
                "Background of messages you receive",
                "수신 메시지 풍선의 배경",
                "接收消息气泡的背景",
                "受信メッセージの背景",
            ],
            Msg::ColorPanelBg => ["Panel background", "패널 배경", "面板背景", "パネル背景"],
            Msg::ColorPanelBgDesc => [
                "List and conversation background",
                "목록·대화 화면의 배경",
                "列表与会话背景",
                "リスト・会話の背景",
            ],
            Msg::ColorText => ["Text", "본문 텍스트", "正文文本", "本文テキスト"],
            Msg::ColorTextDesc => [
                "Primary text color",
                "기본 글자 색",
                "主要文字颜色",
                "基本の文字色",
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
            Msg::CtxCopy => ["Copy", "복사", "复制", "コピー"],
            Msg::CtxCut => ["Cut", "잘라내기", "剪切", "切り取り"],
            Msg::CtxPaste => ["Paste", "붙여넣기", "粘贴", "貼り付け"],
            Msg::CtxSelectAll => ["Select All", "전체 선택", "全选", "すべて選択"],
            Msg::CtxCopyMessage => [
                "Copy message",
                "메시지 복사",
                "复制消息",
                "メッセージをコピー",
            ],
            Msg::CarouselScroll => [
                "Carousel scroll direction",
                "캐러셀 스크롤 방향",
                "轮播滚动方向",
                "カルーセルのスクロール方向",
            ],
            Msg::CarouselScrollDesc => [
                "Trackpad horizontal scroll direction for image strips (OS default: natural on macOS)",
                "이미지 띠의 트랙패드 가로 스크롤 방향(OS 기본: macOS는 내추럴)",
                "图片条的触控板横向滚动方向(OS 默认: macOS 为自然方向)",
                "画像ストリップのトラックパッド横スクロール方向(OS 既定: macOSはナチュラル)",
            ],
            Msg::ScrollOsDefault => ["OS default", "OS 기본", "OS 默认", "OS 既定"],
            Msg::ScrollForward => ["Forward", "정방향", "正向", "順方向"],
            Msg::ScrollNatural => ["Natural (reversed)", "내추럴(반전)", "自然(反向)", "ナチュラル(反転)"],
            Msg::TooltipDelay => [
                "Tooltip delay (ms)",
                "툴팁 표시 대기(ms)",
                "工具提示延迟(ms)",
                "ツールチップ表示までの時間(ms)",
            ],
            Msg::TooltipDelayDesc => [
                "Hover this long before a tooltip appears (e.g. recent profile image file name)",
                "마우스를 이 시간만큼 올려 두면 툴팁을 표시한다(예: 최근 프로필 이미지의 파일명)",
                "悬停此时间后显示工具提示(如最近头像的文件名)",
                "この時間ホバーするとツールチップを表示(例: 最近のプロフィール画像のファイル名)",
            ],
            Msg::ScrollbarHide => [
                "Scrollbar auto-hide (ms)",
                "스크롤바 자동 숨김(ms)",
                "滚动条自动隐藏(ms)",
                "スクロールバー自動非表示(ms)",
            ],
            Msg::ScrollbarHideDesc => [
                "Hide the overlay scrollbar this long after scrolling stops (hovering the bar keeps it visible)",
                "스크롤이 멈추고 이 시간이 지나면 오버레이 스크롤바를 숨긴다(막대에 마우스를 올리면 계속 보인다)",
                "停止滚动后经过此时间隐藏浮层滚动条(鼠标停留在滚动条上时保持显示)",
                "スクロール停止からこの時間で重ねて表示のスクロールバーを隠す(バーにカーソルを置くと表示を維持)",
            ],
            Msg::ScrollbarHideNever => [
                "Never hide",
                "숨기지 않음",
                "不隐藏",
                "隠さない",
            ],
            Msg::Ms500 => ["500ms"; 4],
            Msg::Ms1500 => ["1500ms"; 4],
            Msg::Ms20 => ["20ms"; 4],
            Msg::Ms40 => ["40ms"; 4],
            Msg::Ms80 => ["80ms"; 4],
            Msg::Ms120 => ["120ms"; 4],
            Msg::Ms150 => ["150ms"; 4],
            Msg::Ms200 => ["200ms"; 4],
            Msg::Ms250 => ["250ms"; 4],
            Msg::Ms300 => ["300ms"; 4],
            Msg::Ms400 => ["400ms"; 4],
            Msg::Ms800 => ["800ms"; 4],
            Msg::Ms1600 => ["1600ms"; 4],
            Msg::CatIme => [
                "Korean input (IME)",
                "한글 입력(IME)",
                "韩文输入(IME)",
                "ハングル入力(IME)",
            ],
            Msg::ImeInject => [
                "Recover swallowed keys (macOS)",
                "삼킨 키 보충 주입(macOS)",
                "恢复被吞按键(macOS)",
                "飲み込まれたキーの補充(macOS)",
            ],
            Msg::ImeInjectDesc => [
                "macOS only: right after Korean composition ends, the OS may swallow the first ASCII key — observe and re-inject it in order. Windows is unaffected",
                "macOS 전용: 한글 조합 종료 직후 첫 ASCII 키를 OS가 삼키는 실측 문제의 보충 주입. Windows는 이 문제가 없다",
                "仅macOS: 韩文组合结束后系统可能吞掉首个ASCII键 — 观察并按序补入。Windows无此问题",
                "macOSのみ: ハングル変換終了直後の最初のASCIIキーをOSが飲み込む問題の補充。Windowsは無関係",
            ],
            Msg::ImeLeak => [
                "Assemble leaked jamo (macOS)",
                "유출 자모 조합기(macOS)",
                "组合泄漏字母(macOS)",
                "漏れた字母の組み立て(macOS)",
            ],
            Msg::ImeLeakDesc => [
                "macOS only: the first key after switching to Korean can arrive as a raw jamo — assemble it into a syllable locally. Off = raw jamo is typed as-is",
                "macOS 전용: 전환 직후 첫 키가 낱자모로 새는 실측 문제 — 앱이 음절로 조합한다. 끄면 낱자모가 그대로 들어간다",
                "仅macOS: 切换后首键可能以单字母到达 — 本地组合成音节。关闭则原样输入",
                "macOSのみ: 切替直後の最初のキーが字母単体で届く問題 — アプリが音節に組み立てる。オフなら字母のまま",
            ],
            Msg::ImeStale => [
                "Re-inject fallback wait (ms)",
                "보충 주입 틱 대기(ms)",
                "补入回退等待(ms)",
                "補充注入の待機(ms)",
            ],
            Msg::ImeStaleDesc => [
                "How long to wait for the next key before re-injecting a swallowed key on the timer (macOS path)",
                "삼킨 키를 다음 키 없이 타이머로 주입하기까지의 대기(macOS 경로 — 기본 250ms 실측값)",
                "无后续按键时通过计时器补入被吞按键前的等待(macOS路径)",
                "後続キーが無いとき、タイマーで補充するまでの待機(macOS経路)",
            ],
            Msg::ImeSameKey => [
                "Same-keypress window (ms)",
                "같은 키 판정 창(ms)",
                "同一按键判定窗(ms)",
                "同一キー判定ウィンドウ(ms)",
            ],
            Msg::ImeSameKeyDesc => [
                "A held jamo older than this is treated as a leaked previous key, not a duplicate of the current composition (fixes repeated-jamo loss like ㅇㅇ · macOS)",
                "이보다 오래된 보류 자모는 현재 조합의 중복이 아니라 유출된 이전 키로 본다(ㅇㅇ 같은 반복 자모 유실 해소 · macOS · 기본 40ms)",
                "早于此值的暂留字母视为泄漏的前一键而非当前重复(修复ㅇㅇ类重复丢失·macOS)",
                "これより古い保留字母は前のキーの漏れとみなす(ㅇㅇ等の重複消失を修正・macOS)",
            ],
            Msg::ImePending => [
                "Held-key verdict wait (ms)",
                "보류 판정 유예(ms)",
                "暂留判定等待(ms)",
                "保留判定の猶予(ms)",
            ],
            Msg::ImePendingDesc => [
                "If no IME event follows a jamo keydown within this time, it is real input and is released",
                "자모 keydown 뒤 이 시간 안에 IME 이벤트가 안 붙으면 진짜 입력으로 방출한다",
                "字母按下后此时间内无IME事件则视为真实输入并放行",
                "字母キー押下後この時間内にIMEイベントが無ければ実入力として放出",
            ],
            Msg::ImeEcho => [
                "Commit echo window (ms)",
                "확정 잔향 창(ms)",
                "确认回声窗(ms)",
                "確定エコーウィンドウ(ms)",
            ],
            Msg::ImeEchoDesc => [
                "Right after IME commits, the same character arriving again as a keydown is dropped once (double-delivery echo)",
                "IME 확정 직후 같은 문자가 keydown으로 또 오면 1회 버린다(이중 배달 잔향)",
                "IME确认后同字符再次以按键到达时丢弃一次(双重投递回声)",
                "IME確定直後に同じ文字がキーで再到達したら1回捨てる(二重配達エコー)",
            ],
            Msg::ImeStash => [
                "Preedit stash lifetime (ms)",
                "프리에딧 스태시 수명(ms)",
                "预编辑暂存寿命(ms)",
                "プリエディット退避の寿命(ms)",
            ],
            Msg::ImeStashDesc => [
                "Keeps a cleared preedit briefly so focus-loss can still commit it (clear/focus order can invert)",
                "소거된 프리에딧을 잠시 보관해 포커스 이탈 확정이 줍는다(소거·이탈 순서가 뒤집힐 수 있다)",
                "短暂保留被清除的预编辑, 供失焦确认拾取(顺序可能颠倒)",
                "消去されたプリエディットを一時保持し、フォーカス喪失時の確定が拾う",
            ],
            Msg::ImeOwed => [
                "Late-observe offset window (ms)",
                "늦은 관측 상쇄 창(ms)",
                "迟到观察抵消窗(ms)",
                "遅延観測の相殺ウィンドウ(ms)",
            ],
            Msg::ImeOwedDesc => [
                "Cancels a key observation that arrives after its keydown was already delivered (prevents double-typing from proxy lag · macOS)",
                "keydown이 이미 배달된 뒤 늦게 도착한 관측을 상쇄한다(프록시 지연 이중 입력 방지 · macOS)",
                "抵消在按键已投递后迟到的观察(防止代理延迟双输·macOS)",
                "キー配達後に遅れて届いた観測を相殺(プロキシ遅延の二重入力防止・macOS)",
            ],
            Msg::ImePreClear => [
                "Swallow lead slack (ms)",
                "삼킴 선행 여유(ms)",
                "吞键前置余量(ms)",
                "飲み込み先行余裕(ms)",
            ],
            Msg::ImePreClearDesc => [
                "The keypress that closes a composition is observed slightly before the commit is processed — allow this much lead when judging swallows (macOS)",
                "조합을 닫는 keypress의 관측은 Commit 처리보다 약간 먼저 찍힌다 — 삼킴 판정에서 허용하는 선행 폭(macOS)",
                "结束组合的按键观察略早于Commit处理 — 判吞时允许的前置量(macOS)",
                "変換を閉じるキーの観測はCommit処理より少し先 — 判定で許す先行幅(macOS)",
            ],
            Msg::ImeSwallow => [
                "Swallow eligibility window (ms)",
                "삼킴 판정 창(ms)",
                "吞键判定窗(ms)",
                "飲み込み判定ウィンドウ(ms)",
            ],
            Msg::ImeSwallowDesc => [
                "Only observations within this window after composition end count as swallowed keys (macOS)",
                "조합 종료 후 이 시간 안의 관측만 삼킨 키 후보로 본다(macOS)",
                "仅组合结束后此窗口内的观察视为被吞按键(macOS)",
                "変換終了後この時間内の観測のみ飲み込み候補(macOS)",
            ],
            Msg::ImeSelfcommit => [
                "Late-commit echo window (ms)",
                "수동 확정 잔향 창(ms)",
                "手动确认回声窗(ms)",
                "手動確定エコーウィンドウ(ms)",
            ],
            Msg::ImeSelfcommitDesc => [
                "After the app commits a composition itself (click/close), a late identical IME commit within this window is dropped once",
                "앱이 조합을 스스로 확정한 뒤(클릭·창 닫기) 이 시간 안에 온 같은 내용의 늦은 IME Commit을 1회 버린다",
                "应用自行确认组合后, 此窗口内相同内容的迟到Commit丢弃一次",
                "アプリが自ら確定した後、この時間内の同内容の遅延Commitを1回捨てる",
            ],
            Msg::CatPeerList => ["List view", "목록 보기", "列表视图", "リスト表示"],
            Msg::ListRefresh => [
                "List refresh interval (ms)",
                "목록 갱신 주기(ms)",
                "列表刷新间隔(ms)",
                "リスト更新間隔(ms)",
            ],
            Msg::ListRefreshDesc => [
                "Batch discovery-driven list rebuilds to this interval (peer appear/vanish)",
                "발견 이벤트(상대 등장·소멸)로 인한 목록 재구성을 이 간격으로 묶는다",
                "将发现事件(对等出现/消失)引起的列表重建合并到此间隔",
                "発見イベント(相手の出現/消滅)によるリスト再構成をこの間隔にまとめる",
            ],
            Msg::ListScroll => [
                "Scroll on refresh",
                "갱신 시 스크롤 동작",
                "刷新时的滚动行为",
                "更新時のスクロール動作",
            ],
            Msg::ListScrollDesc => [
                "Where the viewport goes when the list refreshes",
                "목록이 갱신될 때 뷰포트를 어디에 둘지",
                "列表刷新时视口的位置",
                "リスト更新時にビューポートをどこに置くか",
            ],
            Msg::ListScrollKeep => [
                "Keep position",
                "현재 위치 유지",
                "保持当前位置",
                "現在位置を維持",
            ],
            Msg::ListScrollCaret => [
                "Selected row to top",
                "선택 행을 맨 위에",
                "选中行置顶",
                "選択行を最上部に",
            ],
            Msg::ListScrollTop => ["Jump to top", "맨 위로 이동", "回到顶部", "先頭へ移動"],
            Msg::ListSort => ["List order", "목록 정렬", "列表排序", "リストの並び順"],
            Msg::ListSortDesc => [
                "Pinned first, then online peers, then this attribute (name order ignores status)",
                "고정 항목 → 접속 중 상대가 먼저 오고, 그 안을 이 속성으로 정렬한다(이름순은 상태 무시)",
                "置顶→在线优先, 区内按此属性排序(名称序忽略状态)",
                "固定→接続中が先、その中をこの属性で並べる(名前順は状態無視)",
            ],
            Msg::SortSeen => [
                "Recently seen",
                "최근 접속순",
                "最近上线",
                "最近接続順",
            ],
            Msg::SortChat => [
                "Recently chatted",
                "최근 대화순",
                "最近对话",
                "最近会話順",
            ],
            Msg::SortOnline => [
                "Online first",
                "접속 우선",
                "在线优先",
                "接続優先",
            ],
            Msg::SortName => ["By name", "이름순", "按名称", "名前順"],
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
            Msg::Tb48 => ["48×48"; 4],
            Msg::Tb64 => ["64×64"; 4],
            Msg::RefreshList => ["Refresh list", "목록 갱신", "刷新列表", "一覧を更新"],
            Msg::CatProfile => ["Profile", "프로필", "个人资料", "プロフィール"],
            Msg::DisplayNameLabel => ["Display name", "표시 이름", "显示名称", "表示名"],
            Msg::DisplayNameDesc => [
                "Broadcast in PLAIN TEXT to everyone on the LAN. Default hides personal parts of the host name.",
                "LAN 전체에 평문으로 방송됩니다. 기본값은 호스트명에서 개인 정보 부분을 제거한 이름입니다.",
                "以明文向局域网内所有人广播。默认值会去除主机名中的个人信息部分。",
                "LAN 全体に平文でブロードキャストされます。既定値はホスト名から個人情報部分を除いた名前です。",
            ],
            Msg::NameAuto => [
                "Auto (cleaned host name)",
                "자동(정제된 호스트명)",
                "自动（净化后的主机名）",
                "自動（整形済みホスト名）",
            ],
            Msg::IdBackup => ["Back up identity key", "신원 키 백업", "备份身份密钥", "識別キーをバックアップ"],
            Msg::IdBackupDesc => [
                "Copies your identity key to a folder you choose. Anyone with this file can act as you — store it safely.",
                "신원 키를 선택한 폴더로 복사합니다. 이 파일을 가진 사람은 나로 행세할 수 있습니다 — 안전하게 보관하세요.",
                "将身份密钥复制到所选文件夹。持有此文件者可冒充您——请妥善保管。",
                "識別キーを選択したフォルダーへコピーします。このファイルの所持者はあなたに成りすませます——安全に保管してください。",
            ],
            Msg::CatAdvanced => ["Advanced", "고급", "高级", "詳細"],
            Msg::SetBackup => [
                "Back up settings",
                "설정 백업",
                "备份设置",
                "設定のバックアップ",
            ],
            Msg::SetBackupDesc => [
                "Save the current settings file (settings.cfg) to a folder you choose",
                "현재 설정 파일(settings.cfg)을 선택한 폴더에 저장한다",
                "将当前设置文件(settings.cfg)保存到所选文件夹",
                "現在の設定ファイル(settings.cfg)を選択フォルダに保存",
            ],
            Msg::SetRestore => [
                "Restore settings",
                "설정 복원",
                "恢复设置",
                "設定の復元",
            ],
            Msg::SetRestoreDesc => [
                "Load a backed-up settings file and apply every known key immediately (unknown keys are preserved)",
                "백업한 설정 파일을 읽어 아는 키 전부를 즉시 적용한다(모르는 키는 보존)",
                "读取备份的设置文件并立即应用所有已知键(未知键保留)",
                "バックアップした設定ファイルを読み込み既知キーを即時適用(未知キーは保持)",
            ],
            Msg::SetReset => [
                "Reset settings",
                "설정 초기화",
                "重置设置",
                "設定の初期化",
            ],
            Msg::SetResetDesc => [
                "Return every visible setting to its default (confirmation required · hidden state like window position is kept)",
                "표시되는 설정 전부를 기본값으로 되돌린다(확인 후 진행 · 창 위치 등 숨김 상태는 유지)",
                "将所有可见设置恢复为默认值(需确认·窗口位置等隐藏状态保留)",
                "表示される設定を既定値に戻す(要確認・ウィンドウ位置等の隠し状態は維持)",
            ],
            Msg::ActReset => ["Reset…", "초기화…", "重置…", "初期化…"],
            Msg::ActBackup => ["Back up…", "백업…", "备份…", "バックアップ…"],
            Msg::IdRestore => ["Restore identity key", "신원 키 복원", "恢复身份密钥", "識別キーを復元"],
            Msg::IdRestoreDesc => [
                "Pick a backup file to replace the current identity. Applies immediately — open conversations are closed.",
                "백업 파일을 선택해 현재 신원을 교체합니다. 즉시 적용되며 열린 대화는 닫힙니다.",
                "选择备份文件以替换当前身份。立即生效，已打开的对话将关闭。",
                "バックアップファイルを選んで現在の識別を置き換えます。即時適用され、開いている会話は閉じられます。",
            ],
            Msg::ActRestore => ["Restore…", "복원…", "恢复…", "復元…"],
            Msg::ShareBasic => ["Share basic profile", "기본 정보 공개", "公开基本资料", "基本情報を公開"],
            Msg::ShareBasicDesc => [
                "Photo and display name — shown only to connected peers on request (never broadcast).",
                "사진·표시 이름 — 연결된 상대의 요청에만 제공됩니다(브로드캐스트에 실리지 않음).",
                "照片与显示名称——仅应已连接对方的请求提供（绝不广播）。",
                "写真と表示名——接続済みの相手のリクエストにのみ提供（ブロードキャストされません）。",
            ],
            Msg::ShareEmail => ["Share email", "이메일 공개", "公开电子邮件", "メールを公開"],
            Msg::ShareEmailDesc => [
                "Off by default. Shared only with connected peers.",
                "기본 비공개. 연결된 상대에게만 제공됩니다.",
                "默认关闭。仅提供给已连接的对方。",
                "既定でオフ。接続済みの相手にのみ提供されます。",
            ],
            Msg::ProfileTitle => ["Profile", "프로필", "个人资料", "プロフィール"],
            Msg::ProfileImage => ["Profile image", "프로필 이미지", "头像", "プロフィール画像"],
            Msg::AvatarBorderLabel => ["Border color", "테두리 색", "边框颜色", "枠線の色"],
            Msg::ActChoose => ["Choose…", "선택…", "选择…", "選択…"],
            Msg::FieldEmail => ["Email", "이메일", "电子邮件", "メール"],
            Msg::FieldPhone => ["Phone", "전화번호", "电话", "電話番号"],
            Msg::ProfileShareNote => [
                "Never broadcast. Shared only with connected peers, and only fields you turned on.",
                "브로드캐스트에 실리지 않습니다. 연결된 상대에게만, 켠 항목만 제공됩니다.",
                "绝不广播。仅提供给已连接的对方，且仅限已开启的字段。",
                "ブロードキャストされません。接続済みの相手にのみ、オンにした項目だけ提供されます。",
            ],
            Msg::ControlSize => ["Control size", "컨트롤 크기", "控件大小", "コントロールの大きさ"],
            Msg::ControlSizeDesc => [
                "Size of checkboxes, switches and option glyphs.",
                "체크박스·스위치·옵션박스 표시 크기입니다.",
                "复选框、开关与选项标记的显示大小。",
                "チェックボックス・スイッチ・オプション記号の表示サイズ。",
            ],
            Msg::SharePhone => ["Share phone number", "전화번호 공개", "公开电话号码", "電話番号を公開"],
            Msg::SharePhoneDesc => [
                "Off by default. Shared only with connected peers.",
                "기본 비공개. 연결된 상대에게만 제공됩니다.",
                "默认关闭。仅提供给已连接的对方。",
                "既定でオフ。接続済みの相手にのみ提供されます。",
            ],
            Msg::CatNetwork => ["Network", "네트워크", "网络", "ネットワーク"],
            Msg::SessionPort => ["Listening port", "수신 포트", "监听端口", "受信ポート"],
            Msg::SessionPortDesc => [
                "Port for incoming connections; also used when an address omits a port. Falls back to a random port if taken.",
                "연결을 받는 포트이며, 주소에서 포트를 생략하면 이 포트로 겁니다. 점유 중이면 임의 포트로 물러납니다.",
                "接收连接的端口；地址省略端口时也使用此端口。被占用时退回随机端口。",
                "接続を受け付けるポートで、アドレスでポート省略時にもこの値を使用。使用中なら任意ポートへ退避。",
            ],
            Msg::PortDefault => [
                "Default (47200)",
                "기본(47200)",
                "默认(47200)",
                "既定(47200)",
            ],
            Msg::CatGroup => ["Groups", "그룹", "群组", "グループ"],
            Msg::GroupResyncKeep => [
                "Undelivered message retention",
                "미전달 메시지 보관 상한",
                "未送达消息保留上限",
                "未配信メッセージ保持上限",
            ],
            Msg::GroupResyncKeepDesc => [
                "How many undelivered group messages the sender keeps per member. Delivered when they come online; oldest dropped over the limit.",
                "구성원별로 미전달 그룹 메시지를 발신자가 몇 개까지 보관할지. 상대가 접속하면 이어 전달되고, 상한 초과분은 오래된 것부터 지워집니다.",
                "发送方为每个成员保留多少条未送达的群消息。对方上线后补发，超限时从最旧的开始丢弃。",
                "未配信のグループメッセージを送信者がメンバーごとに何件保持するか。相手の接続時に配信され、上限超過分は古い順に破棄。",
            ],
            Msg::GroupMemberInvite => [
                "Members can invite (new room default)",
                "구성원 초대 허용(새 방 기본값)",
                "允许成员邀请（新房间默认）",
                "メンバー招待を許可（新規ルーム既定）",
            ],
            Msg::GroupMemberInviteDesc => [
                "Applied to rooms you create. Off = owner-only invites. Each room's owner can change it per room.",
                "내가 만드는 방에 적용됩니다. 끄면 소유자만 초대할 수 있습니다. 방별 설정은 그룹 행 우클릭에서 소유자가 바꿉니다.",
                "应用于你创建的房间。关闭后仅房主可邀请。各房间可由房主单独更改。",
                "自分が作るルームに適用。オフ = オーナーのみ招待可。ルームごとの変更はオーナーが行う。",
            ],
            Msg::Count50 => ["50", "50개", "50条", "50件"],
            Msg::Count200 => ["200 (default)", "200개(기본)", "200条(默认)", "200件(既定)"],
            Msg::Count1000 => ["1000", "1000개", "1000条", "1000件"],
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
