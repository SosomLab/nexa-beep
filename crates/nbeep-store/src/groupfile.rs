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
/// v3(08-15) = v2 + 공유 그룹 **목록 고정(pinned) 꼬리**(사용자 요청 — 그룹방 핀).
/// v2(08-13) = v1(로컬 그룹) + 공유 그룹 레코드(M5-1g · ADR-0012).
/// 구버전 파일도 읽는다(없는 필드 = 기본값 — 전방·후방 관용).
const VER: u8 = 3;
const VER_V2: u8 = 2;
const VER_V1: u8 = 1;

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
}

impl FileGroupStore {
    /// 세그먼트를 열거나 새로 준비한다. 잠김이어도 저장소는 동작한다(메모리 전용).
    #[must_use]
    pub fn open(path: PathBuf, wrap_secret: [u8; 32]) -> (Self, GroupLoad) {
        match std::fs::read(&path) {
            Ok(bytes) => match Self::parse(&bytes, &wrap_secret) {
                Some((salt, master, store, shared)) => {
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
        });
        self.persist();
        Some(id)
    }

    /// 내 상태 전환(초대 수락 등). 반환 = 성공 여부.
    pub fn set_mine(&mut self, uid: GroupUid, mine: MineState) -> bool {
        let Some(s) = self.shared.iter_mut().find(|s| s.roster.uid == uid) else {
            return false;
        };
        s.mine = mine;
        self.persist();
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
    ) -> Option<([u8; SALT_LEN], [u8; 32], GroupStore, Vec<SharedGroup>)> {
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
        let (store, shared) = decode_body(ver, &body)?;
        Some((salt, master, store, shared))
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
        use std::io::Write as _;
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
        let body_pt = encode_body(&self.inner, &self.shared);
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
        if let Some(dir) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&out)?;
            f.sync_all()?;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
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
fn encode_body(store: &GroupStore, shared: &[SharedGroup]) -> Vec<u8> {
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
    }
    out
}

fn decode_body(ver: u8, bytes: &[u8]) -> Option<(GroupStore, Vec<SharedGroup>)> {
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
            shared.push(SharedGroup {
                local_id,
                roster,
                mine,
                pinned,
            });
        }
    }
    (p == bytes.len()).then(|| (GroupStore::from_export(next, list), shared))
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

    /// 코덱 왕복 + 꼬리 쓰레기 거부.
    #[test]
    fn store_codec_roundtrip() {
        let mut s = GroupStore::new();
        let a = s.create(name("a"));
        s.add_member(a, pid(1));
        s.add_member(a, pid(2));
        let _b = s.create(name("한국어 그룹"));
        let enc = encode_body(&s, &[]);
        let (back, shared) = decode_body(VER, &enc).unwrap();
        assert_eq!(back.export(), s.export());
        assert!(shared.is_empty());
        let mut bad = enc;
        bad.push(0);
        assert!(decode_body(VER, &bad).is_none());
    }
}
