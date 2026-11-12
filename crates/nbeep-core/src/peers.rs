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
use std::collections::{BTreeMap, HashMap};

/// ★ 세션 세대(M1-13 ⓒ · 11-05) — 발견 `flags` 하위 4비트에 실린다. **핸드셰이크·멀티플렉스가 호환되지 않게 바뀔 때만**
/// 올린다(지금 1 = Noise XX/XXpsk3 + mux v1). 다른 세대의 상대는 목록에서 **제외가 아니라 "버전 불일치"로 표시**된다
/// (조용한 비호환 금지). 0 = 세대 비트가 없던 구버전 광고 = 1세대로 본다.
pub const SESSION_GEN: u8 = 1;
/// 미래 와이어 버전의 상대 — 헌법 접두(매직·버전·키)만 읽혀 세대를 모른다 = 비호환으로 표시.
pub const FOREIGN_GEN: u8 = 0xFF;

/// 관측 경로 토큰 — "어느 인터페이스/발견 단계/랑데부 세대에서 봤나". 내용은 전송 어댑터만 안다(불투명).
/// X-2f(11-06): 랑데부 전송은 **세대마다 다른 토큰**을 쓴다 — 서버 재접속 때 옛 명부의 늦은 이탈이 새 명부를 지우지 않게.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u32);

/// 힌트 **출처**(X-2f · ADR-0003 §9 A·B) — Locator가 **아니다**(주소·포트·인터페이스 없음). UI 표시(Server 칩·강등)와
/// 이름 우선권(로컬 광고 이름 > 랑데부 카드 이름)에만 쓴다. 경로 **등급**은 성립한 세션이 정한다(DR-28).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HintOrigin {
    /// LAN 발견 광고(멀티캐스트·브로드캐스트·S1~S6).
    #[default]
    Local,
    /// 랑데부 서버 공개 명부(X-2e roster) — 존재만 · 미검증.
    Rendezvous,
}

/// 경로 **수명 방식**(X-2f · ADR-0003 §9 D) — 테이블의 만료 규칙 선택.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Liveness {
    /// 주기 광고 재관측으로 생존 · 마지막 관측 TTL 만료([`PeerTable::sweep`]).
    #[default]
    Advertised,
    /// 이벤트(Up/Down)로만 들고 난다 — **타임아웃 없음** · [`PeerTable::goodbye`]로만 제거.
    Eventful,
}

/// 랑데부 서버가 중계한 **공개 카드**(옵트인 · X-2e) — 경계(전송)에서 정제가 끝난 값만 올라온다(이름 = [`DisplayName`]
/// 통과분 · 이메일·소개 = 제어 문자 제거·200자). 서버 중계 = **미검증 주장**(종단 프로필이 오면 호스트가 그쪽을 우선).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublicCard {
    /// 카드 이름(빈 값 = 없음).
    pub name: String,
    /// 공개 이메일(빈 값 = 없음).
    pub email: String,
    /// 소개(빈 값 = 없음).
    pub bio: String,
}

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
    /// 표시 재료 변경(X-2f) — 출처 집합(로컬 ↔ 랑데부 전용 강등/승격)·공개 카드가 바뀌었다. 이름은 그대로.
    Updated(PeerId),
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
    /// 상대가 광고한 세션 세대([`SESSION_GEN`] · 0 = 구버전 광고 또는 랑데부만(세대 미상) · [`FOREIGN_GEN`] = 미래 와이어).
    pub gen: u8,
    /// LAN 광고 경로가 하나라도 살아 있다(X-2f) — "발견됨" 판정·정렬 계층·경로 칩 `local`의 근거.
    pub local: bool,
    /// 랑데부 경로가 하나라도 있다(X-2f).
    pub rendezvous: bool,
    /// 공개 카드(랑데부 상대 · X-2f) — 없으면 None.
    pub profile: Option<PublicCard>,
}

impl PeerEntry {
    /// 이 상대와 세션을 맺을 수 있는가(M1-13 ⓒ) — 같은 세대(또는 세대 비트 없는 구버전).
    #[must_use]
    pub fn compatible(&self) -> bool {
        self.gen == 0 || self.gen == SESSION_GEN
    }

    /// 랑데부 명부로만 아는 상대(X-2f) — Server 칩·왕래 기준 승격의 근거.
    #[must_use]
    pub fn rendezvous_only(&self) -> bool {
        self.rendezvous && !self.local
    }
}

/// 관측 하나(X-2f) — [`PeerTable::observe_with`] 입력(인자 묶음).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    /// 상대(미검증).
    pub peer: PeerId,
    /// 무해화된 표시 이름(출처가 랑데부면 카드 이름 또는 기본 라벨).
    pub name: DisplayName,
    /// 세션 세대(랑데부 = 0 · 미상).
    pub gen: u8,
    /// 관측 경로 토큰.
    pub source: SourceId,
    /// 출처.
    pub origin: HintOrigin,
    /// 수명 방식.
    pub liveness: Liveness,
    /// 공개 카드(Some = 갱신 · None = 유지).
    pub profile: Option<PublicCard>,
}

#[derive(Debug)]
struct Entry {
    name: DisplayName,
    gen: u8,
    /// 살아 있는 경로 → (출처, 수명 방식).
    sources: BTreeMap<SourceId, (HintOrigin, Liveness)>,
    profile: Option<PublicCard>,
    /// 마지막 **광고** 관측 시각(Eventful 관측은 갱신하지 않는다 — 명부 재등장이 LAN 생존을 늘리면 안 된다).
    last_seen: MonoInstant,
}

impl Entry {
    fn has_local(&self) -> bool {
        self.sources.values().any(|(o, _)| *o == HintOrigin::Local)
    }
    fn has_rendezvous(&self) -> bool {
        self.sources
            .values()
            .any(|(o, _)| *o == HintOrigin::Rendezvous)
    }
    fn has_advertised(&self) -> bool {
        self.sources
            .values()
            .any(|(_, l)| *l == Liveness::Advertised)
    }
    fn view(&self, peer: PeerId) -> PeerEntry {
        PeerEntry {
            peer,
            name: self.name.clone(),
            paths: self.sources.len(),
            gen: self.gen,
            local: self.has_local(),
            rendezvous: self.has_rendezvous(),
            profile: self.profile.clone(),
        }
    }
}

/// 발견 관측을 목록으로 접는 테이블.
#[derive(Debug)]
pub struct PeerTable {
    /// 무응답 이탈 임계(ms) — **D-8 실측 후 확정값을 주입**(하드코딩 금지).
    timeout_ms: u32,
    entries: HashMap<PeerId, Entry>,
    /// 방금 이탈한 항목의 마지막 표시 이름 — 이탈 사건의 소비자가 [`Self::take_departed_name`]으로 곧바로
    /// 꺼낸다(오프라인 행 강등 때 이름이 `beep-<지문>`으로 내려가지 않게 · 11-12). 꺼내지 않으면 다음 이탈이
    /// 덮을 뿐이라 이탈한 수만큼만 남는다.
    departed_names: HashMap<PeerId, DisplayName>,
}

impl PeerTable {
    /// `timeout_ms` 동안 관측이 없으면 이탈로 판정하는 테이블.
    #[must_use]
    pub fn new(timeout_ms: u32) -> Self {
        Self {
            timeout_ms,
            entries: HashMap::new(),
            departed_names: HashMap::new(),
        }
    }

    /// 발견 관측 하나를 접는다(LAN 광고 · 세대 0) — 새 항목이면 `Appeared`, 이름이 바뀌었으면 `Renamed`,
    /// 이미 알던 관측(경로 추가·생존 신호)이면 `None`.
    pub fn observe(
        &mut self,
        peer: PeerId,
        name: DisplayName,
        source: SourceId,
        now: MonoInstant,
    ) -> Option<PeerEvent> {
        self.observe_gen(peer, name, 0, source, now)
    }

    /// 세대 비트를 동반한 LAN 관측(M1-13 ⓒ) — 세대가 바뀌면(업데이트) 이름 변경과 같이 `Renamed`로 행을 다시 그린다.
    pub fn observe_gen(
        &mut self,
        peer: PeerId,
        name: DisplayName,
        gen: u8,
        source: SourceId,
        now: MonoInstant,
    ) -> Option<PeerEvent> {
        self.observe_with(
            Observation {
                peer,
                name,
                gen,
                source,
                origin: HintOrigin::Local,
                liveness: Liveness::Advertised,
                profile: None,
            },
            now,
        )
    }

    /// 출처·수명·카드를 동반한 관측(X-2f · ADR-0003 §9) — 병합 규칙:
    /// - 같은 `PeerId`는 출처가 달라도 **한 항목**(FR-D-6). 경로는 토큰 단위로 쌓인다.
    /// - **이름·세대 우선권은 로컬 광고**: 로컬 경로가 살아 있으면 랑데부 관측은 이름을 바꾸지 않는다(카드 이름은
    ///   로컬 광고가 없을 때만 표시 이름이 된다).
    /// - `last_seen`은 `Advertised` 관측만 갱신한다.
    /// - 반환: 새 항목 `Appeared` · 이름/세대 변경 `Renamed` · 출처 집합(로컬/랑데부 유무)이나 카드 변경 `Updated` · 그 외 `None`.
    pub fn observe_with(&mut self, o: Observation, now: MonoInstant) -> Option<PeerEvent> {
        let Observation {
            peer,
            name,
            gen,
            source,
            origin,
            liveness,
            profile,
        } = o;
        if let Some(e) = self.entries.get_mut(&peer) {
            let before = (e.has_local(), e.has_rendezvous());
            e.sources.insert(source, (origin, liveness));
            if liveness == Liveness::Advertised {
                e.last_seen = now;
            }
            let mut updated = (e.has_local(), e.has_rendezvous()) != before;
            if let Some(card) = profile {
                if e.profile.as_ref() != Some(&card) {
                    e.profile = Some(card);
                    updated = true;
                }
            }
            let names_it = origin == HintOrigin::Local || !e.has_local();
            if names_it && (e.name != name || e.gen != gen) {
                e.name = name;
                e.gen = gen;
                return Some(PeerEvent::Renamed(peer));
            }
            return updated.then_some(PeerEvent::Updated(peer));
        }
        self.entries.insert(
            peer,
            Entry {
                name,
                gen,
                sources: BTreeMap::from([(source, (origin, liveness))]),
                profile,
                last_seen: now,
            },
        );
        Some(PeerEvent::Appeared(peer))
    }

    /// 한 경로의 goodbye — **마지막 경로였을 때만** 이탈이다(다중 경로 병합의 이탈 면). 남은 경로로 출처 집합이
    /// 바뀌면(예: LAN 경로가 빠져 랑데부 전용으로 강등) `Updated`.
    pub fn goodbye(&mut self, peer: PeerId, source: SourceId) -> Option<PeerEvent> {
        let e = self.entries.get_mut(&peer)?;
        let before = (e.has_local(), e.has_rendezvous());
        e.sources.remove(&source);
        if e.sources.is_empty() {
            if let Some(gone) = self.entries.remove(&peer) {
                self.departed_names.insert(peer, gone.name);
            }
            return Some(PeerEvent::Departed(peer, DepartReason::Goodbye));
        }
        ((e.has_local(), e.has_rendezvous()) != before).then_some(PeerEvent::Updated(peer))
    }

    /// 발견 테이블에서 즉시 제거(08-17 — 사용자 "목록에서 삭제"). 살아 있는
    /// 모든 경로를 지운다. 상대가 다시 비컨하면 새 [`PeerEvent::Appeared`]로 돌아온다
    /// (삭제 = 역사 지움이지 영구 차단이 아니다 — 차단은 `TrustStore::block`).
    pub fn forget(&mut self, peer: PeerId) {
        self.entries.remove(&peer);
        self.departed_names.remove(&peer);
    }

    /// 이탈(`Departed`)한 항목의 마지막 표시 이름을 꺼낸다(1회 · 미검증 힌트 그대로 — 표시용).
    pub fn take_departed_name(&mut self, peer: PeerId) -> Option<DisplayName> {
        self.departed_names.remove(&peer)
    }

    /// 무응답 판정 — `now` 기준 임계를 넘긴 항목의 **광고 경로**를 거둔다. 주기 호출(발견 틱).
    /// 광고 경로가 전부였으면 `Departed(Timeout)` · 이벤트형(랑데부) 경로가 남으면 강등 `Updated`(행 유지) ·
    /// 광고 경로가 없는 항목(랑데부 전용)은 만료 대상이 아니다(ADR-0003 §9 D).
    pub fn sweep(&mut self, now: MonoInstant) -> Vec<PeerEvent> {
        let timeout = self.timeout_ms;
        let expired: Vec<PeerId> = self
            .entries
            .iter()
            .filter(|(_, e)| e.has_advertised() && now.saturating_ms_since(e.last_seen) >= timeout)
            .map(|(&p, _)| p)
            .collect();
        let mut out = Vec::with_capacity(expired.len());
        for p in expired {
            let Some(e) = self.entries.get_mut(&p) else {
                continue;
            };
            e.sources.retain(|_, (_, l)| *l != Liveness::Advertised);
            if e.sources.is_empty() {
                if let Some(gone) = self.entries.remove(&p) {
                    self.departed_names.insert(p, gone.name);
                }
                out.push(PeerEvent::Departed(p, DepartReason::Timeout));
            } else {
                out.push(PeerEvent::Updated(p));
            }
        }
        out
    }

    /// 현재 목록(표시 이름 순 — 결정적).
    #[must_use]
    pub fn list(&self) -> Vec<PeerEntry> {
        let mut v: Vec<PeerEntry> = self.entries.iter().map(|(&peer, e)| e.view(peer)).collect();
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
        self.entries.get(&peer).map(|e| e.view(peer))
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
        assert!(
            t.take_departed_name(pid(1)).is_some(),
            "이탈한 항목의 마지막 이름을 1회 꺼낼 수 있다"
        );
        assert!(t.take_departed_name(pid(1)).is_none(), "두 번째는 없다");
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
    /// M1-13 ⓒ — 세대 비트: 없음(0)·같음(1) = 호환, 다른 세대·미래 와이어 = 비호환으로 **표시**(행은 남는다 · 제외 아님).
    #[test]
    fn generation_marks_incompatible_but_keeps_row() {
        let mut t = PeerTable::new(60_000);
        assert_eq!(
            t.observe_gen(pid(1), name("old"), 0, SourceId(0), at(0)),
            Some(PeerEvent::Appeared(pid(1)))
        );
        assert!(t.get(pid(1)).unwrap().compatible(), "구버전 광고 = 호환");
        t.observe_gen(pid(2), name("same"), SESSION_GEN, SourceId(0), at(0));
        assert!(t.get(pid(2)).unwrap().compatible());
        t.observe_gen(pid(3), name("future"), SESSION_GEN + 1, SourceId(0), at(0));
        assert!(
            !t.get(pid(3)).unwrap().compatible(),
            "다른 세대 = 비호환 표시"
        );
        t.observe_gen(pid(4), name("v9"), FOREIGN_GEN, SourceId(0), at(0));
        assert!(!t.get(pid(4)).unwrap().compatible());
        assert_eq!(
            t.list().len(),
            4,
            "비호환도 목록에 남는다(조용한 비호환 금지)"
        );
        // 세대가 바뀌면(업데이트) 행 갱신 이벤트.
        assert_eq!(
            t.observe_gen(pid(3), name("future"), SESSION_GEN, SourceId(0), at(1)),
            Some(PeerEvent::Renamed(pid(3)))
        );
        assert!(t.get(pid(3)).unwrap().compatible());
    }

    // ── X-2f(11-06) 출처·수명·카드 ──

    fn obs(b: u8, n: &str, src: u32, origin: HintOrigin) -> Observation {
        let liveness = match origin {
            HintOrigin::Local => Liveness::Advertised,
            HintOrigin::Rendezvous => Liveness::Eventful,
        };
        Observation {
            peer: pid(b),
            name: name(n),
            gen: 0,
            source: SourceId(src),
            origin,
            liveness,
            profile: None,
        }
    }

    #[test]
    fn eventful_source_never_times_out() {
        let mut t = PeerTable::new(TIMEOUT);
        assert_eq!(
            t.observe_with(obs(1, "bob", 1, HintOrigin::Rendezvous), at(0)),
            Some(PeerEvent::Appeared(pid(1)))
        );
        assert!(
            t.sweep(at(u64::from(TIMEOUT) * 10)).is_empty(),
            "명부 전용 = 만료 없음"
        );
        let e = t.get(pid(1)).unwrap();
        assert!(e.rendezvous_only() && !e.local);
        assert_eq!(
            t.goodbye(pid(1), SourceId(1)),
            Some(PeerEvent::Departed(pid(1), DepartReason::Goodbye)),
            "Withdrawn = 마지막 경로 이탈"
        );
    }

    #[test]
    fn local_timeout_downgrades_to_rendezvous_only() {
        let mut t = PeerTable::new(TIMEOUT);
        t.observe_with(obs(1, "bob", 0, HintOrigin::Local), at(0));
        assert_eq!(
            t.observe_with(obs(1, "Bobby", 1, HintOrigin::Rendezvous), at(1)),
            Some(PeerEvent::Updated(pid(1))),
            "랑데부 경로 추가 = 출처 집합 변경(이름은 로컬 우선 유지)"
        );
        assert_eq!(
            t.get(pid(1)).unwrap().name,
            name("bob"),
            "로컬 광고 이름이 이긴다"
        );
        assert_eq!(
            t.sweep(at(u64::from(TIMEOUT))),
            vec![PeerEvent::Updated(pid(1))],
            "광고 만료 = 강등(행 유지)"
        );
        let e = t.get(pid(1)).unwrap();
        assert!(e.rendezvous_only());
        assert_eq!(e.paths, 1);
        // 로컬이 돌아오면 승격 — 이름은 로컬 관측이 다시 정한다(같으면 Updated · 다르면 Renamed).
        assert_eq!(
            t.observe_with(obs(1, "bob", 0, HintOrigin::Local), at(9_000)),
            Some(PeerEvent::Updated(pid(1)))
        );
        assert!(t.get(pid(1)).unwrap().local);
    }

    #[test]
    fn rendezvous_name_applies_only_without_local() {
        let mut t = PeerTable::new(TIMEOUT);
        t.observe_with(obs(1, "beep-0100", 1, HintOrigin::Rendezvous), at(0));
        assert_eq!(
            t.observe_with(obs(1, "Alice", 1, HintOrigin::Rendezvous), at(1)),
            Some(PeerEvent::Renamed(pid(1))),
            "로컬 없음 = 카드 이름이 표시 이름"
        );
        assert_eq!(
            t.observe_with(obs(1, "DESKTOP-A", 0, HintOrigin::Local), at(2)),
            Some(PeerEvent::Renamed(pid(1))),
            "로컬 등장 = 로컬 이름으로"
        );
        assert_eq!(
            t.observe_with(obs(1, "Alice", 1, HintOrigin::Rendezvous), at(3)),
            None,
            "로컬이 있으면 랑데부 이름 무시 · 집합도 그대로"
        );
    }

    #[test]
    fn profile_change_is_updated_and_none_keeps() {
        let mut t = PeerTable::new(TIMEOUT);
        let mut o = obs(1, "a", 1, HintOrigin::Rendezvous);
        o.profile = Some(PublicCard {
            name: "A".into(),
            email: String::new(),
            bio: "hi".into(),
        });
        t.observe_with(o.clone(), at(0));
        assert_eq!(t.observe_with(o.clone(), at(1)), None, "같은 카드 = 무변화");
        o.profile.as_mut().unwrap().bio = "bye".into();
        assert_eq!(t.observe_with(o, at(2)), Some(PeerEvent::Updated(pid(1))));
        assert_eq!(t.get(pid(1)).unwrap().profile.unwrap().bio, "bye");
        let mut l = obs(1, "a", 0, HintOrigin::Local);
        l.profile = None;
        t.observe_with(l, at(3));
        assert_eq!(
            t.get(pid(1)).unwrap().profile.unwrap().bio,
            "bye",
            "None = 유지"
        );
    }

    #[test]
    fn eventful_observe_does_not_extend_local_ttl() {
        let mut t = PeerTable::new(TIMEOUT);
        t.observe_with(obs(1, "bob", 0, HintOrigin::Local), at(0));
        t.observe_with(
            obs(1, "bob", 1, HintOrigin::Rendezvous),
            at(u64::from(TIMEOUT) - 1),
        );
        assert_eq!(
            t.sweep(at(u64::from(TIMEOUT))),
            vec![PeerEvent::Updated(pid(1))],
            "명부 관측은 LAN 생존을 연장하지 않는다"
        );
    }
}
