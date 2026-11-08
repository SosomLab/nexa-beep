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
    // 폴더 유일화 = pid + **프로세스 내 원자 카운터** — 시각(subsec_nanos)은 병렬
    // 테스트 스레드가 같은 값을 볼 수 있어 유일하지 않다(08-21 dir_writable 프로브
    // 경합과 같은 유형 · 실측: 테스트 8개 병렬에서 AlreadyExists 충돌).
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "beepd-test-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    spawn(&Config {
        bind_ip: IpAddr::from([127, 0, 0, 1]),
        port: 0, // 임시 — TCP가 정한 번호로 UDP 동번호
        key_path: dir.join("beepd.key"),
        rate_bps,
        verbose: false,
        ..Config::default()
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
    inc.accept(); // 지연 수락 — 이때부터 서버가 중계한다
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
    assert_eq!(
        c.open(ghost, Duration::from_secs(5)).err(),
        Some(nbeep_relay::proto::ErrorCode::NoSuchPeer as u16)
    );
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

/// X-UDP-e 사다리 1단 — [`connect_via`]/[`accept_via`]가 프로브→랑데부→**동시 펀치**를
/// 조율해 UDP 직결로 성립하고, 그 위에서 종단 Noise가 돈다(루프백).
/// 실 NAT 통과는 실기 항목(E-3류 — 추정 금지·실측 필수).
#[test]
fn ladder_takes_udp_on_loopback() {
    let server = test_server(0);
    let addr = server.tcp_addr;

    let ida = Identity::generate();
    let idb = Identity::generate();
    let peer_a = ida.peer_id();
    let peer_b = idb.peer_id();

    let ca = RelayClient::connect(addr, &ida, &rids_around(&peer_a), None).unwrap();
    let cb = RelayClient::connect(addr, &idb, &rids_around(&peer_b), None).unwrap();

    let a_side = std::thread::spawn(move || {
        let mut via =
            nbeep_relay::connect_via(&ca, &ida, &peer_b, true, Duration::from_secs(10), None)
                .unwrap();
        assert_eq!(
            via.taken,
            nbeep_relay::PathTaken::Udp,
            "루프백 펀치 = UDP 단"
        );
        assert_eq!(via.session.peer(), peer_b, "인증된 상대 = B의 실키");
        via.session.send(b"punched!").unwrap();
        assert_eq!(via.session.recv().unwrap(), b"ack");
        ca
    });

    let inc = cb.accept_incoming(Duration::from_secs(10)).unwrap();
    let mut via =
        nbeep_relay::accept_via(&cb, inc, &idb, true, Duration::from_secs(15), None).unwrap();
    assert_eq!(
        via.taken,
        nbeep_relay::PathTaken::Udp,
        "첫 프레임이 UDP로 왔다"
    );
    assert_eq!(via.session.peer(), peer_a);
    assert_eq!(via.session.recv().unwrap(), b"punched!");
    via.session.send(b"ack").unwrap();

    let _ca = a_side.join().unwrap();
    server.shutdown();
}

/// X-UDP-e 사다리 2단 — 받는 쪽이 펀치를 안 하면(관측 없음 = peer_udp None) 여는 쪽은
/// 조용히 **릴레이 단으로 내려간다**. 실패가 아니라 사다리의 정상 경로다.
#[test]
fn ladder_falls_back_to_relay() {
    let server = test_server(0);
    let addr = server.tcp_addr;

    let ida = Identity::generate();
    let idb = Identity::generate();
    let peer_b = idb.peer_id();

    let ca = RelayClient::connect(addr, &ida, &rids_around(&ida.peer_id()), None).unwrap();
    let cb = RelayClient::connect(addr, &idb, &rids_around(&peer_b), None).unwrap();

    let a_side = std::thread::spawn(move || {
        let mut via =
            nbeep_relay::connect_via(&ca, &ida, &peer_b, true, Duration::from_secs(10), None)
                .unwrap();
        assert_eq!(
            via.taken,
            nbeep_relay::PathTaken::Relay,
            "상대 관측 없음 = 펀치 생략 = 릴레이"
        );
        via.session.send(b"over relay rung").unwrap();
        ca
    });

    let inc = cb.accept_incoming(Duration::from_secs(10)).unwrap();
    assert!(inc.peer_udp.is_some(), "여는 쪽은 프로브했다(관측 있음)");
    // 받는 쪽은 펀치 없이(punch=false) — 프로브를 안 하므로 여는 쪽 OpenResult엔
    // 내 관측이 없고, 여는 쪽 사다리는 릴레이 단을 탄다.
    let mut via =
        nbeep_relay::accept_via(&cb, inc, &idb, false, Duration::from_secs(15), None).unwrap();
    assert_eq!(via.taken, nbeep_relay::PathTaken::Relay);
    assert_eq!(via.session.recv().unwrap(), b"over relay rung");

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
            inc.accept(); // 지연 수락 — 여는 쪽 open()이 이걸 기다린다
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

/// 접속 단일 정책([`nbeep_relay::attach`] — X-2b · CLI·GUI 공용)의 핀 TOFU 수명:
/// 첫 접속 = 핀 저장 → 재접속 = 핀 일치 → 핀 변조 = **PinMismatch로 중단**(조용한
/// 재핀 없음 — DR-28). `is_alive`는 서버 생존/종료를 그대로 비춘다.
#[test]
fn attach_pin_tofu_and_liveness() {
    let server = test_server(0);
    let raw = format!("127.0.0.1:{}", server.tcp_addr.port());
    let dir = std::env::temp_dir().join(format!(
        "nb-attach-{}-{}",
        std::process::id(),
        server.tcp_addr.port()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let pin = dir.join("server.pin");
    let _ = std::fs::remove_file(&pin);

    let id = Identity::generate();
    // ① 첫 접속 — TOFU 핀 저장.
    let at1 = nbeep_relay::attach(&raw, &id, &pin, &[]).unwrap();
    assert!(at1.first_pin, "핀 파일이 없으니 첫 접속");
    assert!(!at1.pin_write_failed);
    assert_eq!(at1.client.server_peer(), server.server_peer);
    assert!(at1.client.is_alive(), "접속 직후 액터 생존");
    assert_eq!(
        nbeep_relay::pinfile::lookup(&pin, &at1.addr),
        Some(server.server_peer),
        "저장된 핀 = 서버 실키"
    );
    drop(at1);

    // ② 재접속 — 핀 일치(첫 접속 아님).
    let at2 = nbeep_relay::attach(&raw, &id, &pin, &[]).unwrap();
    assert!(!at2.first_pin, "핀이 있으니 대조 경로");
    let addr = at2.addr.clone();
    drop(at2);

    // ③ 핀 변조(다른 키) — 접속 중단(시끄럽게 · 재핀은 사람의 결정).
    let wrong = Identity::generate().peer_id();
    nbeep_relay::pinfile::store(&pin, &addr, &wrong).unwrap();
    match nbeep_relay::attach(&raw, &id, &pin, &[]) {
        Err(nbeep_relay::AttachError::Relay(nbeep_relay::RelayError::PinMismatch {
            expected,
            got,
        })) => {
            assert_eq!(expected, wrong);
            assert_eq!(got, server.server_peer);
        }
        other => panic!("핀 불일치가 통과했다: {other:?}"),
    }

    // ④ 서버 종료 → 액터 사망 → is_alive = false (GUI 재접속 틱의 판정 근거).
    nbeep_relay::pinfile::store(&pin, &addr, &server.server_peer).unwrap();
    let at3 = nbeep_relay::attach(&raw, &id, &pin, &[]).unwrap();
    server.shutdown();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while at3.client.is_alive() {
        assert!(
            std::time::Instant::now() < deadline,
            "서버 종료 후에도 액터가 살아 있다"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&pin);
}

/// X-2e roster — 옵트인·상호성·스냅숏·델타·이탈 방송.
#[test]
fn roster_announce_snapshot_deltas_and_reciprocity() {
    use nbeep_relay::RosterEvent;
    fn wait_events(c: &RelayClient, n: usize) -> Vec<RosterEvent> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < n && std::time::Instant::now() < deadline {
            got.extend(c.poll_roster());
            std::thread::sleep(Duration::from_millis(20));
        }
        got
    }
    let server = test_server(0);
    let ida = Identity::generate();
    let idb = Identity::generate();
    let idc = Identity::generate();
    let (pa, pb) = (ida.peer_id(), idb.peer_id());

    let ca = RelayClient::connect(server.tcp_addr, &ida, &rids_around(&pa), None).unwrap();
    ca.set_announce(true, Vec::new()); // 혼자 — 스냅숏 없음
    let cb = RelayClient::connect(server.tcp_addr, &idb, &rids_around(&pb), None).unwrap();
    cb.set_announce(true, Vec::new());
    // B 입장 스냅숏 = A · A 델타 = B 등장.
    assert_eq!(
        wait_events(&cb, 1),
        vec![RosterEvent::Up(pa, Vec::new())],
        "입장 스냅숏"
    );
    assert_eq!(
        wait_events(&ca, 1),
        vec![RosterEvent::Up(pb, Vec::new())],
        "등장 델타"
    );

    // C = 공개 안 켬 — 아무것도 받지 못한다(상호성 — 잠복 수확 방지).
    let cc =
        RelayClient::connect(server.tcp_addr, &idc, &rids_around(&idc.peer_id()), None).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(cc.poll_roster().is_empty(), "비공개 = 목록도 못 받는다");

    // B 공개 해제 → A에 이탈 델타.
    cb.set_announce(false, Vec::new());
    assert_eq!(
        wait_events(&ca, 1),
        vec![RosterEvent::Down(pb)],
        "해제 = 이탈"
    );
    // B 재공개 후 연결 자체를 내리면 → 종료 정리가 이탈을 방송한다.
    cb.set_announce(true, Vec::new());
    assert_eq!(wait_events(&ca, 1), vec![RosterEvent::Up(pb, Vec::new())]);
    drop(cb);
    assert_eq!(
        wait_events(&ca, 1),
        vec![RosterEvent::Down(pb)],
        "연결 종료 = 이탈 방송"
    );
    drop(cc);
    server.shutdown();
}

/// X-2e 카드(08-22) — 공개 카드가 스냅숏·델타·갱신 세 경로로 전달되고,
/// 상한 초과 카드는 빈 카드로 강등된다(서버는 내용을 해석하지 않는다).
#[test]
fn roster_card_delivery_and_cap() {
    use nbeep_relay::RosterEvent;
    fn wait_events(c: &RelayClient, n: usize) -> Vec<RosterEvent> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < n && std::time::Instant::now() < deadline {
            got.extend(c.poll_roster());
            std::thread::sleep(Duration::from_millis(20));
        }
        got
    }
    let server = test_server(0);
    let ida = Identity::generate();
    let idb = Identity::generate();
    let (pa, pb) = (ida.peer_id(), idb.peer_id());
    let card_a = nbeep_relay::encode_card("Alice", "a@x.com", "hello");

    let ca = RelayClient::connect(server.tcp_addr, &ida, &rids_around(&pa), None).unwrap();
    ca.set_announce(true, card_a.clone());
    let cb = RelayClient::connect(server.tcp_addr, &idb, &rids_around(&pb), None).unwrap();
    cb.set_announce(true, Vec::new());
    // 입장 스냅숏 = A의 카드 동봉 · A 쪽 델타 = B(카드 없음).
    assert_eq!(
        wait_events(&cb, 1),
        vec![RosterEvent::Up(pa, card_a.clone())],
        "스냅숏에 카드 동봉"
    );
    assert_eq!(wait_events(&ca, 1), vec![RosterEvent::Up(pb, Vec::new())]);
    // 카드 갱신 — 같은 listed 재공지가 v2 수신자에게 재방송된다.
    let card_a2 = nbeep_relay::encode_card("Alice2", "", "");
    ca.set_announce(true, card_a2.clone());
    assert_eq!(
        wait_events(&cb, 1),
        vec![RosterEvent::Up(pa, card_a2)],
        "카드 갱신 재방송"
    );
    // 상한 초과 = 빈 카드 강등(항목은 유지).
    let idc = Identity::generate();
    let cc =
        RelayClient::connect(server.tcp_addr, &idc, &rids_around(&idc.peer_id()), None).unwrap();
    cc.set_announce(true, vec![1u8; nbeep_relay::CARD_MAX + 1]);
    let got = wait_events(&ca, 1);
    assert_eq!(
        got,
        vec![RosterEvent::Up(idc.peer_id(), Vec::new())],
        "초과 = 빈 카드"
    );
    // 디코더 왕복.
    assert_eq!(
        nbeep_relay::decode_card(&card_a),
        Some(("Alice".into(), "a@x.com".into(), "hello".into()))
    );
    drop((ca, cb, cc));
    server.shutdown();
}

// ── 와이어 v2(ADR-0016 · docs/56) ──────────────────────────────────────

use nbeep_core::link::Link as _;
use nbeep_relay::proto;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};

/// 원시 v2 핸드셰이크(프리앰블 순차 왕복 → Noise prologue) — 봉투를 직접 보내는 시험용.
fn raw_v2(server: &nexa_beepd::Handle, id: &Identity) -> NoiseSession<nbeep_net::TcpLink> {
    let mut s = TcpStream::connect(server.tcp_addr).unwrap();
    s.write_all(&proto::preamble::encode(proto::PROTO_MAJOR))
        .unwrap();
    let mut pre = [0u8; proto::preamble::LEN];
    s.read_exact(&mut pre).unwrap();
    assert_eq!(proto::preamble::parse(&pre), Some(proto::PROTO_MAJOR));
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let link = nbeep_net::TcpLink::new(s).unwrap();
    NoiseSession::initiate_with(link, id, Some(proto::PROLOGUE)).unwrap()
}

fn recv_env(sess: &mut NoiseSession<nbeep_net::TcpLink>) -> proto::Envelope {
    proto::Envelope::decode(&sess.recv().unwrap()).unwrap()
}

fn hello(app: &str) -> Vec<u8> {
    proto::Hello {
        proto_major: proto::PROTO_MAJOR,
        proto_minor: proto::PROTO_MINOR,
        app: app.into(),
        app_version: "test".into(),
        caps: proto::caps::ROSTER | proto::caps::UDP_OBSERVE,
    }
    .encode()
    .encode()
}

fn connect_app(
    server: &nexa_beepd::Handle,
    id: &Identity,
    app: &str,
    bind: proto::BindMode,
) -> RelayClient {
    RelayClient::connect_as(
        server.tcp_addr,
        id,
        &rids_around(&id.peer_id()),
        None,
        app,
        "test",
        bind,
    )
    .unwrap()
}

/// v1 클라(프리앰블 없음)·다른 세대(major 3)는 **명시적으로** 거절되고 계측에 남는다. 반대로 클라가 프리앰블을
/// 모르는 서버(v1)나 다른 세대 서버를 만나면 `VersionMismatch{server_major}`(0 = 프리앰블 없음)로 **표시 가능**하다.
#[test]
fn generation_mismatch_is_explicit_both_ways() {
    let server = test_server(0);
    // v1 클라 흉내 — [len u32 = 0x20][e 32B]: 서버가 첫 8B에서 갈라 끊는다.
    let mut s = TcpStream::connect(server.tcp_addr).unwrap();
    s.write_all(&[0, 0, 0, 0x20]).unwrap();
    s.write_all(&[7u8; 32]).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut b = [0u8; 8];
    assert!(matches!(s.read(&mut b), Ok(0) | Err(_)), "v1 클라는 끊긴다");
    // 다른 세대(major 3) — 서버는 자기 major만 알리고 끊는다.
    let mut s = TcpStream::connect(server.tcp_addr).unwrap();
    s.write_all(&proto::preamble::encode(3)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.read_exact(&mut b).unwrap();
    assert_eq!(proto::preamble::parse(&b), Some(proto::PROTO_MAJOR));
    assert!(matches!(s.read(&mut b), Ok(0) | Err(_)));
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while server.metrics().rejected_version < 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let m = server.metrics();
    assert_eq!((m.rejected_v1, m.rejected_version), (1, 1));

    // 클라 ← 프리앰블을 모르는 서버(v1 흉내: 44B 읽고 끊음) = server_major 0.
    let id = Identity::generate();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let t = std::thread::spawn(move || {
        let (mut c, _) = l.accept().unwrap();
        let mut junk = [0u8; 44];
        let _ = c.read_exact(&mut junk);
    });
    match RelayClient::connect(addr, &id, &rids_around(&id.peer_id()), None) {
        Err(nbeep_relay::RelayError::VersionMismatch { server_major: 0 }) => {}
        other => panic!("v1 서버 → VersionMismatch{{0}} 기대: {other:?}"),
    }
    t.join().unwrap();
    // 클라 ← major 3 서버 = server_major 3.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let t = std::thread::spawn(move || {
        let (mut c, _) = l.accept().unwrap();
        let mut pre = [0u8; 8];
        c.read_exact(&mut pre).unwrap();
        c.write_all(&proto::preamble::encode(3)).unwrap();
        let mut junk = [0u8; 36];
        let _ = c.read_exact(&mut junk);
    });
    match RelayClient::connect(addr, &id, &rids_around(&id.peer_id()), None) {
        Err(nbeep_relay::RelayError::VersionMismatch { server_major: 3 }) => {}
        other => panic!("major 3 서버 → VersionMismatch{{3}} 기대: {other:?}"),
    }
    t.join().unwrap();
    server.shutdown();
}

/// 헬스 프로브 — `Hello{app="probe"}`는 Welcome(버전·종류·능력·상한·관측)만 받고 끊긴다(등록·명부 없음).
#[test]
fn probe_hello_gets_welcome_then_close() {
    let server = test_server(0);
    let id = Identity::generate();
    let mut s = raw_v2(&server, &id);
    s.send(&hello(proto::app::PROBE)).unwrap();
    let env = recv_env(&mut s);
    assert_eq!(env.kind(), Some(proto::Kind::Welcome));
    let w = proto::Welcome::decode(&env).unwrap();
    assert_eq!(w.proto_major, proto::PROTO_MAJOR);
    assert_eq!(w.server_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(w.server_type, proto::ServerType::Relay);
    assert_eq!(w.caps, proto::caps::ROSTER | proto::caps::UDP_OBSERVE);
    assert_eq!(
        w.limits,
        proto::Limits {
            rate_bps: 0,
            ..proto::Limits::default()
        }
    );
    assert_eq!(w.udp_port, server.udp_port);
    assert!(matches!(w.observed, proto::Ep::V4([127, 0, 0, 1], _)));
    assert!(s.recv().is_err(), "프로브는 Welcome 뒤 끊긴다");
    assert_eq!(server.metrics().probes, 1);
    server.shutdown();
}

/// 오류 통로 — Hello 전 요청 = `NotRegistered`+끊김 · 모르는 핵심 kind = `Error{UnknownKind, ref_kind}` 응답 ·
/// 확장 kind = 침묵 · 등록 전 Open = `NotRegistered`(연결 유지) · Pong은 nonce를 되비춘다.
#[test]
fn unknown_kind_is_answered_and_order_is_enforced() {
    let server = test_server(0);
    let id = Identity::generate();
    let mut s = raw_v2(&server, &id);
    s.send(
        &proto::Register {
            rids: vec![[1u8; 16]],
            bind: proto::BindMode::ExclusiveLastWins,
        }
        .encode()
        .encode(),
    )
    .unwrap();
    let env = recv_env(&mut s);
    assert_eq!(env.kind(), Some(proto::Kind::Error));
    let p = proto::Problem::decode(&env).unwrap();
    assert_eq!(p.code, proto::ErrorCode::NotRegistered as u16);
    assert_eq!(p.ref_kind, Some(proto::Kind::Register as u16));
    assert!(s.recv().is_err(), "Hello 없는 세션은 끊긴다");

    let mut s = raw_v2(&server, &id);
    s.send(&hello(proto::app::BEEP)).unwrap();
    assert_eq!(recv_env(&mut s).kind(), Some(proto::Kind::Welcome));
    let unknown_core = proto::Envelope {
        kind: 0x0050,
        flags: 0,
        body: Vec::new(),
    };
    s.send(&unknown_core.encode()).unwrap();
    let env = recv_env(&mut s);
    let p = proto::Problem::decode(&env).unwrap();
    assert_eq!(p.code, proto::ErrorCode::UnknownKind as u16);
    assert_eq!(p.ref_kind, Some(0x0050));
    let unknown_ext = proto::Envelope {
        kind: 0x4001,
        flags: 0,
        body: Vec::new(),
    };
    s.send(&unknown_ext.encode()).unwrap();
    s.send(&proto::ping(proto::Kind::Ping, 42).encode())
        .unwrap();
    let env = recv_env(&mut s);
    assert_eq!(
        env.kind(),
        Some(proto::Kind::Pong),
        "확장 kind는 침묵 — 다음 응답은 Pong"
    );
    assert_eq!(proto::nonce(&env).unwrap(), 42);
    let mut open = proto::Channel::empty();
    open.token = Some(1);
    open.rid = Some([9u8; 16]);
    s.send(&open.encode(proto::Kind::Open).encode()).unwrap();
    let env = recv_env(&mut s);
    let p = proto::Problem::decode(&env).unwrap();
    assert_eq!(p.code, proto::ErrorCode::NotRegistered as u16);
    assert_eq!(p.ref_kind, Some(proto::Kind::Open as u16));
    s.send(&proto::ping(proto::Kind::Ping, 43).encode())
        .unwrap();
    assert_eq!(
        proto::nonce(&recv_env(&mut s)).unwrap(),
        43,
        "연결은 유지된다"
    );
    assert!(server.metrics().errors_sent >= 3);
    server.shutdown();
}

/// 앱 이름공간 — 서버는 `(app, rid)`로 라우팅한다: 다른 앱의 RID는 **보이지 않고**(`NoSuchPeer`) 같은 앱끼리만 열린다.
/// `ServerInfo`(Welcome)가 클라에 남는다.
#[test]
fn app_namespace_isolates_rids() {
    let server = test_server(0);
    let (ida, idb, idc) = (
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
    );
    let b = connect_app(
        &server,
        &idb,
        proto::app::CLIP,
        proto::BindMode::ExclusiveLastWins,
    );
    let a =
        RelayClient::connect(server.tcp_addr, &ida, &rids_around(&ida.peer_id()), None).unwrap();
    assert_eq!(a.server_info().version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        a.server_info().caps,
        proto::caps::ROSTER | proto::caps::UDP_OBSERVE
    );
    let dst = rid_for(&idb.peer_id(), nbeep_relay::current_epoch_day());
    assert_eq!(
        a.open(dst, Duration::from_secs(5)).err(),
        Some(proto::ErrorCode::NoSuchPeer as u16),
        "beep → clip RID는 보이지 않는다"
    );
    let c = connect_app(
        &server,
        &idc,
        proto::app::CLIP,
        proto::BindMode::ExclusiveLastWins,
    );
    let t = std::thread::spawn(move || (c.open(dst, Duration::from_secs(10)).map(|_| ()), c));
    let inc = b
        .accept_incoming(Duration::from_secs(5))
        .expect("같은 앱 → Incoming");
    inc.accept();
    let (r, _c) = t.join().unwrap();
    assert!(r.is_ok(), "clip → clip은 열린다: {r:?}");
    server.shutdown();
}

/// Shared 바인딩(docs/44 I-2) — 같은 RID에 두 기기: Incoming은 **둘 다** 받고 첫 Accept가 채널을 잡는다(다른 쪽은 ChClosed).
/// 둘 다 거절하면 여는 쪽은 `Busy`.
#[test]
fn shared_bind_fans_out_and_first_accept_wins() {
    let server = test_server(0);
    let (ida, idb) = (Identity::generate(), Identity::generate());
    let b1 = connect_app(&server, &idb, proto::app::BEEP, proto::BindMode::Shared);
    let b2 = connect_app(&server, &idb, proto::app::BEEP, proto::BindMode::Shared);
    let a =
        RelayClient::connect(server.tcp_addr, &ida, &rids_around(&ida.peer_id()), None).unwrap();
    let dst = rid_for(&idb.peer_id(), nbeep_relay::current_epoch_day());

    let t = std::thread::spawn(move || (a.open(dst, Duration::from_secs(10)), a));
    let i1 = b1
        .accept_incoming(Duration::from_secs(5))
        .expect("b1 Incoming");
    let i2 = b2
        .accept_incoming(Duration::from_secs(5))
        .expect("b2 Incoming(팬아웃)");
    i1.accept();
    let (r, a) = t.join().unwrap();
    let (mut la, _) = r.expect("첫 Accept = 성립");
    let mut l2 = i2.link;
    l2.set_recv_timeout(Some(Duration::from_secs(3))).unwrap();
    assert!(
        matches!(l2.recv(), Err(nbeep_core::link::LinkError::Closed)),
        "늦은 후보는 ChClosed"
    );
    la.send(b"hi").unwrap();
    let mut l1 = i1.link;
    l1.set_recv_timeout(Some(Duration::from_secs(3))).unwrap();
    assert_eq!(l1.recv().unwrap(), b"hi");

    // 전원 거절 → Busy.
    let t = std::thread::spawn(move || (a.open(dst, Duration::from_secs(10)).map(|_| ()), a));
    let j1 = b1.accept_incoming(Duration::from_secs(5)).unwrap();
    let j2 = b2.accept_incoming(Duration::from_secs(5)).unwrap();
    drop(j1);
    drop(j2);
    let (r, _a) = t.join().unwrap();
    assert_eq!(r.err(), Some(proto::ErrorCode::Busy as u16));
    assert_eq!(server.metrics().channels_total, 1);
    server.shutdown();
}

/// 우아한 종료 — `Handle::shutdown`이 연결마다 `Goaway{Shutdown}`을 보내고, 클라 액터는 그걸로 내려간다(호스트 백오프 재접속).
#[test]
fn server_shutdown_sends_goaway() {
    let server = test_server(0);
    let id = Identity::generate();
    let c = RelayClient::connect(server.tcp_addr, &id, &rids_around(&id.peer_id()), None).unwrap();
    assert!(c.is_alive());
    server.shutdown();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while c.is_alive() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!c.is_alive(), "Goaway 뒤 액터 종료");
}
