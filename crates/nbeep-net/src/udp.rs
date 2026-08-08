//! S2 — IPv4 멀티캐스트 발견(**첫 실물 소켓** · M1-4 슬라이스 1 · [docs/06 §4]).
//!
//! [`wire`](crate::wire) 패킷을 실제 UDP 멀티캐스트로 쏘고 받는다. S1(IPv6)·S3(브로드캐스트)·
//! S4(유니캐스트 프로브)는 후속 슬라이스 — 폴백 사다리의 뼈대는 같고 소켓만 다르다.
//!
//! - **자기 패킷 필터** — 멀티캐스트는 루프백된다. **키(`PeerId`)로 거른다**([docs/08 §5] —
//!   주소·포트가 아니라 신원 기준. 같은 호스트의 다른 인스턴스는 걸러지지 않아야 한다).
//! - **`SO_REUSEADDR`(+unix `SO_REUSEPORT`)** — 같은 호스트에서 여러 인스턴스가 같은 발견
//!   포트에 바인딩(개발·테스트 필수, socket2 — std 미노출 옵션).
//! - **타이밍은 전부 주입**(announce 주기) 또는 **잠정 상수**(그룹·포트·TTL — ⚠️ D-8b 실측 후
//!   확정, [docs/08 §8]). 잠정치는 상수 주석에 명시한다.
//! - 종료 시 **GOODBYE 2회**(FR-D-8 명시적 이탈 — 유실 대비 중복. 수신 측 중복은 무해).

use crate::wire::{Decoded, Packet, PacketKind};
use nbeep_core::{DisplayName, PeerId};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// 발견 멀티캐스트 그룹(잠정 — D-8b 실측 후 확정). 239.255/16 = 조직 로컬 범위.
pub const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 77, 77);
/// 발견 포트(잠정 — D-8b).
pub const DISCOVERY_PORT: u16 = 47_100;
/// 멀티캐스트 TTL(잠정 — 링크 로컬 1홉. 라우팅 확장은 S6/릴레이 몫).
const TTL: u32 = 1;
/// 수신 폴링 간격(정지 플래그 확인용).
const RECV_POLL: Duration = Duration::from_millis(300);

/// 수신된 발견 관측 — 해석된 패킷 + 발신 주소(연결 힌트).
#[derive(Debug)]
pub struct Observation {
    /// 해석된 패킷(자기 자신은 이미 걸러짐 · **미검증 힌트**).
    pub packet: Packet,
    /// 발신 소켓 주소(세션 연결의 주소 힌트 — 신원 아님).
    pub from: SocketAddr,
}

/// S2 발견 노드 — 주기 광고 + 수신. 드롭 시 GOODBYE.
#[derive(Debug)]
pub struct UdpDiscovery {
    events: std::sync::Mutex<Option<Receiver<Observation>>>,
    stop: Arc<AtomicBool>,
    send_sock: UdpSocket,
    /// GOODBYE에 넣을 내 광고 원본(seq는 전송 시 갱신).
    template: Packet,
    seq: Arc<AtomicU32>,
}

impl UdpDiscovery {
    /// 발견을 시작한다 — 광고 스레드(주기 `announce_ms`) + 수신 스레드.
    ///
    /// # Errors
    /// 소켓 생성·바인딩·그룹 가입 실패 시 `io::Error`(방화벽·권한·인터페이스 부재).
    pub fn spawn(
        me: PeerId,
        instance: [u8; 16],
        name: DisplayName,
        tcp_port: u16,
        epoch: u64,
        announce_ms: u32,
    ) -> std::io::Result<Self> {
        // ── 수신 소켓: 발견 포트에 재사용 바인딩 + 그룹 가입 ──
        let recv = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        recv.set_reuse_address(true)?;
        #[cfg(unix)]
        recv.set_reuse_port(true)?; // 같은 호스트 다중 인스턴스(개발·테스트)
        recv.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).into())?;
        recv.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
        recv.set_multicast_loop_v4(true)?; // 같은 호스트 인스턴스 간 도달
        let recv: UdpSocket = recv.into();
        recv.set_read_timeout(Some(RECV_POLL))?;

        // ── 송신 소켓: 임의 포트(발신 주소가 연결 힌트가 된다) ──
        let send_sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        send_sock.set_multicast_ttl_v4(TTL)?;
        send_sock.set_multicast_loop_v4(true)?;

        let template = Packet {
            kind: PacketKind::Announce,
            flags: 0,
            peer: me,
            tcp_port,
            epoch,
            seq: 0,
            instance,
            name,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let seq = Arc::new(AtomicU32::new(0));
        let (tx, events) = channel::<Observation>();

        Self::spawn_receiver(recv, me, tx, Arc::clone(&stop));
        Self::spawn_announcer(
            send_sock.try_clone()?,
            template.clone(),
            announce_ms,
            Arc::clone(&stop),
            Arc::clone(&seq),
        );

        Ok(Self {
            events: std::sync::Mutex::new(Some(events)),
            stop,
            send_sock,
            template,
            seq,
        })
    }

    /// 관측 수신단의 **소유권**을 가져간다(1회 — InMemory `discovery()`와 같은 계약).
    ///
    /// # Panics
    /// 두 번 호출하면 패닉(구성 오류).
    #[must_use]
    pub fn take_events(&self) -> Receiver<Observation> {
        self.events
            .lock()
            .expect("잠금")
            .take()
            .expect("take_events는 1회만")
    }

    fn dest() -> SocketAddrV4 {
        SocketAddrV4::new(MULTICAST_GROUP, DISCOVERY_PORT)
    }

    fn spawn_announcer(
        sock: UdpSocket,
        mut template: Packet,
        announce_ms: u32,
        stop: Arc<AtomicBool>,
        seq: Arc<AtomicU32>,
    ) {
        std::thread::spawn(move || {
            // 기동 직후 HELLO(응답 유도) 1회, 이후 주기 ANNOUNCE.
            template.kind = PacketKind::Hello;
            template.seq = seq.fetch_add(1, Ordering::Relaxed);
            let _ = sock.send_to(&template.encode(), Self::dest());
            template.kind = PacketKind::Announce;
            let step = Duration::from_millis(100);
            let mut waited = Duration::ZERO;
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(step);
                waited += step;
                if waited >= Duration::from_millis(u64::from(announce_ms)) {
                    waited = Duration::ZERO;
                    template.seq = seq.fetch_add(1, Ordering::Relaxed);
                    let _ = sock.send_to(&template.encode(), Self::dest());
                }
            }
        });
    }

    fn spawn_receiver(sock: UdpSocket, me: PeerId, tx: Sender<Observation>, stop: Arc<AtomicBool>) {
        std::thread::spawn(move || {
            let mut buf = [0u8; crate::wire::MAX_PACKET];
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                match sock.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        if let Decoded::Packet(packet) = Packet::decode(&buf[..n]) {
                            // 자기 패킷 필터 — 주소가 아니라 키 기준(docs/08 §5).
                            if packet.peer != me && tx.send(Observation { packet, from }).is_err() {
                                return; // 수신자 소멸
                            }
                        }
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(_) => return,
                }
            }
        });
    }
}

impl Drop for UdpDiscovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // 명시적 이탈(FR-D-8) — 유실 대비 2회(수신 중복은 무해).
        let mut bye = self.template.clone();
        bye.kind = PacketKind::Goodbye;
        for _ in 0..2 {
            bye.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let _ = self.send_sock.send_to(&bye.encode(), Self::dest());
        }
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

    /// 같은 호스트 두 인스턴스의 상호 발견 — 실물 UDP 멀티캐스트.
    ///
    /// CI 러너의 멀티캐스트 지원이 보장되지 않아 `#[ignore]` — 로컬·Docker(D-8a)에서
    /// `cargo test -- --ignored`로 실행한다([docs/18] 절차).
    #[test]
    #[ignore = "실네트워크 멀티캐스트 필요 — 로컬/Docker에서 --ignored로 실행(D-8a)"]
    fn two_instances_discover_each_other_via_real_multicast() {
        let a = UdpDiscovery::spawn(pid(1), [1; 16], name("alpha"), 1000, 1, 300).unwrap();
        let b = UdpDiscovery::spawn(pid(2), [2; 16], name("beta"), 2000, 1, 300).unwrap();
        let (a_ev, b_ev) = (a.take_events(), b.take_events());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (mut a_saw, mut b_saw) = (false, false);
        while std::time::Instant::now() < deadline && !(a_saw && b_saw) {
            if let Ok(o) = a_ev.recv_timeout(Duration::from_millis(200)) {
                assert_ne!(o.packet.peer, pid(1), "자기 패킷은 키로 걸러진다");
                if o.packet.peer == pid(2) {
                    assert_eq!(o.packet.name.as_str(), "beta");
                    assert_eq!(o.packet.tcp_port, 2000);
                    a_saw = true;
                }
            }
            if let Ok(o) = b_ev.recv_timeout(Duration::from_millis(200)) {
                if o.packet.peer == pid(1) {
                    b_saw = true;
                }
            }
        }
        assert!(
            a_saw && b_saw,
            "상호 발견 실패(a_saw={a_saw} b_saw={b_saw})"
        );
    }
}
