//! `TrustedSession` — 세션과 신뢰를 잇는 **데코레이터**([docs/13] §2-4).
//!
//! [`Session`]은 "이 상대가 이 키를 갖고 있다"만 증명한다. [`TrustStore`]는 "이 키를 믿는가"만 안다.
//! **둘은 서로를 모른다** — 이 어댑터가 감싸서 붙인다. 덕분에 `NoiseSession`은 신뢰 정책 변경에,
//! `TrustStore`는 암호 구현 변경에 각각 영향받지 않는다.
//!
//! `TrustedSession`도 [`Session`]이라 상위 계층(대화·그룹)은 감싼 것/안 감싼 것을 구별하지 않는다.
//! **차단된 상대는 수립 자체가 실패**한다(fail-closed — [docs/13] §7).

use crate::identity::{PeerId, TrustLevel};
use crate::session::{Session, SessionError};
use crate::trust::{TrustDecision, TrustStore};

/// [`TrustedSession::wrap`]의 결과 — 세션과 **이번 수립의 신뢰 판정**.
#[derive(Debug)]
pub struct Established<S> {
    /// 신뢰 등급이 반영된 세션.
    pub session: S,
    /// 이번 수립이 첫 접촉이었는지 등 — UI가 "새 상대입니다" 배너를 띄울 근거.
    pub decision: TrustDecision,
}

/// [`Session`]에 [`TrustStore`]의 판정을 입힌 세션.
#[derive(Debug)]
pub struct TrustedSession<S: Session> {
    inner: S,
    trust: TrustLevel,
}

impl<S: Session> TrustedSession<S> {
    /// 수립된 세션을 신뢰 저장소에 조회·핀하고 감싼다.
    ///
    /// # Errors
    /// 상대가 차단되어 있으면 [`SessionError::Blocked`] — 세션은 여기서 드롭된다(fail-closed).
    pub fn wrap<T: TrustStore + ?Sized>(
        inner: S,
        store: &mut T,
    ) -> Result<Established<Self>, SessionError> {
        let peer = inner.peer();
        let decision = store.on_session(peer);
        if decision == TrustDecision::Blocked {
            return Err(SessionError::Blocked);
        }
        Ok(Established {
            session: Self {
                inner,
                trust: store.level(peer),
            },
            decision,
        })
    }

    /// 감싼 세션에 대한 참조(테스트·진단용).
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: Session> Session for TrustedSession<S> {
    fn peer(&self) -> PeerId {
        self.inner.peer()
    }

    fn trust(&self) -> TrustLevel {
        // 안쪽 세션이 아니라 **저장소의 판정**이 신뢰의 출처다.
        self.trust
    }

    fn send(&mut self, message: &[u8]) -> Result<(), SessionError> {
        self.inner.send(message)
    }

    fn recv(&mut self) -> Result<Vec<u8>, SessionError> {
        self.inner.recv()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::MemoryTrustStore;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    /// 링크 없이 신뢰 배선만 검증하는 세션 스텁.
    #[derive(Debug)]
    struct StubSession(PeerId);
    impl Session for StubSession {
        fn peer(&self) -> PeerId {
            self.0
        }
        fn trust(&self) -> TrustLevel {
            TrustLevel::Unverified // 세션 스스로는 늘 미검증
        }
        fn send(&mut self, _: &[u8]) -> Result<(), SessionError> {
            Ok(())
        }
        fn recv(&mut self) -> Result<Vec<u8>, SessionError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn first_contact_pins_and_reports() {
        let mut ts = MemoryTrustStore::new();
        let est = TrustedSession::wrap(StubSession(pid(1)), &mut ts).expect("수립");
        assert_eq!(est.decision, TrustDecision::FirstContact);
        assert_eq!(
            est.session.trust(),
            TrustLevel::Pinned,
            "세션이 아니라 저장소가 신뢰의 출처"
        );
    }

    #[test]
    fn verified_peer_surfaces_verified_trust() {
        let mut ts = MemoryTrustStore::new();
        ts.on_session(pid(1));
        ts.verify(pid(1));
        let est = TrustedSession::wrap(StubSession(pid(1)), &mut ts).expect("수립");
        assert_eq!(est.session.trust(), TrustLevel::FingerprintVerified);
        assert_eq!(
            est.decision,
            TrustDecision::Known(TrustLevel::FingerprintVerified)
        );
    }

    #[test]
    fn blocked_peer_cannot_establish() {
        let mut ts = MemoryTrustStore::new();
        ts.block(pid(9));
        let err = TrustedSession::wrap(StubSession(pid(9)), &mut ts)
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err, SessionError::Blocked, "차단은 수립 자체를 막는다");
    }

    #[test]
    fn wrapped_session_is_transparent_for_io() {
        let mut ts = MemoryTrustStore::new();
        let mut est = TrustedSession::wrap(StubSession(pid(1)), &mut ts).expect("수립");
        assert_eq!(est.session.peer(), pid(1));
        est.session.send(b"x").expect("통과");
    }
}
