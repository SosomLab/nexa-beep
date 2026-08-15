//! S2 — IPv4 멀티캐스트 발견(**첫 실물 소켓** · M1-4 슬라이스 1 · [docs/06 §4]).
//!
//! [`wire`](crate::wire) 패킷을 **S2(IPv4 멀티캐스트) + S3(IPv4 브로드캐스트) 동시**로 쏘고 받는다.
//! 멀티캐스트를 막는 기업 Wi-Fi에서도 브로드캐스트로 발견되게 하는 폴백([docs/06 §4]). 수신
//! 소켓은 `0.0.0.0:PORT` 바인딩이라 **둘 다 같은 소켓으로 받는다**(같은 peer 이중 관측은 PeerTable이
//! 병합 — FR-D-6). **S1(IPv6 멀티캐스트)** 도 best-effort 병행(IPv6 미지원 환경은 조용히 IPv4만).
//! S4(유니캐스트)는 후속.
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
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// 발견 멀티캐스트 그룹(잠정 — D-8b 실측 후 확정). 239.255/16 = 조직 로컬 범위.
pub const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 77, 77);
/// 발견 포트(잠정 — D-8b).
pub const DISCOVERY_PORT: u16 = 47_100;
/// S3 브로드캐스트 목적지(제한 브로드캐스트 — 서브넷 무관 · 라우터는 넘지 않음).
const BROADCAST: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 255);
/// S1 IPv6 멀티캐스트 그룹(링크로컬 ff02::/16 — 관리자 지정 잠정, D-8b 확정).
const MULTICAST_GROUP_V6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x0beb);
/// 멀티캐스트 TTL(잠정 — 링크 로컬 1홉. 라우팅 확장은 S6/릴레이 몫).
const TTL: u32 = 1;
/// 수신 폴링 간격(정지 플래그 확인용).
const RECV_POLL: Duration = Duration::from_millis(300);
/// S4(M1-8) 라운드 주기 — 광고 몇 주기마다 이웃 유니캐스트 프로브를 도는가
/// (기본 광고 800ms면 ≈12.8초 · macOS는 라운드마다 `arp -an` 스폰이라 성기게).
const S4_EVERY: u32 = 16;
/// S4 라운드당 프로브 상한(큐 상한 필수 규칙 — NFR-B-6 결).
const S4_MAX: usize = 64;

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
    /// 내 광고 원본(seq는 전송 시 갱신) — 애넌서와 공유. 이름 변경(M1-10 즉시
    /// 재공지)이 여기를 갱신하면 다음 주기부터 새 이름으로 나간다.
    template: Arc<std::sync::Mutex<Packet>>,
    seq: Arc<AtomicU32>,
    /// 수신 강등(M1-13ⓔ) — 발견 포트를 다른 프로세스가 배타 점유해 **듣지 못한다**.
    /// 발신은 임의 포트라 정상(상대 목록에 나는 뜬다) — 호스트가 상태바에 고지한다.
    recv_degraded: bool,
    /// 수신 소켓의 조인용 복제(M1-2 [`Self::kick`] — 링크 변화 시 새 인터페이스
    /// 그룹 재조인). 소켓 옵션은 어느 핸들로든 같은 소켓에 적용된다.
    recv_join: Option<UdpSocket>,
}

impl UdpDiscovery {
    /// 발견을 시작한다 — 광고 스레드(주기 `announce_ms`) + 수신 스레드.
    ///
    /// # Errors
    /// **송신** 소켓 생성 실패 시 `io::Error`(방화벽·권한·인터페이스 부재).
    /// 수신 소켓 실패(발견 포트 배타 점유)는 오류가 아니라 **발신 전용 강등**이다
    /// (M1-13ⓔ — 종전엔 기동 패닉 · [`Self::recv_degraded`]로 조회).
    pub fn spawn(
        me: PeerId,
        instance: [u8; 16],
        name: DisplayName,
        tcp_port: u16,
        epoch: u64,
        announce_ms: u32,
    ) -> std::io::Result<Self> {
        // ── 수신 소켓: 발견 포트에 재사용 바인딩 + 그룹 가입 ──
        // ★ 실패 = 치명 아님(M1-13ⓔ): 포트 47100은 프로토콜 헌법이라 바꿀 수 없고,
        // 다른 앱이 배타 점유하면 들을 수만 없다. 발신·세션은 멀쩡하므로 수신만
        // 포기한다 — 상대 화면에 나는 뜨고, 상대가 걸어오면 인바운드로 대화 성립.
        let recv: Option<UdpSocket> = (|| -> std::io::Result<UdpSocket> {
            let recv = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            recv.set_reuse_address(true)?;
            #[cfg(unix)]
            recv.set_reuse_port(true)?; // 같은 호스트 다중 인스턴스(개발·테스트)
            recv.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).into())?;
            recv.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
            // 인터페이스별 그룹 가입(M1-9) — 기본 경로 인터페이스만 조인되면
            // 링크로컬 직결(169.254)·보조 NIC의 광고를 못 듣는다. 실패는 무시
            // (이미 조인된 인터페이스는 EADDRINUSE류 — best-effort).
            for i in crate::netif::eligible_v4() {
                let _ = recv.join_multicast_v4(&MULTICAST_GROUP, &i.v4);
            }
            recv.set_multicast_loop_v4(true)?; // 같은 호스트 인스턴스 간 도달
            let recv: UdpSocket = recv.into();
            recv.set_read_timeout(Some(RECV_POLL))?;
            Ok(recv)
        })()
        .ok();
        let recv_degraded = recv.is_none();

        // ── 송신 소켓: 임의 포트(발신 주소가 연결 힌트가 된다) ──
        let send_sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        send_sock.set_multicast_ttl_v4(TTL)?;
        send_sock.set_multicast_loop_v4(true)?;
        send_sock.set_broadcast(true)?; // S3 — 멀티캐스트 차단 폴백

        // ── S1: IPv6 멀티캐스트(best-effort — 실패해도 IPv4로 계속) ──
        let v6 = Self::setup_ipv6().ok();

        let template = Arc::new(std::sync::Mutex::new(Packet {
            kind: PacketKind::Announce,
            flags: 0,
            peer: me,
            tcp_port,
            epoch,
            seq: 0,
            instance,
            name,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let seq = Arc::new(AtomicU32::new(0));
        let (tx, events) = channel::<Observation>();

        let recv_join = recv.as_ref().and_then(|r| r.try_clone().ok());
        if let Some(recv) = recv {
            // v4 수신자는 HELLO 유니캐스트 응답까지(S4 왕복 — 송신 소켓 복제 실패 시
            // 응답 생략 = 종전 동작).
            let reply = send_sock
                .try_clone()
                .ok()
                .map(|s| (s, Arc::clone(&template), Arc::clone(&seq)));
            Self::spawn_receiver_with_reply(recv, me, tx.clone(), Arc::clone(&stop), reply);
        }
        let v6_send = if let Some((v6_recv, v6_send)) = v6 {
            if let Ok(rx) = v6_recv.try_clone() {
                Self::spawn_receiver(rx, me, tx, Arc::clone(&stop));
            }
            v6_send.try_clone().ok()
        } else {
            None
        };
        Self::spawn_announcer(
            send_sock.try_clone()?,
            v6_send,
            Arc::clone(&template),
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
            recv_degraded,
            recv_join,
        })
    }

    /// 수신 강등 여부(M1-13ⓔ) — true = 발견 포트 점유로 **듣지 못하는** 발신 전용.
    #[must_use]
    pub fn recv_degraded(&self) -> bool {
        self.recv_degraded
    }

    /// 링크 변화 후 재발견(M1-2) — 새로 생긴 인터페이스에 그룹 재조인(best-effort ·
    /// 이미 조인된 곳은 오류 무시) + 즉시 HELLO(응답 유도) + S4 1라운드. 주기 광고를
    /// 기다리지 않고 새 링크의 이웃을 당긴다. 호출 주체 = 호스트의 디바운서(폭주는
    /// 거기서 접힌다 — 여기서는 접지 않는다).
    pub fn kick(&self) {
        if let Some(j) = &self.recv_join {
            let sref = socket2::SockRef::from(j);
            for i in crate::netif::eligible_v4() {
                let _ = sref.join_multicast_v4(&MULTICAST_GROUP, &i.v4);
            }
        }
        let hello = {
            let mut p = match self.template.lock() {
                Ok(g) => g.clone(),
                Err(e) => e.into_inner().clone(),
            };
            p.kind = PacketKind::Hello;
            p.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            p
        }
        .encode();
        Self::send_all(&self.send_sock, None, &hello);
        Self::send_s4(&self.send_sock, &hello);
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

    fn broadcast_dest() -> SocketAddrV4 {
        SocketAddrV4::new(BROADCAST, DISCOVERY_PORT)
    }

    fn dest_v6() -> SocketAddrV6 {
        SocketAddrV6::new(MULTICAST_GROUP_V6, DISCOVERY_PORT, 0, 0)
    }

    /// S1 IPv6 멀티캐스트 소켓 쌍(recv, send) — best-effort. 링크로컬 그룹 가입은
    /// 기본 인터페이스(0)로 시도한다(정밀 인터페이스 선택은 M1-3 인터페이스 바인딩·D-8b).
    fn setup_ipv6() -> std::io::Result<(UdpSocket, UdpSocket)> {
        let recv = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        recv.set_only_v6(true)?; // IPv4는 별도 소켓 — 이중 바인딩 회피
        recv.set_reuse_address(true)?;
        #[cfg(unix)]
        recv.set_reuse_port(true)?;
        recv.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, DISCOVERY_PORT)).into())?;
        recv.join_multicast_v6(&MULTICAST_GROUP_V6, 0)?;
        recv.set_multicast_loop_v6(true)?;
        let recv: UdpSocket = recv.into();
        recv.set_read_timeout(Some(RECV_POLL))?;

        let send = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        send.set_only_v6(true)?;
        send.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)).into())?;
        send.set_multicast_loop_v6(true)?;
        let send: UdpSocket = send.into();
        Ok((recv, send))
    }

    /// S2 멀티캐스트 + S3 브로드캐스트 (+ 있으면 S1 IPv6) 동시 발신 — 하나만 통과해도 발견 성립.
    fn send_all(v4: &UdpSocket, v6: Option<&UdpSocket>, bytes: &[u8]) {
        // 기본 경로 발신(종전 동작) — 인터페이스 열거가 비는 환경(Windows 폴백 등)의
        // 안전망. 멀티캐스트 IF를 바꾼 뒤에도 마지막에 기본으로 되돌린다.
        let _ = v4.send_to(bytes, Self::dest());
        let _ = v4.send_to(bytes, Self::broadcast_dest());
        // ★ 인터페이스별 명시 발신(M1-9 · FR-D-7) — 기본 경로가 없는 링크로컬 직결
        // (자기 배정 169.254)·다중 NIC에서도 상대 링크로 나간다. 가상·터널 제외는
        // netif가 맡는다(VPN으로 존재 방송 금지 — R-18 결).
        let sref = socket2::SockRef::from(v4);
        for i in crate::netif::eligible_v4() {
            if sref.set_multicast_if_v4(&i.v4).is_ok() {
                let _ = v4.send_to(bytes, Self::dest());
            }
            // S3 지향 브로드캐스트 — 서브넷 브로드캐스트는 IF 지정과 무관하게
            // 목적지 주소로 경로가 정해진다(255.255.255.255와 달리 링크가 특정된다).
            if let Some(b) = i.subnet_broadcast() {
                let _ = v4.send_to(bytes, SocketAddr::from((b, DISCOVERY_PORT)));
            }
        }
        let _ = sref.set_multicast_if_v4(&Ipv4Addr::UNSPECIFIED);
        if let Some(v6) = v6 {
            let _ = v6.send_to(bytes, Self::dest_v6());
        }
    }

    fn spawn_announcer(
        sock: UdpSocket,
        v6: Option<UdpSocket>,
        template: Arc<std::sync::Mutex<Packet>>,
        announce_ms: u32,
        stop: Arc<AtomicBool>,
        seq: Arc<AtomicU32>,
    ) {
        std::thread::spawn(move || {
            // 스냅샷 발신 — 잠금은 복사 순간만(소켓 I/O 중 잠금 유지 금지).
            let snapshot = |kind: PacketKind| {
                let mut p = match template.lock() {
                    Ok(g) => g.clone(),
                    Err(e) => e.into_inner().clone(),
                };
                p.kind = kind;
                p.seq = seq.fetch_add(1, Ordering::Relaxed);
                p
            };
            // 기동 직후 HELLO(응답 유도) 1회, 이후 주기 ANNOUNCE.
            Self::send_all(&sock, v6.as_ref(), &snapshot(PacketKind::Hello).encode());
            // ★ S4(M1-8) — 기동 직후 이웃 유니캐스트 1회(막힌 망에서 첫 발견을 당긴다).
            Self::send_s4(&sock, &snapshot(PacketKind::Hello).encode());
            let step = Duration::from_millis(100);
            let mut waited = Duration::ZERO;
            let mut cycles: u32 = 0;
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(step);
                waited += step;
                if waited >= Duration::from_millis(u64::from(announce_ms)) {
                    waited = Duration::ZERO;
                    Self::send_all(&sock, v6.as_ref(), &snapshot(PacketKind::Announce).encode());
                    cycles = cycles.wrapping_add(1);
                    // S4 주기 라운드 — 광고 틱 재사용(새 타이머 0 · [13 §12-1]):
                    // 16주기(기본 800ms면 ≈12.8초)마다 이웃 테이블을 다시 읽어 프로브.
                    // 멀티캐스트가 통하는 망에선 중복 관측일 뿐이고(무해), 막힌 망에선
                    // 이것이 유일한 발견 경로다(S1~S3 폴백 사다리의 다음 단 — 06 §4).
                    if cycles % S4_EVERY == 0 {
                        Self::send_s4(&sock, &snapshot(PacketKind::Hello).encode());
                    }
                }
            }
        });
    }

    /// S4 유니캐스트 프로브(M1-8) — OS 이웃 테이블(ARP)의 주소 중 **자격 인터페이스
    /// 서브넷 안**의 것들에게 1:1 HELLO. 봉투 원리: 프로브 내용은 방송과 동일하다
    /// (이웃에게만 더 보내는 것 — 새 정보 없음). 상한 [`S4_MAX`](S4_MAX)로 폭주 방지.
    fn send_s4(sock: &UdpSocket, bytes: &[u8]) {
        for n in s4_targets(
            &crate::netif::eligible_v4(),
            crate::neigh::neighbors_v4(),
            S4_MAX,
        ) {
            let _ = sock.send_to(bytes, SocketAddr::from((n, DISCOVERY_PORT)));
        }
    }

    /// 표시 이름 교체 + **즉시 재공지**(M1-10 · 사용자 확정 08-11) — 상대 목록은
    /// `PeerTable::observe`의 `Renamed` 경로로 갱신된다. 즉시 발신은 v4(멀티캐스트+
    /// 브로드캐스트)로 하고, v6은 다음 주기 광고에 실린다(GOODBYE와 같은 정책).
    pub fn set_name(&self, name: DisplayName) {
        let announce = {
            let mut t = match self.template.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            t.name = name;
            let mut p = t.clone();
            p.kind = PacketKind::Announce;
            p.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            p
        };
        Self::send_all(&self.send_sock, None, &announce.encode());
    }

    fn spawn_receiver(sock: UdpSocket, me: PeerId, tx: Sender<Observation>, stop: Arc<AtomicBool>) {
        Self::spawn_receiver_with_reply(sock, me, tx, stop, None);
    }

    /// 수신 스레드 — `reply`가 있으면 **남의 HELLO에 유니캐스트 Announce로 응답**한다
    /// (S4 왕복의 수신측 절반 · M1-8): 멀티캐스트가 막힌 망에선 내 주기 광고가 프로브
    /// 발신자에게 닿지 않으므로, 발신 주소로 직접 되쏴야 상호 발견이 성립한다.
    /// 멀티캐스트로 받은 HELLO에도 응답하지만 수신측 관측은 멱등이라 무해하고,
    /// 응답은 Announce(비유도)라 응답 연쇄는 생기지 않는다.
    fn spawn_receiver_with_reply(
        sock: UdpSocket,
        me: PeerId,
        tx: Sender<Observation>,
        stop: Arc<AtomicBool>,
        reply: Option<(UdpSocket, Arc<std::sync::Mutex<Packet>>, Arc<AtomicU32>)>,
    ) {
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
                            if packet.peer == me {
                                continue;
                            }
                            if packet.kind == PacketKind::Hello {
                                if let Some((rs, template, seq)) = &reply {
                                    let mut p = match template.lock() {
                                        Ok(g) => g.clone(),
                                        Err(e) => e.into_inner().clone(),
                                    };
                                    p.kind = PacketKind::Announce;
                                    p.seq = seq.fetch_add(1, Ordering::Relaxed);
                                    let _ = rs.send_to(&p.encode(), from);
                                }
                            }
                            if tx.send(Observation { packet, from }).is_err() {
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

/// S4 프로브 대상 선별(M1-8 · 순수 — 회귀 대상): 이웃 중 **자격 인터페이스 서브넷
/// 안**의 주소만, 내 주소 제외, `cap`개까지. 서브넷 밖 이웃(라우터 너머 잔존 항목)에
/// 발견 패킷을 쏘지 않는다 — 링크 밖 유출 금지(FR-S-49 결).
fn s4_targets(ifs: &[crate::netif::NetIf], neighbors: Vec<Ipv4Addr>, cap: usize) -> Vec<Ipv4Addr> {
    neighbors
        .into_iter()
        .filter(|n| {
            ifs.iter().any(|i| {
                i.mask.is_some_and(|m| {
                    let m = u32::from(m);
                    m != 0 && (u32::from(*n) & m) == (u32::from(i.v4) & m) && *n != i.v4
                })
            })
        })
        .take(cap)
        .collect()
}

impl Drop for UdpDiscovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // 명시적 이탈(FR-D-8) — 유실 대비 2회(수신 중복은 무해).
        let mut bye = match self.template.lock() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        };
        bye.kind = PacketKind::Goodbye;
        for _ in 0..2 {
            bye.seq = self.seq.fetch_add(1, Ordering::Relaxed);
            // Drop 시엔 v4로만(v6 send 핸들은 애넌서 스레드 소유 — GOODBYE는 v4로 충분).
            Self::send_all(&self.send_sock, None, &bye.encode());
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

    /// S4 대상 선별(M1-8) — 서브넷 안만·내 주소 제외·상한. 라우터 너머 잔존
    /// ARP 항목으로 발견 패킷이 새면 링크 밖 유출이다(FR-S-49 결).
    #[test]
    fn s4_targets_stay_inside_subnet_and_cap() {
        use crate::netif::NetIf;
        use std::net::Ipv4Addr;
        let ifs = vec![NetIf {
            name: "en0".into(),
            v4: Ipv4Addr::new(192, 168, 45, 84),
            mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            up: true,
            loopback: false,
        }];
        let neigh = vec![
            Ipv4Addr::new(192, 168, 45, 1),   // 같은 서브넷 ✓
            Ipv4Addr::new(192, 168, 45, 84),  // 나 자신 ✗
            Ipv4Addr::new(10, 0, 0, 5),       // 다른 서브넷 ✗(잔존 항목)
            Ipv4Addr::new(192, 168, 45, 200), // 같은 서브넷 ✓
        ];
        let t = s4_targets(&ifs, neigh.clone(), 64);
        assert_eq!(
            t,
            vec![
                Ipv4Addr::new(192, 168, 45, 1),
                Ipv4Addr::new(192, 168, 45, 200)
            ]
        );
        assert_eq!(s4_targets(&ifs, neigh, 1).len(), 1, "상한 준수");
        assert!(s4_targets(&[], vec![Ipv4Addr::new(1, 2, 3, 4)], 64).is_empty());
    }

    /// M1-13ⓔ — 발견 포트를 다른 프로세스가 **배타 점유**해도 spawn은 성공하고
    /// 발신 전용으로 강등된다(종전엔 기동 패닉 = 제로 컨피그 정체성 파탄).
    ///
    /// Windows는 SO_REUSEADDR 의미가 달라(비재사용 소켓의 포트도 빼앗을 수 있다)
    /// 점유 시뮬레이션이 성립하지 않는다 — unix 한정("네트워크는 실측" · R-7 결).
    #[cfg(unix)]
    #[test]
    fn occupied_discovery_port_degrades_to_send_only() {
        // 재사용 플래그 없는 배타 점유자 — 우리 수신 바인딩(reuse여도)은 실패한다.
        let Ok(_hog) = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)) else {
            return; // 이 호스트에서 진짜 점유 중(다른 인스턴스) — 환경 의존이라 건너뜀
        };
        let d = UdpDiscovery::spawn(pid(9), [9; 16], name("hog"), 9000, 1, 60_000)
            .expect("점유 시에도 spawn은 성공(발신 전용)");
        assert!(d.recv_degraded(), "수신 강등이 표시돼야 호스트가 고지한다");
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

    /// M1-10 — `set_name`이 **즉시 재공지**하고, 이후 주기 광고에도 새 이름이 실린다.
    #[test]
    #[ignore = "실네트워크 멀티캐스트 필요 — 로컬/Docker에서 --ignored로 실행(D-8a)"]
    fn rename_reannounces_immediately() {
        let a = UdpDiscovery::spawn(pid(1), [1; 16], name("before"), 1000, 1, 60_000).unwrap();
        let b = UdpDiscovery::spawn(pid(2), [2; 16], name("watcher"), 2000, 1, 60_000).unwrap();
        let b_ev = b.take_events();
        // a의 기동 HELLO가 b에 닿을 때까지 대기(채널 정착).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen_before = false;
        while std::time::Instant::now() < deadline && !seen_before {
            if let Ok(o) = b_ev.recv_timeout(Duration::from_millis(200)) {
                seen_before = o.packet.peer == pid(1) && o.packet.name.as_str() == "before";
            }
        }
        assert!(seen_before, "기동 광고 미도달");
        // 주기(60s)가 오기 한참 전에 — set_name의 즉시 발신만으로 새 이름이 닿아야 한다.
        a.set_name(name("after"));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen_after = false;
        while std::time::Instant::now() < deadline && !seen_after {
            if let Ok(o) = b_ev.recv_timeout(Duration::from_millis(200)) {
                seen_after = o.packet.peer == pid(1) && o.packet.name.as_str() == "after";
            }
        }
        assert!(seen_after, "즉시 재공지 미도달(주기 60s 내 도달 = 실패)");
    }
}
