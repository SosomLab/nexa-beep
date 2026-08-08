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
}

/// 신뢰 저장 포트 — 파일 기반 구현은 `nbeep-store`(M2-5)가 이 트레이트로 제공한다.
pub trait TrustStore {
    /// 세션 성립 시 호출 — 처음 보는 키면 핀하고 [`TrustDecision::FirstContact`], 아니면 판정을 돌려준다.
    fn on_session(&mut self, peer: PeerId) -> TrustDecision;
    /// 현재 신뢰 등급(미지 키는 [`TrustLevel::Unverified`]).
    fn level(&self, peer: PeerId) -> TrustLevel;
    /// SAS 지문 대조 완료 → [`TrustLevel::FingerprintVerified`]로 승격.
    fn verify(&mut self, peer: PeerId);
    /// 차단(사람이 아니라 이 키 단위).
    fn block(&mut self, peer: PeerId);
    /// 차단 여부.
    fn is_blocked(&self, peer: PeerId) -> bool;
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
        });
        rec.level = TrustLevel::FingerprintVerified;
    }

    fn block(&mut self, peer: PeerId) {
        let rec = self.records.entry(peer).or_insert_with(|| Record {
            level: TrustLevel::Unverified,
            blocked: false,
            names: Vec::new(),
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
