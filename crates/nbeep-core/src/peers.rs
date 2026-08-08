//! 피어 목록 — **지문 병합·이탈 판정**(M1-5 · FR-D-6 · FR-D-8).
//!
//! 발견(전송 어댑터)이 관측을 흘려 넣고, 이 테이블이 **목록의 진실**을 만든다:
//!
//! - **병합의 열쇠는 오직 [`PeerId`]** — 같은 노드가 여러 인터페이스·여러 발견 단계(S1~S6)로
//!   보여도 항목은 하나다(FR-D-6). 다른 `PeerId`는 이름이 같아도 **절대 병합하지 않는다**
//!   (다른 PC — DR-18. 병합의 근거는 언제나 암호학적 증거다).
//! - **이탈은 이중 판정**(FR-D-8) — ① goodbye는 **관측 경로(source) 단위**로 지운다: 한 인터페이스가
//!   내려가도 다른 경로가 살아 있으면 목록에 남는다. 마지막 경로의 goodbye만 이탈이다.
//!   ② 무응답 타임아웃은 경로와 무관하게 **마지막 관측 시각** 기준이다.
//! - **타임아웃 수치는 주입**한다 — 발견 타이밍 6종은 D-8 실기 실측 후 확정된다([08 §8]).
//!   여기 하드코딩하면 실측이 무의미해진다(네트워크는 추정 금지).
//!
//! 시간은 [`MonoInstant`]로 받는다(Clock 포트 — 테스트는 `FixedClock`).
//! 호스트명 기본 이름(FR-D-9)은 힌트를 만드는 쪽(전송 어댑터) 소관이다 — 여기 오는 이름은
//! 이미 무해화된 [`DisplayName`]이며, **미검증 주장**이다(신뢰는 [`crate::trust`]).

use crate::identity::PeerId;
use crate::name::DisplayName;
use crate::ports::MonoInstant;
use std::collections::{BTreeSet, HashMap};

/// 관측 경로 토큰 — "어느 인터페이스/발견 단계에서 봤나". 내용은 전송 어댑터만 안다(불투명).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u32);

/// 이탈 사유(FR-D-8 이중 판정).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepartReason {
    /// 명시적 goodbye(마지막 경로까지 닫힘).
    Goodbye,
    /// 무응답 타임아웃.
    Timeout,
}

/// 목록 변경 이벤트 — UI가 구독한다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerEvent {
    /// 새 항목 등장.
    Appeared(PeerId),
    /// 표시 이름 변경(이름은 미검증 힌트 — 신뢰 경고는 [`crate::trust`] 몫).
    Renamed(PeerId),
    /// 목록에서 이탈.
    Departed(PeerId, DepartReason),
}

/// 목록 항목(읽기 뷰).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerEntry {
    /// 상대 식별자(미검증 — 신원 확정은 세션).
    pub peer: PeerId,
    /// 무해화된 표시 이름.
    pub name: DisplayName,
    /// 현재 살아 있는 관측 경로 수(다중 경로 병합의 가시화 — 진단용).
    pub paths: usize,
}

#[derive(Debug)]
struct Entry {
    name: DisplayName,
    sources: BTreeSet<SourceId>,
    last_seen: MonoInstant,
}

/// 발견 관측을 목록으로 접는 테이블.
#[derive(Debug)]
pub struct PeerTable {
    /// 무응답 이탈 임계(ms) — **D-8 실측 후 확정값을 주입**(하드코딩 금지).
    timeout_ms: u32,
    entries: HashMap<PeerId, Entry>,
}

impl PeerTable {
    /// `timeout_ms` 동안 관측이 없으면 이탈로 판정하는 테이블.
    #[must_use]
    pub fn new(timeout_ms: u32) -> Self {
        Self {
            timeout_ms,
            entries: HashMap::new(),
        }
    }

    /// 발견 관측 하나를 접는다 — 새 항목이면 `Appeared`, 이름이 바뀌었으면 `Renamed`,
    /// 이미 알던 관측(경로 추가·생존 신호)이면 `None`.
    pub fn observe(
        &mut self,
        peer: PeerId,
        name: DisplayName,
        source: SourceId,
        now: MonoInstant,
    ) -> Option<PeerEvent> {
        if let Some(e) = self.entries.get_mut(&peer) {
            e.sources.insert(source);
            e.last_seen = now;
            if e.name != name {
                e.name = name;
                return Some(PeerEvent::Renamed(peer));
            }
            return None;
        }
        self.entries.insert(
            peer,
            Entry {
                name,
                sources: BTreeSet::from([source]),
                last_seen: now,
            },
        );
        Some(PeerEvent::Appeared(peer))
    }

    /// 한 경로의 goodbye — **마지막 경로였을 때만** 이탈이다(다중 경로 병합의 이탈 면).
    pub fn goodbye(&mut self, peer: PeerId, source: SourceId) -> Option<PeerEvent> {
        let e = self.entries.get_mut(&peer)?;
        e.sources.remove(&source);
        if e.sources.is_empty() {
            self.entries.remove(&peer);
            return Some(PeerEvent::Departed(peer, DepartReason::Goodbye));
        }
        None
    }

    /// 무응답 판정 — `now` 기준 임계를 넘긴 항목을 이탈시킨다. 주기 호출(발견 틱).
    pub fn sweep(&mut self, now: MonoInstant) -> Vec<PeerEvent> {
        let timeout = self.timeout_ms;
        let expired: Vec<PeerId> = self
            .entries
            .iter()
            .filter(|(_, e)| now.saturating_ms_since(e.last_seen) >= timeout)
            .map(|(&p, _)| p)
            .collect();
        expired
            .into_iter()
            .map(|p| {
                self.entries.remove(&p);
                PeerEvent::Departed(p, DepartReason::Timeout)
            })
            .collect()
    }

    /// 현재 목록(표시 이름 순 — 결정적).
    #[must_use]
    pub fn list(&self) -> Vec<PeerEntry> {
        let mut v: Vec<PeerEntry> = self
            .entries
            .iter()
            .map(|(&peer, e)| PeerEntry {
                peer,
                name: e.name.clone(),
                paths: e.sources.len(),
            })
            .collect();
        v.sort_by(|a, b| {
            a.name
                .as_str()
                .cmp(b.name.as_str())
                .then(a.peer.cmp(&b.peer))
        });
        v
    }

    /// 항목 조회.
    #[must_use]
    pub fn get(&self, peer: PeerId) -> Option<PeerEntry> {
        self.entries.get(&peer).map(|e| PeerEntry {
            peer,
            name: e.name.clone(),
            paths: e.sources.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: u32 = 3_000; // 테스트용 — 실제 값은 D-8 실측 후

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn name(s: &str) -> DisplayName {
        DisplayName::parse(s).unwrap()
    }
    fn at(ms: u64) -> MonoInstant {
        MonoInstant(ms * 1_000_000)
    }

    #[test]
    fn same_peer_multiple_paths_is_one_entry() {
        // FR-D-6 — 여러 인터페이스(S1 멀티캐스트 + S3 브로드캐스트)로 보여도 항목은 1개.
        let mut t = PeerTable::new(TIMEOUT);
        assert_eq!(
            t.observe(pid(1), name("bob"), SourceId(0), at(0)),
            Some(PeerEvent::Appeared(pid(1)))
        );
        assert_eq!(
            t.observe(pid(1), name("bob"), SourceId(1), at(1)),
            None,
            "경로 추가는 무이벤트"
        );
        let list = t.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].paths, 2, "경로 2개가 한 항목에 병합");
    }

    #[test]
    fn different_peer_ids_never_merge_even_with_same_name() {
        // DR-18 — 이름은 근거가 아니다. 다른 키 = 다른 PC = 별개 항목.
        let mut t = PeerTable::new(TIMEOUT);
        t.observe(pid(1), name("kim"), SourceId(0), at(0));
        t.observe(pid(2), name("kim"), SourceId(0), at(0));
        assert_eq!(t.list().len(), 2);
    }

    #[test]
    fn goodbye_on_one_path_keeps_entry_until_last() {
        // FR-D-8 ① — 한 인터페이스가 내려가도 다른 경로가 살아 있으면 목록 유지.
        let mut t = PeerTable::new(TIMEOUT);
        t.observe(pid(1), name("bob"), SourceId(0), at(0));
        t.observe(pid(1), name("bob"), SourceId(1), at(0));
        assert_eq!(
            t.goodbye(pid(1), SourceId(0)),
            None,
            "경로 하나 남음 — 유지"
        );
        assert_eq!(t.get(pid(1)).unwrap().paths, 1);
        assert_eq!(
            t.goodbye(pid(1), SourceId(1)),
            Some(PeerEvent::Departed(pid(1), DepartReason::Goodbye)),
            "마지막 경로의 goodbye = 이탈"
        );
        assert!(t.list().is_empty());
    }

    #[test]
    fn timeout_departs_regardless_of_paths() {
        // FR-D-8 ② — 무응답은 경로 수와 무관하게 마지막 관측 기준.
        let mut t = PeerTable::new(TIMEOUT);
        t.observe(pid(1), name("bob"), SourceId(0), at(0));
        t.observe(pid(1), name("bob"), SourceId(1), at(0));
        assert!(t.sweep(at(u64::from(TIMEOUT) - 1)).is_empty(), "임계 전");
        assert_eq!(
            t.sweep(at(u64::from(TIMEOUT))),
            vec![PeerEvent::Departed(pid(1), DepartReason::Timeout)]
        );
    }

    #[test]
    fn observation_refreshes_timeout() {
        let mut t = PeerTable::new(TIMEOUT);
        t.observe(pid(1), name("bob"), SourceId(0), at(0));
        t.observe(pid(1), name("bob"), SourceId(0), at(2_000)); // 생존 신호
        assert!(t.sweep(at(4_000)).is_empty(), "마지막 관측 기준으로 연장");
        assert_eq!(t.sweep(at(2_000 + u64::from(TIMEOUT))).len(), 1);
    }

    #[test]
    fn rename_is_an_event_not_a_new_entry() {
        let mut t = PeerTable::new(TIMEOUT);
        t.observe(pid(1), name("DESKTOP-A7X3"), SourceId(0), at(0));
        assert_eq!(
            t.observe(pid(1), name("bob의 노트북"), SourceId(0), at(1)),
            Some(PeerEvent::Renamed(pid(1))),
            "같은 키의 이름 변경 = 같은 항목의 개명"
        );
        assert_eq!(t.list().len(), 1);
    }

    #[test]
    fn reappear_after_depart_is_appeared_again() {
        let mut t = PeerTable::new(TIMEOUT);
        t.observe(pid(1), name("bob"), SourceId(0), at(0));
        t.sweep(at(u64::from(TIMEOUT)));
        assert_eq!(
            t.observe(pid(1), name("bob"), SourceId(0), at(10_000)),
            Some(PeerEvent::Appeared(pid(1))),
            "이탈 후 재등장 = 새 등장 이벤트"
        );
    }

    #[test]
    fn list_is_deterministic_by_name_then_peer() {
        let mut t = PeerTable::new(TIMEOUT);
        t.observe(pid(3), name("carol"), SourceId(0), at(0));
        t.observe(pid(1), name("alice"), SourceId(0), at(0));
        t.observe(pid(2), name("bob"), SourceId(0), at(0));
        let list = t.list();
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alice", "bob", "carol"]);
    }
}
