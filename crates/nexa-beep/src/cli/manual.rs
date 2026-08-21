//! 수동 엔드포인트(DR-19) — `--serve`/`--connect`(발견 우회 IP 연결·에코 왕복).

/// 수동 엔드포인트 **서버**(DR-19 · ADR-0006 §6) — 발견 없이 고정 포트로 TCP 수신,
/// 인바운드마다 Noise accept + 에코. docker에서 `-p <port>:<port>`로 노출해 맥과 IP로 붙는다.
pub(crate) fn serve_manual(port: u16) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{ChatMessage, MessageBody, Sequencer, Session as _};
    let identity = std::sync::Arc::new(nbeep_crypto::Identity::generate());
    let listener = std::net::TcpListener::bind(("0.0.0.0", port)).expect("포트 바인딩");
    listener.set_nonblocking(true).expect("논블로킹");
    let shutdown = nbeep_plat::shutdown::install();
    println!("SERVE me={} port={}", identity.peer_id().short(), port);
    loop {
        if shutdown.requested() {
            println!("SERVE 종료(정상)");
            return;
        }
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(150));
                continue;
            }
            Err(_) => continue,
        };
        let identity = std::sync::Arc::clone(&identity);
        std::thread::spawn(move || {
            let Ok(link) = nbeep_net::TcpLink::new(stream) else {
                return;
            };
            let Ok(session) = nbeep_crypto::NoiseSession::accept(
                Box::new(link) as Box<dyn nbeep_core::Link>,
                &identity,
            ) else {
                return; // 핸드셰이크 실패(자기 키 복제 U-P2 포함)
            };
            let user = session.peer();
            println!("SERVE accept from {}", user.short());
            let mut mux = MuxSession::new(session);
            let mut seq = Sequencer::new();
            while let Ok(bytes) = mux.recv(StreamId::Chat) {
                let Ok(m) = ChatMessage::decode(&bytes, user) else {
                    break;
                };
                if let MessageBody::Text(t) = m.body {
                    println!("SERVE recv from {}: {t}", user.short());
                    let reply = ChatMessage {
                        sender_device: identity.peer_id(),
                        seq: seq.issue(),
                        body: MessageBody::Text(format!("에코: {t}")),
                        importance: nbeep_core::Importance::Normal,
                        broadcast: false,
                    };
                    if mux.send(StreamId::Chat, &reply.encode()).is_err() {
                        break;
                    }
                }
            }
        });
    }
}

/// 수동 엔드포인트 **클라이언트**(DR-19) — 발견 없이 주소로 직접 연결→Noise→인사·에코 왕복.
/// LocalDirect::add_endpoint(진짜 DR-19 API)로 링크를 얻는다.
pub(crate) fn connect_manual(addr: &str) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{ChatMessage, MessageBody, Sequencer, Session as _};
    use nbeep_net::Transport as _;
    let identity = nbeep_crypto::Identity::generate();
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let name = nbeep_core::DisplayName::parse("manual").expect("라벨");
    // LocalDirect(발견 스레드 포함 — add_endpoint만 쓴다). 발견 실패해도 add_endpoint는 독립.
    // 수신 포트 0(임의) — 발신 전용 도구가 같은 PC GUI의 기본 포트(47200)를 뺏으면 안 된다.
    let transport =
        match nbeep_net::LocalDirect::spawn_on(identity.peer_id(), instance, name, 5000, 1, 0) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("전송 시작 실패: {e}");
                return;
            }
        };
    let link = match transport.add_endpoint(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("연결 실패({addr}): {e:?}");
            return;
        }
    };
    let session = match nbeep_crypto::NoiseSession::initiate(link, &identity) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("핸드셰이크 실패: {e}");
            return;
        }
    };
    let peer = session.peer();
    println!(
        "CONNECT me={} peer={}(지문 확정)",
        identity.peer_id().short(),
        peer.short()
    );
    let mut mux = MuxSession::new(session);
    let mut seq = Sequencer::new();
    let msg = ChatMessage {
        sender_device: identity.peer_id(),
        seq: seq.issue(),
        body: MessageBody::Text("안녕! 수동 연결이야".into()),
        importance: nbeep_core::Importance::Normal,
        broadcast: false,
    };
    if mux.send(StreamId::Chat, &msg.encode()).is_ok() {
        if let Ok(bytes) = mux.recv(StreamId::Chat) {
            if let Ok(r) = ChatMessage::decode(&bytes, peer) {
                if let MessageBody::Text(t) = r.body {
                    println!("CONNECT reply from {}: {t}", peer.short());
                }
            }
        }
    }
    println!("DONE");
}
