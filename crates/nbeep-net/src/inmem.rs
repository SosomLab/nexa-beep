//! `InMemoryTransport` — 소켓 없는 전송 **fake**([docs/13] §11-1 · [docs/09] §6).
//!
//! 여러 노드가 하나의 `InMemoryBus`에 참여해 **서로를 발견**하고 채널로 **연결**한다.
//! 이걸로 세션·대화·그룹·무해화 로직을 **네트워크 없이** 단위 테스트한다 — 4타깃 CI에서 실제
//! 멀티캐스트를 쏠 수 없다는 현실(NFR-O-2)의 해답. 실물 `LocalDirectTransport`는 M1-2~4.
//!
//! `testkit` feature(또는 테스트 빌드)에서만 컴파일 — 릴리스 바이너리에 들어가지 않는다.
//! 테스트 지원 코드이므로 `Mutex`/채널 `unwrap`을 허용한다(포이즌 = 프로그래머 오류).
#![allow(clippy::unwrap_used)]

use crate::transport::{Caps, ConnectError, DiscoveryEvent, PeerHint, Transport};
use nbeep_core::link::{Link, LinkError};
use nbeep_core::{DisplayName, PeerId};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// 버스에 등록된 한 노드의 수신 채널.
struct Node {
    hint: PeerHint,
    discovery_tx: Sender<DiscoveryEvent>,
    incoming_tx: Sender<Box<dyn Link>>,
}

/// 인메모리 발견 버스 — 노드들이 여기 참여해 서로를 본다. `Arc`로 공유한다.
#[derive(Default)]
pub struct InMemoryBus {
    nodes: Mutex<HashMap<PeerId, Node>>,
}

impl std::fmt::Debug for InMemoryBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.nodes.lock().unwrap().len();
        f.debug_struct("InMemoryBus").field("nodes", &n).finish()
    }
}

impl InMemoryBus {
    /// 빈 버스.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 노드로 참여한다. 기존 노드와 **양방향으로 서로를 발견**하고 [`InMemoryTransport`]를 돌려준다.
    pub fn join(
        self: &Arc<Self>,
        peer: PeerId,
        name: DisplayName,
        caps: Caps,
    ) -> InMemoryTransport {
        let (disc_tx, disc_rx) = channel();
        let (inc_tx, inc_rx) = channel();
        let hint = PeerHint { peer, name, caps };

        {
            let mut nodes = self.nodes.lock().unwrap();
            for existing in nodes.values() {
                // 새 노드는 기존 노드를, 기존 노드는 새 노드를 본다.
                disc_tx
                    .send(DiscoveryEvent::Appeared(existing.hint.clone()))
                    .ok();
                existing
                    .discovery_tx
                    .send(DiscoveryEvent::Appeared(hint.clone()))
                    .ok();
            }
            nodes.insert(
                peer,
                Node {
                    hint,
                    discovery_tx: disc_tx,
                    incoming_tx: inc_tx,
                },
            );
        }

        InMemoryTransport {
            bus: Arc::clone(self),
            me: peer,
            caps,
            discovery_rx: Mutex::new(Some(disc_rx)),
            incoming_rx: Mutex::new(Some(inc_rx)),
        }
    }

    fn leave(&self, peer: PeerId) {
        let mut nodes = self.nodes.lock().unwrap();
        nodes.remove(&peer);
        for n in nodes.values() {
            n.discovery_tx.send(DiscoveryEvent::Vanished(peer)).ok();
        }
    }

    fn incoming_sender(&self, peer: PeerId) -> Option<Sender<Box<dyn Link>>> {
        self.nodes
            .lock()
            .unwrap()
            .get(&peer)
            .map(|n| n.incoming_tx.clone())
    }
}

/// 한 노드의 전송 핸들. 드롭 시 버스에서 이탈(다른 노드에 `Vanished` 통지).
pub struct InMemoryTransport {
    bus: Arc<InMemoryBus>,
    me: PeerId,
    caps: Caps,
    discovery_rx: Mutex<Option<Receiver<DiscoveryEvent>>>,
    incoming_rx: Mutex<Option<Receiver<Box<dyn Link>>>>,
}

impl std::fmt::Debug for InMemoryTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryTransport")
            .field("me", &self.me)
            .finish()
    }
}

impl Transport for InMemoryTransport {
    fn discovery(&self) -> Receiver<DiscoveryEvent> {
        self.discovery_rx
            .lock()
            .unwrap()
            .take()
            .expect("discovery()는 한 번만 호출한다")
    }

    fn incoming(&self) -> Receiver<Box<dyn Link>> {
        self.incoming_rx
            .lock()
            .unwrap()
            .take()
            .expect("incoming()은 한 번만 호출한다")
    }

    fn connect(&self, peer: PeerId) -> Result<Box<dyn Link>, ConnectError> {
        let target = self
            .bus
            .incoming_sender(peer)
            .ok_or(ConnectError::Unreachable)?;
        // 채널 두 쌍으로 양방향 링크를 만든다.
        let (a_tx, a_rx) = channel::<Vec<u8>>(); // me → peer
        let (b_tx, b_rx) = channel::<Vec<u8>>(); // peer → me
                                                 // 상대에게 넘길 반쪽: 상대 입장에서 이 링크는 "우리(me)를 향한" 것.
        let their = InMemoryLink {
            peer: self.me,
            tx: b_tx,
            rx: a_rx,
        };
        target
            .send(Box::new(their))
            .map_err(|_| ConnectError::Unreachable)?;
        // 우리 반쪽: peer를 향한 링크.
        Ok(Box::new(InMemoryLink {
            peer,
            tx: a_tx,
            rx: b_rx,
        }))
    }

    fn caps(&self) -> Caps {
        self.caps
    }
}

impl Drop for InMemoryTransport {
    fn drop(&mut self) {
        self.bus.leave(self.me);
    }
}

/// 채널로 이어진 인메모리 링크(바이트 관).
struct InMemoryLink {
    peer: PeerId,
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
}

impl Link for InMemoryLink {
    fn peer(&self) -> PeerId {
        self.peer
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        self.tx.send(frame.to_vec()).map_err(|_| LinkError::Closed)
    }
    fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        self.rx.recv().map_err(|_| LinkError::Closed)
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

    fn drain(rx: &Receiver<DiscoveryEvent>) -> Vec<DiscoveryEvent> {
        let mut v = Vec::new();
        while let Ok(e) = rx.try_recv() {
            v.push(e);
        }
        v
    }

    #[test]
    fn two_nodes_discover_each_other() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("alice-mac"), Caps::default());
        let b = bus.join(pid(2), name("bob-pc"), Caps::default());

        let a_disc = a.discovery();
        let b_disc = b.discovery();

        // a는 (나중에 참여한) b를, b는 (먼저 있던) a를 본다.
        let a_events = drain(&a_disc);
        let b_events = drain(&b_disc);
        assert!(
            a_events
                .iter()
                .any(|e| matches!(e, DiscoveryEvent::Appeared(h) if h.peer == pid(2))),
            "a가 b를 발견: {a_events:?}"
        );
        assert!(
            b_events
                .iter()
                .any(|e| matches!(e, DiscoveryEvent::Appeared(h) if h.peer == pid(1))),
            "b가 a를 발견: {b_events:?}"
        );
    }

    #[test]
    fn connect_delivers_bidirectional_link() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("alice"), Caps::default());
        let b = bus.join(pid(2), name("bob"), Caps::default());
        let b_incoming = b.incoming();

        // a가 b에게 연결.
        let mut a_link = a.connect(pid(2)).expect("연결 성공");
        assert_eq!(a_link.peer(), pid(2), "우리 링크는 상대(b)를 향한다");

        // b는 수신 링크를 받는다 — 그 링크는 a를 향한다.
        let mut b_link = b_incoming.recv().expect("수신 링크 도착");
        assert_eq!(b_link.peer(), pid(1), "b가 받은 링크는 a를 향한다");

        // 양방향 송수신.
        a_link.send(b"hello").unwrap();
        assert_eq!(b_link.recv().unwrap(), b"hello");
        b_link.send(b"hi back").unwrap();
        assert_eq!(a_link.recv().unwrap(), b"hi back");
    }

    #[test]
    fn connect_unknown_peer_is_unreachable() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("alice"), Caps::default());
        assert_eq!(a.connect(pid(99)).err(), Some(ConnectError::Unreachable));
    }

    #[test]
    fn leaving_notifies_others_vanished() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("alice"), Caps::default());
        let a_disc = a.discovery();
        let b = bus.join(pid(2), name("bob"), Caps::default());
        // a가 b의 Appeared를 소진.
        let _ = drain(&a_disc);
        // b가 떠나면 a는 Vanished(2)를 받는다.
        drop(b);
        let events = drain(&a_disc);
        assert!(
            events.contains(&DiscoveryEvent::Vanished(pid(2))),
            "b 이탈 통지: {events:?}"
        );
    }

    #[test]
    fn link_recv_errors_when_peer_drops() {
        let bus = InMemoryBus::new();
        let a = bus.join(pid(1), name("alice"), Caps::default());
        let b = bus.join(pid(2), name("bob"), Caps::default());
        let b_incoming = b.incoming();
        let mut a_link = a.connect(pid(2)).unwrap();
        let b_link = b_incoming.recv().unwrap();
        drop(b_link); // 상대가 링크를 닫음
        assert_eq!(a_link.recv().err(), Some(LinkError::Closed));
    }
}
