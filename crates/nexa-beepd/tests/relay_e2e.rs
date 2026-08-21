//! 릴레이 서버 e2e — X-1 MVP의 성공 기준을 실소켓으로 검증한다.
//!
//! - **DR-21 증명**: 기존 `NoiseSession`이 **코드 무변경**으로 `RelayLink`/`UdpLink` 위에서 돈다.
//! - **S-3 증명**: 서버는 종단 암호문만 나른다(핸드셰이크는 A↔B 직접 — 서버 키로는 못 연다).
//! - **X-UDP-c 배관**: 서버 UDP 관측 → 엔드포인트 교환 → 동시 열기 펀치(루프백).
#![allow(clippy::unwrap_used)] // 테스트 — docs/13 §9 예외

use nbeep_core::session::Session as _;
use nbeep_crypto::{Identity, NoiseSession};
use nbeep_relay::{probe_udp, rid_for, rids_around, RelayClient};
use nexa_beepd::{spawn, Config};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

fn test_server(rate_bps: u64) -> nexa_beepd::Handle {
    let dir = std::env::temp_dir().join(format!(
        "beepd-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    spawn(&Config {
        bind_ip: IpAddr::from([127, 0, 0, 1]),
        port: 0, // 임시 — TCP가 정한 번호로 UDP 동번호
        key_path: dir.join("beepd.key"),
        rate_bps,
        verbose: false,
    })
    .unwrap()
}

/// 성공 기준의 본체 — A↔B가 서버 경유로 **종단** Noise 세션을 성립하고, 큰 프레임
/// (조각 분할·조립)까지 비트 동일로 왕복한다.
#[test]
fn e2e_noise_through_relay() {
    let server = test_server(0);
    let addr = server.tcp_addr;

    let ida = Identity::generate();
    let idb = Identity::generate();
    let peer_a = ida.peer_id();
    let peer_b = idb.peer_id();

    let ca = RelayClient::connect(addr, &ida, &rids_around(&peer_a), None).unwrap();
    let cb = RelayClient::connect(addr, &idb, &rids_around(&peer_b), None).unwrap();
    // 같은 서버 = 같은 신원 키(둘 다 같은 값을 핀하게 된다).
    assert_eq!(ca.server_peer(), cb.server_peer());
    assert_eq!(ca.server_peer(), server.server_peer);

    // A가 B의 RID(공개키에서 유도 — 서버는 원본 키를 모른다)로 채널을 연다.
    let dst = rid_for(&peer_b, nbeep_relay::current_epoch_day());
    let opener = std::thread::spawn(move || {
        let (link, _peer_udp) = ca.open(dst, Duration::from_secs(10)).unwrap();
        // ★ 종단 핸드셰이크 — 서버는 바이트만 나른다(당사자가 아니다).
        let mut sess = NoiseSession::initiate(link, &ida).unwrap();
        assert_eq!(sess.peer(), peer_b, "인증된 상대 = B의 실제 키");
        sess.send(b"over the relay").unwrap();
        let echo = sess.recv().unwrap();
        assert_eq!(echo, b"over the relay");
        // 큰 메시지 — RelayLink 조각 분할·조립(RELAY_CHUNK=32KiB 초과) 검증.
        let big: Vec<u8> = (0..60_000u32).map(|i| (i % 249) as u8).collect();
        sess.send(&big).unwrap();
        let back = sess.recv().unwrap();
        assert_eq!(back.len(), 5, "종료 신호");
        ca // 소유권 유지(액터 생존)
    });

    let inc = cb.accept_incoming(Duration::from_secs(10)).unwrap();
    // 여는 쪽 RID가 A의 회전 RID 중 하나 — 내가 아는 키로 역산 가능(서버는 불가).
    assert!(
        rids_around(&peer_a).contains(&inc.src),
        "src RID = A의 회전 RID"
    );
    let mut sess = NoiseSession::accept(inc.link, &idb).unwrap();
    assert_eq!(sess.peer(), peer_a, "인증된 상대 = A의 실제 키");
    let m1 = sess.recv().unwrap();
    sess.send(&m1).unwrap(); // 에코
    let big = sess.recv().unwrap();
    assert_eq!(big.len(), 60_000, "60KB 프레임이 릴레이 조각을 넘어 보존");
    sess.send(b"done!").unwrap();

    let _ca = opener.join().unwrap();
    server.shutdown();
}

/// 서버 사칭 방지 — 핀과 다른 키의 서버는 시끄럽게 거부(DR-28 "신원이 바뀌면 시끄럽게").
#[test]
fn pin_mismatch_is_loud() {
    let server = test_server(0);
    let id = Identity::generate();
    let wrong_pin = Identity::generate().peer_id(); // 다른 서버의 키를 핀했다고 가정
    let r = RelayClient::connect(
        server.tcp_addr,
        &id,
        &rids_around(&id.peer_id()),
        Some(wrong_pin),
    );
    match r {
        Err(nbeep_relay::RelayError::PinMismatch { expected, got }) => {
            assert_eq!(expected, wrong_pin);
            assert_eq!(got, server.server_peer);
        }
        other => panic!("핀 불일치가 조용히 지나갔다: {other:?}"),
    }
    server.shutdown();
}

/// 모르는 RID로 열기 = 대상 없음(1) — 서버는 존재 스캔에 채널을 만들지 않는다.
#[test]
fn open_unknown_rid_fails() {
    let server = test_server(0);
    let id = Identity::generate();
    let c = RelayClient::connect(server.tcp_addr, &id, &rids_around(&id.peer_id()), None).unwrap();
    let ghost = rid_for(&Identity::generate().peer_id(), 1);
    assert_eq!(c.open(ghost, Duration::from_secs(5)).err(), Some(1u8));
    server.shutdown();
}

/// UDP 관측(STUN-lite) — 서버가 밖에서 본 내 UDP 엔드포인트를 돌려준다.
#[test]
fn udp_observation_echoes_mapping() {
    let server = test_server(0);
    let id = Identity::generate();
    let c = RelayClient::connect(server.tcp_addr, &id, &rids_around(&id.peer_id()), None).unwrap();
    let reg = c.register_info();
    assert_eq!(reg.udp_port, server.udp_port);
    assert!(reg.observed_tcp.is_some(), "서버가 본 내 TCP 주소");

    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let server_udp = SocketAddr::new(server.tcp_addr.ip(), server.udp_port);
    let observed = probe_udp(&sock, server_udp, reg.udp_token, Duration::from_secs(5)).unwrap();
    // 루프백에선 관측 = 로컬 주소 그대로(NAT 없음). 포트가 일치해야 매핑이 유효하다.
    assert_eq!(observed.port(), sock.local_addr().unwrap().port());
    server.shutdown();
}

/// X-UDP-c 배관 — 관측 엔드포인트 교환 → **동시 열기 펀치**(루프백) → 그 UDP 링크 위
/// 종단 Noise. 실 NAT 통과는 실기 항목(E-3류 — 추정 금지·실측 필수).
#[test]
fn hole_punch_plumbing_on_loopback() {
    let server = test_server(0);
    let addr = server.tcp_addr;
    let server_udp = SocketAddr::new(addr.ip(), server.udp_port);

    let ida = Identity::generate();
    let idb = Identity::generate();
    let peer_b = idb.peer_id();

    let ca = RelayClient::connect(addr, &ida, &rids_around(&ida.peer_id()), None).unwrap();
    let cb = RelayClient::connect(addr, &idb, &rids_around(&peer_b), None).unwrap();

    // 양쪽 다 UDP 프로브로 매핑을 서버에 관측시킨다(펀치에 쓸 소켓 그대로).
    let sock_a = UdpSocket::bind("127.0.0.1:0").unwrap();
    let sock_b = UdpSocket::bind("127.0.0.1:0").unwrap();
    probe_udp(
        &sock_a,
        server_udp,
        ca.register_info().udp_token,
        Duration::from_secs(5),
    )
    .unwrap();
    probe_udp(
        &sock_b,
        server_udp,
        cb.register_info().udp_token,
        Duration::from_secs(5),
    )
    .unwrap();

    let dst = rid_for(&peer_b, nbeep_relay::current_epoch_day());
    let a_side = std::thread::spawn(move || {
        let (_relay_link, peer_udp) = ca.open(dst, Duration::from_secs(10)).unwrap();
        let peer_udp = peer_udp.expect("B의 관측 UDP 엔드포인트");
        // 서버가 알려 준 상대 관측 주소로 동시 열기 — 같은 소켓(관측된 포트)이어야 한다.
        let link = nbeep_net::UdpLink::punch(sock_a, peer_udp, Duration::from_secs(5)).unwrap();
        let mut sess = NoiseSession::initiate(link, &ida).unwrap();
        sess.send(b"punched!").unwrap();
        assert_eq!(sess.recv().unwrap(), b"ack");
        ca
    });

    let inc = cb.accept_incoming(Duration::from_secs(10)).unwrap();
    let peer_udp = inc.peer_udp.expect("A의 관측 UDP 엔드포인트");
    let link = nbeep_net::UdpLink::punch(sock_b, peer_udp, Duration::from_secs(5)).unwrap();
    let mut sess = NoiseSession::accept(link, &idb).unwrap();
    assert_eq!(sess.recv().unwrap(), b"punched!");
    sess.send(b"ack").unwrap();

    let _ca = a_side.join().unwrap();
    server.shutdown();
}

/// 상대가 끊기면 내 채널도 닫힌다(ChClosed 전파) — 유령 채널 방지.
#[test]
fn peer_disconnect_closes_channel() {
    let server = test_server(0);
    let ida = Identity::generate();
    let idb = Identity::generate();
    let peer_b = idb.peer_id();
    let ca =
        RelayClient::connect(server.tcp_addr, &ida, &rids_around(&ida.peer_id()), None).unwrap();
    let cb = RelayClient::connect(server.tcp_addr, &idb, &rids_around(&peer_b), None).unwrap();

    let dst = rid_for(&peer_b, nbeep_relay::current_epoch_day());
    let (mut link, _) = {
        // B가 수락하도록 인바운드를 소비하는 스레드.
        let t = std::thread::spawn(move || {
            let inc = cb.accept_incoming(Duration::from_secs(10)).unwrap();
            (cb, inc)
        });
        let opened = ca.open(dst, Duration::from_secs(10)).unwrap();
        let (cb, _inc) = t.join().unwrap();
        drop(cb); // ★ B 클라이언트 전체 종료 — 세션 액터가 죽고 서버가 정리한다
        opened
    };
    // B가 사라졌으므로 A의 링크는 곧 Closed를 본다(무한 대기 금지).
    use nbeep_core::link::{Link as _, LinkError};
    link.set_recv_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match link.recv() {
            Err(LinkError::Closed) => break,
            Err(LinkError::TimedOut) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "ChClosed 전파가 오지 않았다"
                );
            }
            Ok(_) => panic!("데이터가 올 수 없는 채널"),
        }
    }
    server.shutdown();
}
