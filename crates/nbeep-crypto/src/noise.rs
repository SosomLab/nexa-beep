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
            return Err(SessionError::TooLarge); // 큰 메시지 청킹은 M4(파일 전송)
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

#[cfg(test)]
mod trust_integration {
    //! 실물 Noise 세션 + TOFU 저장소가 함께 도는 경로([docs/08] §4).
    //! **소켓 없이**(duplex fake) 암호·신뢰 배선을 끝까지 검증한다.

    use super::*;
    use nbeep_core::testkit::duplex;
    use nbeep_core::trust::{MemoryTrustStore, TrustDecision, TrustStore};
    use nbeep_core::trusted::TrustedSession;
    use std::thread;

    /// 두 신원 사이에 실물 Noise 세션을 수립한다(개시자 측을 돌려줌).
    fn establish(alice: &Identity, bob: Identity) -> NoiseSession<impl Link> {
        let (la, lb) = duplex(alice.peer_id(), bob.peer_id());
        let hb = thread::spawn(move || NoiseSession::accept(lb, &bob));
        let a = NoiseSession::initiate(la, alice).expect("a 수립");
        hb.join().unwrap().expect("b 수립");
        a
    }

    #[test]
    fn first_contact_pins_the_authenticated_key() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let bob_id = bob.peer_id();
        let mut ts = MemoryTrustStore::new();

        let session = establish(&alice, bob);
        // 핸드셰이크가 인증한 키가 그대로 TOFU에 핀된다.
        let est = TrustedSession::wrap(session, &mut ts).expect("신뢰 수립");
        assert_eq!(est.decision, TrustDecision::FirstContact);
        assert_eq!(est.session.peer(), bob_id);
        assert_eq!(
            est.session.trust(),
            TrustLevel::Pinned,
            "세션 단독은 Unverified였다"
        );
        assert_eq!(ts.level(bob_id), TrustLevel::Pinned, "저장소에 남는다");
    }

    #[test]
    fn blocked_key_is_rejected_after_handshake() {
        // 핸드셰이크는 성립해도(상대는 막을 수 없다) 수립은 거부된다 — fail-closed.
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let mut ts = MemoryTrustStore::new();
        ts.block(bob.peer_id());

        let session = establish(&alice, bob);
        let err = TrustedSession::wrap(session, &mut ts)
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err, SessionError::Blocked);
    }
}

#[cfg(test)]
mod mux_integration {
    //! 실물 Noise 세션 위 다중화(M2-3) — 암호화 세션 하나로 제어/대화 스트림이 독립 동작.

    use super::*;
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::testkit::duplex;
    use std::thread;

    #[test]
    fn control_and_chat_share_one_encrypted_session() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let bob_id = bob.peer_id();
        let (la, lb) = duplex(alice.peer_id(), bob.peer_id());
        let hb = thread::spawn(move || NoiseSession::accept(lb, &bob));
        let a = NoiseSession::initiate(la, &alice).expect("a 수립");
        let b = hb.join().unwrap().expect("b 수립");

        let (mut ma, mut mb) = (MuxSession::new(a), MuxSession::new(b));
        // 한 세션에서 제어(ack)와 대화가 섞여 흘러도 스트림별로 분리 수신된다.
        ma.send(StreamId::Chat, b"hi bob").unwrap();
        ma.send(StreamId::Control, b"ack:42").unwrap();
        assert_eq!(mb.recv(StreamId::Control).unwrap(), b"ack:42");
        assert_eq!(mb.recv(StreamId::Chat).unwrap(), b"hi bob");
        // 역방향도 동일.
        mb.send(StreamId::Chat, b"hi alice").unwrap();
        assert_eq!(ma.recv(StreamId::Chat).unwrap(), b"hi alice");
        // 위임 확인 — mux가 안쪽 세션의 인증 결과를 그대로 노출한다.
        assert_eq!(ma.peer(), bob_id);
        assert_eq!(mb.trust(), TrustLevel::Unverified);
    }
}

#[cfg(test)]
mod chat_integration {
    //! M2-4 종단 검증 — 실물 Noise 세션 위에서 시퀀서→팬아웃→수신→중복 제거가 한 줄로 돈다.

    use super::*;
    use nbeep_core::chat::{fanout, ChatMessage, DedupIndex, MessageBody, Sequencer};
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::testkit::duplex;
    use nbeep_core::Recipients;
    use std::thread;

    #[test]
    fn encrypted_one_to_one_text_with_dedup() {
        let (alice, bob) = (Identity::generate(), Identity::generate());
        let (a_id, b_id) = (alice.peer_id(), bob.peer_id());
        let (la, lb) = duplex(a_id, b_id);
        let hb = thread::spawn(move || NoiseSession::accept(lb, &bob));
        let a = NoiseSession::initiate(la, &alice).expect("a 수립");
        let b = hb.join().unwrap().expect("b 수립");
        let mut sessions = vec![MuxSession::new(a)];
        let mut mb = MuxSession::new(b);

        // 발신: 시퀀서가 seq를 발급하고, 1:1도 그룹과 같은 fanout 경로를 탄다.
        let mut seq = Sequencer::new();
        let m = ChatMessage {
            sender_device: a_id,
            seq: seq.issue(),
            body: MessageBody::Text("첫 암호화 메시지".into()),
        };
        let report = fanout(&mut sessions, &Recipients::one(b_id), &m);
        assert!(report[0].1.is_ok());
        // 같은 메시지가 다른 경로로 한 번 더 도착한 상황(재전송) 시뮬레이션.
        sessions[0].send(StreamId::Chat, &m.encode()).unwrap();

        // 수신: 복호 → 봉투 해석(발신자 = 세션 인증 상대 검증) → 중복 제거.
        let mut dedup = DedupIndex::new();
        let first = ChatMessage::decode(&mb.recv(StreamId::Chat).unwrap(), a_id).unwrap();
        assert_eq!(first.body, MessageBody::Text("첫 암호화 메시지".into()));
        assert!(dedup.accept(first.sender_device, first.seq), "처음 = 표시");
        let second = ChatMessage::decode(&mb.recv(StreamId::Chat).unwrap(), a_id).unwrap();
        assert_eq!(second, first);
        assert!(
            !dedup.accept(second.sender_device, second.seq),
            "재전송 = 한 번만 표시"
        );
    }
}
