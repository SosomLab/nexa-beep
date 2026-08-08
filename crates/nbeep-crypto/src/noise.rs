//! `NoiseSession` — 실물 보안 세션([docs/08] ADR-0002 · DR-11).
//!
//! **`Noise_XX_25519_ChaChaPoly_BLAKE2s`** — 사전 등록 없는(DR-1) 우리 상황에서 양쪽이 핸드셰이크 중
//! 정적 공개키를 교환하는 `XX`가 유일한 선택이다([docs/08] §4). 암호 자체는 직접 구현하지 않고
//! 검증된 [`snow`] 프레임워크에 위임한다(NFR-S-3).
//!
//! **`PeerId` = X25519 정적 공개키**(32바이트) — 핸드셰이크가 상대의 키 소유를 증명하므로 `peer()`는
//! **암호학적으로 인증된** 값이다. 단 **TOFU 핀·SAS 대조는 M2-2** 소관이라, 여기서 `trust()`는
//! 기본 [`TrustLevel::Unverified`]다(핸드셰이크 성립 ≠ 신뢰 확정).

use nbeep_core::link::Link;
use nbeep_core::session::{Session, SessionError};
use nbeep_core::{PeerId, TrustLevel};
use snow::{Builder, HandshakeState, TransportState};

/// Noise 프로토콜 파라미터([docs/08] — X25519 / ChaCha20-Poly1305 / BLAKE2s).
const PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

/// Noise 메시지 상한(프레임워크 제약). 페이로드는 태그(16B)만큼 작아야 한다.
const NOISE_MAX: usize = 65535;
const TAG_LEN: usize = 16;

/// 기기 장기 신원 — X25519 정적 키쌍. **공개키가 곧 [`PeerId`]**(DR-8).
///
/// 개인키는 **로컬에만**(NFR-S-1). 저장·로딩은 `nbeep-store`(M2-5) 소관이며 여기서는 메모리 표현만 든다.
pub struct Identity {
    private: Vec<u8>,
    public: [u8; PeerId::LEN],
}

impl core::fmt::Debug for Identity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 개인키를 찍지 않는다(docs/13 §7). 공개 지문만.
        f.debug_struct("Identity")
            .field("peer", &self.peer_id())
            .field("private", &"[redacted]")
            .finish()
    }
}

impl Identity {
    /// 새 정적 키쌍을 생성한다(OS 난수 — snow 내부).
    ///
    /// # Panics
    /// Noise 파라미터가 유효하지 않거나 키 생성에 실패하면(사실상 불가) 패닉.
    #[must_use]
    pub fn generate() -> Self {
        let kp = Builder::new(PARAMS.parse().expect("유효한 Noise 파라미터"))
            .generate_keypair()
            .expect("X25519 키 생성");
        let public: [u8; PeerId::LEN] = kp.public.try_into().expect("32바이트 X25519 공개키");
        Self {
            private: kp.private,
            public,
        }
    }

    /// 이 신원의 `PeerId`(= X25519 공개키).
    #[must_use]
    pub fn peer_id(&self) -> PeerId {
        PeerId::from_bytes(self.public)
    }
}

fn builder(id: &Identity) -> Result<Builder<'_>, SessionError> {
    let params = PARAMS.parse().map_err(|_| SessionError::Handshake)?;
    Ok(Builder::new(params).local_private_key(&id.private))
}

fn remote_peer(hs: &HandshakeState) -> Result<PeerId, SessionError> {
    let remote = hs.get_remote_static().ok_or(SessionError::Handshake)?;
    let bytes: [u8; PeerId::LEN] = remote.try_into().map_err(|_| SessionError::Handshake)?;
    Ok(PeerId::from_bytes(bytes))
}

/// Noise_XX로 인증·암호화된 세션. `initiate`/`accept`로 수립한다.
pub struct NoiseSession<L: Link> {
    link: L,
    transport: TransportState,
    peer: PeerId,
}

impl<L: Link> core::fmt::Debug for NoiseSession<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NoiseSession")
            .field("peer", &self.peer)
            .finish()
    }
}

impl<L: Link> NoiseSession<L> {
    /// 개시자 측 핸드셰이크(`-> e` / `<- e,ee,s,es` / `-> s,se`).
    ///
    /// # Errors
    /// 링크 종료·프로토콜 실패 시 [`SessionError`].
    pub fn initiate(mut link: L, id: &Identity) -> Result<Self, SessionError> {
        let mut hs = builder(id)?
            .build_initiator()
            .map_err(|_| SessionError::Handshake)?;
        let mut buf = vec![0u8; NOISE_MAX];

        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|_| SessionError::Handshake)?;
        link.send(&buf[..n])?;

        let msg = link.recv()?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| SessionError::Handshake)?;

        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|_| SessionError::Handshake)?;
        link.send(&buf[..n])?;

        Self::finish(link, hs)
    }

    /// 수신자 측 핸드셰이크(`<- e` / `-> e,ee,s,es` / `<- s,se`).
    ///
    /// # Errors
    /// 링크 종료·프로토콜 실패 시 [`SessionError`].
    pub fn accept(mut link: L, id: &Identity) -> Result<Self, SessionError> {
        let mut hs = builder(id)?
            .build_responder()
            .map_err(|_| SessionError::Handshake)?;
        let mut buf = vec![0u8; NOISE_MAX];

        let msg = link.recv()?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| SessionError::Handshake)?;

        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|_| SessionError::Handshake)?;
        link.send(&buf[..n])?;

        let msg = link.recv()?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| SessionError::Handshake)?;

        Self::finish(link, hs)
    }

    fn finish(link: L, hs: HandshakeState) -> Result<Self, SessionError> {
        let peer = remote_peer(&hs)?;
        let transport = hs
            .into_transport_mode()
            .map_err(|_| SessionError::Handshake)?;
        Ok(Self {
            link,
            transport,
            peer,
        })
    }
}

impl<L: Link> Session for NoiseSession<L> {
    fn peer(&self) -> PeerId {
        self.peer
    }

    fn trust(&self) -> TrustLevel {
        // 핸드셰이크는 키를 인증하지만, TOFU 핀·SAS 대조는 M2-2 소관.
        TrustLevel::Unverified
    }

    fn send(&mut self, message: &[u8]) -> Result<(), SessionError> {
        if message.len() > NOISE_MAX - TAG_LEN {
            return Err(SessionError::TooLarge); // 큰 메시지 청킹은 M2-3
        }
        let mut out = vec![0u8; message.len() + TAG_LEN];
        let n = self
            .transport
            .write_message(message, &mut out)
            .map_err(|_| SessionError::Closed)?;
        self.link.send(&out[..n])?;
        Ok(())
    }

    fn recv(&mut self) -> Result<Vec<u8>, SessionError> {
        let ciphertext = self.link.recv()?;
        if ciphertext.len() > NOISE_MAX {
            return Err(SessionError::TooLarge);
        }
        let mut out = vec![0u8; ciphertext.len()];
        let n = self
            .transport
            .read_message(&ciphertext, &mut out)
            .map_err(|_| SessionError::Closed)?;
        out.truncate(n);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbeep_core::testkit::duplex;
    use std::thread;

    #[test]
    fn handshake_authenticates_peers_and_encrypts() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let a_id = alice.peer_id();
        let b_id = bob.peer_id();
        let (la, lb) = duplex(a_id, b_id);

        let hb = thread::spawn(move || NoiseSession::accept(lb, &bob));
        let mut a = NoiseSession::initiate(la, &alice).expect("a 수립");
        let mut b = hb.join().unwrap().expect("b 수립");

        // 핸드셰이크가 상대의 정적 공개키(=PeerId)를 인증했다.
        assert_eq!(a.peer(), b_id, "a는 b의 키를 인증");
        assert_eq!(b.peer(), a_id, "b는 a의 키를 인증");

        // 암호화된 왕복.
        a.send(b"secret message").unwrap();
        assert_eq!(b.recv().unwrap(), b"secret message");
        b.send(b"reply").unwrap();
        assert_eq!(a.recv().unwrap(), b"reply");
    }

    #[test]
    fn ciphertext_on_the_wire_is_not_plaintext() {
        // 링크에 실제로 흐르는 바이트가 평문이 아님을 확인 — 별도 fake 링크로 가로채기.
        use nbeep_core::link::{Link, LinkError};
        use std::sync::mpsc::{channel, Receiver, Sender};

        // a→b 프레임을 가로채는 링크(테스트 전용).
        struct Tap {
            peer: PeerId,
            tx: Sender<Vec<u8>>,
            rx: Receiver<Vec<u8>>,
            sniff: Sender<Vec<u8>>,
        }
        impl Link for Tap {
            fn peer(&self) -> PeerId {
                self.peer
            }
            fn send(&mut self, f: &[u8]) -> Result<(), LinkError> {
                self.sniff.send(f.to_vec()).ok();
                self.tx.send(f.to_vec()).map_err(|_| LinkError::Closed)
            }
            fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
                self.rx.recv().map_err(|_| LinkError::Closed)
            }
        }

        let alice = Identity::generate();
        let bob = Identity::generate();
        let (a_id, b_id) = (alice.peer_id(), bob.peer_id());
        let (a_tx, a_rx) = channel();
        let (b_tx, b_rx) = channel();
        let (sniff_tx, sniff_rx) = channel();
        let la = Tap {
            peer: b_id,
            tx: a_tx,
            rx: b_rx,
            sniff: sniff_tx,
        };
        let lb = {
            struct Plain {
                peer: PeerId,
                tx: Sender<Vec<u8>>,
                rx: Receiver<Vec<u8>>,
            }
            impl Link for Plain {
                fn peer(&self) -> PeerId {
                    self.peer
                }
                fn send(&mut self, f: &[u8]) -> Result<(), LinkError> {
                    self.tx.send(f.to_vec()).map_err(|_| LinkError::Closed)
                }
                fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
                    self.rx.recv().map_err(|_| LinkError::Closed)
                }
            }
            Plain {
                peer: a_id,
                tx: b_tx,
                rx: a_rx,
            }
        };

        let hb = thread::spawn(move || NoiseSession::accept(lb, &bob));
        let mut a = NoiseSession::initiate(la, &alice).expect("a 수립");
        let mut b = hb.join().unwrap().expect("b 수립");
        // 핸드셰이크 중 흐른 스니핑 프레임을 비운다.
        while sniff_rx.try_recv().is_ok() {}

        a.send(b"topsecret").unwrap();
        assert_eq!(b.recv().unwrap(), b"topsecret");
        let on_wire = sniff_rx.try_recv().expect("전송된 프레임");
        assert!(
            !on_wire.windows(9).any(|w| w == b"topsecret"),
            "평문이 링크에 노출되면 안 된다"
        );
    }

    #[test]
    fn distinct_identities_have_distinct_peer_ids() {
        assert_ne!(
            Identity::generate().peer_id(),
            Identity::generate().peer_id()
        );
    }
}
