//! `LocalDirect` — 실물 로컬 직접 전송(M1-4 · [docs/09] 전송 모드 ①).
//!
//! [`UdpDiscovery`](crate::udp)(발견) + TCP([`TcpLink`](crate::tcp))(세션 링크)를
//! [`Transport`] 트레이트로 묶는다 — **InMemory fake가 서 있던 자리에 그대로 꽂힌다**
//! (같은 트레이트 · 조립 지점은 한 줄 교체).
//!
//! - 발견 관측의 **발신 IP + 광고된 tcp_port**가 연결 주소가 된다. 주소록은 이 크레이트
//!   내부에만 있다([docs/09] 규칙 1 — `Locator`는 전송 밖으로 안 나간다).
//! - **주소는 힌트, 신원은 세션이 확정**한다 — connect가 성공해도 상대 신원은 Noise
//!   핸드셰이크(호출자 몫)가 검증한다.

use crate::tcp::TcpLink;
use crate::transport::{Caps, ConnectError, DiscoveryEvent, PeerHint, Transport};
use crate::udp::UdpDiscovery;
use crate::wire::PacketKind;
use nbeep_core::link::Link;
use nbeep_core::{DisplayName, PeerId};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 실물 로컬 직접 전송 — 발견(S2 멀티캐스트) + TCP 수신/연결.
#[derive(Debug)]
pub struct LocalDirect {
    addrs: Arc<Mutex<HashMap<PeerId, SocketAddr>>>,
    discovery_rx: Mutex<Option<Receiver<DiscoveryEvent>>>,
    incoming_rx: Mutex<Option<Receiver<Box<dyn Link>>>>,
    /// 발견 핸들 — 드롭 시 GOODBYE.
    _discovery: UdpDiscovery,
}

impl LocalDirect {
    /// 발견 광고 + TCP 수신을 시작한다.
    ///
    /// # Errors
    /// TCP 바인딩·발견 소켓 실패 시 `io::Error`.
    pub fn spawn(
        me: PeerId,
        instance: [u8; 16],
        name: DisplayName,
        announce_ms: u32,
        epoch: u64,
    ) -> std::io::Result<Self> {
        // 세션 수신 TCP — 포트 0(임의)·발견 패킷에 광고.
        let listener = TcpListener::bind("0.0.0.0:0")?;
        let tcp_port = listener.local_addr()?.port();

        let discovery = UdpDiscovery::spawn(me, instance, name, tcp_port, epoch, announce_ms)?;

        // ── 발견 관측 → DiscoveryEvent + 주소록 갱신 ──
        let addrs = Arc::new(Mutex::new(HashMap::new()));
        let (disc_tx, discovery_rx) = channel();
        {
            let addrs = Arc::clone(&addrs);
            let events = discovery.take_events();
            std::thread::spawn(move || {
                while let Ok(o) = events.recv() {
                    let peer = o.packet.peer;
                    match o.packet.kind {
                        PacketKind::Goodbye => {
                            addrs.lock().expect("주소록 잠금").remove(&peer);
                            if disc_tx.send(DiscoveryEvent::Vanished(peer)).is_err() {
                                return;
                            }
                        }
                        _ => {
                            let dest = SocketAddr::new(o.from.ip(), o.packet.tcp_port);
                            addrs.lock().expect("주소록 잠금").insert(peer, dest);
                            let hint = PeerHint {
                                peer,
                                name: o.packet.name,
                                caps: Caps::default(),
                            };
                            // 매 관측을 전달 — 수신 측 PeerTable이 병합·생존 갱신한다(FR-D-6).
                            if disc_tx.send(DiscoveryEvent::Appeared(hint)).is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }

        // ── TCP 수신 → Link ──
        let (inc_tx, incoming_rx) = channel::<Box<dyn Link>>();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let Ok(link) = TcpLink::new(stream) else {
                    continue;
                };
                if inc_tx.send(Box::new(link)).is_err() {
                    return;
                }
            }
        });

        Ok(Self {
            addrs,
            discovery_rx: Mutex::new(Some(discovery_rx)),
            incoming_rx: Mutex::new(Some(incoming_rx)),
            _discovery: discovery,
        })
    }
}

impl Transport for LocalDirect {
    fn discovery(&self) -> Receiver<DiscoveryEvent> {
        self.discovery_rx
            .lock()
            .expect("잠금")
            .take()
            .expect("discovery()는 한 번만 호출")
    }

    fn incoming(&self) -> Receiver<Box<dyn Link>> {
        self.incoming_rx
            .lock()
            .expect("잠금")
            .take()
            .expect("incoming()은 한 번만 호출")
    }

    fn connect(&self, peer: PeerId) -> Result<Box<dyn Link>, ConnectError> {
        let addr = self
            .addrs
            .lock()
            .expect("주소록 잠금")
            .get(&peer)
            .copied()
            .ok_or(ConnectError::Unreachable)?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
            .map_err(|_| ConnectError::Unreachable)?;
        let link = TcpLink::new(stream).map_err(|_| ConnectError::Unreachable)?;
        Ok(Box::new(link))
    }

    fn caps(&self) -> Caps {
        Caps::default()
    }
}
