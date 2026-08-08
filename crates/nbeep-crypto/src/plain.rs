//! `PlainSession` — 세션 **스텁**([docs/13] §11-1 · [docs/09] §6).
//!
//! `nbeep_core`의 `Session`을 **암호화 없이** 구현한다 — 상위 계층(대화·그룹·전송)을 실물 Noise 없이
//! 먼저 개발·검증하기 위한 test double. 핸드셰이크는 `PeerId`를 평문 교환하는 흉내이고, `send`/`recv`는
//! 링크로 프레임을 그대로 통과시킨다.
//!
//! ⚠️ **암호화하지 않는다 · 인증하지 않는다.** 실물 `NoiseSession`(Noise_XX)은 M2-1b에서
//! 같은 `Session` 트레이트로 들어온다 — 상위 코드는 스텁↔실물 교체에 바뀌지 않는다.
//! `testkit` feature(또는 테스트 빌드)에서만 컴파일 — 릴리스 미포함.

use nbeep_core::link::Link;
use nbeep_core::session::{Session, SessionError};
use nbeep_core::{PeerId, TrustLevel};

/// 암호화 없는 세션 스텁. `initiate`/`accept`로 수립한다.
#[derive(Debug)]
pub struct PlainSession<L: Link> {
    link: L,
    peer: PeerId,
    trust: TrustLevel,
}

impl<L: Link> PlainSession<L> {
    /// 개시자 측 수립 — 내 `PeerId`를 먼저 보내고 상대 것을 받는다.
    ///
    /// # Errors
    /// 링크가 닫혔거나 상대 인사가 32바이트가 아니면 [`SessionError`].
    pub fn initiate(mut link: L, me: PeerId) -> Result<Self, SessionError> {
        link.send(me.as_bytes())?;
        let peer = recv_peer(&mut link)?;
        Ok(Self {
            link,
            peer,
            // 스텁은 인증하지 않는다 — 미검증으로 둔다(실물 TOFU는 M2-2).
            trust: TrustLevel::Unverified,
        })
    }

    /// 수신자 측 수립 — 상대 `PeerId`를 먼저 받고 내 것을 보낸다.
    ///
    /// # Errors
    /// 링크가 닫혔거나 상대 인사가 32바이트가 아니면 [`SessionError`].
    pub fn accept(mut link: L, me: PeerId) -> Result<Self, SessionError> {
        let peer = recv_peer(&mut link)?;
        link.send(me.as_bytes())?;
        Ok(Self {
            link,
            peer,
            trust: TrustLevel::Unverified,
        })
    }
}

fn recv_peer(link: &mut impl Link) -> Result<PeerId, SessionError> {
    let hello = link.recv()?;
    let bytes: [u8; PeerId::LEN] = hello.try_into().map_err(|_| SessionError::Handshake)?;
    Ok(PeerId::from_bytes(bytes))
}

impl<L: Link> Session for PlainSession<L> {
    fn peer(&self) -> PeerId {
        self.peer
    }
    fn trust(&self) -> TrustLevel {
        self.trust
    }
    fn send(&mut self, message: &[u8]) -> Result<(), SessionError> {
        // 스텁: 평문 통과. 실물은 여기서 AEAD 암호화.
        self.link.send(message)?;
        Ok(())
    }
    fn recv(&mut self) -> Result<Vec<u8>, SessionError> {
        Ok(self.link.recv()?)
    }
    fn set_recv_timeout(&mut self, dur: Option<core::time::Duration>) {
        let _ = self.link.set_recv_timeout(dur);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbeep_core::testkit::duplex;
    use std::thread;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    #[test]
    fn handshake_learns_peer_and_exchanges_messages() {
        let (la, lb) = duplex(pid(1), pid(2));

        // 양쪽 핸드셰이크를 스레드로(초기화 프레임 교환).
        let ha = thread::spawn(move || PlainSession::initiate(la, pid(1)));
        let hb = thread::spawn(move || PlainSession::accept(lb, pid(2)));
        let mut a = ha.join().unwrap().expect("a 수립");
        let mut b = hb.join().unwrap().expect("b 수립");

        // 서로의 PeerId를 배웠다.
        assert_eq!(a.peer(), pid(2));
        assert_eq!(b.peer(), pid(1));
        // 스텁은 미검증.
        assert_eq!(a.trust(), TrustLevel::Unverified);

        // 메시지 왕복.
        a.send(b"hello").unwrap();
        assert_eq!(b.recv().unwrap(), b"hello");
        b.send(b"hi").unwrap();
        assert_eq!(a.recv().unwrap(), b"hi");
    }

    #[test]
    fn recv_after_peer_drops_is_closed() {
        let (la, lb) = duplex(pid(1), pid(2));
        let hb = thread::spawn(move || PlainSession::accept(lb, pid(2)));
        let mut a = PlainSession::initiate(la, pid(1)).expect("a 수립");
        let b = hb.join().unwrap().expect("b 수립");
        drop(b);
        assert_eq!(a.recv().err(), Some(SessionError::Closed));
    }
}
