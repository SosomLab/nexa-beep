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
/// 형제 기기 세션(ADR-0015 §3-3 · DR-29) — `psk3` = 세 번째 메시지 뒤에 PSK를 섞는다.
/// 정적 키 교환·`PeerId` 확정은 XX와 같고, PSK가 다르면 msg3 복호에서 실패한다(= 같은
/// 사용자가 아니다). PSK는 임시 DH 뒤에 섞이므로 도청 기록만으로 추측을 검증할 수 없다(온라인 추측만).
const PARAMS_PSK: &str = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s";
/// psk 개시자의 msg1 payload 마커 — psk 패턴은 `e` 뒤 `MixKey(e.pub)`가 있어 msg1 payload가
/// **e.pub에서 파생된 키로 AEAD 봉인**된다(비밀 없이 누구나 열지만 **태그**가 붙는다). 응답자는
/// 이 태그·마커로 XX/XXpsk3를 **추가 왕복 없이** 가른다([docs/48 §3-2] · F-2).
const PSK_MARKER: &[u8] = b"NBPSK1";

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

    /// 키 복제 시나리오 재현용(테스트 한정) — 같은 키 자료의 별개 `Identity`.
    #[cfg(test)]
    pub(crate) fn from_parts_for_test(other: &Identity) -> Self {
        Self {
            private: other.private.clone(),
            public: other.public,
        }
    }

    /// 저장용 키 자료(개인 32B ‖ 공개 32B) — [`crate::keyfile`] 전용(M2-5a).
    /// **로그·전송 금지**(NFR-S-1). 공개키를 함께 저장하는 이유: 개인키에서 공개키를
    /// 유도하려면 X25519 스칼라 곱이 필요한데 snow가 노출하지 않는다 — 쌍을 저장하고,
    /// 불일치(변조)는 핸드셰이크 실패로 드러난다(가용성 문제일 뿐 기밀성 문제가 아니다).
    ///
    /// # Panics
    /// 개인키가 32바이트가 아니면(생성 경로상 불가) 패닉.
    #[must_use]
    pub fn key_bytes(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&self.private);
        out[32..].copy_from_slice(&self.public);
        out
    }

    /// 저장된 키 자료 복원 — [`Self::key_bytes`]의 역. 파일 무결성(매직·길이)은
    /// [`crate::keyfile`]이 거른다.
    #[must_use]
    pub fn from_key_bytes(bytes: &[u8; 64]) -> Self {
        let mut public = [0u8; PeerId::LEN];
        public.copy_from_slice(&bytes[32..]);
        Self {
            private: bytes[..32].to_vec(),
            public,
        }
    }

    /// 래핑 KDF 원료(개인키 32B — ADR-0005 §3 기본 A "기기 키 파생"). **로그 금지.**
    /// 256비트 무작위 키라 메모리-하드 KDF가 불필요하다(암호가 아니다 — 승격 ②만 해당).
    ///
    /// # Panics
    /// 개인키가 32바이트가 아니면(생성 경로상 불가) 패닉.
    #[must_use]
    pub fn wrap_secret(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(&self.private);
        out
    }
}

fn builder(id: &Identity) -> Result<Builder<'_>, SessionError> {
    let params = PARAMS.parse().map_err(|_| SessionError::Handshake)?;
    Ok(Builder::new(params).local_private_key(&id.private))
}

/// XXpsk3 빌더 — PSK 위치 3.
fn builder_psk<'a>(id: &'a Identity, psk: &'a [u8; 32]) -> Result<Builder<'a>, SessionError> {
    let params = PARAMS_PSK.parse().map_err(|_| SessionError::Handshake)?;
    Ok(Builder::new(params)
        .local_private_key(&id.private)
        .psk(3, psk))
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

        Self::finish(link, hs, id)
    }

    /// 개시자 측 **형제 기기** 핸드셰이크(XXpsk3 · ADR-0015 §3-3) — msg1 payload에 [`PSK_MARKER`]를
    /// 실어 응답자가 패턴을 가르게 한다. PSK가 다르면 응답자가 msg3에서 끊는다(이쪽은 그 뒤 첫
    /// 수신에서 `Closed`).
    ///
    /// # Errors
    /// 링크 종료·프로토콜 실패 시 [`SessionError`].
    pub fn initiate_psk(mut link: L, id: &Identity, psk: &[u8; 32]) -> Result<Self, SessionError> {
        let mut hs = builder_psk(id, psk)?
            .build_initiator()
            .map_err(|_| SessionError::Handshake)?;
        let mut buf = vec![0u8; NOISE_MAX];

        let n = hs
            .write_message(PSK_MARKER, &mut buf)
            .map_err(|_| SessionError::Handshake)?;
        link.send(&buf[..n])?;

        let msg = link.recv()?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| SessionError::Handshake)?;

        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|_| SessionError::Handshake)?;
        link.send(&buf[..n])?;

        Self::finish(link, hs, id)
    }

    /// 수신자 측 핸드셰이크(`<- e` / `-> e,ee,s,es` / `<- s,se`) — XX 전용([`Self::accept_any`]의 PSK 없는 판).
    ///
    /// # Errors
    /// 링크 종료·프로토콜 실패 시 [`SessionError`].
    pub fn accept(link: L, id: &Identity) -> Result<Self, SessionError> {
        Self::accept_any(link, id, None).map(|(s, _)| s)
    }

    /// 수신자 측 — **XX와 XXpsk3를 첫 메시지로 가른다**(추가 왕복 0). 돌려주는 `bool` = psk 세션이었는가
    /// (= 상대가 내 KP를 안다 = **같은 사용자**). `psk`가 `None`이면 XX만 받는다(사용자 기능 꺼짐).
    ///
    /// 판별: XXpsk3 응답자 상태로 msg1을 먼저 파싱한다 — psk 패턴은 `e` 뒤 `MixKey(e.pub)`가 있어
    /// payload에 AEAD 태그가 붙는다. 태그가 맞고 payload가 마커면 psk 경로, 아니면 XX 상태로 재파싱.
    /// 비용 = 해시 2회 + AEAD 1회(DH 없음). psk 상태는 파싱 실패 시 버리고 새로 만든다(상태 독립).
    ///
    /// # Errors
    /// 링크 종료·프로토콜 실패(PSK 불일치 포함 — msg3 복호 실패) 시 [`SessionError`].
    pub fn accept_any(
        mut link: L,
        id: &Identity,
        psk: Option<&[u8; 32]>,
    ) -> Result<(Self, bool), SessionError> {
        let mut buf = vec![0u8; NOISE_MAX];
        let msg1 = link.recv()?;

        // ① psk 경로 시도(내 KP가 있을 때만).
        if let Some(psk) = psk {
            let mut hs = builder_psk(id, psk)?
                .build_responder()
                .map_err(|_| SessionError::Handshake)?;
            if let Ok(n) = hs.read_message(&msg1, &mut buf) {
                if &buf[..n] == PSK_MARKER {
                    let n = hs
                        .write_message(&[], &mut buf)
                        .map_err(|_| SessionError::Handshake)?;
                    link.send(&buf[..n])?;
                    let msg3 = link.recv()?;
                    // PSK가 다르면 여기서 실패한다 — "같은 사용자가 아니다"는 정상 결과.
                    hs.read_message(&msg3, &mut buf)
                        .map_err(|_| SessionError::Handshake)?;
                    return Self::finish(link, hs, id).map(|s| (s, true));
                }
            }
        }

        // ② XX 경로(현행).
        let mut hs = builder(id)?
            .build_responder()
            .map_err(|_| SessionError::Handshake)?;
        hs.read_message(&msg1, &mut buf)
            .map_err(|_| SessionError::Handshake)?;

        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|_| SessionError::Handshake)?;
        link.send(&buf[..n])?;

        let msg = link.recv()?;
        hs.read_message(&msg, &mut buf)
            .map_err(|_| SessionError::Handshake)?;

        Self::finish(link, hs, id).map(|s| (s, false))
    }

    fn finish(link: L, hs: HandshakeState, id: &Identity) -> Result<Self, SessionError> {
        let peer = remote_peer(&hs)?;
        if peer == id.peer_id() {
            // D-22 U-P2(사용자 확정 08-08): 상대가 내 신원과 같다 — 자기 연결이거나
            // 키 파일 복제다. 즉시 거부 + 경고 대상([docs/21 §5] I-7).
            return Err(SessionError::SelfPeer);
        }
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

    fn set_recv_timeout(&mut self, dur: Option<core::time::Duration>) {
        let _ = self.link.set_recv_timeout(dur); // 실패해도 블로킹 폴백(다음 recv에서 드러남)
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

    /// ★ M1-11③ tap 평문 부재의 일반화 — 단일 문자열이 아니라 **금칙어 목록**
    /// (이메일·전화·한글 본문·프로필 설정 키)으로 확장. 세션에 실어 보낸 어떤
    /// 민감 문자열도 링크 바이트에 그대로 나타나면 안 된다(FR-S-51).
    #[test]
    fn wire_never_carries_forbidden_plaintext() {
        use nbeep_core::link::{Link, LinkError};
        use std::sync::mpsc::{channel, Receiver, Sender};
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
        let lb = Plain {
            peer: a_id,
            tx: b_tx,
            rx: a_rx,
        };
        let hb = thread::spawn(move || NoiseSession::accept(lb, &bob));
        let mut a = NoiseSession::initiate(la, &alice).expect("a 수립");
        let mut b = hb.join().unwrap().expect("b 수립");
        while sniff_rx.try_recv().is_ok() {}

        // 금칙어 세트 — 프로필·연락처·한글 본문(29 §4-1 취지: 사람이 읽을 것).
        const FORBIDDEN: &[&[u8]] = &[
            b"kiros33@gmail.com",
            b"010-1234-5678",
            b"profile.share.email",
            "홍길동".as_bytes(),
            "비밀 회의는 3시".as_bytes(),
        ];
        for f in FORBIDDEN {
            a.send(f).unwrap();
            assert_eq!(&b.recv().unwrap(), f, "복호는 온전");
        }
        let mut wire = Vec::new();
        while let Ok(fr) = sniff_rx.try_recv() {
            wire.extend_from_slice(&fr);
        }
        assert!(!wire.is_empty());
        for f in FORBIDDEN {
            assert!(
                !wire.windows(f.len()).any(|w| w == *f),
                "링크 바이트에 금칙어 노출: {:?}",
                String::from_utf8_lossy(f)
            );
        }
    }

    /// ★ M1-11④ Debug 마스킹 — Identity의 Debug 출력에 개인키 바이트가 새지
    /// 않는다(docs/13 §7 · 로그는 상태만). 지문(공개)만 허용.
    #[test]
    fn identity_debug_never_leaks_private_key() {
        let id = Identity::generate();
        let dbg = format!("{id:?}");
        // 개인키의 어떤 4바이트 연속도 hex로 나타나지 않아야 한다(전수 검사 —
        // 2바이트는 공개 지문과 우연히 겹칠 확률이 있어 4바이트로 판정).
        for w in id.private.windows(4) {
            let hex_lower = format!("{:02x}{:02x}{:02x}{:02x}", w[0], w[1], w[2], w[3]);
            let hex_upper = hex_lower.to_uppercase();
            assert!(
                !dbg.contains(&hex_lower) && !dbg.contains(&hex_upper),
                "Debug에 개인키 hex 조각: {dbg}"
            );
        }
        assert!(dbg.contains("Identity"), "형식 확인: {dbg}");
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
            importance: nbeep_core::Importance::Normal,
            broadcast: false,
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

#[cfg(test)]
mod clone_guard {
    //! D-22 U-P2 — 자기 신원(복제 키)과의 세션은 수립 자체가 거부된다.

    use super::*;
    use nbeep_core::testkit::duplex;
    use std::thread;

    #[test]
    fn session_with_own_identity_is_rejected() {
        // 키 파일 복제 시나리오: 양쪽이 같은 Identity(같은 개인키)로 핸드셰이크.
        let cloned = Identity::generate();
        let clone2 = Identity {
            // 같은 키 재구성 — 복제 파일을 양쪽이 로드한 상황.
            ..Identity::from_parts_for_test(&cloned)
        };
        let me = cloned.peer_id();
        let (la, lb) = duplex(me, me);
        let hb = thread::spawn(move || NoiseSession::accept(lb, &clone2));
        let a = NoiseSession::initiate(la, &cloned);
        let b = hb.join().unwrap();
        assert_eq!(a.err(), Some(SessionError::SelfPeer), "개시자 거부");
        assert_eq!(b.err(), Some(SessionError::SelfPeer), "수신자 거부");
    }
}

#[cfg(test)]
mod group_fanout {
    //! FR-G-2/G-6 — 그룹이 fanout으로 전 구성원에게 암호화 전송됨을 실물 세션으로 검증.

    use super::*;
    use nbeep_core::chat::{fanout, ChatMessage, MessageBody, Sequencer};
    use nbeep_core::group::GroupStore;
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::testkit::duplex;
    use nbeep_core::DisplayName;
    use std::thread;

    #[test]
    fn group_send_reaches_all_members_via_fanout() {
        let me = Identity::generate();
        let bob = Identity::generate();
        let carol = Identity::generate();
        let (bob_id, carol_id) = (bob.peer_id(), carol.peer_id());

        // 그룹 구성(로컬 개념) — bob·carol.
        let mut groups = GroupStore::new();
        let g = groups.create(DisplayName::parse("팀").unwrap());
        groups.add_member(g, bob_id);
        groups.add_member(g, carol_id);

        // 각 구성원과 실물 Noise 세션 + 수신 스레드(에코 아님 — 도달만 확인).
        let (lb_a, lb_b) = duplex(me.peer_id(), bob_id);
        let (lc_a, lc_b) = duplex(me.peer_id(), carol_id);
        let hb = thread::spawn(move || {
            let s = NoiseSession::accept(lb_b, &bob).unwrap();
            let mut m = MuxSession::new(s);
            m.recv(StreamId::Chat).unwrap()
        });
        let hc = thread::spawn(move || {
            let s = NoiseSession::accept(lc_b, &carol).unwrap();
            let mut m = MuxSession::new(s);
            m.recv(StreamId::Chat).unwrap()
        });
        let sb = MuxSession::new(NoiseSession::initiate(lb_a, &me).unwrap());
        let sc = MuxSession::new(NoiseSession::initiate(lc_a, &me).unwrap());
        let mut sessions = vec![sb, sc];

        // 그룹 발신 — recipients()가 fanout으로(FR-G-6).
        let mut seq = Sequencer::new();
        let msg = ChatMessage {
            sender_device: me.peer_id(),
            seq: seq.issue(),
            body: MessageBody::Text("팀 전체 공지".into()),
            importance: nbeep_core::Importance::Normal,
            broadcast: false,
        };
        let report = fanout(&mut sessions, &groups.get(g).unwrap().recipients(), &msg);
        assert!(
            report.iter().all(|(_, r)| r.is_ok()),
            "전원 전달: {report:?}"
        );

        // 두 구성원 모두 복호·수신했는가.
        let got_b = ChatMessage::decode(&hb.join().unwrap(), me.peer_id()).unwrap();
        let got_c = ChatMessage::decode(&hc.join().unwrap(), me.peer_id()).unwrap();
        assert_eq!(got_b.body, MessageBody::Text("팀 전체 공지".into()));
        assert_eq!(got_c, got_b, "전 구성원 동일 메시지");
    }
}

#[cfg(test)]
mod psk_tests {
    //! ADR-0015 §3-3 · docs/48 T-1 — XX/XXpsk3 판별·불일치·폴백(소켓 없이 duplex fake).
    #![allow(clippy::unwrap_used)]
    use super::*;
    use nbeep_core::testkit::duplex;

    fn ids() -> (Identity, Identity) {
        (Identity::generate(), Identity::generate())
    }

    /// 같은 PSK = 성립 · 응답자가 psk 세션임을 안다 · 데이터 왕복.
    #[test]
    fn psk_both_sides_establish_and_flag() {
        let (a, b) = ids();
        let (la, lb) = duplex(a.peer_id(), b.peer_id());
        let psk = [7u8; 32];
        let bp = b.peer_id();
        let t = std::thread::spawn(move || NoiseSession::accept_any(lb, &b, Some(&psk)));
        let mut sa = NoiseSession::initiate_psk(la, &a, &psk).unwrap();
        let (mut sb, via_psk) = t.join().unwrap().unwrap();
        assert!(via_psk, "psk 경로 판별");
        assert_eq!(sa.peer(), bp);
        assert_eq!(sb.peer(), a.peer_id());
        sa.send(b"hi").unwrap();
        assert_eq!(sb.recv().unwrap(), b"hi");
    }

    /// XX 개시자 → psk를 가진 응답자: 태그 실패 → XX 폴백 · psk 아님 표시.
    #[test]
    fn xx_initiator_falls_back_on_psk_responder() {
        let (a, b) = ids();
        let (la, lb) = duplex(a.peer_id(), b.peer_id());
        let psk = [9u8; 32];
        let t = std::thread::spawn(move || NoiseSession::accept_any(lb, &b, Some(&psk)));
        let mut sa = NoiseSession::initiate(la, &a).unwrap();
        let (mut sb, via_psk) = t.join().unwrap().unwrap();
        assert!(!via_psk, "XX 폴백");
        sa.send(b"plain").unwrap();
        assert_eq!(sb.recv().unwrap(), b"plain");
    }

    /// PSK 불일치 = 응답자가 msg3에서 실패(같은 사용자가 아니다) — 개시자는 세션을 못 쓴다.
    #[test]
    fn psk_mismatch_fails_at_responder() {
        let (a, b) = ids();
        let (la, lb) = duplex(a.peer_id(), b.peer_id());
        let t = std::thread::spawn(move || NoiseSession::accept_any(lb, &b, Some(&[1u8; 32])));
        let sa = NoiseSession::initiate_psk(la, &a, &[2u8; 32]);
        let rb = t.join().unwrap();
        assert!(rb.is_err(), "응답자 = 실패(PSK 다름)");
        // 개시자는 msg3까지 보내고 transport로 들어갈 수 있으나 상대가 끊어 첫 수신이 실패한다.
        if let Ok(mut sa) = sa {
            assert!(sa.recv().is_err());
        }
    }

    /// psk 개시자 → PSK 없는 응답자(사용자 기능 꺼짐): 응답자는 XX로 파싱 → 개시자 msg2 복호 실패.
    #[test]
    fn psk_initiator_vs_plain_responder_fails() {
        let (a, b) = ids();
        let (la, lb) = duplex(a.peer_id(), b.peer_id());
        let t = std::thread::spawn(move || NoiseSession::accept_any(lb, &b, None));
        let sa = NoiseSession::initiate_psk(la, &a, &[3u8; 32]);
        let _ = t.join().unwrap();
        assert!(sa.is_err(), "개시자는 msg2에서 실패(패턴 다름)");
    }

    /// 빈 psk(None) 응답자 = 현행 XX와 동일 경로.
    #[test]
    fn accept_without_psk_is_plain_xx() {
        let (a, b) = ids();
        let (la, lb) = duplex(a.peer_id(), b.peer_id());
        let t = std::thread::spawn(move || NoiseSession::accept_any(lb, &b, None));
        let _sa = NoiseSession::initiate(la, &a).unwrap();
        let (_sb, via) = t.join().unwrap().unwrap();
        assert!(!via);
    }
}
