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
    /// 정지 방치 자동 취소 시간 설정(M4-2e ⓓ · 08-20).
    XferAutoCancel,
    XferAutoCancelDesc,
    Min1,
    Min2,
    Min5,
    Min10,
    /// 전송 취소 상태줄(08-20 다국어 — 하드코딩 한국어 정리).
    StXferCanceled,
    /// 타임아웃 취소 상태줄("{}초 동안 응답이 없어 전송을 취소했습니다").
    StfXferTimeoutCanceled,
    /// 설정 입력 검증 실패 경고(08-20 — 직전값 원복 고지).
    ValOutOfRangeTitle,
    ValMinutesRange,
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
    /// 격리물 무결성 검증 중(08-18 — 승인은 검증 후).
    QVerifying,
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
    /// 이어받기 버튼(M4-10c — {0} = 보존율 %).
    /// 승인 창 파일 수 라벨(M4-2e 요청 단위 승인 — 08-20).
    OfferCount,
    /// 승인 창 제외 목록 라벨(08-20 — 상한 초과 등으로 배치에서 빠진 파일).
    OfferExcluded,
    /// 요청당 파일 수 상한 초과 모달(08-20 3차 — 닫기 버튼만).
    WarnBatchLimitTitle,
    /// 본문 — `{}` = 현재 설정 상한.
    WarnBatchLimitBody,
    /// 폴더 드롭 제외 안내 — `{}` = 폴더명.
    StfFolderExcluded,
    /// 최대 송신 개수 설정(08-20).
    XferBatchMax,
    XferBatchMaxDesc,
    Cnt1,
    Cnt2,
    Cnt3,
    Cnt4,
    Cnt5,
    /// I18N-2 스윕(08-20) — 액터 fail 사유·상태줄·피커 타이틀.
    XfRecvRefused,
    XfOpenFailed,
    XfRecvError,
    XfDoneFailed,
    XfWireError,
    StfResumeFrom,
    StfAcceptStart,
    StfDeclineName,
    StfMoreOffers,
    StfTimeoutDeclined,
    StfGroupFileOffer,
    StfPeerRemoved,
    StNoLogs,
    TitleFilePick,
    TitlePickBackupDir,
    TitlePickBackup,
    TitlePickProfileImage,
    TitlePickSettingsBackupDir,
    TitlePickSettingsBackup,
    PickDirPrefix,
    /// 대화함(M3-23 · 08-20) — 제목·버튼·상태.
    ConvboxTitle,
    CvEmpty,
    CvFilterPh,
    CvBackup,
    CvRestore,
    CvClear,
    CvClearConfirm,
    CvDelConfirm,
    CvGroupTag,
    CvCount,
    StfCvBackupDone,
    StfCvRestoreDone,
    StfCvDeleted,
    StCvCleared,
    StCvNone,
    TitlePickCvBackupDir,
    TitlePickCvRestoreDir,
    PickRestoreHere,
    WordFile,
    OfferResumeBtn,
    /// 로그 하위 섹션(M3-22 — 고급 하위).
    SubLog,
    LogEnabled,
    LogEnabledDesc,
    LogRetain,
    LogRetainDesc,
    LogMaxTotal,
    LogMaxTotalDesc,
    LogView,
    LogViewDesc,
    /// "로그 보기" 행위 동사.
    ActOpen,
    /// 폰트 크기 XL 라벨(값은 절대 px — 크기 안내는 Base UI 설명문에).
    SizeXLarge,
    /// 로그 보존 기본 라벨(7일).
    LogRetainDefault,
    /// 로그 총량 기본 라벨(20MB).
    LogCapDefault,
    /// 네트워크 점검 하위 섹션(netmon · 08-21 — 고급 하위 · 옵트인 계측 기록).
    SubNetmon,
    NetmonEnabled,
    NetmonEnabledDesc,
    NetmonInterval,
    NetmonIntervalDesc,
    /// 점검 주기 기본 라벨(10초).
    NetmonIntervalDefault,
    /// 상태바 — netmon 과다 경고(경고 태그 목록).
    StfNetmonWarn,
    /// 공지 발신 빈도 제한(08-21 — 3초 1회) 안내.
    StBroadcastRateLimit,
    /// 검사 사실 표기 3종(FR-S-15 · NFR-S-5 — "안전" 단정 금지 · 08-21).
    ScanNotDone,
    ScanClean,
    ScanDetected,
    /// 상태바 — 수신물 검사 탐지 경고(파일명).
    StfScanDetected,
    /// 격리함 — 아카이브 정책 위반 라벨(M4-4 · Zip Slip·폭탄·판정 불가).
    ArchiveViol,
    /// 상태바 — 데이터 폴더가 클라우드 동기화 폴더 안(M2-5b · 17 §6 경고).
    StfSyncFolderWarn,
    /// 등급 배지 라벨(④ docs/24 — 입력줄 칩 · 08-21 i18n 승격).
    GradeNormal,
    GradeNotice,
    GradeUrgent,
    /// 공지 프롬프트 — 창 제목·본문 플레이스홀더(08-21 i18n 승격).
    WinBroadcast,
    PhBroadcastBody,
    /// 그룹 이름 프롬프트 플레이스홀더.
    PhGroupName,
    /// 설정 — 공지(브로드캐스트) 받지 않기 토글.
    NotifyBroadcastMute,
    NotifyBroadcastMuteDesc,
    /// 처음부터 버튼(M4-10c — 보존분을 버리고 새로 받기).
    OfferFreshBtn,
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
    /// 시스템 시작 시 자동 실행(08-20 — 기본 on · OS별 사용자 수준 등록).
    AutoStart,
    /// 자동 실행 설명.
    AutoStartDesc,
    /// 닫기 버튼 = 트레이 상주(M3-2 · 08-15).
    CloseToTray,
    /// 닫기 = 트레이 설명.
    CloseToTrayDesc,
    /// 트레이 메뉴 — 열기.
    TrayOpen,
    /// 트레이 메뉴 — 종료.
    TrayQuit,
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
    // ── 신뢰 배지 확장(M3-14 · 08-15) — 화면에 없던 상태 + 툴팁 한 줄 ──
    TrustBlocked,
    TrustConflict,
    TrustUnverifiedTip,
    TrustPinnedTip,
    TrustVerifiedTip,
    TrustBlockedTip,
    TrustConflictTip,
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
    // ── 세션 배지 실루엣(M3-19 · 08-15) — 색+모양 2중 부호화 ──
    LinkBadgeShape,
    LinkBadgeShapeDesc,
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
    /// 프로필 적용 버튼(M3-18).
    ActApply,
    /// 프로필 미저장 닫기 경고(M3-18 — Esc 2단계).
    ProfileUnsavedHint,
    ToggleIgnore,
    // ── 툴바·메뉴 ──
    MenuLabel,
    MenuGallery,
    /// 메뉴 ▸ 종료(08-15 — close_to_tray가 켜지면 X로는 못 끝낸다).
    MenuQuit,
    // ── 알림(M3-8 최소 슬라이스 · 08-15) ──
    NotifyEnabled,
    NotifyEnabledDesc,
    NotifyPreview,
    NotifyPreviewDesc,
    NotifyNewMessage,
    NotifyFileOffer,
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
    // ── 서버 (ADR-0013 · 08-17 · 개발은 클라 설정 저장만 · 네트워킹은 TODO) ──
    /// 카테고리: 서버.
    CatServer,
    ServerMode,
    ServerModeDesc,
    ServerModeUnmanaged,
    ServerModeManaged,
    ServerAddress,
    ServerAddressDesc,
    ServerPort,
    ServerPortDesc,
    ServerType,
    ServerTypeDesc,
    ServerTypeAuto,
    ServerTypeRelay,
    ServerTypeContent,
    ServerTypeRegistered,
    SendDelivered,
    SendDeliveredDesc,
    SendRead,
    SendReadDesc,
    // 파일 전송 스레드 메시지(i18n-1 · 08-17 · 대화에 남는 영구 텍스트)
    // 전송 스레드 라벨(08-18 — 대화 항목의 한글 고정 해소).
    XferFileTag,
    XferWaiting,
    XferDirSend,
    XferDirRecv,
    XferAwaitAck,
    XferDoneLbl,
    XferFailLbl,
    XferPeerCanceled,
    /// 진행 배너 전체취소 버튼(M4-2e · 08-19).
    XferCancelAll,
    /// 상대가 전체 취소(M4-2e · 08-19).
    XferPeerCanceledAll,
    /// 10분 정지 방치 자동 전체 취소(M4-2e ⓓ · 08-19).
    XferStaleAutoCancel,
    /// 자동 취소 카운트다운 표시("{} 후 자동 취소" — M4-2e ⓓ · 08-20).
    XferAutoCancelIn,
    /// 발신 원본 읽기 실패로 중단(M4-2e i18n 08-19).
    XferSrcReadFail,
    /// 전송 중 원본 변경 감지로 중단(무결성 가드).
    XferSrcChanged,
    XferResumeFrom,
    XferReofferN,
    /// 발신 사전 상한 초과(M4 메모리 조립 상한 — {0} = 파일명 · {1} = 상한).
    XferTooBigLocal,
    /// 스레드 실패줄 짧은 사유 — `{}` = 상한.
    XferTooBigWhy,
    /// 격리함 첫 스캔 로딩(08-18 — 대용량 개봉이 수 초라 빈 목록이 오류로 보였다).
    QLoading,
    /// 상대 수신 상한 차단(스레드 사유) — `{}` = 상한.
    XferPeerCapBlock,
    /// 상대 수신 상한 차단(상태바) — `{}` 파일명 · `{}` 상한.
    XferPeerCapBlockStatus,
    /// 해시 진행 — `{}` 파일명 · `{}` %.
    XferHashingPct,
    /// 발신 대기 상태바 — `{}` 건수 · `{}` 총량.
    StfSendQueued,
    /// 상대 거절 상태 — `{}` 사유 · `{}` 부가(상한 공지 등 · 빈 문자열 가능).
    XferPeerRejected,
    /// 상한 공지 부가 — `{}` = 상한(사람 표기).
    PeerCapSuffix,
    /// 파일 준비(전체 해시 계산) 상태 — `{}` = 파일명. 대용량은 수십 초 걸린다.
    XferHashing,
    /// 첫 왕래 전 상대에게 파일을 보낼 때의 안내(스레드 공지) — 두부(⚠ 글리프
    /// 부재) 회피를 위해 '!' ASCII 접두(08-17 격리함과 같은 처방).
    NoticeFirstContact,
    /// 파일 크기 상한 설정(M4 · 08-18 — 발신/수신 각각).
    XferSendMax,
    XferSendMaxDesc,
    XferRecvMax,
    XferRecvMaxDesc,
    Cap100MiB,
    Cap256MiB,
    Cap512MiB,
    Cap1GiB,
    CapUnlimited,
    XferDeclined,
    XferCanceled,
    XferTimeoutReject,
    XferTimeoutCancel,
    XferPeerFailed,
    XferQuarantined,
    XferRisk,
    XferMismatch,
    XferAvg,
    StXferNeedPeer,
    StGroupGone,
    StSessionDropSend,
    StNoPendingOffer,
    StAutoRevert,
    StChatClosed,
    StContactSaveFail,
    StContactSealFail,
    StContactOpenFail,
    StHistorySealFail,
    StAutoRestart,
    StImeApplied,
    StAutostartOn,
    StAutostartOff,
    StAvatarChanged,
    StFontApplied,
    StGroupNeedSelect,
    StGroupOwnerInvite,
    StGroupAllMembers,
    StGroupOwnerRemove,
    StGroupRemoved,
    StGroupOwnerPolicy,
    StGroupDisband,
    StGroupLeave,
    StGroupBadName,
    StGroupOwnerRename,
    StGroupRenamed,
    StPasteFail,
    StGroupNoMembers,
    StRemoveCancel,
    StResetCancel,
    StInviteDeclined,
    StNoActiveXfer,
    StSessionEnded,
    StCopied,
    StPeerAccepted,
    StSessionEndedPeer,
    StNetChanged,
    StWireAvatarReady,
    PortLabel,
    // ── 경로 2축(M5-3b/c · ADR-0006 §5-1) — 원격 경유 표시·파일 게이트·경로 무효화 ──
    NoticeRemotePath,
    StRemoteFileBlocked,
    AlertCannotSendFile,
    RemoteFileNeedVerify,
    StRemoteInboundDropped,
    AlertRemoteReqTitle,
    StfRemoteReqBody,
    BtnAccept,
    BtnDecline,
    PathRemoteLabel,
    PathServerLabel,
    /// 경로 배지 — 로컬(같은 망 · 승인 창 식별용).
    PathLocalLabel,
    /// 설정: 원격 경로 파일 발신 허용(08-22 — 기본 끄기 · 발신만).
    RemoteFilesOpt,
    /// 위 설정 설명.
    RemoteFilesOptDesc,
    /// 파일 발신 차단 안내 — 서버 경유(옵션 안내 포함 · 08-22).
    RemoteFileServerOff,
    /// 파일 발신 차단 안내 — 인터넷 직결(지문 대조 우선 안내).
    RemoteFileInternetOff,
    // ── Managed 서버 접속(X-2b · ADR-0013 §12-1) — 상태·핀 경고 ──
    StfServerAttached,
    StfServerFirstPin,
    StServerPinWriteFail,
    StfServerRetry,
    StServerResolveFail,
    StServerLost,
    StServerDetached,
    AlertServerPinTitle,
    StfServerPinMismatch,
    StFingerprintNeedsServer,
    Port47300,
    ServerAddrDefault,
    ServerAnnounce,
    ServerAnnounceDesc,
    CopyFpFull,
    CopyFpShort,
    StFpCopied,
    ServerTest,
    ServerTestDesc,
    ServerTestVerb,
    StServerTestNeedManaged,
    StfServerTesting,
    StfServerTestOk,
    StfServerTestFail,
    StNoteSrvOff,
    StfNoteSrvOk,
    StfNoteSrvConnecting,
    StfNoteSrvRetry,
    StNoteSrvPinStop,
    StNoteSrvIdle,
    StNoteSrvPrevVerified,
    StNoteSrvNeedTest,
    StfNoteSrvTestFailed,
    StfNoteSrvPort,
    StNoteSrvTypeRelay,
    // ── 격리 아카이브 내용 목록(M4-4 ⓐ) — 해제 없는 중앙 디렉터리 목록 ──
    ArchiveSummary,
    ArchiveMore,
    ArchiveUnreadable,
    ArchiveLinkTag,
    StfPathInvalidated,
    StfCannotSend,
    StfFileReadFail,
    StfFileOffer,
    StfAcceptRecv,
    StfAutoAcceptStart,
    StfConnectingBusy,
    StfConnecting,
    StfNewMsgUnread,
    StfConnectingProfile,
    StfSettingsSaveFail,
    StfWindowMode,
    StfColorApplied,
    StfListRefresh,
    StfTooltipDelay,
    StfDisplayName,
    StfPortListening,
    StfPortApplied,
    StfRestartFail,
    StfAutostartFail,
    /// 오프라인 대기 저장(M4-6 · 08-20) — 한계(내 PC가 켜져 있어야) 명시.
    StfQueuedSaved,
    /// 오프라인 대기 전달 완료.
    StfQueuedFlushed,
    /// 클립보드 이미지 없음/변환 실패(③ 08-20).
    StClipImageNone,
    StfInvitesSent,
    StfGroupSaveFail,
    StfGroupPendingSent,
    StfJoinedGroup,
    StfInviteDeclinedBy,
    StfRemovedFromGroup,
    StfGroupNewMsg,
    StfSentSeq,
    StfVerified,
    StfUnverified,
    /// 그룹 행·구성원 모달 부제 — `{0}` = 구성원 수, `{1}` = 온라인 수.
    ListGroupMembers,
    /// "키 지문" 라벨(프로필·상대 카드 공용 · 08-17).
    FingerprintLabel,
    /// 소개글 필드 라벨(프로필 · 08-17).
    BioLabel,
    /// 소개글 필드 placeholder(08-17).
    BioPlaceholder,
    /// 상대 프로필 카드 — 라벨·안내(08-17 i18n).
    CardLastSeen,
    CardLastChat,
    CardPrivate,
    CardNoRecord,
    CardImageCached,
    /// `{0}` = 수신 시각 상대 표기.
    CardImageCachedAt,
    /// `{0}` = 수신 시각 상대 표기.
    CardProfileAt,
    CardImageNone,
    CardVerifyPrompt,
    CardVerified,
    CardVerifyBtn,
    CardUnverifyBtn,
    CardFooter,
    /// 창 타이틀(08-17 i18n) — `format!("Nexa Beep — {}", t(..))`로 쓴다.
    WinAlert,
    WinPeerProfile,
    WinMembers,
    WinGroup,
    WinConnectAddr,
    WinConfirm,
    WinFileRequest,
    WinTransferWait,
    /// `{0}` = 새 메시지 수.
    WinNewMessages,
    /// `{0}` = 파일명. 미리보기 창.
    WinPreview,
    /// 상대 시각(08-17 i18n) — `{0}` = 수(AgoJustNow 제외).
    AgoJustNow,
    AgoMinutes,
    AgoHours,
    AgoDays,
    AgoMonths,
    AgoYears,
    /// 하단 상태바 기동 안내(08-17 i18n).
    HintDemo,
    HintDiscovery,
    HintDiscoveryDegraded,
    HintOpenChat,
    HintNewWindow,
    HintAddAddr,
    HintSettings,
    HintGallery,
    HintTrustLocked,
    HintGroupLocked,
    /// 잠긴 세그먼트를 보관 이동하고 새로 시작(08-19 — 신원 교체 뒤 자가 복구).
    HintTrustArchived,
    HintGroupArchived,
    /// 프로필 수신 상태(08-17 i18n) — `{0}` = 상대 이름, `{1}` = 받은 항목 목록.
    ProfileReceived,
    /// 받은 항목 이름(프로필 수신 요약 · `·` 로 이어 붙인다).
    ItemName,
    ItemEmail,
    ItemPhone,
    ItemImage,
    ItemBio,
    /// 구성원 표기 설명(08-17 i18n) — `표시이름 — 지문 · {설명}`.
    MemberSelf,
    MemberOwner,
    /// 구성원 목록 — 아직 수락 안 한(초대 대기) 구성원(M5-1 · 08-19).
    MemberPending,
    /// 초대받았지만 아직 수락 안 한 그룹 행 표식(클릭 = 수락 · M5-1 · 08-19).
    GroupInvitedTag,
    /// 설정 Files 페이지 속도 노트(08-17 i18n).
    RateSendFloor, // `{0}` = 하한
    RateSendMeasured,  // `{0}` = 실측 최고, `{1}` = 자동 목표
    RateRecvUnclaimed, // (인자 없음)
    RateRecvMeasured,  // `{0}` = 실측 최고
    /// 자동 수락 카운트다운 — `{0}`시작 `{1}`경과 `{2}`잔여 `{3}`종료.
    AutoAcceptCountdown,
    /// 우클릭 메뉴(08-17 i18n) — 목록/그룹.
    MenuProfile,
    MenuPinTop,
    MenuUnpin,
    /// `{0}` = 선택 수.
    MenuCreateGroup,
    MenuForget,
    /// `{0}` = 최근 접속 상대 시각. 삭제 옆 시각 표기.
    MenuForgetAt,
    MenuGroupRename,
    /// `{0}` = 선택 수.
    MenuGroupInvite,
    /// `{0}` = 선택 수.
    MenuGroupRemoveMembers,
    MenuGroupPolicyToOwner,
    MenuGroupPolicyToMembers,
    MenuGroupDisband,
    MenuGroupLeave,
    /// 주소로 연결 다이얼로그(08-17 i18n).
    AddrTitle,
    AddrPlaceholder,
    AddrConnect,
    /// `{0}`·`{1}`·`{2}` = 기본 포트(3회).
    AddrExample,
    AddrEnterConnect,
    AddrFormatHint,
    /// About 페이지(08-17 i18n).
    AboutTagline,
    AboutHomepage,
    /// 대화 열림 상태바(08-18 i18n).
    ChatOpenedProfile, // `{0}` = 연락처 목록
    ChatOpenedSession,
    StItemImagePresent,
    /// 명령(/help 등) 안내·응답(08-18 i18n).
    CmdHelpHeader,
    CmdHelpHelp,
    CmdHelpVerify,
    CmdHelpUnverify,
    CmdHelpTrust,
    CmdHelpFingerprint,
    CmdHelpClose,
    /// 등급 명령 안내(④ 08-20).
    CmdHelpNotice,
    CmdHelpUrgent,
    /// 등급 명령 빈 본문 사용법.
    StfGradeUsage,
    /// 긴급 선택됨(배지 순환 마찰 1단계).
    StUrgentArmed,
    /// 그룹 방 등급 미지원 안내.
    StGradeGroupUnsupported,
    /// 공지 발송 요약(즉시/대기).
    StfBroadcastSent,
    /// 메뉴 — 공지 보내기.
    MenuBroadcast,
    /// 공지 입력 창 제목.
    BroadcastTitle,
    CmdHelpNote,
    /// `/fingerprint` 출력(08-18) — `{0}` = 내 지문, `{1}` = 상대 이름, `{2}` = 상대 지문.
    CmdFingerprint,
    /// `/verify` 직접 검증 완료(08-18).
    CmdVerifiedNow,
    CmdUnknown, // `{0}` = 이름
    CmdTrustGroup,
    CmdTrustStatus, // `{0}` = 상대, `{1}` = 등급
    CmdVerifyAlready,
    CmdVerifyOpened,
    CmdVerify1to1,
    CmdUnverifyDone,
    CmdUnverifyNone,
    CmdUnverify1to1,
    /// 목록 복귀 안내(08-18 i18n · "(한글 가능)"만 한글 고정 유지).
    ListNavHint,
    /// 지문 대조 추천 안내(08-18 i18n · `/verify` suggest).
    SuggestVerify,
    /// 확인 버튼(모달 OK · 08-18 i18n).
    ActOk,
    /// 전송 취소 버튼(08-18 i18n · `{0}` = 파일명).
    XferCancelBtn,
    /// 전송 배치 대기 안내(M4-2d · 08-19).
    XferWaitApproval,
    /// 전송 배치 요약 — `{0}` = 파일 개수 · `{1}` = 총 용량(M4-2d · 08-19).
    XferBatchSummary,
    /// 전송 상태 라벨(M4-2d · 08-19) — 목록 행.
    XferStWaiting,
    /// 현재 파일 — 오퍼 발신·상대 승인 대기(M4-2d · 08-19).
    XferStOffered,
    XferStActive,
    XferStPaused,
    /// 파일 단위 용량 검사에서 제외된 파일(M4-2e · 08-19) — 전송 안 하지만 목록엔 남는다.
    XferExcluded,
    XferStDone,
    XferStFailed,
    StfAutoAcceptRecv,
    StfFileRejected,
    StfDeliveredWait,
    StfFileWhy,
    StfTrustReject,
    StfConnectedOpen,
    StfManualConnFail,
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
            Msg::SendDelivered => [
                "Send delivery receipts",
                "수신 확인 보내기",
                "发送送达回执",
                "配信確認を送る",
            ],
            Msg::SendDeliveredDesc => [
                "Let senders know their message reached you (verified peers only). Off = you stay silent.",
                "상대가 보낸 메시지가 나에게 닿았음을 알립니다(검증된 상대만). 끄면 확인을 보내지 않습니다.",
                "让发送者知道消息已送达（仅限已验证的对方）。关闭 = 不发送回执。",
                "相手のメッセージが届いたことを知らせます（検証済みの相手のみ）。オフ = 送りません。",
            ],
            Msg::SendRead => ["Send read receipts", "읽음 확인 보내기", "发送已读回执", "既読を送る"],
            Msg::SendReadDesc => [
                "Let senders know you read their message when you open the chat (verified peers only). Independent of delivery. Off = you stay silent.",
                "대화창을 열어 상대의 메시지를 읽으면 읽음을 알립니다(검증된 상대만). 전달 확인과 독립입니다. 끄면 읽음을 보내지 않습니다.",
                "打开对话读取消息时通知对方已读（仅限已验证的对方）。与送达独立。关闭 = 不发送。",
                "チャットを開いて読むと既読を知らせます（検証済みの相手のみ）。配信とは独立。オフ = 送りません。",
            ],
            Msg::XferDeclined => ["Declined", "거절함", "已拒绝", "拒否しました"],
            Msg::XferCanceled => ["Canceled", "취소함", "已取消", "キャンセルしました"],
            Msg::XferFileTag => ["[File]", "[파일]", "[文件]", "[ファイル]"],
            Msg::XferWaiting => ["Awaiting approval", "승인 대기", "等待批准", "承認待ち"],
            Msg::XferDirSend => ["Sending", "전송", "发送", "送信"],
            Msg::XferDirRecv => ["Receiving", "수신", "接收", "受信"],
            Msg::XferAwaitAck => [
                "Delivered · awaiting confirm",
                "전달됨 · 확인 대기",
                "已送达 · 等待确认",
                "送達 · 確認待ち",
            ],
            Msg::XferDoneLbl => ["Done", "완료", "完成", "完了"],
            Msg::XferFailLbl => ["Failed", "실패", "失败", "失敗"],
            Msg::XferSrcReadFail => ["Stopped — cannot read source file", "원본 파일을 읽을 수 없어 전송을 중단했습니다", "已中止 — 无法读取源文件", "中断 — 元ファイルを読めません"],
            Msg::XferSrcChanged => ["Stopped — source changed during transfer", "전송 중 원본이 변경되어 중단했습니다", "已中止 — 传输中源文件被修改", "中断 — 転送中に元ファイルが変更"],
            Msg::XferCancelAll => ["Cancel all", "전체취소", "全部取消", "全て取消"],
            Msg::XferPeerCanceledAll => ["Peer canceled all", "상대가 전체 취소", "对方已全部取消", "相手が全てキャンセル"],
            Msg::XferStaleAutoCancel => ["Paused over 10 min — canceled all", "10분 이상 일시중지 방치 — 전체 취소", "暂停超过10分钟 — 已全部取消", "10分以上一時停止 — 全て取消"],
            Msg::XferAutoCancelIn => ["auto-cancel in {}", "{} 후 자동 취소", "{} 后自动取消", "{} 後に自動取消"],
            Msg::XferPeerCanceled => [
                "Peer canceled",
                "상대가 취소",
                "对方已取消",
                "相手がキャンセル",
            ],
            Msg::XferResumeFrom => [
                "Resume — {} from {}%",
                "이어받기 — {} {}%부터",
                "续传 — {} 从 {}%",
                "再開 — {} {}%から",
            ],
            Msg::XferSendMax => [
                "Max send file size",
                "보내기 파일 상한",
                "发送文件上限",
                "送信ファイル上限",
            ],
            Msg::XferSendMaxDesc => [
                "Checked before sending — larger files are skipped. Sending streams from disk, so Unlimited is safe",
                "보내기 전에 검사해 초과 파일은 거릅니다. 발신은 디스크 스트리밍이라 무제한도 안전합니다",
                "发送前检查，超限文件将被跳过。发送为磁盘流式，无限制也安全",
                "送信前に検査し、超過ファイルは除外。送信はディスクストリーミングのため無制限でも安全",
            ],
            Msg::XferRecvMax => [
                "Max receive file size",
                "받기 파일 상한",
                "接收文件上限",
                "受信ファイル上限",
            ],
            Msg::XferRecvMaxDesc => [
                "Offers larger than this are auto-rejected (cap is announced to sender)",
                "초과 제안은 자동 거절되고 상한이 발신자에게 공지됩니다",
                "超限的提议将被自动拒绝（上限会告知发送方）",
                "超過する提案は自動拒否（上限は送信側へ通知）",
            ],
            Msg::Cap100MiB => ["100 MiB", "100 MiB", "100 MiB", "100 MiB"],
            Msg::Cap256MiB => ["256 MiB", "256 MiB", "256 MiB", "256 MiB"],
            Msg::Cap512MiB => ["512 MiB", "512 MiB", "512 MiB", "512 MiB"],
            Msg::Cap1GiB => ["1 GiB", "1 GiB", "1 GiB", "1 GiB"],
            Msg::CapUnlimited => ["Unlimited", "무제한", "无限制", "無制限"],
            Msg::XferPeerCapBlock => [
                "over peer receive cap {}",
                "상대 수신 상한 {} 초과",
                "超出对方接收上限 {}",
                "相手の受信上限 {} 超過",
            ],
            Msg::XferPeerCapBlockStatus => [
                "{} exceeds peer receive cap {} — not sent",
                "{} — 상대 수신 상한 {} 초과로 보내지 않았습니다",
                "{} 超出对方接收上限 {} — 未发送",
                "{} は相手の受信上限 {} を超えるため送信しません",
            ],
            Msg::XferHashingPct => [
                "Preparing {} — {}%",
                "{} 준비 중 — {}%",
                "正在准备 {} — {}%",
                "{} を準備中 — {}%",
            ],
            Msg::StfSendQueued => [
                "Queued {} file(s) · total {}",
                "전송 대기 {}건 · 총 {}",
                "排队 {} 个文件 · 共 {}",
                "送信待ち {}件 · 合計 {}",
            ],
            Msg::XferPeerRejected => [
                "Peer rejected: {}{}",
                "상대가 거절: {}{}",
                "对方拒绝：{}{}",
                "相手が拒否: {}{}",
            ],
            Msg::PeerCapSuffix => [
                " (peer receive cap {})",
                " (상대 수신 상한 {})",
                "（对方接收上限 {}）",
                "（相手の受信上限 {}）",
            ],
            Msg::QLoading => [
                "Scanning quarantine…",
                "격리함을 불러오는 중…",
                "正在扫描隔离区…",
                "隔離ボックスを読み込み中…",
            ],
            Msg::XferTooBigWhy => [
                "over cap {}",
                "상한 {} 초과",
                "超出上限 {}",
                "上限 {} 超過",
            ],
            Msg::XferHashing => [
                "Preparing {} (integrity hash) — large files take a while",
                "{} 준비 중(무결성 해시) — 대용량은 수십 초 걸립니다",
                "正在准备 {}（完整性哈希）— 大文件需要一些时间",
                "{} を準備中（整合性ハッシュ）— 大容量は時間がかかります",
            ],
            Msg::NoticeFirstContact => [
                "! You have not exchanged messages with this peer yet — they must approve receiving before the transfer proceeds",
                "! 아직 서로 메시지를 주고받은 적이 없는 상대입니다 — 상대가 수신 승인을 눌러야 전송이 진행됩니다",
                "! 你与该用户尚未互发过消息 — 对方需批准接收后传输才会进行",
                "! この相手とはまだメッセージを交わしていません — 相手が受信を承認すると転送が始まります",
            ],
            Msg::XferTooBigLocal => [
                "{} is too large — cap {} per file",
                "{} — 파일당 상한 {}을 넘습니다",
                "{} 过大 — 单文件上限 {}",
                "{} が大きすぎます — 1ファイル上限 {}",
            ],
            Msg::XferReofferN => [
                "Re-offering {} interrupted transfer(s)",
                "중단 전송 재제안 — {}건 (이어받기 협상)",
                "重新提议 {} 个中断的传输",
                "中断転送を再提案 — {}件",
            ],
            Msg::XferTimeoutReject => [
                "No response — declined on timeout",
                "응답 없음 — 시간 초과로 거절",
                "无响应 — 超时拒绝",
                "応答なし — タイムアウトで拒否",
            ],
            Msg::XferTimeoutCancel => [
                "No response — canceled on timeout",
                "응답 없음 — 시간 초과로 취소",
                "无响应 — 超时取消",
                "応答なし — タイムアウトでキャンセル",
            ],
            Msg::XferPeerFailed => [
                "Peer could not receive (integrity/save failed)",
                "상대가 받지 못함(무결성·저장 실패)",
                "对方无法接收（完整性/保存失败）",
                "相手が受信できませんでした（整合性・保存失敗）",
            ],
            Msg::XferQuarantined => ["Quarantined", "격리됨", "已隔离", "隔離済み"],
            Msg::XferRisk => ["risk", "위험", "风险", "リスク"],
            Msg::XferMismatch => ["type mismatch", "형식 불일치", "类型不匹配", "形式不一致"],
            Msg::XferAvg => ["avg", "평균", "平均", "平均"],
            Msg::StXferNeedPeer => ["Open a conversation first to send a file — select a peer", "파일 전송은 대화를 연 뒤에 — 상대를 먼저 선택하세요", "请先打开对话再发送文件 — 先选择对方", "ファイル送信は会話を開いてから — 先に相手を選択"],
            Msg::StGroupGone => ["Group no longer exists (disbanded)", "그룹이 없습니다(해산됨)", "群组已不存在（已解散）", "グループがありません（解散済み）"],
            Msg::StSessionDropSend => ["Session dropped — cannot send", "세션이 끊겨 전송할 수 없습니다", "会话已断开 — 无法发送", "セッションが切断され送信できません"],
            Msg::StNoPendingOffer => ["No file offer awaiting your response", "수락 대기 중인 파일 제안이 없습니다", "没有待接受的文件邀请", "承認待ちのファイル提案はありません"],
            Msg::StAutoRevert => ["Auto-accept period ended — reverted to previous mode", "자동 수락 기간이 끝나 직전 방식으로 되돌렸습니다", "自动接收时段结束 — 已恢复之前的方式", "自動受信期間が終了し前の方式に戻りました"],
            Msg::StChatClosed => ["Closed the chat (the conversation is kept)", "대화창을 닫았습니다(대화는 유지됩니다)", "已关闭对话窗口（对话保留）", "チャットを閉じました（会話は保持されます）"],
            Msg::StContactSaveFail => ["⚠ Failed to save protected contacts — will retry on next change", "⚠ 연락처 보호 저장 실패 — 다음 변경 때 재시도", "⚠ 联系人保护保存失败 — 下次更改时重试", "⚠ 連絡先の保護保存に失敗 — 次の変更時に再試行"],
            Msg::StContactSealFail => ["⚠ Contact sealing failed — not saved", "⚠ 연락처 봉인 실패 — 저장하지 않음", "⚠ 联系人封装失败 — 未保存", "⚠ 連絡先の封印に失敗 — 保存しません"],
            Msg::StContactOpenFail => ["⚠ Cannot open protected contacts (different identity or corrupt)", "⚠ 연락처 보호 파일을 열 수 없음(다른 신원·손상)", "⚠ 无法打开受保护的联系人（不同身份或损坏）", "⚠ 連絡先の保護ファイルを開けません（別の身元・破損）"],
            Msg::StHistorySealFail => ["⚠ Conversation history sealing failed — not saved", "⚠ 대화 기록 봉인 실패 — 저장하지 않음", "⚠ 对话记录封装失败 — 未保存", "⚠ 会話履歴の封印に失敗 — 保存しません"],
            Msg::StAutoRestart => ["Restarted the auto-accept period", "자동 수락 기간을 다시 시작했습니다", "已重新开始自动接收时段", "自動受信期間を再開しました"],
            Msg::StImeApplied => ["Korean input (IME) settings applied", "한글 입력(IME) 기준값 적용", "已应用韩语输入（IME）设置", "韓国語入力（IME）設定を適用"],
            Msg::StAutostartOn => ["Start at login: on — registered with the OS", "시스템 시작 시 자동 실행: 켬 — OS에 등록했습니다", "登录时自动启动：开 — 已注册到系统", "ログイン時自動起動: オン — OSに登録しました"],
            Msg::StAutostartOff => ["Start at login: off — removed the OS registration", "시스템 시작 시 자동 실행: 끔 — OS 등록을 제거했습니다", "登录时自动启动：关 — 已移除系统注册", "ログイン時自動起動: オフ — OS登録を解除しました"],
            Msg::StAvatarChanged => ["Avatar changed — propagated to connected peers", "아바타 변경 — 연결된 상대에게 반영", "头像已更改 — 已同步给已连接的对方", "アバター変更 — 接続中の相手に反映"],
            Msg::StFontApplied => ["Font settings applied", "글꼴 설정 적용됨", "已应用字体设置", "フォント設定を適用しました"],
            Msg::StGroupNeedSelect => ["To make a group, ⌘/Ctrl-click to select peers first", "그룹을 만들려면 ⌘/Ctrl+클릭으로 상대를 먼저 선택하세요", "创建群组请先用 ⌘/Ctrl+点击选择对方", "グループ作成は ⌘/Ctrl+クリックで先に相手を選択"],
            Msg::StGroupOwnerInvite => ["Only the owner can invite to this room", "이 방은 소유자만 초대할 수 있습니다", "此房间仅所有者可邀请", "この部屋は所有者のみ招待できます"],
            Msg::StGroupAllMembers => ["Everyone is already a member", "이미 전원 구성원입니다", "已经全员是成员", "すでに全員がメンバーです"],
            Msg::StGroupOwnerRemove => ["Only the owner can remove members", "구성원 제외는 소유자만 할 수 있습니다", "仅所有者可移除成员", "メンバー除外は所有者のみ可能"],
            Msg::StGroupRemoved => ["Removed — distributed to members", "제외 완료 — 구성원에게 배포", "已移除 — 已分发给成员", "除外完了 — メンバーに配布"],
            Msg::StGroupOwnerPolicy => ["Only the owner can change room policy", "방 정책은 소유자만 바꿀 수 있습니다", "仅所有者可更改房间策略", "部屋のポリシーは所有者のみ変更できます"],
            Msg::StGroupDisband => ["Group disbanded — members notified", "그룹 해산 — 구성원에게 통지", "群组已解散 — 已通知成员", "グループ解散 — メンバーに通知"],
            Msg::StGroupLeave => ["Left the group — owner notified", "그룹 탈퇴 — 소유자에게 통지", "已退出群组 — 已通知所有者", "グループ退出 — 所有者に通知"],
            Msg::StGroupBadName => ["That character can't be used in a group name", "그룹 이름으로 쓸 수 없는 문자입니다", "该字符不能用于群组名称", "グループ名に使えない文字です"],
            Msg::StGroupOwnerRename => ["Only the owner can rename the group", "그룹 이름은 소유자만 바꿀 수 있습니다", "仅所有者可重命名群组", "グループ名は所有者のみ変更できます"],
            Msg::StGroupRenamed => ["Group renamed — distributed to members", "그룹 이름 변경됨 — 구성원에게 배포", "群组已重命名 — 已分发给成员", "グループ名を変更 — メンバーに配布"],
            Msg::StPasteFail => ["Paste failed — cannot read clipboard", "붙여넣기 실패 — 클립보드를 읽을 수 없습니다", "粘贴失败 — 无法读取剪贴板", "貼り付け失敗 — クリップボードを読めません"],
            Msg::StGroupNoMembers => ["No other members in this room yet", "이 방에는 아직 다른 구성원이 없습니다", "此房间还没有其他成员", "この部屋にはまだ他のメンバーがいません"],
            Msg::StRemoveCancel => ["Member removal canceled", "구성원 제외 취소", "已取消移除成员", "メンバー除外をキャンセル"],
            Msg::StResetCancel => ["Settings reset canceled", "설정 초기화 취소", "已取消设置重置", "設定リセットをキャンセル"],
            Msg::StInviteDeclined => ["Declined the group invitation", "그룹 초대를 거절했습니다", "已拒绝群组邀请", "グループ招待を拒否しました"],
            Msg::StNoActiveXfer => ["No active transfer to cancel", "취소할 진행 중 전송이 없습니다", "没有可取消的进行中传输", "キャンセルする進行中の転送がありません"],
            Msg::StSessionEnded => ["Session ended", "세션 종료됨", "会话已结束", "セッション終了"],
            Msg::StCopied => ["Copied", "복사됨", "已复制", "コピーしました"],
            Msg::StPeerAccepted => ["Peer accepted — starting transfer", "상대가 수락 — 전송 시작", "对方已接受 — 开始传输", "相手が承認 — 転送開始"],
            Msg::StSessionEndedPeer => ["Session with peer ended", "상대와의 세션이 종료됨", "与对方的会话已结束", "相手とのセッションが終了"],
            Msg::StNetChanged => ["Network change detected — rediscovering and reconnecting", "네트워크 변경 감지 — 재발견·재연결 중", "检测到网络变化 — 正在重新发现和连接", "ネットワーク変更を検知 — 再発見・再接続中"],
            Msg::StWireAvatarReady => ["Profile photo thumbnail ready — propagating to peers", "프로필 사진 축소본 준비 — 연결된 상대에게 전파", "头像缩略图已就绪 — 正在传播给对方", "プロフィール写真の縮小版を用意 — 相手に伝播"],
            Msg::PortLabel => ["Listening :", "수신 :", "监听 :", "受信 :"],
            Msg::NoticeRemotePath => ["This conversation goes over the internet — files stay blocked until fingerprints (SAS) are verified.", "이 대화는 인터넷 경유입니다 — 지문(SAS) 대조 전에는 파일을 주고받을 수 없습니다.", "此对话经由互联网 — 完成指纹（SAS）核对前无法收发文件。", "この会話はインターネット経由です — 指紋（SAS）照合前はファイルを送受信できません。"],
            Msg::StRemoteFileBlocked => ["File blocked: internet path — verify fingerprints first (/verify)", "파일 차단: 인터넷 경유 — 지문 대조(/verify)가 먼저입니다", "文件已被阻止：互联网路径 — 请先核对指纹（/verify）", "ファイルをブロック: インターネット経由 — まず指紋照合（/verify）"],
            Msg::AlertCannotSendFile => ["Cannot send file", "파일을 보낼 수 없습니다", "无法发送文件", "ファイルを送信できません"],
            Msg::RemoteFileNeedVerify => ["This peer is connected over the internet. Files are blocked until you verify fingerprints (SAS) — the network path is outside your local trust boundary.\n\nRun /verify in the chat, compare the code with the other person, then try again.", "이 상대는 인터넷을 경유해 연결되어 있습니다. 로컬 신뢰 경계 밖의 경로라, 지문(SAS) 대조 전에는 파일 전송이 차단됩니다.\n\n대화창에서 /verify 를 실행해 상대와 코드를 맞춰 본 뒤 다시 시도하세요.", "对方经由互联网连接。该路径在本地信任边界之外，完成指纹（SAS）核对前文件传输将被阻止。\n\n请在聊天中运行 /verify，与对方核对代码后重试。", "この相手はインターネット経由で接続されています。ローカル信頼境界の外の経路のため、指紋（SAS）照合まではファイル転送がブロックされます。\n\nチャットで /verify を実行し、相手とコードを照合してから再試行してください。"],
            Msg::StRemoteInboundDropped => ["Blocked an unregistered inbound connection from outside the local network", "로컬 네트워크 밖에서 온 미등록 상대의 연결을 차단했습니다", "已阻止来自本地网络之外的未注册连接", "ローカルネットワーク外からの未登録の接続をブロックしました"],
            Msg::AlertRemoteReqTitle => ["Remote connection request", "원격 연결 요청", "远程连接请求", "リモート接続リクエスト"],
            Msg::StfRemoteReqBody => ["'{}' is requesting a connection from outside your local network.\nAccepting registers this peer; files stay blocked until fingerprints are verified.", "'{}' 상대가 로컬 네트워크 밖에서 연결을 요청했습니다.\n수락하면 목록에 등록됩니다 — 지문 대조 전에는 파일이 차단됩니다.", "'{}' 正在从本地网络之外请求连接。\n接受后将注册该对方；完成指纹核对前文件仍被阻止。", "'{}' がローカルネットワーク外から接続をリクエストしています。\n受諾すると一覧に登録されます — 指紋照合まではファイルはブロックされます。"],
            Msg::BtnAccept => ["Accept", "수락", "接受", "受諾"],
            Msg::BtnDecline => ["Decline", "거절", "拒绝", "拒否"],
            Msg::PathRemoteLabel => ["via internet", "인터넷 경유", "经由互联网", "インターネット経由"],
            Msg::PathServerLabel => ["via server", "서버 경유", "经由服务器", "サーバー経由"],
            Msg::PathLocalLabel => ["via local", "로컬(같은 망)", "本地(同一网络)", "ローカル(同一LAN)"],
            Msg::RemoteFilesOpt => ["Files on remote paths", "원격 경로 파일 전송", "远程路径文件传输", "リモート経路のファイル送信"],
            Msg::RemoteFilesOptDesc => ["Allow SENDING files in via-server / via-internet chats (default off). Receiving is never restricted — every incoming request still asks you, with the path clearly shown.", "서버 경유·인터넷 직결 대화에서 파일 발신을 허용합니다(기본 끄기). 수신은 제한하지 않습니다 — 요청마다 경로가 표시된 승인 창에서 직접 결정합니다.", "允许在经由服务器/互联网的对话中发送文件（默认关闭）。接收不受限 — 每个请求都会弹出标明路径的确认窗口。", "サーバー経由/インターネット経由の会話でのファイル送信を許可（既定オフ）。受信は制限せず、経路が明示された承認画面で毎回判断します。"],
            Msg::RemoteFileServerOff => ["This chat goes via server — file sending is off. Turn on Settings › Server › 'Files on remote paths' to send.", "서버 경유 대화라 파일 전송이 꺼져 있습니다. 설정 › Server › '원격 경로 파일 전송'을 켜면 보낼 수 있습니다.", "此对话经由服务器 — 文件发送已关闭。在 设置 › Server › '远程路径文件传输' 中开启即可发送。", "この会話はサーバー経由のためファイル送信はオフです。設定 › Server › 'リモート経路のファイル送信' をオンにすると送信できます。"],
            Msg::RemoteFileInternetOff => ["This chat goes over the internet — files stay blocked until fingerprints are verified (/fingerprint → /verify). You can also enable Settings › Server › 'Files on remote paths'.", "인터넷 직결 대화라 파일 전송이 차단되어 있습니다 — /fingerprint로 지문을 확인하고 다른 채널로 대조한 뒤 /verify 하면 열립니다. 설정 › Server › '원격 경로 파일 전송'으로도 켤 수 있습니다.", "此对话直连互联网 — 文件在指纹核对（/verify）前保持封锁。也可在 设置 › Server › '远程路径文件传输' 中开启。", "この会話はインターネット直結のため、指紋照合（/verify）までファイルは遮断されます。設定 › Server › 'リモート経路のファイル送信' でも有効化できます。"],
            Msg::StfServerAttached => ["Server registered — {}", "서버 등록 완료 — {}", "服务器注册完成 — {}", "サーバー登録完了 — {}"],
            Msg::StfServerFirstPin => ["First contact with server {} — its key is now pinned", "서버 {} 첫 접속 — 서버 키를 핀했습니다", "首次连接服务器 {} — 已固定其密钥", "サーバー {} 初回接続 — キーをピン留めしました"],
            Msg::StServerPinWriteFail => ["Warning: server pin could not be saved — the next connect will look like first contact", "경고: 서버 핀 저장 실패 — 다음 접속이 다시 첫 접속으로 보입니다", "警告：服务器密钥固定保存失败 — 下次连接将再次视为首次连接", "警告: サーバーピンの保存に失敗 — 次回接続は初回接続として扱われます"],
            Msg::StfServerRetry => ["Server connection failed ({}) — retrying in {}s", "서버 접속 실패({}) — {}초 후 재시도", "服务器连接失败（{}）— {} 秒后重试", "サーバー接続失敗（{}）— {} 秒後に再試行"],
            Msg::StServerResolveFail => ["address lookup failed", "주소 해석 실패", "地址解析失败", "アドレス解決失敗"],
            Msg::StServerLost => ["Server connection lost — reconnecting", "서버 연결 끊김 — 재접속을 시도합니다", "服务器连接已断开 — 正在重连", "サーバー接続が切断 — 再接続します"],
            Msg::StServerDetached => ["Server detached — LAN only (unmanaged)", "서버 접속 해제 — LAN만 사용합니다(Unmanaged)", "已断开服务器 — 仅使用局域网（Unmanaged）", "サーバー接続を解除 — LAN のみ使用（Unmanaged）"],
            Msg::AlertServerPinTitle => ["Server key mismatch", "서버 키 불일치", "服务器密钥不匹配", "サーバーキー不一致"],
            Msg::StFingerprintNeedsServer => ["Fingerprint connect needs a registered server (Settings › Server, managed)", "지문 연결은 서버가 등록·연결된 상태에서만 가능합니다(설정 › 서버 · Managed)", "指纹连接需要已注册并连接的服务器（设置 › 服务器 · managed）", "指紋接続には登録・接続済みサーバーが必要です（設定 › サーバー · managed）"],
            Msg::Port47300 => ["47300", "47300", "47300", "47300"],
            Msg::ServerAddrDefault => ["beepd.sosomlab.com", "beepd.sosomlab.com", "beepd.sosomlab.com", "beepd.sosomlab.com"],
            Msg::ServerAnnounce => ["Show me to server users", "서버 사용자에게 나를 표시", "向服务器用户显示我", "サーバー利用者に自分を表示"],
            Msg::CopyFpFull => ["Copy key (64)", "전체 지문 복사", "复制完整指纹", "全指紋をコピー"],
            Msg::CopyFpShort => ["Copy short (8)", "짧은 지문 복사", "复制短指纹", "短い指紋をコピー"],
            Msg::StFpCopied => ["Fingerprint copied to clipboard", "지문을 클립보드에 복사했습니다", "指纹已复制到剪贴板", "指紋をクリップボードにコピーしました"],
            Msg::ServerAnnounceDesc => ["Adds you to the shared user list of this server, and shows that list here (mutual — only listed users see each other). Only your key is shared; name and profile still travel peer-to-peer after you connect.", "이 서버의 공개 사용자 목록에 나를 싣고, 그 목록을 내 목록에 표시합니다(상호 — 공개한 사용자끼리만 보입니다). 실리는 것은 키뿐이며 이름·프로필은 연결 후에 상대와 직접 교환됩니다.", "将你加入该服务器的公开用户列表，并在此显示该列表（互相 — 仅公开的用户彼此可见）。仅共享密钥；名称和资料在连接后点对点交换。", "このサーバーの公開ユーザー一覧に自分を載せ、その一覧をここに表示します（相互 — 公開したユーザー同士のみ見えます）。共有されるのはキーのみで、名前・プロフィールは接続後にP2Pで交換されます。"],
            Msg::ServerTest => ["Connection test", "연결 테스트", "连接测试", "接続テスト"],
            Msg::ServerTestDesc => ["Actually connects to the server with the values above and verifies registration. A successful test (or any successful automatic registration) is remembered — no need to test again.", "위 설정값으로 서버에 실제 접속해 등록까지 검증합니다. 성공한 테스트(또는 자동 등록 성공)는 저장되어 다시 누를 필요가 없습니다.", "使用上方设置实际连接服务器并验证注册。成功的测试（或任何成功的自动注册）会被记住 — 无需再次测试。", "上の設定値でサーバーに実際に接続し登録まで検証します。成功したテスト（または自動登録の成功）は保存され、再度押す必要はありません。"],
            Msg::ServerTestVerb => ["Test", "테스트", "测试", "テスト"],
            Msg::StServerTestNeedManaged => ["Test needs Managed mode and a server address", "테스트하려면 Managed 모드와 서버 주소가 필요합니다", "测试需要 Managed 模式和服务器地址", "テストには Managed モードとサーバーアドレスが必要です"],
            Msg::StfServerTesting => ["Testing server {}…", "서버 테스트 중… {}", "正在测试服务器 {}…", "サーバーをテスト中… {}"],
            Msg::StfServerTestOk => ["✓ Server verified — {} (key {}) · saved", "✓ 서버 검증 완료 — {} (키 {}) · 저장됨", "✓ 服务器验证完成 — {}（密钥 {}）· 已保存", "✓ サーバー検証完了 — {}（キー {}）· 保存済み"],
            Msg::StfServerTestFail => ["Server test failed: {}", "서버 테스트 실패: {}", "服务器测试失败：{}", "サーバーテスト失敗: {}"],
            Msg::StNoteSrvOff => ["Server not in use (unmanaged — LAN only)", "서버 미사용(Unmanaged — LAN만)", "未使用服务器（unmanaged — 仅局域网）", "サーバー未使用（unmanaged — LAN のみ）"],
            Msg::StfNoteSrvOk => ["✓ Server verified — {} (key {})", "✓ 서버 검증됨 — {} (키 {})", "✓ 服务器已验证 — {}（密钥 {}）", "✓ サーバー検証済み — {}（キー {}）"],
            Msg::StfNoteSrvConnecting => ["○ Connecting… {}", "○ 접속 중… {}", "○ 正在连接… {}", "○ 接続中… {}"],
            Msg::StfNoteSrvRetry => ["○ Retry in {}s — {}", "○ {}초 후 재시도 — {}", "○ {} 秒后重试 — {}", "○ {} 秒後に再試行 — {}"],
            Msg::StNoteSrvPinStop => ["■ Stopped: server key mismatch — clear the pin line, then re-save server settings", "■ 정지: 서버 키 불일치 — 핀 줄 정리 후 서버 설정을 다시 저장하면 재시도", "■ 已停止：服务器密钥不匹配 — 清理固定行后重新保存服务器设置", "■ 停止: サーバーキー不一致 — ピン行を整理しサーバー設定を保存し直すと再試行"],
            Msg::StNoteSrvIdle => ["○ Not connected", "○ 미접속", "○ 未连接", "○ 未接続"],
            Msg::StNoteSrvPrevVerified => [" · previously verified (reconnect pending)", " · 이전 검증됨(재접속 대기)", " · 此前已验证（等待重连）", " · 以前に検証済み（再接続待ち）"],
            Msg::StNoteSrvNeedTest => ["Not verified — press [Test]; the server is used only after a successful test", "미검증 — [테스트]를 눌러 성공해야 서버를 사용합니다", "未验证 — 按 [测试]，测试成功后才会使用服务器", "未検証 — [テスト] を押し、成功して初めてサーバーを使用します"],
            Msg::StfNoteSrvTestFailed => ["Server connection failed: {}", "서버 연결 실패: {}", "服务器连接失败：{}", "サーバー接続失敗: {}"],
            Msg::StfNoteSrvPort => ["TCP {} connected · UDP observe {} · server sees me as {}", "TCP {} 연결 · UDP 관측 {} · 서버가 본 내 주소 {}", "TCP {} 已连接 · UDP 观测 {} · 服务器所见我的地址 {}", "TCP {} 接続 · UDP 観測 {} · サーバーから見た自分 {}"],
            Msg::StNoteSrvTypeRelay => ["Relay (auto — provided by server: rendezvous + hole-punch + relay fallback)", "Relay (auto — 서버 제공: 랑데부+홀펀칭+릴레이 폴백)", "Relay（auto — 服务器提供：会合+打洞+中继回退）", "Relay（auto — サーバー提供：ランデブー+ホールパンチ+リレー フォールバック）"],
            Msg::StfServerPinMismatch => ["The server '{}' presented a key different from the pinned one — connection aborted (possible impersonation or server reinstall).\n\nIf the server was really replaced, delete its line in:\n{}\nand change any server setting to reconnect. Re-pinning is a human decision — it will not retry automatically.", "서버 '{}'가 핀과 다른 키를 제시했습니다 — 접속을 중단합니다(사칭·서버 재설치 가능성).\n\n서버 교체가 맞다면 아래 파일에서 해당 줄을 지운 뒤,\n{}\n서버 설정을 다시 저장하면 재접속합니다. 재핀은 사람의 결정입니다 — 자동으로 재시도하지 않습니다.", "服务器 '{}' 提供的密钥与已固定的不一致 — 已中止连接（可能是冒充或服务器重装）。\n\n如果确实更换了服务器，请在下列文件中删除对应行：\n{}\n然后重新保存服务器设置以重连。重新固定由人决定 — 不会自动重试。", "サーバー '{}' がピン留めと異なるキーを提示しました — 接続を中止します（なりすまし・サーバー再設置の可能性）。\n\n本当にサーバーを入れ替えた場合は、次のファイルから該当行を削除し、\n{}\nサーバー設定を保存し直すと再接続します。再ピン留めは人の決定です — 自動では再試行しません。"],
            Msg::ArchiveSummary => ["{} file(s) · declared unpacked size {} — listing only, nothing is extracted", "항목 {}개 · 선언 해제 크기 {} — 목록만 읽었고 해제하지 않았습니다", "{} 个条目 · 声明解压大小 {} — 仅读取列表，未解压", "{} 件 · 宣言解凍サイズ {} — 一覧のみ・解凍していません"],
            Msg::ArchiveMore => ["…and {} more", "…외 {}개", "…以及另外 {} 个", "…ほか {} 件"],
            Msg::ArchiveUnreadable => ["Cannot read archive", "아카이브를 읽을 수 없습니다", "无法读取压缩包", "アーカイブを読み取れません"],
            Msg::ArchiveLinkTag => ["(link)", "(링크)", "（链接）", "（リンク）"],
            Msg::StfPathInvalidated =>["Saved address for {} connected as a different identity — removed", "{} 의 저장 주소가 다른 신원으로 성립해 삭제했습니다", "为 {} 保存的地址已以其他身份建立连接 — 已删除", "{} の保存アドレスは別の身元で成立したため削除しました"],
            Msg::StfCannotSend =>["Cannot send — {}", "보낼 수 없습니다 — {}", "无法发送 — {}", "送信できません — {}"],
            Msg::StfFileReadFail => ["Failed to read file: {}", "파일 읽기 실패: {}", "读取文件失败：{}", "ファイル読み込み失敗: {}"],
            Msg::StfFileOffer => ["File offer: {} ({}) — awaiting peer approval", "파일 제안: {} ({}) — 상대 승인 대기", "文件邀请：{}（{}）— 等待对方批准", "ファイル提案: {} ({}) — 相手の承認待ち"],
            Msg::StfAcceptRecv => ["Accepted — receiving {}", "수락 — {} 수신 시작", "已接受 — 开始接收 {}", "承認 — {} の受信開始"],
            Msg::StfAutoAcceptStart => ["Auto-accept on — offers from {} onward accepted automatically", "자동 수락 시작 — {} 포함 이후 제안을 자동 수락합니다", "已开启自动接收 — 从 {} 起自动接受邀请", "自動受信開始 — {} 以降の提案を自動承認"],
            Msg::StfConnectingBusy => ["Connecting… {} (already trying)", "연결 중… {} (이미 시도 중)", "连接中… {}（已在尝试）", "接続中… {}（すでに試行中）"],
            Msg::StfConnecting => ["Connecting… {}", "연결 중… {}", "连接中… {}", "接続中… {}"],
            Msg::StfNewMsgUnread => ["New message: {} (unread {})", "새 메시지: {} (읽지 않음 {})", "新消息：{}（未读 {}）", "新着メッセージ: {}（未読 {}）"],
            Msg::StfConnectingProfile => ["Connecting to refresh profile… {}", "프로필 갱신을 위해 연결 중… {}", "正在连接以刷新资料… {}", "プロフィール更新のため接続中… {}"],
            Msg::StfSettingsSaveFail => ["Failed to save settings — {} (will retry on next change)", "설정 저장 실패 — {} (다음 변경 때 재시도)", "保存设置失败 — {}（下次更改时重试）", "設定保存失敗 — {}（次の変更時に再試行）"],
            Msg::StfWindowMode => ["Window mode = {} (applies from new conversations)", "창 모드 = {} (새 대화부터 적용)", "窗口模式 = {}（从新对话起生效）", "ウィンドウモード = {}（新しい会話から適用）"],
            Msg::StfColorApplied => ["Color applied — {} = {}", "색 적용 — {} = {}", "已应用颜色 — {} = {}", "色を適用 — {} = {}"],
            Msg::StfListRefresh => ["List refresh interval = {}ms", "목록 갱신 주기 = {}ms", "列表刷新周期 = {}ms", "リスト更新間隔 = {}ms"],
            Msg::StfTooltipDelay => ["Tooltip delay = {}ms", "툴팁 표시 대기 = {}ms", "工具提示延迟 = {}ms", "ツールチップ遅延 = {}ms"],
            Msg::StfDisplayName => ["Display name = {} — broadcast across the LAN", "표시 이름 = {} — LAN 전체에 방송됩니다", "显示名称 = {} — 在局域网内广播", "表示名 = {} — LAN全体に配信されます"],
            Msg::StfPortListening => ["Listen port = {} (already listening on it)", "수신 포트 = {} (이미 이 포트로 듣는 중)", "监听端口 = {}（已在监听）", "受信ポート = {}（すでにこのポートで待受中）"],
            Msg::StfPortApplied => ["Listen port = {} — applied now (re-announce)", "수신 포트 = {} — 즉시 적용(재공지)", "监听端口 = {} — 立即生效（重新通告）", "受信ポート = {} — 即時適用（再通知）"],
            Msg::StfRestartFail => ["Transport restart failed: {} — keeping current port", "전송 재시작 실패: {} — 기존 포트 유지", "传输重启失败：{} — 保留原端口", "転送再起動失敗: {} — 既存ポート維持"],
            Msg::StfAutostartFail => ["⚠ Autostart registration failed: {} (setting kept — will retry on next boot/toggle)", "⚠ 자동 실행 등록 실패: {} (설정은 유지 — 다음 부팅·토글에서 재시도)", "⚠ 自动启动注册失败：{}（设置保留 — 下次启动/切换时重试）", "⚠ 自動起動の登録に失敗: {}（設定は維持 — 次回起動・切替時に再試行）"],
            Msg::StfQueuedSaved => ["Queued for delivery ({} waiting) — sent automatically when the peer appears while this PC is on", "전송 대기 저장({}건) — 내 PC가 켜져 있고 상대가 나타나면 자동 전달됩니다", "已加入待发队列（{}条）— 本机开机且对方出现时自动发送", "送信待ちに保存（{}件）— このPCが起動中で相手が現れたら自動送信"],
            Msg::StClipImageNone => ["No image in clipboard (or conversion failed)", "클립보드에 이미지가 없거나 변환에 실패했습니다", "剪贴板中没有图片（或转换失败）", "クリップボードに画像がない（または変換失敗）"],
            Msg::StfQueuedFlushed => ["Delivered {} queued message(s)", "대기 메시지 {}건 전달됨", "已送达 {} 条待发消息", "待機メッセージ{}件を配信"],
            Msg::StfInvitesSent => ["Sent {} invitations (awaiting acceptance)", "{}명 초대 발송(수락 대기)", "已发送 {} 个邀请（等待接受）", "{}名に招待送信（承認待ち）"],
            Msg::StfGroupSaveFail => ["{} · ⚠ Group save failed (will retry on next change)", "{} · ⚠ 그룹 저장 실패(다음 변경에서 재시도)", "{} · ⚠ 群组保存失败（下次更改时重试）", "{} · ⚠ グループ保存失敗（次の変更で再試行）"],
            Msg::StfGroupPendingSent => ["Group pending messages delivered: {}", "그룹 대기분 전달됨: {}", "群组待发消息已送达：{}", "グループ保留分を配信: {}"],
            Msg::StfJoinedGroup => ["Joined group '{}' — open it from the list", "'{}' 그룹에 참여했습니다 — 목록에서 열기", "已加入群组 '{}' — 从列表打开", "グループ '{}' に参加 — リストから開く"],
            Msg::StfInviteDeclinedBy => ["{} declined the invitation", "{} 님이 초대를 거절했습니다", "{} 拒绝了邀请", "{} さんが招待を拒否しました"],
            Msg::StfRemovedFromGroup => ["Removed from group '{}'", "'{}' 그룹에서 제외되었습니다", "已从群组 '{}' 移除", "グループ '{}' から除外されました"],
            Msg::StfGroupNewMsg => ["[{}] {}: new message", "[{}] {}: 새 메시지", "[{}] {}：新消息", "[{}] {}: 新着メッセージ"],
            Msg::StfSentSeq => ["Sent seq={} (awaiting response)", "전송 seq={} (응답 대기)", "已发送 seq={}（等待响应）", "送信 seq={}（応答待ち）"],
            Msg::StfVerified => ["Fingerprint verified — {} authenticated", "지문 대조 완료 — {} 인증됨", "指纹核对完成 — {} 已验证", "指紋照合完了 — {} を認証"],
            Msg::StfUnverified => ["Verification canceled — {} demoted", "인증 취소 — {} 대조 해제", "已取消验证 — {} 已降级", "認証取り消し — {} を降格"],
            Msg::ListGroupMembers => ["Members {} · Online {}", "구성원 {} · 온라인 {}", "成员 {} · 在线 {}", "メンバー {} · オンライン {}"],
            Msg::FingerprintLabel => ["Key fingerprint", "키 지문", "密钥指纹", "鍵フィンガープリント"],
            Msg::BioLabel => ["Bio", "소개글", "简介", "自己紹介"],
            Msg::BioPlaceholder => ["A short intro (multiple lines allowed)", "간단한 소개 (여러 줄 가능)", "简短介绍（可多行）", "短い自己紹介（複数行可）"],
            Msg::CardLastSeen => ["Last seen", "최근 접속", "最近上线", "最終接続"],
            Msg::CardLastChat => ["Last chat", "최근 대화", "最近对话", "最近の会話"],
            Msg::CardPrivate => ["(private)", "(비공개)", "(未公开)", "(非公開)"],
            Msg::CardNoRecord => ["(no record)", "(기록 없음)", "(无记录)", "(記録なし)"],
            Msg::CardImageCached => ["Image · cached", "이미지 · 캐시됨", "图片 · 已缓存", "画像 · キャッシュ済み"],
            Msg::CardImageCachedAt => ["Image · cached · received {}", "이미지 · 캐시됨 · {} 수신", "图片 · 已缓存 · {} 收到", "画像 · キャッシュ済み · {} 受信"],
            Msg::CardProfileAt => ["Profile · received {}", "프로필 · {} 수신", "个人资料 · {} 收到", "プロフィール · {} 受信"],
            Msg::CardImageNone => ["Image · (none/private)", "이미지 · (없음/비공개)", "图片 · (无/未公开)", "画像 · (なし/非公開)"],
            Msg::CardVerifyPrompt => ["Verify · compare the key fingerprint above over phone/in person", "지문 대조 · 전화·대면으로 위 키 지문을 직접 대조", "指纹核对 · 通过电话/当面直接核对上方密钥指纹", "指紋照合 · 電話・対面で上の鍵指紋を直接照合"],
            Msg::CardVerified => ["✓ Fingerprint verified — a person confirmed this key", "✓ 지문 대조 완료 — 이 키는 사람이 확인했습니다", "✓ 指纹核对完成 — 已由本人确认此密钥", "✓ 指紋照合完了 — この鍵は本人が確認しました"],
            Msg::CardVerifyBtn => ["Match confirmed — mark verified", "일치 확인 — 대조 완료로 표시", "确认一致 — 标记为已核对", "一致確認 — 照合完了に"],
            Msg::CardUnverifyBtn => ["Cancel verification", "인증 취소", "取消验证", "認証取り消し"],
            Msg::CardFooter => ["Esc = close · identity is the key fingerprint, not the name", "Esc = 닫기 · 신원은 이름이 아니라 키 지문입니다", "Esc = 关闭 · 身份是密钥指纹，而非名称", "Esc = 閉じる · 身元は名前ではなく鍵指紋です"],
            Msg::WinAlert => ["Notification", "알림", "通知", "通知"],
            Msg::WinPeerProfile => ["Peer profile", "상대 프로필", "对方资料", "相手プロフィール"],
            Msg::WinMembers => ["Members", "구성원", "成员", "メンバー"],
            Msg::WinGroup => ["Group", "그룹", "群组", "グループ"],
            Msg::WinConnectAddr => ["Connect by address", "주소로 연결", "按地址连接", "アドレスで接続"],
            Msg::WinConfirm => ["Confirm", "확인", "确认", "確認"],
            Msg::WinFileRequest => ["Incoming file", "파일 수신 요청", "文件接收请求", "ファイル受信要求"],
            Msg::WinTransferWait => ["Transfer pending", "전송 대기", "传输等待", "転送待機"],
            Msg::WinNewMessages => ["New messages {}", "새 메시지 {}", "新消息 {}", "新着メッセージ {}"],
            Msg::WinPreview => ["Preview — {}", "미리보기 — {}", "预览 — {}", "プレビュー — {}"],
            Msg::AgoJustNow => ["just now", "방금 전", "刚刚", "たった今"],
            Msg::AgoMinutes => ["{} min ago", "{}분 전", "{} 分钟前", "{} 分前"],
            Msg::AgoHours => ["{} h ago", "{}시간 전", "{} 小时前", "{} 時間前"],
            Msg::AgoDays => ["{} d ago", "{}일 전", "{} 天前", "{} 日前"],
            Msg::AgoMonths => ["{} mo ago", "{}달 전", "{} 个月前", "{} か月前"],
            Msg::AgoYears => ["{} y ago", "{}년 전", "{} 年前", "{} 年前"],
            Msg::HintDemo => ["Demo (echo bot)", "데모(에코 봇)", "演示(回声机器人)", "デモ(エコーボット)"],
            Msg::HintDiscovery => ["Discovered (LAN)", "실물 발견(LAN)", "已发现(LAN)", "実機発見(LAN)"],
            Msg::HintDiscoveryDegraded => ["Discovered · ⚠ can't receive (port 47100 in use) — send-only (pick me from their list)", "실물 발견 · ⚠ 수신 불가(포트 47100 점유) — 발신 전용(상대 목록에서 나를 선택)", "已发现 · ⚠ 无法接收(端口 47100 被占用) — 仅发送(请对方从列表选择我)", "実機発見 · ⚠ 受信不可(ポート 47100 使用中) — 送信のみ(相手のリストから自分を選択)"],
            Msg::HintOpenChat => ["Enter = open chat", "Enter = 대화 열기", "Enter = 打开对话", "Enter = 会話を開く"],
            Msg::HintNewWindow => ["Enter = new window per peer", "Enter = 상대별 새 창(동시 대화)", "Enter = 每个对象新窗口(同时对话)", "Enter = 相手ごとに新規ウィンドウ(同時会話)"],
            Msg::HintAddAddr => ["⌘/Ctrl+K = add address", "⌘/Ctrl+K = 주소 추가", "⌘/Ctrl+K = 添加地址", "⌘/Ctrl+K = アドレス追加"],
            Msg::HintSettings => ["⌘/Ctrl+, = settings", "⌘/Ctrl+, = 설정", "⌘/Ctrl+, = 设置", "⌘/Ctrl+, = 設定"],
            Msg::HintGallery => ["⌘/Ctrl+G = control gallery", "⌘/Ctrl+G = 컨트롤 갤러리", "⌘/Ctrl+G = 控件库", "⌘/Ctrl+G = コントロールギャラリー"],
            Msg::HintTrustLocked => ["⚠ trust list locked (file damaged — all treated unverified)", "⚠ 신뢰 목록 잠김(파일 손상 — 전부 미검증 취급)", "⚠ 信任列表已锁定(文件损坏 — 全部视为未验证)", "⚠ 信頼リストロック(ファイル破損 — 全て未検証扱い)"],
            Msg::HintGroupLocked => ["⚠ group list locked (file damaged — this run is temporary)", "⚠ 그룹 목록 잠김(파일 손상 — 이번 실행은 임시)", "⚠ 群组列表已锁定(文件损坏 — 本次运行为临时)", "⚠ グループリストロック(ファイル破損 — 今回の実行は一時的)"],
            Msg::HintTrustArchived => ["⚠ old trust list (other identity) set aside — starting fresh", "⚠ 옛 신뢰 목록(다른 신원)을 보관하고 새로 시작", "⚠ 旧信任列表(其他身份)已存档 — 重新开始", "⚠ 旧信頼リスト(別の識別)を保管し新規開始"],
            Msg::HintGroupArchived => ["⚠ old group list (other identity) set aside — starting fresh", "⚠ 옛 그룹 목록(다른 신원)을 보관하고 새로 시작", "⚠ 旧群组列表(其他身份)已存档 — 重新开始", "⚠ 旧グループリスト(別の識別)を保管し新規開始"],
            Msg::ProfileReceived => ["Profile received ({}) — {}", "프로필 수신({}) — {}", "已接收资料({}) — {}", "プロフィール受信({}) — {}"],
            Msg::ItemName => ["name", "이름", "姓名", "名前"],
            Msg::ItemEmail => ["email", "이메일", "邮箱", "メール"],
            Msg::ItemPhone => ["phone", "전화", "电话", "電話"],
            Msg::ItemImage => ["image", "이미지", "图片", "画像"],
            Msg::ItemBio => ["bio", "소개글", "简介", "自己紹介"],
            Msg::MemberSelf => ["me", "나", "我", "自分"],
            Msg::MemberOwner => ["owner", "소유자", "所有者", "オーナー"],
            Msg::MemberPending => ["invite pending", "초대 대기", "邀请待处理", "招待待ち"],
            Msg::GroupInvitedTag => [
                "invited · click to accept",
                "초대됨 · 클릭하여 수락",
                "已邀请 · 点击接受",
                "招待済み · クリックで承認",
            ],
            Msg::RateSendFloor => ["Before measurement — starts at floor {}", "실측 전 — 하한 {}에서 시작", "测量前 — 从下限 {} 开始", "実測前 — 下限 {} から開始"],
            Msg::RateSendMeasured => ["Measured peak {} → auto target {}", "실측 최고 {} → 자동 목표 {}", "实测峰值 {} → 自动目标 {}", "実測ピーク {} → 自動目標 {}"],
            Msg::RateRecvUnclaimed => ["Before measurement · no cap claimed — sender yields to half its own measurement", "실측 전 · 상한 무주장 — 발신자가 자기 실측의 절반으로 양보", "测量前 · 不主张上限 — 发送方让步至自身实测的一半", "実測前 · 上限を主張せず — 送信者が自身の実測の半分に譲る"],
            Msg::RateRecvMeasured => ["Measured peak {} · no cap claimed — sender yields to half", "실측 최고 {} · 상한 무주장 — 발신자가 절반으로 양보", "实测峰值 {} · 不主张上限 — 发送方让步至一半", "実測ピーク {} · 上限を主張せず — 送信者が半分に譲る"],
            Msg::AutoAcceptCountdown => ["start {}, elapsed {}, remaining {}, end {}", "시작 {}, 경과 {}, 잔여 {}, 종료 {}", "开始 {}，已过 {}，剩余 {}，结束 {}", "開始 {}、経過 {}、残り {}、終了 {}"],
            Msg::MenuProfile => ["View profile", "프로필 보기", "查看资料", "プロフィールを見る"],
            Msg::MenuPinTop => ["Pin to top", "목록 상단에 고정", "置顶", "リスト上部に固定"],
            Msg::MenuUnpin => ["Unpin", "목록 고정 해제", "取消置顶", "固定を解除"],
            Msg::MenuCreateGroup => ["Create group ({})", "그룹 만들기 ({}명)", "创建群组({})", "グループ作成({})"],
            Msg::MenuForget => ["Remove from list", "목록에서 삭제", "从列表移除", "リストから削除"],
            Msg::MenuForgetAt => ["Remove from list · {}", "목록에서 삭제 · {}", "从列表移除 · {}", "リストから削除 · {}"],
            Msg::MenuGroupRename => ["Rename", "이름 변경", "重命名", "名前変更"],
            Msg::MenuGroupInvite => ["Invite {} selected", "선택한 {}명 초대", "邀请所选 {} 人", "選択した {} 名を招待"],
            Msg::MenuGroupRemoveMembers => ["Remove {} selected", "선택한 {}명 제외", "移除所选 {} 人", "選択した {} 名を除外"],
            Msg::MenuGroupPolicyToOwner => ["Switch to owner-only invites", "소유자만 초대로 전환", "切换为仅所有者可邀请", "オーナーのみ招待に切替"],
            Msg::MenuGroupPolicyToMembers => ["Allow member invites", "구성원 초대 허용으로 전환", "切换为允许成员邀请", "メンバー招待許可に切替"],
            Msg::MenuGroupDisband => ["Disband group", "그룹 해산", "解散群组", "グループ解散"],
            Msg::MenuGroupLeave => ["Leave group", "그룹 나가기", "退出群组", "グループを退出"],
            Msg::AddrTitle => ["Connect directly by address", "주소로 직접 연결", "按地址直接连接", "アドレスで直接接続"],
            Msg::AddrPlaceholder => ["host or host:port · [v6]:port · 64-hex fingerprint", "host 또는 host:port · [v6]:port · 64자리 지문", "host 或 host:port · [v6]:port · 64位指纹", "host または host:port · [v6]:port · 64桁指紋"],
            Msg::AddrConnect => ["Connect", "연결", "连接", "接続"],
            Msg::AddrExample => ["e.g. 10.0.0.5 (port omitted → {}) · 10.0.0.5:{} · [fe80::1]:{} · 64-hex fingerprint = via server", "예: 10.0.0.5 (포트 생략 시 {}) · 10.0.0.5:{} · [fe80::1]:{} · 64자리 지문 = 서버 랑데부", "例: 10.0.0.5 (省略端口 → {}) · 10.0.0.5:{} · [fe80::1]:{} · 64位指纹 = 经服务器", "例: 10.0.0.5 (ポート省略 → {}) · 10.0.0.5:{} · [fe80::1]:{} · 64桁指紋 = サーバー経由"],
            Msg::AddrEnterConnect => ["Enter to connect", "Enter로 연결", "按 Enter 连接", "Enter で接続"],
            Msg::AddrFormatHint => ["format: host:port or [v6]:port (port 1–65535)", "형식: host:port 또는 [v6]:port (포트 1~65535)", "格式: host:port 或 [v6]:port (端口 1~65535)", "形式: host:port または [v6]:port (ポート 1~65535)"],
            Msg::AboutTagline => ["Zero-config LAN messenger", "제로 컨피그 로컬 네트워크 메신저 · Zero-config LAN messenger", "零配置局域网通讯 · Zero-config LAN messenger", "ゼロ設定ローカルメッセンジャー · Zero-config LAN messenger"],
            Msg::AboutHomepage => ["Nexa Beep homepage", "Nexa Beep 홈페이지", "Nexa Beep 主页", "Nexa Beep ホームページ"],
            Msg::ChatOpenedProfile => ["Chat opened — profile: {}", "대화 열림 — 프로필: {}", "已打开对话 — 资料: {}", "会話を開いた — プロフィール: {}"],
            Msg::ChatOpenedSession => ["Chat opened — session active", "대화 열림 — 세션 유지 중", "已打开对话 — 会话保持中", "会話を開いた — セッション維持中"],
            Msg::StItemImagePresent => ["has image", "이미지 있음", "有图片", "画像あり"],
            Msg::CmdHelpHeader => ["Available commands", "사용 가능한 명령", "可用命令", "使用可能なコマンド"],
            Msg::CmdHelpHelp => ["  /help       this help", "  /help            이 안내", "  /help       此帮助", "  /help       このヘルプ"],
            Msg::CmdHelpVerify => ["  /verify     mark this peer verified (compare fingerprint first)", "  /verify          이 상대를 대조 완료로 표시(먼저 지문 대조)", "  /verify     标记对方为已核对(请先核对指纹)", "  /verify     この相手を照合完了に(先に指紋照合)"],
            Msg::CmdHelpFingerprint => ["  /fingerprint show this peer's key fingerprint", "  /fingerprint     이 상대의 키 지문 출력", "  /fingerprint 显示对方的密钥指纹", "  /fingerprint 相手の鍵指紋を表示"],
            Msg::CmdFingerprint => ["Key fingerprint — you: {} · {}: {} — compare over another channel (phone/in person), then /verify", "키 지문 — 나: {} · {}: {} — 전화·대면 등 다른 채널로 맞춰 본 뒤 /verify", "密钥指纹 — 我: {} · {}: {} — 通过电话/当面等其他渠道核对后 /verify", "鍵指紋 — 自分: {} · {}: {} — 電話・対面など別の経路で照合後 /verify"],
            Msg::CmdVerifiedNow => ["Marked verified — this key is now trusted (blue seal badge).", "대조 완료로 표시했습니다 — 이 키를 신뢰합니다(파란 실 배지).", "已标记为已核对 — 现已信任此密钥(蓝色印章徽章)。", "照合完了に設定 — この鍵を信頼します(青いシール)。"],
            Msg::CmdHelpUnverify => ["  /unverify   cancel verification", "  /unverify        인증 취소", "  /unverify   取消验证", "  /unverify   認証取り消し"],
            Msg::CmdHelpTrust => ["  /trust      show this peer's trust status", "  /trust           이 상대의 신뢰 상태 보기", "  /trust      查看对方的信任状态", "  /trust      相手の信頼状態を見る"],
            Msg::CmdHelpClose => ["  /close      close the chat window", "  /close           대화창 닫기", "  /close      关闭对话窗口", "  /close      会話ウィンドウを閉じる"],
            Msg::CmdHelpNotice => ["  /notice <text>  send as Notice grade", "  /notice <내용>   알림 등급으로 전송(강도는 수신자 정책)", "  /notice <内容>  以提醒等级发送", "  /notice <内容>  通知グレードで送信"],
            Msg::CmdHelpUrgent => ["  /urgent <text>  send as Urgent grade (receiver may demote)", "  /urgent <내용>   긴급 등급으로 전송(수신측 정책·신뢰에 따라 강등)", "  /urgent <内容>  以紧急等级发送（接收方可降级）", "  /urgent <内容>  緊急グレードで送信（受信側で降格あり）"],
            Msg::StfGradeUsage => ["Usage: {} <text> (one line)", "사용법: {} <내용> (한 줄)", "用法：{} <内容>（单行）", "使い方: {} <内容>（1行）"],
            Msg::StUrgentArmed => ["Urgent armed for the next send — requests a strong alert on the peer (their policy decides)", "긴급 선택됨 — 다음 전송 1회에 적용 · 상대에게 강한 알림을 요청합니다(수신자 정책이 최종 결정)", "已选紧急 — 仅应用于下一次发送 · 请求对方强提醒（由接收方策略决定）", "緊急を選択 — 次の送信1回に適用 · 相手に強い通知を要求（受信側ポリシーが決定）"],
            Msg::StGradeGroupUnsupported => ["Grades are 1:1 only for now — use Menu ▸ Send notice for broadcast", "등급은 아직 1:1 전용 — 전체 공지는 메뉴 ▸ 공지 보내기", "等级暂仅支持1:1 — 群发请用菜单▸发送公告", "グレードは1:1のみ — 一斉通知はメニュー▸お知らせ送信"],
            Msg::StfBroadcastSent => ["Notice sent — {} now · {} queued (delivered when they appear)", "공지 발송 — 즉시 {} · 대기 {} (상대가 나타나면 자동 전달)", "公告已发 — 立即 {} · 排队 {}（对方出现时送达）", "お知らせ送信 — 即時 {} · 待機 {}（相手が現れたら配信）"],
            Msg::MenuBroadcast => ["Send notice…", "공지 보내기…", "发送公告…", "お知らせ送信…"],
            Msg::BroadcastTitle => ["Send notice to everyone discovered (Notice grade · once per 3s)", "공지 보내기 — 발견된 전체에게(알림 등급 · 3초에 1번)", "向所有已发现的对象发送公告（提醒等级 · 每3秒1次）", "発見中の全員へお知らせ（通知グレード · 3秒に1回）"],
            Msg::CmdHelpNote => ["  ※ Input starting with / is never sent", "  ※ /로 시작하는 입력은 어떤 경우에도 전송되지 않습니다", "  ※ 以 / 开头的输入绝不会被发送", "  ※ / で始まる入力は決して送信されません"],
            Msg::CmdUnknown => ["Unknown command /{} — see /help (not sent)", "모르는 명령 /{} — /help 로 목록을 봅니다(보내지 않았습니다)", "未知命令 /{} — 请见 /help(未发送)", "不明なコマンド /{} — /help を参照(送信していません)"],
            Msg::CmdTrustGroup => ["No single peer in a group room — check each in the members modal", "그룹 방에는 단일 상대가 없습니다 — 구성원 모달에서 각자 확인합니다", "群组房间没有单一对象 — 请在成员窗口逐一确认", "グループ部屋には単一の相手がいません — メンバー画面で各自確認"],
            Msg::CmdTrustStatus => ["{} — trust status: {}", "{} — 신뢰 상태: {}", "{} — 信任状态: {}", "{} — 信頼状態: {}"],
            Msg::CmdVerifyAlready => ["Fingerprint already verified (blue seal badge).", "이미 지문 대조가 끝난 상대입니다(파란 실 배지).", "已完成指纹核对(蓝色印章徽章)。", "既に指紋照合済みです(青いシール)。"],
            Msg::CmdVerifyOpened => ["Safety number card opened. Compare the number with the peer over another channel (phone/in person), then press 'Verified'.", "안전 번호 카드를 열었습니다. 전화·대면 등 다른 채널로 상대와 같은 번호인지 맞춰 본 뒤 '대조 완료'를 누르세요.", "已打开安全码卡片。请通过电话/当面等其他渠道与对方核对号码，然后按'核对完成'。", "安全番号カードを開きました。電話・対面など別の経路で相手と番号を照合し、'照合完了'を押してください。"],
            Msg::CmdVerify1to1 => ["Fingerprint verification is only for 1:1 chats (a single peer required).", "지문 대조는 1:1 대화에서만 가능합니다(상대가 하나로 정해져야 합니다).", "指纹核对仅在 1:1 对话中可用(需确定单一对象)。", "指紋照合は 1:1 の会話でのみ可能です(相手が一人に定まる必要があります)。"],
            Msg::CmdUnverifyDone => ["Verification canceled — fingerprint match cleared (use /verify to compare again).", "인증을 취소했습니다 — 지문 대조 상태가 해제됐습니다(다시 대조하려면 /verify).", "已取消验证 — 指纹核对状态已解除(重新核对请用 /verify)。", "認証を取り消しました — 指紋照合状態が解除されました(再照合は /verify)。"],
            Msg::CmdUnverifyNone => ["This peer is not in a verified state (nothing to cancel).", "이 상대는 지문 대조 상태가 아닙니다(취소할 인증이 없습니다).", "对方并非已验证状态(无可取消的验证)。", "この相手は照合済み状態ではありません(取り消す認証がありません)。"],
            Msg::CmdUnverify1to1 => ["Canceling verification is only for 1:1 chats (a single peer required).", "인증 취소는 1:1 대화에서만 가능합니다(상대가 하나로 정해져야 합니다).", "取消验证仅在 1:1 对话中可用(需确定单一对象)。", "認証取り消しは 1:1 の会話でのみ可能です(相手が一人に定まる必要があります)。"],
            Msg::ListNavHint => ["↑↓ move · type = jump to name(한글 가능) · Enter = open chat", "↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기", "↑↓ 移动 · 输入 = 跳转到名称(한글 가능) · Enter = 打开对话", "↑↓ 移動 · 入力 = 名前ジャンプ(한글 가능) · Enter = 会話を開く"],
            Msg::SuggestVerify => ["This peer isn't fingerprint-verified yet. Type /fingerprint to see the key fingerprint, compare it over another channel (phone/in person), then /verify.", "이 상대는 아직 지문 대조 전입니다. /fingerprint 로 키 지문을 확인하고, 전화·대면 등 다른 채널로 맞춰 본 뒤 /verify 를 입력하세요.", "对方尚未完成指纹核对。用 /fingerprint 查看密钥指纹，通过电话/当面等其他渠道核对后输入 /verify。", "この相手はまだ指紋照合前です。/fingerprint で鍵指紋を確認し、電話・対面など別の経路で照合してから /verify を入力してください。"],
            Msg::ActOk => ["OK", "확인", "确定", "OK"],
            Msg::XferCancelBtn => ["Cancel transfer — {}", "전송 취소 — {}", "取消传输 — {}", "転送キャンセル — {}"],
            Msg::XferWaitApproval => ["Waiting for the other side to accept…", "상대의 승인을 기다리는 중…", "正在等待对方接受…", "相手の承認を待っています…"],
            Msg::XferBatchSummary => ["{} files · {} total", "{}개 파일 · 총 {}", "{} 个文件 · 共 {}", "{} ファイル · 合計 {}"],
            Msg::XferStWaiting => ["Waiting", "대기", "等待", "待機"],
            Msg::XferStOffered => ["Awaiting approval", "승인 대기", "等待批准", "承認待ち"],
            Msg::XferStActive => ["Sending", "전송 중", "传输中", "転送中"],
            Msg::XferStPaused => ["Paused", "일시정지", "已暂停", "一時停止"],
            Msg::XferExcluded => ["Excluded — over size limit", "전송 제외 — 용량 초과", "已排除 — 超出大小限制", "除外 — サイズ上限超過"],
            Msg::XferStDone => ["Done", "완료", "完成", "完了"],
            Msg::XferStFailed => ["Failed", "실패", "失败", "失敗"],
            Msg::StfAutoAcceptRecv => ["Auto-accepted: {} ({}) receiving", "자동 수락: {} ({}) 수신 시작", "自动接受：{}（{}）开始接收", "自動受信: {} ({}) 受信開始"],
            Msg::StfFileRejected => ["File rejected ({}): {}", "파일 거부({}): {}", "文件已拒绝（{}）：{}", "ファイル拒否({}): {}"],
            Msg::StfDeliveredWait => ["Delivered {}/{} — awaiting peer confirmation", "전달됨 {}/{} — 상대 확인 대기", "已送达 {}/{} — 等待对方确认", "配信済み {}/{} — 相手の確認待ち"],
            Msg::StfFileWhy => ["File: {}", "파일: {}", "文件：{}", "ファイル: {}"],
            Msg::StfTrustReject => ["Trust decision rejected: {}", "신뢰 판정 거부: {}", "信任判定被拒：{}", "信頼判定拒否: {}"],
            Msg::StfConnectedOpen => ["Connected: {} — open from the list", "연결됨: {} — 목록에서 열기", "已连接：{} — 从列表打开", "接続済み: {} — リストから開く"],
            Msg::StfManualConnFail => ["Manual connection failed ({}): {}", "수동 연결 실패({}): {}", "手动连接失败（{}）：{}", "手動接続失敗({}): {}"],
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
            Msg::XferAutoCancel => [
                "Paused auto-cancel",
                "일시중지 자동 취소 시간",
                "暂停自动取消时间",
                "一時停止の自動取消時間",
            ],
            Msg::XferAutoCancelDesc => [
                "Cancel the whole batch on both sides if paused transfers sit untouched this long",
                "일시중지 전송이 이 시간 동안 방치되면 양쪽 모두 전체 취소됩니다",
                "暂停的传输闲置超过此时间后双方全部取消",
                "一時停止の転送がこの時間放置されると両側で全て取消",
            ],
            Msg::Min1 => ["1m", "1분", "1分钟", "1分"],
            Msg::Min2 => ["2m", "2분", "2分钟", "2分"],
            Msg::Min5 => ["5m", "5분", "5分钟", "5分"],
            Msg::Min10 => ["10m", "10분", "10分钟", "10分"],
            Msg::StXferCanceled => [
                "Transfer canceled",
                "전송을 취소했습니다",
                "已取消传输",
                "転送をキャンセルしました",
            ],
            Msg::StfXferTimeoutCanceled => [
                "No response for {}s — transfer canceled",
                "{}초 동안 응답이 없어 전송을 취소했습니다",
                "{}秒无响应 — 已取消传输",
                "{}秒応答なし — 転送をキャンセル",
            ],
            Msg::ValOutOfRangeTitle => ["Invalid value", "잘못된 값", "无效值", "無効な値"],
            Msg::ValMinutesRange => [
                "Enter 1-10 minutes. Reverted to the previous value.",
                "1~10분 사이로 입력하세요. 직전 값으로 되돌렸습니다.",
                "请输入1~10分钟。已恢复为上一个值。",
                "1〜10分で入力してください。直前の値に戻しました。",
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
            Msg::QVerifying => ["Verifying… (approval available after verification)", "검증 중… (승인은 검증 후)", "校验中…（校验后可批准）", "検証中…（承認は検証後）"],
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
            Msg::OfferCount => ["Files", "파일 수", "文件数", "ファイル数"],
            Msg::OfferExcluded => ["Excluded", "제외됨", "已排除", "除外"],
            Msg::StfFolderExcluded => [
                "Folders cannot be sent — {} excluded",
                "폴더는 전송할 수 없습니다 — {} 제외됨",
                "无法发送文件夹 — {} 已排除",
                "フォルダは送信できません — {} を除外",
            ],
            Msg::XferBatchMax => [
                "Max files per request",
                "요청당 최대 파일 수",
                "每次请求最大文件数",
                "1リクエスト最大ファイル数",
            ],
            Msg::XferBatchMaxDesc => [
                "Counts everything attempted, including excluded items. Files over the size cap and folders are excluded automatically",
                "제외 대상을 포함해 시도한 전체를 셉니다. 용량 제한을 넘는 파일과 폴더는 자동으로 제외됩니다",
                "计入所有尝试项（含被排除项）。超过容量上限的文件和文件夹会被自动排除",
                "除外分を含む試行全体を数えます。容量上限超過のファイルとフォルダは自動的に除外されます",
            ],
            Msg::Cnt1 => ["1", "1개", "1个", "1件"],
            Msg::Cnt2 => ["2", "2개", "2个", "2件"],
            Msg::Cnt3 => ["3", "3개", "3个", "3件"],
            Msg::Cnt4 => ["4", "4개", "4个", "4件"],
            Msg::Cnt5 => ["5 (default)", "5개(기본)", "5个(默认)", "5件(既定)"],
            Msg::XfRecvRefused => [
                "Receive refused: {}",
                "수신 거부: {}",
                "接收被拒: {}",
                "受信拒否: {}",
            ],
            Msg::XfOpenFailed => [
                "Cannot open file: {}",
                "파일 열기 실패: {}",
                "无法打开文件: {}",
                "ファイルを開けません: {}",
            ],
            Msg::XfRecvError => [
                "Receive error: {} — discarded",
                "수신 오류: {} — 폐기",
                "接收错误: {} — 已丢弃",
                "受信エラー: {} — 破棄",
            ],
            Msg::XfDoneFailed => [
                "Finalize failed: {} — discarded",
                "완료 실패: {} — 폐기",
                "完成失败: {} — 已丢弃",
                "完了失敗: {} — 破棄",
            ],
            Msg::XfWireError => [
                "Wire error: {}",
                "와이어 오류: {}",
                "线路错误: {}",
                "ワイヤエラー: {}",
            ],
            Msg::StfResumeFrom => [
                "Resume — {} from {}%",
                "이어받기 — {} {}%부터",
                "续传 — {} 从 {}%",
                "再開 — {} {}%から",
            ],
            Msg::StfAcceptStart => [
                "Accepted — receiving {}",
                "수락 — {} 수신 시작",
                "已接受 — 开始接收 {}",
                "承認 — {} 受信開始",
            ],
            Msg::StfDeclineName => ["Declined — {}", "거절 — {}", "已拒绝 — {}", "拒否 — {}"],
            Msg::StfMoreOffers => [
                "{} · {} more offers pending",
                "{} · 대기 중인 제안 {}건 더 있음",
                "{} · 还有 {} 个待处理请求",
                "{} · 保留中の提案があと{}件",
            ],
            Msg::StfTimeoutDeclined => [
                "No response for {}s — declined {}",
                "{}초 동안 응답이 없어 거절했습니다 — {}",
                "{} 秒无响应 — 已拒绝 {}",
                "{}秒応答がないため拒否しました — {}",
            ],
            Msg::StfGroupFileOffer => [
                "Group file transfer — offered {} · waiting {}",
                "그룹 파일 전송 — 오퍼 {} · 연결 대기 {}",
                "群文件传输 — 已发起 {} · 等待连接 {}",
                "グループ送信 — オファー {} · 接続待ち {}",
            ],
            Msg::StfPeerRemoved => [
                "{} — removed from list (pin/cache cleared; reappears when met again)",
                "{} — 목록에서 삭제(핀·캐시 정리 · 다시 만나면 새로 뜹니다)",
                "{} — 已从列表移除（清除固定/缓存 · 再次相遇会重新出现）",
                "{} — リストから削除（ピン·キャッシュ整理 · 再発見で再表示）",
            ],
            Msg::StNoLogs => [
                "No logs — enable 'Status logging' to collect",
                "로그 없음 — '상태 로그 기록'을 켜면 쌓입니다",
                "无日志 — 开启「状态日志」后开始记录",
                "ログなし — 「ステータスログ」を有効にすると記録されます",
            ],
            Msg::TitleFilePick => [
                "Choose file — {}",
                "파일 선택 — {}",
                "选择文件 — {}",
                "ファイル選択 — {}",
            ],
            Msg::TitlePickBackupDir => [
                "Choose backup folder — {}",
                "백업 폴더 선택 — {}",
                "选择备份文件夹 — {}",
                "バックアップフォルダ選択 — {}",
            ],
            Msg::TitlePickBackup => [
                "Choose backup file — {}",
                "백업 파일 선택 — {}",
                "选择备份文件 — {}",
                "バックアップファイル選択 — {}",
            ],
            Msg::TitlePickProfileImage => [
                "Choose profile image — {}",
                "프로필 이미지 선택 — {}",
                "选择头像图片 — {}",
                "プロフィール画像選択 — {}",
            ],
            Msg::TitlePickSettingsBackupDir => [
                "Settings backup folder — {}",
                "설정 백업 폴더 — {}",
                "设置备份文件夹 — {}",
                "設定バックアップフォルダ — {}",
            ],
            Msg::TitlePickSettingsBackup => [
                "Choose settings backup file — {}",
                "설정 백업 파일 선택 — {}",
                "选择设置备份文件 — {}",
                "設定バックアップファイル選択 — {}",
            ],
            Msg::PickDirPrefix => ["[Folder] {}", "[폴더] {}", "[文件夹] {}", "[フォルダ] {}"],
            Msg::ConvboxTitle => ["Conversations", "대화함", "会话箱", "会話ボックス"],
            Msg::CvEmpty => [
                "No saved conversations",
                "저장된 대화 기록이 없습니다",
                "没有保存的会话记录",
                "保存された会話履歴はありません",
            ],
            Msg::CvFilterPh => [
                "Filter by name",
                "이름으로 필터",
                "按名称筛选",
                "名前でフィルタ",
            ],
            Msg::CvBackup => ["Back up all", "전체 백업", "全部备份", "全体バックアップ"],
            Msg::CvRestore => ["Restore", "복원", "恢复", "復元"],
            Msg::CvClear => ["Delete all", "전체 삭제", "全部删除", "全体削除"],
            Msg::CvClearConfirm => [
                "Delete ALL conversation history? Click again to confirm",
                "대화 기록을 전부 삭제할까요? 한 번 더 누르면 삭제됩니다",
                "删除全部会话记录？再点一次确认",
                "会話履歴を全削除しますか？もう一度押すと確定",
            ],
            Msg::CvDelConfirm => [
                "Click the trash again to delete this history",
                "휴지통을 한 번 더 누르면 이 기록이 삭제됩니다",
                "再点一次垃圾桶即删除该记录",
                "もう一度ゴミ箱を押すとこの履歴を削除",
            ],
            Msg::CvGroupTag => ["group", "그룹", "群", "グループ"],
            Msg::CvCount => [
                "{} conversation(s)",
                "대화 기록 {}건",
                "{} 条会话记录",
                "会話履歴 {}件",
            ],
            Msg::StfCvBackupDone => [
                "Backed up {} history file(s) — {}",
                "대화 기록 {}개 백업 완료 — {}",
                "已备份 {} 个会话记录 — {}",
                "会話履歴 {}件をバックアップ — {}",
            ],
            Msg::StfCvRestoreDone => [
                "Restored {} file(s) — duplicates overwritten, others kept",
                "{}개 복원 — 중복은 덮어쓰고 나머지는 유지했습니다",
                "已恢复 {} 个 — 重复覆盖，其余保留",
                "{}件を復元 — 重複は上書き・他は維持",
            ],
            Msg::StfCvDeleted => [
                "Conversation history deleted — {}",
                "대화 기록 삭제 — {}",
                "会话记录已删除 — {}",
                "会話履歴を削除 — {}",
            ],
            Msg::StCvCleared => [
                "All conversation history deleted",
                "대화 기록을 전부 삭제했습니다",
                "已删除全部会话记录",
                "会話履歴を全削除しました",
            ],
            Msg::StCvNone => [
                "No history files",
                "대화 기록 파일이 없습니다",
                "没有会话记录文件",
                "会話履歴ファイルがありません",
            ],
            Msg::TitlePickCvBackupDir => [
                "History backup folder — {}",
                "대화 기록 백업 폴더 — {}",
                "会话记录备份文件夹 — {}",
                "会話履歴バックアップフォルダ — {}",
            ],
            Msg::TitlePickCvRestoreDir => [
                "Restore history from — {}",
                "대화 기록 복원 위치 — {}",
                "从此处恢复会话记录 — {}",
                "会話履歴の復元元 — {}",
            ],
            Msg::WordFile => ["File", "파일", "文件", "ファイル"],
            Msg::PickRestoreHere => [
                "[Restore from this folder]",
                "[이 폴더에서 복원]",
                "[从此文件夹恢复]",
                "[このフォルダから復元]",
            ],
            Msg::WarnBatchLimitTitle => [
                "Too many files",
                "파일 개수 초과",
                "文件数量超限",
                "ファイル数の上限超過",
            ],
            Msg::WarnBatchLimitBody => [
                "Too many files — nothing was sent.\n\nCurrent limit: {} per request. You can set up to 5 in File settings (\"Max files per request\").",
                "개수 제한을 넘어 전송이 시작되지 않았습니다.\n\n현재 설정값: 요청당 {}개. 파일 설정의 '요청당 최대 파일 수'에서 최대 5개까지 지정할 수 있습니다.",
                "超出数量限制，未开始发送。\n\n当前设置：每次请求 {} 个。可在文件设置的\"每次请求最大文件数\"中最多指定 5 个。",
                "上限を超えたため送信を開始しませんでした。\n\n現在の設定: 1リクエスト {}件。ファイル設定の「1リクエスト最大ファイル数」で最大5件まで指定できます。",
            ],
            Msg::OfferResumeBtn => ["Resume {}%", "이어받기 {}%", "续传 {}%", "再開 {}%"],
            Msg::SubLog => ["Log", "로그", "日志", "ログ"],
            Msg::LogEnabled => ["Status logging", "상태 로그 기록", "状态日志", "ステータスログ"],
            Msg::LogEnabledDesc => [
                "Write status bar messages to data/logs (diagnostics). Off = no disk writes.",
                "상태바 메시지를 data/logs 파일로 남깁니다(진단용). 끄면 디스크 쓰기가 없습니다.",
                "将状态栏消息写入 data/logs（诊断用）。关闭 = 不写磁盘。",
                "ステータスバーの内容を data/logs に記録します（診断用）。オフ = 書き込みなし。",
            ],
            Msg::LogRetain => ["Log retention (days)", "로그 보존 일수", "日志保留天数", "ログ保持日数"],
            Msg::LogRetainDesc => [
                "Older daily files are deleted.",
                "지난 날짜 파일은 삭제됩니다.",
                "超过天数的文件将被删除。",
                "期限を過ぎたファイルは削除されます。",
            ],
            Msg::LogMaxTotal => ["Log size cap (MB)", "로그 총량 상한 (MB)", "日志总量上限 (MB)", "ログ合計上限 (MB)"],
            Msg::LogMaxTotalDesc => [
                "Oldest files are removed first when over the cap.",
                "상한을 넘으면 오래된 파일부터 지웁니다.",
                "超出上限时先删除最旧的文件。",
                "上限を超えると古いファイルから削除します。",
            ],
            Msg::LogView => ["View log", "로그 보기", "查看日志", "ログを見る"],
            Msg::LogViewDesc => [
                "Open today's log with the default .log app (folder if absent).",
                "오늘 로그를 .log 기본 프로그램으로 엽니다(없으면 폴더).",
                "用默认程序打开今天的日志（无则打开文件夹）。",
                "今日のログを既定のアプリで開きます（無ければフォルダ）。",
            ],
            Msg::ActOpen => ["Open", "열기", "打开", "開く"],
            Msg::SizeXLarge => ["Extra Large", "아주 크게", "特大", "特大"],
            Msg::LogRetainDefault => ["Default (7)", "기본(7일)", "默认(7天)", "既定(7日)"],
            Msg::LogCapDefault => ["Default (20)", "기본(20MB)", "默认(20MB)", "既定(20MB)"],
            Msg::SubNetmon => ["Network check", "네트워크 점검", "网络检查", "ネットワーク点検"],
            Msg::NetmonEnabled => [
                "Traffic monitoring log",
                "트래픽 계측 기록",
                "流量监测日志",
                "トラフィック計測ログ",
            ],
            Msg::NetmonEnabledDesc => [
                "Record packet/byte counts per interval to data/logs/netmon-*.log to spot \
                 excessive traffic (counts only — no addresses or content). Off = no writes.",
                "주기마다 패킷·바이트 수를 data/logs/netmon-*.log에 남겨 과도한 송수신을 \
                 찾습니다(횟수만 — 주소·내용 없음). 끄면 기록하지 않습니다.",
                "按周期将数据包/字节计数写入 data/logs/netmon-*.log 以发现异常流量（仅计数——\
                 无地址与内容）。关闭 = 不记录。",
                "周期ごとにパケット/バイト数を data/logs/netmon-*.log に記録し過剰な送受信を\
                 見つけます（回数のみ — アドレス・内容なし）。オフ = 記録なし。",
            ],
            Msg::NetmonInterval => [
                "Check interval (sec)",
                "점검 주기 (초)",
                "检查周期（秒）",
                "点検間隔（秒）",
            ],
            Msg::NetmonIntervalDesc => [
                "One summary line is written per interval.",
                "주기마다 요약 한 줄이 기록됩니다.",
                "每个周期写入一行摘要。",
                "間隔ごとに要約1行を記録します。",
            ],
            Msg::NetmonIntervalDefault => ["Default (10)", "기본(10초)", "默认(10秒)", "既定(10秒)"],
            Msg::StBroadcastRateLimit => [
                "Notices are limited to once per 3 seconds — try again shortly (not sent)",
                "공지는 3초에 1번만 보낼 수 있습니다 — 잠시 후 다시 시도하세요(보내지 않았습니다)",
                "公告限每3秒发送1次 — 请稍后再试（未发送）",
                "お知らせは3秒に1回までです — 少し待って再試行してください（送信していません）",
            ],
            Msg::ScanNotDone => ["Not scanned", "검사 안 됨", "未检查", "未検査"],
            Msg::ScanClean => [
                "Scanned (no detection)",
                "검사됨(탐지 없음)",
                "已检查（未检出）",
                "検査済み（検出なし）",
            ],
            Msg::ScanDetected => [
                "Scanned (DETECTED)",
                "검사됨(탐지)",
                "已检查（检出威胁）",
                "検査済み（検出あり）",
            ],
            Msg::StfScanDetected => [
                "⚠ Threat detected in received file: {} — kept quarantined (do not approve)",
                "⚠ 수신 파일에서 위협 탐지: {} — 격리 유지(승인하지 마세요)",
                "⚠ 接收文件检出威胁：{} — 保持隔离（请勿批准）",
                "⚠ 受信ファイルで脅威を検出: {} — 隔離を維持（承認しないでください）",
            ],
            Msg::StfSyncFolderWarn => [
                "⚠ Data folder is inside a {} sync folder — sync conflicts may corrupt or resurrect history",
                "⚠ 데이터 폴더가 {} 동기화 폴더 안에 있습니다 — 동기화 충돌로 기록이 깨지거나 지운 대화가 되살아날 수 있습니다",
                "⚠ 数据文件夹位于 {} 同步文件夹内 — 同步冲突可能损坏或复活记录",
                "⚠ データフォルダが {} 同期フォルダ内にあります — 同期競合で記録が壊れたり復活したりする恐れ",
            ],
            Msg::ArchiveViol => [
                "Archive policy violation",
                "아카이브 위반",
                "压缩包策略违规",
                "アーカイブ規則違反",
            ],
            Msg::GradeNormal => ["Normal", "일반", "普通", "通常"],
            Msg::GradeNotice => ["Notice", "알림", "提醒", "通知"],
            Msg::GradeUrgent => ["Urgent", "긴급", "紧急", "緊急"],
            Msg::WinBroadcast => ["Notice", "공지", "公告", "お知らせ"],
            Msg::PhBroadcastBody => ["Notice body", "공지 내용", "公告内容", "お知らせ内容"],
            Msg::PhGroupName => ["Group name", "그룹 이름", "群组名称", "グループ名"],
            Msg::NotifyBroadcastMute => [
                "Ignore broadcasts",
                "공지(브로드캐스트) 받지 않기",
                "忽略广播公告",
                "お知らせ（ブロードキャスト）を受け取らない",
            ],
            Msg::NotifyBroadcastMuteDesc => [
                "Silently discard notices sent to everyone discovered — no display, \
                 notification, or history. The sender is not told.",
                "발견 전체에게 뿌려진 공지를 조용히 버립니다 — 표시·알림·기록 없음. \
                 발신자에게는 알리지 않습니다.",
                "静默丢弃发给所有人的公告 — 不显示、不通知、不记录。发送方不会得知。",
                "全員宛のお知らせを静かに破棄します — 表示・通知・記録なし。送信者には知らせません。",
            ],
            Msg::StfNetmonWarn => [
                "⚠ Network check: excessive traffic ({}) — see netmon log",
                "⚠ 네트워크 점검: 과다 트래픽({}) — netmon 로그 확인",
                "⚠ 网络检查：流量异常（{}）— 请查看 netmon 日志",
                "⚠ ネットワーク点検: 過剰トラフィック（{}）— netmon ログ参照",
            ],
            Msg::OfferFreshBtn => ["From start", "처음부터", "重新开始", "最初から"],
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
            Msg::AutoStart => [
                "Start at login",
                "시스템 시작 시 자동 실행",
                "登录时自动启动",
                "ログイン時に自動起動",
            ],
            Msg::AutoStartDesc => [
                "Launch Nexa Beep automatically when you sign in to the OS (Windows Run registry · macOS LaunchAgent · Linux autostart — no admin rights needed)",
                "OS 로그인 시 Nexa Beep을 자동으로 실행합니다 (Windows 레지스트리 Run · macOS LaunchAgent · Linux autostart — 관리자 권한 불요)",
                "登录系统时自动启动 Nexa Beep（Windows 注册表 Run · macOS LaunchAgent · Linux autostart — 无需管理员权限）",
                "OSログイン時に Nexa Beep を自動起動します（Windows レジストリ Run · macOS LaunchAgent · Linux autostart — 管理者権限不要）",
            ],
            Msg::CloseToTray => [
                "Close button minimizes to tray",
                "닫기 버튼 = 트레이로(종료하지 않음)",
                "关闭按钮最小化到托盘",
                "閉じるボタンでトレイに常駐",
            ],
            Msg::CloseToTrayDesc => [
                "Keep running in the system tray when the main window is closed. Right-click the tray icon to quit. (Windows)",
                "메인 창을 닫아도 시스템 트레이에 남아 계속 실행됩니다. 종료는 트레이 아이콘 우클릭 메뉴에서. (Windows)",
                "关闭主窗口后仍驻留在系统托盘。右键托盘图标可退出。(Windows)",
                "メインウィンドウを閉じてもトレイに常駐します。終了はトレイの右クリックから。(Windows)",
            ],
            Msg::TrayOpen => ["Open", "열기", "打开", "開く"],
            Msg::TrayQuit => ["Quit", "종료", "退出", "終了"],
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
                "Font for buttons, headers, settings and other base UI. Sizes: Small 14px · Normal 16px · Large 18px · Extra Large 22px",
                "버튼·헤더·설정 등 기본 UI 영역의 글꼴. 크기: 작게 14px · 보통 16px · 크게 18px · 아주 크게 22px",
                "按钮、标题、设置等基本界面的字体。大小：小 14px · 标准 16px · 大 18px · 特大 22px",
                "ボタン・見出し・設定など基本UIのフォント。サイズ：小 14px · 標準 16px · 大 18px · 特大 22px",
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
                "Type a message… (Enter to send · Shift+Enter for newline · Esc for list)",
                "메시지 입력… (Enter 전송 · Shift+Enter 줄바꿈 · Esc 목록)",
                "输入消息…（Enter 发送 · Shift+Enter 换行 · Esc 返回列表）",
                "メッセージ入力…（Enter 送信 · Shift+Enter 改行 · Esc 一覧）",
            ],
            Msg::TrustUnverified => ["Unverified", "미검증", "未验证", "未検証"],
            Msg::TrustPinned => ["Pinned", "핀 고정", "已固定", "ピン留め"],
            Msg::TrustVerified => ["Verified", "대조 완료", "已核对", "照合済み"],
            Msg::TrustBlocked => ["Blocked", "차단됨", "已屏蔽", "ブロック済み"],
            Msg::TrustConflict => ["Name conflict", "이름 충돌", "名称冲突", "名前の衝突"],
            Msg::TrustUnverifiedTip => [
                "Key not yet confirmed by a session",
                "세션으로 아직 확인되지 않은 키",
                "密钥尚未经会话确认",
                "セッションで未確認の鍵",
            ],
            Msg::TrustPinnedTip => [
                "Key pinned on first contact (TOFU)",
                "첫 접촉에서 고정된 키(TOFU)",
                "首次接触时固定的密钥(TOFU)",
                "初回接触で固定した鍵(TOFU)",
            ],
            Msg::TrustVerifiedTip => [
                "Fingerprint compared and confirmed",
                "지문 대조까지 완료된 키",
                "指纹已核对确认",
                "指紋照合まで完了した鍵",
            ],
            Msg::TrustBlockedTip => [
                "Sessions from this key are refused",
                "이 키의 세션 수립을 거부한다",
                "拒绝此密钥的会话",
                "この鍵のセッションを拒否する",
            ],
            Msg::TrustConflictTip => [
                "Same name on a different key — possible impersonation",
                "같은 이름을 다른 키가 쓴다 — 사칭 의심",
                "不同密钥使用相同名称 — 疑似冒充",
                "同じ名前を別の鍵が使用 — なりすまし疑い",
            ],
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
            Msg::LinkBadgeShape => [
                "Status badge shapes",
                "상태 배지 모양 구분",
                "状态徽章形状",
                "状態バッジの形状",
            ],
            Msg::LinkBadgeShapeDesc => [
                "Show session state by shape as well as color (empty ring / gap ring / filled / bar) — readable without color vision",
                "세션 상태를 색과 함께 모양(빈 링·갭 링·찬 원·막대)으로도 구분 — 색을 못 읽어도 상태가 보인다",
                "用形状与颜色共同表示会话状态(空环/缺口环/实心/横条)",
                "セッション状態を色に加えて形(空リング/欠けリング/塗り/バー)でも表示",
            ],
            Msg::ListSort => ["List order", "목록 정렬", "列表排序", "リストの並び順"],
            Msg::ListSortDesc => [
                "Pinned first, then online peers; within each section: recent chat, then recent presence (same rule offline · name order ignores status)",
                "고정 → 접속 중이 먼저. 각 구획 안은 ① 최근 대화 ② 최근 접속 순(비접속도 동일 기준 · 이름순은 상태 무시)",
                "置顶→在线优先; 区内①最近对话②最近上线(离线同规则·名称序忽略状态)",
                "固定→接続中が先。区画内は①最近の会話②最近の接続(オフライン同基準)",
            ],
            Msg::SortSeen => [
                "Recently seen (ignore online)",
                "최근 접속순(온라인 무관)",
                "最近上线(不分在线)",
                "最近接続順(接続無関係)",
            ],
            Msg::SortChat => [
                "Default (recent chat)",
                "기본(최근 대화)",
                "默认(最近对话)",
                "既定(最近の会話)",
            ],
            Msg::SortOnline => [
                "Online first (recent seen)",
                "온라인순(최근 접속)",
                "在线优先(最近上线)",
                "接続優先(最近接続)",
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
            Msg::ActApply => ["Apply", "적용", "应用", "適用"],
            Msg::ProfileUnsavedHint => [
                "Unsaved changes — Esc again to discard",
                "저장 안 된 변경 — 한 번 더 Esc면 버립니다",
                "未保存的更改 — 再按 Esc 放弃",
                "未保存の変更 — もう一度 Esc で破棄",
            ],
            Msg::ToggleIgnore => ["Off", "미적용", "关", "オフ"],
            Msg::MenuLabel => ["Menu", "메뉴", "菜单", "メニュー"],
            Msg::MenuHelp => ["Help", "도움말", "帮助", "ヘルプ"],
            Msg::MenuGallery => [
                "Controls gallery",
                "컨트롤 갤러리",
                "控件库",
                "コントロールギャラリー",
            ],
            Msg::MenuQuit => ["Quit", "종료", "退出", "終了"],
            Msg::NotifyEnabled => [
                "Desktop notifications",
                "데스크톱 알림",
                "桌面通知",
                "デスクトップ通知",
            ],
            Msg::NotifyEnabledDesc => [
                "Show an OS notification for new messages and file offers while the app is in the background (unverified senders are silent)",
                "앱이 뒤에 있을 때 새 메시지·파일 요청을 OS 알림으로 — 미검증 상대는 소리 없음",
                "应用在后台时以系统通知提示新消息与文件请求(未验证发送者静音)",
                "アプリが背面のとき新着とファイル要求をOS通知で(未検証は無音)",
            ],
            Msg::NotifyPreview => [
                "Show message text in notifications",
                "알림에 메시지 내용 표시",
                "通知中显示消息内容",
                "通知に本文を表示",
            ],
            Msg::NotifyPreviewDesc => [
                "Off shows only \"New message\" — safer on shared or recorded screens (file names are never shown)",
                "끄면 \"새 메시지\"만 표시 — 화면 공유·녹화 자리에서 안전(파일명은 어떤 경우에도 표시하지 않음)",
                "关闭时仅显示\"新消息\"(文件名从不显示)",
                "オフは\"新着\"のみ表示(ファイル名は常に非表示)",
            ],
            Msg::NotifyNewMessage => ["New message", "새 메시지", "新消息", "新着メッセージ"],
            Msg::NotifyFileOffer => [
                "File transfer request",
                "파일 수신 요청",
                "文件传输请求",
                "ファイル受信リクエスト",
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
            Msg::CatServer => ["Server", "서버", "服务器", "サーバー"],
            Msg::ServerMode => ["Server mode", "서버 모드", "服务器模式", "サーバーモード"],
            Msg::ServerModeDesc => [
                "Unmanaged = LAN only (default). Managed = connect through one registered server.",
                "관리 안 함(Unmanaged) = LAN만(기본). 관리형(Managed) = 등록한 서버 1개를 경유해 연결.",
                "非托管 = 仅局域网（默认）。托管 = 通过一个已注册的服务器连接。",
                "非管理 = LANのみ（既定）。管理 = 登録した1つのサーバー経由で接続。",
            ],
            Msg::ServerModeUnmanaged => ["Unmanaged (LAN)", "관리 안 함 (LAN)", "非托管（局域网）", "非管理（LAN）"],
            Msg::ServerModeManaged => ["Managed (server)", "관리형 (서버)", "托管（服务器）", "管理（サーバー）"],
            Msg::ServerAddress => ["Server address", "서버 주소", "服务器地址", "サーバーアドレス"],
            Msg::ServerAddressDesc => [
                "IP or domain of the server. Managed mode only.",
                "서버의 IP 또는 도메인. 관리형 모드에서만 사용됩니다.",
                "服务器的 IP 或域名。仅托管模式。",
                "サーバーのIPまたはドメイン。管理モードのみ。",
            ],
            Msg::ServerPort => ["Server port", "서버 포트", "服务器端口", "サーバーポート"],
            Msg::ServerPortDesc => [
                "Port of the server. Managed mode only.",
                "서버의 포트. 관리형 모드에서만 사용됩니다.",
                "服务器的端口。仅托管模式。",
                "サーバーのポート。管理モードのみ。",
            ],
            Msg::ServerType => ["Server type", "서버 타입", "服务器类型", "サーバータイプ"],
            Msg::ServerTypeDesc => [
                "Auto (recommended) = provided by the server. Relay = hole-punch only. Content = relay + server file delivery. Registered = content + pre-approved signup. Selectable only in Managed mode.",
                "자동(권장) = 서버가 제공. 릴레이 = 홀펀칭만. 컨텐츠 = 릴레이 + 서버 파일 전달. 등록 전용 = 컨텐츠 + 사전 승인 가입. 관리형 모드에서만 직접 선택.",
                "自动（推荐）= 服务器提供。中继 = 仅打洞。内容 = 中继 + 服务器文件传递。注册制 = 内容 + 预批准注册。仅托管模式可选。",
                "自動（推奨）= サーバーが提供。リレー = ホールパンチのみ。コンテンツ = リレー + サーバーファイル配信。登録制 = コンテンツ + 事前承認登録。管理モードのみ選択可。",
            ],
            Msg::ServerTypeAuto => ["Auto (server-provided)", "자동 (서버 제공)", "自动（服务器提供）", "自動（サーバー提供）"],
            Msg::ServerTypeRelay => ["Relay (hole-punch)", "릴레이 (홀펀칭)", "中继（打洞）", "リレー（ホールパンチ）"],
            Msg::ServerTypeContent => ["Content (relay + files)", "컨텐츠 (릴레이+파일)", "内容（中继+文件）", "コンテンツ（リレー+ファイル）"],
            Msg::ServerTypeRegistered => ["Registered (content + signup)", "등록 전용 (컨텐츠+가입)", "注册制（内容+注册）", "登録制（コンテンツ+登録）"],
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

/// 인자 있는 번역(i18n-1 · 08-17) — 번역 템플릿의 `{}`를 **순서대로** `args`로
/// 치환한다. 정적 `t()`로는 못 담는 동적 문자열(파일명·수·오류 등)용. 언어마다
/// 어순이 달라도 템플릿이 언어별로 다르므로 `{}` 위치가 자연스럽게 맞는다.
/// `{}`보다 args가 적으면 남은 `{}`는 그대로 둔다(누락 티가 나게 · fail-visible).
#[must_use]
pub fn tf(msg: Msg, args: &[&str]) -> String {
    let template = t(msg);
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    let mut it = args.iter();
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        match it.next() {
            Some(a) => out.push_str(a),
            None => out.push_str("{}"), // args 소진 — 자리 유지
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
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
    fn tf_replaces_in_order() {
        set_lang(Lang::En);
        // 임의 템플릿 키로 순서 치환 검증(Theme는 '{}' 없어 그대로).
        assert_eq!(tf(Msg::Theme, &["x"]), "Theme"); // {} 없으면 원문
        set_lang(Lang::En);
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
