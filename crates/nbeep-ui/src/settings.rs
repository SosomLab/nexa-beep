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
    Button, ColorPicker, Combo, ComboControl, ComboItem, Control, LabelSide, PositionDropdown,
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
/// 폰트 크기 프리셋(08-18 2차 확정 — **값은 절대 px · 라벨은 이름만**):
/// 크기 안내는 Base UI 설명문이 진다("Normal (16px)" 병기는 원복).
/// 기본은 **전 슬롯 Normal(16px)** · Small = 14px(사용자 확정).
const FONT_SIZE_OPTS: &[(&str, Msg)] = &[
    ("14", Msg::SizeSmall),
    ("16", Msg::SizeNormal),
    ("18", Msg::SizeLarge),
    ("22", Msg::SizeXLarge),
];

/// 폰트 크기 기본(px) — 전 슬롯 공통 Normal(사용자 확정 08-18).
const FONT_SIZE_DEFAULT: &str = "16";

/// [`SIZE_OPTS`]의 `Radio` kind용 정적 참조(컨트롤 크기 항목 재사용).
const SIZE_OPTS_STATIC: &[(&str, Msg)] = SIZE_OPTS;

/// 설정 **화면에는 없지만** 영속되는 키(M3-17 프로필 화면이 쓴다) — 기본 빈 문자열.
/// ⚠ 이메일·전화는 PII다 — 평문 settings.cfg 보관은 잠정이며 M2-5b(암호화 저장)로
/// 이관 후보(journal 08-11 명기).
const HIDDEN_KEYS: &[&str] = &[
    "profile.email",
    "profile.phone",
    "profile.bio", // 소개글(08-17) — 프로필 화면 멀티라인 필드가 편집·영속
    "profile.image_path",
    "profile.avatar",        // 아바타 선택(08-14) — 프로필 화면 스와치가 편집한다
    "profile.avatar_border", // 아바타 보더 색(08-14) — 프로필 화면 ColorPick이 편집한다
    // 최근 프로필 이미지(08-14 — 탭 구분 목록). ★ 여기 없으면 저장은 되는데
    // **부팅 로드에서 미지 키로 무시**돼 재시작마다 목록이 증발한다(실기로 잡음).
    "profile.image_recent",
    // 사용자 인증 마커(ADR-0015 · 09-06) — 값(핸들·암호)에 붙는 검증 상태. 성공 = on.
    "user.verified",
    // 서명 기기 목록 버전(ADR-0015 S2-e) — 내 기기 집합이 바뀔 때마다 +1(롤백 방지 · 단조).
    "user.list_ver",
    // 마지막 발급 발신 seq(ADR-0015 S4 · 10-10) — 따라잡기 열쇠 `(기기, seq)`가 재시작을 넘어
    // 단조여야 해서 부팅이 `max(이 값+1, 현재 ms)`로 시퀀서를 잇는다(저장은 quiet 지연이라
    // 현재 ms 하한이 크래시 뒤 재사용을 막는다).
    "chat.seq_last",
    // 목록 필터 바(08-22) — 툴바 아래 칩 3그룹의 선택 영속("" = 전체).
    // ★ 키 등록과 값 저장은 쌍 — 여기 없으면 재시작 로드가 미지 키로 흘린다.
    "list.filter.path",
    "list.filter.presence",
    "list.filter.trust",
    // 창 위치·크기 기억(08-14) — Moved/Resized가 쓰고 기동이 읽는다.
    "ui.win_x",
    "ui.win_y",
    "ui.win_w",
    "ui.win_h",
    // 서버 연결 검증 마커(08-22 — "주소 핀64hex unix초"). Test 버튼·자동 등록 성공이
    // 갱신하고, 재시작 후에도 "이미 검증된 서버"를 알 수 있게 영속한다.
    "server.verified",
    // 자동 실행 "등록해 둔 적 있음" 마커(08-24 — "on"/"off"·기본 ""). 부팅 동기화가
    // OS 등록 부재를 **외부 삭제**(사용자가 레지스트리 등에서 지움)로 판정하는 기준 —
    // 마커 없이는 첫 실행과 삭제를 구분할 수 없어 무조건 재등록하게 된다.
    "app.autostart_reg",
    // ── P2 설정 체계 개편(10-09 · docs/50) ──
    // 고급 설정 스위치 상태(기본 off) — 설정 창 하단 스위치가 쓰고 열 때 되살린다.
    "ui.prefs_advanced",
    // 설정 검색 이력(탭 구분 · 최근 20) — 검색창 ↑/↓.
    "prefs.search",
    // 설정 창 위치·크기 기억(논리 px · 메인 창 `ui.win_*`와 같은 규약).
    "ui.prefs_x",
    "ui.prefs_y",
    "ui.prefs_w",
    "ui.prefs_h",
    // 라이선스 게이트 스위치(P4 · 기본 "" = off — D-145 계승 "개인 사용은 전 기능 오픈").
    "license.gates",
];

/// ★ 설정 트리(10-09 · nexa-sql `CATEGORY_TREE` 차용 — DBeaver Preferences 모양): **그룹 → 카테고리**.
/// 사이드바·표시 순서·검색 결과 정렬의 단일 원천. 카테고리 안 하위 그룹(`Entry::sub`)은 본문 섹션 제목으로만 쓴다.
/// 새 카테고리는 여기 한 줄 — 트리에 없는 카테고리의 Entry는 시험 `tree_covers_every_category`가 잡는다.
pub const CATEGORY_TREE: &[(Msg, &[Msg])] = &[
    (
        Msg::GrpGeneral,
        &[Msg::CatSystem, Msg::CatProfile, Msg::CatUser],
    ),
    (
        Msg::GrpConversation,
        &[Msg::CatConversation, Msg::CatNotify, Msg::CatGroup],
    ),
    (
        Msg::GrpAppearance,
        &[
            Msg::CatAppearance,
            Msg::CatColors,
            Msg::CatFont,
            Msg::CatPeerList,
        ],
    ),
    (Msg::GrpFiles, &[Msg::CatFiles]),
    (Msg::GrpNetwork, &[Msg::CatNetwork, Msg::CatServer]),
    (Msg::GrpAdvanced, &[Msg::CatAdvanced, Msg::CatIme]),
];

/// 트리 안 위치 `(그룹 순서, 카테고리 순서)` — 없으면 맨 뒤.
#[must_use]
pub fn tree_pos(cat: Msg) -> (usize, usize) {
    for (gi, (_, cats)) in CATEGORY_TREE.iter().enumerate() {
        if let Some(ci) = cats.iter().position(|c| *c == cat) {
            return (gi, ci);
        }
    }
    (usize::MAX, usize::MAX)
}

/// 카테고리가 속한 그룹.
#[must_use]
pub fn group_of(cat: Msg) -> Option<Msg> {
    CATEGORY_TREE
        .iter()
        .find(|(_, cats)| cats.contains(&cat))
        .map(|(g, _)| *g)
}

/// ★ 고급 설정(10-09 · 설정 창 "고급 설정" 스위치 대상) — 한 번 정하면 거의 손대지 않는 구현값·시간 상수.
/// 글꼴·색·모드·켜기/끄기 같은 습관값은 기본 표시로 남긴다. 표에 없는 키는 시험 `advanced_keys_exist`가 잡는다.
pub const ADVANCED: &[&str] = &[
    // 한글 입력(IME) 튜닝 전부 — macOS 실측 기준값(H-27). 카테고리째 고급.
    "ime.inject",
    "ime.leak",
    "ime.stale_ms",
    "ime.same_key_ms",
    "ime.pending_ms",
    "ime.echo_ms",
    "ime.stash_ms",
    "ime.owed_ms",
    "ime.pre_clear_ms",
    "ime.swallow_ms",
    "ime.selfcommit_ms",
    // 시간 상수·주기
    "ui.tooltip_ms",
    "ui.scrollbar_hide",
    "ui.list_refresh_ms",
    "ui.list_refresh_scroll",
    "ui.typeahead_timeout",
    "ui.typeahead_pos",
    "ui.typeahead_space",
    "ui.typeahead_special",
    "xfer.timeout_sec",
    "xfer.auto_cancel_min",
    "group.resync_keep",
    "net.session_port",
    "log.retain_days",
    "log.max_total_mb",
    "netmon.enabled",
    "netmon.interval_s",
];

/// 고급 설정인가(설정 창 스위치가 꺼져 있으면 숨기고 수만 센다).
#[must_use]
pub fn is_advanced(key: &str) -> bool {
    ADVANCED.contains(&key)
}

/// 종속 조건(10-09 · nexa-sql `Dep` 차용 — "종속 설정은 부모가 조건을 만족할 때만 만질 수 있다").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dep {
    /// 부모가 `on`.
    On,
    /// 부모가 이 값.
    Eq(&'static str),
}

impl Dep {
    /// 부모 값이 조건을 만족하는가.
    #[must_use]
    pub fn satisfied(self, parent_value: &str) -> bool {
        match self {
            Dep::On => parent_value == "on",
            Dep::Eq(v) => parent_value == v,
        }
    }
}

/// (자식, 부모, 조건) — 부모가 조건을 만족하지 않으면 자식 행은 **잠긴다**(값은 유지 · 흐리게 + 안내 1줄).
/// 호스트가 `set_disabled`로 거는 런타임 잠금과 **합집합**이다.
pub const DEPENDS: &[(&str, &str, Dep)] = &[
    ("notify.preview", "notify.enabled", Dep::On),
    ("notify.broadcast_mute", "notify.enabled", Dep::On),
    ("ui.tray_hide_taskbar", "ui.close_to_tray", Dep::On),
    ("ui.typeahead_timeout", "ui.typeahead", Dep::On),
    ("ui.typeahead_pos", "ui.typeahead", Dep::On),
    ("ui.typeahead_space", "ui.typeahead", Dep::On),
    ("ui.typeahead_special", "ui.typeahead", Dep::On),
    ("user.handle", "user.enabled", Dep::On),
    ("user.passphrase", "user.enabled", Dep::On),
    ("user.test", "user.enabled", Dep::On),
    ("user.rotate", "user.enabled", Dep::On),
    ("user.devices", "user.enabled", Dep::On),
    ("net.server.address", "net.server.mode", Dep::Eq("managed")),
    ("net.server.port", "net.server.mode", Dep::Eq("managed")),
    ("net.server.type", "net.server.mode", Dep::Eq("managed")),
    ("net.server.announce", "net.server.mode", Dep::Eq("managed")),
    ("net.server.test", "net.server.mode", Dep::Eq("managed")),
    (
        "xfer.remote_files_server",
        "net.server.mode",
        Dep::Eq("managed"),
    ),
    ("log.retain_days", "log.enabled", Dep::On),
    ("log.max_total_mb", "log.enabled", Dep::On),
    ("log.view", "log.enabled", Dep::On),
    ("netmon.interval_s", "netmon.enabled", Dep::On),
];

/// 이 키의 종속(부모 키, 조건) — 없으면 `None`.
#[must_use]
pub fn depends_of(key: &str) -> Option<(&'static str, Dep)> {
    DEPENDS
        .iter()
        .find(|(c, _, _)| *c == key)
        .map(|(_, p, d)| (*p, *d))
}

/// ★ 표시 순서(10-09 · nexa-sql `display_order` 차용) = (그룹, 카테고리, 하위 섹션, 키 접두 첫 등재, 등재 순).
/// 같은 카테고리에 여러 접두(`ui.`·`app.`)가 섞여도 접두끼리 모인다.
#[must_use]
pub fn display_order(idx: usize) -> (usize, usize, usize, usize, usize) {
    let e = &registry()[idx];
    let (g, c) = tree_pos(e.cat);
    // 하위 섹션: 직속(None) = 0 · 하위는 같은 카테고리 안 첫 등재 순.
    let si = match e.sub {
        None => 0,
        Some(sub) => {
            registry()
                .iter()
                .filter(|x| x.cat == e.cat)
                .filter_map(|x| x.sub)
                .fold((Vec::<Msg>::new(), 0usize), |(mut seen, _), x| {
                    if !seen.contains(&x) {
                        seen.push(x);
                    }
                    let pos = seen
                        .iter()
                        .position(|y| *y == sub)
                        .map_or(usize::MAX, |p| p + 1);
                    (seen, pos)
                })
                .1
        }
    };
    let prefix = e.key.split('.').next().unwrap_or(e.key);
    let first = registry()
        .iter()
        .position(|x| {
            x.cat == e.cat && x.sub == e.sub && x.key.split('.').next().unwrap_or(x.key) == prefix
        })
        .unwrap_or(idx);
    (g, c, si, first, idx)
}

/// 직접 입력이 **텍스트**인 RadioInput 키(08-22) — 기본은 숫자 전용(포트·ms·MiB).
/// 서버 주소는 도메인·IP를 받아야 해서 숫자 필터가 입력 자체를 막았다(실기).
const FREE_TEXT_KEYS: &[&str] = &["net.server.address"];

/// ★ 암호 미리보기 아이콘(Material `password visibility` 96² 알파 — nexa-clip 09-03 자산 공유).
const PW_EYE_ALPHA: &[u8] = include_bytes!("../assets/icon-pw-eye-96.alpha");
/// ★ 암호 생성 아이콘(Material `flip_camera_android` 96² 알파 — clip 동일).
const PW_REGEN_ALPHA: &[u8] = include_bytes!("../assets/icon-pw-regen-96.alpha");
/// 자산 변 크기(px).
const PW_EYE_SIDE: u32 = 96;
/// 생성 버튼 무장 유지 시간(clip 09-03 사용자 — "2초 내에 다시 누르지 않으면 원복").
/// 생성 무장 창(09-06 사용자 요청 — 키 교체와 같은 5초 · 무장 중엔 행 노트에 남은 시간 안내).
pub const PW_ARM_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// 비밀 행 버튼 자리(clip 09-03 사용자 확정 "두 버튼을 텍스트 우상단으로") — 상자 위 한 줄,
/// 오른쪽 끝 정렬 \[생성\]\[눈\] · 버튼 크기 = 상자 높이 · 간격 = 높이/8.
fn pw_btn_rects(b: Rect) -> (Rect, Rect) {
    // 10-09 카드 레이아웃: 컨트롤이 좌하단으로 가면서 버튼은 **상자 오른쪽** 같은 줄에([생성][눈] 순).
    let regen = Rect::new(b.right() + b.h / 8, b.y, b.h, b.h);
    let eye = Rect::new(regen.right() + b.h / 8, b.y, b.h, b.h);
    (eye, regen)
}

/// 96² 알파 자산을 잉크색으로 틴트해 캐시에 담는다(색이 같으면 재사용).
fn tint_icon(
    cell: &std::cell::RefCell<Option<(u32, crate::theme::IconImage)>>,
    alpha: &[u8],
    ink: u32,
) {
    let mut cache = cell.borrow_mut();
    let stale = !matches!(cache.as_ref(), Some((c, _)) if *c == ink);
    if stale {
        let (r, g, b) = ((ink >> 16) as u8, (ink >> 8) as u8, ink as u8);
        let mut rgba = Vec::with_capacity(alpha.len() * 4);
        for &a in alpha {
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        *cache = Some((
            ink,
            crate::theme::IconImage::from_rgba(PW_EYE_SIDE, PW_EYE_SIDE, rgba),
        ));
    }
}

/// 틴트된 아이콘을 버튼 자리에 그린다(안쪽 여백 = 높이/8).
fn draw_pw_icon(
    cell: &std::cell::RefCell<Option<(u32, crate::theme::IconImage)>>,
    r: Rect,
    ctx: &mut dyn DrawCtx,
) {
    if let Some((_, img)) = cell.borrow().as_ref() {
        let ins = r.h / 8;
        let dst = Rect::new(r.x + ins, r.y + ins, r.w - ins * 2, r.h - ins * 2);
        ctx.image_scaled(dst, img, r);
    }
}

/// 기본 off 토글 — 프로필 공개(DR-22 **기본 전부 비노출** · 옵트인). 미등록 토글은 on.
// ★ M3-2d ① 확정(08-15 **사용자 확정 = 3-OS 공통 off**): `ui.close_to_tray`
// 기본 꺼짐 — X = 종료(예측 가능성 우선). mac 관례 차등(빨간 버튼 = 유지)을
// 권고했으나 사용자가 공통 off로 확정했다. 켜는 것은 설정 고급에서 옵트인.
const TOGGLE_DEFAULT_OFF: &[&str] = &[
    "ui.close_to_tray",      // 닫기 = 트레이(M3-2d — 기본 off · 사용자 확정)
    "log.enabled",           // 상태 로그(M3-22 — 기본 off · 사용자 확정 08-18)
    "netmon.enabled",        // 네트워크 점검 기록(08-21 — 기본 off · 의도적으로 켤 때만)
    "notify.preview",        // 알림 본문 표시(M3-8 — 기본 끔: 화면 공유·녹화 안전)
    "user.enabled", // 다중 기기 신원(ADR-0015 — 기본 끔: 옵트인 오버레이 S-0 · 사용자 확정 09-06)
    "notify.broadcast_mute", // 공지 받지 않기(08-21 — 기본 끔 = 공지 받음)
    // 원격 파일 발신 옵트인 2종(08-23 분리 — **기본 끄기** · 사용자 확정. ⚠08-23
    // 실기 발각: 여기 없으면 Toggle 기본 on이라 켜진 채 나갔다).
    "xfer.remote_files_server",
    "xfer.remote_files_internet",
    "profile.share.basic",
    "profile.share.email",
    "profile.share.phone",
];

/// Radio 기본값 예외 — 표시 순서(오름차순 등)와 기본값이 다른 키만 등록.
/// 미등록 키의 기본은 첫 옵션(기존 규약).
const RADIO_DEFAULTS: &[(&str, &str)] = &[
    ("ui.toolbar_size", "32"),
    ("xfer.send_max_mb", "unlimited"), // 발신 = 무제한 기본(08-18 — 스트리밍이라 안전)
    ("xfer.batch_max", "5"),           // 요청당 최대 파일 수 = 5 기본(08-20 확정)
    ("xfer.recv_max_mb", "512"),       // 수신 = 512MiB 기본(메모리 조립 방어선)
    // 정지 방치 자동 취소 — 기본 2분(사용자 확정 08-20 · 표시 순서는 1·2·5·10).
    ("xfer.auto_cancel_min", "2"),
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
    /// 3×3 위치 — **이미지 드롭다운**([`PositionDropdown`] · 선택 타일 + ▾ · 팝업 = 그리드 · 10-09 사용자 "콤보 방식으로"
    /// = nexa-sql 설정과 동일). 값 = 위치 코드(`bl` 등).
    PositionGrid,
    /// 자유 텍스트 한 줄 — [`TextBox`] 행(clip 09-03 이식). `secret` = 비밀 행: 기본 `•` 가림 ·
    /// 상자 위 오른쪽에 \[생성\]\[눈\] 아이콘 버튼(생성은 2초 무장 후 2차 클릭 · 생성 시 자동 표시).
    Text {
        /// placeholder.
        hint: Msg,
        /// 비밀 행인가.
        secret: bool,
    },
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
    /// ★ 읽기 전용 정보(10-09 · nexa-sql `INFO_KEYS` 차용 — P4 라이선스 상태 등): 호스트가 `set_info`로
    /// 그때그때 채운 글을 잠긴 칸으로 보여 준다. 값 키가 없어 영속되지 않는다.
    Info,
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
    /// 레지스트리 기본값(각 값 키 → 기본 문자열). **한 항목이 값 키를 여럿 가질 수
    /// 있다**(FontSection = family+size) — 초기화·시드가 `key` 하나만 돌면 짝 키가
    /// 새므로, 기본값을 다루는 쪽은 반드시 이 목록을 쓴다(08-15 초기화 점검).
    pub fn default_values(&self) -> Vec<(&'static str, String)> {
        match self.kind {
            SettingKind::Radio(opts) | SettingKind::RadioInput(opts, _) => RADIO_DEFAULTS
                .iter()
                .find(|(k, _)| *k == self.key)
                .map(|(_, v)| *v)
                .or_else(|| opts.first().map(|(v, _)| *v))
                .map(|v| (self.key, v.to_string()))
                // ★ 옵션 없는 자유 입력형(RadioInput(&[], _) — net.server.address)도
                //   **빈 기본값으로 키를 등록**한다. 키가 values 맵에 없으면
                //   set_by_name이 "미지 키"로 흘려 — 화면에서 입력·저장한 값이
                //   재시작 때마다 조용히 증발했다(08-22 X-2b 배선이 발각 · 08-14
                //   "저장은 되는데 로드가 무시" 영속 구멍과 같은 유형).
                .or_else(|| {
                    matches!(self.kind, SettingKind::RadioInput(..))
                        .then(|| (self.key, String::new()))
                })
                .into_iter()
                .collect(),
            // 자유 텍스트도 **빈 기본값으로 키를 등록**한다(RadioInput 자유 입력과 같은 이유).
            SettingKind::Text { .. } => vec![(self.key, String::new())],
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
                (size_key, FONT_SIZE_DEFAULT.to_string()),
            ],
            // 행위·정보 항목은 값이 없다 — 영속·검증 대상에서 자연히 빠진다.
            SettingKind::Action { .. } | SettingKind::Info => vec![],
        }
    }
}

/// 입력 검증 규칙(08-20 — 확정 시 즉시 검사): `Ok` = 적용 · `Err(경고 Msg)` =
/// 호스트가 경고를 띄우고 컨트롤은 **직전 확정값으로 원복**([`Control::last_value`]).
/// 새 규칙은 여기 한 곳에만 추가한다(검증·경고·원복 배선은 공용).
fn validate(key: &str, value: &str) -> Result<(), Msg> {
    match key {
        // 정지 방치 자동 취소 — 1~10분 범위(사용자 확정 08-20).
        "xfer.auto_cancel_min" => {
            if value.parse::<u64>().is_ok_and(|v| (1..=10).contains(&v)) {
                Ok(())
            } else {
                Err(Msg::ValMinutesRange)
            }
        }
        // 타입어헤드 유효시간 — 200~60000ms(nexa-sql `explorer.typeahead_timeout_ms` Int 범위 차용 · 10-09).
        "ui.typeahead_timeout" => {
            if value
                .parse::<u64>()
                .is_ok_and(|v| (200..=60_000).contains(&v))
            {
                Ok(())
            } else {
                Err(Msg::ValTypeaheadRange)
            }
        }
        _ => Ok(()),
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
        // 수신 확인(N-2 · ADR-0010 §5 · 사용자 요청 08-17 — **수신자 제어**). 기본 on ·
        // 켜도 검증된 상대에게만 자동 Delivered(미검증엔 안 보냄 — 프라이버시 게이트).
        Entry {
            cat: Msg::CatConversation,
            sub: None,
            label: Msg::SendDelivered,
            desc: Msg::SendDeliveredDesc,
            kind: SettingKind::Toggle,
            key: "chat.send_delivered",
        },
        // 읽음 확인(N-2 · 사용자 요청 08-17 — 전달과 **독립** · 수신자 제어 · 기본 on).
        // 대화창을 열어 읽으면 읽음이 발신자에게(검증 상대만 · 프라이버시 게이트).
        Entry {
            cat: Msg::CatConversation,
            sub: None,
            label: Msg::SendRead,
            desc: Msg::SendReadDesc,
            kind: SettingKind::Toggle,
            key: "chat.send_read",
        },
        // 알림(M3-8 최소 슬라이스) — 표시 on/off + 본문 미리보기(기본 끔 = FR-S-42 결).
        Entry {
            cat: Msg::CatNotify,
            sub: None,
            label: Msg::NotifyEnabled,
            desc: Msg::NotifyEnabledDesc,
            kind: SettingKind::Toggle,
            key: "notify.enabled",
        },
        Entry {
            cat: Msg::CatNotify,
            sub: None,
            label: Msg::NotifyPreview,
            desc: Msg::NotifyPreviewDesc,
            kind: SettingKind::Toggle,
            key: "notify.preview",
        },
        // 공지(브로드캐스트) 받지 않기(08-21 사용자 확정 — 옵트아웃 · 기본 받음).
        Entry {
            cat: Msg::CatNotify,
            sub: None,
            label: Msg::NotifyBroadcastMute,
            desc: Msg::NotifyBroadcastMuteDesc,
            kind: SettingKind::Toggle,
            key: "notify.broadcast_mute",
        },
        Entry {
            cat: Msg::CatSystem,
            sub: None,
            label: Msg::Theme,
            desc: Msg::ThemeDesc,
            // 시스템 추종이 기본(08-29 사용자 확정 · 첫 옵션 = 기본값).
            kind: SettingKind::Radio(&[
                ("system", Msg::ThemeSystem),
                ("dark", Msg::ThemeDark),
                ("light", Msg::ThemeLight),
            ]),
            key: "ui.theme",
        },
        // ── 테마 주요 색(08-10 · 사용자 요청) — 다크/라이트 각각. 즉시 적용(영속은 M3-15). ──
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorAccent,
            desc: Msg::ColorAccentDesc,
            kind: SettingKind::Color { default: "#3D8BFF" },
            key: "theme.dark.accent",
        },
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorBubblePeer,
            desc: Msg::ColorBubblePeerDesc,
            kind: SettingKind::Color { default: "#313947" },
            key: "theme.dark.bubble_peer",
        },
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorPanelBg,
            desc: Msg::ColorPanelBgDesc,
            kind: SettingKind::Color { default: "#191C21" },
            key: "theme.dark.panel_bg",
        },
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsDark),
            label: Msg::ColorText,
            desc: Msg::ColorTextDesc,
            kind: SettingKind::Color { default: "#D6DAE0" },
            key: "theme.dark.text",
        },
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorAccent,
            desc: Msg::ColorAccentDesc,
            kind: SettingKind::Color { default: "#3D8BFF" },
            key: "theme.light.accent",
        },
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorBubblePeer,
            desc: Msg::ColorBubblePeerDesc,
            kind: SettingKind::Color { default: "#E2E7EE" },
            key: "theme.light.bubble_peer",
        },
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorPanelBg,
            desc: Msg::ColorPanelBgDesc,
            kind: SettingKind::Color { default: "#FFFFFF" },
            key: "theme.light.panel_bg",
        },
        Entry {
            cat: Msg::CatColors,
            sub: Some(Msg::CatColorsLight),
            label: Msg::ColorText,
            desc: Msg::ColorTextDesc,
            kind: SettingKind::Color { default: "#1B1F26" },
            key: "theme.light.text",
        },
        Entry {
            cat: Msg::CatSystem,
            sub: None,
            label: Msg::Language,
            desc: Msg::LanguageDesc,
            // ★ `system`(기본 · 10-09 D-33-4) = 부팅마다 OS 표시 언어 추종(nexa-sys locale · 미지원 = en).
            //   명시 선택이 우선 · 변경 즉시 반영(재시작 없음). 해석은 앱(`resolve_lang`).
            kind: SettingKind::Radio(&[
                ("system", Msg::LangSystem),
                ("en", Msg::LangEnglish),
                ("ko", Msg::LangKorean),
                ("zh", Msg::LangChinese),
                ("ja", Msg::LangJapanese),
            ]),
            key: "ui.language",
        },
        // 컨트롤 글리프 크기(체크·스위치·옵션박스 — 08-11 사용자 요청 · 기본 "크게").
        Entry {
            cat: Msg::CatSystem,
            sub: None,
            label: Msg::ControlSize,
            desc: Msg::ControlSizeDesc,
            kind: SettingKind::Radio(SIZE_OPTS_STATIC),
            key: "ui.control_size",
        },
        Entry {
            cat: Msg::CatSystem,
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
            cat: Msg::CatPeerList,
            sub: None,
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
            cat: Msg::CatPeerList,
            sub: None,
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
            cat: Msg::CatPeerList,
            sub: None,
            label: Msg::ListScroll,
            desc: Msg::ListScrollDesc,
            kind: SettingKind::Radio(&[
                ("keep", Msg::ListScrollKeep),
                ("caret", Msg::ListScrollCaret),
                ("top", Msg::ListScrollTop),
            ]),
            key: "ui.list_refresh_scroll",
        },
        // 세션 배지 실루엣(M3-19) — 색+모양 2중 부호화. off = 종전 채운 원(색만).
        Entry {
            cat: Msg::CatPeerList,
            sub: None,
            label: Msg::LinkBadgeShape,
            desc: Msg::LinkBadgeShapeDesc,
            kind: SettingKind::Toggle,
            key: "ui.link_badge_shape",
        },
        // 타입어헤드 마스터(10-09 · nexa-sql `explorer.typeahead` 차용) — 끄면 글자 입력이 목록 탐색에 안 쓰인다 ·
        // 아래 4항목은 이 스위치에 종속(DEPENDS).
        Entry {
            cat: Msg::CatPeerList,
            sub: Some(Msg::CatTypeahead),
            label: Msg::TypeaheadEnable,
            desc: Msg::TypeaheadEnableDesc,
            kind: SettingKind::Toggle,
            key: "ui.typeahead",
        },
        Entry {
            cat: Msg::CatPeerList,
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
            cat: Msg::CatPeerList,
            sub: Some(Msg::CatTypeahead),
            label: Msg::TypeaheadPos,
            desc: Msg::TypeaheadPosDesc,
            kind: SettingKind::PositionGrid,
            key: "ui.typeahead_pos",
        },
        Entry {
            cat: Msg::CatPeerList,
            sub: Some(Msg::CatTypeahead),
            label: Msg::TypeaheadSpace,
            desc: Msg::TypeaheadSpaceDesc,
            kind: SettingKind::Toggle,
            key: "ui.typeahead_space",
        },
        Entry {
            cat: Msg::CatPeerList,
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
        // 파일 크기 상한(08-18 사용자 요청 — 발신/수신 각각 · 송수신 전 점검).
        // 값 = MiB 숫자 · "unlimited" = 무제한 · 커스텀 = 직접 입력(MiB).
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::XferSendMax,
            desc: Msg::XferSendMaxDesc,
            // 커스텀 제외(사용자 확정 08-18) — 발신은 스트리밍이라 프리셋으로 충분.
            kind: SettingKind::Radio(&[
                ("100", Msg::Cap100MiB),
                ("256", Msg::Cap256MiB),
                ("512", Msg::Cap512MiB),
                ("1024", Msg::Cap1GiB),
                ("unlimited", Msg::CapUnlimited),
            ]),
            key: "xfer.send_max_mb",
        },
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::XferRecvMax,
            desc: Msg::XferRecvMaxDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("100", Msg::Cap100MiB),
                    ("256", Msg::Cap256MiB),
                    ("512", Msg::Cap512MiB),
                    ("1024", Msg::Cap1GiB),
                    ("unlimited", Msg::CapUnlimited),
                ],
                "MiB",
            ),
            key: "xfer.recv_max_mb",
        },
        // 요청당 최대 파일 수(M4-2e · 08-20) — 제외 포함 합산 기준 · 기본 5.
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::XferBatchMax,
            desc: Msg::XferBatchMaxDesc,
            kind: SettingKind::Radio(&[
                ("1", Msg::Cnt1),
                ("2", Msg::Cnt2),
                ("3", Msg::Cnt3),
                ("4", Msg::Cnt4),
                ("5", Msg::Cnt5),
            ]),
            key: "xfer.batch_max",
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
        // 정지 방치 자동 취소(M4-2e ⓓ · 사용자 확정 08-20) — 일시중지 전송이 이
        // 시간 동안 방치되면 양쪽 모두 전체 취소. Custom = 분 직접 입력.
        Entry {
            cat: Msg::CatFiles,
            sub: None,
            label: Msg::XferAutoCancel,
            desc: Msg::XferAutoCancelDesc,
            kind: SettingKind::RadioInput(
                &[
                    ("1", Msg::Min1),
                    ("2", Msg::Min2),
                    ("5", Msg::Min5),
                    ("10", Msg::Min10),
                ],
                "분",
            ),
            key: "xfer.auto_cancel_min",
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
        // ── 서버(ADR-0013 · 08-17) — ★ **개발 범위 = 이 설정값 저장까지만**.
        //   Managed 네트워킹·릴레이 서버는 TODO(개발 제외 · 사용자 확정). 기본
        //   Unmanaged(Radio 첫 옵션 = 자동 기본)라 서버를 몰라도 제품이 완전하다(S-0).
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::ServerMode,
            desc: Msg::ServerModeDesc,
            kind: SettingKind::Radio(&[
                ("unmanaged", Msg::ServerModeUnmanaged),
                ("managed", Msg::ServerModeManaged),
            ]),
            key: "net.server.mode",
        },
        // 서버 주소·포트 — Managed에서만 의미(값은 항상 영속 · 네트워킹 배선은 TODO).
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::ServerAddress,
            desc: Msg::ServerAddressDesc,
            // ★ 기본 서버 = SosomLab 공식 릴레이(08-22 사용자 등록 — DNS 실측 확인).
            //   첫 옵션 = 기본값 규약이라 Managed로 켜면 바로 이 서버로 붙는다.
            //   라벨 = 실값(08-22 ② 교훈 — 라벨과 값이 갈릴 자리엔 값을 라벨로).
            kind: SettingKind::RadioInput(&[("beepd.sosomlab.com", Msg::ServerAddrDefault)], ""),
            key: "net.server.address",
        },
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::ServerPort,
            desc: Msg::ServerPortDesc,
            // ⚠ 라벨은 실값을 그대로 보여 준다(08-22 실기 — Msg::PortDefault를 공유하면
            // 세션 포트의 "기본(47200)" 문구가 그대로 떠서, 저장값 47300이 47200으로
            // **잘못 보였다**. 라벨과 값이 갈릴 수 있는 자리엔 값 자체를 라벨로).
            kind: SettingKind::RadioInput(&[("47300", Msg::Port47300)], ""),
            key: "net.server.port",
        },
        // 서버 타입 — 기본 auto(서버 제공). Managed에서만 직접 선택(제약은 네트워킹
        // 슬라이스에서 · 지금은 값만 저장). Radio 첫 옵션 = auto가 기본.
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::ServerType,
            desc: Msg::ServerTypeDesc,
            kind: SettingKind::Radio(&[
                ("auto", Msg::ServerTypeAuto),
                ("relay", Msg::ServerTypeRelay),
                ("content", Msg::ServerTypeContent),
                ("registered", Msg::ServerTypeRegistered),
            ]),
            key: "net.server.type",
        },
        // 프레즌스 공개(X-2e roster · 08-22 확정 ⓐ 기본 on) — 같은 서버 공개
        // 사용자 목록에 나를 싣고 그 목록을 받는다(존재만 — 이름·프로필은 P2P).
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::ServerAnnounce,
            desc: Msg::ServerAnnounceDesc,
            kind: SettingKind::Toggle,
            key: "net.server.announce",
        },
        // 연결 테스트(08-22 사용자 요청) — 지금 설정값으로 서버에 실제 붙어 본다.
        // 성공 = 검증 마커(server.verified — HIDDEN_KEYS) 영속: 자동 등록 성공도
        // 같은 마커를 갱신하므로, 한 번 검증된 서버는 다시 누를 필요가 없다.
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::ServerTest,
            desc: Msg::ServerTestDesc,
            kind: SettingKind::Action {
                verb: Msg::ServerTestVerb,
            },
            key: "net.server.test",
        },
        // 원격 경로 파일 발신(08-22 확정 · 08-23 **경로별 2분리** — 사용자 확정):
        // 서버 경유/인터넷 직결 대화의 **발신**만 각각 게이트한다(기본 둘 다 끄기).
        // 수신은 제한 없음 — 승인 창이 경로를 표시하고 사람이 결정한다.
        // ⚠ Managed 잠금 목록에 넣지 않는다 — 인터넷 직결(수동 IP)은 Unmanaged에서도 있다.
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::RemoteFilesServerOpt,
            desc: Msg::RemoteFilesServerOptDesc,
            kind: SettingKind::Toggle,
            key: "xfer.remote_files_server",
        },
        Entry {
            cat: Msg::CatServer,
            sub: None,
            label: Msg::RemoteFilesInternetOpt,
            desc: Msg::RemoteFilesInternetOptDesc,
            kind: SettingKind::Toggle,
            key: "xfer.remote_files_internet",
        },
        // ── 사용자(ADR-0015 · DR-29 · 09-06) — 다중 기기 신원 ──
        // 스위치 off = 단독 노드(기본 · 입력란 잠금). on = 빈 칸 기본값 자동 채움 + 자동 인증.
        // ★ 필수 값이 비거나 형식이 틀리면 **어디에도 등록되지 않는다**(빈 사용자로 묶임 방지).
        Entry {
            cat: Msg::CatUser,
            sub: None,
            label: Msg::UserEnabled,
            desc: Msg::UserEnabledDesc,
            kind: SettingKind::Toggle,
            key: "user.enabled",
        },
        Entry {
            cat: Msg::CatUser,
            sub: None,
            label: Msg::UserHandle,
            desc: Msg::UserHandleDesc,
            kind: SettingKind::Text {
                hint: Msg::UserHandle,
                secret: false,
            },
            key: "user.handle",
        },
        // 암호는 PII_KEYS(앱) — settings.cfg가 아니라 봉인 사이드카 profile.sec에 영속.
        Entry {
            cat: Msg::CatUser,
            sub: None,
            label: Msg::UserPass,
            desc: Msg::UserPassDesc,
            // 비밀 행 — 상자 위 [생성][눈] 버튼(clip 동일). 생성 = 가짜 키 `user.passphrase.regen` = run.
            kind: SettingKind::Text {
                hint: Msg::UserPass,
                secret: true,
            },
            key: "user.passphrase",
        },
        // 인증 테스트 — 성공 = 마커(user.verified — HIDDEN_KEYS) 영속 · 값 변경 = 마커 해제.
        Entry {
            cat: Msg::CatUser,
            sub: None,
            label: Msg::UserTest,
            desc: Msg::UserTestDesc,
            kind: SettingKind::Action {
                verb: Msg::UserTestVerb,
            },
            key: "user.test",
        },
        // 사용자 키 교체(ADR-0015 §3-5 · S2-f) — 기기 분실 대응. 인증 상태에서만 활성(호스트 잠금) ·
        // 2회 클릭(5초 무장)으로 실행.
        // 내 기기 목록(10-09) — 행위 항목: 창을 연다(호스트 `open_devices` · 기기별 폐기 = Succession 부분 집합).
        Entry {
            cat: Msg::CatUser,
            sub: None,
            label: Msg::UserDevices,
            desc: Msg::UserDevicesDesc,
            kind: SettingKind::Action {
                verb: Msg::UserDevicesVerb,
            },
            key: "user.devices",
        },
        Entry {
            cat: Msg::CatUser,
            sub: None,
            label: Msg::UserRotate,
            desc: Msg::UserRotateDesc,
            kind: SettingKind::Action {
                verb: Msg::UserRotateVerb,
            },
            key: "user.rotate",
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
        // ── 고급 — **가장 밑 배치**(08-15 사용자 확정 · 첫 등장 순서 = 사이드바 순서라
        //    close_to_tray를 여기로 내렸다) · 스위치 먼저, 행위(백업·복원·초기화) 마지막 ──
        // 시스템 시작 시 자동 실행(08-20 사용자 확정 — **기본 on** · 끄기는 옵트아웃).
        // OS별 사용자 수준 등록(T0 무권한)은 nbeep-plat::autostart, 부팅 재동기화는
        // apply_boot_settings(포터블 경로 이동 대응 · DR-4).
        Entry {
            cat: Msg::CatSystem,
            sub: None,
            label: Msg::AutoStart,
            desc: Msg::AutoStartDesc,
            kind: SettingKind::Toggle,
            key: "app.autostart",
        },
        Entry {
            cat: Msg::CatSystem,
            sub: None,
            label: Msg::CloseToTray,
            desc: Msg::CloseToTrayDesc,
            kind: SettingKind::Toggle,
            key: "ui.close_to_tray",
        },
        // 08-30 사용자 확정 — 3-OS 공통 · **기본 on(숨김)**. Linux = 창 파괴(Wayland는
        // 숨김 불가) · Windows = 숨김 · mac = 숨김 + Dock 아이콘 제거(Accessory 정책).
        Entry {
            cat: Msg::CatSystem,
            sub: None,
            label: Msg::TrayHideTaskbar,
            desc: Msg::TrayHideTaskbarDesc,
            kind: SettingKind::Toggle,
            key: "ui.tray_hide_taskbar",
        },
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
        // ── 고급 › 로그(M3-22 · 08-18 — `log.enabled` 기본 off 사용자 확정) ──
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLog),
            label: Msg::LogEnabled,
            desc: Msg::LogEnabledDesc,
            kind: SettingKind::Toggle,
            key: "log.enabled",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLog),
            label: Msg::LogRetain,
            desc: Msg::LogRetainDesc,
            kind: SettingKind::RadioInput(&[("7", Msg::LogRetainDefault)], ""),
            key: "log.retain_days",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLog),
            label: Msg::LogMaxTotal,
            desc: Msg::LogMaxTotalDesc,
            kind: SettingKind::RadioInput(&[("20", Msg::LogCapDefault)], ""),
            key: "log.max_total_mb",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLog),
            label: Msg::LogView,
            desc: Msg::LogViewDesc,
            kind: SettingKind::Action { verb: Msg::ActOpen },
            key: "log.view",
        },
        // ── 고급 › 네트워크 점검(netmon · 08-21 — 옵트인 계측 기록 · 기본 off) ──
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubNetmon),
            label: Msg::NetmonEnabled,
            desc: Msg::NetmonEnabledDesc,
            kind: SettingKind::Toggle,
            key: "netmon.enabled",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubNetmon),
            label: Msg::NetmonInterval,
            desc: Msg::NetmonIntervalDesc,
            kind: SettingKind::RadioInput(&[("10", Msg::NetmonIntervalDefault)], ""),
            key: "netmon.interval_s",
        },
        // ── 라이선스 정보(P4 ⑤ · 읽기 전용 · 호스트 set_info — 값은 영속되지 않는다) ──
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLicense),
            label: Msg::LicInfoState,
            desc: Msg::LicInfoStateDesc,
            kind: SettingKind::Info,
            key: "license.state",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLicense),
            label: Msg::LicInfoId,
            desc: Msg::LicInfoIdDesc,
            kind: SettingKind::Info,
            key: "license.id",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLicense),
            label: Msg::LicInfoFile,
            desc: Msg::LicInfoFileDesc,
            kind: SettingKind::Info,
            key: "license.file",
        },
        Entry {
            cat: Msg::CatAdvanced,
            sub: Some(Msg::SubLicense),
            label: Msg::LicInfoMachine,
            desc: Msg::LicInfoMachineDesc,
            kind: SettingKind::Info,
            key: "license.machine",
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
            // 자유 텍스트(핸들·암호) — 빈 값도 유효(미설정 상태 그 자체). 형식 검증은 앱(userident).
            SettingKind::Text { .. } => true,
            SettingKind::Toggle => value == "on" || value == "off",
            SettingKind::Color { .. } => crate::theme::color_from_hex(value).is_some(),
            // 위치 코드·글꼴명(빈 값 = 시스템 기본)·크기 코드는 소비처가 관용 파싱한다.
            SettingKind::PositionGrid | SettingKind::FontFace { .. } => true,
            SettingKind::FontSection { .. } => true,
            // 행위·정보 항목은 값이 없다 — 파일에서 와도 무시(default_values가 비어 도달 불가).
            SettingKind::Action { .. } | SettingKind::Info => false,
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

/// 검색 매칭(10-09 개정 · D-33-1): 공백 토큰 **AND** · 대상 = 키 이름 + 그룹·카테고리·하위·제목·설명(전 언어) ·
/// 한글 토큰은 **자모열 대조**(조합 중 "ㅌ"·"테"도 "테마"에 맞는다 · [`crate::jamo`]).
fn entry_matches(e: &Entry, toks: &[String]) -> bool {
    if toks.is_empty() {
        return true;
    }
    let mut hay = String::new();
    hay.push_str(e.key);
    hay.push(' ');
    // 전 언어(종전 동작 유지) — 영어 UI에서도 "테마"로, 한국어 UI에서도 "theme"로 찾힌다.
    for lang in Lang::ALL {
        if let Some(g) = group_of(e.cat) {
            hay.push_str(tr(lang, g));
            hay.push(' ');
        }
        hay.push_str(tr(lang, e.cat));
        hay.push(' ');
        if let Some(sub) = e.sub {
            hay.push_str(tr(lang, sub));
            hay.push(' ');
        }
        hay.push_str(tr(lang, e.label));
        hay.push(' ');
        hay.push_str(tr(lang, e.desc));
        hay.push(' ');
    }
    let hay = hay.to_lowercase();
    toks.iter().all(|t| {
        if crate::jamo::has_hangul(t) {
            crate::jamo::contains_jamo(&hay, &crate::jamo::decompose(t, true), true)
        } else {
            hay.contains(t.as_str())
        }
    })
}

// 레이아웃(논리 px).
const SIDEBAR_W: i32 = 170;
/// 하단 줄(고급 스위치 · 설정 파일 열기 · 닫기 — 10-09 nexa-sql 차용) 높이.
const BOTTOM_H: i32 = 44;
/// 고급 숨김 배너(밴드 셋째 줄) 높이.
const BANNER_H: i32 = 22;
/// 카드 \[초기화\] 버튼 폭.
const RESET_W: i32 = 72;
/// 검색 이력 보관 수.
const HISTORY_MAX: usize = 20;
const SEARCH_H: i32 = 30;
/// 카드(10-09 사용자 확정 — nexa-sql 설정 창 모양): 제목 줄 높이 · 안쪽 여백 · 카드 간격 · 자유 텍스트 상자 폭.
const TITLE_H: i32 = 20;
const CARD_PAD: i32 = 12;
const CARD_GAP: i32 = 8;
const TEXT_W: i32 = 260;
/// 위치 드롭다운 폭(nexa-sql 64).
const POS_W: i32 = 64;
/// 설정 행에 붙는 정보 줄 높이(논리 px).
const NOTE_H: i32 = 22;
/// 설명 워드랩 줄 높이(논리 px — Status 폰트 한 줄 + 행간).
const DESC_LINE_H: i32 = 16;
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
#[allow(clippy::large_enum_variant)] // 행마다 1개 · nexa-ctl TextBox가 커졌다(10-09) — Box는 접근 비용만 더한다
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
    /// 3×3 위치 드롭다운(10-09 · 종전 인라인 PositionPicker).
    Pos(PositionDropdown),
    /// 글꼴 **얼굴만**(고정폭 — 크기는 Base UI를 따른다).
    Face(TextBox),
    /// 색상(스와치 + hex + 프리셋 · 08-10).
    Color(ColorPicker),
    /// 읽기 전용 정보(10-09 · 호스트가 채운 글).
    Info(String),
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
    /// [초기화](10-09 · 값이 기본값과 다를 때만 보인다 · 값 키가 없는 행위/정보 행은 `None`).
    reset: Option<Button>,
    /// 키 이름+복사 글리프 자리(페인트가 실측해 채운다 · 클릭 = 키 복사 요청).
    key_rect: std::cell::Cell<Rect>,
}

/// 사이드바 트리 선택(10-09 그룹 트리) — 그룹 행 = 그 그룹의 카테고리 전부 · 카테고리 행 = 그것만.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeSel {
    /// `CATEGORY_TREE` 그룹 인덱스.
    Group(usize),
    /// `SettingsWidget::cats` 인덱스(트리 순서의 카테고리).
    Cat(usize),
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
    /// 사이드바 가시 행 → 트리 선택(그룹/카테고리 · 트리 행 순서와 같다).
    cat_map: Vec<TreeSel>,
    /// 현재 선택.
    selected: TreeSel,
    /// 고급 설정 스위치 상태(`ui.prefs_advanced`) — 끄면 [`ADVANCED`] 항목은 숨기고 수만 센다.
    advanced: bool,
    /// 지금 보기에서 숨긴 고급 항목 수(밴드 배너).
    adv_hidden: usize,
    /// 하단 줄 컨트롤 — 고급 스위치 · \[설정 파일 열기…\] · \[닫기\].
    adv_switch: Switch,
    btn_file: Button,
    btn_close: Button,
    /// 읽기 전용 정보 행 본문(`SettingKind::Info` — 호스트 `set_info`).
    infos: HashMap<&'static str, String>,
    /// 콤보 옵션 **표시 라벨 덮어쓰기**((키, 값) → 라벨 · 10-09 사용자 "시스템 옆에 (설정값)" — nexa-dir3
    /// `system_state_labels` 선례): `ui.theme`/`ui.language`의 `system` 항목을 "시스템 (다크)"·"시스템 (한국어)"처럼
    /// **지금 OS가 무엇으로 풀리는지**와 함께 보인다. 판정은 호스트(`set_option_label`) · 위젯은 OS를 모른다.
    option_labels: HashMap<(&'static str, &'static str), String>,
    /// 검색 이력(최근이 앞 · `prefs.search` 탭 구분 · 최대 [`HISTORY_MAX`]) · ↑/↓ 탐색 위치.
    history: Vec<String>,
    hist_pos: Option<usize>,
    /// 우측 행들(가시 항목 + 컨트롤).
    rows: Vec<RowUi>,
    /// 현재 값 스냅숏(컨트롤 초기화·보고 근거).
    values: HashMap<&'static str, String>,
    changes: Vec<(&'static str, String)>,
    /// 검증 실패 경고(08-20 — 확정 즉시 · 호스트가 모달/상태줄로 표출).
    warnings: Vec<Msg>,
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
    notes: HashMap<&'static str, (String, NoteTone)>,
    /// 암호 눈 아이콘 틴트 캐시(색 키).
    pw_eye: std::cell::RefCell<Option<(u32, crate::theme::IconImage)>>,
    /// 암호 생성 아이콘 틴트 캐시(색 키).
    pw_regen: std::cell::RefCell<Option<(u32, crate::theme::IconImage)>>,
    /// 생성 2단 확인 — 첫 클릭 = 무장(빨강) · 2초 안 재클릭 = 생성 · 지나면 원복.
    pw_arm: Option<std::time::Instant>,
}

/// 행 노트의 시각 톤(08-22 — "검증됨"이 눈에 띄어야 한다는 사용자 요청).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteTone {
    /// 일반 정보(종전 동작 — 흐린 글자·배경 없음).
    Plain,
    /// 긍정 상태(검증 완료 등) — ok색 옅은 배경 + ok색 글자.
    Ok,
    /// 경고 상태 — warn색 옅은 배경 + warn색 글자.
    Warn,
    /// 자동 판정 정보(08-22 — 서버 타입 판정·관측값처럼 **시스템이 정한 값**을
    /// 눈에 띄게) — accent색 옅은 배경 + 본문색 글자.
    Info,
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
        let lang = current_lang();
        let advanced = state.get("ui.prefs_advanced") == "on";
        let history: Vec<String> = state
            .get("prefs.search")
            .split('\t')
            .filter(|h| !h.is_empty())
            .take(HISTORY_MAX)
            .map(str::to_string)
            .collect();
        let mut w = Self {
            bounds: Rect::default(),
            scale: 1.0,
            search: TextBox::new("Search").with_clearable(),
            query: String::new(),
            default_base_name: String::new(),
            default_mono_name: String::new(),
            tree: TreeView::new(TreeModel::default()),
            cat_map: Vec::new(),
            selected: TreeSel::Cat(0),
            advanced,
            adv_hidden: 0,
            adv_switch: Switch::new(tr(lang, Msg::PrefsAdvanced), advanced),
            btn_file: Button::new(tr(lang, Msg::BtnOpenSettingsFile)),
            btn_close: Button::new(tr(lang, Msg::BtnClose)),
            infos: HashMap::new(),
            option_labels: HashMap::new(),
            history,
            hist_pos: None,
            rows: Vec::new(),
            values,
            changes: Vec::new(),
            warnings: Vec::new(),
            back: false,
            scroll: 0,
            content_h: 0,
            bars: ScrollBars::new(),
            sidebar_w: SIDEBAR_W,
            split_drag: false,
            disabled: std::collections::HashSet::new(),
            notes: HashMap::new(),
            pw_eye: std::cell::RefCell::new(None),
            pw_regen: std::cell::RefCell::new(None),
            pw_arm: None,
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
        // 콤보 직접 입력(08-22 — TextBox 위임)도 같은 텍스트 입력이다.
        self.rows.iter().find_map(|r| match &r.ctl {
            RowCtl::Font { family, size } => family
                .copy_selection()
                .or_else(|| size.editing_input_ref().and_then(TextBox::copy_selection)),
            RowCtl::Face(family) => family.copy_selection(),
            RowCtl::Combo(c) => c.editing_input_ref().and_then(TextBox::copy_selection),
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
            RowCtl::Font { family, size } => family
                .cut_selection(inv)
                .or_else(|| size.editing_input().and_then(|tb| tb.cut_selection(inv))),
            RowCtl::Face(family) => family.cut_selection(inv),
            RowCtl::Combo(c) => c.editing_input().and_then(|tb| tb.cut_selection(inv)),
            _ => None,
        })
    }

    /// 붙여넣기(①) — 포커스된 텍스트 입력만 받는다.
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        self.search.paste(text, inv);
        self.sync_query(inv);
        for r in &mut self.rows {
            match &mut r.ctl {
                RowCtl::Font { family, size } => {
                    family.paste(text, inv);
                    if let Some(tb) = size.editing_input() {
                        tb.paste(text, inv);
                    }
                }
                RowCtl::Face(family) => family.paste(text, inv),
                RowCtl::Combo(c) => {
                    if let Some(tb) = c.editing_input() {
                        tb.paste(text, inv);
                    }
                }
                _ => {}
            }
        }
    }

    /// 우클릭 편집 메뉴 행동(1회성 — 08-13 전수 검사) — 어느 텍스트 입력에서든.
    pub fn take_edit_ctx(&mut self) -> Option<crate::controls::EditCtxAction> {
        if let Some(a) = self.search.take_edit_ctx() {
            return Some(a);
        }
        self.rows.iter_mut().find_map(|r| match &mut r.ctl {
            RowCtl::Font { family, size } => family
                .take_edit_ctx()
                .or_else(|| size.editing_input().and_then(|tb| tb.take_edit_ctx())),
            RowCtl::Face(family) => family.take_edit_ctx(),
            RowCtl::Combo(c) => c.editing_input().and_then(|tb| tb.take_edit_ctx()),
            _ => None,
        })
    }

    /// 클립보드 텍스트 유무 주입(우클릭 시점 — 붙여넣기 항목 활성 근거).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.search.set_clipboard_has_text(yes);
        for r in &mut self.rows {
            match &mut r.ctl {
                RowCtl::Font { family, size } => {
                    family.set_clipboard_has_text(yes);
                    if let Some(tb) = size.editing_input() {
                        tb.set_clipboard_has_text(yes);
                    }
                }
                RowCtl::Face(family) => family.set_clipboard_has_text(yes),
                RowCtl::Combo(c) => {
                    if let Some(tb) = c.editing_input() {
                        tb.set_clipboard_has_text(yes);
                    }
                }
                _ => {}
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
    /// 지정 카테고리로 직행(08-22 — 툴바 서버 표시 클릭 = 서버 설정 바로가기).
    /// 검색은 지우고 스크롤은 맨 위로. 미지 카테고리는 무시.
    pub fn select_category(&mut self, cat: Msg, inv: &mut Invalidations) {
        if let Some(ci) = Self::cats().iter().position(|(c, _)| *c == cat) {
            self.selected = TreeSel::Cat(ci);
            self.query.clear();
            self.search.set_text("");
            self.scroll = 0;
            self.rebuild(inv);
        }
    }

    /// 고급 설정 스위치 상태를 외부에서 맞춘다(열 때 `ui.prefs_advanced` 복원은 `new`가 한다 — 런타임 동기용).
    pub fn set_advanced(&mut self, on: bool, inv: &mut Invalidations) {
        if self.advanced != on {
            self.advanced = on;
            self.adv_switch.set_on(on);
            self.rebuild(inv);
        }
    }

    /// 읽기 전용 정보 행 본문 지정(`SettingKind::Info` · 빈 문자열 = 비움) — 바뀔 때만 다시 그린다.
    pub fn set_info(&mut self, key: &'static str, text: &str, inv: &mut Invalidations) {
        if self.infos.get(key).map(String::as_str) == Some(text) {
            return;
        }
        self.infos.insert(key, text.to_string());
        for row in &mut self.rows {
            if registry()[row.idx].key == key {
                if let RowCtl::Info(t) = &mut row.ctl {
                    *t = text.to_string();
                }
            }
        }
        inv.push(self.bounds);
    }

    /// 콤보 옵션 하나의 표시 라벨을 바꾼다(호스트 — OS 판정값 동반 표기 · 10-09). 같은 값이면 no-op ·
    /// 바뀌면 보이는 행을 다시 짓는다(콤보 라벨은 생성 시 고정 — 선택값은 `values`가 지켜 유지된다).
    pub fn set_option_label(
        &mut self,
        key: &'static str,
        value: &'static str,
        text: &str,
        inv: &mut Invalidations,
    ) {
        if self.option_labels.get(&(key, value)).map(String::as_str) == Some(text) {
            return;
        }
        self.option_labels.insert((key, value), text.to_string());
        if self.rows.iter().any(|r| registry()[r.idx].key == key) {
            self.rebuild(inv);
        }
    }

    /// 현재 선택(시험·호스트 진단용).
    #[must_use]
    pub fn selected(&self) -> TreeSel {
        self.selected
    }

    /// IME 조합 중 텍스트(10-09 사용자 실기 "검색에 ㅌ가 안 보인다" — nexa-sql은 보인다): 포커스된 글꼴명
    /// 입력란이 있으면 거기로, 아니면 **검색창**으로(기본 타이핑 = 검색). 검색은 조합 중 글자까지 포함해
    /// **즉시 필터링**(자모열 대조라 "ㅌ"도 테마를 찾는다).
    pub fn set_preedit(&mut self, text: &str, inv: &mut Invalidations) {
        if self.any_family_focused() {
            for row in &mut self.rows {
                match &mut row.ctl {
                    RowCtl::Font { family, .. } if family.is_focused() => {
                        family.set_preedit(text, inv);
                    }
                    RowCtl::Face(f) if f.is_focused() => f.set_preedit(text, inv),
                    _ => {}
                }
            }
            inv.push(self.bounds);
            return;
        }
        if !text.is_empty() {
            self.search.set_focused(true);
        }
        self.search.set_preedit(text, inv);
        let q = self.search.display_text();
        if q != self.query {
            self.query = q;
            self.rebuild(inv);
        }
        inv.push(self.bounds);
    }

    pub fn take_changes(&mut self) -> Vec<(&'static str, String)> {
        std::mem::take(&mut self.changes)
    }

    /// 검증 실패 경고를 꺼낸다(1회성 — 확정 시 즉시 발생 · 원복은 이미 끝난 뒤).
    pub fn take_warnings(&mut self) -> Vec<Msg> {
        std::mem::take(&mut self.warnings)
    }

    /// Esc 닫기 요청(1회성).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 카테고리 목록 — **트리 순서**(`CATEGORY_TREE`)로, 레지스트리에 항목이 있는 것만: (카테고리, 그룹 인덱스).
    fn cats() -> Vec<(Msg, usize)> {
        let mut out = Vec::new();
        for (gi, (_, cats)) in CATEGORY_TREE.iter().enumerate() {
            for &c in *cats {
                if registry().iter().any(|e| e.cat == c) {
                    out.push((c, gi));
                }
            }
        }
        out
    }

    /// 이 행이 기본값과 다른가(값 키가 하나라도 다르면 · 값 키 없는 행 = 거짓).
    fn is_modified(&self, idx: usize) -> bool {
        registry()[idx]
            .default_values()
            .iter()
            .any(|(k, d)| self.values.get(k).map(String::as_str) != Some(d.as_str()))
    }

    /// 종속 잠금(DEPENDS) — 부모 값이 조건을 만족하지 않으면 `Some(부모 키, 조건)`.
    fn dep_lock(&self, idx: usize) -> Option<(&'static str, Dep)> {
        let (parent, dep) = depends_of(registry()[idx].key)?;
        let pv = self.values.get(parent).map_or("", String::as_str);
        (!dep.satisfied(pv)).then_some((parent, dep))
    }

    /// 행 아래 한 줄 — 호스트 노트가 우선 · 없으면 종속 잠금 안내.
    fn row_note(&self, idx: usize) -> Option<(String, NoteTone)> {
        let key = registry()[idx].key;
        if let Some((t, tone)) = self.notes.get(key) {
            return Some((t.clone(), *tone));
        }
        let (parent, dep) = self.dep_lock(idx)?;
        let lang = current_lang();
        let plabel = registry()
            .iter()
            .find(|e| e.key == parent)
            .map_or(parent, |e| tr(lang, e.label));
        let text = match dep {
            Dep::On => nbeep_core::tf(Msg::PrefsLockedBy, &[plabel]),
            Dep::Eq(v) => {
                // 부모 옵션의 표시 라벨(없으면 값 그대로).
                let vlabel = registry()
                    .iter()
                    .find(|e| e.key == parent)
                    .and_then(|e| match e.kind {
                        SettingKind::Radio(opts) | SettingKind::RadioInput(opts, _) => opts
                            .iter()
                            .find(|(o, _)| *o == v)
                            .map(|(_, m)| tr(lang, *m)),
                        _ => None,
                    })
                    .unwrap_or(v);
                nbeep_core::tf(Msg::PrefsLockedByValue, &[plabel, vlabel])
            }
        };
        Some((text, NoteTone::Plain))
    }

    /// 카테고리 매치 수(고급 숨김 반영 — 사이드바 "(N)"은 실제로 보일 수와 같아야 한다).
    fn cat_match_count(&self, cat: Msg, toks: &[String]) -> usize {
        registry()
            .iter()
            .filter(|e| {
                e.cat == cat && (self.advanced || !is_advanced(e.key)) && entry_matches(e, toks)
            })
            .count()
    }

    /// 고급 필터 **전** 후보(검색 또는 선택 범위) — 숨긴 수를 세는 기준.
    fn candidate_indices(&self) -> Vec<usize> {
        let toks = tokens(&self.query);
        let searching = !toks.is_empty();
        let cats = Self::cats();
        let allowed: Vec<Msg> = match self.selected {
            TreeSel::Group(gi) => CATEGORY_TREE
                .get(gi)
                .map_or(Vec::new(), |(_, c)| c.to_vec()),
            TreeSel::Cat(ci) => cats.get(ci).map(|(c, _)| *c).into_iter().collect(),
        };
        registry()
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if searching {
                    entry_matches(e, &toks)
                } else {
                    allowed.contains(&e.cat)
                }
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 가시 항목(registry 인덱스) — 검색 중 = 전 카테고리 매치 · 아니면 선택 범위(그룹/카테고리) ·
    /// 고급 스위치가 꺼져 있으면 [`ADVANCED`] 제외 · 순서 = [`display_order`](그룹 → 카테고리 → 하위 → 접두 → 등재).
    fn visible_indices(&self) -> Vec<usize> {
        let mut hits: Vec<usize> = self
            .candidate_indices()
            .into_iter()
            .filter(|&i| self.advanced || !is_advanced(registry()[i].key))
            .collect();
        hits.sort_by_key(|&i| display_order(i));
        hits
    }

    /// 사이드바·우측 행(컨트롤 포함)을 현재 상태(검색·선택·값)로 다시 만든다.
    fn rebuild(&mut self, inv: &mut Invalidations) {
        let lang = current_lang();
        let toks = tokens(&self.query);
        let searching = !toks.is_empty();

        // ── 사이드바 트리(그룹 → 카테고리 · 10-09 · 검색 중엔 매치만 + "(N)") ──
        let cats = Self::cats();
        self.cat_map.clear();
        let mut roots = Vec::new();
        for (gi, (g, _)) in CATEGORY_TREE.iter().enumerate() {
            let mut children = Vec::new();
            let mut sels = Vec::new();
            let mut gn = 0usize;
            for (ci, (c, cgi)) in cats.iter().enumerate() {
                if *cgi != gi {
                    continue;
                }
                let n = self.cat_match_count(*c, &toks);
                if searching && n == 0 {
                    continue;
                }
                gn += n;
                let label = if searching {
                    format!("{} ({n})", tr(lang, *c))
                } else {
                    tr(lang, *c).to_string()
                };
                children.push(TreeNode::leaf(label));
                sels.push(TreeSel::Cat(ci));
            }
            if children.is_empty() {
                continue;
            }
            let glabel = if searching {
                format!("{} ({gn})", tr(lang, *g))
            } else {
                tr(lang, *g).to_string()
            };
            self.cat_map.push(TreeSel::Group(gi));
            self.cat_map.extend(sels);
            roots.push(TreeNode::branch(glabel, children)); // 기본 펼침
        }
        let mut tree = TreeView::new(TreeModel::new(roots));
        tree.set_scale(self.scale);
        tree.set_focused(true); // 사이드바는 ↑↓ 상시 탐색(트리 자체 포커스 링 없음)
        let sel_row = self
            .cat_map
            .iter()
            .position(|&sel| sel == self.selected)
            .unwrap_or(0);
        tree.set_selected_row(sel_row);
        self.tree = tree;
        let candidates = self.candidate_indices().len();

        // ── 우측 행 + 컨트롤 ──
        self.rows.clear();
        let visible = self.visible_indices();
        self.adv_hidden = candidates.saturating_sub(visible.len());
        // 여러 카테고리가 한 목록에 섞이는 보기(그룹 선택·검색)는 카테고리 경계에 제목을 붙인다.
        let multi_cat = searching || matches!(self.selected, TreeSel::Group(_));
        for idx in visible {
            let e = &registry()[idx];
            let ctl = match e.kind {
                SettingKind::Info => {
                    RowCtl::Info(self.infos.get(e.key).cloned().unwrap_or_default())
                }
                SettingKind::Radio(opts) | SettingKind::RadioInput(opts, _) => {
                    let items: Vec<ComboItem> = opts
                        .iter()
                        .map(|(v, m)| {
                            let label = self
                                .option_labels
                                .get(&(e.key, *v))
                                .map_or_else(|| tr(lang, *m).to_string(), Clone::clone);
                            ComboItem::new(*v, label)
                        })
                        .collect();
                    let mut c = Combo::new(items, 0);
                    if let SettingKind::RadioInput(_, suffix) = e.kind {
                        c.set_custom_entry(tr(lang, Msg::CustomInput), suffix);
                        // 텍스트 값 행(서버 주소 — 도메인·IP)은 숫자 필터를 푼다(08-22).
                        c.set_custom_text(FREE_TEXT_KEYS.contains(&e.key));
                    }
                    let cur = self.values.get(e.key).map_or("", String::as_str);
                    c.select_value(cur);
                    c.note_value(cur); // 직전 확정값 시드(08-20 — 검증 원복 기준점)
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
                SettingKind::Text { hint, secret } => {
                    let mut t = TextBox::new(tr(lang, hint))
                        .with_text(self.values.get(e.key).map_or("", String::as_str));
                    t.set_scale(self.scale);
                    t.set_masked(secret); // 기본 가림 — 눈 버튼으로 본다.
                    RowCtl::Face(t)
                }
                SettingKind::PositionGrid => {
                    let mut p =
                        PositionDropdown::new(self.values.get(e.key).map_or("bl", String::as_str));
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
                    let items: Vec<ComboItem> = FONT_SIZE_OPTS
                        .iter()
                        .map(|(v, m)| ComboItem::new(*v, tr(lang, *m)))
                        .collect();
                    let mut size = Combo::new(items, 0);
                    // 숫자 직접 입력(08-18 사용자 요청) — 프리셋도 절대 px 값이라
                    // 같은 축이다(해석·클램프는 소비 측 fonts_from_settings).
                    size.set_custom_entry(tr(lang, Msg::CustomInput), "px");
                    size.select_value(
                        self.values
                            .get(size_key)
                            .map_or(FONT_SIZE_DEFAULT, String::as_str),
                    );
                    size.set_scale(self.scale);
                    RowCtl::Font { family, size }
                }
            };
            // 제목 규칙: 카테고리가 바뀌면(여러 카테고리 보기) 카테고리 제목 · 같은 카테고리 안에서 하위 섹션이
            // 바뀌면 하위 제목 · 직속 구간은 없음(상위 제목은 고정 밴드 몫).
            let group = (e.cat, e.sub);
            let head = match self.rows.last().map(|r| r.group) {
                Some((pc, ps)) if pc == e.cat => e.sub.filter(|_| ps != e.sub),
                _ if multi_cat => Some(e.cat),
                _ => e.sub,
            };
            // [초기화] — 값 키가 있는 행만(행위·정보 행은 없다). 보이기는 layout이 `is_modified`로 정한다.
            let reset = (!e.default_values().is_empty()).then(|| {
                let mut b = Button::new(tr(lang, Msg::BtnReset));
                b.set_scale(self.scale);
                b
            });
            self.rows.push(RowUi {
                idx,
                rect: Rect::default(),
                ctl,
                group,
                head,
                head_h: 0,
                desc_lines: 1,
                desc_avail: 0,
                reset,
                key_rect: std::cell::Cell::new(Rect::default()),
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
            match &mut row.ctl {
                RowCtl::Combo(c) => c.select_value(value),
                // 토글도 역반영(08-15 — 쌍방 동기화: 다른 경로가 켠/끈 것을 표시).
                RowCtl::Check(c) => c.set_on(value == "on"),
                // 자유 텍스트 행(암호 생성 등 프로그램 변경)도 역반영.
                RowCtl::Face(t) if matches!(registry()[row.idx].kind, SettingKind::Text { .. }) => {
                    t.set_text(value);
                }
                RowCtl::Pos(g) => g.select_value(value),
                _ => {}
            }
        }
        inv.push(self.bounds);
    }

    /// 비활성 키 지정 — 조건부로만 쓰이는 설정을 흐리게 잠근다(예: 기간은 "기간 자동"일 때만).
    /// 생성 무장 남은 시간(ms · 무장 아님 = None) — 호스트가 행 노트 카운트다운을 만든다(09-06).
    #[must_use]
    pub fn pw_arm_remaining_ms(&self) -> Option<u64> {
        let t = self.pw_arm?;
        let left = PW_ARM_WINDOW.checked_sub(t.elapsed())?;
        Some(u64::try_from(left.as_millis()).unwrap_or(u64::MAX))
    }

    /// 행위 버튼 색조 지정(09-06 — 무장 중 빨강). 그 키가 행위 항목이 아니면 no-op.
    pub fn set_action_tone(
        &mut self,
        key: &'static str,
        tone: crate::controls::ButtonTone,
        inv: &mut Invalidations,
    ) {
        for r in &mut self.rows {
            if registry()[r.idx].key != key {
                continue;
            }
            if let RowCtl::Act(b) = &mut r.ctl {
                // nexa-ctl `set_tone`은 반환값이 없다(10-09 이관) — 변경 여부는 여기서 비교.
                if b.tone() != tone {
                    b.set_tone(tone);
                    inv.push(self.bounds);
                }
            }
        }
    }

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
        self.set_row_note_toned(key, text, NoteTone::Plain, inv);
    }

    /// 톤 있는 행 노트(08-22) — Ok/Warn은 옅은 배경으로 눈에 띈다(검증 상태 표시).
    pub fn set_row_note_toned(
        &mut self,
        key: &'static str,
        text: &str,
        tone: NoteTone,
        inv: &mut Invalidations,
    ) {
        let had = self.notes.contains_key(key);
        if self.notes.get(key).map(|(t, tn)| (t.as_str(), *tn)) == Some((text, tone))
            || (text.is_empty() && !had)
        {
            return;
        }
        if text.is_empty() {
            self.notes.remove(key);
        } else {
            self.notes.insert(key, (text.to_string(), tone));
        }
        if had != self.notes.contains_key(key) {
            self.layout(inv); // 줄이 생기거나 사라지면 행 높이가 달라진다
        }
        inv.push(self.bounds);
    }

    /// 이 행에 붙은 정보 줄 높이(없으면 0) — 노트 아래 **여백 8**을 포함해
    /// 다음 행과 시각 구분한다(08-23 사용자 확정 — 검증 노트와 다음 설정이 붙어
    /// 보였다).
    fn note_h(&self, idx: usize) -> i32 {
        if self.row_note(idx).is_some() {
            self.s(NOTE_H + 6) // 컨트롤 줄 아래 간격 6 + 노트(카드 안)
        } else {
            0
        }
    }

    /// 이 행이 잠겼는가 — 호스트 런타임 잠금 ∪ 종속(DEPENDS) 불충족.
    fn is_locked(&self, idx: usize) -> bool {
        self.disabled.contains(registry()[idx].key) || self.dep_lock(idx).is_some()
    }

    /// 하단 줄 높이(물리 px).
    fn bottom_h(&self) -> i32 {
        self.s(BOTTOM_H)
    }

    /// 하단 줄 영역.
    fn bottom_rect(&self) -> Rect {
        let b = self.bounds;
        let h = self.bottom_h();
        Rect::new(b.x, b.bottom() - h, b.w, h)
    }

    /// 상단 고정 밴드(상위 + 하위 제목) 높이 — 하위가 없어도 **줄어들지 않는다**.
    /// 그룹 경계를 넘을 때 아래 내용이 위아래로 튀면 읽던 자리를 잃는다.
    fn crumb_h(&self) -> i32 {
        self.s(CRUMB_CAT_H)
            + self.s(CRUMB_SUB_H)
            + if self.adv_hidden > 0 {
                self.s(BANNER_H)
            } else {
                0
            }
    }

    /// 우측 패널 뷰포트(사이드바 제외 · **고정 밴드 아래**부터).
    fn right_viewport(&self) -> Rect {
        let sw = self.s(self.sidebar_w);
        let b = self.bounds;
        let top = b.y + self.crumb_h();
        let bottom = b.bottom() - self.bottom_h();
        Rect::new(b.x + sw, top, (b.w - sw).max(0), (bottom - top).max(0))
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
        let mut dirty = self.bars.tick(now_ms) | self.tree.tick(now_ms);
        // ★ 생성 무장 타이머 — 무장 중엔 계속 깨워 만료를 제때 잡는다(≤ 5초).
        if let Some(t) = self.pw_arm {
            if t.elapsed() > PW_ARM_WINDOW {
                self.pw_arm = None;
            }
            dirty = true;
        }
        dirty
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
        let bottom_top = b.bottom() - self.bottom_h();
        self.tree.set_bounds(
            Rect::new(b.x, tree_top, sw, (bottom_top - tree_top).max(0)),
            inv,
        );
        // ── 하단 줄(10-09): [고급 스위치] ………… [설정 파일 열기…][닫기] ──
        {
            let ctl_h = self.s(CTL_H);
            let pad = self.s(PAD);
            let cy = bottom_top + (self.bottom_h() - ctl_h) / 2;
            self.adv_switch.set_scale(self.scale);
            self.adv_switch
                .set_bounds(Rect::new(b.x + pad, cy, self.s(190), ctl_h), inv);
            let close_w = self.s(90);
            let file_w = self.s(150);
            self.btn_close.set_scale(self.scale);
            self.btn_close.set_bounds(
                Rect::new(b.right() - pad - close_w, cy, close_w, ctl_h),
                inv,
            );
            self.btn_file.set_scale(self.scale);
            self.btn_file.set_bounds(
                Rect::new(
                    b.right() - pad - close_w - self.s(8) - file_w,
                    cy,
                    file_w,
                    ctl_h,
                ),
                inv,
            );
        }

        let rx = b.x + sw; // 우측 패널 시작
        let rw = (b.w - sw).max(0);
        // ── 카드 레이아웃(10-09 사용자 확정 · nexa-sql 모양) ──
        //   ┌ 제목 ……………………………… 키 이름 ⧉ ┐
        //   │ 설명(워드랩 1~3줄)                      │
        //   │ [컨트롤] [초기화]           기본값: … │
        //   └ (노트 — 호스트/종속 잠금)              ┘
        let (ctl_h, pad, cpad, gap) = (
            self.s(CTL_H),
            self.s(PAD),
            self.s(CARD_PAD),
            self.s(CARD_GAP),
        );
        let (combo_w, check_w) = (self.s(COMBO_W), self.s(crate::controls::ctl_size(20)));
        let (family_w, size_w, gap10, text_w) =
            (self.s(FAMILY_W), self.s(SIZE_W), self.s(10), self.s(TEXT_W));
        let lang = current_lang();
        let scale = self.scale;
        let desc_line_h = self.s(DESC_LINE_H);
        let title_h = self.s(TITLE_H);
        let head_h = self.s(SUB_HEAD_H);
        let reset_w = self.s(RESET_W);
        let card_x = rx + pad;
        let card_w = (rw - pad * 2).max(self.s(120));
        let desc_avail = (card_w - cpad * 2).max(self.s(60));
        let desc_gap = self.s(2);
        let ctl_gap = self.s(8);
        // 행별 치수를 **한 번** 계산해 총높이와 배치가 같은 값을 쓴다(08-15 상한 불일치 재발 방지).
        struct Metric {
            desc_lines: i32,
            ctl_block: i32,
            h: i32,
        }
        let metrics: Vec<Metric> = self
            .rows
            .iter()
            .map(|row| {
                let e = &registry()[row.idx];
                // 설명 줄 수 추정(ASCII 7·그 외 14 논리px — 실측은 페인트가 하고 여기는 **예약**).
                let est_logical: i32 = tr(lang, e.desc)
                    .chars()
                    .map(|c| if c.is_ascii() { 7 } else { 14 })
                    .sum();
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                let est_px = (est_logical as f32 * scale).round() as i32;
                let desc_lines = ((est_px + desc_avail - 1) / desc_avail).clamp(1, 3);
                let ctl_block = ctl_h; // 전 종류 한 줄(위치도 10-09부터 드롭다운)
                let h = cpad
                    + title_h
                    + desc_gap
                    + desc_lines * desc_line_h
                    + ctl_gap
                    + ctl_block
                    + self.note_h(row.idx)
                    + cpad;
                Metric {
                    desc_lines,
                    ctl_block,
                    h,
                }
            })
            .collect();
        self.content_h = self
            .rows
            .iter()
            .zip(&metrics)
            .map(|(row, m)| m.h + gap + if row.head.is_some() { head_h } else { 0 })
            .sum();
        let vp_h = self.right_viewport().h;
        self.scroll = self.scroll.clamp(0, (self.content_h - vp_h).max(0));
        // [초기화] 표시 여부 = 기본값과 다름 ∧ 잠기지 않음.
        let show_reset: Vec<bool> = self
            .rows
            .iter()
            .map(|r| self.is_modified(r.idx) && !self.is_locked(r.idx))
            .collect();
        let vp_bottom = self.bounds.bottom() - self.bottom_h();
        let min_color_w = self.s(80);
        let pos_w = self.s(POS_W);
        // 내용은 **밴드 아래**에서 시작한다(밴드가 첫 행을 가리면 못 만진다).
        let mut top = b.y + self.crumb_h() - self.scroll;
        for (ri, row) in self.rows.iter_mut().enumerate() {
            let e = &registry()[row.idx];
            let m = &metrics[ri];
            row.desc_avail = desc_avail;
            row.desc_lines = m.desc_lines;
            row.head_h = if row.head.is_some() { head_h } else { 0 };
            top += row.head_h;
            row.rect = Rect::new(card_x, top, card_w, m.h);
            // 컨트롤 줄 = 좌하단(제목·설명 아래).
            let cy = top + cpad + title_h + desc_gap + m.desc_lines * desc_line_h + ctl_gap;
            let cx = card_x + cpad;
            let secret = matches!(e.kind, SettingKind::Text { secret: true, .. });
            let text_row = matches!(e.kind, SettingKind::Text { .. });
            let ctl_right = match &mut row.ctl {
                RowCtl::Combo(c) => {
                    c.set_bounds(Rect::new(cx, cy, combo_w, ctl_h), inv);
                    c.set_viewport_bottom(vp_bottom); // 아래 끝 행의 팝업이 잘리지 않게(08-20)
                    cx + combo_w
                }
                RowCtl::Check(c) => {
                    c.set_bounds(Rect::new(cx, cy, check_w, ctl_h), inv);
                    cx + check_w
                }
                RowCtl::Font { family, size } => {
                    family.set_bounds(Rect::new(cx, cy, family_w, ctl_h), inv);
                    size.set_bounds(Rect::new(cx + family_w + gap10, cy, size_w, ctl_h), inv);
                    size.set_viewport_bottom(vp_bottom);
                    cx + family_w + gap10 + size_w
                }
                RowCtl::Face(f) if text_row => {
                    f.set_bounds(Rect::new(cx, cy, text_w, ctl_h), inv);
                    // 비밀 행은 상자 오른쪽에 [생성][눈] 두 칸(ctl_h 정사각 · 간격 ctl_h/8).
                    cx + text_w + if secret { ctl_h / 8 * 2 + ctl_h * 2 } else { 0 }
                }
                RowCtl::Face(f) => {
                    f.set_bounds(Rect::new(cx, cy, family_w, ctl_h), inv);
                    cx + family_w
                }
                RowCtl::Pos(p) => {
                    p.set_scale(scale);
                    p.set_bounds(Rect::new(cx, cy, pos_w, ctl_h), inv);
                    p.set_max_bottom(vp_bottom); // 팝업 그리드가 창 아래로 잘리지 않게
                    cx + pos_w
                }
                RowCtl::Color(c) => {
                    c.set_scale(scale);
                    let cw = c
                        .preferred_width()
                        .min((card_w - cpad * 2 - reset_w - gap10).max(min_color_w));
                    c.set_bounds(Rect::new(cx, cy, cw, ctl_h), inv);
                    cx + cw
                }
                RowCtl::Act(b) => {
                    b.set_scale(scale);
                    b.set_bounds(Rect::new(cx, cy, combo_w, ctl_h), inv);
                    cx + combo_w
                }
                RowCtl::Info(_) => cx + text_w,
            };
            let _ = m.ctl_block;
            // [초기화] — 컨트롤 오른쪽(값이 기본값과 다를 때만 · 잠기면 숨김).
            if let Some(btn) = &mut row.reset {
                btn.set_scale(scale);
                let rect = if show_reset[ri] {
                    Rect::new(ctl_right + gap10, cy, reset_w, ctl_h)
                } else {
                    Rect::default()
                };
                btn.set_bounds(rect, inv);
            }
            top += m.h + gap;
        }
        inv.push(self.bounds);
    }

    /// 자식 컨트롤 변경분을 회수해 values/changes에 반영.
    fn drain_changes(&mut self, inv: &mut Invalidations) {
        let mut got = Vec::new();
        let mut warn: Vec<Msg> = Vec::new();
        for row in &mut self.rows {
            let e = &registry()[row.idx];
            match &mut row.ctl {
                RowCtl::Combo(c) => {
                    if let Some(v) = c.take_changed() {
                        // 검증(08-20) — 실패 = 경고 + **직전 확정값 원복**(ControlBase
                        // last_value 상속 · rebuild가 현재값을 시드, 성공 확정마다 갱신).
                        match validate(e.key, &v) {
                            Ok(()) => {
                                c.note_value(v.clone());
                                got.push((e.key, v));
                            }
                            Err(msg) => {
                                let prev = c
                                    .last_value()
                                    .map(str::to_owned)
                                    .or_else(|| self.values.get(e.key).cloned())
                                    .unwrap_or_default();
                                c.select_value(&prev);
                                warn.push(msg);
                            }
                        }
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
                RowCtl::Info(_) => {}
                RowCtl::Font { family, size } => {
                    if let SettingKind::FontSection {
                        family_key,
                        size_key,
                    } = e.kind
                    {
                        // 글꼴명은 **확정 시점만** 보고(08-18 사용자 요청 — 글자마다
                        // 리로드 낭비 · Face 행과 같은 규약). 포커스 아웃 확정은
                        // 위젯 on_event의 blur 수확이 같은 경로로 밀어 넣는다.
                        if let Some(v) = family.take_committed() {
                            got.push((family_key, v));
                        }
                        let _ = family.take_changed(); // 중간 변경은 버린다
                        if let Some(v) = size.take_changed() {
                            got.push((size_key, v));
                        }
                    }
                }
            }
        }
        // [초기화](10-09) — 그 행의 값 키 전부를 기본값으로(FontSection = family+size).
        let mut reset_any = false;
        for row in &mut self.rows {
            if let Some(b) = &mut row.reset {
                if b.take_clicked() {
                    for (k, d) in registry()[row.idx].default_values() {
                        got.push((k, d));
                    }
                    reset_any = true;
                }
            }
        }
        if !got.is_empty() {
            for (k, v) in &got {
                self.values.insert(k, v.clone());
            }
            self.changes.extend(got);
            inv.push(self.bounds);
            if reset_any {
                // 컨트롤 표시를 값에 맞춘다(종류마다 역반영 API가 달라 재구성이 가장 확실하다).
                self.rebuild(inv);
            } else {
                // 기본값 여부([초기화] 노출)·종속 잠금(부모 값)이 바뀌었을 수 있다 — 재배치.
                self.layout(inv);
            }
        }
        if !warn.is_empty() {
            self.warnings.extend(warn);
            inv.push(self.bounds); // 원복된 표시를 즉시 갱신
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

    /// 카드 우상단 **키 이름 + 복사 글리프**(10-09 · nexa-sql 차용 — 고급 키는 accent · 클릭 = 복사) — `right`에서
    /// 왼쪽으로 정렬. 자리를 `key_rect`에 남긴다.
    fn paint_key(
        &self,
        ctx: &mut dyn DrawCtx,
        theme: &Theme,
        row: &RowUi,
        right: i32,
        y: i32,
        clip: Rect,
    ) {
        let key = registry()[row.idx].key;
        let color = if is_advanced(key) {
            theme.accent
        } else {
            theme.text_dim
        };
        ctx.select_font(FontSlot::Base, false);
        let bh = ctx.text_height();
        ctx.select_font(FontSlot::Status, false);
        let th = ctx.text_height();
        let glyph = "⧉";
        let kw = ctx.text_width(key);
        let gw = ctx.text_width(glyph);
        let w = kw + self.s(4) + gw;
        let kx = right - w;
        let ky = y + (bh - th) / 2;
        ctx.text(kx, ky, clip, key, color);
        ctx.text(kx + kw + self.s(4), ky, clip, glyph, color);
        row.key_rect
            .set(Rect::new(kx, y, w, bh).intersection(&clip));
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
        // ── 글꼴명 blur 확정(08-18 사용자 요청 "Enter 또는 포커스 아웃에 반영") —
        //    포커스된 글꼴명 밖을 클릭하면 미확정 텍스트를 그 자리에서 확정 보고한다
        //    (Enter의 take_committed와 같은 경로 · Esc는 취소라 여기 안 온다).
        if let &InputEvent::MouseDown { x, y, .. } = ev {
            let mut got: Vec<(&'static str, String)> = Vec::new();
            for r in &mut self.rows {
                let e = &registry()[r.idx];
                let (family, key) = match (&mut r.ctl, e.kind) {
                    (RowCtl::Font { family, .. }, SettingKind::FontSection { family_key, .. }) => {
                        (family, family_key)
                    }
                    (RowCtl::Face(family), _) => (family, e.key),
                    _ => continue,
                };
                if family.is_focused() && !family.bounds().contains(crate::Point { x, y }) {
                    let v = family.text().trim().to_string();
                    if self.values.get(key).map(String::as_str) != Some(v.as_str()) {
                        got.push((key, v));
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
        // ── 모달 캡처: 열린 위치 드롭다운(10-09)·콤보가 있으면 그것만 이벤트를 받는다(전파 차단) ──
        if let Some(p) = self.rows.iter_mut().find_map(|r| match &mut r.ctl {
            RowCtl::Pos(p) if p.is_open() => Some(p),
            _ => None,
        }) {
            p.on_event(ev, inv);
            self.drain_changes(inv);
            inv.push(self.bounds);
            return;
        }
        if let Some(c) = self.open_combo_mut() {
            c.on_event(ev, inv);
            self.drain_changes(inv);
            inv.push(self.bounds); // 드롭다운 영역 재그리기
            return;
        }

        // ── 인라인 편집(직접 입력) 모달 캡처 — 편집 중 콤보가 모든 입력을 받는다 ──
        // FontSection의 크기 콤보도 포함(08-18 실기 — 빠져 있어 커스텀 px 입력이
        // 검색란으로 샜다: 캐럿은 콤보에, 글자는 검색에 가는 어긋남).
        if let Some(c) = self.rows.iter_mut().find_map(|r| match &mut r.ctl {
            RowCtl::Combo(c) if c.is_editing() => Some(c),
            RowCtl::Font { size, .. } if size.is_editing() => Some(size),
            _ => None,
        }) {
            c.on_event(ev, inv);
            self.drain_changes(inv);
            inv.push(self.bounds);
            return;
        }

        // ── 포커스된 글꼴명 텍스트박스 — **편집 이벤트 일반 라우팅**(08-18 사용자
        //    지적: 드래그 선택·우클릭 메뉴·전체 선택이 공통 기능인데 컨테이너의
        //    키 화이트리스트가 끊었다 — 기능은 TextBox에 이미 있다).
        //    우클릭 편집 메뉴가 열려 있으면 그 박스가 **모달로 전부** 받고,
        //    아니면 Move/Up(드래그 추적)·안쪽 우클릭·SelectAll을 흘린다.
        //    Char·Enter·이동 키는 기존 arm 그대로.
        {
            let mut handled = false;
            if let Some(f) = self.rows.iter_mut().find_map(|r| match &mut r.ctl {
                RowCtl::Font { family, .. } if family.popup_open() || family.is_focused() => {
                    Some(family)
                }
                RowCtl::Face(family) if family.popup_open() || family.is_focused() => Some(family),
                _ => None,
            }) {
                if f.popup_open() {
                    f.on_event(ev, inv);
                    handled = true;
                } else {
                    match *ev {
                        InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. } => {
                            f.on_event(ev, inv); // 드래그 선택 추적(비캡처 — 아래로도 흐른다)
                        }
                        InputEvent::RightDown { x, y } if f.bounds().contains(Point { x, y }) => {
                            f.on_event(ev, inv); // 편집 메뉴 열기
                            handled = true;
                        }
                        InputEvent::SelectAll => {
                            f.on_event(ev, inv);
                            handled = true;
                        }
                        _ => {}
                    }
                }
            }
            if handled {
                self.drain_changes(inv);
                inv.push(self.bounds);
                return;
            }
        }

        // ── 하단 줄(10-09): 고급 스위치 · 설정 파일 열기 · 닫기 ──
        {
            let in_bar = match *ev {
                InputEvent::MouseDown { x, y, .. }
                | InputEvent::MouseUp { x, y }
                | InputEvent::MouseMove { x, y } => self.bottom_rect().contains(Point { x, y }),
                _ => false,
            };
            // MouseUp은 누른 컨트롤이 떼는 자리를 봐야 하므로 항상 흘린다(안쪽에서 눌러 밖에서 떼면 취소).
            if in_bar
                || matches!(
                    *ev,
                    InputEvent::MouseUp { .. } | InputEvent::MouseMove { .. }
                )
            {
                self.adv_switch.on_event(ev, inv);
                self.btn_file.on_event(ev, inv);
                self.btn_close.on_event(ev, inv);
                if let Some(on) = self.adv_switch.take_toggled() {
                    self.advanced = on;
                    self.changes.push((
                        "ui.prefs_advanced",
                        if on { "on" } else { "off" }.to_string(),
                    ));
                    self.rebuild(inv);
                    inv.push(self.bounds);
                    return;
                }
                if self.btn_close.take_clicked() {
                    self.back = true;
                    inv.push(self.bounds);
                    return;
                }
                if self.btn_file.take_clicked() {
                    self.changes.push(("settings.open_file", "run".to_string()));
                    inv.push(self.bounds);
                    return;
                }
                if in_bar {
                    inv.push(self.bounds);
                    return;
                }
            }
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
                // ★ 비밀 행 버튼(clip 09-03 이식) — 눈 = 가림 토글 · 생성 = 2초 무장 후 2차 클릭.
                for r in &mut self.rows {
                    let e = &registry()[r.idx];
                    if self.disabled.contains(e.key) {
                        continue;
                    }
                    if let (RowCtl::Face(f), SettingKind::Text { secret: true, .. }) =
                        (&mut r.ctl, e.kind)
                    {
                        let (er, rr) = pw_btn_rects(f.bounds());
                        if er.contains(p) {
                            f.set_masked(!f.masked());
                            inv.push(self.bounds);
                            return;
                        }
                        if rr.contains(p) {
                            match self.pw_arm {
                                // 무장 창 안 재클릭 = 생성 — 새 암호는 **반드시 보이게**(가림 해제).
                                Some(t) if t.elapsed() <= PW_ARM_WINDOW => {
                                    self.pw_arm = None;
                                    f.set_masked(false);
                                    // 값 생성은 호스트 몫 — 가짜 키로 요청만 올린다(user.test = run 문법).
                                    self.changes
                                        .push(("user.passphrase.regen", "run".to_string()));
                                }
                                // 첫 클릭 = 무장(빨강) — 실수 클릭으로 암호가 바뀌지 않게.
                                _ => self.pw_arm = Some(std::time::Instant::now()),
                            }
                            inv.push(self.bounds);
                            return;
                        }
                    }
                }
                // ★ 키 이름·복사 글리프 클릭(10-09) = 키 복사 요청(클립보드는 호스트 몫).
                if let Some(key) = self
                    .rows
                    .iter()
                    .find(|r| r.key_rect.get().contains(p))
                    .map(|r| registry()[r.idx].key)
                {
                    self.changes.push(("prefs.copy_key", key.to_string()));
                    inv.push(self.bounds);
                    return;
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
                        RowCtl::Info(_) => {}
                    }
                }
                // 사이드바 트리 — ★트리 영역 안의 클릭만 전달한다(08-22 실기: 우측
                // 노트 줄 클릭이 같은 y의 트리 행을 하이라이트 — 카테고리 전환만
                // bounds로 막고 내부 상태는 무방비였다).
                let before = self.tree.selected_row();
                if self.tree.bounds().contains(p) {
                    self.tree.on_event(ev, inv);
                }
                let after = self.tree.selected_row();
                if self.tree.bounds().contains(p) && after != before
                    || (self.tree.bounds().contains(p) && !self.query.is_empty())
                {
                    if let Some(&sel) = self.cat_map.get(after) {
                        self.selected = sel;
                    }
                    self.hist_pos = None;
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
                        RowCtl::Info(_) => {}
                    }
                    if let Some(b) = &mut row.reset {
                        b.on_event(ev, inv);
                    }
                }
                self.drain_changes(inv);
                inv.push(self.bounds);
            }
            InputEvent::MouseUp { .. } => {
                // nexa-ctl 컨트롤은 **전부 "안에서 떼야" 확정**이다(Button·Switch·Combo·Carousel —
                // 10-09 이관 · 종전 nbeep-ctl은 Button만 MouseUp이고 나머지는 MouseDown 완결이었다).
                // 잠긴 행은 MouseDown과 같은 기준으로 건너뛴다(누름이 없었으니 뗌도 무효).
                let locked: Vec<bool> = self.rows.iter().map(|r| self.is_locked(r.idx)).collect();
                for (row, lock) in self.rows.iter_mut().zip(locked) {
                    if lock {
                        continue;
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
                        RowCtl::Info(_) => {}
                    }
                    if let Some(b) = &mut row.reset {
                        b.on_event(ev, inv);
                    }
                }
                self.drain_changes(inv);
            }
            // ★ 포커스된 실행 버튼(09-06) — Space/Enter = 클릭(종전엔 "기본 타이핑 = 검색"이
            //   삼켜 키보드로는 행위 버튼을 누를 수 없었다 · 실기 자동화에서 발각).
            InputEvent::Char { c: ' ', .. }
            | InputEvent::Key {
                key: Key::Enter, ..
            } if self
                .rows
                .iter()
                .any(|r| matches!(&r.ctl, RowCtl::Act(b) if b.is_focused())) =>
            {
                let locked: Vec<bool> = self.rows.iter().map(|r| self.is_locked(r.idx)).collect();
                for (row, lock) in self.rows.iter_mut().zip(locked) {
                    if let RowCtl::Act(b) = &mut row.ctl {
                        if b.is_focused() && !lock {
                            // nexa-ctl Button은 Enter/Space를 스스로 클릭으로 처리한다(`press()` 없음 · 10-09).
                            b.on_event(ev, inv);
                        }
                    }
                }
                self.drain_changes(inv);
                inv.push(self.bounds);
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
                    } else if self.hist_pos.is_some() {
                        self.hist_pos = None; // 이력 탐색 중 Esc = 탐색만 끝낸다
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
                // ★ 검색 이력(10-09 · nexa-sql 차용): 검색 중 ↑/↓ = 최근 검색어 순환(최근이 먼저).
                Key::Up | Key::Down
                    if !self.history.is_empty()
                        && self.search.is_focused()
                        && (!self.query.is_empty() || self.hist_pos.is_some()) =>
                {
                    let n = self.history.len();
                    let next = match (key, self.hist_pos) {
                        (Key::Up, None) => Some(0),
                        (Key::Up, Some(i)) => Some((i + 1).min(n - 1)),
                        (Key::Down, Some(0)) | (Key::Down, None) => None,
                        (Key::Down, Some(i)) => Some(i - 1),
                        _ => self.hist_pos,
                    };
                    self.hist_pos = next;
                    let text = next.map_or(String::new(), |i| self.history[i].clone());
                    self.search.set_text(&text);
                    self.query = text;
                    self.rebuild(inv);
                    inv.push(self.bounds);
                }
                // Enter = 검색어를 이력에 기록(중복 제거 · 최근이 앞 · 최대 20) → 호스트가 영속.
                Key::Enter if self.search.is_focused() && !self.query.trim().is_empty() => {
                    let q = self.query.trim().to_string();
                    self.history.retain(|h| *h != q);
                    self.history.insert(0, q);
                    self.history.truncate(HISTORY_MAX);
                    self.hist_pos = None;
                    self.changes.push(("prefs.search", self.history.join("\t")));
                    inv.push(self.bounds);
                }
                Key::Up | Key::Down if self.query.is_empty() => {
                    // 사이드바 카테고리 탐색(검색 중엔 유지).
                    let before = self.tree.selected_row();
                    self.tree.on_event(ev, inv);
                    let after = self.tree.selected_row();
                    if after != before {
                        if let Some(&sel) = self.cat_map.get(after) {
                            self.selected = sel;
                        }
                        self.hist_pos = None;
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
        ctx.fill_rect(self.bounds, theme.window_bg); // 카드(panel_bg)가 떠 보이는 바탕(10-09)
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

        // 우측 카드(10-09): 바탕 → 제목·키 → 설명 → 컨트롤(+비밀 행 아이콘) → [초기화]·기본값 → 노트.
        let cpad = self.s(CARD_PAD);
        let title_h = self.s(TITLE_H);
        let desc_gap = self.s(2);
        let ctl_gap = self.s(8);
        let desc_line_h = self.s(DESC_LINE_H);
        let ctl_h = self.s(CTL_H);
        for row in &self.rows {
            let e = &registry()[row.idx];
            let r = row.rect;
            if r.bottom() <= vp_clip.y || r.y >= vp_clip.bottom() {
                continue; // 화면 밖 카드
            }
            ctx.fill_round_rect(r, self.s(6), theme.panel_bg);
            let tx = r.x + cpad;
            let ty = r.y + cpad;
            // 제목(글꼴 영역은 굵게 — 종전 규약 유지).
            ctx.select_font(FontSlot::Base, matches!(row.ctl, RowCtl::Font { .. }));
            ctx.text(tx, ty, r, tr(lang, e.label), theme.text);
            // 키 이름 + ⧉ — 우상단.
            self.paint_key(ctx, theme, row, r.right() - cpad, ty, r);
            // 설명 — 카드 폭 전체(컨트롤이 아래로 내려가 침범할 것이 없다).
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
                let dy = title_h + desc_gap + i as i32 * desc_line_h;
                ctx.text(tx, ty + dy, r, line, theme.text_dim);
            }
            let cy = ty + title_h + desc_gap + row.desc_lines * desc_line_h + ctl_gap;
            // 컨트롤.
            match &row.ctl {
                RowCtl::Combo(c) => c.paint(ctx, theme),
                RowCtl::Check(c) => c.paint(ctx, theme),
                RowCtl::Act(b) => b.paint(ctx, theme),
                RowCtl::Pos(g) => g.paint(ctx, theme),
                RowCtl::Face(f) => {
                    f.paint(ctx, theme);
                    // ★ 비밀 행 버튼 — 눈: 보임 = accent · 가림 = 흐림 / 생성: 평소 흐림 · 무장 = 빨강.
                    if matches!(e.kind, SettingKind::Text { secret: true, .. }) {
                        let (er, rr) = pw_btn_rects(f.bounds());
                        let ink = if f.masked() {
                            theme.text_dim
                        } else {
                            theme.accent
                        };
                        tint_icon(&self.pw_eye, PW_EYE_ALPHA, ink.0);
                        draw_pw_icon(&self.pw_eye, er, ctx);
                        let rink = if self.pw_arm.is_some() {
                            theme.danger
                        } else {
                            theme.text_dim
                        };
                        tint_icon(&self.pw_regen, PW_REGEN_ALPHA, rink.0);
                        draw_pw_icon(&self.pw_regen, rr, ctx);
                    }
                }
                RowCtl::Color(c) => c.paint(ctx, theme),
                RowCtl::Font { family, size } => {
                    family.paint(ctx, theme);
                    size.paint(ctx, theme);
                }
                RowCtl::Info(text) => {
                    // 읽기 전용 — 컨트롤 자리에 흐린 글(왼쪽 정렬).
                    ctx.select_font(FontSlot::Status, false);
                    let th = ctx.text_height();
                    let slot = Rect::new(tx, cy, r.w - cpad * 2, ctl_h);
                    ctx.text(tx, cy + (ctl_h - th) / 2, slot, text, theme.text_dim);
                }
            }
            // [초기화](컨트롤 오른쪽 · 기본값과 다를 때만) + "기본값: …"(우하단 · 항상).
            if let Some(b) = &row.reset {
                if b.bounds().w > 0 {
                    b.paint(ctx, theme);
                }
            }
            if let Some(def) = e
                .default_values()
                .into_iter()
                .map(|(_, d)| d)
                .find(|d| !d.is_empty())
            {
                ctx.select_font(FontSlot::Status, false);
                let txt = nbeep_core::tf(Msg::LblDefaultValue, &[&def]);
                let tw = ctx.text_width(&txt);
                let th = ctx.text_height();
                // [초기화]/컨트롤과 겹치면 생략(좁은 카드).
                let left_edge = row.reset.as_ref().filter(|b| b.bounds().w > 0).map_or_else(
                    || ctl_rect(&row.ctl).map_or(tx, |c| c.right()),
                    |b| b.bounds().right(),
                );
                let dx = r.right() - cpad - tw;
                if dx > left_edge + self.s(12) {
                    ctx.text(dx, cy + (ctl_h - th) / 2, r, &txt, theme.text_dim);
                }
            }
            // 노트(호스트 정보 또는 종속 잠금 안내) — 컨트롤 줄 아래 · 카드 안.
            if let Some((note, tone)) = self.row_note(row.idx) {
                let key = e.key;
                let mono = key == "xfer.approval_window"; // 자동 수락 카운트다운만 고정폭
                ctx.select_font(
                    if mono {
                        FontSlot::Mono
                    } else {
                        FontSlot::Status
                    },
                    false,
                );
                let nh = self.s(NOTE_H);
                let ctl_block = ctl_h; // 전 종류 한 줄(위치도 10-09부터 드롭다운)
                let nr = Rect::new(
                    r.x + cpad - self.s(6),
                    cy + ctl_block + self.s(6),
                    r.w - cpad * 2 + self.s(12),
                    nh,
                );
                let th = ctx.text_height();
                let color = match tone {
                    NoteTone::Plain => theme.text_dim,
                    NoteTone::Ok => {
                        ctx.fill_round_rect_alpha(nr, self.s(5), theme.ok, 0.14);
                        theme.ok
                    }
                    NoteTone::Warn => {
                        ctx.fill_round_rect_alpha(nr, self.s(5), theme.warn, 0.14);
                        theme.warn
                    }
                    NoteTone::Info => {
                        ctx.fill_round_rect_alpha(nr, self.s(5), theme.accent, 0.10);
                        theme.text
                    }
                };
                ctx.text(nr.x + self.s(6), nr.y + (nr.h - th) / 2, nr, &note, color);
            }
            // 잠긴 카드는 얇은 가림막(그 위 글은 흐려진다 — "지금은 못 만진다").
            if self.is_locked(row.idx) {
                ctx.fill_round_rect_alpha(r, self.s(6), theme.panel_bg, 0.55);
            }
        }
        // 열린 콤보 드롭다운은 맨 위에 다시 그린다(아래 행에 가리지 않게).
        for row in &self.rows {
            match &row.ctl {
                RowCtl::Combo(c) if c.is_open() || c.editing_popup_open() => c.paint(ctx, theme),
                RowCtl::Font { size, .. } if size.is_open() || size.editing_popup_open() => {
                    size.paint(ctx, theme);
                }
                RowCtl::Pos(p) if p.is_open() => p.paint_popup(ctx, theme),
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
            // 상위 제목 = "그룹 › 카테고리"(10-09) · 본문(Base)보다 **+2px · 굵게**(사용자 확정 08-11).
            ctx.select_font_sized(FontSlot::Base, true, 2.0);
            let th = ctx.text_height();
            let cat_h = self.s(CRUMB_CAT_H);
            let title = match group_of(cat) {
                Some(g) => format!("{} › {}", tr(lang, g), tr(lang, cat)),
                None => tr(lang, cat).to_string(),
            };
            ctx.text(
                crumb.x + self.s(PAD),
                crumb.y + (cat_h - th) / 2,
                crumb,
                &title,
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
        // 고급 숨김 배너(밴드 셋째 줄 · 10-09) — "고급 설정 N개 숨김 — 고급 설정을 켜면 보입니다".
        if self.adv_hidden > 0 {
            ctx.select_font(FontSlot::Status, false);
            let th = ctx.text_height();
            let bh = self.s(BANNER_H);
            let by = crumb.bottom() - bh;
            ctx.text(
                crumb.x + self.s(PAD),
                by + (bh - th) / 2,
                crumb,
                &nbeep_core::tf(Msg::PrefsAdvancedHidden, &[&self.adv_hidden.to_string()]),
                theme.text_dim,
            );
        }
        ctx.fill_rect(
            Rect::new(crumb.x, crumb.bottom() - 1, crumb.w, 1),
            theme.border,
        );
        // ── 하단 줄(10-09) ──
        let bar = self.bottom_rect();
        ctx.fill_rect(bar, theme.chrome_bg);
        ctx.fill_rect(Rect::new(bar.x, bar.y, bar.w, 1), theme.border);
        self.adv_switch.paint(ctx, theme);
        self.btn_file.paint(ctx, theme);
        self.btn_close.paint(ctx, theme);

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

/// 행 컨트롤의 자리(글꼴 영역·정보 행은 `None`) — \[초기화\] 배치 기준.
fn ctl_rect(ctl: &RowCtl) -> Option<Rect> {
    match ctl {
        RowCtl::Combo(c) => Some(c.bounds()),
        RowCtl::Check(c) => Some(c.bounds()),
        RowCtl::Act(b) => Some(b.bounds()),
        RowCtl::Pos(g) => Some(g.bounds()),
        RowCtl::Face(f) => Some(f.bounds()),
        RowCtl::Color(c) => Some(c.bounds()),
        RowCtl::Font { .. } | RowCtl::Info(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (SettingsWidget, Invalidations) {
        // 테스트는 연타 가드(nexa-ctl 기본 350ms · 2단계 확인 버튼이 같은 ms에 두 번 눌린다)를 끈다.
        crate::controls::button::set_default_click_guard_ms(0);
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

    /// 08-20 — 확정 시 검증 규칙: 자동 취소 분은 1~10만 유효(경계 포함),
    /// 그 밖(0·11·비숫자·빈 값)은 경고 Msg. 미등록 키는 무조건 통과.
    #[test]
    fn validate_auto_cancel_range() {
        for ok in ["1", "2", "5", "10"] {
            assert!(validate("xfer.auto_cancel_min", ok).is_ok(), "{ok}");
        }
        for bad in ["0", "11", "60", "abc", "", "-1", "2.5"] {
            assert!(validate("xfer.auto_cancel_min", bad).is_err(), "{bad}");
        }
        assert!(
            validate("xfer.timeout_sec", "999999").is_ok(),
            "미등록 키 = 통과"
        );
        // 타입어헤드 유효시간 200~60000ms(nexa-sql 범위).
        for ok in ["200", "2000", "60000"] {
            assert!(validate("ui.typeahead_timeout", ok).is_ok(), "{ok}");
        }
        for bad in ["199", "60001", "0", "x", ""] {
            assert!(validate("ui.typeahead_timeout", bad).is_err(), "{bad}");
        }
    }

    /// 08-20 — ControlBase 직전값 상속: note_value가 기록하고 last_value로 읽는다
    /// (검증 실패 원복의 기준점 — 전 컨트롤 공통).
    #[test]
    fn control_last_value_inherited() {
        let mut c = Combo::new(vec![ComboItem::new("a", "A")], 0);
        assert_eq!(c.last_value(), None, "확정 이력 없음");
        c.note_value("a");
        assert_eq!(c.last_value(), Some("a"));
        c.note_value("b");
        assert_eq!(c.last_value(), Some("b"), "최신 확정만 유지");
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
    /// 클릭 = 누름+뗌 한 쌍(nexa-ctl 컨트롤은 **MouseUp에서 확정** · 10-09 이관 — 종전 nbeep-ctl은 MouseDown).
    fn click(x: i32, y: i32) -> [InputEvent; 2] {
        [
            InputEvent::MouseDown {
                x,
                y,
                shift: false,
                primary: false,
            },
            InputEvent::MouseUp { x, y },
        ]
    }
    /// 카테고리 강제 선택(테스트 헬퍼).
    fn select_cat(w: &mut SettingsWidget, cat: Msg) {
        let ci = SettingsWidget::cats()
            .iter()
            .position(|(c, _)| *c == cat)
            .unwrap();
        w.selected = TreeSel::Cat(ci);
        let mut inv = Invalidations::default();
        w.rebuild(&mut inv);
        w.set_bounds(Rect::new(0, 0, 560, 560), &mut inv);
    }
    fn select_group(w: &mut SettingsWidget, g: Msg) {
        let gi = CATEGORY_TREE.iter().position(|(x, _)| *x == g).unwrap();
        w.selected = TreeSel::Group(gi);
        let mut inv = Invalidations::default();
        w.rebuild(&mut inv);
        w.set_bounds(Rect::new(0, 0, 560, 560), &mut inv);
    }
    fn keys_of(w: &SettingsWidget) -> Vec<&'static str> {
        w.rows.iter().map(|r| registry()[r.idx].key).collect()
    }

    #[test]
    fn direct_settings_come_first_then_each_sub_group() {
        // 사용자 확정 08-10 — 상위 직속 → 하위1 → 하위2 순서.
        // registry 순서 그대로면 다크 색과 라이트 색 사이에 언어·툴바가 끼어든다.
        // 10-09 재분류: 직속+하위가 함께 있는 카테고리 = 목록(직속 4 · 타입어헤드 4 — 고급 포함).
        let (mut w, mut inv) = widget();
        w.set_advanced(true, &mut inv);
        select_cat(&mut w, Msg::CatPeerList);
        let groups: Vec<Option<Msg>> = w.rows.iter().map(|r| r.group.1).collect();
        assert!(!groups.is_empty());
        // 직속(None)이 앞에 몰려 있어야 한다 — 뒤쪽에 None이 다시 나오면 섞인 것이다.
        let last_direct = groups.iter().rposition(Option::is_none).unwrap();
        let first_sub = groups.iter().position(Option::is_some).unwrap();
        assert!(
            last_direct < first_sub,
            "직속 설정이 하위 그룹 뒤로 흩어졌다: {groups:?}"
        );
        // 하위가 둘인 카테고리(색 — 다크/라이트)에서 같은 하위는 **연속**해야 한다.
        select_cat(&mut w, Msg::CatColors);
        let groups: Vec<Option<Msg>> = w.rows.iter().map(|r| r.group.1).collect();
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
        select_cat(&mut w, Msg::CatColors);
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
        w.set_advanced(true, &mut inv);
        select_cat(&mut w, Msg::CatPeerList);
        // 뷰포트를 낮춰 스크롤 여지를 만든다(하단 줄 44가 생겨 560에선 거의 안 밀린다 · 10-09).
        w.set_bounds(Rect::new(0, 0, 560, 320), &mut inv);
        // 맨 위 = 상위 직속 구간이므로 하위 줄은 비어 있다.
        assert_eq!(w.current_group(), Some((Msg::CatPeerList, None)));
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
            Some((Msg::CatPeerList, Some(want_sub))),
            "스크롤한 그룹이 상단에 남아야 한다"
        );
    }

    /// ★ 08-15 실기 — 설명 워드랩(2~3줄)이 많은 카테고리(IME)에서 **끝까지 스크롤이
    /// 안 되던 것**: 총높이 합산이 워드랩 예약분을 빼고 계산돼 스크롤 상한이 실제
    /// 콘텐츠 끝보다 작았다. 상한까지 스크롤하면 마지막 행이 뷰포트 안에 들어와야 한다.
    #[test]
    fn scroll_upper_bound_reaches_last_row() {
        let (mut w, mut inv) = widget();
        w.set_advanced(true, &mut inv); // IME 카테고리는 전부 고급(10-09)
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
        for e in click(w.bounds.x + sw + 20, w.bounds.y + 4) {
            w.on_event(&e, &mut inv);
        }
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
        let (mut w, mut inv) = widget();
        w.set_advanced(true, &mut inv); // 고급 포함 전부
        let mut shown = 0;
        for i in 0..SettingsWidget::cats().len() {
            w.selected = TreeSel::Cat(i);
            shown += w.visible_indices().len();
        }
        assert_eq!(shown, registry().len());
        // 그룹 보기 합도 같다(그룹 ↔ 카테고리 1:N · 누락·중복 없음).
        let mut by_group = 0;
        for gi in 0..CATEGORY_TREE.len() {
            w.selected = TreeSel::Group(gi);
            by_group += w.visible_indices().len();
        }
        assert_eq!(by_group, registry().len());
    }

    #[test]
    fn defaults_include_toggles_on() {
        let s = SettingsState::with_defaults();
        assert_eq!(s.get("ui.typeahead_space"), "on");
        assert_eq!(s.get("ui.typeahead_special"), "on");
        assert_eq!(s.get("chat.window_mode"), "single");
        assert_eq!(s.get("ui.language"), "system"); // 10-09 D-33-4 = OS 언어 추종이 기본
        assert_eq!(s.get("font.base.size"), "16"); // 절대 px 프리셋(08-18)
    }

    #[test]
    fn combo_row_selection_reports_change() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatSystem);
        // 테마 행(10-09 시스템 카테고리) 콤보 클릭 → 열림 → 두 번째 항목(dark) 클릭.
        let ti = w
            .rows
            .iter()
            .position(|r| registry()[r.idx].key == "ui.theme")
            .expect("테마 행");
        let cb = match &w.rows[ti].ctl {
            RowCtl::Combo(c) => c.bounds(),
            _ => panic!("테마 행은 콤보"),
        };
        for e in click(cb.x + 5, cb.y + 5) {
            w.on_event(&e, &mut inv);
        }
        let pop = match &w.rows[ti].ctl {
            RowCtl::Combo(c) => {
                assert!(c.is_open(), "클릭 = 드롭다운 열림");
                c.popup_rect()
            }
            _ => unreachable!(),
        };
        let item_h = 26; // combo ROW_H(scale 1)
        for e in click(pop.x + 30, pop.y + 4 + item_h + 5) {
            w.on_event(&e, &mut inv);
        }
        // 옵션 = [system, dark, light] — 둘째 항목 = dark(08-29 시스템 추가).
        assert_eq!(w.take_changes(), vec![("ui.theme", "dark".to_string())]);
    }

    #[test]
    fn checkbox_row_toggles_off() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatPeerList);
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
        // 목록 카테고리의 첫 토글(10-09 재분류 — 항목이 앞에 끼면 여기도 갱신).
        assert_eq!(registry()[w.rows[i].idx].key, "ui.link_badge_shape");
        for e in click(cb.x + 3, cb.y + cb.h / 2) {
            w.on_event(&e, &mut inv);
        }
        assert_eq!(
            w.take_changes(),
            vec![("ui.link_badge_shape", "off".to_string())]
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
        for e in click(fb.x + 5, fb.y + 5) {
            w.on_event(&e, &mut inv);
        }
        for c in "Arial".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        // 글자마다 보고하지 않는다(08-18 — Enter/블러 확정만 · 글자마다 리로드 낭비).
        assert!(w.take_changes().is_empty(), "중간 타이핑 = 무보고");
        w.on_event(&key(Key::Enter), &mut inv);
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
        for e in click(fb.x + 5, fb.y + 5) {
            w.on_event(&e, &mut inv);
        }
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
        assert_eq!(w.cat_map.len(), 2, "매치 있는 그룹+카테고리만 사이드바에");
        // ★ 자모열(10-09): 조합 중 "ㅌ"·"테"도 테마를 찾는다 · 키 이름으로도.
        for q in ["테", "ㅌㅔ", "ui.theme"] {
            let (mut w3, mut inv3) = widget();
            for c in q.chars() {
                w3.on_event(&ch(c), &mut inv3);
            }
            assert!(
                w3.visible_indices()
                    .iter()
                    .any(|&i| registry()[i].key == "ui.theme"),
                "{q} → 테마"
            );
        }
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
            .position(|s| matches!(s, TreeSel::Cat(ci) if SettingsWidget::cats()[*ci].0 == Msg::CatAppearance))
            .expect("모양 행");
        let tb = w.tree.bounds();
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        for e in click(tb.x + 10, tb.y + 24 * row as i32 + 5) {
            w.on_event(&e, &mut inv);
        }
        assert!(
            matches!(w.selected, TreeSel::Cat(ci) if SettingsWidget::cats()[ci].0 == Msg::CatAppearance),
            "모양 선택: {:?}",
            w.selected
        );
        assert!(keys_of(&w).contains(&"ui.carousel_scroll"));
    }

    #[test]
    fn advanced_switch_hides_and_counts() {
        // 10-09: 고급 스위치 꺼짐 = ADVANCED 항목 숨김 + 배너 수 · 켜면 전부.
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatPeerList);
        let shown = keys_of(&w);
        assert!(shown.contains(&"ui.list_sort") && shown.contains(&"ui.link_badge_shape"));
        assert!(!shown.contains(&"ui.typeahead_timeout"), "고급은 숨긴다");
        assert_eq!(w.adv_hidden, 6, "숨긴 고급 수 = 배너 수");
        assert!(
            w.crumb_h() > w.s(CRUMB_CAT_H) + w.s(CRUMB_SUB_H),
            "배너 줄만큼 밴드가 높다"
        );
        w.set_advanced(true, &mut inv);
        assert_eq!(keys_of(&w).len(), 9, "타입어헤드 마스터+4 포함 전부");
        assert_eq!(w.adv_hidden, 0);
        // 하단 스위치 클릭으로도 토글되고 변경이 보고된다.
        let sb = w.adv_switch.bounds();
        for e in click(sb.x + 3, sb.y + sb.h / 2) {
            w.on_event(&e, &mut inv);
        }
        assert!(!w.advanced);
        assert_eq!(
            w.take_changes(),
            vec![("ui.prefs_advanced", "off".to_string())]
        );
    }

    #[test]
    fn group_view_shows_all_categories_with_headers() {
        // 그룹 행 선택 = 그 그룹의 카테고리 전부 · 카테고리 경계마다 제목.
        let (mut w, _) = widget();
        select_group(&mut w, Msg::GrpConversation);
        let keys = keys_of(&w);
        assert!(keys.contains(&"chat.window_mode") && keys.contains(&"notify.enabled"));
        let heads: Vec<Msg> = w.rows.iter().filter_map(|r| r.head).collect();
        assert!(heads.contains(&Msg::CatConversation) && heads.contains(&Msg::CatNotify));
        // 표시 순서 = 트리 순서(대화 → 알림 → 그룹).
        let pos = |k: &str| keys.iter().position(|x| *x == k).unwrap();
        assert!(pos("chat.window_mode") < pos("notify.enabled"));
        assert!(pos("notify.enabled") < pos("group.member_invite"));
    }

    #[test]
    fn tree_covers_every_category_and_tables_name_real_keys() {
        for e in registry() {
            assert!(
                tree_pos(e.cat).0 != usize::MAX,
                "{:?}({})가 CATEGORY_TREE에 없다",
                e.cat,
                e.key
            );
        }
        let all: Vec<&str> = registry()
            .iter()
            .flat_map(|e| {
                e.default_values()
                    .into_iter()
                    .map(|(k, _)| k)
                    .chain([e.key])
            })
            .collect();
        for k in ADVANCED {
            assert!(all.contains(k), "ADVANCED 미등록 키 {k}");
        }
        for (c, p, _) in DEPENDS {
            assert!(all.contains(c), "DEPENDS 자식 미등록 {c}");
            assert!(all.contains(p), "DEPENDS 부모 미등록 {p}");
        }
    }

    #[test]
    fn depends_locks_child_until_parent_satisfied() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatNotify);
        let child = w
            .rows
            .iter()
            .position(|r| registry()[r.idx].key == "notify.preview")
            .unwrap();
        assert!(
            !w.is_locked(w.rows[child].idx),
            "알림 켜짐(기본) = 미리보기 잠기지 않음"
        );
        // 부모를 끄면 자식이 잠기고 안내 노트가 생긴다.
        w.set_value("notify.enabled", "off", &mut inv);
        w.layout(&mut inv);
        assert!(w.is_locked(w.rows[child].idx));
        let (note, _) = w.row_note(w.rows[child].idx).expect("잠금 안내");
        assert!(!note.is_empty());
        // 잠긴 자식은 클릭해도 변경이 나오지 않는다.
        let cb = match &w.rows[child].ctl {
            RowCtl::Check(c) => c.bounds(),
            _ => panic!("토글"),
        };
        for e in click(cb.x + 3, cb.y + cb.h / 2) {
            w.on_event(&e, &mut inv);
        }
        assert!(w.take_changes().is_empty());
    }

    #[test]
    fn reset_button_appears_when_modified_and_restores_default() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatSystem);
        let ti = w
            .rows
            .iter()
            .position(|r| registry()[r.idx].key == "ui.theme")
            .unwrap();
        assert_eq!(
            w.rows[ti].reset.as_ref().unwrap().bounds().w,
            0,
            "기본값 = 초기화 숨김"
        );
        w.set_value("ui.theme", "dark", &mut inv);
        w.layout(&mut inv);
        let br = w.rows[ti].reset.as_ref().unwrap().bounds();
        assert!(br.w > 0, "기본값과 다르면 [초기화]가 보인다");
        for e in click(br.x + 3, br.y + br.h / 2) {
            w.on_event(&e, &mut inv);
        }
        assert_eq!(w.take_changes(), vec![("ui.theme", "system".to_string())]);
        assert_eq!(w.values.get("ui.theme").map(String::as_str), Some("system"));
    }

    #[test]
    fn bottom_bar_close_and_open_file() {
        let (mut w, mut inv) = widget();
        let cb = w.btn_close.bounds();
        assert!(cb.w > 0 && cb.y >= w.bounds.bottom() - w.bottom_h());
        for e in click(cb.x + 3, cb.y + 3) {
            w.on_event(&e, &mut inv);
        }
        assert!(w.take_back(), "[닫기] = 닫기 요청");
        let fb = w.btn_file.bounds();
        for e in click(fb.x + 3, fb.y + 3) {
            w.on_event(&e, &mut inv);
        }
        assert_eq!(
            w.take_changes(),
            vec![("settings.open_file", "run".to_string())]
        );
    }

    #[test]
    fn key_click_requests_copy_after_paint() {
        let (mut w, mut inv) = widget();
        select_cat(&mut w, Msg::CatSystem);
        // 페인트가 키 자리를 채운다(ProbeCtx로 실측).
        let mut probe = crate::controls::ProbeCtx;
        w.paint(&mut probe, &Theme::dark());
        let (kr, key) = w
            .rows
            .iter()
            .map(|r| (r.key_rect.get(), registry()[r.idx].key))
            .find(|(r, _)| r.w > 0)
            .expect("키 자리");
        for e in click(kr.x + 2, kr.y + kr.h / 2) {
            w.on_event(&e, &mut inv);
        }
        assert_eq!(w.take_changes(), vec![("prefs.copy_key", key.to_string())]);
    }

    #[test]
    fn search_history_records_on_enter_and_cycles() {
        let (mut w, mut inv) = widget();
        for c in "theme".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        w.on_event(&key(Key::Enter), &mut inv);
        assert_eq!(
            w.take_changes(),
            vec![("prefs.search", "theme".to_string())]
        );
        assert_eq!(w.history, vec!["theme".to_string()]);
        // 두 번째 검색어 → 최근이 앞 · ↑ = 최근부터.
        w.search.set_text("");
        w.query.clear();
        for c in "font".chars() {
            w.on_event(&ch(c), &mut inv);
        }
        w.on_event(&key(Key::Enter), &mut inv);
        assert_eq!(w.history, vec!["font".to_string(), "theme".to_string()]);
        w.on_event(&key(Key::Up), &mut inv);
        assert_eq!(w.query, "font");
        w.on_event(&key(Key::Up), &mut inv);
        assert_eq!(w.query, "theme");
        w.on_event(&key(Key::Down), &mut inv);
        assert_eq!(w.query, "font");
    }

    #[test]
    fn language_defaults_to_system() {
        let s = SettingsState::with_defaults();
        assert_eq!(s.get("ui.language"), "system");
        let mut s2 = SettingsState::with_defaults();
        assert!(s2.set_by_name("ui.language", "ko"));
        assert_eq!(s2.get("ui.language"), "ko");
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
        for e in click(r.x + 3, r.y + 3) {
            w.on_event(&e, &mut inv);
        }
        assert!(w.query.is_empty(), "검색 해제");
    }

    #[test]
    fn escape_requests_close() {
        let (mut w, mut inv) = widget();
        assert!(!w.take_back());
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(w.take_back());
    }

    /// 옵션 없는 자유 입력형(RadioInput(&[], _))의 영속 왕복 — 키가 기본값으로
    /// 등록돼 있어야 set_by_name(파일 로드)이 "아는 키"로 적용한다. 종전엔 키
    /// 부재로 미지 키 취급 = 화면에서 넣은 서버 주소가 재시작마다 증발(08-22 발각).
    #[test]
    fn free_input_setting_survives_reload() {
        let mut st = SettingsState::with_defaults();
        assert_eq!(
            st.get("net.server.address"),
            "beepd.sosomlab.com",
            "기본 서버 = SosomLab 공식 릴레이(08-22 — 첫 옵션 = 기본값)"
        );
        assert!(
            st.set_by_name("net.server.address", "relay.example.com"),
            "아는 키로 인식(미지 키 강등 금지)"
        );
        assert_eq!(st.get("net.server.address"), "relay.example.com");
        // 빈 값은 무시(기본값 유지 — ADR-0011 §4-3 관용 검증).
        assert!(st.set_by_name("net.server.address", ""));
        assert_eq!(st.get("net.server.address"), "relay.example.com");
        // 저장 스냅샷에도 항상 실린다(known_pairs — 로드와 저장의 키 집합 일치).
        assert!(st
            .known_pairs()
            .iter()
            .any(|(k, v)| *k == "net.server.address" && *v == "relay.example.com"));
    }
}
