//! 피어 목록 위젯 — 첫 실물 [`Widget`](M3-1 · FR-D-2 · FR-U-4).
//!
//! M1-6의 즉시 렌더 함수를 위젯으로 전환했다: 캐럿 탐색(↑↓·Home/End·PgUp/PgDn) ·
//! 클릭 선택 · 휠 스크롤(분수 노치 누적) · **타입어헤드**(표시 이름 접두사) · Enter 활성화.
//! 색은 전부 [`Theme`] 토큰이다(하드코딩 금지 — [docs/12 §B]). **신뢰 배지 3종은 항상 표시**.
//!
//! 활성화(대화 열기)는 [`PeerListWidget::take_activated`] 폴링으로 꺼낸다 — 위젯은 부모를
//! 모른다(원본 통지 모델의 번역 — [docs/12 §B]).

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key, WheelAccum};
use crate::geom::{Point, Rect};
use crate::theme::{Color, Theme};
use crate::typeahead::{TypeAhead, TYPEAHEAD_TIMEOUT_MS};
use crate::widget::{Invalidations, Widget};
use nbeep_core::group::GroupId;
use nbeep_core::peers::PeerEntry;
use nbeep_core::{PeerId, TrustLevel};

/// 목록 상단 **그룹 섹션**의 한 행(M5-1 · 사용자 확정 08-13 — 그룹·개인을 한 화면에).
#[derive(Clone, Debug)]
pub struct GroupRow {
    /// 그룹 식별자(로컬).
    pub id: GroupId,
    /// 그룹 이름.
    pub name: String,
    /// 구성원 수.
    pub members: u32,
    /// 지금 세션이 살아 있는 구성원 수(발신 즉시 도달 예상치).
    pub online: u32,
    /// 읽지 않은 방 메시지 수(M5-1g — 0이면 배지 없음).
    pub unread: u32,
    /// 목록 상단 고정(08-15 사용자 요청 — ★ 표시·고정 구획 정렬).
    pub fav: bool,
    /// 내가 소유자인가(메뉴 분기 — 명부 편집·정책 토글은 소유자만).
    pub owned: bool,
    /// 이 방의 구성원 초대 허용 정책(메뉴 라벨·비소유자 초대 가능 여부).
    pub member_invite: bool,
}

/// Enter/더블클릭 활성화 결과 — 그룹 행이 생기며 상대가 둘로 갈렸다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activated {
    /// 1:1 대화 열기.
    Peer(PeerId),
    /// 그룹 스레드 열기.
    Group(GroupId),
}

/// 그룹 컨텍스트 메뉴·다중 선택에서 나온 **호스트 몫 행동**(위젯은 저장소를 모른다).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupAction {
    /// 선택한 상대들로 그룹 생성(이름은 호스트가 모달로 받는다).
    Create { members: Vec<PeerId> },
    /// 그룹 이름 변경(새 이름은 호스트가 모달로 받는다).
    Rename(GroupId),
    /// 현재 선택한 상대들을 이 그룹에 추가.
    AddMembers(GroupId, Vec<PeerId>),
    /// 현재 선택한 상대들을 이 그룹에서 제외.
    RemoveMembers(GroupId, Vec<PeerId>),
    /// 그룹 삭제(소유자 = 해산 · 비소유자 = 탈퇴 — 호스트가 가른다).
    Delete(GroupId),
    /// 구성원 초대 허용 정책 토글(소유자만 — 방별 설정 · 08-13 사용자 확정).
    TogglePolicy(GroupId),
    /// 구성원 목록 보기(08-14 사용자 요청 — 그룹 아이콘 클릭 · 방 헤더 클릭과
    /// **같은 모달**로 호스트가 처리).
    Members(GroupId),
    /// 목록 상단 고정 토글(08-15 사용자 요청 — 그룹방 핀).
    ToggleFav(GroupId),
}

/// 목록 한 행 — 목록 항목(발견) + 신뢰 상태(`TrustStore`). 출처가 달라 조립 지점에서 합친다.
#[derive(Clone, Debug)]
pub struct PeerRow {
    /// 발견 목록 항목.
    pub entry: PeerEntry,
    /// 신뢰 상태(배지 결정).
    pub trust: TrustLevel,
    /// 세션 링크 상태(상태 점 — 사용자 요청 08-09: 끊어진 대상 식별).
    pub link: LinkState,
    /// 진행 중 파일 전송(있으면 이름 **바로 아래**에 진행 막대 · 사용자 요청 08-09).
    pub xfer: Option<XferProgress>,
    /// 프로필에 등록된 표시 이름(M3-17 — 2번째 줄 · 없으면 공백).
    /// 굵은 1번째 줄은 언제나 **기본(발견) 이름**이다(사용자 확정 08-11).
    pub profile_name: Option<String>,
    /// 프로필 사진(M4-5 — imgdec 격리 디코드·원형 마스크 완료본). 없으면 이니셜.
    pub avatar: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 아바타 보더 색(08-14 — 상대가 공개한 값 · 검증 통과분). 소형 표시라 2px.
    pub border: Option<(u8, u8, u8)>,
    /// 읽지 않은 수신 메시지 수(③ 08-13) — 0이면 배지 없음.
    pub unread: u32,
    /// 마지막으로 대화를 확인한 시각 라벨(③ — `unread > 0`일 때만 Some · 배지 왼쪽에 흐리게).
    pub last_read: Option<String>,
    /// 목록 상단 고정(08-15 사용자 요청 — ★ 표시 · 고정 구획 정렬은 호스트 몫).
    pub fav: bool,
    /// 이 키 차단됨(M3-14 — 등급 아이콘을 `badge-x`로 덮는다 · fail-closed의 가시화).
    pub blocked: bool,
    /// 같은 표시 이름을 **다른 키**가 쓴다(M3-14 — v1에서 사칭을 드러내는 유일한
    /// 가시 신호 · 등급 아이콘 옆 `badge-alert` 덧붙는 표식).
    pub conflict: bool,
}

/// 목록 행에 그릴 전송 진행 상태.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XferProgress {
    /// 전송된 바이트(현재 파일까지 누적).
    pub done_bytes: u64,
    /// 전체 대상 바이트.
    pub total_bytes: u64,
    /// 완료된 파일 수.
    pub done_files: u32,
    /// 전체 파일 수.
    pub total_files: u32,
    /// 보내는 중인가(false = 받는 중) — 막대 색을 가른다.
    pub sending: bool,
}

impl XferProgress {
    /// 0.0~1.0 비율.
    #[must_use]
    pub fn ratio(self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.done_bytes as f32 / self.total_bytes as f32).clamp(0.0, 1.0)
    }
}

/// 세션 링크 상태 — 목록에서 색 점으로 식별한다.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LinkState {
    /// 발견만(세션 없음) — 회색.
    #[default]
    Idle,
    /// 세션 활성(대화 중) — 초록.
    Active,
    /// 세션 끊김(있었는데 종료) — 빨강.
    Lost,
    /// 연결 수립 중(워커가 connect+Noise 진행 · M2-8) — 강조색.
    Connecting,
}

/// 행 높이(px) — 아바타 + 2줄(기본 이름·프로필 이름)용으로 확장(사용자 요청 08-11).
pub const ROW_H: i32 = 56;
/// 아바타 지름(논리 px) — 행 높이에서 상하 여백을 뺀 원.
pub const AVATAR_D: i32 = 40;

/// 타입어헤드 HUD 표시 위치 — 3×3 중 택1(기본 좌측하단 · 사용자 확정).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HudPos {
    /// 좌상.
    TopLeft,
    /// 상중앙.
    TopCenter,
    /// 우상.
    TopRight,
    /// 좌중앙.
    MidLeft,
    /// 정중앙.
    Center,
    /// 우중앙.
    MidRight,
    /// 좌하(기본).
    #[default]
    BottomLeft,
    /// 하중앙.
    BottomCenter,
    /// 우하.
    BottomRight,
}

impl HudPos {
    /// 설정 값 코드 → 위치(미지 = 기본 좌하).
    #[must_use]
    pub fn from_code(s: &str) -> Self {
        match s {
            "tl" => Self::TopLeft,
            "tc" => Self::TopCenter,
            "tr" => Self::TopRight,
            "ml" => Self::MidLeft,
            "c" => Self::Center,
            "mr" => Self::MidRight,
            "bc" => Self::BottomCenter,
            "br" => Self::BottomRight,
            _ => Self::BottomLeft,
        }
    }
}

/// 신뢰 배지 라벨(현재 언어) + 테마 토큰 선택.
#[must_use]
pub fn badge(trust: TrustLevel, theme: &Theme) -> (&'static str, Color) {
    match trust {
        TrustLevel::Unverified => (nbeep_core::t(nbeep_core::Msg::TrustUnverified), theme.warn),
        TrustLevel::Pinned => (nbeep_core::t(nbeep_core::Msg::TrustPinned), theme.sel_bg),
        TrustLevel::FingerprintVerified => {
            (nbeep_core::t(nbeep_core::Msg::TrustVerified), theme.ok)
        }
    }
}

/// 신뢰 아이콘 틴트 캐시 슬롯(M3-14) — ((마스크 ptr, 색), 96px 틴트 이미지).
type TrustTintSlot = ((usize, u32), std::rc::Rc<crate::theme::IconImage>);

/// 신뢰 배지 아이콘 선택(M3-14b · Material Symbols 컬러 — 08-15 개편).
/// 반환 = (컬러 RGBA, 등급명, 툴팁 한 줄) · **`None` = 그리지 않는다**.
///
/// `Blocked`가 등급을 **덮고**(fail-closed의 가시화), `Pinned`(정상 기본값)는
/// **목록에서 숨긴다** — 1차의 "흐린 빈 배지"가 실기에서 의미 없는 유령 원으로
/// 읽혔다(사용자 확정 08-15: 문제 상태만 표시 · 색은 자산에 구워져 있다).
#[must_use]
pub fn trust_icon(
    trust: TrustLevel,
    blocked: bool,
) -> Option<(&'static [u8], &'static str, &'static str)> {
    use nbeep_core::{t, Msg};
    if blocked {
        return Some((
            crate::icons::id::BLOCKED_RGBA,
            t(Msg::TrustBlocked),
            t(Msg::TrustBlockedTip),
        ));
    }
    match trust {
        TrustLevel::Unverified => Some((
            crate::icons::id::UNVERIFIED_RGBA,
            t(Msg::TrustUnverified),
            t(Msg::TrustUnverifiedTip),
        )),
        TrustLevel::Pinned => None, // 정상 기본값 — 목록은 조용히(카드·툴팁에서만)
        TrustLevel::FingerprintVerified => Some((
            crate::icons::id::VERIFIED_RGBA,
            t(Msg::TrustVerified),
            t(Msg::TrustVerifiedTip),
        )),
    }
}

/// 세션 상태 점 색(목록 행·그룹 구성원 모달 공용 — 08-15 사용자 요청 "색을 맞춰줘").
/// 색의 뜻을 한 곳에 둔다 — 여기와 다르게 칠하는 곳이 생기면 그게 버그다.
#[must_use]
pub fn link_color(theme: &Theme, link: LinkState) -> Color {
    match link {
        LinkState::Active => theme.ok,
        LinkState::Lost => theme.danger,
        LinkState::Idle => theme.text_dim,
        LinkState::Connecting => theme.accent, // 연결 중(M2-8)
    }
}

/// 세션 상태 배지(M3-19 · [14 §12]) — **색 + 실루엣 2중 부호화**. 적록 색각·저대비에서
/// 색이 무너져도 모양이 남는다: `Idle`=빈 링 · `Connecting`=갭 링(90°·회전) ·
/// `Active`=꽉 찬 원 · `Lost`=가로 막대. 기하는 지름 `D` **비율 고정**(구멍 0.53D ·
/// 막대 0.56×0.19D · 갭 90°)이라 배율 무관. `shape=false`(`ui.link_badge_shape` off)면
/// 종전 채운 원. 자산·의존 0 — 전부 수식 렌더다.
pub fn draw_link_badge(
    ctx: &mut dyn DrawCtx,
    dot: Rect,
    theme: &Theme,
    link: LinkState,
    shape: bool,
    spin_step: u8,
) {
    // 배경색 테두리 — 아바타와 분리(현행 유지 · 파냄도 같은 면으로 뚫는다).
    let back = Rect::new(dot.x - 1, dot.y - 1, dot.w + 2, dot.h + 2);
    ctx.fill_ellipse(back, theme.panel_bg);
    let color = link_color(theme, link);
    if !shape {
        ctx.fill_ellipse(dot, color);
        return;
    }
    let d = dot.w.min(dot.h) as f32;
    match link {
        LinkState::Active => ctx.fill_ellipse(dot, color), // 가장 꽉 찬 실루엣 = 살아 있는 세션
        LinkState::Idle | LinkState::Connecting => {
            ctx.fill_ellipse(dot, color);
            let hole = ((d * 0.53).round() as i32).max(2);
            ctx.fill_ellipse(
                Rect::new(
                    dot.x + (dot.w - hole) / 2,
                    dot.y + (dot.h - hole) / 2,
                    hole,
                    hole,
                ),
                theme.panel_bg,
            );
            if matches!(link, LinkState::Connecting) {
                // 갭 90° — 캐럿 530ms 틱마다 90°씩 회전(새 타이머 0). 파이 반경을
                // 배경 테두리(+1)까지 잡아 링 바깥 AA 잔흔 없이, 테두리 밖은 안 나간다.
                ctx.fill_pie(back, 90.0 * f32::from(spin_step % 4), 90.0, theme.panel_bg);
            }
        }
        LinkState::Lost => {
            ctx.fill_ellipse(dot, color);
            let bw = ((d * 0.56).round() as i32).max(3);
            let bh = ((d * 0.19).round() as i32).max(2);
            ctx.fill_round_rect(
                Rect::new(dot.x + (dot.w - bw) / 2, dot.y + (dot.h - bh) / 2, bw, bh),
                (d * 0.09).round() as i32,
                theme.panel_bg,
            );
        }
    }
}

/// 목록 갱신 직후 스크롤 동작(사용자 확정 08-14 — `ui.list_refresh_scroll`).
/// 발견 이벤트마다 목록이 재조립되는데, 그때 뷰포트를 어떻게 둘지의 3택이다.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefreshScroll {
    /// 현재 위치 유지(기본) — 갱신은 뷰포트를 옮기지 않는다.
    #[default]
    Keep,
    /// 선택(캐럿) 행을 맨 위에 — 갱신마다 캐럿 행이 첫 행으로 온다.
    CaretTop,
    /// 맨 위로 이동.
    Top,
}

impl RefreshScroll {
    /// 설정 문자열 → 모드(미지 값 = 기본 Keep — 관용 파싱).
    #[must_use]
    pub fn from_code(s: &str) -> Self {
        match s {
            "caret" => Self::CaretTop,
            "top" => Self::Top,
            _ => Self::Keep,
        }
    }
}

/// 피어 목록 위젯.
#[derive(Debug)]
pub struct PeerListWidget {
    bounds: Rect,
    rows: Vec<PeerRow>,
    /// 그룹 섹션(목록 상단 · M5-1) — 인덱스 공간에서 그룹이 먼저 온다
    /// (`i < groups.len()` = 그룹 행 · 그 뒤 = `rows[i - groups.len()]`).
    groups: Vec<GroupRow>,
    /// 다중 선택(⌘/Ctrl+클릭 토글 — 그룹 만들기·구성원 편집의 재료).
    /// **PeerId 키**다 — 행 인덱스는 발견 이벤트마다 재배열되어 못 쓴다.
    selected: std::collections::HashSet<PeerId>,
    /// 캐럿(키보드 포커스 행). 목록이 비면 무의미.
    caret: usize,
    /// 범위 선택 기준점(탐색기 anchor · 08-15) — 일반 클릭/⌘클릭/이동이 갱신하고
    /// Shift 계열은 유지한다(Shift 반복 = 범위가 늘었다 줄었다).
    anchor: usize,
    /// 스크롤 상단 행 인덱스.
    top: usize,
    hover: Option<usize>,
    /// 드래그 다중 선택의 시작 행(일반 클릭에서 시작 — 08-13 전수 검사).
    drag_from: Option<usize>,
    wheel: WheelAccum,
    typeahead: TypeAhead,
    activated: Option<Activated>,
    /// 타입어헤드 HUD 위치(설정).
    hud_pos: HudPos,
    /// 타입어헤드에 공백 포함(설정 · 기본 true).
    ta_space: bool,
    /// 타입어헤드에 특수문자 포함(설정 · 기본 true).
    ta_special: bool,
    /// 마지막으로 관측한 단조 시각(ms) — Key 이벤트엔 시각이 없어 힌트로 보관(↑↓ touch용).
    now_hint: u64,
    /// 더블클릭 판정 — 마지막 클릭 (행, 시각 ms). 같은 행 500ms 내 재클릭 = Enter와 동일.
    last_click: Option<(usize, u64)>,
    /// 배율(고DPI — FR-U-6). 행 높이·여백에 곱한다. 좌표·bounds는 물리 px.
    scale: f32,
    /// 우클릭 메뉴(08-11 — "프로필 보기" · M5-1 그룹 항목 추가).
    ctx_menu: crate::controls::ContextMenu,
    /// 메뉴가 가리키는 행의 상대.
    ctx_peer: Option<PeerId>,
    /// 메뉴가 가리키는 그룹 행.
    ctx_group: Option<GroupId>,
    /// "프로필 보기" 선택 결과(1회성 — 호스트 폴).
    profile_req: Option<PeerId>,
    /// 목록 고정 토글 요청(1회성 · 08-15) — (상대, 고정으로 바꿀 값).
    fav_req: Option<(PeerId, bool)>,
    /// 그룹 관련 행동(1회성 — 호스트가 저장소·모달로 처리).
    group_action: Option<GroupAction>,
    /// 갱신 직후 스크롤 동작(설정 — 기본 = 현재 위치 유지 · 08-14).
    refresh_scroll: RefreshScroll,
    /// 세션 배지 실루엣 구분(M3-19 · `ui.link_badge_shape` — 기본 on). off = 종전 채운 원.
    badge_shape: bool,
    /// 신뢰 아이콘 틴트 캐시(M3-14) — (마스크 ptr, 색) → 96px 틴트 이미지.
    /// 페인트가 `&self`라 내부 가변(툴바 `tint`와 같은 문법).
    trust_tint: std::cell::RefCell<Vec<TrustTintSlot>>,
    /// 신뢰 아이콘 hover 툴팁(M3-14 — 등급명이 글자에서 아이콘이 되며 사라져 필수).
    /// (행 인덱스, 충돌 표식 위인가).
    trust_tip: Option<(usize, bool)>,
    /// `Connecting` 갭 링 회전 스텝(90°×4) — 캐럿 530ms 틱을 재사용한다(새 타이머 0).
    /// paint가 `&self`라 [`std::cell::Cell`](Cell)로 위상만 굴린다(레이아웃 불변).
    spin_step: std::cell::Cell<u8>,
    /// 마지막으로 관측한 캐럿 위상 — 뒤집힐 때마다 스텝 전진.
    spin_caret: std::cell::Cell<bool>,
}

impl Default for PeerListWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerListWidget {
    /// 빈 목록 위젯.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bounds: Rect::default(),
            rows: Vec::new(),
            caret: 0,
            anchor: 0,
            top: 0,
            hover: None,
            drag_from: None,
            wheel: WheelAccum::default(),
            typeahead: TypeAhead::new(TYPEAHEAD_TIMEOUT_MS),
            activated: None,
            hud_pos: HudPos::default(),
            ta_space: true,
            ta_special: true,
            now_hint: 0,
            last_click: None,
            scale: 1.0,
            ctx_menu: crate::controls::ContextMenu::new(),
            ctx_peer: None,
            ctx_group: None,
            profile_req: None,
            fav_req: None,
            group_action: None,
            groups: Vec::new(),
            selected: std::collections::HashSet::new(),
            refresh_scroll: RefreshScroll::default(),
            badge_shape: true,
            spin_step: std::cell::Cell::new(0),
            spin_caret: std::cell::Cell::new(true),
            trust_tint: std::cell::RefCell::new(Vec::new()),
            trust_tip: None,
        }
    }

    /// 신뢰 아이콘 자리(행 rect 기준 · **페인트·히트 공유** — 어긋나면 툴팁이 빗나간다).
    /// 반환 = (등급 아이콘, 충돌 표식 자리 — `conflict`일 때만). 18px(사용자 확정
    /// 08-15 "조금 크게" — 컬러 자산이라 커도 소음이 아니라 신호다).
    fn trust_icon_rects(&self, r: Rect, conflict: bool) -> (Rect, Option<Rect>) {
        let d = self.s(18);
        let tr = Rect::new(
            r.right() - self.s(10) - d,
            r.y + (self.row_h() - d) / 2,
            d,
            d,
        );
        let cr = conflict.then(|| Rect::new(tr.x - d - self.s(4), tr.y, d, d));
        (tr, cr)
    }

    /// 96px 컬러 RGBA → 이미지(캐시 — 색이 자산에 구워져 있어 키는 ptr뿐).
    fn trust_image(&self, rgba: &'static [u8]) -> std::rc::Rc<crate::theme::IconImage> {
        let key = (rgba.as_ptr() as usize, 0);
        if let Some((_, img)) = self.trust_tint.borrow().iter().find(|(k, _)| *k == key) {
            return std::rc::Rc::clone(img);
        }
        let img = std::rc::Rc::new(crate::theme::IconImage::from_rgba(
            crate::icons::id::SIZE,
            crate::icons::id::SIZE,
            rgba.to_vec(),
        ));
        let mut cache = self.trust_tint.borrow_mut();
        if cache.len() > 16 {
            cache.clear(); // 상한 — 자산 6종이라 실제로는 닿지 않는다
        }
        cache.push((key, std::rc::Rc::clone(&img)));
        img
    }

    /// 그룹 섹션 교체(M5-1) — 그룹 저장소 변경·온라인 수 갱신 시 호스트가 부른다.
    /// 스크롤 규칙은 [`Self::set_rows`]와 동일(그룹 수 변화가 인덱스를 밀어도
    /// 캐럿은 상대 기준 유지 · 뷰포트는 갱신 정책을 따른다).
    pub fn set_groups(&mut self, groups: Vec<GroupRow>, inv: &mut Invalidations) {
        let anchor = self.peer_at(self.caret).map(|r| r.entry.peer);
        self.groups = groups;
        self.re_anchor(anchor);
        self.apply_refresh_scroll();
        inv.push(self.bounds);
    }

    /// 그룹 관련 행동(1회성) — 생성·개명은 호스트가 이름 모달을 이어 연다.
    pub fn take_group_action(&mut self) -> Option<GroupAction> {
        self.group_action.take()
    }

    /// 현재 다중 선택된 상대들(정렬 — 결정적).
    #[must_use]
    pub fn selected_peers(&self) -> Vec<PeerId> {
        let mut v: Vec<PeerId> = self.selected.iter().copied().collect();
        v.sort();
        v
    }

    /// 다중 선택 해제(그룹 생성 완료 후 등 — 호스트 호출).
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// 피어의 현재 행 인덱스(그룹 구획 오프셋 포함) — 선택 국소 무효화용.
    fn index_of_peer(&self, p: PeerId) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.entry.peer == p)
            .map(|k| k + self.groups.len())
    }

    /// 포커스만(일반 클릭·무수식 이동 — 08-15 사용자 확정 2차: "일반 좌클릭은
    /// Selection 없이 대상만"). 캐럿·anchor를 세우고 **기존 선택은 해제**한다 —
    /// 파란 선택 표시는 Shift/⌘ 제스처에서만 생긴다. 무효화는 바뀐 행만
    /// (캐럿 이동 국소성 계약 FR-U-13).
    fn focus_only(&mut self, i: usize, inv: &mut Invalidations) {
        self.anchor = i;
        if self.selected.is_empty() {
            return;
        }
        if self.selected.len() > 4 {
            inv.push(self.bounds); // 넓은 범위가 접힌다 — 전체가 싸다
        } else {
            let old: Vec<PeerId> = self.selected.iter().copied().collect();
            for p in old {
                if let Some(idx) = self.index_of_peer(p) {
                    inv.push(self.row_rect(idx));
                }
            }
        }
        self.selected.clear();
    }

    /// anchor..=i 범위로 선택을 **교체**(탐색기 Shift 문법) — 더하기가 아니라
    /// 교체라서 Shift를 반대로 움직이면 범위가 줄어든다. 그룹 행은 건너뛴다.
    fn select_range_from_anchor(&mut self, i: usize, inv: &mut Invalidations) {
        self.selected.clear();
        let (lo, hi) = (self.anchor.min(i), self.anchor.max(i));
        for k in lo..=hi {
            if let Some(row) = self.peer_at(k) {
                self.selected.insert(row.entry.peer);
            }
        }
        inv.push(self.bounds);
    }

    #[cfg(test)]
    fn click_at(&mut self, i: usize, shift: bool, primary: bool, inv: &mut Invalidations) {
        let y = self.bounds.y + i32::try_from(i - self.top).unwrap_or(0) * self.row_h() + 2;
        self.on_event(
            &InputEvent::MouseDown {
                x: self.bounds.x + 40,
                y,
                shift,
                primary,
            },
            inv,
        );
    }

    /// 목록 고정 토글 요청(1회성 · 08-15) — 호스트가 신뢰 저장소에 반영한다.
    pub fn take_fav_toggle(&mut self) -> Option<(PeerId, bool)> {
        self.fav_req.take()
    }

    /// "프로필 보기" 선택(1회성) — 호스트가 상대 프로필 창을 연다.
    pub fn take_profile_request(&mut self) -> Option<PeerId> {
        self.profile_req.take()
    }

    /// 타입어헤드 유효시간(ms) 설정 — 마지막 입력 후 이 시간 지나면 초기화.
    pub fn set_typeahead_timeout(&mut self, ms: u64) {
        self.typeahead.set_timeout(ms);
    }

    /// 타입어헤드 HUD 위치 설정.
    pub fn set_hud_pos(&mut self, pos: HudPos, inv: &mut Invalidations) {
        self.hud_pos = pos;
        inv.push(self.bounds);
    }

    /// 타입어헤드에 공백 포함 여부.
    pub fn set_typeahead_space(&mut self, on: bool) {
        self.ta_space = on;
    }

    /// 타입어헤드에 특수문자 포함 여부.
    pub fn set_typeahead_special(&mut self, on: bool) {
        self.ta_special = on;
    }

    /// 이 문자가 타입어헤드에 반영되는가(설정 필터). 한/영/숫자는 항상 포함.
    fn ta_accepts(&self, c: char) -> bool {
        if c == ' ' {
            return self.ta_space;
        }
        if c.is_alphanumeric() {
            return true; // 한글 음절·영문·숫자
        }
        self.ta_special // 그 외 = 특수문자
    }

    /// 타임아웃 틱 — 유효시간 경과 시 버퍼 초기화(HUD 자동 숨김). 소거 시 `true`(재그리기).
    pub fn typeahead_tick(&mut self, now_ms: u64, inv: &mut Invalidations) -> bool {
        self.now_hint = now_ms;
        if self.typeahead.tick(now_ms) {
            inv.push(self.bounds);
            true
        } else {
            false
        }
    }

    /// 배율 지정(창 scale factor 변경·모니터 이동 시) — 레이아웃 전체가 다시 계산된다.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let scale = scale.max(0.5);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.clamp_scroll();
            inv.push(self.bounds);
        }
    }

    /// 물리 px 행 높이(배율 반영).
    #[must_use]
    pub fn row_h(&self) -> i32 {
        (ROW_H as f32 * self.scale).round() as i32
    }

    /// 물리 px 보조 치수.
    fn s(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    /// 목록 교체(발견 이벤트 반영) — 캐럿은 **상대 기준** 유지, 전체 무효화.
    /// 선택은 **사라진 상대만** 걷어낸다(재배열에도 선택 유지 — PeerId 키의 이유).
    /// 스크롤은 [`RefreshScroll`] 모드를 따른다(기본 = 현재 위치 유지 · 08-14).
    pub fn set_rows(&mut self, rows: Vec<PeerRow>, inv: &mut Invalidations) {
        let anchor = self.peer_at(self.caret).map(|r| r.entry.peer);
        self.rows = rows;
        self.selected
            .retain(|p| self.rows.iter().any(|r| r.entry.peer == *p));
        self.re_anchor(anchor);
        self.apply_refresh_scroll();
        inv.push(self.bounds);
    }

    /// 갱신 직후 스크롤 정책 적용(사용자 확정 08-14 — 3택 옵션).
    fn apply_refresh_scroll(&mut self) {
        match self.refresh_scroll {
            RefreshScroll::Keep => self.clamp_top(),
            RefreshScroll::CaretTop => {
                // 선택(캐럿) 행을 맨 위로 — 캐럿이 없으면 Keep과 같다.
                self.top = self.caret;
                self.clamp_top();
            }
            RefreshScroll::Top => self.top = 0,
        }
    }

    /// 갱신 시 스크롤 동작 설정(`ui.list_refresh_scroll` — 호스트가 주입).
    pub fn set_refresh_scroll(&mut self, mode: RefreshScroll) {
        self.refresh_scroll = mode;
    }

    /// 세션 배지 실루엣 on/off(M3-19 · `ui.link_badge_shape` — 핫 스왑).
    pub fn set_badge_shape(&mut self, on: bool, inv: &mut Invalidations) {
        if self.badge_shape != on {
            self.badge_shape = on;
            inv.push(self.bounds);
        }
    }

    /// 전체 행 수 = 그룹 섹션 + 피어(M5-1 — 인덱스 공간은 그룹이 먼저).
    fn total(&self) -> usize {
        self.groups.len() + self.rows.len()
    }

    /// 이 인덱스가 피어 행이면 그 행(그룹 행이면 None).
    fn peer_at(&self, i: usize) -> Option<&PeerRow> {
        self.rows.get(i.checked_sub(self.groups.len())?)
    }

    /// 현재 캐럿 행.
    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Enter/더블클릭으로 활성화된 대상(상대·그룹)을 꺼낸다(1회성 — 루프 소유자가 연다).
    pub fn take_activated(&mut self) -> Option<Activated> {
        self.activated.take()
    }

    /// 이 인덱스의 활성화 대상.
    fn activated_at(&self, i: usize) -> Option<Activated> {
        if let Some(g) = self.groups.get(i) {
            return Some(Activated::Group(g.id));
        }
        self.peer_at(i).map(|r| Activated::Peer(r.entry.peer))
    }

    /// **IME 조합 중 텍스트로 실시간 타입어헤드**(한글 "김" 조합 즉시 이동 — 확정/Space 불필요).
    /// 호스트가 `Ime::Preedit`를 목록 모드일 때 이리로 넘긴다.
    pub fn set_preedit(&mut self, text: &str, now_ms: u64, inv: &mut Invalidations) {
        let q = self.typeahead.set_preedit(text, now_ms);
        if !q.prefix.is_empty() {
            let from = self.caret % self.total().max(1);
            if let Some(hit) = self.find_prefix(&q.prefix, from) {
                self.move_caret(hit, inv);
            }
        }
        inv.push(self.bounds); // HUD 갱신
    }

    fn visible_rows(&self) -> usize {
        (self.bounds.h.max(0) as usize) / (self.row_h().max(1) as usize)
    }

    /// 스크롤 **범위만** 보정(뷰포트 유지) — 내용 교체(set_rows/set_groups)용.
    /// 여기서 캐럿을 따라가면 안 된다(08-14 사용자 실기: 발견 갱신마다 스크롤이
    /// 캐럿 행으로 튀어, 아래로 내려 보던 목록이 1초 뒤 제자리로 끌려왔다).
    fn clamp_top(&mut self) {
        let vis = self.visible_rows().max(1);
        let max_top = self.total().saturating_sub(vis);
        self.top = self.top.min(max_top);
    }

    /// 범위 보정 + **캐럿 따라가기** — 사용자 탐색(캐럿 이동·타입어헤드)용.
    fn clamp_scroll(&mut self) {
        self.clamp_top();
        let vis = self.visible_rows().max(1);
        if self.caret < self.top {
            self.top = self.caret;
        } else if self.caret >= self.top + vis {
            self.top = self.caret + 1 - vis;
        }
    }

    /// 캐럿을 같은 **상대**에 다시 앵커한다 — 재정렬·행 증감에도 캐럿이 인덱스가
    /// 아니라 상대를 따라간다(그전엔 갱신마다 캐럿이 다른 행을 가리킬 수 있었다).
    /// 앵커 대상이 사라졌으면 인덱스 유지(범위 보정만).
    fn re_anchor(&mut self, anchor: Option<PeerId>) {
        if let Some(p) = anchor {
            if let Some(i) = self.rows.iter().position(|r| r.entry.peer == p) {
                self.caret = self.groups.len() + i;
                return;
            }
        }
        self.caret = self.caret.min(self.total().saturating_sub(1));
    }

    fn row_rect(&self, i: usize) -> Rect {
        let rel = i as i64 - self.top as i64;
        Rect::new(
            self.bounds.x,
            self.bounds.y
                + i32::try_from(rel)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(self.row_h()),
            self.bounds.w,
            self.row_h(),
        )
    }

    /// 팝업(우클릭 메뉴)만 다시 그린다 — 호스트가 상태 바 등 자기 크롬을 그린 뒤
    /// 호출해 z-순서를 복구한다(08-13 실기: 메뉴가 하단 정보 텍스트에 덮였다).
    pub fn paint_popup(&self, ctx: &mut dyn crate::draw::DrawCtx, theme: &crate::theme::Theme) {
        if self.ctx_menu.is_open() {
            self.ctx_menu.paint(ctx, theme);
        }
    }

    fn move_caret(&mut self, to: usize, inv: &mut Invalidations) {
        let to = to.min(self.total().saturating_sub(1));
        if to == self.caret || self.total() == 0 {
            return;
        }
        inv.push(self.row_rect(self.caret));
        self.caret = to;
        let before = self.top;
        self.clamp_scroll();
        if self.top != before {
            inv.push(self.bounds); // 스크롤됨 — 전체
        } else {
            inv.push(self.row_rect(self.caret));
        }
    }

    fn row_at(&self, y: i32) -> Option<usize> {
        if y < self.bounds.y {
            return None;
        }
        let rel = ((y - self.bounds.y) / self.row_h().max(1)) as usize;
        let idx = self.top + rel;
        (idx < self.total()).then_some(idx)
    }

    /// 접두사 매치(대소문자 무시) — `from`부터 **앞으로** 순환 검색.
    /// 그룹 행도 매치한다(그룹 이름 점프 — M5-1).
    fn find_prefix(&self, prefix: &str, from: usize) -> Option<usize> {
        let n = self.total();
        if n == 0 {
            return None;
        }
        let p = prefix.to_lowercase();
        (0..n)
            .map(|k| (from + k) % n)
            .find(|&i| self.row_matches(i, &p))
    }

    /// 접두사 매치 — `from`부터 **뒤로** 순환 검색(↑ 순환용).
    fn find_prefix_rev(&self, prefix: &str, from: usize) -> Option<usize> {
        let n = self.total();
        if n == 0 {
            return None;
        }
        let p = prefix.to_lowercase();
        (0..n)
            .map(|k| (from + n - (k % n)) % n)
            .find(|&i| self.row_matches(i, &p))
    }

    fn row_matches(&self, i: usize, lower_prefix: &str) -> bool {
        if let Some(g) = self.groups.get(i) {
            return g.name.to_lowercase().starts_with(lower_prefix);
        }
        self.peer_at(i).is_some_and(|r| {
            r.entry
                .name
                .as_str()
                .to_lowercase()
                .starts_with(lower_prefix)
        })
    }
}

impl Widget for PeerListWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.clamp_scroll();
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        // ── 컨텍스트 메뉴가 열려 있으면 가장 먼저 먹는다(팝업 최상위 — 대화 창과 동일) ──
        if self.ctx_menu.is_open() {
            let menu_rect = self.ctx_menu.bounds();
            if self.ctx_menu.on_event(ev) {
                inv.push(menu_rect);
                inv.push(self.bounds);
                if let Some(id) = self.ctx_menu.take_picked() {
                    match id.as_str() {
                        "profile" => self.profile_req = self.ctx_peer.take(),
                        "fav" => {
                            if let Some(p) = self.ctx_peer.take() {
                                let cur = self
                                    .rows
                                    .iter()
                                    .find(|r| r.entry.peer == p)
                                    .is_some_and(|r| r.fav);
                                self.fav_req = Some((p, !cur));
                            }
                        }
                        "g-fav" => {
                            self.group_action = self.ctx_group.take().map(GroupAction::ToggleFav);
                        }
                        // ── 그룹 행동(M5-1) — 위젯은 요청만 남기고 저장소는 호스트 몫 ──
                        "g-create" => {
                            // 선택이 비어 있으면(선택 밖 우클릭) 우클릭한 그 사람 1명이 재료.
                            let mut members = self.selected_peers();
                            if members.is_empty() {
                                if let Some(p) = self.ctx_peer {
                                    members.push(p);
                                }
                            }
                            self.group_action = Some(GroupAction::Create { members });
                        }
                        "g-rename" => {
                            self.group_action = self.ctx_group.take().map(GroupAction::Rename);
                        }
                        "g-add" => {
                            self.group_action = self
                                .ctx_group
                                .take()
                                .map(|g| GroupAction::AddMembers(g, self.selected_peers()));
                        }
                        "g-remove" => {
                            self.group_action = self
                                .ctx_group
                                .take()
                                .map(|g| GroupAction::RemoveMembers(g, self.selected_peers()));
                        }
                        "g-delete" => {
                            self.group_action = self.ctx_group.take().map(GroupAction::Delete);
                        }
                        "g-policy" => {
                            self.group_action =
                                self.ctx_group.take().map(GroupAction::TogglePolicy);
                        }
                        _ => {}
                    }
                }
                return;
            }
        }
        // ── 우클릭 메뉴(08-11 프로필 · M5-1 그룹) — 즉시 실행이 아니라 메뉴 경유 ──
        if let InputEvent::RightDown { x, y } = *ev {
            if let Some(idx) = self.row_at(y) {
                if self.bounds.contains(crate::geom::Point { x, y }) {
                    self.caret = idx;
                    let sel_n = self.selected.len();
                    let items = if let Some(g) = self.groups.get(idx) {
                        // 그룹 행(M5-1g) — 소유자/비소유자 메뉴 분기 · 편입/초대는
                        // 현재 다중 선택을 재료로 쓴다.
                        self.ctx_group = Some(g.id);
                        self.ctx_peer = None;
                        let mut v = Vec::new();
                        v.push(crate::controls::CtxItem::item(
                            "g-fav",
                            if g.fav {
                                "목록 고정 해제"
                            } else {
                                "목록 상단에 고정"
                            },
                        ));
                        if g.owned {
                            v.push(crate::controls::CtxItem::item("g-rename", "이름 변경"));
                            if sel_n > 0 {
                                v.push(crate::controls::CtxItem::item(
                                    "g-add",
                                    format!("선택한 {sel_n}명 초대"),
                                ));
                                v.push(crate::controls::CtxItem::item(
                                    "g-remove",
                                    format!("선택한 {sel_n}명 제외"),
                                ));
                            }
                            v.push(crate::controls::CtxItem::item(
                                "g-policy",
                                if g.member_invite {
                                    "소유자만 초대로 전환"
                                } else {
                                    "구성원 초대 허용으로 전환"
                                },
                            ));
                            v.push(crate::controls::CtxItem::item("g-delete", "그룹 해산"));
                        } else {
                            // 구성원 — 방 정책이 허용일 때만 초대(요청은 소유자 경유).
                            if g.member_invite && sel_n > 0 {
                                v.push(crate::controls::CtxItem::item(
                                    "g-add",
                                    format!("선택한 {sel_n}명 초대"),
                                ));
                            }
                            v.push(crate::controls::CtxItem::item("g-delete", "그룹 나가기"));
                        }
                        v
                    } else if let Some((peer, fav)) =
                        self.peer_at(idx).map(|row| (row.entry.peer, row.fav))
                    {
                        // 탐색기 관례(08-15 사용자 확정): **선택 밖 우클릭 = 기존 선택
                        // 전부 해제**하고, 우클릭만으로는 선택하지 않는다(메뉴 재료는
                        // ctx_peer 폴백). **선택 안 우클릭 = 선택 유지**(그룹 기능의 진입).
                        self.ctx_peer = Some(peer);
                        self.ctx_group = None;
                        if !self.selected.contains(&peer) {
                            self.selected.clear();
                        }
                        let n = self.selected.len().max(1);
                        vec![
                            crate::controls::CtxItem::item("profile", "프로필 보기"),
                            crate::controls::CtxItem::item(
                                "fav",
                                if fav {
                                    "목록 고정 해제"
                                } else {
                                    "목록 상단에 고정"
                                },
                            ),
                            crate::controls::CtxItem::item(
                                "g-create",
                                format!("그룹 만들기 ({n}명)"),
                            ),
                        ]
                    } else {
                        return;
                    };
                    self.ctx_menu.set_scale(self.scale);
                    // 폭 = 가장 긴 라벨 근사(대화 뷰와 동일 — 고정 120px은 "소유자만
                    // 초대로 전환" 같은 긴 항목이 잘렸다 · 08-14 실기).
                    let widest = items
                        .iter()
                        .map(|it| match it {
                            crate::controls::CtxItem::Item { label, .. } => label
                                .chars()
                                .map(|c| if c.is_ascii() { 8 } else { 15 })
                                .sum::<i32>(),
                            crate::controls::CtxItem::Separator => 0,
                        })
                        .max()
                        .unwrap_or(0);
                    self.ctx_menu.open_at(x, y, items, self.bounds, widest);
                    inv.push(self.bounds);
                    return;
                }
            }
        }
        match *ev {
            // ⌘/Ctrl+A = 피어 전체 선택(다중 선택 집합 — 그룹 생성/초대 흐름 ·
            // 08-13 전수 검사. 그룹 행은 선택 대상이 아니다 — 선택은 PeerId 집합).
            InputEvent::SelectAll => {
                self.selected = self.rows.iter().map(|r| r.entry.peer).collect();
                inv.push(self.bounds);
            }
            InputEvent::Key {
                key,
                shift,
                primary,
            } => {
                let vis = self.visible_rows().max(1);
                let caret_before = self.caret;
                // ⌘/Ctrl+Space = 캐럿 행 선택 토글(탐색기 — ⌘+방향키로 이동하며
                // Space로 골라 담는 키보드 다중 선택 문법). anchor도 그 행으로.
                if primary && matches!(key, Key::Space) {
                    if let Some(row) = self.peer_at(self.caret) {
                        let peer = row.entry.peer;
                        if !self.selected.remove(&peer) {
                            self.selected.insert(peer);
                        }
                        self.anchor = self.caret;
                        inv.push(self.bounds);
                    }
                    return;
                }
                match key {
                    // 타입어헤드 활성이면 ↑/↓ = 현재 접두사 매치 순환(역방향 포함 · 언어 중립).
                    // 순환 중엔 타임아웃 기준 시각을 리셋한다(이동 중 초기화 방지 — 사용자 확정).
                    Key::Down => {
                        let p = self.typeahead.composing();
                        if p.is_empty() {
                            self.move_caret(self.caret + 1, inv);
                        } else {
                            self.typeahead.touch(self.now_hint);
                            let n = self.total().max(1);
                            if let Some(hit) = self.find_prefix(&p, (self.caret + 1) % n) {
                                self.move_caret(hit, inv);
                            }
                        }
                    }
                    Key::Up => {
                        let p = self.typeahead.composing();
                        if p.is_empty() {
                            self.move_caret(self.caret.saturating_sub(1), inv);
                        } else {
                            self.typeahead.touch(self.now_hint);
                            if let Some(hit) =
                                self.find_prefix_rev(&p, self.caret.saturating_sub(1))
                            {
                                self.move_caret(hit, inv);
                            }
                        }
                    }
                    Key::Home => self.move_caret(0, inv),
                    Key::End => self.move_caret(self.total().saturating_sub(1), inv),
                    Key::PageUp => self.move_caret(self.caret.saturating_sub(vis), inv),
                    Key::PageDown => self.move_caret(self.caret + vis, inv),
                    Key::Enter => {
                        if let Some(a) = self.activated_at(self.caret) {
                            self.activated = Some(a);
                        }
                    }
                    Key::Escape => {
                        // 즉시 초기화 + HUD 숨김(사용자 확정).
                        self.typeahead.clear();
                        inv.push(self.bounds);
                    }
                    _ => {}
                }
                // 이동 후 선택 규칙(탐색기 · 08-15 개편):
                //   ⇧+이동 = anchor부터 캐럿까지 **범위 교체**(반대로 가면 줄어든다)
                //   ⌘/Ctrl+이동 = 캐럿만(선택·anchor 불변 — Space로 골라 담는 짝)
                //   무수식 이동 = 캐럿만 + 기존 선택 해제(선택 표시는 Shift/⌘에서만)
                if self.caret != caret_before {
                    if shift && !primary {
                        self.select_range_from_anchor(self.caret, inv);
                    } else if !shift && !primary {
                        self.focus_only(self.caret, inv);
                    }
                }
            }
            InputEvent::Wheel { delta } => {
                let lines = self.wheel.add(delta, 3);
                if lines != 0 {
                    let new_top = if lines > 0 {
                        self.top.saturating_sub(lines.unsigned_abs() as usize)
                    } else {
                        self.top + lines.unsigned_abs() as usize
                    };
                    let vis = self.visible_rows().max(1);
                    let clamped = new_top.min(self.total().saturating_sub(vis));
                    if clamped != self.top {
                        self.top = clamped;
                        inv.push(self.bounds);
                    }
                }
            }
            InputEvent::Char { c, now_ms } => {
                self.now_hint = now_ms;
                if c == '\u{8}' {
                    if let Some(q) = self.typeahead.backspace(now_ms) {
                        if let Some(hit) = self.find_prefix(&q.prefix, self.caret) {
                            self.move_caret(hit, inv);
                        }
                    }
                    inv.push(self.bounds); // HUD 갱신(축소·소거 표시)
                    return;
                }
                if !self.ta_accepts(c) {
                    return; // 설정상 미포함(공백·특수문자)
                }
                let q = self.typeahead.push(c, now_ms);
                let from = if q.include_caret {
                    self.caret
                } else {
                    self.caret + 1
                };
                if let Some(hit) = self.find_prefix(&q.prefix, from % self.total().max(1)) {
                    self.move_caret(hit, inv);
                }
                // 캐럿이 안 움직여도(확장 매치 유지) HUD 텍스트는 바뀐다 — 항상 다시 그린다.
                inv.push(self.bounds);
            }
            InputEvent::MouseDown {
                x,
                y,
                primary,
                shift,
                ..
            } => {
                if let Some(i) = self.row_at(y) {
                    // 그룹 아이콘 클릭 = **구성원 목록**(08-14 사용자 요청 — 방 헤더
                    // 클릭과 같은 모달). 각진 아이콘이라 사각 판정으로 충분하다.
                    if !shift && !primary {
                        if let Some(g) = self.groups.get(i) {
                            let r = self.row_rect(i);
                            let av_d = self.s(AVATAR_D);
                            let icon = Rect::new(
                                r.x + self.s(8),
                                r.y + (self.row_h() - av_d) / 2,
                                av_d,
                                av_d,
                            );
                            if icon.contains(Point { x, y }) {
                                self.group_action = Some(GroupAction::Members(g.id));
                                self.last_click = None;
                                return;
                            }
                        }
                    }
                    // 아바타 원 클릭 = **프로필 보기**(08-14 사용자 요청 — 우클릭 메뉴
                    // "프로필 보기"와 같은 경로). **원 안쪽만**(행 선택과 구분되는 표적 —
                    // 반지름 판정). ⇧/⌘ 수식 클릭은 선택 제스처가 우선이라 제외.
                    if !shift && !primary {
                        if let Some(row) = self.peer_at(i) {
                            let r = self.row_rect(i);
                            let av_d = self.s(AVATAR_D);
                            let (cx, cy) = (
                                r.x + self.s(8) + av_d / 2,
                                r.y + (self.row_h() - av_d) / 2 + av_d / 2,
                            );
                            let (dx, dy) = (x - cx, y - cy);
                            if dx * dx + dy * dy <= (av_d / 2) * (av_d / 2) {
                                self.profile_req = Some(row.entry.peer);
                                self.last_click = None; // 더블클릭(대화 열기)으로 안 이어진다
                                return;
                            }
                        }
                    }
                    // Shift+클릭 = **anchor부터 범위 교체**(탐색기 문법 · 08-15 개편 —
                    // 종전 "더하기"는 반대로 움직여도 안 줄었다). anchor는 유지.
                    if shift && self.peer_at(i).is_some() {
                        self.select_range_from_anchor(i, inv);
                        self.move_caret(i, inv);
                        self.last_click = None;
                        return;
                    }
                    // ⌘/Ctrl+클릭 = 개별 토글(선택은 사람 단위 — 그룹 행 제외) ·
                    // 캐럿·anchor가 그 행으로 온다(탐색기와 동일).
                    if primary {
                        if let Some(row) = self.peer_at(i) {
                            let peer = row.entry.peer;
                            if !self.selected.remove(&peer) {
                                self.selected.insert(peer);
                            }
                            self.anchor = i;
                            self.move_caret(i, inv);
                            inv.push(self.bounds);
                            self.last_click = None; // 선택 토글은 더블클릭으로 안 이어진다
                            return;
                        }
                    } else {
                        // 일반 클릭 = **캐럿(대상)만** — 선택 표시는 만들지 않고 기존
                        // 선택은 해제(사용자 확정 08-15 2차) + anchor 갱신.
                        self.focus_only(i, inv);
                    }
                    self.move_caret(i, inv);
                    // 일반 클릭 = 드래그 다중 선택의 시작 후보(움직여야 발동).
                    self.drag_from = Some(i);
                    // 더블클릭 = Enter 동일 동작(사용자 확정 08-09) — 같은 행 500ms 내 재클릭.
                    // 시각은 now_hint(~5Hz 틱 해상도 ±200ms)라 여유 있는 임계값을 쓴다.
                    let now = self.now_hint;
                    if let Some((li, lt)) = self.last_click {
                        if li == i && now.saturating_sub(lt) <= 500 {
                            if let Some(a) = self.activated_at(i) {
                                self.activated = Some(a);
                            }
                            self.last_click = None; // 트리플클릭 중복 활성화 방지
                            return;
                        }
                    }
                    self.last_click = Some((i, now));
                } else {
                    self.last_click = None;
                }
            }
            InputEvent::MouseMove { x, y, .. } => {
                // 드래그 다중 선택(08-13 전수 검사) — 일반 클릭에서 끌면 시작 행부터
                // 현재 행까지의 피어를 범위 선택(파일 관리자 관례 · 그룹 행은 건너뜀).
                if let Some(a) = self.drag_from {
                    // 경계 밖 = **자동 스크롤**(한 행씩) — 가려진 행까지 선택이 이어진다.
                    if y < self.bounds.y {
                        self.top = self.top.saturating_sub(1);
                        inv.push(self.bounds);
                    } else if y > self.bounds.bottom() {
                        let vis = self.visible_rows().max(1);
                        self.top = (self.top + 1).min(self.total().saturating_sub(vis));
                        inv.push(self.bounds);
                    }
                    let yy = y.clamp(self.bounds.y, self.bounds.bottom() - 1);
                    if let Some(j) = self.row_at(yy) {
                        if j != a {
                            let (lo, hi) = (a.min(j), a.max(j));
                            self.selected.clear();
                            for k in lo..=hi {
                                if let Some(row) = self.peer_at(k) {
                                    self.selected.insert(row.entry.peer);
                                }
                            }
                            self.caret = j.min(self.total().saturating_sub(1));
                            inv.push(self.bounds);
                        }
                    }
                    return;
                }
                let over = self.row_at(y);
                if over != self.hover {
                    if let Some(old) = self.hover {
                        inv.push(self.row_rect(old));
                    }
                    if let Some(new) = over {
                        inv.push(self.row_rect(new));
                    }
                    self.hover = over;
                }
                // 신뢰 아이콘 hover 툴팁(M3-14) — 페인트와 같은 기하로 히트.
                // **그려진 표식만** 히트한다(Pinned 숨김 = 툴팁 대상도 아님).
                let p = Point { x, y };
                let tip = over.and_then(|i| {
                    let row = self.peer_at(i)?;
                    let (tir, cir) = self.trust_icon_rects(self.row_rect(i), row.conflict);
                    if let Some(cr) = cir {
                        if cr.contains(p) {
                            return Some((i, true));
                        }
                    }
                    (trust_icon(row.trust, row.blocked).is_some() && tir.contains(p))
                        .then_some((i, false))
                });
                if tip != self.trust_tip {
                    self.trust_tip = tip;
                    inv.push(self.bounds); // 툴팁은 행 경계를 넘는다 — 전체 무효화
                }
            }
            InputEvent::MouseUp { .. } => {
                self.drag_from = None;
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        ctx.select_font(FontSlot::PeerList, false);
        // Connecting 갭 링 회전(M3-19) — 캐럿 위상이 뒤집힐 때마다 90° 전진.
        // 새 타이머 없이 기존 530ms 틱을 재사용한다(포커스 창이 아니면 정지 = fail-soft).
        let caret_phase = ctx.caret_on();
        if self.spin_caret.get() != caret_phase {
            self.spin_caret.set(caret_phase);
            self.spin_step.set(self.spin_step.get().wrapping_add(1) % 4);
        }
        let vis = self.visible_rows();

        let rh = self.row_h();
        for (rel, i) in (self.top..self.total().min(self.top + vis + 1)).enumerate() {
            let r = Rect::new(
                self.bounds.x,
                self.bounds.y + i32::try_from(rel).unwrap_or(i32::MAX) * rh,
                self.bounds.w,
                rh,
            );
            // 행 배경 — 캐럿 > 다중 선택 > hover > 기본(다중 선택 = 반전 배경 +
            // 좌측 강조 막대 · 08-13 실기: 막대만으로는 "선택됨"이 안 보였다).
            let multi_sel = self
                .peer_at(i)
                .is_some_and(|row| self.selected.contains(&row.entry.peer));
            let bg = if i == self.caret {
                theme.sel_bg
            } else if multi_sel {
                theme.sel_bg_inactive
            } else if Some(i) == self.hover {
                theme.panel_bg_alt
            } else {
                theme.panel_bg
            };
            // 행 배경 먼저(불투명) — 아바타·이름은 그 위에.
            ctx.fill_rect(r, bg);

            // ── 그룹 행(M5-1 · 목록 상단 섹션) — 각진 아이콘으로 개인과 즉시 구분 ──
            if let Some(g) = self.groups.get(i) {
                let av_d = self.s(AVATAR_D);
                let av = Rect::new(r.x + self.s(8), r.y + (rh - av_d) / 2, av_d, av_d);
                ctx.fill_round_rect(av, self.s(10), theme.accent);
                ctx.select_font(FontSlot::PeerList, true);
                let initial: String = g.name.chars().take(1).collect();
                let iw = ctx.text_width(&initial);
                let ith = ctx.text_height();
                ctx.text(
                    av.x + (av.w - iw) / 2,
                    av.y + (av.h - ith) / 2,
                    av,
                    &initial,
                    theme.text,
                );
                let name_x = av.right() + self.s(10);
                let name_th = ctx.text_height();
                let name_y = r.y + self.s(8);
                // 고정 표시(08-15) — 색이 아니라 글리프(★)로(색맹 규약과 정합).
                let gname = if g.fav {
                    format!("★ {}", g.name)
                } else {
                    g.name.clone()
                };
                ctx.text(name_x, name_y, r, &gname, theme.text);
                ctx.select_font(FontSlot::Status, false);
                ctx.text(
                    name_x,
                    name_y + name_th + self.s(3),
                    r,
                    &format!("구성원 {} · 온라인 {}", g.members, g.online),
                    theme.text_dim,
                );
                // 읽지 않은 방 메시지 배지(M5-1g) — 피어 행과 같은 문법(우측 알약).
                if g.unread > 0 {
                    let label = if g.unread > 99 {
                        "99+".to_string()
                    } else {
                        g.unread.to_string()
                    };
                    let bh2 = self.s(18);
                    let bwd = (ctx.text_width(&label) + self.s(12)).max(bh2);
                    let br =
                        Rect::new(r.right() - bwd - self.s(10), r.y + (rh - bh2) / 2, bwd, bh2);
                    ctx.fill_round_rect(br, bh2 / 2, theme.accent);
                    let bth = ctx.text_height();
                    let btw = ctx.text_width(&label);
                    ctx.text(
                        br.x + (br.w - btw) / 2,
                        br.y + (br.h - bth) / 2,
                        br,
                        &label,
                        theme.text,
                    );
                }
                ctx.select_font(FontSlot::PeerList, false);
                ctx.fill_rect(Rect::new(r.x, r.bottom() - 1, r.w, 1), theme.border);
                continue;
            }
            let Some(row) = self.peer_at(i) else { continue };

            // 다중 선택 표시(M5-1) — 좌측 강조 막대(색만으로 안 가르는 규칙 — 위치 신호).
            if self.selected.contains(&row.entry.peer) {
                ctx.fill_rect(Rect::new(r.x, r.y, self.s(4), r.h), theme.accent);
            }
            // ── 원형 이니셜 아바타(가상 이미지 · 08-11) — 색 시드 = 키 지문(안정) ──
            // 실제 사진 렌더는 M4-5(imgdec) 후 — 그때까지 프로필 이미지가 있어도 이니셜.
            let av_d = self.s(AVATAR_D);
            let av = Rect::new(r.x + self.s(8), r.y + (rh - av_d) / 2, av_d, av_d);
            if let Some(img) = &row.avatar {
                // 실사진(원형 마스크 완료본)·내장 12간지(투명 배경) 공용 — **원 배경을
                // 깔고 얹는다**(08-14): 사진은 불투명 원이라 배경이 가려지고, 내장은
                // 시드 색 원이 이니셜·사진과 같은 시각 문법을 만든다.
                ctx.fill_ellipse(av, crate::avatar::avatar_color(row.entry.peer.as_bytes()));
                ctx.image_scaled(av, img, r);
            } else {
                crate::avatar::draw_avatar(
                    ctx,
                    av,
                    row.entry.name.as_str(),
                    row.entry.peer.as_bytes(),
                    6.0,
                );
            }
            // 아바타 보더(08-14) — 상대가 공개한 색 · **소형은 2px**(사용자 확정).
            if let Some((br, bg, bb)) = row.border {
                let c = crate::theme::Color(
                    (u32::from(br) << 16) | (u32::from(bg) << 8) | u32::from(bb),
                );
                ctx.stroke_ellipse(av, c, self.s(2).max(2) as f32);
            }
            // 세션 상태 배지 — 아바타 우하단에 겹쳐(메신저 관례 · M3-19 색+실루엣).
            let dot_d = self.s(11);
            let dot = Rect::new(
                av.right() - dot_d + self.s(2),
                av.bottom() - dot_d + self.s(2),
                dot_d,
                dot_d,
            );
            draw_link_badge(
                ctx,
                dot,
                theme,
                row.link,
                self.badge_shape,
                self.spin_step.get(),
            );

            // 1줄: **기본(발견) 이름 — 굵게**(사용자 확정 08-11).
            let name_x = av.right() + self.s(10);
            ctx.select_font(FontSlot::PeerList, true);
            let name_th = ctx.text_height();
            let name_y = r.y + self.s(8);
            let shown_name = if row.fav {
                format!("★ {}", row.entry.name.as_str())
            } else {
                row.entry.name.as_str().to_string()
            };
            ctx.text(name_x, name_y, r, &shown_name, theme.text);
            // 다중 경로 ×N(진단) — 이름 뒤.
            if row.entry.paths > 1 {
                let name_w = ctx.text_width(&shown_name);
                ctx.select_font(FontSlot::Status, false);
                ctx.text(
                    name_x + name_w + self.s(6),
                    name_y + self.s(2),
                    r,
                    &format!("×{}", row.entry.paths),
                    theme.text_dim,
                );
            }
            // 2줄: 프로필에 등록된 표시 이름(없으면 공백 · M3-17).
            if let Some(pn) = &row.profile_name {
                ctx.select_font(FontSlot::Status, false);
                ctx.text(name_x, name_y + name_th + self.s(3), r, pn, theme.text_dim);
            }

            // 전송 진행 막대 — 행 하단(이름·프로필 줄 아래). 진행 중일 때만.
            if let Some(xp) = row.xfer {
                let bar_h = self.s(4);
                let bar_y = r.bottom() - self.s(8);
                let bar_w = (r.right() - name_x - self.s(120)).max(self.s(40));
                let track = Rect::new(name_x, bar_y, bar_w, bar_h);
                ctx.fill_round_rect(track, bar_h / 2, theme.panel_bg_alt);
                let fill_w = (bar_w as f32 * xp.ratio()).round() as i32;
                if fill_w > 0 {
                    ctx.fill_round_rect(
                        Rect::new(name_x, bar_y, fill_w, bar_h),
                        bar_h / 2,
                        if xp.sending { theme.accent } else { theme.ok },
                    );
                }
            }

            // 신뢰 배지 아이콘(M3-14b · Material Symbols 컬러 — 08-15 개편). 문제
            // 상태만 그린다: Pinned(정상 기본값)는 숨김 — 1차의 흐린 빈 배지가
            // 유령 원으로 읽혔다. Blocked가 등급을 덮고, 이름 충돌은 옆 덧표식.
            // 등급명·설명은 hover 툴팁(아이콘만으로 등급명을 다 나르지 못한다).
            ctx.select_font(FontSlot::PeerList, false);
            let (tir, cir) = self.trust_icon_rects(r, row.conflict);
            let shown = trust_icon(row.trust, row.blocked);
            if let Some((rgba, _label, _tip)) = shown {
                let img = self.trust_image(rgba);
                ctx.image_scaled(tir, &img, r);
            }
            if let Some(cr) = cir {
                let a = self.trust_image(crate::icons::id::CONFLICT_RGBA);
                ctx.image_scaled(cr, &a, r);
            }
            // 뒤따르는 배지들의 왼쪽 기준 — 그린 표식이 없으면 우측 여백 자리.
            let chip_r = match (cir, shown.is_some()) {
                (Some(cr), _) => cr,
                (None, true) => tir,
                (None, false) => Rect::new(r.right() - self.s(10), tir.y, 0, tir.h),
            };

            // 읽지 않은 메시지 배지(③ 08-13) — 신뢰 칩 왼쪽에 강조색 알약 + 개수.
            // 뷰가 닫혀 있는 동안 도착한 수신만 센다(여는 순간 사라진다).
            if row.unread > 0 {
                ctx.select_font(FontSlot::Status, false);
                let label = if row.unread > 99 {
                    "99+".to_string()
                } else {
                    row.unread.to_string()
                };
                let bh = self.s(18);
                let bwd = (ctx.text_width(&label) + self.s(12)).max(bh);
                let br = Rect::new(chip_r.x - bwd - self.s(8), r.y + (rh - bh) / 2, bwd, bh);
                ctx.fill_round_rect(br, bh / 2, theme.accent);
                let bth = ctx.text_height();
                let btw = ctx.text_width(&label);
                ctx.text(
                    br.x + (br.w - btw) / 2,
                    br.y + (br.h - bth) / 2,
                    br,
                    &label,
                    theme.text,
                );
                // 마지막 확인 시각(③) — 배지 왼쪽에 흐리게("그 뒤로 안 봤다"의 기준점).
                if let Some(tl) = &row.last_read {
                    let full = format!("확인 {tl}");
                    let ftw = ctx.text_width(&full);
                    ctx.text(
                        br.x - ftw - self.s(8),
                        r.y + (rh - bth) / 2,
                        r,
                        &full,
                        theme.text_dim,
                    );
                }
                ctx.select_font(FontSlot::PeerList, false);
            }

            // 행 구분선.
            ctx.fill_rect(Rect::new(r.x, r.bottom() - 1, r.w, 1), theme.border);
        }
        ctx.select_font(FontSlot::PeerList, false);
        // 신뢰 아이콘 툴팁(M3-14) — 등급명 + 한 줄 설명. 행들 위에 마지막으로 그린다.
        if let Some((i, on_conflict)) = self.trust_tip {
            if let Some(row) = self.peer_at(i) {
                let r = self.row_rect(i);
                let (tir, cir) = self.trust_icon_rects(r, row.conflict);
                let anchor = if on_conflict { cir.unwrap_or(tir) } else { tir };
                // 숨긴 표식(Pinned)은 대상이 아니다 — None이면 조용히 건너뛴다
                // (paint는 계속돼야 한다 — 메뉴·HUD가 뒤에 그려진다).
                let lt = if on_conflict {
                    Some((
                        nbeep_core::t(nbeep_core::Msg::TrustConflict),
                        nbeep_core::t(nbeep_core::Msg::TrustConflictTip),
                    ))
                } else {
                    trust_icon(row.trust, row.blocked).map(|(_, l, t)| (l, t))
                };
                if let Some((label, tip)) = lt {
                    ctx.select_font(FontSlot::Status, false);
                    let text = format!("{label} — {tip}");
                    let tw = ctx.text_width(&text);
                    let th = ctx.text_height();
                    let (bw, bh) = (tw + self.s(12), th + self.s(8));
                    let bx = (anchor.x + anchor.w / 2 - bw / 2).clamp(
                        self.bounds.x + 2,
                        (self.bounds.right() - bw - 2).max(self.bounds.x + 2),
                    );
                    // 기본은 아이콘 위 — 첫 행처럼 위가 모자라면 아래로 뒤집는다.
                    let mut by = anchor.y - bh - self.s(4);
                    if by < self.bounds.y {
                        by = anchor.bottom() + self.s(4);
                    }
                    let tb = Rect::new(bx, by, bw, bh);
                    ctx.fill_round_rect(tb, self.s(4), theme.panel_bg_alt);
                    ctx.stroke_round_rect(tb, self.s(4), theme.border, 1.0);
                    ctx.text(tb.x + self.s(6), tb.y + self.s(4), tb, &text, theme.text);
                    ctx.select_font(FontSlot::PeerList, false);
                }
            }
        }
        // 우클릭 메뉴(최상위 레이어).
        self.ctx_menu.paint(ctx, theme);
        // 타입어헤드 HUD(입력·조합 중일 때만) — 위치는 설정(3×3). 조합 중 텍스트도 표시.
        let buf = self.typeahead.composing();
        if !buf.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            ctx.select_font(FontSlot::Base, false); // 타입어헤드 = 기본 글꼴(사용자 확정)
            let w = ctx.text_width(&buf) + self.s(16);
            let hh = self.s(20);
            let m = self.s(8);
            let (hx, hy) = {
                use HudPos::*;
                let left = self.bounds.x + m;
                let cx = self.bounds.x + (self.bounds.w - w) / 2;
                let right = self.bounds.right() - w - m;
                let topy = self.bounds.y + m;
                let midy = self.bounds.y + (self.bounds.h - hh) / 2;
                let boty = self.bounds.bottom() - hh - m;
                match self.hud_pos {
                    TopLeft => (left, topy),
                    TopCenter => (cx, topy),
                    TopRight => (right, topy),
                    MidLeft => (left, midy),
                    Center => (cx, midy),
                    MidRight => (right, midy),
                    BottomLeft => (left, boty),
                    BottomCenter => (cx, boty),
                    BottomRight => (right, boty),
                }
            };
            let hud = Rect::new(hx, hy, w, hh);
            ctx.fill_round_rect(hud, self.s(6), theme.field_bg);
            ctx.text(
                hud.x + self.s(8),
                hud.y + self.s(3),
                hud,
                &buf,
                theme.accent,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbeep_core::DisplayName;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn row(b: u8, name: &str, trust: TrustLevel) -> PeerRow {
        PeerRow {
            entry: PeerEntry {
                peer: pid(b),
                name: DisplayName::parse(name).unwrap(),
                paths: 1,
            },
            trust,
            link: LinkState::Idle,
            xfer: None,
            profile_name: None,
            avatar: None,
            border: None,
            unread: 0,
            last_read: None,
            fav: false,
            blocked: false,
            conflict: false,
        }
    }
    fn widget(names: &[(u8, &str)]) -> (PeerListWidget, Invalidations) {
        let mut w = PeerListWidget::new();
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 300, ROW_H * 4), &mut inv); // 4행 가시
        let rows = names
            .iter()
            .map(|&(b, n)| row(b, n, TrustLevel::Unverified))
            .collect();
        w.set_rows(rows, &mut inv);
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
    fn caret_moves_and_invalidates_only_two_rows() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::Down), &mut inv);
        assert_eq!(w.caret(), 1);
        let rects: Vec<_> = inv.drain().collect();
        // 이전 행 + 새 행(인접 = 병합될 수 있음) — 전체 무효화가 아니어야 한다(FR-U-13).
        assert!(rects.iter().all(|r| r.h <= ROW_H * 2), "{rects:?}");
    }

    /// 그룹 아이콘 클릭 = 구성원 목록 요청(08-14 — 방 헤더 클릭과 같은 모달).
    #[test]
    fn group_icon_click_requests_members() {
        let (mut w, mut inv) = widget(&[(1, "alice")]);
        w.set_groups(
            vec![GroupRow {
                id: GroupId(7),
                name: "동물농장".into(),
                members: 3,
                online: 1,
                unread: 0,
                fav: false,
                owned: true,
                member_invite: false,
            }],
            &mut inv,
        );
        // 그룹 행(인덱스 0)의 아이콘 중심 클릭.
        let r = w.row_rect(0);
        let av_d = w.s(AVATAR_D);
        let (cx, cy) = (
            r.x + w.s(8) + av_d / 2,
            r.y + (w.row_h() - av_d) / 2 + av_d / 2,
        );
        w.on_event(&click(cx, cy), &mut inv);
        assert_eq!(
            w.take_group_action(),
            Some(GroupAction::Members(GroupId(7))),
            "아이콘 클릭 = 구성원 목록"
        );
        // 행 본문 클릭 = 캐럿 이동(기존 동작) · 액션 없음.
        w.on_event(&click(r.x + r.w - 20, cy), &mut inv);
        assert_eq!(w.take_group_action(), None);
    }

    /// 아바타 원 클릭 = 프로필 보기(08-14) — 원 안쪽만, 행 본문 클릭은 선택 그대로.
    #[test]
    fn avatar_circle_click_requests_profile_view() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        // 두 번째 행(bob)의 아바타 중심.
        let r = w.row_rect(1);
        let av_d = w.s(AVATAR_D);
        let (cx, cy) = (
            r.x + w.s(8) + av_d / 2,
            r.y + (w.row_h() - av_d) / 2 + av_d / 2,
        );
        w.on_event(&click(cx, cy), &mut inv);
        assert_eq!(
            w.take_profile_request(),
            Some(pid(2)),
            "아바타 원 클릭 = 프로필 보기(우클릭 메뉴와 같은 경로)"
        );
        assert_eq!(
            w.caret(),
            0,
            "프로필 보기 클릭은 행 선택(캐럿)을 바꾸지 않는다"
        );
        // 행 본문(아바타 밖) 클릭 = 기존 선택 동작 그대로 · 프로필 요청 없음.
        w.on_event(&click(r.x + r.w - 20, cy), &mut inv);
        assert_eq!(w.take_profile_request(), None);
        assert_eq!(w.caret(), 1, "본문 클릭 = 캐럿 이동");
    }

    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }

    /// ★ 실기 재현(08-14) — 발견 갱신이 뷰포트를 캐럿 행으로 끌어오면 안 된다.
    /// (행 선택 → 아래로 스크롤 → ~1초 뒤 갱신 → 목록이 선택 행 위치로 튀던 버그.)
    #[test]
    fn refresh_keeps_viewport_by_default() {
        let names: Vec<(u8, String)> = (1..=10).map(|i| (i, format!("peer{i:02}"))).collect();
        let refs: Vec<(u8, &str)> = names.iter().map(|(b, s)| (*b, s.as_str())).collect();
        let (mut w, _) = widget(&refs);
        let mut inv = Invalidations::default();
        // 캐럿은 0(선택 상태) 그대로, 휠로만 아래로 — 캐럿과 뷰포트를 분리한다.
        for _ in 0..6 {
            w.on_event(&InputEvent::Wheel { delta: -120 }, &mut inv);
        }
        let top_before = w.top;
        assert!(top_before > 0, "휠로 내려간 상태여야 재현이 된다");
        let rows2 = refs
            .iter()
            .map(|&(b, n)| row(b, n, TrustLevel::Unverified))
            .collect();
        w.set_rows(rows2, &mut inv);
        assert_eq!(
            w.top, top_before,
            "기본(Keep) = 갱신이 뷰포트를 옮기지 않는다"
        );
        assert_eq!(w.caret, 0, "캐럿(선택) 불변");
    }

    /// 갱신으로 행 순서가 바뀌어도 캐럿은 인덱스가 아니라 **상대**를 따라간다.
    #[test]
    fn refresh_re_anchors_caret_to_peer() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::Down), &mut inv);
        assert_eq!(w.peer_at(w.caret).unwrap().entry.peer, pid(2), "캐럿 = bob");
        // 새 상대가 앞에 끼어들어 인덱스가 밀리는 상황(발견 재정렬).
        let rows2 = vec![
            row(9, "aaron", TrustLevel::Unverified),
            row(1, "alice", TrustLevel::Unverified),
            row(2, "bob", TrustLevel::Unverified),
            row(3, "carol", TrustLevel::Unverified),
        ];
        w.set_rows(rows2, &mut inv);
        assert_eq!(
            w.peer_at(w.caret).unwrap().entry.peer,
            pid(2),
            "캐럿은 여전히 bob(인덱스 1→2로 따라감)"
        );
    }

    /// 3택 옵션(사용자 확정 08-14) — 선택 행 맨 위 / 맨 위로 이동.
    #[test]
    fn refresh_scroll_modes_caret_top_and_top() {
        let names: Vec<(u8, String)> = (1..=10).map(|i| (i, format!("peer{i:02}"))).collect();
        let refs: Vec<(u8, &str)> = names.iter().map(|(b, s)| (*b, s.as_str())).collect();
        let (mut w, _) = widget(&refs);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::Down), &mut inv);
        w.on_event(&key(Key::Down), &mut inv); // 캐럿 = 2
        for _ in 0..6 {
            w.on_event(&InputEvent::Wheel { delta: -120 }, &mut inv);
        }
        let fresh = |names: &[(u8, &str)]| -> Vec<PeerRow> {
            names
                .iter()
                .map(|&(b, n)| row(b, n, TrustLevel::Unverified))
                .collect()
        };
        w.set_refresh_scroll(RefreshScroll::CaretTop);
        w.set_rows(fresh(&refs), &mut inv);
        assert_eq!(w.top, w.caret, "선택 행이 맨 위");
        w.set_refresh_scroll(RefreshScroll::Top);
        for _ in 0..6 {
            w.on_event(&InputEvent::Wheel { delta: -120 }, &mut inv);
        }
        w.set_rows(fresh(&refs), &mut inv);
        assert_eq!(w.top, 0, "맨 위로 이동");
        // 설정 문자열 매핑(관용 파싱 — 미지 값 = 기본).
        assert_eq!(RefreshScroll::from_code("caret"), RefreshScroll::CaretTop);
        assert_eq!(RefreshScroll::from_code("top"), RefreshScroll::Top);
        assert_eq!(RefreshScroll::from_code("whatever"), RefreshScroll::Keep);
    }

    #[test]
    fn caret_follows_scroll_beyond_visible() {
        let names: Vec<(u8, String)> = (1..=10).map(|i| (i, format!("peer{i:02}"))).collect();
        let refs: Vec<(u8, &str)> = names.iter().map(|(b, s)| (*b, s.as_str())).collect();
        let (mut w, _) = widget(&refs);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::End), &mut inv);
        assert_eq!(w.caret(), 9);
        // 4행 가시 창에서 마지막 행이 보이려면 top = 6.
        let rects: Vec<_> = inv.drain().collect();
        assert!(!rects.is_empty(), "스크롤 = 전체 무효화");
    }

    #[test]
    fn double_click_activates_like_enter() {
        let (mut w, mut inv) = widget(&[(1, "alice"), (2, "bob")]);
        w.typeahead_tick(1_000, &mut inv); // now_hint 주입
        let down = |y| InputEvent::MouseDown {
            x: 10,
            y,
            shift: false,
            primary: false,
        };
        // 두 번째 행 더블클릭 = Enter 동일(활성화).
        let y2 = ROW_H + 5;
        w.on_event(&down(y2), &mut inv);
        assert_eq!(w.take_activated(), None, "싱글클릭 = 선택만");
        w.on_event(&down(y2), &mut inv);
        assert_eq!(
            w.take_activated(),
            Some(Activated::Peer(pid(2))),
            "더블클릭 = 활성화"
        );
        // 트리플클릭이 또 활성화하지 않는다(활성화 직후 해제 — 이 클릭은 다시 무장).
        w.on_event(&down(y2), &mut inv);
        assert_eq!(w.take_activated(), None);
        // 시간 경과(500ms 초과) 후 재클릭 = 더블 아님(위 클릭이 t=1000 무장 상태).
        w.typeahead_tick(2_000, &mut inv);
        w.on_event(&down(y2), &mut inv);
        assert_eq!(w.take_activated(), None, "간격 초과 = 활성화 없음");
    }

    #[test]
    fn enter_activates_caret_peer_once() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob")]);
        let mut inv = Invalidations::default();
        w.on_event(&key(Key::Down), &mut inv);
        w.on_event(&key(Key::Enter), &mut inv);
        assert_eq!(w.take_activated(), Some(Activated::Peer(pid(2))));
        assert_eq!(w.take_activated(), None, "1회성");
    }

    #[test]
    fn click_selects_row_under_cursor() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        w.on_event(
            &InputEvent::MouseDown {
                x: 10,
                y: ROW_H * 2 + 5,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 2);
        // 목록 밖 클릭은 무시.
        w.on_event(
            &InputEvent::MouseDown {
                x: 10,
                y: ROW_H * 3 + 5,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 2);
    }

    #[test]
    fn typeahead_prefix_and_single_key_cycle() {
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "bora"), (4, "carol")]);
        let mut inv = Invalidations::default();
        // "bo" 누적 → bob.
        w.on_event(&InputEvent::Char { c: 'b', now_ms: 0 }, &mut inv);
        assert_eq!(w.caret(), 1);
        w.on_event(
            &InputEvent::Char {
                c: 'o',
                now_ms: 100,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 1, "확장 매치 — bob 유지");
        // 타임아웃 후 'b' = 새 접두사(캐럿 다음부터 첫 매치). 반복 키 자동 순환 없음(↑↓ 전용).
        w.on_event(
            &InputEvent::Char {
                c: 'b',
                now_ms: 5000,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 2, "새 접두사 = 다음 매치 bora");
        w.on_event(
            &InputEvent::Char {
                c: 'b',
                now_ms: 5100,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 2, "'bb' 누적 — 매치 없으면 유지(자동 순환 제거)");
    }

    #[test]
    fn arrows_cycle_matches_when_typeahead_active() {
        let (mut w, _) = widget(&[
            (1, "alice"),
            (2, "bob"),
            (3, "bora"),
            (4, "bill"),
            (5, "carol"),
        ]);
        let mut inv = Invalidations::default();
        w.on_event(&InputEvent::Char { c: 'b', now_ms: 0 }, &mut inv);
        assert_eq!(w.caret(), 1, "첫 b = bob");
        w.on_event(&key(Key::Down), &mut inv);
        assert_eq!(w.caret(), 2, "↓ = bora");
        w.on_event(&key(Key::Down), &mut inv);
        assert_eq!(w.caret(), 3, "↓ = bill");
        w.on_event(&key(Key::Up), &mut inv);
        assert_eq!(w.caret(), 2, "↑ = 역순 bora");
        // 타입어헤드 비활성이면 ↓는 일반 행 이동.
        w.on_event(&key(Key::Escape), &mut inv); // 버퍼 소거
        w.on_event(&key(Key::Down), &mut inv);
        assert_eq!(w.caret(), 3, "비활성 = 다음 행(전체)");
    }

    #[test]
    fn scale_resizes_rows_and_hit_testing() {
        // 2배율에서 행 높이·클릭 좌표→행 매핑이 함께 커진다(FR-U-6).
        let (mut w, _) = widget(&[(1, "alice"), (2, "bob"), (3, "carol")]);
        let mut inv = Invalidations::default();
        w.set_scale(2.0, &mut inv);
        assert!(!inv.is_empty(), "배율 변경 = 전체 무효화");
        assert_eq!(w.row_h(), ROW_H * 2);
        w.on_event(
            &InputEvent::MouseDown {
                x: 10,
                y: ROW_H * 2 + 5, // 1배율이면 2행, 2배율이면 1행
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.caret(), 1, "물리 좌표는 배율 행 높이로 나눈다");
    }

    #[test]
    fn wheel_scrolls_with_fractional_accumulation() {
        let names: Vec<(u8, String)> = (1..=10).map(|i| (i, format!("p{i:02}"))).collect();
        let refs: Vec<(u8, &str)> = names.iter().map(|(b, s)| (*b, s.as_str())).collect();
        let (mut w, _) = widget(&refs);
        let mut inv = Invalidations::default();
        // 아래로 한 노치(-120) = 3행.
        w.on_event(&InputEvent::Wheel { delta: -120 }, &mut inv);
        w.on_event(&key(Key::Up), &mut inv); // 캐럿 0인 채 top만 이동했는지 확인용
        assert!(!inv.is_empty());
    }

    /// 다중 선택(M5-1 · 08-13 수정) — ⌘클릭 토글 + Shift 범위 + 일반 클릭 해제.
    /// (⌘클릭이 죽어 있던 원인은 앱의 마우스 변환이 수식키를 false로 하드코딩한 것 —
    /// 위젯 계층은 이 테스트로 계약을 고정한다.)
    #[test]
    fn multi_select_toggle_and_shift_range() {
        let (mut w, mut inv) = widget(&[(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
        w.click_at(0, false, true, &mut inv); // ⌘클릭 = 토글 on
        w.click_at(2, false, true, &mut inv);
        assert_eq!(w.selected_peers(), vec![pid(1), pid(3)]);
        w.click_at(2, false, true, &mut inv); // 같은 행 다시 = 해제
        assert_eq!(w.selected_peers(), vec![pid(1)]);
        // 일반 클릭 = 캐럿(대상)만 — 선택 표시 없음 + 기존 선택 해제 + anchor.
        w.click_at(1, false, false, &mut inv);
        assert!(w.selected_peers().is_empty(), "일반 클릭 = 선택 없음");
        w.click_at(3, true, false, &mut inv); // Shift = anchor(1)..=3 범위 교체
        assert_eq!(w.selected_peers(), vec![pid(2), pid(3), pid(4)]);
        // Shift를 반대로 = 범위가 **줄어든다**(교체 문법 — 종전 더하기와 다르다).
        w.click_at(2, true, false, &mut inv);
        assert_eq!(
            w.selected_peers(),
            vec![pid(2), pid(3)],
            "역방향 Shift = 축소"
        );
    }

    /// 탐색기 우클릭 규칙(08-15 사용자 확정) — ① 선택 밖 우클릭 = 전부 해제(우클릭
    /// 만으로 선택하지 않는다) ② 선택 안 우클릭 = 선택 유지(그룹 기능 진입).
    #[test]
    fn right_click_outside_clears_inside_keeps() {
        let (mut w, mut inv) = widget(&[(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
        w.click_at(0, false, true, &mut inv);
        w.click_at(1, false, true, &mut inv);
        assert_eq!(w.selected_peers(), vec![pid(1), pid(2)]);
        let rd = |i: usize, w: &PeerListWidget| InputEvent::RightDown {
            x: w.bounds.x + 40,
            y: w.bounds.y + i32::try_from(i).unwrap() * w.row_h() + 2,
        };
        // 선택 안(행 1) 우클릭 = 유지.
        w.on_event(&rd(1, &w), &mut inv);
        assert_eq!(w.selected_peers(), vec![pid(1), pid(2)], "선택 안 = 유지");
        w.ctx_menu.close();
        // 선택 밖(행 3) 우클릭 = 전부 해제 + 그 행도 **선택되지 않는다**.
        w.on_event(&rd(3, &w), &mut inv);
        assert!(w.selected_peers().is_empty(), "선택 밖 = 전부 해제·미선택");
    }

    /// 키보드 문법(탐색기 · 08-15): 무수식 이동=단일 · ⇧이동=범위 교체(축소 포함) ·
    /// ⌘이동=캐럿만 · ⌘Space=토글.
    #[test]
    fn keyboard_selection_grammar() {
        let (mut w, mut inv) = widget(&[(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
        let k = |key, shift, primary| InputEvent::Key {
            key,
            shift,
            primary,
        };
        w.on_event(&k(Key::Down, false, false), &mut inv); // 캐럿 1 — 선택 없음
        assert!(w.selected_peers().is_empty(), "무수식 이동 = 선택 없음");
        w.on_event(&k(Key::Down, true, false), &mut inv); // ⇧↓ = 1..=2
        w.on_event(&k(Key::Down, true, false), &mut inv); // ⇧↓ = 1..=3
        assert_eq!(w.selected_peers(), vec![pid(2), pid(3), pid(4)]);
        w.on_event(&k(Key::Up, true, false), &mut inv); // ⇧↑ = 범위 축소 1..=2
        assert_eq!(w.selected_peers(), vec![pid(2), pid(3)], "역방향 = 축소");
        // ⌘↑ = 캐럿만 이동(선택 불변) → ⌘Space = 캐럿(1행) 토글 해제.
        w.on_event(&k(Key::Up, false, true), &mut inv);
        w.on_event(&k(Key::Up, false, true), &mut inv); // 캐럿 0
        assert_eq!(
            w.selected_peers(),
            vec![pid(2), pid(3)],
            "⌘이동 = 선택 불변"
        );
        w.on_event(&k(Key::Space, false, true), &mut inv); // ⌘Space = 0행(pid 1) 담기
        assert_eq!(w.selected_peers(), vec![pid(1), pid(2), pid(3)]);
        w.on_event(&k(Key::Space, false, true), &mut inv); // 다시 = 빼기
        assert_eq!(w.selected_peers(), vec![pid(2), pid(3)]);
        // 무수식 이동 = 선택이 전부 접힌다(캐럿만 남는다 — 08-15 2차 확정).
        w.on_event(&k(Key::Down, false, false), &mut inv);
        assert!(w.selected_peers().is_empty());
    }

    #[test]
    fn select_all_selects_every_peer() {
        let (mut w, mut inv) = widget(&[(1, "a"), (2, "b"), (3, "c")]);
        w.on_event(&InputEvent::SelectAll, &mut inv);
        assert_eq!(
            w.selected_peers(),
            vec![pid(1), pid(2), pid(3)],
            "⌘/Ctrl+A = 피어 전체 선택(그룹 생성 흐름)"
        );
    }

    /// 신뢰 배지 아이콘 매핑(M3-14b) — Blocked가 등급을 덮고, **Pinned는 숨긴다**
    /// (1차의 흐린 빈 배지가 유령 원으로 읽힌 실기 — 문제 상태만 표시).
    #[test]
    fn trust_icon_mapping_and_blocked_override() {
        let (m, _, _) = trust_icon(TrustLevel::Unverified, false).expect("미검증은 표시");
        assert_eq!(m, crate::icons::id::UNVERIFIED_RGBA);
        assert!(
            trust_icon(TrustLevel::Pinned, false).is_none(),
            "정상 기본값은 그리지 않는다"
        );
        let (m, _, _) = trust_icon(TrustLevel::FingerprintVerified, false).expect("표시");
        assert_eq!(m, crate::icons::id::VERIFIED_RGBA);
        // 차단은 등급이 아니라 fail-closed 상태 — 어떤 등급이든 덮는다(Pinned조차).
        let (m, _, _) = trust_icon(TrustLevel::Pinned, true).expect("차단은 항상 표시");
        assert_eq!(m, crate::icons::id::BLOCKED_RGBA);
        // 자산 계약 — 96×96 RGBA 원시 바이트.
        let want = (crate::icons::id::SIZE * crate::icons::id::SIZE * 4) as usize;
        for a in [
            crate::icons::id::UNVERIFIED_RGBA,
            crate::icons::id::PINNED_RGBA,
            crate::icons::id::VERIFIED_RGBA,
            crate::icons::id::BLOCKED_RGBA,
            crate::icons::id::CONFLICT_RGBA,
            crate::icons::id::FIRSTCONTACT_RGBA,
        ] {
            assert_eq!(a.len(), want);
        }
    }

    /// 충돌 표식 자리(M3-14) — 등급 아이콘 **왼쪽**에 같은 크기로 나란히.
    #[test]
    fn conflict_rect_sits_left_of_trust_icon() {
        let (w, _) = widget(&[(1, "a")]);
        let r = w.row_rect(0);
        let (tir, cir) = w.trust_icon_rects(r, true);
        let cr = cir.expect("충돌 표식 자리");
        assert_eq!(cr.w, tir.w);
        assert_eq!(cr.y, tir.y);
        assert!(cr.right() < tir.x, "충돌 표식은 등급 왼쪽");
        assert!(tir.right() <= r.right(), "행 안에 있다");
    }

    /// 아이콘 hover = 툴팁 상태 성립·이탈 소거(M3-14 — 등급명이 글자에서 사라져 필수).
    #[test]
    fn trust_tooltip_arms_on_icon_hover() {
        let (mut w, mut inv) = widget(&[(1, "a"), (2, "b")]);
        let (tir, _) = w.trust_icon_rects(w.row_rect(0), false);
        w.on_event(
            &InputEvent::MouseMove {
                x: tir.x + tir.w / 2,
                y: tir.y + tir.h / 2,
            },
            &mut inv,
        );
        assert_eq!(w.trust_tip, Some((0, false)), "아이콘 위 = 툴팁");
        w.on_event(&InputEvent::MouseMove { x: 5, y: tir.y }, &mut inv);
        assert_eq!(w.trust_tip, None, "아이콘 밖 = 소거");
    }
}
