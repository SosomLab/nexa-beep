#![allow(clippy::unwrap_used)] // 테스트 — 실패 = 패닉이 곧 보고(13 §9 프로덕션 한정)

//! M1-11 — **와이어 평문 자동 회귀**(SEC-1 잔여 절반 · CI 상시).
//!
//! 종전 검증은 수동 캡처(`tools/wirecap.sh` · M1-12)뿐이라 회귀가 없었다. 이
//! 테스트는 **실경로 그대로**(TcpLink → Noise XX → Mux → Chat/Control/File
//! 인코딩) 두 신원을 성립시키되, 중간에 **투명 TCP 릴레이**를 끼워 와이어에
//! 흐른 바이트 전부를 캡처한다 — "릴레이는 봉투만 본다"(DR-7)의 테스트판.
//! 캡처에서 마커 문자열이 발견되면 평문 유출이다(fail-closed).
//!
//! ⚠ 마커가 **수신측에 실제로 도달**했는지도 단언한다 — 안 그러면 "안 보냈으니
//! 안 새는" 빈 테스트가 된다(전송이 실경로였음의 증명).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use nbeep_core::mux::StreamId;
use nbeep_core::{ChatMessage, MessageBody, MuxSession, ProfileMsg, Session as _, XferMsg};
use nbeep_net::TcpLink;

/// 와이어에 실려서는 안 되는 평문 마커 — 우연 일치가 불가능한 무작위성 문자열.
const MARK_CHAT: &str = "PLAINTEXT-CANARY-chat-7f3a9d";
const MARK_EMAIL: &str = "canary-7f3a9d@leak.example";
const MARK_FILE: &str = "PLAINTEXT-CANARY-file-7f3a9d.txt";
const MARK_CONTENT: &str = "PLAINTEXT-CANARY-content-7f3a9d";

/// 투명 TCP 릴레이 — a↔b 사이 바이트를 복사하며 **전부** 캡처한다.
fn spawn_relay(to_addr: std::net::SocketAddr) -> (std::net::SocketAddr, Arc<Mutex<Vec<u8>>>) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").expect("릴레이 바인드");
    let addr = listener.local_addr().expect("주소");
    let cap = Arc::clone(&captured);
    std::thread::spawn(move || {
        let (client, _) = listener.accept().expect("릴레이 accept");
        let server = TcpStream::connect(to_addr).expect("릴레이 연결");
        let c2 = client.try_clone().expect("clone");
        let s2 = server.try_clone().expect("clone");
        let cap_a = Arc::clone(&cap);
        let cap_b = Arc::clone(&cap);
        let pump = |mut from: TcpStream, mut to: TcpStream, cap: Arc<Mutex<Vec<u8>>>| {
            move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match from.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            cap.lock().unwrap().extend_from_slice(&buf[..n]);
                            if to.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        };
        let t1 = std::thread::spawn(pump(client, server, cap_a));
        let t2 = std::thread::spawn(pump(s2, c2, cap_b));
        let _ = t1.join();
        let _ = t2.join();
    });
    (addr, captured)
}

/// 바이트열에서 부분열 검색(표준 라이브러리엔 없다).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn session_wire_carries_no_plaintext() {
    // ── 종단 B(수락측) ──
    let listener = TcpListener::bind("127.0.0.1:0").expect("B 바인드");
    let b_addr = listener.local_addr().expect("주소");
    let b = std::thread::spawn(move || {
        let id_b = nbeep_crypto::Identity::generate();
        let (s, _) = listener.accept().expect("accept");
        let link = TcpLink::new(s).expect("링크");
        let session = nbeep_crypto::NoiseSession::accept(link, &id_b).expect("핸드셰이크");
        let peer_a = session.peer();
        let mut mux = MuxSession::new(session);
        // 도달 검증(빈 테스트 방지) — 3종을 받는다(도착 순서 무관 recv_any).
        let (mut got_chat, mut got_profile, mut got_offer, mut got_chunk) =
            (false, false, false, false);
        for _ in 0..4 {
            let (stream, bytes) = mux.recv_any().expect("수신");
            match stream {
                StreamId::Chat => {
                    let m = ChatMessage::decode(&bytes, peer_a).expect("chat");
                    let MessageBody::Text(t) = m.body else {
                        panic!("텍스트")
                    };
                    assert_eq!(t, MARK_CHAT);
                    got_chat = true;
                }
                StreamId::Control => {
                    let Some(ProfileMsg::Info { email, .. }) = ProfileMsg::decode(&bytes) else {
                        panic!("profile")
                    };
                    assert_eq!(email.as_deref(), Some(MARK_EMAIL));
                    got_profile = true;
                }
                StreamId::File => match XferMsg::decode(&bytes) {
                    Ok(XferMsg::Offer { name, .. }) => {
                        assert_eq!(String::from_utf8_lossy(&name), MARK_FILE);
                        got_offer = true;
                    }
                    Ok(XferMsg::Chunk { data, .. }) => {
                        assert_eq!(String::from_utf8_lossy(&data), MARK_CONTENT);
                        got_chunk = true;
                    }
                    other => panic!("예상 밖 파일 프레임: {other:?}"),
                },
                other => panic!("예상 밖 스트림: {other:?}"),
            }
        }
        assert!(
            got_chat && got_profile && got_offer && got_chunk,
            "4종 전부 도달해야 한다"
        );
    });

    // ── 릴레이(캡처) + 종단 A(개시측) ──
    let (relay_addr, captured) = spawn_relay(b_addr);
    let id_a = nbeep_crypto::Identity::generate();
    let link = TcpLink::new(TcpStream::connect(relay_addr).expect("connect")).expect("링크");
    let session = nbeep_crypto::NoiseSession::initiate(link, &id_a).expect("핸드셰이크");
    let mut mux = MuxSession::new(session);

    // 실경로 인코딩 그대로 3종 송신(대화·프로필·파일 오퍼).
    let chat = ChatMessage {
        seq: 1,
        sender_device: id_a.peer_id(),
        body: MessageBody::Text(MARK_CHAT.to_string()),
        importance: nbeep_core::Importance::Normal,
    };
    mux.send(StreamId::Chat, &chat.encode()).expect("chat 송신");
    let info = ProfileMsg::Info {
        name: None,
        email: Some(MARK_EMAIL.into()),
        phone: None,
        image_len: 0,
        avatar: None,
        border: None,
        image_keep: false,
        bio: None,
    };
    mux.send(StreamId::Control, &info.encode())
        .expect("info 송신");
    let offer = XferMsg::Offer {
        id: [9u8; 16],
        size: 12,
        sha256: [0u8; 32],
        name: MARK_FILE.as_bytes().to_vec(),
    };
    mux.send(StreamId::File, &offer.encode())
        .expect("offer 송신");
    // 파일 **내용**(청크) — 오퍼 메타만 검사하면 실데이터 평문 유출을 놓친다.
    let chunk = XferMsg::Chunk {
        id: [9u8; 16],
        offset: 0,
        data: MARK_CONTENT.as_bytes().to_vec(),
    };
    mux.send(StreamId::File, &chunk.encode())
        .expect("chunk 송신");

    b.join().expect("B 종단 정상 종료");

    // ── 판정 — 와이어 캡처에 평문 마커가 없어야 한다 ──
    let wire = captured.lock().unwrap();
    assert!(!wire.is_empty(), "캡처가 비었다 — 릴레이 경유가 아니었다");
    for (what, mark) in [
        ("대화 본문", MARK_CHAT),
        ("프로필 이메일", MARK_EMAIL),
        ("파일 이름", MARK_FILE),
        ("파일 내용", MARK_CONTENT),
    ] {
        assert!(
            !contains(&wire, mark.as_bytes()),
            "★ 평문 유출: {what}({mark})이 와이어에 그대로 보인다"
        );
    }
}

// 발견 브로드캐스트의 프로필 부재는 타입이 보장한다 — `nbeep_net::Packet`에
// 프로필 필드 자체가 없다(DR-22). 필드가 생기는 순간 이 주석이 회귀의 자리다.
