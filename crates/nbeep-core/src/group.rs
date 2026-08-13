//! 그룹 — **로컬 개념**의 발견 사용자 묶음(FR-G-1~3 · [docs/10] DR-20 대비).
//!
//! ★ **그룹은 순수 로컬이다** — 서버도, 구성원 간 합의도 없다(FR-G-3). 내가 내 목적으로 묶는
//! 이름표일 뿐이라 상대는 자기가 어느 그룹에 속하는지 **알 필요도, 알 수도 없다**. 이것이
//! `UserId`(DR-20 — 상대가 믿어야 하는 서명된 주장)와 결정적으로 다른 점이다.
//!
//! 발신은 [`Group::recipients`]가 [`Recipients`](PeerId 집합)를 내주고, 그걸 기존
//! [`fanout`](crate::chat::fanout)에 넘긴다 — **1:1·그룹·다중 기기가 같은 발신 함수**(FR-G-6).
//! 여기엔 전송 코드가 없다(로컬 개념 · I/O 없음). 영속은 `nbeep-store`(M2-5) 소관.

use crate::identity::{PeerId, Recipients};
use crate::name::DisplayName;
use std::collections::BTreeSet;

/// 그룹 식별자 — 로컬 발급 순번(로컬 개념이라 전역 유일성·서명 불요).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(pub u32);

/// 한 그룹 — 이름 + 구성원(PeerId 집합).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// 그룹 표시 이름(무해화됨).
    pub name: DisplayName,
    members: BTreeSet<PeerId>,
}

impl Group {
    /// 구성원 목록(정렬 · 결정적).
    #[must_use]
    pub fn members(&self) -> Vec<PeerId> {
        self.members.iter().copied().collect()
    }

    /// 구성원 수.
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// 빈 그룹 여부.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// 이 그룹의 발신 대상([`Recipients`]) — 그대로 [`fanout`](crate::chat::fanout)에 넘긴다(FR-G-6).
    #[must_use]
    pub fn recipients(&self) -> Recipients {
        Recipients::from_peers(self.members())
    }

    /// 구성원 포함 여부.
    #[must_use]
    pub fn contains(&self, peer: PeerId) -> bool {
        self.members.contains(&peer)
    }
}

/// 그룹 저장(로컬) — 생성·개명·편입·제외·삭제(FR-G-1). 영속은 `nbeep-store`가 이걸 감싼다.
#[derive(Debug, Default)]
pub struct GroupStore {
    next: u32,
    groups: Vec<(GroupId, Group)>,
}

impl GroupStore {
    /// 빈 저장소.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 그룹 생성(빈 구성원). 로컬 발급 id 반환.
    pub fn create(&mut self, name: DisplayName) -> GroupId {
        let id = GroupId(self.next);
        self.next += 1;
        self.groups.push((
            id,
            Group {
                name,
                members: BTreeSet::new(),
            },
        ));
        id
    }

    /// 이름 변경. 없는 id면 `false`.
    pub fn rename(&mut self, id: GroupId, name: DisplayName) -> bool {
        match self.get_mut(id) {
            Some(g) => {
                g.name = name;
                true
            }
            None => false,
        }
    }

    /// 구성원 편입(중복은 무시). 없는 id면 `false`.
    pub fn add_member(&mut self, id: GroupId, peer: PeerId) -> bool {
        match self.get_mut(id) {
            Some(g) => {
                g.members.insert(peer);
                true
            }
            None => false,
        }
    }

    /// 구성원 제외. 없는 id·비구성원이면 `false`.
    pub fn remove_member(&mut self, id: GroupId, peer: PeerId) -> bool {
        self.get_mut(id).is_some_and(|g| g.members.remove(&peer))
    }

    /// 그룹 삭제. 없으면 `false`.
    pub fn delete(&mut self, id: GroupId) -> bool {
        let before = self.groups.len();
        self.groups.retain(|(gid, _)| *gid != id);
        self.groups.len() != before
    }

    /// 그룹 조회.
    #[must_use]
    pub fn get(&self, id: GroupId) -> Option<&Group> {
        self.groups
            .iter()
            .find(|(gid, _)| *gid == id)
            .map(|(_, g)| g)
    }

    fn get_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups
            .iter_mut()
            .find(|(gid, _)| *gid == id)
            .map(|(_, g)| g)
    }

    /// 전체 그룹(생성 순).
    #[must_use]
    pub fn list(&self) -> &[(GroupId, Group)] {
        &self.groups
    }

    /// 어떤 peer가 이탈(발견 목록에서 사라짐)해도 그룹 구성원은 **유지**한다
    /// (FR-D-12 고정 노드처럼 — 오프라인이어도 그룹에 남는다). 명시 제외만 지운다.
    /// (이 메서드는 그 불변을 문서화하는 no-op 자리표시 — 이탈 시 호출하지 않는다.)
    pub fn note_departed(&self, _peer: PeerId) {}

    // ── 영속 통로(M5-1 · FR-G-1 "재시작 후 유지") — 도메인 변경 없이 추가 API만 ──

    /// 영속 스냅샷 — `(다음 발급 번호, [(id, 이름, 구성원)])`. `nbeep-store`가 봉인한다.
    #[must_use]
    pub fn export(&self) -> (u32, Vec<(GroupId, DisplayName, Vec<PeerId>)>) {
        (
            self.next,
            self.groups
                .iter()
                .map(|(id, g)| (*id, g.name.clone(), g.members()))
                .collect(),
        )
    }

    /// 영속 복원 — [`GroupStore::export`]의 역. `next`가 기존 id보다 작으면
    /// **id 충돌을 막기 위해 최대 id+1로 승격**한다(손상·구버전 파일 관용).
    #[must_use]
    pub fn from_export(next: u32, list: Vec<(GroupId, DisplayName, Vec<PeerId>)>) -> Self {
        let max_next = list
            .iter()
            .map(|(id, _, _)| id.0.saturating_add(1))
            .max()
            .unwrap_or(0);
        Self {
            next: next.max(max_next),
            groups: list
                .into_iter()
                .map(|(id, name, members)| {
                    (
                        id,
                        Group {
                            name,
                            members: members.into_iter().collect(),
                        },
                    )
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn name(s: &str) -> DisplayName {
        DisplayName::parse(s).unwrap()
    }

    #[test]
    fn create_add_and_send_recipients() {
        let mut store = GroupStore::new();
        let g = store.create(name("개발팀"));
        store.add_member(g, pid(1));
        store.add_member(g, pid(2));
        store.add_member(g, pid(1)); // 중복 무시
        let group = store.get(g).unwrap();
        assert_eq!(group.len(), 2);
        // FR-G-6 — 그룹이 그대로 fanout Recipients가 된다.
        assert_eq!(group.recipients().peers(), &[pid(1), pid(2)]);
    }

    #[test]
    fn rename_and_delete() {
        let mut store = GroupStore::new();
        let g = store.create(name("임시"));
        assert!(store.rename(g, name("영업팀")));
        assert_eq!(store.get(g).unwrap().name.as_str(), "영업팀");
        assert!(store.delete(g));
        assert!(store.get(g).is_none());
        assert!(!store.delete(g), "이미 삭제됨");
    }

    #[test]
    fn remove_member() {
        let mut store = GroupStore::new();
        let g = store.create(name("g"));
        store.add_member(g, pid(1));
        assert!(store.remove_member(g, pid(1)));
        assert!(!store.remove_member(g, pid(1)), "이미 없음");
        assert!(store.get(g).unwrap().is_empty());
    }

    #[test]
    fn missing_id_ops_return_false() {
        let mut store = GroupStore::new();
        let ghost = GroupId(99);
        assert!(!store.rename(ghost, name("x")));
        assert!(!store.add_member(ghost, pid(1)));
        assert!(!store.remove_member(ghost, pid(1)));
        assert!(!store.delete(ghost));
    }

    #[test]
    fn ids_are_unique_and_stable_across_ops() {
        let mut store = GroupStore::new();
        let a = store.create(name("a"));
        let b = store.create(name("b"));
        assert_ne!(a, b);
        store.delete(a);
        let c = store.create(name("c"));
        assert_ne!(c, b, "삭제 후 새 id도 유일(순번 재사용 안 함)");
        assert_ne!(c, a);
    }

    #[test]
    fn empty_group_sends_to_nobody() {
        let mut store = GroupStore::new();
        let g = store.create(name("빈그룹"));
        assert!(store.get(g).unwrap().recipients().peers().is_empty());
    }
}
