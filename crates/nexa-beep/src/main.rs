//! Nexa Beep 본체 — 진입·조립·생명주기.
//!
//! `--window` = **InMemory 종단 데모**: 실물 발견(에코 봇) → 목록 → Enter →
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
    let open_window = args.iter().any(|a| a == "--window");
    let separate = args.iter().any(|a| a == "--separate-windows");
    if open_window || separate {
        app_window::run(if separate {
            app_window::WindowMode::Separate
        } else {
            app_window::WindowMode::Single
        });
    } else {
        println!(
            "nexa-beep {} — scaffold (창 `--window`/`--separate-windows` · 발견 `--discover-probe [초]` · 실물 종단 `--live-echo [초]`)",
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
    while std::time::Instant::now() < deadline {
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
    while std::time::Instant::now() < deadline {
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
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::keyboard::{Key as WKey, NamedKey};
    use winit::window::{Window, WindowId};

    use nbeep_core::PeerId;
    use nbeep_ui::{
        ChatLine, ChatViewWidget, DrawCtx, InputEvent, Invalidations, Key, PeerListWidget, PeerRow,
        RasterCtx, Rect, SettingsState, SettingsWidget, Theme, Widget,
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

    /// 대화 상태 — **뷰(창)와 분리**된 상대별 세션+스레드(DR-26).
    ///
    /// 뷰를 닫아도 대화는 살아 있다 — 세션 유지·스레드 보존·재진입 시 복원(재핸드셰이크 없음).
    /// 창 모드는 뷰 계층만의 문제다: 어느 모드든 이 구조는 그대로고 뷰가 몇 개 붙느냐만 다르다.
    struct Conversation {
        session: LiveSession,
        lines: Vec<ChatLine>,
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
        /// InMemory 전송(실물 발견 이벤트 공급원 — M1-4에서 실물 네트워크로 교체).
        transport: nbeep_net::inmem::InMemoryTransport,
        discovery: std::sync::mpsc::Receiver<nbeep_net::DiscoveryEvent>,
        table: nbeep_core::PeerTable,
        trust: nbeep_core::MemoryTrustStore,
        /// 상대별 대화 상태 — 뷰와 무관하게 유지(동시 대화의 실체).
        conversations: HashMap<PeerId, Conversation>,
        dedup: nbeep_core::DedupIndex,
        started: Instant,
        /// 주 창 하단 상태바 문구.
        status: String,
        /// 설정 값(런타임 — 영속은 M2-5). `chat.window_mode`·`ui.theme`.
        settings: SettingsState,
        /// 열린 설정 창의 뷰(설정 창은 항상 별도 OS 창 1개).
        settings_view: Option<SettingsWidget>,
        /// OS 주 수식키(⌘/Ctrl) 눌림 상태 — `Cmd/Ctrl+,` 판정.
        primary_down: bool,
    }

    impl App {
        fn now_ms(&self) -> u64 {
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
        }

        fn bar_h(scale: f32) -> i32 {
            (26.0 * scale).round() as i32
        }

        fn request_redraw(&self, id: WindowId) {
            if let Some(e) = self.windows.get(&id) {
                e.window.request_redraw();
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
            use nbeep_net::Transport as _;
            let link = self.transport.connect(peer).map_err(|e| format!("{e:?}"))?;
            let noise = nbeep_crypto::NoiseSession::initiate(link, &self.identity)
                .map_err(|e| e.to_string())?;
            let est = nbeep_core::TrustedSession::wrap(noise, &mut self.trust)
                .map_err(|e| e.to_string())?;
            Ok((nbeep_core::MuxSession::new(est.session), est.decision))
        }

        fn peer_title(&self, peer: PeerId) -> String {
            self.table
                .get(peer)
                .map_or_else(|| format!("{peer:?}"), |e| e.name.as_str().to_string())
        }

        /// 대화 상태 확보(없으면 세션 수립) + 뷰 생성(스레드 복원).
        fn ensure_conversation(&mut self, peer: PeerId) -> Result<ChatViewWidget, String> {
            if !self.conversations.contains_key(&peer) {
                let (session, decision) = self.open_session(peer)?;
                self.conversations.insert(
                    peer,
                    Conversation {
                        session,
                        lines: Vec::new(),
                    },
                );
                self.status = match decision {
                    nbeep_core::TrustDecision::FirstContact => {
                        "Noise 세션 수립 — 첫 접촉(TOFU 핀 고정)".into()
                    }
                    d => format!("Noise 세션 수립 — {d:?}"),
                };
            } else {
                self.status = "대화 복원 — 세션 유지 중".into();
            }
            let mut chat = ChatViewWidget::new(self.peer_title(peer));
            let mut inv = Invalidations::default();
            if let Some(conv) = self.conversations.get(&peer) {
                for line in &conv.lines {
                    chat.push_line(line.clone(), &mut inv);
                }
            }
            Ok(chat)
        }

        /// 설정 창을 연다(있으면 포커스) — `Cmd/Ctrl+,`.
        fn open_settings(&mut self, el: &ActiveEventLoop) {
            if let Some((id, _)) = self.windows.iter().find(|(_, e)| e.role == Role::Settings) {
                if let Some(e) = self.windows.get(id) {
                    e.window.focus_window();
                }
                return;
            }
            let attrs = Window::default_attributes().with_title("Nexa Beep — 설정");
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

        /// 설정 변경 즉시 적용(DR-24 — 저장 버튼 없음).
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
                WindowMode::Separate => {
                    // 같은 상대 재활성화 = 기존 창 포커스(14 §11).
                    let existing = self
                        .windows
                        .iter()
                        .find(|(_, e)| e.role == Role::Chat(peer))
                        .map(|(id, _)| *id);
                    if let Some(id) = existing {
                        if let Some(e) = self.windows.get(&id) {
                            e.window.focus_window();
                        }
                        return;
                    }
                    match self.ensure_conversation(peer) {
                        Ok(chat) => {
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
                        Err(e) => {
                            self.status = format!("연결 실패: {e}");
                            if let Some(mid) = self.main_id {
                                self.request_redraw(mid);
                            }
                        }
                    }
                }
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
                    use nbeep_core::mux::StreamId;
                    let session = &mut conv.session;
                    let authenticated = session.peer();
                    let sent = session.send(StreamId::Chat, &msg.encode());
                    // 데모: 에코 봇이 즉시 응답(인프로세스) — 블로킹 수신.
                    // 실물 네트워크의 비동기 수신 펌프는 M2-7에서.
                    let reply = sent.and_then(|()| session.recv(StreamId::Chat));
                    match reply {
                        Ok(bytes) => {
                            if let Ok(rmsg) = nbeep_core::ChatMessage::decode(&bytes, authenticated)
                            {
                                if self.dedup.accept(rmsg.sender_device, rmsg.seq) {
                                    if let nbeep_core::MessageBody::Text(t) = rmsg.body {
                                        let line = ChatLine {
                                            mine: false,
                                            text: nbeep_core::sanitize_message(&t),
                                        };
                                        conv.lines.push(line.clone());
                                        if let Some(chat) = self.chats.get_mut(&peer) {
                                            chat.push_line(line, &mut inv);
                                        }
                                    }
                                }
                            }
                            self.status = format!("암호화 왕복 OK — seq={}", msg.seq);
                        }
                        Err(e) => self.status = format!("세션 오류: {e}"),
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
            let mut ctx = RasterCtx::new(&mut px, &self.font).with_scale(entry.scale);
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
                    ctx.text_opaque(
                        bar.x + pad,
                        bar.y + dy,
                        bar,
                        &self.status,
                        theme.text_dim,
                        theme.chrome_bg,
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
            }
            buffer.present().unwrap();
        }
    }

    impl ApplicationHandler for App {
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

        fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
            // 발견 이벤트 폴링 — 봇은 시작 시 join하므로 첫 배치에서 다 접힌다.
            // 실물 네트워크의 상시 깨우기(EventLoopProxy)는 M2-7에서.
            self.poll_discovery();
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
                WindowEvent::MouseWheel { delta, .. } => {
                    let d = match delta {
                        MouseScrollDelta::LineDelta(_, y) => (y * 120.0) as i32,
                        MouseScrollDelta::PixelDelta(p) => (p.y * 120.0 / 38.0) as i32,
                    };
                    if d != 0 {
                        self.route(id, InputEvent::Wheel { delta: d }, el);
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
                    // Cmd/Ctrl+, = 설정(DR-24 · VS Code/macOS 표준).
                    if self.primary_down {
                        if let WKey::Character(t) = &event.logical_key {
                            if t.as_str() == "," {
                                self.open_settings(el);
                                return;
                            }
                        }
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
    pub(crate) fn run(mode: WindowMode) {
        let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트 없음");
        let font = nbeep_gfx::Font::from_static(data, index).expect("폰트 파싱");
        let identity = std::sync::Arc::new(nbeep_crypto::Identity::generate());
        let bus = nbeep_net::inmem::InMemoryBus::new();
        // 에코 봇들 — 발견·세션·에코까지 실물 스택으로 참여한다.
        for name in ["김철수의 MacBook", "이영희 (개발2팀)", "bob-linux"] {
            spawn_echo_bot(&bus, name);
        }
        use nbeep_net::Transport as _;
        let transport = bus.join(
            identity.peer_id(),
            nbeep_core::DisplayName::parse("나").unwrap(),
            nbeep_net::Caps::default(),
        );
        let discovery = transport.discovery();

        let mut settings = SettingsState::with_defaults();
        if mode == WindowMode::Separate {
            settings.set("chat.window_mode", "separate".into());
        }
        let event_loop = EventLoop::new().unwrap();
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
            status: format!("↑↓ 이동 · 타이핑 = 이름 점프 · {mode_hint} · ⌘/Ctrl+, = 설정"),
            settings,
            settings_view: None,
            primary_down: false,
        };
        event_loop.run_app(&mut app).unwrap();
    }
}
