//! X-2 Transport 승격 e2e(11-06) — 릴레이가 **`Transport` 트레이트로** 돈다는 것을 실소켓·실서버로 증명한다.
//!
//! - **ADR-0003 핵심 규칙**: 종단 Noise는 `Transport::connect`/`incoming`이 준 **핸드셰이크 전 `Link`** 위에서
//!   호출자가 얹는다(LAN과 같은 코드) — 서버는 종단 암호문만 나른다(S-3).
//! - **사다리 통합**: [`LadderTransport`]가 로컬(InMemory) → 랑데부 순으로 `connect` 하나에서 탄다(ADR-0003 §4).
//! - **수명**: 호스트가 `Arc<RelayClient>`를 놓으면 전송은 `Lost`(Weak) · `set_relay(None)` = Unreachable.
#![allow(clippy::unwrap_used)] // 테스트 — docs/13 §9 예외

use nbeep_core::session::Session as _;
use nbeep_core::{DisplayName, PeerId};
use nbeep_crypto::{Identity, NoiseSession};
use nbeep_net::{Caps, ConnectError, Transport as _};
use nbeep_relay::{rids_around, LadderTransport, RelayClient, RelayTransport};
use nexa_beepd::{spawn, Config};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

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
    let ta = RelayTransport::new(&ca);
    let tb = RelayTransport::new(&cb);
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
        "큰 프레임도 단 무관 보존"
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
    let ladder = LadderTransport::new(Arc::new(local_a));
    assert_eq!(
        ladder.connect(peer_b).err(),
        Some(ConnectError::Unreachable),
        "랑데부 단 미장착 = 경로 없음"
    );
    ladder.set_relay(Some(&ca));
    assert!(ladder.relay_attached());

    let tb = RelayTransport::new(&cb);
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
    let ladder = LadderTransport::new(Arc::new(local_a));
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
    let ta = RelayTransport::new(&ca);
    drop(ca);
    assert_eq!(
        ta.connect(PeerId::from_bytes([9u8; 32])).err(),
        Some(ConnectError::Lost)
    );
    server.shutdown();
}
