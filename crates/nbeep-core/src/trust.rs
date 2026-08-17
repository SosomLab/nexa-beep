//! TOFU 신뢰 저장 — 세션이 인증한 키 위에 **신뢰 상태**를 얹는다([docs/08] ADR-0002 §4).
//!
//! 세션([`crate::session::Session`])은 상대가 **이 키를 갖고 있음**을 증명할 뿐이다. "이 키를 믿는가"는
//! 별개다 — 이 모듈이 [`PeerId`]별 신뢰 등급([`TrustLevel`])·차단·이름 이력을 관리한다.
//!
//! ## v1의 TOFU — "지문 바뀜"이 왜 없나
//!
//! **[`PeerId`] = X25519 공개키**(DR-8)라, "같은 상대의 지문이 바뀐다"는 **암호학적으로 불가능**하다
//! (다른 키 = 다른 `PeerId` = 다른 항목). 그래서 v1 보호는 *차단*이 아니라 **불상속**이다:
//! 신뢰는 **키별로만** 쌓이고, **이름이 같아도 다른 키에 절대 옮겨가지 않는다**. 이름 재사용은
//! [`name_conflict`](MemoryTrustStore::name_conflict) 경고로 드러낸다. ("지문 바뀜 → 차단"의
//! 원래 시나리오는 **v2 `UserId`**(안정 신원 위에 기기 목록)에서 실체가 된다 — [docs/20] ADR-0007.)

use crate::identity::{PeerId, TrustLevel};
use crate::name::DisplayName;
use std::collections::HashMap;

/// 세션 성립 시점의 신뢰 판정([docs/08] §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustDecision {
    /// 처음 보는 키 — **TOFU 핀**(자동으로 [`TrustLevel::Pinned`]로 고정).
    FirstContact,
    /// 이미 아는 키 — 저장된 등급을 돌려준다.
    Known(TrustLevel),
    /// 차단된 상대 — 진행 금지(fail-closed).
    Blocked,
}

/// 한 `PeerId`의 신뢰 기록.
#[derive(Clone, Debug)]
struct Record {
    level: TrustLevel,
    blocked: bool,
    /// 이 키로 본 표시 이름들(가장 최근이 뒤). 이름 위장 감사를 위해 이력을 남긴다.
    names: Vec<DisplayName>,
    /// 목록 상단 고정(08-15 — TOFU "핀"과 별개인 즐겨찾기).
    fav: bool,
    /// 최근 접속·대화 시각(unix ms · 0 = 기록 없음).
    last_seen: u64,
    last_chat: u64,
}

/// 신뢰 저장 포트 — 파일 기반 구현은 `nbeep-store`(M2-5)가 이 트레이트로 제공한다.
pub trait TrustStore {
    /// 세션 성립 시 호출 — 처음 보는 키면 핀하고 [`TrustDecision::FirstContact`], 아니면 판정을 돌려준다.
    fn on_session(&mut self, peer: PeerId) -> TrustDecision;
    /// 현재 신뢰 등급(미지 키는 [`TrustLevel::Unverified`]).
    fn level(&self, peer: PeerId) -> TrustLevel;
    /// SAS 지문 대조 완료 → [`TrustLevel::FingerprintVerified`]로 승격.
    fn verify(&mut self, peer: PeerId);
    /// 인증 취소 — SAS 승격을 되돌린다(`/unverify`). `FingerprintVerified`였던
    /// 키만 [`TrustLevel::Pinned`]로 강등한다(핸드셰이크로 재확인된 TOFU 상태 =
    /// 안전한 하한). 그 아래 등급·미지 키는 손대지 않는다(멱등).
    fn unverify(&mut self, peer: PeerId);
    /// 목록에서 삭제 — 이 키의 핀 레코드를 통째로 지운다(08-17 사용자 요청).
    /// 신뢰·이름 이력·즐겨찾기가 모두 사라진다. 그 키가 다시 세션을 맺으면
    /// [`TrustDecision::FirstContact`]로 처음처럼 새로 핀된다(되돌릴 수 있음 =
    /// 삭제를 안전한 기본으로 두는 근거). 없던 키면 아무 일도 없다(멱등).
    fn forget(&mut self, peer: PeerId);
    /// 차단(사람이 아니라 이 키 단위).
    fn block(&mut self, peer: PeerId);
    /// 차단 여부.
    fn is_blocked(&self, peer: PeerId) -> bool;
}

/// 영속 스냅샷 단위(M2-5a) — `nbeep-store`가 이 형태로 파일에 나르고 되돌린다.
/// 도메인 로직은 이 타입을 소비하지 않는다(직렬화 경계 전용).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinRecord {
    /// 키(= 신뢰의 유일한 귀속 대상).
    pub peer: PeerId,
    /// 신뢰 등급.
    pub level: TrustLevel,
    /// 차단 여부.
    pub blocked: bool,
    /// 이 키로 본 이름 이력(가장 최근이 뒤).
    pub names: Vec<DisplayName>,
    /// 목록 상단 고정(08-15 사용자 요청 — TOFU "핀"과 별개인 즐겨찾기).
    pub fav: bool,
    /// 최근 접속 관측 시각(unix ms · 0 = 기록 없음) — 발견·세션 성립.
    pub last_seen: u64,
    /// 최근 대화 시각(unix ms · 0 = 기록 없음) — 메시지 송·수신.
    pub last_chat: u64,
}

/// 인메모리 TOFU 저장(순수 로직). 영속은 `nbeep-store`가 이걸 파일로 감싼다(M2-5).
#[derive(Debug, Default)]
pub struct MemoryTrustStore {
    records: HashMap<PeerId, Record>,
}

impl MemoryTrustStore {
    /// 빈 저장소.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 이 키로 본 표시 이름을 기록한다(중복 연속은 합침).
    pub fn record_name(&mut self, peer: PeerId, name: DisplayName) {
        let rec = self.records.entry(peer).or_insert_with(|| Record {
            level: TrustLevel::Unverified,
            blocked: false,
            names: Vec::new(),
            fav: false,
            last_seen: 0,
            last_chat: 0,
        });
        if rec.names.last() != Some(&name) {
            rec.names.push(name);
        }
    }

    /// **이름 재사용 경고** — `name`이 **다른** 키에서 이미 관찰된 적이 있으면 그 키를 돌려준다.
    ///
    /// 이름은 신원이 아니므로(같은 이름 ≠ 같은 사람), 같은 이름이 다른 키로 나타나면 UI가
    /// "이 이름은 다른 신원으로 본 적 있음"을 알릴 근거가 된다. **검증된(FingerprintVerified) 키를 우선** 반환.
    #[must_use]
    pub fn name_conflict(&self, peer: PeerId, name: &DisplayName) -> Option<PeerId> {
        let mut fallback = None;
        for (&other, rec) in &self.records {
            if other == peer {
                continue;
            }
            if rec.names.iter().any(|n| n == name) {
                if rec.level == TrustLevel::FingerprintVerified {
                    return Some(other);
                }
                fallback.get_or_insert(other);
            }
        }
        fallback
    }

    /// 이 키로 본 이름 이력(가장 최근이 뒤).
    #[must_use]
    pub fn names(&self, peer: PeerId) -> &[DisplayName] {
        self.records.get(&peer).map_or(&[], |r| &r.names)
    }

    /// 목록 고정 지정(08-15) — 기록이 있는(아는) 상대만. 바뀌었으면 `true`.
    pub fn set_fav(&mut self, peer: PeerId, fav: bool) -> bool {
        match self.records.get_mut(&peer) {
            Some(r) if r.fav != fav => {
                r.fav = fav;
                true
            }
            _ => false,
        }
    }

    /// 목록 고정 여부.
    #[must_use]
    pub fn fav(&self, peer: PeerId) -> bool {
        self.records.get(&peer).is_some_and(|r| r.fav)
    }

    /// 최근 접속 관측 기록(08-15) — **아는 상대만** 갱신한다(발견만 된 상대에 기록을
    /// 만들면 "핀 = 연결됐던 기록"이라는 부팅 시드 의미가 깨진다). 바뀌면 `true`.
    pub fn note_seen(&mut self, peer: PeerId, unix_ms: u64) -> bool {
        match self.records.get_mut(&peer) {
            Some(r) if r.last_seen < unix_ms => {
                r.last_seen = unix_ms;
                true
            }
            _ => false,
        }
    }

    /// 최근 대화 시각 기록(08-15) — 메시지 송·수신. 아는 상대만.
    pub fn note_chat(&mut self, peer: PeerId, unix_ms: u64) -> bool {
        match self.records.get_mut(&peer) {
            Some(r) if r.last_chat < unix_ms => {
                r.last_chat = unix_ms;
                true
            }
            _ => false,
        }
    }

    /// (최근 접속, 최근 대화) — 없으면 (0, 0).
    #[must_use]
    pub fn meta(&self, peer: PeerId) -> (u64, u64) {
        self.records
            .get(&peer)
            .map_or((0, 0), |r| (r.last_seen, r.last_chat))
    }

    /// 영속 스냅샷(M2-5a) — **키 바이트 정렬**로 결정적(같은 상태 = 같은 직렬화).
    #[must_use]
    pub fn export(&self) -> Vec<PinRecord> {
        let mut out: Vec<PinRecord> = self
            .records
            .iter()
            .map(|(&peer, r)| PinRecord {
                peer,
                level: r.level,
                blocked: r.blocked,
                names: r.names.clone(),
                fav: r.fav,
                last_seen: r.last_seen,
                last_chat: r.last_chat,
            })
            .collect();
        out.sort_unstable_by(|a, b| a.peer.as_bytes().cmp(b.peer.as_bytes()));
        out
    }

    /// 스냅샷 복원(M2-5a) — [`Self::export`]의 역.
    #[must_use]
    pub fn from_records(records: Vec<PinRecord>) -> Self {
        let mut store = Self::default();
        for r in records {
            store.records.insert(
                r.peer,
                Record {
                    level: r.level,
                    blocked: r.blocked,
                    names: r.names,
                    fav: r.fav,
                    last_seen: r.last_seen,
                    last_chat: r.last_chat,
                },
            );
        }
        store
    }
}

impl TrustStore for MemoryTrustStore {
    fn on_session(&mut self, peer: PeerId) -> TrustDecision {
        if let Some(rec) = self.records.get(&peer) {
            if rec.blocked {
                return TrustDecision::Blocked;
            }
            // 이미 알던 키 — 최소 Pinned 이상으로 본다(핸드셰이크로 재확인됨).
            if rec.level == TrustLevel::Unverified {
                // 발견만 하고 세션은 처음 — 지금 핀한다.
                let level = TrustLevel::Pinned;
                self.records.get_mut(&peer).expect("존재 확인됨").level = level;
                return TrustDecision::FirstContact;
            }
            return TrustDecision::Known(rec.level);
        }
        self.records.insert(
            peer,
            Record {
                level: TrustLevel::Pinned,
                blocked: false,
                names: Vec::new(),
                fav: false,
                last_seen: 0,
                last_chat: 0,
            },
        );
        TrustDecision::FirstContact
    }

    fn level(&self, peer: PeerId) -> TrustLevel {
        self.records
            .get(&peer)
            .map_or(TrustLevel::Unverified, |r| r.level)
    }

    fn verify(&mut self, peer: PeerId) {
        let rec = self.records.entry(peer).or_insert_with(|| Record {
            level: TrustLevel::Unverified,
            blocked: false,
            names: Vec::new(),
            fav: false,
            last_seen: 0,
            last_chat: 0,
        });
        rec.level = TrustLevel::FingerprintVerified;
    }

    fn unverify(&mut self, peer: PeerId) {
        if let Some(rec) = self.records.get_mut(&peer) {
            if rec.level == TrustLevel::FingerprintVerified {
                rec.level = TrustLevel::Pinned;
            }
        }
    }

    fn forget(&mut self, peer: PeerId) {
        self.records.remove(&peer);
    }

    fn block(&mut self, peer: PeerId) {
        let rec = self.records.entry(peer).or_insert_with(|| Record {
            level: TrustLevel::Unverified,
            blocked: false,
            names: Vec::new(),
            fav: false,
            last_seen: 0,
            last_chat: 0,
        });
        rec.blocked = true;
    }

    fn is_blocked(&self, peer: PeerId) -> bool {
        self.records.get(&peer).is_some_and(|r| r.blocked)
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
    fn first_contact_pins() {
        let mut ts = MemoryTrustStore::new();
        assert_eq!(ts.level(pid(1)), TrustLevel::Unverified, "미지 키");
        assert_eq!(ts.on_session(pid(1)), TrustDecision::FirstContact);
        assert_eq!(ts.level(pid(1)), TrustLevel::Pinned, "핀됨");
    }

    #[test]
    fn known_peer_returns_stored_level() {
        let mut ts = MemoryTrustStore::new();
        ts.on_session(pid(1)); // 핀
        assert_eq!(
            ts.on_session(pid(1)),
            TrustDecision::Known(TrustLevel::Pinned)
        );
    }

    #[test]
    fn sas_elevates_to_verified() {
        let mut ts = MemoryTrustStore::new();
        ts.on_session(pid(1));
        ts.verify(pid(1));
        assert_eq!(ts.level(pid(1)), TrustLevel::FingerprintVerified);
        assert_eq!(
            ts.on_session(pid(1)),
            TrustDecision::Known(TrustLevel::FingerprintVerified)
        );
    }

    #[test]
    fn unverify_demotes_only_verified() {
        let mut ts = MemoryTrustStore::new();
        ts.on_session(pid(1)); // Pinned
        ts.verify(pid(1)); // FingerprintVerified
        ts.unverify(pid(1));
        assert_eq!(ts.level(pid(1)), TrustLevel::Pinned, "검증만 되돌린다");
        // Pinned에 unverify = 무변(멱등) · 미지 키에도 안전.
        ts.unverify(pid(1));
        assert_eq!(ts.level(pid(1)), TrustLevel::Pinned);
        ts.unverify(pid(99));
        assert_eq!(ts.level(pid(99)), TrustLevel::Unverified);
    }

    #[test]
    fn forget_removes_record() {
        let mut ts = MemoryTrustStore::new();
        ts.on_session(pid(1));
        ts.verify(pid(1));
        ts.forget(pid(1));
        assert_eq!(ts.level(pid(1)), TrustLevel::Unverified, "레코드 삭제");
        // 삭제 후 재세션 = 처음처럼 새 핀.
        assert_eq!(ts.on_session(pid(1)), TrustDecision::FirstContact);
    }

    #[test]
    fn block_is_fail_closed() {
        let mut ts = MemoryTrustStore::new();
        ts.on_session(pid(1));
        ts.block(pid(1));
        assert!(ts.is_blocked(pid(1)));
        assert_eq!(
            ts.on_session(pid(1)),
            TrustDecision::Blocked,
            "차단은 진행 금지"
        );
    }

    #[test]
    fn trust_never_transfers_across_keys() {
        // pid(1)을 검증해도 pid(2)는 여전히 미지 — 신뢰는 키별로만.
        let mut ts = MemoryTrustStore::new();
        ts.on_session(pid(1));
        ts.verify(pid(1));
        assert_eq!(ts.level(pid(2)), TrustLevel::Unverified);
    }

    #[test]
    fn name_conflict_flags_reused_name_on_different_key() {
        let mut ts = MemoryTrustStore::new();
        // pid(1)을 "bob"으로 보고 검증.
        ts.on_session(pid(1));
        ts.verify(pid(1));
        ts.record_name(pid(1), name("bob"));

        // pid(2)가 같은 이름 "bob"으로 나타남 → 충돌은 검증된 pid(1)을 가리킨다.
        assert_eq!(ts.name_conflict(pid(2), &name("bob")), Some(pid(1)));
        // 다른 이름은 충돌 없음.
        assert_eq!(ts.name_conflict(pid(2), &name("carol")), None);
        // 자기 자신 이름은 충돌 아님.
        assert_eq!(ts.name_conflict(pid(1), &name("bob")), None);
    }

    #[test]
    fn verified_conflict_wins_over_unverified() {
        let mut ts = MemoryTrustStore::new();
        // pid(1) 미검증 "bob", pid(2) 검증 "bob".
        ts.record_name(pid(1), name("bob"));
        ts.on_session(pid(2));
        ts.verify(pid(2));
        ts.record_name(pid(2), name("bob"));
        // pid(3)의 "bob" 충돌은 검증된 pid(2)를 우선 가리킨다.
        assert_eq!(ts.name_conflict(pid(3), &name("bob")), Some(pid(2)));
    }

    #[test]
    fn names_deduplicate_consecutive() {
        let mut ts = MemoryTrustStore::new();
        ts.record_name(pid(1), name("bob"));
        ts.record_name(pid(1), name("bob"));
        ts.record_name(pid(1), name("bobby"));
        assert_eq!(ts.names(pid(1)).len(), 2);
    }
}
