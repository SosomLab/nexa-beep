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
//! - **주소는 후보 목록이다**(R-21 · M1-4b) — 발견은 v4·v6 여러 경로로 관측되는데 수신
//!   TCP는 v4 전용이라, 마지막 관측으로 덮어쓰면 아무도 듣지 않는 v6 링크로컬이 유일한
//!   주소가 되어 `Unreachable`이 고착된다. 후보를 전부 보관하고 연결 시 **성공 경로 →
//!   v4 → v6 순으로 순차 시도**한다([docs/06 §4] 폴백 사다리 취지).

use crate::tcp::TcpLink;
use crate::transport::{AddError, Caps, ConnectError, DiscoveryEvent, PeerHint, Transport};
use crate::udp::UdpDiscovery;
use crate::wire::PacketKind;
use nbeep_core::link::Link;
use nbeep_core::{DisplayName, PeerId};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 상대별 후보 상한 — 초과 시 가장 오래된 후보부터 밀어낸다(성공 경로는 `preferred`가 지킨다).
const MAX_CANDIDATES: usize = 8;

/// 연결 시도 1회 제한 시간(후보당).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 한 상대의 연결 후보 목록(R-21 · M1-4b).
///
/// 마지막 관측 IP 하나가 아니라 **관측된 주소 전부**를 보관한다. 시도 순서는
/// [`PeerAddrs::try_order`]가 정하고, 성공한 주소는 [`PeerAddrs::promote`]로 승격되어
/// 다음 연결에서 최우선이 된다.
#[derive(Debug, Default, Clone)]
struct PeerAddrs {
    /// 관측 순서대로, 중복 없이(상한 [`MAX_CANDIDATES`]).
    candidates: Vec<SocketAddr>,
    /// 마지막으로 연결에 성공한 주소.
    preferred: Option<SocketAddr>,
}

impl PeerAddrs {
    fn observe(&mut self, addr: SocketAddr) {
        if self.candidates.contains(&addr) {
            return;
        }
        if self.candidates.len() >= MAX_CANDIDATES {
            self.candidates.remove(0);
        }
        self.candidates.push(addr);
    }

    /// 시도 순서 — 성공 경로 → v4(관측 순) → v6(관측 순).
    ///
    /// v4를 앞세우는 이유: 수신 TCP가 현재 v4 전용이라 상대의 v6 광고 주소는
    /// 듣는 이가 없을 수 있다. v6 단독 환경은 v4 후보가 없으므로 자연히 v6로 간다.
    fn try_order(&self) -> Vec<SocketAddr> {
        let mut order = Vec::with_capacity(self.candidates.len() + 1);
        order.extend(self.preferred);
        let (v4, v6): (Vec<_>, Vec<_>) = self
            .candidates
            .iter()
            .copied()
            .filter(|a| Some(*a) != self.preferred)
            .partition(SocketAddr::is_ipv4);
        order.extend(v4);
        order.extend(v6);
        order
    }

    fn promote(&mut self, addr: SocketAddr) {
        self.preferred = Some(addr);
    }
}

/// 후보를 순서대로 시도해 첫 성공 스트림을 돌려준다.
fn connect_first(order: &[SocketAddr], timeout: Duration) -> Option<(TcpStream, SocketAddr)> {
    order.iter().find_map(|&addr| {
        TcpStream::connect_timeout(&addr, timeout)
            .ok()
            .map(|s| (s, addr))
    })
}

/// 실물 로컬 직접 전송 — 발견(S2 멀티캐스트) + TCP 수신/연결.
#[derive(Debug)]
pub struct LocalDirect {
    addrs: Arc<Mutex<HashMap<PeerId, PeerAddrs>>>,
    discovery_rx: Mutex<Option<Receiver<DiscoveryEvent>>>,
    incoming_rx: Mutex<Option<Receiver<Box<dyn Link>>>>,
    /// 발견 핸들 — 드롭 시 GOODBYE · 이름 교체 재공지(M1-10).
    disc: UdpDiscovery,
    /// 실제로 바인딩된 세션 수신 포트(선호 포트가 점유면 임의 폴백 — 값이 달라질 수 있다).
    tcp_port: u16,
}

/// **세션 수신 TCP 기본 포트**(DR-19 · ADR-0006 §3-1).
///
/// 발견이 닿는 곳에서는 광고된 포트를 쓰므로 값이 무엇이든 상관없다. 문제는 **발견이 닿지
/// 않는 곳**(다른 서브넷·라우터 너머)이다 — 거기서는 수동 등록이 유일한 길인데, 포트가
/// 매 실행 바뀌면 **넣을 주소를 알 수 없어 수동 등록 자체가 성립하지 않는다**(2026-08-13 실측).
/// 그래서 **먼저 이 포트를 시도**하고, 점유돼 있을 때만 임의 포트로 물러선다.
/// `--serve`/`--chat-serve` 기본값과 같은 값이라 CLI↔GUI도 맞물린다.
pub const DEFAULT_SESSION_PORT: u16 = 47_200;

/// 선호 포트를 먼저 시도하고, 실패하면 임의 포트로 폴백한다.
///
/// 폴백을 **조용히** 하지 않는다 — 실제 포트는 [`LocalDirect::tcp_port`]로 노출되고
/// 화면에 표시된다(사람이 상대에게 알려줄 수 있어야 한다).
fn bind_session_listener(preferred: u16) -> std::io::Result<TcpListener> {
    if preferred != 0 {
        if let Ok(l) = TcpListener::bind(("0.0.0.0", preferred)) {
            return Ok(l);
        }
    }
    TcpListener::bind("0.0.0.0:0")
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
        Self::spawn_on(me, instance, name, announce_ms, epoch, DEFAULT_SESSION_PORT)
    }

    /// 수신 포트를 지정해 시작한다 — `0`이면 임의 포트.
    ///
    /// 같은 PC에서 인스턴스를 여럿 띄우는 테스트·데모는 `0`을 넘겨 충돌을 피한다.
    ///
    /// # Errors
    /// TCP 바인딩·발견 소켓 실패 시 `io::Error`.
    pub fn spawn_on(
        me: PeerId,
        instance: [u8; 16],
        name: DisplayName,
        announce_ms: u32,
        epoch: u64,
        preferred_port: u16,
    ) -> std::io::Result<Self> {
        // 세션 수신 TCP — 선호 포트 우선·점유 시 임의 폴백. 실제 포트를 발견 패킷에 광고한다.
        let listener = bind_session_listener(preferred_port)?;
        let tcp_port = listener.local_addr()?.port();

        let discovery = UdpDiscovery::spawn(me, instance, name, tcp_port, epoch, announce_ms)?;

        // ── 발견 관측 → DiscoveryEvent + 주소록 갱신 ──
        let addrs = Arc::new(Mutex::new(HashMap::<PeerId, PeerAddrs>::new()));
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
                            addrs
                                .lock()
                                .expect("주소록 잠금")
                                .entry(peer)
                                .or_default()
                                .observe(dest);
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
            disc: discovery,
            tcp_port,
        })
    }

    /// **실제로 듣고 있는 세션 포트.** 선호 포트가 점유돼 폴백했으면 그 값이 나온다.
    ///
    /// 발견이 닿지 않는 상대에게는 사람이 이 값을 알려줘야 수동 등록이 가능하다
    /// — 그래서 UI가 이걸 표시한다(ADR-0006 §3-1).
    #[must_use]
    pub fn tcp_port(&self) -> u16 {
        self.tcp_port
    }

    /// 발견 **수신 강등** 여부(M1-13ⓔ) — true면 발견 포트가 배타 점유돼 남의 방송을
    /// 듣지 못한다(발신·세션은 정상). 호스트가 상태바에 "단방향" 고지를 띄운다.
    #[must_use]
    pub fn discovery_recv_degraded(&self) -> bool {
        self.disc.recv_degraded()
    }
}

impl Transport for LocalDirect {
    fn link_changed(&self) {
        // L1 변화(M1-2) — 발견에 재발견을 시킨다(그룹 재조인 + 즉시 HELLO + S4).
        self.disc.kick();
    }

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
        // 후보 순차 시도(R-21) — 잠금은 스냅숏·승격 순간에만 잡는다(연결 대기 중 보유 금지).
        let order = self
            .addrs
            .lock()
            .expect("주소록 잠금")
            .get(&peer)
            .map(PeerAddrs::try_order)
            .ok_or(ConnectError::Unreachable)?;
        let (stream, addr) =
            connect_first(&order, CONNECT_TIMEOUT).ok_or(ConnectError::Unreachable)?;
        let link = TcpLink::new(stream).map_err(|_| ConnectError::Unreachable)?;
        if let Some(entry) = self.addrs.lock().expect("주소록 잠금").get_mut(&peer) {
            entry.promote(addr);
        }
        Ok(Box::new(link))
    }

    fn add_endpoint(&self, addr: &str) -> Result<Box<dyn Link>, AddError> {
        // 주소로 직접 TCP 연결(발견 우회 — DR-19). 신원은 호출자의 Noise 핸드셰이크가 확정한다.
        // 호스트명·DDNS는 to_socket_addrs가 해석한다(ADR-0006 ③). 해석 결과가 여럿이면
        // 발견 주소록과 같은 원리로 v4 우선 순차 시도한다(R-21 — 첫 항만 쓰면 v6 우선
        // 해석 환경에서 같은 비대칭이 재발한다).
        use std::net::ToSocketAddrs as _;
        let resolved: Vec<SocketAddr> = addr
            .to_socket_addrs()
            .map_err(|_| AddError::BadAddress)?
            .collect();
        if resolved.is_empty() {
            return Err(AddError::BadAddress);
        }
        let (v4, v6): (Vec<_>, Vec<_>) = resolved.into_iter().partition(SocketAddr::is_ipv4);
        let order: Vec<SocketAddr> = v4.into_iter().chain(v6).collect();
        let (stream, _) =
            connect_first(&order, Duration::from_secs(5)).ok_or(AddError::Unreachable)?;
        let link = TcpLink::new(stream).map_err(|_| AddError::Unreachable)?;
        Ok(Box::new(link))
    }

    fn caps(&self) -> Caps {
        Caps::default()
    }

    fn set_display_name(&self, name: DisplayName) {
        // 즉시 재공지(M1-10) — 상대는 PeerTable Renamed로 목록이 갱신된다.
        self.disc.set_name(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn v4(port: u16) -> SocketAddr {
        format!("127.0.0.1:{port}").parse().unwrap()
    }
    fn v6(port: u16) -> SocketAddr {
        format!("[fe80::1]:{port}").parse().unwrap()
    }

    /// R-21 재현 조건 — v6이 **나중에** 관측돼도(구 코드의 "덮어쓰기" 시점) v4가 먼저다.
    #[test]
    fn try_order_puts_v4_first_even_when_v6_observed_last() {
        let mut a = PeerAddrs::default();
        a.observe(v4(1000));
        a.observe(v6(1000));
        a.observe(v6(2000));
        assert_eq!(a.try_order(), vec![v4(1000), v6(1000), v6(2000)]);
    }

    #[test]
    fn try_order_prefers_promoted_path_then_v4() {
        let mut a = PeerAddrs::default();
        a.observe(v6(1000));
        a.observe(v4(1000));
        a.observe(v4(2000));
        a.promote(v6(1000));
        assert_eq!(a.try_order(), vec![v6(1000), v4(1000), v4(2000)]);
    }

    #[test]
    fn observe_dedups_and_caps() {
        let mut a = PeerAddrs::default();
        a.observe(v4(1));
        a.observe(v4(1));
        assert_eq!(a.candidates.len(), 1);
        for p in 1..=20 {
            a.observe(v4(p));
        }
        assert_eq!(a.candidates.len(), MAX_CANDIDATES);
        // 가장 오래된 후보부터 밀려났다.
        assert!(!a.candidates.contains(&v4(1)));
        assert!(a.candidates.contains(&v4(20)));
    }

    /// 회귀(M1-4b) — **주소록에 죽은 v6가 섞여 있어도 연결이 성립하는가.**
    /// 실소켓은 루프백만 쓴다(멀티캐스트·권한 불요 — CI 가능).
    #[test]
    fn connect_succeeds_with_dead_v6_in_book() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap();

        let mut a = PeerAddrs::default();
        a.observe(live);
        a.observe(v6(9)); // 아무도 듣지 않는 v6 링크로컬 — 구 코드라면 이게 유일한 주소가 됐다.

        let (stream, chosen) = connect_first(&a.try_order(), Duration::from_millis(1500))
            .expect("v4 후보로 성립해야 한다");
        assert_eq!(chosen, live);
        drop(stream);
    }

    /// 첫 후보가 죽어 있어도 다음 후보로 넘어간다(순차 폴백 자체의 검증).
    #[test]
    fn connect_falls_through_dead_candidates() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap();
        // 즉시 거절되는 죽은 v4 — 포트를 얻고 바로 닫는다.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };

        let (_, chosen) = connect_first(&[dead, live], Duration::from_millis(1500))
            .expect("두 번째 후보로 성립해야 한다");
        assert_eq!(chosen, live);
    }

    #[test]
    fn connect_first_empty_or_all_dead_is_none() {
        assert!(connect_first(&[], Duration::from_millis(100)).is_none());
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        assert!(connect_first(&[dead], Duration::from_millis(300)).is_none());
    }

    /// 선호 포트가 비어 있으면 **그 포트로** 바인딩된다(08-13 — 수동 등록의 전제).
    /// 고정 47200을 시험하면 CI·병렬 테스트와 경합하므로, OS가 준 임의 포트를 "선호"로
    /// 재사용해 검증한다(닫은 직후라 재점유 가능성은 무시할 수준).
    #[test]
    fn preferred_port_is_used_when_free() {
        let free = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let bound = bind_session_listener(free).unwrap();
        assert_eq!(bound.local_addr().unwrap().port(), free);
    }

    /// 선호 포트가 점유돼 있으면 **실패가 아니라 임의 포트 폴백**이다(DR-4 포터블 —
    /// 같은 PC 다중 인스턴스에서 두 번째가 죽으면 안 된다). 폴백 값은 노출된다.
    #[test]
    fn occupied_preferred_port_falls_back_to_random() {
        let holder = TcpListener::bind("0.0.0.0:0").unwrap();
        let taken = holder.local_addr().unwrap().port();
        let bound = bind_session_listener(taken).unwrap();
        let got = bound.local_addr().unwrap().port();
        assert_ne!(got, taken, "점유 포트를 뺏을 수 없다");
        assert_ne!(got, 0);
    }

    /// `0` = 처음부터 임의 포트(테스트·데모 경로).
    #[test]
    fn zero_means_random_port() {
        let bound = bind_session_listener(0).unwrap();
        assert_ne!(bound.local_addr().unwrap().port(), 0);
    }
}
