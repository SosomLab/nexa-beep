//! X-2 Transport 승격 e2e(11-06) — 릴레이가 **`Transport` 트레이트로** 돈다는 것을 실소켓·실서버로 증명한다.
//! X-2f(11-06) — 서버 명부(roster)가 **발견 스트림**(`Appeared(origin Rendezvous)`/`Withdrawn`)으로 올라온다.
//!
//! - **ADR-0003 핵심 규칙**: 종단 Noise는 `Transport::connect`/`incoming`이 준 **핸드셰이크 전 `Link`** 위에서
//!   호출자가 얹는다(LAN과 같은 코드) — 서버는 종단 암호문만 나른다(S-3).
//! - **사다리 통합**: [`LadderTransport`]가 로컬(InMemory) → 랑데부 순으로 `connect` 하나에서 탄다(ADR-0003 §4).
//! - **수명**: 호스트가 `Arc<RelayClient>`를 놓으면 전송은 `Lost`(Weak) · `set_relay(None)` = Unreachable + 명부 일괄 `Withdrawn`.
#![allow(clippy::unwrap_used)] // 테스트 — docs/13 §9 예외

use nbeep_core::session::Session as _;
use nbeep_core::{DisplayName, HintOrigin, Liveness, PeerId};
use nbeep_crypto::{Identity, NoiseSession};
use nbeep_net::{Caps, ConnectError, DiscoveryEvent, Transport as _};
use nbeep_relay::{encode_card, rids_around, LadderTransport, RelayClient, RelayTransport};
use nexa_beepd::{spawn, Config};
use std::net::IpAddr;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn test_server() -> nexa_beepd::Handle {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "beepd-xport-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    spawn(&Config {
        bind_ip: IpAddr::from([127, 0, 0, 1]),
        port: 0,
        key_path: dir.join("beepd.key"),
        rate_bps: 0,
        verbose: false,
    })
    .unwrap()
}

fn attach(server: &nexa_beepd::Handle, id: &Identity) -> Arc<RelayClient> {
    Arc::new(RelayClient::connect(server.tcp_addr, id, &rids_around(&id.peer_id()), None).unwrap())
}

/// 조건을 만족하는 발견 이벤트가 올 때까지 기다린다(그 밖은 버림 · 5s 상한).
fn wait_event(
    rx: &Receiver<DiscoveryEvent>,
    mut pred: impl FnMut(&DiscoveryEvent) -> bool,
) -> DiscoveryEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ev = rx.recv_timeout(left).expect("발견 이벤트 대기 시간 초과");
        if pred(&ev) {
            return ev;
        }
    }
}

/// `RelayTransport` 단독 — A `connect(peer_b)` → 핸드셰이크 전 링크(랑데부 표식 · 실주소 있음) → 호출자 Noise ·
/// B `incoming()` → 같은 결로 `accept_any`. 양쪽 인증 키가 실키와 같고 왕복이 비트 동일.
#[test]
fn relay_transport_connect_and_incoming_with_caller_noise() {
    let server = test_server();
    let ida = Identity::generate();
    let idb = Identity::generate();
    let (peer_a, peer_b) = (ida.peer_id(), idb.peer_id());
    let ca = attach(&server, &ida);
    let cb = attach(&server, &idb);
    let ta = RelayTransport::new(&ca, peer_a, 1);
    let tb = RelayTransport::new(&cb, peer_b, 1);
    let b_in = tb.incoming();

    let a_side = std::thread::spawn(move || {
        let link = ta.connect(peer_b).expect("랑데부 사다리 링크");
        assert!(link.rendezvous(), "랑데부 표식(배지용)");
        assert!(
            link.remote_ip().is_some(),
            "경로 등급 재료(확정 전 = 서버 주소)"
        );
        // ★ 호출자가 LAN과 같은 코드로 핸드셰이크 — Transport는 바이트만 나른다.
        let mut sess = NoiseSession::initiate(link, &ida).expect("종단 핸드셰이크");
        assert_eq!(sess.peer(), peer_b, "인증된 상대 = B의 실키");
        sess.send(b"via Transport").unwrap();
        assert_eq!(sess.recv().unwrap(), b"ack");
        let big: Vec<u8> = (0..60_000u32).map(|i| (i % 251) as u8).collect();
        sess.send(&big).unwrap();
        assert_eq!(sess.recv().unwrap(), b"big-ok");
        (ta, ca)
    });

    let link = b_in
        .recv_timeout(Duration::from_secs(15))
        .expect("B incoming — 첫 프레임 링크 추종");
    assert!(link.rendezvous());
    let (mut sess, via_psk) = NoiseSession::accept_any(link, &idb, None).expect("수락 핸드셰이크");
    assert!(!via_psk);
    assert_eq!(sess.peer(), peer_a, "인증된 상대 = A의 실키");
    assert_eq!(sess.recv().unwrap(), b"via Transport");
    sess.send(b"ack").unwrap();
    assert_eq!(
        sess.recv().unwrap().len(),
        60_000,
        "큰 프레임(릴레이 조각 32KiB 초과 · Noise 상한 안)도 단 무관 보존"
    );
    sess.send(b"big-ok").unwrap();

    let _keep = a_side.join().unwrap();
    server.shutdown();
}

/// `LadderTransport` — 로컬(InMemory 버스에 A만)은 B를 모르므로 **랑데부 단으로 내려가** 성립한다(발견 밖 상대도
/// `connect` 하나). `set_relay(None)` 뒤에는 Unreachable.
#[test]
fn ladder_transport_falls_through_to_rendezvous() {
    let server = test_server();
    let ida = Identity::generate();
    let idb = Identity::generate();
    let (peer_a, peer_b) = (ida.peer_id(), idb.peer_id());
    let ca = attach(&server, &ida);
    let cb = attach(&server, &idb);

    let bus = nbeep_net::inmem::InMemoryBus::new();
    let local_a = bus.join(peer_a, DisplayName::parse("a").unwrap(), Caps::default());
    let ladder = LadderTransport::new(Arc::new(local_a), peer_a);
    assert_eq!(
        ladder.connect(peer_b).err(),
        Some(ConnectError::Unreachable),
        "랑데부 단 미장착 = 경로 없음"
    );
    ladder.set_relay(Some(&ca));
    assert!(ladder.relay_attached());

    let tb = RelayTransport::new(&cb, peer_b, 1);
    let b_in = tb.incoming();
    let ladder2 = Arc::clone(&ladder);
    let a_side = std::thread::spawn(move || {
        let link = ladder2.connect(peer_b).expect("로컬 실패 → 랑데부 단");
        assert!(link.rendezvous());
        let mut sess = NoiseSession::initiate(link, &ida).unwrap();
        assert_eq!(sess.peer(), peer_b);
        sess.send(b"ladder").unwrap();
        assert_eq!(sess.recv().unwrap(), b"ok");
    });
    let link = b_in.recv_timeout(Duration::from_secs(15)).unwrap();
    let (mut sess, _) = NoiseSession::accept_any(link, &idb, None).unwrap();
    assert_eq!(sess.peer(), peer_a);
    assert_eq!(sess.recv().unwrap(), b"ladder");
    sess.send(b"ok").unwrap();
    a_side.join().unwrap();

    ladder.set_relay(None);
    assert!(!ladder.relay_attached());
    assert_eq!(
        ladder.connect(peer_b).err(),
        Some(ConnectError::Unreachable)
    );
    drop(tb);
    server.shutdown();
}

/// 우선순위 — 로컬이 아는 상대는 랑데부 단이 장착돼 있어도 **로컬 링크**(랑데부 표식 없음 · ADR-0003 §4 "로컬 > 릴레이").
#[test]
fn ladder_transport_prefers_local_over_rendezvous() {
    let server = test_server();
    let ida = Identity::generate();
    let ca = attach(&server, &ida);
    let bus = nbeep_net::inmem::InMemoryBus::new();
    let local_a = bus.join(
        ida.peer_id(),
        DisplayName::parse("a").unwrap(),
        Caps::default(),
    );
    let peer_b = PeerId::from_bytes([7u8; 32]);
    let local_b = bus.join(peer_b, DisplayName::parse("b").unwrap(), Caps::default());
    let b_in = local_b.incoming();
    let ladder = LadderTransport::new(Arc::new(local_a), ida.peer_id());
    ladder.set_relay(Some(&ca));
    let link = ladder.connect(peer_b).expect("로컬 경로");
    assert!(!link.rendezvous(), "로컬이 되면 서버로 우회하지 않는다");
    assert!(b_in.recv_timeout(Duration::from_secs(2)).is_ok());
    server.shutdown();
}

/// 수명 — 호스트가 `Arc<RelayClient>`를 놓으면 전송은 `Lost`(재접속은 호스트 몫 · 조용한 유령 접속 없음).
#[test]
fn relay_transport_reports_lost_after_host_drops_client() {
    let server = test_server();
    let ida = Identity::generate();
    let ca = attach(&server, &ida);
    let ta = RelayTransport::new(&ca, ida.peer_id(), 1);
    drop(ca);
    assert_eq!(
        ta.connect(PeerId::from_bytes([9u8; 32])).err(),
        Some(ConnectError::Lost)
    );
    server.shutdown();
}

/// X-2f — 서버 명부가 발견 스트림으로: B가 공개(카드 동봉)하면 A의 `discovery()`에 `Appeared(origin Rendezvous · Eventful ·
/// source = 세대 토큰 · profile 정제됨 · 이름 = 카드 이름)` · B가 공개 해제하면 `Withdrawn{peer_b, source}`. 나 자신은 오지 않는다.
#[test]
fn relay_transport_turns_roster_into_discovery() {
    let server = test_server();
    let ida = Identity::generate();
    let idb = Identity::generate();
    let (peer_a, peer_b) = (ida.peer_id(), idb.peer_id());
    let ca = attach(&server, &ida);
    let cb = attach(&server, &idb);
    let ta = RelayTransport::new(&ca, peer_a, 7);
    let disc = ta.discovery();
    ta.set_announce(true, encode_card("Alice", "", ""));
    cb.set_announce(true, encode_card("Bob", "bob@example.test", "hi\u{7}there"));

    let ev = wait_event(
        &disc,
        |e| matches!(e, DiscoveryEvent::Appeared(h) if h.peer == peer_b),
    );
    let DiscoveryEvent::Appeared(h) = ev else {
        unreachable!()
    };
    assert_eq!(h.origin, HintOrigin::Rendezvous);
    assert_eq!(h.liveness, Liveness::Eventful);
    assert_eq!(h.source, 7, "세대 토큰");
    assert_eq!(h.gen, 0, "명부는 세대 미상");
    assert_eq!(h.name.as_str(), "Bob", "카드 이름 = 표시 이름");
    let p = h.profile.expect("카드");
    assert_eq!(p.name, "Bob");
    assert_eq!(p.email, "bob@example.test");
    assert_eq!(p.bio, "hithere", "제어 문자는 경계에서 제거");

    cb.set_announce(false, Vec::new());
    let ev = wait_event(
        &disc,
        |e| matches!(e, DiscoveryEvent::Withdrawn { peer, .. } if *peer == peer_b),
    );
    assert_eq!(
        ev,
        DiscoveryEvent::Withdrawn {
            peer: peer_b,
            source: 7
        }
    );
    // 나 자신(A)의 Up은 올라오지 않았어야 한다 — 남은 이벤트에 A가 없다.
    while let Ok(ev) = disc.try_recv() {
        if let DiscoveryEvent::Appeared(h) = ev {
            assert_ne!(h.peer, peer_a, "자기 PeerId 제외");
        }
    }
    drop(cb);
    server.shutdown();
}

/// X-2f — 사다리 전송의 발견 합류: LAN(InMemory · 출처 Local · 토큰 0)과 랑데부(출처 Rendezvous · 토큰 1)가 **한 스트림**으로
/// 오고, `set_relay(None)`이면 랑데부 명부가 일괄 `Withdrawn`된다(서버 전용 상대의 유령 행 방지).
#[test]
fn ladder_transport_merges_local_and_rendezvous_discovery() {
    let server = test_server();
    let ida = Identity::generate();
    let idb = Identity::generate();
    let (peer_a, peer_b) = (ida.peer_id(), idb.peer_id());
    let ca = attach(&server, &ida);
    let cb = attach(&server, &idb);
    let bus = nbeep_net::inmem::InMemoryBus::new();
    let local_a = bus.join(peer_a, DisplayName::parse("a").unwrap(), Caps::default());
    let ladder = LadderTransport::new(Arc::new(local_a), peer_a);
    let disc = ladder.discovery();
    ladder.set_relay(Some(&ca));
    ladder.set_announce(true, Vec::new());
    cb.set_announce(true, Vec::new());
    let peer_c = PeerId::from_bytes([9u8; 32]);
    let _local_c = bus.join(peer_c, DisplayName::parse("c").unwrap(), Caps::default());

    let lan = wait_event(
        &disc,
        |e| matches!(e, DiscoveryEvent::Appeared(h) if h.peer == peer_c),
    );
    let DiscoveryEvent::Appeared(lan) = lan else {
        unreachable!()
    };
    assert_eq!((lan.origin, lan.source), (HintOrigin::Local, 0));
    let rv = wait_event(
        &disc,
        |e| matches!(e, DiscoveryEvent::Appeared(h) if h.peer == peer_b),
    );
    let DiscoveryEvent::Appeared(rv) = rv else {
        unreachable!()
    };
    assert_eq!(
        (rv.origin, rv.source),
        (HintOrigin::Rendezvous, 1),
        "첫 장착 = 세대 1"
    );
    assert!(
        rv.profile.is_none(),
        "빈 카드 바이트 = 카드 없음(None) — 정제 규칙과 일치"
    );

    ladder.set_relay(None);
    let ev = wait_event(
        &disc,
        |e| matches!(e, DiscoveryEvent::Withdrawn { peer, .. } if *peer == peer_b),
    );
    assert_eq!(
        ev,
        DiscoveryEvent::Withdrawn {
            peer: peer_b,
            source: 1
        }
    );
    drop(cb);
    server.shutdown();
}
