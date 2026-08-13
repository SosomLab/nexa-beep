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
use crate::geom::Rect;
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
    /// 읽지 않은 수신 메시지 수(③ 08-13) — 0이면 배지 없음.
    pub unread: u32,
    /// 마지막으로 대화를 확인한 시각 라벨(③ — `unread > 0`일 때만 Some · 배지 왼쪽에 흐리게).
    pub last_read: Option<String>,
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
    /// 스크롤 상단 행 인덱스.
    top: usize,
    hover: Option<usize>,
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
    /// 그룹 관련 행동(1회성 — 호스트가 저장소·모달로 처리).
    group_action: Option<GroupAction>,
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
            top: 0,
            hover: None,
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
            group_action: None,
            groups: Vec::new(),
            selected: std::collections::HashSet::new(),
        }
    }

    /// 그룹 섹션 교체(M5-1) — 그룹 저장소 변경·온라인 수 갱신 시 호스트가 부른다.
    pub fn set_groups(&mut self, groups: Vec<GroupRow>, inv: &mut Invalidations) {
        self.groups = groups;
        self.caret = self.caret.min(self.total().saturating_sub(1));
        self.clamp_scroll();
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

    /// 목록 교체(발견 이벤트 반영) — 캐럿은 가능한 유지, 전체 무효화.
    /// 선택은 **사라진 상대만** 걷어낸다(재배열에도 선택 유지 — PeerId 키의 이유).
    pub fn set_rows(&mut self, rows: Vec<PeerRow>, inv: &mut Invalidations) {
        self.rows = rows;
        self.selected
            .retain(|p| self.rows.iter().any(|r| r.entry.peer == *p));
        self.caret = self.caret.min(self.total().saturating_sub(1));
        self.clamp_scroll();
        inv.push(self.bounds);
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

    fn clamp_scroll(&mut self) {
        let vis = self.visible_rows().max(1);
        let max_top = self.total().saturating_sub(vis);
        self.top = self.top.min(max_top);
        // 캐럿이 보이도록 스크롤 따라가기.
        if self.caret < self.top {
            self.top = self.caret;
        } else if self.caret >= self.top + vis {
            self.top = self.caret + 1 - vis;
        }
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
                        // ── 그룹 행동(M5-1) — 위젯은 요청만 남기고 저장소는 호스트 몫 ──
                        "g-create" => {
                            self.group_action = Some(GroupAction::Create {
                                members: self.selected_peers(),
                            });
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
                    } else if let Some(row) = self.peer_at(idx) {
                        // 피어 행 — 우클릭 대상이 선택에 없으면 그 한 명도 재료에 포함되게
                        // 선택에 넣는다(선택 없이 우클릭 → 그 상대 1명으로 그룹).
                        let peer = row.entry.peer;
                        self.ctx_peer = Some(peer);
                        self.ctx_group = None;
                        self.selected.insert(peer);
                        let n = self.selected.len();
                        vec![
                            crate::controls::CtxItem::item("profile", "프로필 보기"),
                            crate::controls::CtxItem::item(
                                "g-create",
                                format!("그룹 만들기 ({n}명)"),
                            ),
                        ]
                    } else {
                        return;
                    };
                    self.ctx_menu.set_scale(self.scale);
                    self.ctx_menu.open_at(x, y, items, self.bounds, 8 * 15);
                    inv.push(self.bounds);
                    return;
                }
            }
        }
        match *ev {
            InputEvent::Key { key, .. } => {
                let vis = self.visible_rows().max(1);
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
            InputEvent::MouseDown { y, primary, .. } => {
                if let Some(i) = self.row_at(y) {
                    // ⌘/Ctrl+클릭 = 다중 선택 토글(M5-1 — 그룹 만들기·구성원 편집의 재료).
                    // 그룹 행은 선택 대상이 아니다(선택은 사람 단위).
                    if primary {
                        if let Some(row) = self.peer_at(i) {
                            let peer = row.entry.peer;
                            if !self.selected.remove(&peer) {
                                self.selected.insert(peer);
                            }
                            inv.push(self.row_rect(i));
                            self.last_click = None; // 선택 토글은 더블클릭으로 안 이어진다
                            return;
                        }
                    } else if !self.selected.is_empty() {
                        // 일반 클릭 = 선택 해제(파일 관리자 관례).
                        self.selected.clear();
                        inv.push(self.bounds);
                    }
                    self.move_caret(i, inv);
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
            InputEvent::MouseMove { y, .. } => {
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
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        ctx.select_font(FontSlot::PeerList, false);
        let vis = self.visible_rows();

        let rh = self.row_h();
        for (rel, i) in (self.top..self.total().min(self.top + vis + 1)).enumerate() {
            let r = Rect::new(
                self.bounds.x,
                self.bounds.y + i32::try_from(rel).unwrap_or(i32::MAX) * rh,
                self.bounds.w,
                rh,
            );
            // 행 배경 — 캐럿 > hover > 기본.
            let bg = if i == self.caret {
                theme.sel_bg
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
                ctx.text(name_x, name_y, r, &g.name, theme.text);
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
                // 실사진(M4-5 imgdec — 원형 마스크 완료본).
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
            // 세션 상태 점 — 아바타 우하단에 겹쳐(메신저 관례 · 색 의미는 기존 그대로).
            let dot_d = self.s(11);
            let dot = Rect::new(
                av.right() - dot_d + self.s(2),
                av.bottom() - dot_d + self.s(2),
                dot_d,
                dot_d,
            );
            let dot_color = match row.link {
                LinkState::Active => theme.ok,
                LinkState::Lost => theme.danger,
                LinkState::Idle => theme.text_dim,
                LinkState::Connecting => theme.accent, // 연결 중(M2-8)
            };
            ctx.fill_ellipse(
                Rect::new(dot.x - 1, dot.y - 1, dot.w + 2, dot.h + 2),
                theme.panel_bg,
            ); // 배경색 테두리 — 아바타와 분리
            ctx.fill_ellipse(dot, dot_color);

            // 1줄: **기본(발견) 이름 — 굵게**(사용자 확정 08-11).
            let name_x = av.right() + self.s(10);
            ctx.select_font(FontSlot::PeerList, true);
            let name_th = ctx.text_height();
            let name_y = r.y + self.s(8);
            ctx.text(name_x, name_y, r, row.entry.name.as_str(), theme.text);
            // 다중 경로 ×N(진단) — 이름 뒤.
            if row.entry.paths > 1 {
                let name_w = ctx.text_width(row.entry.name.as_str());
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

            // 신뢰 배지(오른쪽 정렬 라운드 칩) — 항상 표시 · 높이 고정(행이 커져도 칩은 그대로).
            ctx.select_font(FontSlot::PeerList, false);
            let (label, chip) = badge(row.trust, theme);
            let bw = ctx.text_width(label) + self.s(16);
            let chip_h = self.s(22);
            let chip_r = Rect::new(
                r.right() - bw - self.s(10),
                r.y + (rh - chip_h) / 2,
                bw,
                chip_h,
            );
            ctx.fill_round_rect(chip_r, chip_h / 2, chip);
            // 텍스트 상자 높이 실측으로 정확히 세로 중앙(고정 오프셋은 하단 여백이 커 보인다).
            let th = ctx.text_height();
            ctx.text(
                chip_r.x + self.s(8),
                chip_r.y + (chip_r.h - th) / 2,
                chip_r,
                label,
                theme.text,
            );

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
            unread: 0,
            last_read: None,
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
}
