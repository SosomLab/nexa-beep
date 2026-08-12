//! 헤드리스 검증 도구 — 발견 프로브(`--discover-probe`)·종단 에코(`--live-echo`).

use nbeep_core::PeerId;

/// 헤드리스 발견 프로브(M1-4 · D-8a) — 실물 멀티캐스트로 `secs`초 동안 발견을 찍는다.
/// Docker 컨테이너 N개 교차 검증·실기 D-8b에서 그대로 쓴다(창 불요).
pub(crate) fn discover_probe(secs: u64) {
    let identity = nbeep_crypto::Identity::generate();
    let me = identity.peer_id();
    // 기동 인스턴스 난수(D-22 U-P1) — 별도 키 생성의 앞 16B(프로브 한정 간이 난수).
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let name = nbeep_core::DisplayName::parse(&format!("probe-{}", me.short()))
        .expect("지문 라벨은 항상 유효");
    let d = nbeep_net::udp::UdpDiscovery::spawn(me, instance, name, 0, 1, 500)
        .expect("발견 시작 실패(방화벽·인터페이스)");
    let events = d.take_events();
    println!("PROBE me={} {}s", me.short(), secs);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut clones = nbeep_net::CloneWatch::new(10_000);
    let started = std::time::Instant::now();
    let shutdown = nbeep_plat::shutdown::install();
    while std::time::Instant::now() < deadline && !shutdown.requested() {
        if let Ok(o) = events.recv_timeout(std::time::Duration::from_millis(300)) {
            let now = nbeep_core::MonoInstant(
                u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
            let clone_flag = clones.observe(o.packet.peer, o.packet.instance, now);
            println!(
                "SAW peer={} name={} kind={:?} from={}{}",
                o.packet.peer.short(),
                o.packet.name.as_str(),
                o.packet.kind,
                o.from,
                if clone_flag { " ⚠️CLONE" } else { "" }
            );
        }
    }
    println!("DONE");
}

/// 실물 종단(M1-4 · D-8a) — LocalDirect로 발견→연결→Noise→대화 왕복을 헤드리스로 검증한다.
/// 두 노드를 띄우면 서로 발견하고, 개시자가 첫 상대에게 메시지를 보내 에코를 받는다.
pub(crate) fn live_echo(secs: u64) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{ChatMessage, MessageBody, Sequencer, Session as _};
    use nbeep_net::Transport as _;

    let identity = std::sync::Arc::new(nbeep_crypto::Identity::generate());
    let me = identity.peer_id();
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let name = nbeep_core::DisplayName::parse(&format!("live-{}", me.short())).expect("라벨");
    // 수신 포트 0(임의) — 테스트 단말은 발견 광고로 포트를 알리므로 고정 포트가 필요 없고,
    // 같은 PC의 GUI가 기본 포트(47200)를 가져가야 한다.
    let transport = std::sync::Arc::new(
        nbeep_net::LocalDirect::spawn_on(me, instance, name, 500, 1, 0).expect("LocalDirect 시작"),
    );
    println!("LIVE me={} {}s", me.short(), secs);

    // 수신 서버 스레드 — 들어온 링크를 accept해 에코한다(누구든 상대).
    {
        let transport = std::sync::Arc::clone(&transport);
        let identity = std::sync::Arc::clone(&identity);
        std::thread::spawn(move || {
            let incoming = transport.incoming();
            while let Ok(link) = incoming.recv() {
                let identity = std::sync::Arc::clone(&identity);
                std::thread::spawn(move || {
                    let Ok(session) = nbeep_crypto::NoiseSession::accept(link, &identity) else {
                        return;
                    };
                    let user = session.peer();
                    let mut mux = MuxSession::new(session);
                    let mut seq = Sequencer::new();
                    while let Ok(bytes) = mux.recv(StreamId::Chat) {
                        let Ok(m) = ChatMessage::decode(&bytes, user) else {
                            break;
                        };
                        if let MessageBody::Text(t) = m.body {
                            println!("SERVER recv from {}: {t}", user.short());
                            let reply = ChatMessage {
                                sender_device: identity.peer_id(),
                                seq: seq.issue(),
                                body: MessageBody::Text(format!("에코: {t}")),
                            };
                            if mux.send(StreamId::Chat, &reply.encode()).is_err() {
                                break;
                            }
                        }
                    }
                });
            }
        });
    }

    // 발견 → 첫 미지 상대에게 연결 시도(개시자 역할).
    let discovery = transport.discovery();
    let mut seq = Sequencer::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut connected: Option<PeerIdShort> = None;
    let shutdown = nbeep_plat::shutdown::install();
    while std::time::Instant::now() < deadline && !shutdown.requested() {
        let Ok(ev) = discovery.recv_timeout(std::time::Duration::from_millis(300)) else {
            continue;
        };
        let nbeep_net::DiscoveryEvent::Appeared(hint) = ev else {
            continue;
        };
        if connected.as_ref().map(|c| c.0) == Some(hint.peer) {
            continue; // 이미 이 상대와 대화함
        }
        match transport.connect(hint.peer) {
            Ok(link) => match nbeep_crypto::NoiseSession::initiate(link, &identity) {
                Ok(session) => {
                    let peer = session.peer();
                    let mut mux = MuxSession::new(session);
                    let msg = ChatMessage {
                        sender_device: me,
                        seq: seq.issue(),
                        body: MessageBody::Text("안녕! 실물 세션이야".into()),
                    };
                    if mux.send(StreamId::Chat, &msg.encode()).is_ok() {
                        if let Ok(bytes) = mux.recv(StreamId::Chat) {
                            if let Ok(r) = ChatMessage::decode(&bytes, peer) {
                                if let MessageBody::Text(t) = r.body {
                                    println!("CLIENT got reply from {}: {t}", peer.short());
                                }
                            }
                        }
                    }
                    connected = Some(PeerIdShort(hint.peer));
                }
                Err(e) => println!("핸드셰이크 실패({}): {e}", hint.peer.short()),
            },
            Err(e) => println!("연결 실패({}): {e:?}", hint.peer.short()),
        }
    }
    println!("DONE");
}

/// 연결 상태 추적용 뉴타입(중복 연결 방지).
struct PeerIdShort(PeerId);
