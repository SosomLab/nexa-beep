//! 그룹 세그먼트 — 로컬 그룹의 암호화 영속(M5-1 · FR-G-1 "재시작 후 유지").
//!
//! [`trustfile`](crate::trustfile)과 같은 결이다 — **그룹 이름·구성원 목록은 곧
//! 인간관계 목록**이라(ADR-0005 §4-2의 핀 목록과 같은 민감도) 평문으로 두지 않는다.
//! 컨테이너 구조(키 계층·fail-closed·원자적 쓰기)도 동일하고 매직·레코드 코덱만 다르다.
//! (같은 구조의 2번째 사용 — 3번째 사용자(M2-5b 기록 저장)가 생기면 컨테이너를 추출한다
//! · [docs/13 §12] 중복 규칙.)
//!
//! ## 파일 포맷 v1 (`groups.seg`)
//!
//! ```text
//! [magic 4B "NBGS"][ver 1B]
//! [salt 16B]                                  ← 래핑 KDF 솔트(기기별)
//! [wrap_nonce 12B][wrapped_master 48B]        ← AEAD(마스터 32B) · aad = 앞 21B
//! [body_nonce 12B][body ct ..]                ← AEAD(레코드 평문) · aad = 앞 81B
//! ```
//!
//! 레코드 평문: `next u32 BE` · `count u32 BE` · 반복 `{ id u32 BE ·
//! name(len u16 BE · UTF-8) · member_count u32 BE · { peer 32B }* }`.
//! 저장은 **변경 즉시**(write-through) — 그룹 편집은 드물어 디바운스가 불필요하고,
//! 크래시에도 마지막 편집이 남는 쪽이 안전하다(trust.seg와 같은 판단).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use nbeep_core::group::{Group, GroupId, GroupStore};
use nbeep_core::sgroup::{GroupUid, Roster};
use nbeep_core::{DisplayName, PeerId};
use std::io;
use std::path::PathBuf;

const MAGIC: [u8; 4] = *b"NBGS";
/// v6(11-04 · GINV-1 S4) = v5 + 메모 `hard`(상대가 차단 = 재초대 불가) + **차단한 초대 목록**(내가 "다시 받지 않기"한 방 · 영속).
/// v5(11-04 · GINV-1 S2) = v4 + 구성원 메모 `last_ms`(마지막 초대 송신 시각 — 재초대 24h 제한 근거) · 거절 메모는 명부에서 빠져도 남는다.
/// v4(11-04 · GINV-1 S1) = v3 + 공유 그룹 **구성원 상태 꼬리**(소유자 로컬 메모 — 초대 대기·수락·거절 · 재시작을 넘긴다).
/// v3(08-15) = v2 + 공유 그룹 **목록 고정(pinned) 꼬리**(사용자 요청 — 그룹방 핀).
/// v2(08-13) = v1(로컬 그룹) + 공유 그룹 레코드(M5-1g · ADR-0012).
/// 구버전 파일도 읽는다(없는 필드 = 기본값 — 전방·후방 관용).
const VER: u8 = 6;
const VER_V2: u8 = 2;
const VER_V1: u8 = 1;

/// ★ 소유자가 기억하는 **구성원의 초대 상태**(GINV-1 S1 · 11-04). 명부(`Roster`)는 구성원만 말하고 상태는 말하지
/// 않는다 — "초대 대기/수락/거절"은 소유자 로컬 메모(봉투 원리: 와이어 무변경). 종전엔 메모리 `HashMap`이라
/// 소유자가 재시작하면 전원이 "초대 대기"로 보였다(상대가 재초대에 Accept를 되돌려야 치유).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberState {
    /// 초대장을 보냈고 아직 수락이 안 왔다.
    Invited,
    /// 수락(Accept) 확인.
    Joined,
    /// 거절(Decline) 확인 — S2에서 "재초대" UI의 근거.
    Declined,
}

impl MemberState {
    fn to_byte(self) -> u8 {
        match self {
            MemberState::Invited => 0,
            MemberState::Joined => 1,
            MemberState::Declined => 2,
        }
    }
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(MemberState::Invited),
            1 => Some(MemberState::Joined),
            2 => Some(MemberState::Declined),
            _ => None,
        }
    }
}

/// 구성원 한 명의 상태 메모(소유자 로컬).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberMeta {
    /// 상태.
    pub state: MemberState,
    /// 그 상태가 된 시각(unix ms · 초대 대기는 **첫 초대** 시각 — "n일 전" 표시 근거).
    pub at_ms: u64,
    /// 초대장을 보낸 횟수(재초대 제한 S2의 근거 · 포화).
    pub tries: u8,
    /// 마지막 초대 송신 시각(unix ms · v5 · S2 재초대 간격 `group.reinvite_hours`의 기준 — `at_ms`는 첫 초대 시각으로 유지).
    pub last_ms: u64,
    /// 상대가 "다시 받지 않기"로 거절(v6 · S4 · `DeclineV2{hard}`) — 재초대 불가 · `[재초대]` 비활성.
    pub hard: bool,
}

/// ★ 내가 차단한 초대(S4 · DR-30 · "이 그룹 초대 다시 받지 않기"): 그 방의 초대는 카드 없이 조용히 DeclineV2{hard}. 해제 =
/// 설정 › 고급 › 그룹 초대 › 차단한 초대 보기.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedInvite {
    /// 방.
    pub uid: GroupUid,
    /// 차단 때의 방 이름(표시용 · 무해화된 값).
    pub name: String,
    /// 소유자(표시·식별).
    pub owner: PeerId,
    /// 차단 시각(unix ms).
    pub at_ms: u64,
}

/// 공유 그룹에서의 내 상태(ADR-0012 G-4 — 초대 수락제).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MineState {
    /// 내가 소유자(생성자) — 명부 갱신 권한.
    Owner,
    /// 초대 수락 완료 — 방이 보인다.
    Joined,
    /// 초대만 받은 상태 — 수락 전(방 미표시 · 카드만).
    Invited,
}

impl MineState {
    fn to_byte(self) -> u8 {
        match self {
            MineState::Owner => 0,
            MineState::Joined => 1,
            MineState::Invited => 2,
        }
    }
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(MineState::Owner),
            1 => Some(MineState::Joined),
            2 => Some(MineState::Invited),
            _ => None,
        }
    }
}

/// 공유 그룹 레코드 — 명부 + 내 상태 + UI 키(로컬 id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedGroup {
    /// UI·스레드 키(로컬 발급 — 로컬 그룹과 같은 id 공간).
    pub local_id: GroupId,
    /// 명부(진실 — 소유자 세션에서 온 것만 갱신).
    pub roster: Roster,
    /// 내 상태.
    pub mine: MineState,
    /// 목록 상단 고정(08-15 사용자 요청 — 상대 고정과 같은 축).
    pub pinned: bool,
    /// ★ 구성원 상태 메모(v4 · 소유자 방에서만 의미 · 명부에 없는 상대는 갱신 때 걷어낸다).
    pub members_meta: Vec<(PeerId, MemberMeta)>,
}
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// 헤더(매직+버전+솔트) 길이 — 래핑 AEAD의 aad.
const HEAD: usize = 4 + 1 + SALT_LEN;
/// 래핑부까지 길이 — 본문 AEAD의 aad.
const WRAPPED_END: usize = HEAD + NONCE_LEN + 32 + TAG_LEN;

/// 열기 결과 — 앱이 상태바에 그대로 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupLoad {
    /// 파일 없음 — 새 저장소(첫 실행).
    Fresh,
    /// 정상 로드(그룹 수).
    Loaded(usize),
    /// ★ 손상·키 불일치 — **그룹 목록 잠김**(fail-closed · 파일 보존 · 영속 중단).
    Locked,
    /// ★ 잠긴 옛 세그먼트를 `<이름>.locked`로 **보관 이동**하고 새로 시작(08-19 —
    /// 잠긴 채 메모리 전용으로 돌면 이번 세션 그룹이 조용히 증발한다. 삭제 아님).
    Archived,
}

/// 파일 기반 그룹 저장 — [`GroupStore`]를 감싸 **변경 즉시** 암호화 저장한다.
/// 도메인 로직은 전부 위임(도메인 코드 변경 0 — trustfile과 같은 문법).
#[derive(Debug)]
pub struct FileGroupStore {
    inner: GroupStore,
    /// 공유 그룹(M5-1g) — uid가 진실 키 · local_id는 UI 키.
    shared: Vec<SharedGroup>,
    path: PathBuf,
    /// 래핑 키 원료(기기 신원 키 32B — trust.seg와 같은 원료).
    wrap_secret: [u8; 32],
    salt: [u8; SALT_LEN],
    /// 마스터 저장 키 — 무작위 생성·래핑 보관.
    master: [u8; 32],
    /// 잠김(fail-closed) — 참이면 메모리로만 동작하고 파일을 건드리지 않는다.
    locked: bool,
    /// 마지막 저장 실패 여부(IO — 다음 변경에서 재시도).
    write_failed: bool,
    /// 차단한 초대(v6 · S4).
    blocked: Vec<BlockedInvite>,
}

impl FileGroupStore {
    /// **부팅용** — 열되, 현 신원으로 못 여는 세그먼트는 `<이름>.locked(-N)`로
    /// **옆으로 보관**(삭제 아님)하고 새 저장소로 시작한다(08-19 실기 — tmp
    /// 청소가 옛 신원을 지운 뒤 옛 세그먼트가 영구 잠김 → 매 세션 그룹이 조용히
    /// 증발했다). ⚠ **임시 신원 부팅(키 파일 손상)에는 쓰지 말 것** — 진짜 주인의
    /// 세그먼트를 밀어내면 안 된다(그땐 [`Self::open`] = 보존·메모리 전용).
    #[must_use]
    pub fn open_or_archive(path: PathBuf, wrap_secret: [u8; 32]) -> (Self, GroupLoad) {
        let (store, load) = Self::open(path.clone(), wrap_secret);
        if load != GroupLoad::Locked {
            return (store, load);
        }
        let Some(arch) = crate::archive_name(&path) else {
            return (store, load); // 보관 이름 소진 — 현행 유지(보존 우선)
        };
        if std::fs::rename(&path, &arch).is_err() {
            return (store, load); // 못 옮기면 현행 유지(보존 우선)
        }
        let (fresh, _) = Self::open(path, wrap_secret);
        (fresh, GroupLoad::Archived)
    }

    /// 세그먼트를 열거나 새로 준비한다. 잠김이어도 저장소는 동작한다(메모리 전용).
    #[must_use]
    pub fn open(path: PathBuf, wrap_secret: [u8; 32]) -> (Self, GroupLoad) {
        match std::fs::read(&path) {
            Ok(bytes) => match Self::parse(&bytes, &wrap_secret) {
                Some((salt, master, store, shared, blocked)) => {
                    let n = store.list().len() + shared.len();
                    (
                        Self {
                            inner: store,
                            shared,
                            path,
                            wrap_secret,
                            salt,
                            master,
                            locked: false,
                            write_failed: false,
                            blocked,
                        },
                        GroupLoad::Loaded(n),
                    )
                }
                None => (
                    // ★ 잠김 — 파일은 보존(덮어쓰기 금지), 이번 실행은 메모리 전용.
                    Self {
                        inner: GroupStore::new(),
                        shared: Vec::new(),
                        path,
                        wrap_secret,
                        salt: [0; SALT_LEN],
                        master: [0; 32],
                        locked: true,
                        write_failed: false,
                        blocked: Vec::new(),
                    },
                    GroupLoad::Locked,
                ),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut salt = [0u8; SALT_LEN];
                let mut master = [0u8; 32];
                let ok = getrandom::getrandom(&mut salt).is_ok()
                    && getrandom::getrandom(&mut master).is_ok();
                (
                    Self {
                        inner: GroupStore::new(),
                        shared: Vec::new(),
                        path,
                        wrap_secret,
                        salt,
                        master,
                        locked: !ok,
                        write_failed: false,
                        blocked: Vec::new(),
                    },
                    if ok {
                        GroupLoad::Fresh
                    } else {
                        GroupLoad::Locked
                    },
                )
            }
            Err(_) => (
                Self {
                    inner: GroupStore::new(),
                    shared: Vec::new(),
                    path,
                    wrap_secret,
                    salt: [0; SALT_LEN],
                    master: [0; 32],
                    locked: true,
                    write_failed: false,
                    blocked: Vec::new(),
                },
                GroupLoad::Locked,
            ),
        }
    }

    /// 잠김 여부(fail-closed 표시용 — "그룹 목록 잠김").
    #[must_use]
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// 직전 저장 IO 실패 여부(다음 변경에서 자동 재시도).
    #[must_use]
    pub fn write_failed(&self) -> bool {
        self.write_failed
    }

    // ── 변경 위임 + 즉시 저장 ──

    /// [`GroupStore::create`] 위임 + 즉시 저장.
    pub fn create(&mut self, name: DisplayName) -> GroupId {
        let id = self.inner.create(name);
        self.persist();
        id
    }

    /// [`GroupStore::rename`] 위임 + 즉시 저장.
    pub fn rename(&mut self, id: GroupId, name: DisplayName) -> bool {
        let ok = self.inner.rename(id, name);
        if ok {
            self.persist();
        }
        ok
    }

    /// [`GroupStore::add_member`] 위임 + 즉시 저장.
    pub fn add_member(&mut self, id: GroupId, peer: PeerId) -> bool {
        let ok = self.inner.add_member(id, peer);
        if ok {
            self.persist();
        }
        ok
    }

    /// [`GroupStore::remove_member`] 위임 + 즉시 저장.
    pub fn remove_member(&mut self, id: GroupId, peer: PeerId) -> bool {
        let ok = self.inner.remove_member(id, peer);
        if ok {
            self.persist();
        }
        ok
    }

    /// [`GroupStore::delete`] 위임 + 즉시 저장.
    pub fn delete(&mut self, id: GroupId) -> bool {
        let ok = self.inner.delete(id);
        if ok {
            self.persist();
        }
        ok
    }

    // ── 조회 위임 ──

    /// [`GroupStore::get`] 위임.
    #[must_use]
    pub fn get(&self, id: GroupId) -> Option<&Group> {
        self.inner.get(id)
    }

    /// [`GroupStore::list`] 위임.
    #[must_use]
    pub fn list(&self) -> &[(GroupId, Group)] {
        self.inner.list()
    }

    // ── 공유 그룹(M5-1g · ADR-0012) ──

    /// 공유 그룹 등록/갱신 — uid가 있으면 **roster 수용 규칙**([`Roster::accepts_update`])
    /// 통과분만 교체, 없으면 새 레코드(로컬 id 발급). 반환 = UI 키.
    /// ⚠️ 호출자는 "이 roster가 소유자 세션에서 왔는가"를 먼저 확인해야 한다(ADR §2 G-1).
    pub fn upsert_shared(&mut self, roster: Roster, mine: MineState) -> Option<GroupId> {
        if let Some(s) = self.shared.iter_mut().find(|s| s.roster.uid == roster.uid) {
            if !s.roster.accepts_update(&roster) {
                return None; // 구버전·소유자 불일치 — 거부(fail-closed)
            }
            s.roster = roster;
            // 명부에서 빠진 상대의 메모는 걷어낸다 — 단 **거절 메모는 남긴다**(S2 · "거절됨 · [재초대]" 행의 근거 ·
            // 재초대하면 Invited로 바뀌며 새로 시작).
            let keep = s.roster.members.clone();
            s.members_meta
                .retain(|(p, m)| keep.contains(p) || m.state == MemberState::Declined);
            let id = s.local_id;
            self.persist();
            return Some(id);
        }
        let id = self.inner.alloc_id();
        self.shared.push(SharedGroup {
            pinned: false,
            local_id: id,
            roster,
            mine,
            members_meta: Vec::new(),
        });
        self.persist();
        Some(id)
    }

    /// ★ 구성원 상태 메모 갱신(GINV-1 S1) — 반환 = 바뀌었는가(바뀐 때만 저장).
    /// 규칙: `Invited`는 **수락을 내리지 않는다**(resync 재초대가 Joined를 지우지 않게) · 첫 초대 시각 보존 ·
    /// 횟수 +1(포화) · `Joined`/`Declined`는 상태·시각 교체. 명부에 없는 상대는 거부.
    pub fn note_member(
        &mut self,
        uid: GroupUid,
        peer: PeerId,
        state: MemberState,
        now_ms: u64,
    ) -> bool {
        let Some(s) = self.shared.iter_mut().find(|s| s.roster.uid == uid) else {
            return false;
        };
        if !s.roster.has_member(peer) {
            return false;
        }
        let changed = match s.members_meta.iter_mut().find(|(p, _)| *p == peer) {
            Some((_, m)) => match state {
                MemberState::Invited => {
                    if m.state == MemberState::Joined {
                        false
                    } else {
                        let was = *m;
                        m.state = MemberState::Invited;
                        m.tries = m.tries.saturating_add(1);
                        m.last_ms = now_ms;
                        if was.state != MemberState::Invited {
                            m.at_ms = now_ms;
                        }
                        *m != was
                    }
                }
                _ => {
                    let was = *m;
                    m.state = state;
                    m.at_ms = now_ms;
                    *m != was
                }
            },
            None => {
                s.members_meta.push((
                    peer,
                    MemberMeta {
                        state,
                        at_ms: now_ms,
                        tries: u8::from(state == MemberState::Invited),
                        last_ms: now_ms,
                        hard: false,
                    },
                ));
                true
            }
        };
        if changed {
            self.persist();
        }
        changed
    }

    /// 방의 구성원 메모 전부(명부 밖 거절 메모 포함 · S2 — 구성원 모달 꼬리 행·dump).
    #[must_use]
    pub fn members_meta_of(&self, uid: GroupUid) -> Vec<(PeerId, MemberMeta)> {
        self.shared
            .iter()
            .find(|s| s.roster.uid == uid)
            .map(|s| s.members_meta.clone())
            .unwrap_or_default()
    }

    /// 메모의 hard 표식(S4 · `DeclineV2{hard}` 수신) — 반환 = 바뀌었는가.
    pub fn set_member_hard(&mut self, uid: GroupUid, peer: PeerId, hard: bool) -> bool {
        let Some(s) = self.shared.iter_mut().find(|s| s.roster.uid == uid) else {
            return false;
        };
        let Some((_, m)) = s.members_meta.iter_mut().find(|(p, _)| *p == peer) else {
            return false;
        };
        if m.hard == hard {
            return false;
        }
        m.hard = hard;
        self.persist();
        true
    }

    /// 초대 차단(S4 · "다시 받지 않기") — 같은 방은 한 번만 · 저장.
    pub fn block_invite(&mut self, uid: GroupUid, name: &str, owner: PeerId, now_ms: u64) -> bool {
        if self.blocked.iter().any(|b| b.uid == uid) {
            return false;
        }
        self.blocked.push(BlockedInvite {
            uid,
            name: name.to_string(),
            owner,
            at_ms: now_ms,
        });
        self.persist();
        true
    }

    /// 차단 해제(설정 › 차단한 초대 보기) — 반환 = 있었는가.
    pub fn unblock_invite(&mut self, uid: GroupUid) -> bool {
        let before = self.blocked.len();
        self.blocked.retain(|b| b.uid != uid);
        let removed = self.blocked.len() != before;
        if removed {
            self.persist();
        }
        removed
    }

    /// 이 방 초대가 차단돼 있는가.
    #[must_use]
    pub fn invite_blocked(&self, uid: GroupUid) -> bool {
        self.blocked.iter().any(|b| b.uid == uid)
    }

    /// 차단한 초대 전부(설정 목록 · dump).
    #[must_use]
    pub fn blocked_invites(&self) -> &[BlockedInvite] {
        &self.blocked
    }

    /// 메모 삭제(거절 기록 지우기 · S2) — 반환 = 있었는가.
    pub fn forget_member(&mut self, uid: GroupUid, peer: PeerId) -> bool {
        let Some(s) = self.shared.iter_mut().find(|s| s.roster.uid == uid) else {
            return false;
        };
        let before = s.members_meta.len();
        s.members_meta.retain(|(p, _)| *p != peer);
        let removed = s.members_meta.len() != before;
        if removed {
            self.persist();
        }
        removed
    }

    /// 구성원 상태 메모 조회(없음 = 초대 기록 없음 — 호출자는 "초대 대기"로 본다).
    #[must_use]
    pub fn member_meta(&self, uid: GroupUid, peer: PeerId) -> Option<MemberMeta> {
        self.shared
            .iter()
            .find(|s| s.roster.uid == uid)
            .and_then(|s| s.members_meta.iter().find(|(p, _)| *p == peer))
            .map(|(_, m)| *m)
    }

    /// 내 상태 전환(초대 수락 등). 반환 = 성공 여부.
    pub fn set_mine(&mut self, uid: GroupUid, mine: MineState) -> bool {
        let Some(s) = self.shared.iter_mut().find(|s| s.roster.uid == uid) else {
            return false;
        };
        if s.mine != mine {
            s.mine = mine;
            self.persist(); // 같은 상태 = 무기록(RL-8 ⓑ)
        }
        true
    }

    /// 공유 그룹 제거(거절·탈퇴·소유자의 삭제 통지).
    pub fn remove_shared(&mut self, uid: GroupUid) -> bool {
        let before = self.shared.len();
        self.shared.retain(|s| s.roster.uid != uid);
        let removed = self.shared.len() != before;
        if removed {
            self.persist();
        }
        removed
    }

    /// uid로 조회.
    #[must_use]
    pub fn shared_by_uid(&self, uid: GroupUid) -> Option<&SharedGroup> {
        self.shared.iter().find(|s| s.roster.uid == uid)
    }

    /// UI 키(로컬 id)로 조회.
    #[must_use]
    pub fn shared_by_id(&self, id: GroupId) -> Option<&SharedGroup> {
        self.shared.iter().find(|s| s.local_id == id)
    }

    /// 공유 그룹 목록 고정 지정(08-15 · UI 키 기준) — 바뀌면 즉시 저장.
    pub fn set_shared_pinned(&mut self, id: GroupId, pinned: bool) -> bool {
        let Some(s) = self.shared.iter_mut().find(|s| s.local_id == id) else {
            return false;
        };
        if s.pinned == pinned {
            return false;
        }
        s.pinned = pinned;
        self.persist();
        true
    }

    /// 전체 공유 그룹.
    #[must_use]
    pub fn shared_list(&self) -> &[SharedGroup] {
        &self.shared
    }

    // ── 직렬화·암호화(트러스트 세그먼트와 같은 컨테이너 · 매직만 다름) ──

    fn wrap_key(salt: &[u8; SALT_LEN], secret: &[u8; 32]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"nexa-beep/m5-1/group-wrap-v1");
        h.update(salt);
        h.update(secret);
        h.finalize().into()
    }

    #[allow(clippy::type_complexity)] // 내부 파서 반환 — 호출자 1곳
    fn parse(
        bytes: &[u8],
        secret: &[u8; 32],
    ) -> Option<(
        [u8; SALT_LEN],
        [u8; 32],
        GroupStore,
        Vec<SharedGroup>,
        Vec<BlockedInvite>,
    )> {
        if bytes.len() < WRAPPED_END + NONCE_LEN + TAG_LEN || bytes[..4] != MAGIC {
            return None;
        }
        let ver = bytes[4];
        if !matches!(ver, VER | VER_V2 | VER_V1) {
            return None;
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&bytes[5..HEAD]);
        let wrap = ChaCha20Poly1305::new(&Self::wrap_key(&salt, secret).into());
        let master_pt = wrap
            .decrypt(
                Nonce::from_slice(&bytes[HEAD..HEAD + NONCE_LEN]),
                Payload {
                    msg: &bytes[HEAD + NONCE_LEN..WRAPPED_END],
                    aad: &bytes[..HEAD],
                },
            )
            .ok()?;
        let master: [u8; 32] = master_pt.try_into().ok()?;
        let body = ChaCha20Poly1305::new(&master.into())
            .decrypt(
                Nonce::from_slice(&bytes[WRAPPED_END..WRAPPED_END + NONCE_LEN]),
                Payload {
                    msg: &bytes[WRAPPED_END + NONCE_LEN..],
                    aad: &bytes[..WRAPPED_END],
                },
            )
            .ok()?;
        let (store, shared, blocked) = decode_body(ver, &body)?;
        Some((salt, master, store, shared, blocked))
    }

    /// 스냅샷 저장 — 실패는 치명적이지 않다(플래그만 · 다음 변경에서 재시도).
    /// 잠김이면 **아무것도 쓰지 않는다**(손상 원본 보존).
    fn persist(&mut self) {
        if self.locked {
            return;
        }
        self.write_failed = self.write_atomic().is_err();
    }

    fn write_atomic(&self) -> io::Result<()> {
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(&MAGIC);
        out.push(VER);
        out.extend_from_slice(&self.salt);
        let rng_err = || io::Error::other("OS 난수 실패");
        let aead_err = || io::Error::other("AEAD 봉인 실패");
        let wrap = ChaCha20Poly1305::new(&Self::wrap_key(&self.salt, &self.wrap_secret).into());
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).map_err(|_| rng_err())?;
        let wrapped = wrap
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &self.master,
                    aad: &out[..HEAD],
                },
            )
            .map_err(|_| aead_err())?;
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&wrapped);
        let body_pt = encode_body(&self.inner, &self.shared, &self.blocked);
        let mut bnonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut bnonce).map_err(|_| rng_err())?;
        let body = ChaCha20Poly1305::new(&self.master.into())
            .encrypt(
                Nonce::from_slice(&bnonce),
                Payload {
                    msg: &body_pt[..],
                    aad: &out[..WRAPPED_END],
                },
            )
            .map_err(|_| aead_err())?;
        out.extend_from_slice(&bnonce);
        out.extend_from_slice(&body);
        // 소유자 전용(0600) 원자적 쓰기 — 한 벌(`privfile` · 09-05 clip A-1 계열).
        crate::privfile::write_atomic(&self.path, &out)
    }
}

// ── 레코드 코덱(평문 — AEAD 안쪽) ──

fn encode_store(store: &GroupStore) -> Vec<u8> {
    let (next, list) = store.export();
    let mut out = Vec::with_capacity(64 * list.len() + 8);
    out.extend_from_slice(&next.to_be_bytes());
    out.extend_from_slice(&u32::try_from(list.len()).unwrap_or(u32::MAX).to_be_bytes());
    for (id, name, members) in &list {
        out.extend_from_slice(&id.0.to_be_bytes());
        let b = name.as_str().as_bytes();
        let len = u16::try_from(b.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&b[..usize::from(len)]);
        out.extend_from_slice(
            &u32::try_from(members.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for m in members {
            out.extend_from_slice(m.as_bytes());
        }
    }
    out
}

/// v2 본문 = 로컬부(기존 v1과 동일) + 공유부(`count u32` · 반복 `{ local_id u32 ·
/// mine u8 · roster_len u32 · roster }`).
fn encode_body(store: &GroupStore, shared: &[SharedGroup], blocked: &[BlockedInvite]) -> Vec<u8> {
    let mut out = encode_store(store);
    out.extend_from_slice(
        &u32::try_from(shared.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for s in shared {
        out.extend_from_slice(&s.local_id.0.to_be_bytes());
        out.push(s.mine.to_byte());
        out.push(u8::from(s.pinned)); // v3 — 목록 고정
        let rb = s.roster.encode();
        out.extend_from_slice(&u32::try_from(rb.len()).unwrap_or(u32::MAX).to_be_bytes());
        out.extend_from_slice(&rb);
        // v4 — 구성원 상태 메모(count u32 · {peer 32 · state u8 · at_ms u64 · tries u8}).
        out.extend_from_slice(
            &u32::try_from(s.members_meta.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for (p, m) in &s.members_meta {
            out.extend_from_slice(p.as_bytes());
            out.push(m.state.to_byte());
            out.extend_from_slice(&m.at_ms.to_be_bytes());
            out.push(m.tries);
            out.extend_from_slice(&m.last_ms.to_be_bytes()); // v5
            out.push(u8::from(m.hard)); // v6
        }
    }
    // v6 — 차단한 초대 목록(count u32 · {uid 32 · owner 32 · at_ms u64 · name_len u16 · name}).
    out.extend_from_slice(
        &u32::try_from(blocked.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for b in blocked {
        out.extend_from_slice(&b.uid.0);
        out.extend_from_slice(b.owner.as_bytes());
        out.extend_from_slice(&b.at_ms.to_be_bytes());
        let nb = b.name.as_bytes();
        let len = u16::try_from(nb.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&nb[..usize::from(len)]);
    }
    out
}

type Body = (GroupStore, Vec<SharedGroup>, Vec<BlockedInvite>);

fn decode_body(ver: u8, bytes: &[u8]) -> Option<Body> {
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let s = bytes.get(*p..*p + n)?;
        *p += n;
        Some(s)
    };
    let next = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
    let count = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
    let mut list = Vec::with_capacity(count.min(4096) as usize);
    for _ in 0..count {
        let id = GroupId(u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?));
        let len = usize::from(u16::from_be_bytes(take(&mut p, 2)?.try_into().ok()?));
        let name = DisplayName::parse(std::str::from_utf8(take(&mut p, len)?).ok()?).ok()?;
        let mc = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
        let mut members = Vec::with_capacity(mc.min(4096) as usize);
        for _ in 0..mc {
            members.push(PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?));
        }
        list.push((id, name, members));
    }
    let mut shared = Vec::new();
    if ver >= 2 {
        let sc = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
        if sc > 4096 {
            return None;
        }
        for _ in 0..sc {
            let local_id = GroupId(u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?));
            let mine = MineState::from_byte(take(&mut p, 1)?[0])?;
            // v3 — 목록 고정(v2 파일은 필드 없음 = false 관용).
            let pinned = if ver >= 3 {
                take(&mut p, 1)?[0] != 0
            } else {
                false
            };
            let rlen = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?) as usize;
            let rb = take(&mut p, rlen)?;
            let (roster, used) = Roster::decode(rb)?;
            if used != rlen {
                return None;
            }
            // v4 — 구성원 상태 메모(v3 이하 = 없음 = 빈 목록 관용).
            let mut members_meta = Vec::new();
            if ver >= 4 {
                let mc = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
                if mc > 4096 {
                    return None;
                }
                for _ in 0..mc {
                    let peer = PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?);
                    let state = MemberState::from_byte(take(&mut p, 1)?[0])?;
                    let at_ms = u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?);
                    let tries = take(&mut p, 1)?[0];
                    // v5 — 마지막 초대 시각(v4 = 첫 초대 시각으로 관용).
                    let last_ms = if ver >= 5 {
                        u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?)
                    } else {
                        at_ms
                    };
                    let hard = ver >= 6 && take(&mut p, 1)?[0] != 0;
                    members_meta.push((
                        peer,
                        MemberMeta {
                            state,
                            at_ms,
                            tries,
                            last_ms,
                            hard,
                        },
                    ));
                }
            }
            shared.push(SharedGroup {
                local_id,
                roster,
                mine,
                pinned,
                members_meta,
            });
        }
    }
    // v6 — 차단한 초대 목록(v5 이하 = 없음).
    let mut blocked = Vec::new();
    if ver >= 6 {
        let bc = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
        if bc > 4096 {
            return None;
        }
        for _ in 0..bc {
            let uid = GroupUid(take(&mut p, 32)?.try_into().ok()?);
            let owner = PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?);
            let at_ms = u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?);
            let len = usize::from(u16::from_be_bytes(take(&mut p, 2)?.try_into().ok()?));
            let name = std::str::from_utf8(take(&mut p, len)?).ok()?.to_string();
            blocked.push(BlockedInvite {
                uid,
                owner,
                at_ms,
                name,
            });
        }
    }
    (p == bytes.len()).then(|| (GroupStore::from_export(next, list), shared, blocked))
}

impl Drop for FileGroupStore {
    fn drop(&mut self) {
        // R-16 zeroize(11-06).
        crate::sealed::wipe(&mut self.wrap_secret);
        crate::sealed::wipe(&mut self.master);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-groupseg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("groups.seg")
    }
    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn name(s: &str) -> DisplayName {
        DisplayName::parse(s).unwrap()
    }

    /// FR-G-1의 핵심 계약 — 그룹 생성·구성원 편집 → 재시작(재오픈) → 그대로.
    #[test]
    fn groups_survive_reopen() {
        let path = tmp("survive");
        let secret = [7u8; 32];
        let gid = {
            let (mut gs, load) = FileGroupStore::open(path.clone(), secret);
            assert_eq!(load, GroupLoad::Fresh);
            let g = gs.create(name("개발팀"));
            gs.add_member(g, pid(1));
            gs.add_member(g, pid(2));
            gs.remove_member(g, pid(2));
            let _trash = gs.create(name("임시"));
            assert!(gs.delete(_trash));
            assert!(!gs.write_failed(), "저장 성공");
            g
        };
        let (mut gs, load) = FileGroupStore::open(path, secret);
        assert_eq!(load, GroupLoad::Loaded(1));
        let g = gs.get(gid).unwrap();
        assert_eq!(g.name.as_str(), "개발팀");
        assert_eq!(g.members(), vec![pid(1)]);
        // 재오픈 후 새 그룹 id가 기존과 충돌하지 않는다(next 승계).
        let g2 = gs.create(name("영업팀"));
        assert_ne!(g2, gid);
    }

    /// 로컬 승격 마이그레이션(G4 · 08-15) — 로컬 그룹을 공유 그룹으로 올리고 로컬을
    /// 지운 결과가 **재시작을 살아남는다**(승격이 반쪽만 영속되면 방이 두 개가 된다).
    #[test]
    fn local_promotion_survives_reopen() {
        use nbeep_core::sgroup::{GroupUid, Roster};
        let path = tmp("promote");
        let secret = [7u8; 32];
        {
            let (mut gs, _) = FileGroupStore::open(path.clone(), secret);
            let g = gs.create(name("옛동보팀"));
            gs.add_member(g, pid(1));
            // 승격 = 같은 이름·구성원의 공유 그룹 upsert + 로컬 삭제(app 부팅 경로).
            let roster = Roster {
                uid: GroupUid([3u8; 32]),
                name: name("옛동보팀"),
                owner: pid(9),
                members: vec![pid(1), pid(9)],
                version: 1,
                member_invite: true,
            };
            assert!(gs.upsert_shared(roster, MineState::Owner).is_some());
            assert!(gs.delete(g));
        }
        let (gs, load) = FileGroupStore::open(path, secret);
        assert_eq!(load, GroupLoad::Loaded(1), "공유 1(로컬+공유 합산 카운트)");
        assert!(gs.list().is_empty(), "로컬은 사라졌다");
        let s = &gs.shared_list()[0];
        assert_eq!(s.roster.name.as_str(), "옛동보팀");
        assert_eq!(s.mine, MineState::Owner);
        assert_eq!(s.roster.members, vec![pid(1), pid(9)]);
    }

    /// fail-closed — 다른 기기 키로는 열리지 않고 잠김 · 잠긴 동안 파일 불변.
    #[test]
    fn wrong_secret_locks_and_preserves_file() {
        let path = tmp("lock");
        {
            let (mut gs, _) = FileGroupStore::open(path.clone(), [1u8; 32]);
            let g = gs.create(name("g"));
            gs.add_member(g, pid(1));
        }
        let before = std::fs::read(&path).unwrap();
        let (mut gs, load) = FileGroupStore::open(path.clone(), [2u8; 32]);
        assert_eq!(load, GroupLoad::Locked);
        assert!(gs.locked());
        assert!(gs.list().is_empty(), "메모리 전용 빈 상태");
        let _ = gs.create(name("x")); // 메모리로는 동작
        assert_eq!(std::fs::read(&path).unwrap(), before, "원본 무손상");
    }

    /// 파일은 그룹 이름·구성원 PeerId를 평문으로 노출하지 않는다(인간관계 목록 보호).
    #[test]
    fn file_does_not_leak_names_or_peers() {
        let path = tmp("leak");
        let (mut gs, _) = FileGroupStore::open(path.clone(), [1u8; 32]);
        let g = gs.create(name("비밀 프로젝트팀"));
        gs.add_member(g, pid(9));
        let raw = std::fs::read(&path).unwrap();
        let n = "비밀 프로젝트팀".as_bytes();
        assert!(!raw.windows(n.len()).any(|w| w == n), "이름 평문 노출 금지");
        assert!(
            !raw.windows(32).any(|w| w == pid(9).as_bytes()),
            "PeerId 평문 노출 금지"
        );
    }

    /// 공유 그룹(M5-1g) — 재오픈 유지 · roster 수용 규칙 · 상태 전환.
    #[test]
    fn shared_groups_survive_and_enforce_roster_rules() {
        use nbeep_core::sgroup::{GroupUid, Roster};
        let path = tmp("shared");
        let secret = [7u8; 32];
        let uid = GroupUid([9u8; 32]);
        let roster = |v: u64| Roster {
            uid,
            name: name("개발방"),
            owner: pid(1),
            members: vec![pid(1), pid(2)],
            version: v,
            member_invite: true,
        };
        let lid = {
            let (mut gs, _) = FileGroupStore::open(path.clone(), secret);
            let lid = gs.upsert_shared(roster(1), MineState::Invited).unwrap();
            assert!(gs.set_mine(uid, MineState::Joined));
            // 구버전 roster는 거부(fail-closed).
            assert!(gs.upsert_shared(roster(1), MineState::Joined).is_none());
            // 소유자 바꿔치기 거부.
            let mut hijack = roster(5);
            hijack.owner = pid(8);
            assert!(gs.upsert_shared(hijack, MineState::Joined).is_none());
            // 정상 갱신은 같은 로컬 id 유지.
            assert_eq!(gs.upsert_shared(roster(2), MineState::Joined), Some(lid));
            lid
        };
        let (mut gs, load) = FileGroupStore::open(path, secret);
        assert_eq!(load, GroupLoad::Loaded(1));
        let s = gs.shared_by_id(lid).unwrap();
        assert_eq!(s.roster.version, 2);
        assert_eq!(s.mine, MineState::Joined, "수락 상태 유지");
        assert_eq!(s.roster.name.as_str(), "개발방");
        // 로컬 id 공간 공유 — 새 로컬 그룹 id와 충돌하지 않는다.
        let g2 = gs.create(name("로컬"));
        assert_ne!(g2, lid);
        assert!(gs.remove_shared(uid));
        assert!(gs.shared_by_uid(uid).is_none());
    }

    /// ★ GINV-1 S1(11-04): 구성원 상태 메모가 재시작을 넘기고 · 재초대(Invited)가 수락을 내리지 않고 · 명부에서
    /// 빠진 상대의 메모는 갱신 때 걷히고 · 명부 밖 상대는 거부된다.
    #[test]
    fn member_meta_persists_and_never_downgrades_joined() {
        use nbeep_core::sgroup::{GroupUid, Roster};
        let path = tmp("meta");
        let secret = [3u8; 32];
        let uid = GroupUid([4u8; 32]);
        let roster = |v: u64, members: Vec<PeerId>| Roster {
            uid,
            name: name("방"),
            owner: pid(1),
            members,
            version: v,
            member_invite: true,
        };
        {
            let (mut gs, _) = FileGroupStore::open(path.clone(), secret);
            gs.upsert_shared(roster(1, vec![pid(1), pid(2), pid(3)]), MineState::Owner)
                .unwrap();
            assert!(gs.note_member(uid, pid(2), MemberState::Invited, 1_000));
            assert!(gs.note_member(uid, pid(3), MemberState::Invited, 1_000));
            // 재초대 = 횟수만 오르고 첫 시각 유지.
            assert!(gs.note_member(uid, pid(2), MemberState::Invited, 5_000));
            let m = gs.member_meta(uid, pid(2)).unwrap();
            assert_eq!(
                (m.state, m.at_ms, m.tries),
                (MemberState::Invited, 1_000, 2)
            );
            // 수락.
            assert!(gs.note_member(uid, pid(3), MemberState::Joined, 7_000));
            // 수락 뒤 resync 재초대는 무변경(내리지 않는다).
            assert!(!gs.note_member(uid, pid(3), MemberState::Invited, 9_000));
            // 명부 밖 상대는 거부.
            assert!(!gs.note_member(uid, pid(9), MemberState::Invited, 1));
        }
        let (mut gs, _) = FileGroupStore::open(path, secret);
        assert_eq!(
            gs.member_meta(uid, pid(3)),
            Some(MemberMeta {
                state: MemberState::Joined,
                at_ms: 7_000,
                tries: 1,
                last_ms: 1_000,
                hard: false
            }),
            "재시작을 넘긴다"
        );
        assert_eq!(
            gs.member_meta(uid, pid(2)).map(|m| (m.tries, m.last_ms)),
            Some((2, 5_000))
        );
        // 명부 갱신으로 pid(2)가 빠지면 메모도 걷힌다.
        gs.upsert_shared(roster(2, vec![pid(1), pid(3)]), MineState::Owner)
            .unwrap();
        assert_eq!(gs.member_meta(uid, pid(2)), None);
        assert!(gs.member_meta(uid, pid(3)).is_some());
        // ★ S2: 거절 메모는 명부에서 빠져도 남는다(재초대 행의 근거) · 지우기로만 사라진다.
        gs.upsert_shared(roster(3, vec![pid(1), pid(3), pid(4)]), MineState::Owner)
            .unwrap();
        assert!(gs.note_member(uid, pid(4), MemberState::Invited, 10_000));
        assert!(gs.note_member(uid, pid(4), MemberState::Declined, 11_000));
        gs.upsert_shared(roster(4, vec![pid(1), pid(3)]), MineState::Owner)
            .unwrap();
        assert_eq!(
            gs.member_meta(uid, pid(4)).map(|m| m.state),
            Some(MemberState::Declined)
        );
        assert_eq!(gs.members_meta_of(uid).len(), 2);
        assert!(gs.forget_member(uid, pid(4)));
        assert_eq!(gs.member_meta(uid, pid(4)), None);
    }

    /// ★ S4(11-04): 메모 hard 표식과 차단한 초대 목록이 재시작을 넘긴다 · 중복 차단 없음 · 해제.
    #[test]
    fn hard_decline_and_blocked_invites_persist() {
        use nbeep_core::sgroup::{GroupUid, Roster};
        let path = tmp("hard");
        let secret = [5u8; 32];
        let uid = GroupUid([6u8; 32]);
        let other = GroupUid([7u8; 32]);
        {
            let (mut gs, _) = FileGroupStore::open(path.clone(), secret);
            gs.upsert_shared(
                Roster {
                    uid,
                    name: name("방"),
                    owner: pid(1),
                    members: vec![pid(1), pid(2)],
                    version: 1,
                    member_invite: true,
                },
                MineState::Owner,
            )
            .unwrap();
            assert!(gs.note_member(uid, pid(2), MemberState::Declined, 1_000));
            assert!(gs.set_member_hard(uid, pid(2), true));
            assert!(!gs.set_member_hard(uid, pid(2), true), "같은 값 = 무변경");
            assert!(gs.block_invite(other, "남의방", pid(9), 2_000));
            assert!(
                !gs.block_invite(other, "남의방", pid(9), 3_000),
                "중복 차단 없음"
            );
            assert!(gs.invite_blocked(other));
        }
        let (mut gs, _) = FileGroupStore::open(path, secret);
        assert_eq!(gs.member_meta(uid, pid(2)).map(|m| m.hard), Some(true));
        assert_eq!(gs.blocked_invites().len(), 1);
        assert_eq!(gs.blocked_invites()[0].name, "남의방");
        assert!(gs.unblock_invite(other));
        assert!(!gs.invite_blocked(other));
        assert!(!gs.unblock_invite(other));
    }

    /// ★ 잠긴 세그먼트 보관 이동(08-19) — 다른 신원 키의 파일을 `<이름>.locked`로
    /// 밀어두고 새로 시작하며, **새 저장소는 영속**한다(종전 잠김 = 메모리 전용으로
    /// 재시작마다 그룹이 증발하던 것). 원본은 보존(삭제 아님).
    #[test]
    fn locked_segment_is_archived_and_fresh_store_persists() {
        let path = tmp("archive");
        let old_key = [1u8; 32];
        let new_key = [2u8; 32];
        {
            let (mut gs, _) = FileGroupStore::open(path.clone(), old_key);
            let _ = gs.create(name("옛신원방"));
        }
        // 종전 open = 잠김(보존·메모리 전용).
        let (_, load) = FileGroupStore::open(path.clone(), new_key);
        assert_eq!(load, GroupLoad::Locked);
        // open_or_archive = 보관 이동 + 새 시작.
        let (mut gs, load) = FileGroupStore::open_or_archive(path.clone(), new_key);
        assert_eq!(load, GroupLoad::Archived);
        let arch = PathBuf::from(format!("{}.locked", path.display()));
        assert!(arch.exists(), "원본은 .locked로 보존");
        // 새 저장소가 실제로 영속하는가 — 쓰고 재오픈.
        let _g = gs.create(name("새신원방"));
        let (gs2, load2) = FileGroupStore::open(path.clone(), new_key);
        assert_eq!(load2, GroupLoad::Loaded(1), "새 키로 영속 재개");
        assert_eq!(gs2.list().len(), 1);
        // 정상 파일에 open_or_archive = 그대로 로드(보관 없음).
        let (_, load3) = FileGroupStore::open_or_archive(path, new_key);
        assert_eq!(load3, GroupLoad::Loaded(1));
    }

    /// 코덱 왕복 + 꼬리 쓰레기 거부.
    #[test]
    fn store_codec_roundtrip() {
        let mut s = GroupStore::new();
        let a = s.create(name("a"));
        s.add_member(a, pid(1));
        s.add_member(a, pid(2));
        let _b = s.create(name("한국어 그룹"));
        let enc = encode_body(&s, &[], &[]);
        let (back, shared, blocked) = decode_body(VER, &enc).unwrap();
        assert_eq!(back.export(), s.export());
        assert!(shared.is_empty());
        assert!(blocked.is_empty());
        let mut bad = enc;
        bad.push(0);
        assert!(decode_body(VER, &bad).is_none());
    }
}
