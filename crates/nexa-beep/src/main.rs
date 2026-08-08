//! Nexa Beep 본체 — 진입·조립·생명주기.
//!
//! `--window` = InMemory 데모(에코 봇). **`--window --live` = 실물 발견(LocalDirect)** —
//! 같은 LAN·컨테이너의 실제 상대가 목록에 뜨고 진짜 Noise 세션으로 대화한다.
//! (구 문구) InMemory 종단 데모: 실물 발견(에코 봇) → 목록 → Enter →
//! Noise 핸드셰이크 → TOFU 핀 → 다중화 세션 위 대화 왕복. 전 계층이 실물이다.
//!
//! **창 모드(DR-26 · FR-U-18)**: 기본 = 단일 창(목록↔대화 전환). `--separate-windows` =
//! **상대별 별도 OS 창**(동시 대화). 대화 상태(`Conversation`)는 어느 모드든 뷰와 분리되어
//! 유지된다. ⚠️ 모드 선택의 설정 화면 연동(`chat.window_mode`)은 M3-11 — 그 전까지 실행 인자.
//!
//! 실물 네트워크 배선은 M1-4, 창 코드의 `nbeep-plat` 이관은 M3-2.
//! 기본 실행은 스캐폴드 출력(헤드리스 CI 안전).

// 조립 지점 바이너리 — 창 초기화 경로의 unwrap 허용(docs/13 §9 — 복구 불가 구성 오류).
#![allow(clippy::unwrap_used)]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--discover-probe") {
        let secs = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(8);
        discover_probe(secs);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--live-echo") {
        let secs = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(15);
        live_echo(secs);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--serve") {
        let port = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(47_200);
        serve_manual(port);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--connect") {
        let Some(addr) = args.get(pos + 1).cloned() else {
            eprintln!("--connect <host:port> 필요");
            return;
        };
        connect_manual(&addr);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--chat-serve") {
        let port = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(47_200);
        chat_interactive(ChatRole::Serve(port));
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--chat-connect") {
        let Some(addr) = args.get(pos + 1).cloned() else {
            eprintln!("--chat-connect <host:port> 필요");
            return;
        };
        chat_interactive(ChatRole::Connect(addr));
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--chat-live") {
        let name = args
            .get(pos + 1)
            .cloned()
            .unwrap_or_else(|| "터미널".into());
        chat_live(&name);
        return;
    }
    let open_window = args.iter().any(|a| a == "--window");
    let separate = args.iter().any(|a| a == "--separate-windows");
    let live = args.iter().any(|a| a == "--live");
    if open_window || separate {
        let mode = if separate {
            app_window::WindowMode::Separate
        } else {
            app_window::WindowMode::Single
        };
        app_window::run(mode, live);
    } else {
        println!(
            "nexa-beep {} — scaffold (창 `--window [--live]` · 발견 `--discover-probe [초]` · 수동 `--serve`/`--connect` · 인터랙티브 `--chat-serve [port]`/`--chat-connect <host:port>`/`--chat-live [이름]`(GUI 목록에 뜸))",
            env!("CARGO_PKG_VERSION")
        );
    }
}

/// 헤드리스 발견 프로브(M1-4 · D-8a) — 실물 멀티캐스트로 `secs`초 동안 발견을 찍는다.
/// Docker 컨테이너 N개 교차 검증·실기 D-8b에서 그대로 쓴다(창 불요).
fn discover_probe(secs: u64) {
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

/// 수동 엔드포인트 **서버**(DR-19 · ADR-0006 §6) — 발견 없이 고정 포트로 TCP 수신,
/// 인바운드마다 Noise accept + 에코. docker에서 `-p <port>:<port>`로 노출해 맥과 IP로 붙는다.
fn serve_manual(port: u16) {
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
fn connect_manual(addr: &str) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{ChatMessage, MessageBody, Sequencer, Session as _};
    use nbeep_net::Transport as _;
    let identity = nbeep_crypto::Identity::generate();
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let name = nbeep_core::DisplayName::parse("manual").expect("라벨");
    // LocalDirect(발견 스레드 포함 — add_endpoint만 쓴다). 발견 실패해도 add_endpoint는 독립.
    let transport = match nbeep_net::LocalDirect::spawn(identity.peer_id(), instance, name, 5000, 1)
    {
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

/// 인터랙티브 채팅 역할.
enum ChatRole {
    /// 고정 포트로 한 상대의 연결을 기다린다.
    Serve(u16),
    /// 주소로 직접 연결한다(DR-19 수동).
    Connect(String),
}

/// **인터랙티브 헤드리스 채팅**(사람이 stdin으로 타이핑·수신 실시간 출력). docker 터미널
/// (`-it`)과 맥 GUI/터미널이 사람 대 사람으로 양방향 대화한다. 세션 스택은 GUI와 동일
/// (Noise→TOFU→다중화) — 프레젠테이션만 stdin/stdout.
fn chat_interactive(role: ChatRole) {
    let identity = nbeep_crypto::Identity::generate();

    // 역할별로 Link 하나를 얻는다(서버=accept, 클라=connect). 이후 경로는 100% 같다.
    let link: Box<dyn nbeep_core::Link> = match &role {
        ChatRole::Serve(port) => {
            let listener = std::net::TcpListener::bind(("0.0.0.0", *port)).expect("포트 바인딩");
            println!(
                "[대기] {} 에서 상대를 기다립니다… (me={})",
                port,
                identity.peer_id().short()
            );
            let (stream, from) = listener.accept().expect("accept");
            println!("[연결] {from} 에서 연결됨");
            Box::new(nbeep_net::TcpLink::new(stream).expect("링크"))
        }
        ChatRole::Connect(addr) => {
            let mut instance = [0u8; 16];
            instance
                .copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
            let name = nbeep_core::DisplayName::parse("chat").expect("라벨");
            let transport =
                nbeep_net::LocalDirect::spawn(identity.peer_id(), instance, name, 5000, 1)
                    .expect("전송");
            use nbeep_net::Transport as _;
            match transport.add_endpoint(addr) {
                Ok(l) => {
                    println!(
                        "[연결] {addr} 로 연결됨 (me={})",
                        identity.peer_id().short()
                    );
                    l
                }
                Err(e) => {
                    eprintln!("[실패] 연결 실패({addr}): {e:?}");
                    return;
                }
            }
        }
    };

    // 핸드셰이크(서버=accept·클라=initiate) → 신원 확정.
    let session = match &role {
        ChatRole::Serve(_) => nbeep_crypto::NoiseSession::accept(link, &identity),
        ChatRole::Connect(_) => nbeep_crypto::NoiseSession::initiate(link, &identity),
    };
    let session = match session {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[실패] 핸드셰이크: {e}");
            return;
        }
    };
    run_interactive(session, identity.peer_id());
}

/// 수립된 세션 위 **인터랙티브 대화 루프** — stdin 라인 = 전송, 수신은 실시간 출력, Ctrl+D 종료.
/// serve/connect/live가 공용(세션 출처만 다르고 대화는 같다).
fn run_interactive<L: nbeep_core::Link + 'static>(
    mut session: nbeep_crypto::NoiseSession<L>,
    me: PeerId,
) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{ChatMessage, MessageBody, Sequencer, Session as _};
    use std::io::BufRead as _;

    let peer = session.peer();
    println!(
        "[대화 시작] 상대={} · 한 줄 입력 = 전송, Ctrl+D = 종료",
        peer.short()
    );
    // 수신 스레드: 세션 소유·recv 폴·도착 출력(액터 — 한 세션 1스레드). 송신은 채널 교대.
    session.set_recv_timeout(Some(std::time::Duration::from_millis(150)));
    let (out_tx, out_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let net = std::thread::spawn(move || {
        let mut mux = MuxSession::new(session);
        loop {
            loop {
                match out_rx.try_recv() {
                    Ok(bytes) => {
                        if mux.send(StreamId::Chat, &bytes).is_err() {
                            println!("[종료] 세션 끊김");
                            return;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            match mux.recv(StreamId::Chat) {
                Ok(bytes) => {
                    if let Ok(m) = ChatMessage::decode(&bytes, peer) {
                        if let MessageBody::Text(t) = m.body {
                            let safe = nbeep_core::sanitize_message(&t);
                            println!("{}> {}", peer.short(), safe.as_str());
                        }
                    }
                }
                Err(nbeep_core::SessionError::TimedOut) => {}
                Err(_) => {
                    println!("[종료] 세션 끊김");
                    return;
                }
            }
        }
    });
    let mut seq = Sequencer::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        let msg = ChatMessage {
            sender_device: me,
            seq: seq.issue(),
            body: MessageBody::Text(line),
        };
        if out_tx.send(msg.encode()).is_err() {
            break;
        }
    }
    drop(out_tx);
    let _ = net.join();
    println!("[끝]");
}

/// **발견 가능한 인터랙티브 클라이언트**(`--chat-live [이름]`) — LocalDirect로 **발견 광고**(GUI
/// 목록에 뜬다) + 첫 인바운드(GUI가 클릭해 연결) 수락 → 인터랙티브 대화. 실행 중인 `--window --live`
/// GUI를 터미널에서 붙어 테스트하는 용도.
fn chat_live(name: &str) {
    use nbeep_net::Transport as _;
    let identity = nbeep_crypto::Identity::generate();
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let display = nbeep_core::DisplayName::parse(name)
        .unwrap_or_else(|_| nbeep_core::DisplayName::parse("chat-live").expect("라벨"));
    let transport =
        match nbeep_net::LocalDirect::spawn(identity.peer_id(), instance, display, 800, 1) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[실패] 전송 시작: {e}");
                return;
            }
        };
    println!(
        "[대기] '{name}'(me={}) 로 발견 광고 중 — 실행 중인 GUI(--window --live) 목록에서 클릭하세요…",
        identity.peer_id().short()
    );
    let incoming = transport.incoming();
    let Ok(link) = incoming.recv() else {
        eprintln!("[실패] 인바운드 없음");
        return;
    };
    match nbeep_crypto::NoiseSession::accept(link, &identity) {
        Ok(session) => run_interactive(session, identity.peer_id()),
        Err(e) => eprintln!("[실패] 핸드셰이크: {e}"),
    }
}

/// 실물 종단(M1-4 · D-8a) — LocalDirect로 발견→연결→Noise→대화 왕복을 헤드리스로 검증한다.
/// 두 노드를 띄우면 서로 발견하고, 개시자가 첫 상대에게 메시지를 보내 에코를 받는다.
fn live_echo(secs: u64) {
    use nbeep_core::mux::{MuxSession, StreamId};
    use nbeep_core::{ChatMessage, MessageBody, Sequencer, Session as _};
    use nbeep_net::Transport as _;

    let identity = std::sync::Arc::new(nbeep_crypto::Identity::generate());
    let me = identity.peer_id();
    let mut instance = [0u8; 16];
    instance.copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
    let name = nbeep_core::DisplayName::parse(&format!("live-{}", me.short())).expect("라벨");
    let transport = std::sync::Arc::new(
        nbeep_net::LocalDirect::spawn(me, instance, name, 500, 1).expect("LocalDirect 시작"),
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
use nbeep_core::PeerId;

/// 창 + 위젯 조립 — winit 이벤트를 `InputEvent`로 번역해 위젯에 라우팅.
mod app_window {
    use std::collections::HashMap;
    use std::num::NonZeroU32;
    use std::rc::Rc;
    use std::time::Instant;
    use winit::application::ApplicationHandler;
    use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::keyboard::{Key as WKey, NamedKey};
    use winit::window::{Window, WindowId};

    use nbeep_core::PeerId;
    use nbeep_ui::{
        ChatLine, ChatViewWidget, DrawCtx, GalleryWidget, InputEvent, Invalidations, Key,
        PeerListWidget, PeerRow, RasterCtx, Rect, SettingsState, SettingsWidget, Theme, Widget,
    };

    type SbSurface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

    /// 창 모드(DR-26). 설정 연동(`chat.window_mode`)은 M3-11.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum WindowMode {
        /// 한 창에서 목록 ↔ 대화 전환(v1 기본).
        Single,
        /// 대화마다 상대별 OS 창(동시 대화).
        Separate,
    }

    /// 열린 대화의 실물 세션 스택 — Noise(암호화) 위 TOFU(신뢰) 위 다중화.
    type LiveSession = nbeep_core::MuxSession<
        nbeep_core::TrustedSession<nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>>,
    >;

    /// GUI 이벤트 루프를 깨우는 커스텀 이벤트(M2-7 비동기 수신 펌프) — 세션 액터 스레드가
    /// `EventLoopProxy`로 보낸다. winit 단일 스레드 규약을 지키면서 백그라운드 수신을 GUI에 반영.
    #[derive(Debug)]
    enum AppEvent {
        /// 상대가 보낸 대화 한 줄(복호·검증·무해화 완료).
        Recv {
            peer: PeerId,
            text: nbeep_core::SafeText,
            seq: u64,
            sender: PeerId,
        },
        /// 세션 종료(상대 이탈·오류).
        Closed { peer: PeerId },
        /// **인바운드** — 남이 나에게 연결해 핸드셰이크가 끝난 세션(아직 TOFU 미판정).
        /// GUI(메인 스레드)가 TrustStore로 판정 후 대화·창을 연다.
        Inbound { session: Box<InboundSession> },
    }

    /// 인바운드 세션 봉투 — `AppEvent`가 Debug라야 해서 수동 Debug.
    struct InboundSession {
        session: nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>,
    }
    impl std::fmt::Debug for InboundSession {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("InboundSession").finish_non_exhaustive()
        }
    }

    /// 대화 상태 — **뷰(창)와 분리**(DR-26). 세션은 **액터 스레드**가 소유하고, 여기엔 그
    /// 액터로 보내는 송신 채널과 스레드 이력만 둔다(M2-7 — 비동기 수신 펌프).
    struct Conversation {
        /// 액터에 보낼 발신 바이트(인코딩된 `ChatMessage`). 드롭 = 액터 종료 신호.
        out_tx: std::sync::mpsc::Sender<Vec<u8>>,
        lines: Vec<ChatLine>,
    }

    /// 세션 액터 — 세션을 전용 스레드로 옮겨 **수신(GUI로 프록시)과 송신(채널)을 교대**한다.
    /// snow `TransportState`가 read/write에 `&mut`를 요구해 한 세션은 한 스레드가 소유해야
    /// 하므로, 송신은 채널로 요청받는 액터 모델이 정석이다.
    fn spawn_session_actor(
        mut session: LiveSession,
        out_rx: std::sync::mpsc::Receiver<Vec<u8>>,
        proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    ) {
        use nbeep_core::mux::StreamId;
        let peer = session.peer();
        // 수신 폴 타임아웃 — recv가 200ms마다 TimedOut으로 돌아와 송신과 교대한다.
        session.set_recv_timeout(Some(std::time::Duration::from_millis(200)));
        std::thread::spawn(move || {
            loop {
                // 송신 먼저(즉시성) — 대기 중 발신 요청을 모두 흘려보낸다.
                loop {
                    match out_rx.try_recv() {
                        Ok(bytes) => {
                            if session.send(StreamId::Chat, &bytes).is_err() {
                                let _ = proxy.send_event(AppEvent::Closed { peer });
                                return;
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => return, // 대화 닫힘
                    }
                }
                // 수신 폴.
                match session.recv(StreamId::Chat) {
                    Ok(bytes) => {
                        if let Ok(m) = nbeep_core::ChatMessage::decode(&bytes, peer) {
                            if let nbeep_core::MessageBody::Text(t) = m.body {
                                let ev = AppEvent::Recv {
                                    peer,
                                    text: nbeep_core::sanitize_message(&t),
                                    seq: m.seq,
                                    sender: m.sender_device,
                                };
                                if proxy.send_event(ev).is_err() {
                                    return; // 이벤트 루프 종료
                                }
                            }
                        }
                    }
                    Err(nbeep_core::SessionError::TimedOut) => {} // 정상 — 송신 교대로
                    Err(_) => {
                        let _ = proxy.send_event(AppEvent::Closed { peer });
                        return;
                    }
                }
            }
        });
    }

    /// OS 창 하나 — 역할(목록/특정 대화) + 표면 + 창별 상태.
    struct WinEntry {
        role: Role,
        window: Rc<Window>,
        surface: SbSurface,
        cursor: (i32, i32),
        scale: f32,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Role {
        /// 주 창 — 목록(단일 모드에서는 대화 전환도 이 창에서).
        Main,
        /// 상대별 대화 창(Separate 모드).
        Chat(PeerId),
        /// 설정 창(`Cmd/Ctrl+,` — DR-24).
        Settings,
        /// 컨트롤 갤러리(임시 검수 — `Cmd/Ctrl+G` 또는 하단 버튼).
        Gallery,
    }

    /// 에코 봇 — 버스에 실물 신원으로 참여해 수신 세션을 받고, Chat 메시지를 에코한다.
    /// 발견 힌트의 `PeerId` == Noise 정적 키(실제 아키텍처 그대로 — 인증이 발견을 검증한다).
    fn spawn_echo_bot(bus: &std::sync::Arc<nbeep_net::inmem::InMemoryBus>, name: &str) {
        use nbeep_core::mux::{MuxSession, StreamId};
        use nbeep_core::{ChatMessage, DisplayName, MessageBody, Sequencer, Session as _};
        let identity = nbeep_crypto::Identity::generate();
        let display = DisplayName::parse(name).unwrap();
        let transport = bus.join(identity.peer_id(), display, nbeep_net::Caps::default());
        std::thread::spawn(move || {
            use nbeep_net::Transport as _;
            let incoming = transport.incoming();
            while let Ok(link) = incoming.recv() {
                let Ok(session) = nbeep_crypto::NoiseSession::accept(link, &identity) else {
                    continue;
                };
                let user = session.peer();
                let mut mux = MuxSession::new(session);
                let mut seq = Sequencer::new();
                while let Ok(bytes) = mux.recv(StreamId::Chat) {
                    let Ok(msg) = ChatMessage::decode(&bytes, user) else {
                        break; // 위조·손상 = 세션 종료(fail-closed)
                    };
                    let MessageBody::Text(text) = msg.body else {
                        continue;
                    };
                    let reply = ChatMessage {
                        sender_device: identity.peer_id(),
                        seq: seq.issue(),
                        body: MessageBody::Text(format!("에코: {text}")),
                    };
                    if mux.send(StreamId::Chat, &reply.encode()).is_err() {
                        break;
                    }
                }
            }
        });
    }

    /// 인바운드 세션 수락 펌프 — 남이 나에게 연결하면 accept 후 에코 응답(대칭 대화 데모).
    /// GUI 스레드로의 실시간 수신 반영(양쪽 창에 표시)은 M2-7(비동기 수신 펌프·EventLoopProxy).
    /// 인바운드 수락 펌프 — 남이 나에게 연결하면 **핸드셰이크만** 하고(블로킹) 완성 세션을
    /// GUI로 넘긴다. TOFU 판정·대화 생성은 메인 스레드(user_event) 몫 — TrustStore가 거기 있다.
    fn spawn_inbound_accept(
        incoming: std::sync::mpsc::Receiver<Box<dyn nbeep_core::Link>>,
        identity: std::sync::Arc<nbeep_crypto::Identity>,
        proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    ) {
        std::thread::spawn(move || {
            while let Ok(link) = incoming.recv() {
                let identity = std::sync::Arc::clone(&identity);
                let proxy = proxy.clone();
                std::thread::spawn(move || {
                    // 핸드셰이크(블로킹) — 실패(자기 키 복제 U-P2 포함)면 조용히 버린다.
                    let Ok(session) = nbeep_crypto::NoiseSession::accept(link, &identity) else {
                        return;
                    };
                    let _ = proxy.send_event(AppEvent::Inbound {
                        session: Box::new(InboundSession { session }),
                    });
                });
            }
        });
    }

    /// 샘플 찾기 어댑터 — 한 폴더의 **단일 파일 선택기**(Adapter 패턴 실증).
    /// `nbeep_ui::ChoosePicker`를 구현한 어떤 화면도 Choose에 꽂을 수 있다(UI 계층은 I/O를 모른다).
    #[derive(Debug)]
    struct FilePicker {
        dir: std::path::PathBuf,
    }

    impl nbeep_ui::ChoosePicker for FilePicker {
        fn title(&self) -> String {
            format!("파일 선택 — {}", self.dir.display())
        }
        fn items(&self) -> Vec<nbeep_ui::ComboItem> {
            let mut v = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&self.dir) {
                for e in rd.flatten() {
                    if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                        let name = e.file_name().to_string_lossy().into_owned();
                        v.push(nbeep_ui::ComboItem::new(name.clone(), name).with_icon("▤"));
                    }
                }
            }
            v.sort_by(|a, b| a.label.cmp(&b.label));
            v
        }
    }

    struct App {
        mode: WindowMode,
        windows: HashMap<WindowId, WinEntry>,
        main_id: Option<WindowId>,
        font: nbeep_gfx::Font,
        theme: Theme,
        list: PeerListWidget,
        /// 대화 뷰 — Separate 모드는 창당 1개, Single 모드는 주 창에 최대 1개.
        chats: HashMap<PeerId, ChatViewWidget>,
        /// Single 모드에서 주 창에 표시 중인 대화(None = 목록).
        single_open: Option<PeerId>,
        /// 내 신원 — 발견·세션·발신 봉투가 전부 이 키 하나에서 나온다.
        identity: std::sync::Arc<nbeep_crypto::Identity>,
        seq: nbeep_core::Sequencer,
        /// 전송 — 데모(InMemory) 또는 실물(`LocalDirect`). 같은 `Transport` 트레이트라 App은 구별 안 함.
        transport: Box<dyn nbeep_net::Transport>,
        discovery: std::sync::mpsc::Receiver<nbeep_net::DiscoveryEvent>,
        table: nbeep_core::PeerTable,
        trust: nbeep_core::MemoryTrustStore,
        /// 상대별 대화 상태 — 뷰와 무관하게 유지(동시 대화의 실체).
        conversations: HashMap<PeerId, Conversation>,
        dedup: nbeep_core::DedupIndex,
        started: Instant,
        /// 주 창 하단 상태바 문구.
        status: String,
        /// 설정 값(런타임 — 영속은 M2-5). `chat.window_mode`·`ui.theme`·`font.*`.
        settings: SettingsState,
        /// 영역별 글꼴 설정(설정에서 파생 — 크기·굵기·기울임).
        fonts: nbeep_ui::FontPrefs,
        /// 열린 설정 창의 뷰(설정 창은 항상 별도 OS 창 1개).
        settings_view: Option<SettingsWidget>,
        /// 컨트롤 갤러리 뷰(임시 검수) — 열려 있을 때만 Some.
        gallery_view: Option<GalleryWidget>,
        /// OS 주 수식키(⌘/Ctrl) 눌림 상태 — `Cmd/Ctrl+,` 판정.
        primary_down: bool,
        /// 세션 액터가 GUI를 깨우는 통로(M2-7).
        proxy: winit::event_loop::EventLoopProxy<AppEvent>,
        /// 수동 엔드포인트 입력 중 버퍼(DR-19 · `⌘/Ctrl+K`). None = 입력 아님.
        adding: Option<String>,
        /// 종료 신호(R-16 · FR-P-7) — SIGINT/SIGTERM 시 el.exit() → Drop 체인이 GOODBYE·정리.
        shutdown: nbeep_plat::shutdown::Shutdown,
    }

    impl App {
        fn now_ms(&self) -> u64 {
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
        }

        fn bar_h(scale: f32) -> i32 {
            (26.0 * scale).round() as i32
        }

        /// 주 창 하단바 오른쪽의 "컨트롤 갤러리" 버튼 rect(임시 검수 진입점).
        fn gallery_btn(win_w: i32, win_h: i32, scale: f32) -> Rect {
            let bar_h = Self::bar_h(scale);
            let m = (4.0 * scale).round() as i32;
            let bw = (108.0 * scale).round() as i32;
            Rect::new(win_w - bw - m, win_h - bar_h + m, bw, bar_h - m * 2)
        }

        fn request_redraw(&self, id: WindowId) {
            if let Some(e) = self.windows.get(&id) {
                e.window.request_redraw();
            }
        }

        /// 이 창에 현재 열린 대화 상대(있으면) — IME 프리에딧 라우팅용.
        fn chat_peer_for(&self, id: WindowId) -> Option<PeerId> {
            match self.windows.get(&id).map(|e| e.role) {
                Some(Role::Chat(peer)) => Some(peer),
                Some(Role::Main) => self.single_open,
                _ => None,
            }
        }

        /// `peer` 대화가 보이는 창을 다시 그린다(Separate = 그 창, Single = 주 창이 이 대화일 때).
        fn redraw_conversation(&self, peer: PeerId) {
            match self.mode {
                WindowMode::Separate => {
                    if let Some((id, _)) = self
                        .windows
                        .iter()
                        .find(|(_, e)| e.role == Role::Chat(peer))
                    {
                        self.request_redraw(*id);
                    }
                }
                WindowMode::Single => {
                    if self.single_open == Some(peer) {
                        if let Some(id) = self.main_id {
                            self.request_redraw(id);
                        }
                    }
                }
            }
        }

        /// 발견 이벤트를 PeerTable에 접고 목록을 다시 만든다.
        fn poll_discovery(&mut self) {
            use nbeep_net::DiscoveryEvent;
            let mut changed = false;
            let now = nbeep_core::MonoInstant(
                u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            );
            while let Ok(ev) = self.discovery.try_recv() {
                match ev {
                    DiscoveryEvent::Appeared(hint) => {
                        self.table
                            .observe(hint.peer, hint.name, nbeep_core::SourceId(0), now);
                    }
                    DiscoveryEvent::Vanished(peer) => {
                        self.table.goodbye(peer, nbeep_core::SourceId(0));
                    }
                }
                changed = true;
            }
            if changed {
                let mut inv = Invalidations::default();
                self.refresh_rows(&mut inv);
                if let Some(id) = self.main_id {
                    self.request_redraw(id);
                }
            }
        }

        /// 목록 행 재구성 — 발견(PeerTable) + 신뢰(TrustStore)의 조립.
        fn refresh_rows(&mut self, inv: &mut Invalidations) {
            use nbeep_core::TrustStore as _;
            let rows = self
                .table
                .list()
                .into_iter()
                .map(|entry| {
                    let trust = self.trust.level(entry.peer);
                    PeerRow { entry, trust }
                })
                .collect();
            self.list.set_rows(rows, inv);
        }

        /// 연결 → Noise 핸드셰이크 → TOFU 판정 → 다중화(실물 스택 전체).
        fn open_session(
            &mut self,
            peer: PeerId,
        ) -> Result<(LiveSession, nbeep_core::TrustDecision), String> {
            // dyn Transport — 메서드는 트레이트 객체에 직접 있어 use 불요.
            let link = self.transport.connect(peer).map_err(|e| format!("{e:?}"))?;
            let noise = nbeep_crypto::NoiseSession::initiate(link, &self.identity)
                .map_err(|e| e.to_string())?;
            let est = nbeep_core::TrustedSession::wrap(noise, &mut self.trust)
                .map_err(|e| e.to_string())?;
            Ok((nbeep_core::MuxSession::new(est.session), est.decision))
        }

        /// 수동 주소로 세션 수립(DR-19) — add_endpoint(발견 우회)→Noise→TOFU→대화 등록.
        /// 반환 = 확정된 `PeerId`(주소는 힌트·신원은 지문). ⚠️ 핸드셰이크는 블로킹(LAN 수십 ms).
        fn open_session_addr(&mut self, addr: &str) -> Result<PeerId, String> {
            use nbeep_core::Session as _;
            let link = self
                .transport
                .add_endpoint(addr)
                .map_err(|e| format!("{e:?}"))?;
            let noise = nbeep_crypto::NoiseSession::initiate(link, &self.identity)
                .map_err(|e| e.to_string())?;
            let est = nbeep_core::TrustedSession::wrap(noise, &mut self.trust)
                .map_err(|e| e.to_string())?;
            let peer = est.session.peer();
            self.install_conversation(nbeep_core::MuxSession::new(est.session));
            Ok(peer)
        }

        /// 수동 입력 확정 — 주소로 연결하고 대화를 연다.
        fn commit_manual_add(&mut self, addr: String, el: &ActiveEventLoop) {
            let addr = addr.trim().to_string();
            self.adding = None;
            if addr.is_empty() {
                return;
            }
            match self.open_session_addr(&addr) {
                Ok(peer) => {
                    self.status = format!("수동 연결 성공 — {}", peer.short());
                    // 대화 뷰 열기(이미 conversation 등록됨 → ensure는 복원 경로).
                    self.activate(peer, el);
                }
                Err(e) => self.status = format!("수동 연결 실패({addr}): {e}"),
            }
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
        }

        fn peer_title(&self, peer: PeerId) -> String {
            self.table
                .get(peer)
                .map_or_else(|| format!("{peer:?}"), |e| e.name.as_str().to_string())
        }

        /// 수립된 세션을 액터로 옮기고 대화 상태를 등록한다(아웃바운드·인바운드 공용).
        fn install_conversation(&mut self, session: LiveSession) {
            let peer = session.peer();
            let (out_tx, out_rx) = std::sync::mpsc::channel();
            spawn_session_actor(session, out_rx, self.proxy.clone());
            self.conversations.insert(
                peer,
                Conversation {
                    out_tx,
                    lines: Vec::new(),
                },
            );
        }

        /// 대화 뷰 생성(스레드 복원 — 상태-뷰 분리).
        fn build_chat_view(&self, peer: PeerId) -> ChatViewWidget {
            let mut chat = ChatViewWidget::new(self.peer_title(peer));
            let mut inv = Invalidations::default();
            if let Some(conv) = self.conversations.get(&peer) {
                for line in &conv.lines {
                    chat.push_line(line.clone(), &mut inv);
                }
            }
            chat
        }

        /// Separate 모드 — `peer` 대화의 별도 창을 생성한다(이미 있으면 포커스).
        fn open_separate_window(
            &mut self,
            peer: PeerId,
            chat: ChatViewWidget,
            el: &ActiveEventLoop,
        ) {
            if let Some((id, _)) = self
                .windows
                .iter()
                .find(|(_, e)| e.role == Role::Chat(peer))
            {
                if let Some(e) = self.windows.get(id) {
                    e.window.focus_window();
                }
                return;
            }
            let title = format!("Nexa Beep — {}", self.peer_title(peer));
            let attrs = Window::default_attributes().with_title(title);
            let window = Rc::new(el.create_window(attrs).unwrap());
            window.set_ime_allowed(true);
            let scale = window.scale_factor() as f32;
            let context = softbuffer::Context::new(window.clone()).unwrap();
            let surface = SbSurface::new(&context, window.clone()).unwrap();
            let id = window.id();
            self.windows.insert(
                id,
                WinEntry {
                    role: Role::Chat(peer),
                    window,
                    surface,
                    cursor: (0, 0),
                    scale,
                },
            );
            self.chats.insert(peer, chat);
            self.layout_window(id);
            let mut inv = Invalidations::default();
            self.refresh_rows(&mut inv);
            if let Some(mid) = self.main_id {
                self.request_redraw(mid);
            }
            self.request_redraw(id);
        }

        /// 대화 상태 확보(없으면 세션 수립) + 뷰 생성(스레드 복원).
        fn ensure_conversation(&mut self, peer: PeerId) -> Result<ChatViewWidget, String> {
            if !self.conversations.contains_key(&peer) {
                let (session, decision) = self.open_session(peer)?;
                self.install_conversation(session);
                self.status = match decision {
                    nbeep_core::TrustDecision::FirstContact => {
                        "Noise 세션 수립 — 첫 접촉(TOFU 핀 고정)".into()
                    }
                    d => format!("Noise 세션 수립 — {d:?}"),
                };
            } else {
                self.status = "대화 복원 — 세션 유지 중".into();
            }
            Ok(self.build_chat_view(peer))
        }

        /// 설정 창을 연다(있으면 포커스) — `Cmd/Ctrl+,`.
        fn open_settings(&mut self, el: &ActiveEventLoop) {
            if let Some((id, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Settings) {
                if let Some(e) = self.windows.get(id) {
                    e.window.focus_window();
                }
                return;
            }
            let attrs = Window::default_attributes().with_title(format!(
                "Nexa Beep — {}",
                nbeep_core::t(nbeep_core::Msg::SettingsTitle)
            ));
            let window = Rc::new(el.create_window(attrs).unwrap());
            window.set_ime_allowed(true);
            let scale = window.scale_factor() as f32;
            let context = softbuffer::Context::new(window.clone()).unwrap();
            let surface = SbSurface::new(&context, window.clone()).unwrap();
            let id = window.id();
            self.windows.insert(
                id,
                WinEntry {
                    role: Role::Settings,
                    window,
                    surface,
                    cursor: (0, 0),
                    scale,
                },
            );
            self.settings_view = Some(SettingsWidget::new(&self.settings));
            self.layout_window(id);
            self.request_redraw(id);
        }

        /// 컨트롤 갤러리 창을 연다(임시 검수 — 이미 열려 있으면 포커스).
        fn open_gallery(&mut self, el: &ActiveEventLoop) {
            if let Some((gid, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Gallery) {
                if let Some(e) = self.windows.get(gid) {
                    e.window.focus_window();
                }
                return;
            }
            let attrs = Window::default_attributes().with_title("Nexa Beep — 컨트롤 갤러리 (임시)");
            let window = Rc::new(el.create_window(attrs).unwrap());
            window.set_ime_allowed(true);
            let scale = window.scale_factor() as f32;
            let context = softbuffer::Context::new(window.clone()).unwrap();
            let surface = SbSurface::new(&context, window.clone()).unwrap();
            let id = window.id();
            self.windows.insert(
                id,
                WinEntry {
                    role: Role::Gallery,
                    window,
                    surface,
                    cursor: (0, 0),
                    scale,
                },
            );
            let mut gallery = GalleryWidget::new();
            // Choose 컨트롤에 샘플 어댑터(단일 파일 선택기) 주입 — Adapter 패턴 실증.
            let dir = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            gallery.set_choose_picker(Box::new(FilePicker { dir }));
            self.gallery_view = Some(gallery);
            self.layout_window(id);
            self.request_redraw(id);
        }

        /// 설정 변경 즉시 적용(DR-24 — 저장 버튼 없음).
        /// 설정 값에서 영역별 글꼴 설정을 만든다(크기 키 s/m/l/xl → px).
        fn fonts_from_settings(settings: &SettingsState) -> nbeep_ui::FontPrefs {
            // 글꼴명(font.{region}.family)은 SettingsState에 저장되지만, 실제 패밀리 로드는
            // 시스템 폰트 열거(M3-3 확장) 후 연결한다 — 지금은 크기만 렌더에 반영.
            let slot = |region: &str, base: f32| -> nbeep_ui::SlotFont {
                let size = match settings.get(&format!("font.{region}.size")) {
                    "s" => base - 2.0,
                    "l" => base + 3.0,
                    "xl" => base + 7.0,
                    _ => base, // "m"·미설정 = 기본
                };
                nbeep_ui::SlotFont {
                    size,
                    bold: false,
                    italic: false,
                }
            };
            nbeep_ui::FontPrefs {
                base: slot("base", 16.0),
                peerlist: slot("peerlist", 16.0),
                message: slot("message", 18.0),
                status: slot("status", 13.0),
            }
        }

        fn apply_settings(&mut self, changes: Vec<(&'static str, String)>) {
            for (key, value) in changes {
                self.settings.set(key, value.clone());
                match key {
                    "chat.window_mode" => {
                        // 새 대화부터 적용(DR-26 — 열린 창은 유지·소급 강제 없음).
                        self.mode = if value == "separate" {
                            WindowMode::Separate
                        } else {
                            WindowMode::Single
                        };
                        self.status = format!("창 모드 = {value} (새 대화부터 적용)");
                    }
                    "ui.theme" => {
                        self.theme = if value == "light" {
                            Theme::light()
                        } else {
                            Theme::dark()
                        };
                        // 전 창 다시 그리기.
                        for e in self.windows.values() {
                            e.window.request_redraw();
                        }
                    }
                    "ui.language" => {
                        // 현재 언어 전환 — 전 위젯이 다음 렌더에서 새 언어로 그린다.
                        nbeep_core::set_lang(
                            nbeep_core::Lang::from_code(&value).unwrap_or_default(),
                        );
                        for e in self.windows.values() {
                            e.window.request_redraw();
                        }
                    }
                    k if k.starts_with("font.") => {
                        self.fonts = Self::fonts_from_settings(&self.settings);
                        self.status = "글꼴 설정 적용됨".into();
                        for e in self.windows.values() {
                            e.window.request_redraw();
                        }
                    }
                    _ => {}
                }
            }
            if let Some(id) = self.main_id {
                self.request_redraw(id);
            }
        }

        /// 대화 활성화 — 모드에 따라 주 창 전환 또는 별도 창 생성/포커스(14 §11).
        fn activate(&mut self, peer: PeerId, el: &ActiveEventLoop) {
            match self.mode {
                WindowMode::Single => match self.ensure_conversation(peer) {
                    Ok(chat) => {
                        self.chats.insert(peer, chat);
                        self.single_open = Some(peer);
                        if let Some(id) = self.main_id {
                            self.layout_window(id);
                            let mut inv = Invalidations::default();
                            self.refresh_rows(&mut inv);
                            self.request_redraw(id);
                        }
                    }
                    Err(e) => self.status = format!("연결 실패: {e}"),
                },
                WindowMode::Separate => match self.ensure_conversation(peer) {
                    Ok(chat) => self.open_separate_window(peer, chat, el),
                    Err(e) => {
                        self.status = format!("연결 실패: {e}");
                        if let Some(mid) = self.main_id {
                            self.request_redraw(mid);
                        }
                    }
                },
            }
        }

        /// 창 크기·배율에 맞춰 그 창의 위젯 경계를 다시 계산한다.
        fn layout_window(&mut self, id: WindowId) {
            let Some(entry) = self.windows.get(&id) else {
                return;
            };
            let size = entry.window.inner_size();
            let scale = entry.scale;
            let role = entry.role;
            let w = i32::try_from(size.width).unwrap_or(i32::MAX);
            let h = i32::try_from(size.height).unwrap_or(i32::MAX);
            let mut inv = Invalidations::default();
            match role {
                Role::Main => {
                    let body = (h - Self::bar_h(scale)).max(0);
                    self.list.set_scale(scale, &mut inv);
                    self.list.set_bounds(Rect::new(0, 0, w, body), &mut inv);
                    if let Some(chat) = self.single_open.and_then(|p| self.chats.get_mut(&p)) {
                        chat.set_scale(scale, &mut inv);
                        chat.set_bounds(Rect::new(0, 0, w, body), &mut inv);
                    }
                }
                Role::Chat(peer) => {
                    if let Some(chat) = self.chats.get_mut(&peer) {
                        chat.set_scale(scale, &mut inv);
                        chat.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                    }
                }
                Role::Settings => {
                    if let Some(sv) = &mut self.settings_view {
                        sv.set_scale(scale, &mut inv);
                        sv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                    }
                }
                Role::Gallery => {
                    if let Some(gv) = &mut self.gallery_view {
                        gv.set_scale(scale, &mut inv);
                        gv.set_bounds(Rect::new(0, 0, w, h), &mut inv);
                    }
                }
            }
        }

        /// 대화 뷰에서 나온 발신·복귀를 처리한다. `peer` = 그 뷰의 상대.
        fn drain_chat_effects(&mut self, peer: PeerId, id: WindowId) {
            let mut inv = Invalidations::default();
            let outgoing = self
                .chats
                .get_mut(&peer)
                .and_then(ChatViewWidget::take_outgoing);
            if let Some(text) = outgoing {
                let msg = nbeep_core::ChatMessage {
                    sender_device: self.identity.peer_id(),
                    seq: self.seq.issue(),
                    body: nbeep_core::MessageBody::Text(text.as_str().to_string()),
                };
                if let Some(chat) = self.chats.get_mut(&peer) {
                    chat.push_line(
                        ChatLine {
                            mine: true,
                            text: text.clone(),
                        },
                        &mut inv,
                    );
                }
                if let Some(conv) = self.conversations.get_mut(&peer) {
                    conv.lines.push(ChatLine { mine: true, text });
                    // 액터에 발신 요청 — 수신은 비동기로 AppEvent::Recv로 돌아온다(M2-7).
                    if conv.out_tx.send(msg.encode()).is_err() {
                        self.status = "세션 종료됨".into();
                    } else {
                        self.status = format!("전송 seq={} (응답 대기)", msg.seq);
                    }
                }
                self.request_redraw(id);
                if let Some(mid) = self.main_id {
                    self.request_redraw(mid); // 상태바 갱신
                }
            }
            if self
                .chats
                .get_mut(&peer)
                .is_some_and(ChatViewWidget::take_back)
            {
                // 뷰만 닫는다 — 대화(세션·스레드)는 유지(DR-26).
                self.chats.remove(&peer);
                match self.mode {
                    WindowMode::Single => {
                        self.single_open = None;
                        self.status =
                            "↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기".into();
                        self.request_redraw(id);
                    }
                    WindowMode::Separate => {
                        self.windows.remove(&id); // 창 닫힘(드롭) — 대화는 유지
                    }
                }
            }
        }

        /// 이 창의 입력 이벤트를 담당 위젯으로 라우팅한다.
        fn route(&mut self, id: WindowId, ev: InputEvent, el: &ActiveEventLoop) {
            let Some(entry) = self.windows.get(&id) else {
                return;
            };
            let role = entry.role;
            let mut inv = Invalidations::default();
            match role {
                Role::Main => {
                    if let Some(peer) = self.single_open {
                        if let Some(chat) = self.chats.get_mut(&peer) {
                            chat.on_event(&ev, &mut inv);
                        }
                        self.drain_chat_effects(peer, id);
                    } else {
                        self.list.on_event(&ev, &mut inv);
                        if let Some(peer) = self.list.take_activated() {
                            self.activate(peer, el);
                        }
                    }
                }
                Role::Chat(peer) => {
                    if let Some(chat) = self.chats.get_mut(&peer) {
                        chat.on_event(&ev, &mut inv);
                    }
                    self.drain_chat_effects(peer, id);
                }
                Role::Settings => {
                    if let Some(sv) = &mut self.settings_view {
                        sv.on_event(&ev, &mut inv);
                        let changes = sv.take_changes();
                        let close = sv.take_back();
                        if !changes.is_empty() {
                            self.apply_settings(changes);
                        }
                        if close {
                            self.settings_view = None;
                            self.windows.remove(&id);
                        }
                    }
                }
                Role::Gallery => {
                    // Esc = 닫기. 그 외는 갤러리 위젯으로 전달.
                    if matches!(
                        ev,
                        InputEvent::Key {
                            key: Key::Escape,
                            ..
                        }
                    ) {
                        self.gallery_view = None;
                        self.windows.remove(&id);
                    } else if let Some(gv) = &mut self.gallery_view {
                        gv.on_event(&ev, &mut inv);
                    }
                }
            }
            if !inv.is_empty() {
                self.request_redraw(id);
            }
        }

        fn redraw(&mut self, id: WindowId) {
            let theme = self.theme;
            let Some(entry) = self.windows.get_mut(&id) else {
                return;
            };
            let size = entry.window.inner_size();
            let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
            else {
                return;
            };
            entry.surface.resize(w, h).unwrap();
            let mut buffer = entry.surface.buffer_mut().unwrap();
            let mut px =
                nbeep_gfx::Surface::new(&mut buffer, size.width as usize, size.height as usize);
            px.fill(theme.window_bg);
            let mut ctx = RasterCtx::new(&mut px, &self.font)
                .with_fonts(self.fonts)
                .with_scale(entry.scale);
            match entry.role {
                Role::Main => {
                    if let Some(chat) = self.single_open.and_then(|p| self.chats.get(&p)) {
                        chat.paint(&mut ctx, &theme);
                    } else {
                        self.list.paint(&mut ctx, &theme);
                    }
                    // 주 창 하단 상태바.
                    let hh = i32::try_from(size.height).unwrap_or(i32::MAX);
                    let ww = i32::try_from(size.width).unwrap_or(i32::MAX);
                    let bar_h = Self::bar_h(entry.scale);
                    let bar = Rect::new(0, hh - bar_h, ww, bar_h);
                    ctx.select_font(nbeep_ui::FontSlot::Status, false);
                    let pad = (8.0 * entry.scale).round() as i32;
                    let dy = (bar_h - (14.0 * entry.scale) as i32) / 2;
                    let bar_text = match &self.adding {
                        Some(buf) => format!("주소(host:port): {buf}▏"),
                        None => self.status.clone(),
                    };
                    ctx.text_opaque(
                        bar.x + pad,
                        bar.y + dy,
                        bar,
                        &bar_text,
                        theme.text_dim,
                        theme.chrome_bg,
                    );
                    // 임시: "컨트롤 갤러리" 버튼(⌘/Ctrl+G).
                    let btn = Self::gallery_btn(ww, hh, entry.scale);
                    ctx.fill_round_rect(btn, (6.0 * entry.scale) as i32, theme.accent);
                    ctx.select_font(nbeep_ui::FontSlot::Status, false);
                    let label = "🎛 컨트롤";
                    let lw = ctx.text_width(label);
                    ctx.text(
                        btn.x + (btn.w - lw) / 2,
                        btn.y + (btn.h - (13.0 * entry.scale) as i32) / 2,
                        btn,
                        label,
                        theme.panel_bg,
                    );
                }
                Role::Chat(peer) => {
                    if let Some(chat) = self.chats.get(&peer) {
                        chat.paint(&mut ctx, &theme);
                    }
                }
                Role::Settings => {
                    if let Some(sv) = &self.settings_view {
                        sv.paint(&mut ctx, &theme);
                    }
                }
                Role::Gallery => {
                    if let Some(gv) = &self.gallery_view {
                        gv.paint(&mut ctx, &theme);
                    }
                }
            }
            buffer.present().unwrap();
        }
    }

    impl ApplicationHandler<AppEvent> for App {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            if self.main_id.is_some() {
                return;
            }
            let attrs = Window::default_attributes().with_title("Nexa Beep");
            let window = Rc::new(el.create_window(attrs).unwrap());
            window.set_ime_allowed(true); // 한글 타입어헤드 — IME 커밋 문자
            let scale = window.scale_factor() as f32;
            let context = softbuffer::Context::new(window.clone()).unwrap();
            let surface = SbSurface::new(&context, window.clone()).unwrap();
            let id = window.id();
            self.windows.insert(
                id,
                WinEntry {
                    role: Role::Main,
                    window,
                    surface,
                    cursor: (0, 0),
                    scale,
                },
            );
            self.main_id = Some(id);
            self.layout_window(id);
        }

        fn user_event(&mut self, el: &ActiveEventLoop, event: AppEvent) {
            // 세션 액터 → GUI(M2-7). 수신 메시지를 해당 대화 스레드에 실시간 반영한다.
            match event {
                AppEvent::Recv {
                    peer,
                    text,
                    seq,
                    sender,
                } => {
                    if !self.dedup.accept(sender, seq) {
                        return; // 중복(다중 경로 — FR-M-9)
                    }
                    let line = ChatLine { mine: false, text };
                    if let Some(conv) = self.conversations.get_mut(&peer) {
                        conv.lines.push(line.clone());
                    }
                    let mut inv = Invalidations::default();
                    if let Some(chat) = self.chats.get_mut(&peer) {
                        chat.push_line(line, &mut inv);
                    }
                    // 이 대화가 보이는 창을 다시 그린다.
                    self.redraw_conversation(peer);
                }
                AppEvent::Closed { peer } => {
                    self.conversations.remove(&peer);
                    self.status = "상대와의 세션이 종료됨".into();
                    if let Some(id) = self.main_id {
                        self.request_redraw(id);
                    }
                }
                AppEvent::Inbound { session } => {
                    use nbeep_core::Session as _;
                    let peer = session.session.peer();
                    if self.conversations.contains_key(&peer) {
                        return; // 이미 이 상대와 대화 중(아웃바운드 세션 존재) — 중복 인바운드 무시
                    }
                    // TOFU 판정(메인 스레드 — TrustStore가 여기 있다) → 다중화 → 대화 등록.
                    let est =
                        match nbeep_core::TrustedSession::wrap(session.session, &mut self.trust) {
                            Ok(est) => est,
                            Err(_) => return, // 차단 상대 등 — fail-closed
                        };
                    self.install_conversation(nbeep_core::MuxSession::new(est.session));
                    let title = self.peer_title(peer);
                    let mut inv = Invalidations::default();
                    self.refresh_rows(&mut inv);
                    match self.mode {
                        WindowMode::Separate => {
                            // 인바운드도 상대별 창을 연다(대칭 실시간 대화).
                            let chat = self.build_chat_view(peer);
                            self.open_separate_window(peer, chat, el);
                        }
                        WindowMode::Single => {
                            if self.single_open.is_none() {
                                // 목록 화면이면 새 대화를 바로 연다.
                                let chat = self.build_chat_view(peer);
                                self.chats.insert(peer, chat);
                                self.single_open = Some(peer);
                                if let Some(mid) = self.main_id {
                                    self.layout_window(mid);
                                }
                            } else {
                                // 다른 대화 중 — 뺏지 않는다. 백그라운드로 쌓이고 목록에서 열면 복원.
                                self.status = format!("새 대화 도착: {title} (목록에서 열기)");
                            }
                        }
                    }
                    if let Some(mid) = self.main_id {
                        self.request_redraw(mid);
                    }
                }
            }
        }

        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            // 종료 신호(SIGINT/SIGTERM) → 이벤트 루프 종료. run_app 반환 → App Drop →
            // transport(LocalDirect) Drop → GOODBYE 발신·소켓/스레드 정리(R-16 · FR-P-7).
            if self.shutdown.requested() {
                el.exit();
                return;
            }
            self.poll_discovery();
            // 오버레이 스크롤바 페이드 틱(~5Hz) — 상태 변화 시 갤러리 재그리기.
            if let Some(gv) = &mut self.gallery_view {
                if gv.tick() {
                    if let Some((gid, _)) =
                        self.windows.iter().find(|(_, e)| e.role == Role::Gallery)
                    {
                        let gid = *gid;
                        self.request_redraw(gid);
                    }
                }
            }
            // 유휴에도 ~5Hz로 깨어나 발견 갱신·종료 신호를 폴한다(입력 없을 때도 목록이 산다).
            el.set_control_flow(ControlFlow::wait_duration(
                std::time::Duration::from_millis(200),
            ));
        }

        fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => {
                    if Some(id) == self.main_id {
                        el.exit(); // 주 창 닫기 = 종료(트레이 상주는 M3-2)
                    } else if let Some(entry) = self.windows.remove(&id) {
                        match entry.role {
                            // 대화 창 닫기 = 뷰만 닫힘(대화 유지 — DR-26).
                            Role::Chat(peer) => {
                                self.chats.remove(&peer);
                            }
                            Role::Settings => self.settings_view = None,
                            Role::Gallery => self.gallery_view = None,
                            Role::Main => {}
                        }
                    }
                }
                WindowEvent::Resized(_) => {
                    self.layout_window(id);
                    self.request_redraw(id);
                }
                WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                    if let Some(e) = self.windows.get_mut(&id) {
                        e.scale = scale_factor as f32;
                    }
                    self.layout_window(id);
                    self.request_redraw(id);
                }
                WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                    // 조합 중 — 활성 대화 뷰에 프리에딧 반영(M3-3).
                    if let Some(peer) = self.chat_peer_for(id) {
                        let mut inv = Invalidations::default();
                        if let Some(chat) = self.chats.get_mut(&peer) {
                            chat.set_preedit(text, &mut inv);
                        }
                        self.request_redraw(id);
                    }
                }
                WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                    let now_ms = self.now_ms();
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.route(id, InputEvent::Char { c, now_ms }, el);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(e) = self.windows.get_mut(&id) {
                        e.cursor = (position.x as i32, position.y as i32);
                        let (x, y) = e.cursor;
                        self.route(id, InputEvent::MouseMove { x, y }, el);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let Some(e) = self.windows.get(&id) {
                        let (x, y) = e.cursor;
                        // 주 창 하단 "컨트롤 갤러리" 버튼 먼저 판정(임시 검수 진입점).
                        if e.role == Role::Main {
                            let sz = e.window.inner_size();
                            let btn = Self::gallery_btn(sz.width as i32, sz.height as i32, e.scale);
                            if btn.contains(nbeep_ui::Point { x, y }) {
                                self.open_gallery(el);
                                return;
                            }
                        }
                        self.route(
                            id,
                            InputEvent::MouseDown {
                                x,
                                y,
                                shift: false,
                                primary: false,
                            },
                            el,
                        );
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let Some(e) = self.windows.get(&id) {
                        let (x, y) = e.cursor;
                        self.route(id, InputEvent::MouseUp { x, y }, el);
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let (dx, dy) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => {
                            ((x * 120.0) as i32, (y * 120.0) as i32)
                        }
                        MouseScrollDelta::PixelDelta(p) => {
                            ((p.x * 120.0 / 38.0) as i32, (p.y * 120.0 / 38.0) as i32)
                        }
                    };
                    if dy != 0 {
                        self.route(id, InputEvent::Wheel { delta: dy }, el);
                    }
                    if dx != 0 {
                        self.route(id, InputEvent::HWheel { delta: dx }, el);
                    }
                }
                WindowEvent::ModifiersChanged(mods) => {
                    let st = mods.state();
                    self.primary_down = if cfg!(target_os = "macos") {
                        st.super_key()
                    } else {
                        st.control_key()
                    };
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state != ElementState::Pressed {
                        return;
                    }
                    // Cmd/Ctrl+, = 설정 · Cmd/Ctrl+K = 수동 엔드포인트 추가(DR-19).
                    if self.primary_down {
                        if let WKey::Character(t) = &event.logical_key {
                            match t.as_str() {
                                "," => {
                                    self.open_settings(el);
                                    return;
                                }
                                "g" | "G" => {
                                    self.open_gallery(el);
                                    return;
                                }
                                "k" | "K" => {
                                    self.adding = Some(String::new());
                                    self.status =
                                        "주소 입력(host:port) · Enter 연결 · Esc 취소: ".into();
                                    if let Some(mid) = self.main_id {
                                        self.request_redraw(mid);
                                    }
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }
                    // 수동 입력 모드 — 문자/Enter/Esc를 목록이 아니라 주소 버퍼로.
                    if self.adding.is_some() {
                        match &event.logical_key {
                            WKey::Named(NamedKey::Enter) => {
                                let addr = self.adding.take().unwrap_or_default();
                                self.commit_manual_add(addr, el);
                            }
                            WKey::Named(NamedKey::Escape) => {
                                self.adding = None;
                                self.status = "수동 추가 취소".into();
                                if let Some(mid) = self.main_id {
                                    self.request_redraw(mid);
                                }
                            }
                            WKey::Named(NamedKey::Backspace) => {
                                if let Some(buf) = self.adding.as_mut() {
                                    buf.pop();
                                }
                                if let Some(mid) = self.main_id {
                                    self.request_redraw(mid);
                                }
                            }
                            WKey::Character(t) => {
                                if let Some(buf) = self.adding.as_mut() {
                                    buf.push_str(t);
                                }
                                if let Some(mid) = self.main_id {
                                    self.request_redraw(mid);
                                }
                            }
                            _ => {}
                        }
                        return;
                    }
                    let key = match &event.logical_key {
                        WKey::Named(NamedKey::ArrowUp) => Some(Key::Up),
                        WKey::Named(NamedKey::ArrowDown) => Some(Key::Down),
                        WKey::Named(NamedKey::PageUp) => Some(Key::PageUp),
                        WKey::Named(NamedKey::PageDown) => Some(Key::PageDown),
                        WKey::Named(NamedKey::Home) => Some(Key::Home),
                        WKey::Named(NamedKey::End) => Some(Key::End),
                        WKey::Named(NamedKey::Enter) => Some(Key::Enter),
                        WKey::Named(NamedKey::Escape) => Some(Key::Escape),
                        _ => None,
                    };
                    if let Some(key) = key {
                        self.route(
                            id,
                            InputEvent::Key {
                                key,
                                shift: false,
                                primary: false,
                            },
                            el,
                        );
                        return;
                    }
                    if let WKey::Named(NamedKey::Backspace) = &event.logical_key {
                        let now_ms = self.now_ms();
                        self.route(id, InputEvent::Char { c: '\u{8}', now_ms }, el);
                        return;
                    }
                    if let WKey::Character(text) = &event.logical_key {
                        let now_ms = self.now_ms();
                        if let Some(c) = text.chars().next() {
                            if !c.is_control() {
                                self.route(id, InputEvent::Char { c, now_ms }, el);
                            }
                        }
                    }
                }
                WindowEvent::RedrawRequested => self.redraw(id),
                _ => {}
            }
        }
    }

    /// 창을 띄우고 이벤트 루프를 돈다(주 창을 닫으면 종료).
    /// 창을 띄운다. `live=true`면 실물 발견(`LocalDirect`), 아니면 InMemory 데모(에코 봇).
    pub(crate) fn run(mode: WindowMode, live: bool) {
        let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트 없음");
        let font = nbeep_gfx::Font::from_static(data, index).expect("폰트 파싱");
        let identity = std::sync::Arc::new(nbeep_crypto::Identity::generate());
        use nbeep_net::Transport as _;

        // 이벤트 루프·프록시 먼저 — 인바운드 수락 펌프가 프록시를 필요로 한다(M2-7).
        let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
        let proxy = event_loop.create_proxy();
        let shutdown = nbeep_plat::shutdown::install(); // R-16 — SIGINT/SIGTERM 포트

        let (transport, discovery): (Box<dyn nbeep_net::Transport>, _) = if live {
            // 실물 — LocalDirect(UDP 발견 + TCP 세션). 실기·컨테이너 상대가 목록에 뜬다.
            let mut instance = [0u8; 16];
            instance
                .copy_from_slice(&nbeep_crypto::Identity::generate().peer_id().as_bytes()[..16]);
            let name =
                nbeep_core::DisplayName::parse(&format!("나-{}", identity.peer_id().short()))
                    .expect("라벨");
            let local = nbeep_net::LocalDirect::spawn(identity.peer_id(), instance, name, 800, 1)
                .expect("LocalDirect 시작(방화벽·인터페이스)");
            let discovery = local.discovery();
            // 인바운드 수락 펌프 — 남이 나에게 연결하면 accept+에코(대칭 대화·비동기 GUI 펌프는 M2-7).
            spawn_inbound_accept(
                local.incoming(),
                std::sync::Arc::clone(&identity),
                proxy.clone(),
            );
            (Box::new(local), discovery)
        } else {
            // 데모 — InMemory 버스 + 에코 봇 3명.
            let bus = nbeep_net::inmem::InMemoryBus::new();
            for name in ["김철수의 MacBook", "이영희 (개발2팀)", "bob-linux"] {
                spawn_echo_bot(&bus, name);
            }
            let transport = bus.join(
                identity.peer_id(),
                nbeep_core::DisplayName::parse("나").unwrap(),
                nbeep_net::Caps::default(),
            );
            let discovery = transport.discovery();
            (Box::new(transport), discovery)
        };

        let mut settings = SettingsState::with_defaults();
        if mode == WindowMode::Separate {
            settings.set("chat.window_mode", "separate".into());
        }
        // 현재 언어를 설정값으로 초기화(기본 en — i18n).
        nbeep_core::set_lang(
            nbeep_core::Lang::from_code(settings.get("ui.language")).unwrap_or_default(),
        );
        let net_hint = if live {
            "실물 발견(LAN)"
        } else {
            "데모(에코 봇)"
        };
        let mode_hint = match mode {
            WindowMode::Single => "Enter = 대화 열기",
            WindowMode::Separate => "Enter = 상대별 새 창(동시 대화)",
        };
        let mut app = App {
            mode,
            windows: HashMap::new(),
            main_id: None,
            font,
            theme: Theme::dark(),
            list: PeerListWidget::new(),
            chats: HashMap::new(),
            single_open: None,
            identity,
            seq: nbeep_core::Sequencer::new(),
            transport,
            discovery,
            table: nbeep_core::PeerTable::new(60_000),
            trust: nbeep_core::MemoryTrustStore::new(),
            conversations: HashMap::new(),
            dedup: nbeep_core::DedupIndex::new(),
            started: Instant::now(),
            status: format!(
                "[{net_hint}] {mode_hint} · ⌘/Ctrl+K = 주소 추가 · ⌘/Ctrl+, = 설정 · ⌘/Ctrl+G = 컨트롤 갤러리"
            ),
            fonts: App::fonts_from_settings(&settings),
            settings,
            settings_view: None,
            gallery_view: None,
            primary_down: false,
            proxy,
            adding: None,
            shutdown,
        };
        event_loop.run_app(&mut app).unwrap();
    }
}
