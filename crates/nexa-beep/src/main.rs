//! Nexa Beep 본체 — 진입·조립·생명주기.
//!
//! `--window` = **피어 목록 위젯 데모**(M3-1 — PeerTable+TrustStore 실물 도메인 경로 위에
//! `PeerListWidget` + `RasterCtx`). 캐럿 탐색·클릭·휠·타입어헤드·Enter 활성화가 동작한다.
//! 실물 발견 배선은 M1-4, 창 코드의 `nbeep-plat` 이관은 M3-2.
//! 기본 실행은 스캐폴드 출력(헤드리스 CI 안전).

// 조립 지점 바이너리 — 창 초기화 경로의 unwrap 허용(docs/13 §9 — 복구 불가 구성 오류).
#![allow(clippy::unwrap_used)]

fn main() {
    let open_window = std::env::args().any(|a| a == "--window");
    if open_window {
        app_window::run();
    } else {
        println!(
            "nexa-beep {} — scaffold (창은 `--window`)",
            env!("CARGO_PKG_VERSION")
        );
    }
}

/// 창 + 위젯 조립 — winit 이벤트를 `InputEvent`로 번역해 위젯에 라우팅.
mod app_window {
    use std::num::NonZeroU32;
    use std::rc::Rc;
    use std::time::Instant;
    use winit::application::ApplicationHandler;
    use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::keyboard::{Key as WKey, NamedKey};
    use winit::window::{Window, WindowId};

    use nbeep_ui::{
        ChatLine, ChatViewWidget, DrawCtx, InputEvent, Invalidations, Key, PeerListWidget, PeerRow,
        RasterCtx, Rect, Theme, Widget,
    };

    type SbSurface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

    struct App {
        window: Option<Rc<Window>>,
        surface: Option<SbSurface>,
        font: nbeep_gfx::Font,
        theme: Theme,
        list: PeerListWidget,
        /// 열린 대화(None = 목록 화면).
        chat: Option<ChatViewWidget>,
        /// 내 신원 — 발견·세션·발신 봉투가 전부 이 키 하나에서 나온다.
        identity: std::sync::Arc<nbeep_crypto::Identity>,
        seq: nbeep_core::Sequencer,
        /// InMemory 전송(실물 발견 이벤트 공급원 — M1-4에서 실물 네트워크로 교체).
        transport: nbeep_net::inmem::InMemoryTransport,
        discovery: std::sync::mpsc::Receiver<nbeep_net::DiscoveryEvent>,
        table: nbeep_core::PeerTable,
        trust: nbeep_core::MemoryTrustStore,
        /// 열린 대화의 실물 세션(Noise+TOFU+다중화).
        session: Option<LiveSession>,
        dedup: nbeep_core::DedupIndex,
        cursor: (i32, i32),
        started: Instant,
        /// 창 배율(고DPI — FR-U-6). 모니터 이동·설정 변경 시 갱신.
        scale: f32,
        /// 하단 상태바 문구(Enter 활성화 피드백 — 대화 열기는 M2-7).
        status: String,
    }

    /// 열린 대화의 실물 세션 스택 — Noise(암호화) 위 TOFU(신뢰) 위 다중화.
    type LiveSession = nbeep_core::MuxSession<
        nbeep_core::TrustedSession<nbeep_crypto::NoiseSession<Box<dyn nbeep_core::Link>>>,
    >;

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

    impl App {
        fn route(&mut self, ev: InputEvent) {
            let mut inv = Invalidations::default();
            if let Some(chat) = &mut self.chat {
                chat.on_event(&ev, &mut inv);
                if let Some(text) = chat.take_outgoing() {
                    // 실물 발신 — 봉투 구성 → 암호화 세션으로 전송 → 에코 수신(즉시).
                    let msg = nbeep_core::ChatMessage {
                        sender_device: self.identity.peer_id(),
                        seq: self.seq.issue(),
                        body: nbeep_core::MessageBody::Text(text.as_str().to_string()),
                    };
                    chat.push_line(ChatLine { mine: true, text }, &mut inv);
                    if let Some(session) = &mut self.session {
                        use nbeep_core::mux::StreamId;
                        let peer = session.peer();
                        let sent = session.send(StreamId::Chat, &msg.encode());
                        // 데모: 에코 봇이 즉시 응답(인프로세스) — 블로킹 수신.
                        // 실물 네트워크의 비동기 수신 펌프는 M2-7에서.
                        let reply = sent.and_then(|()| session.recv(StreamId::Chat));
                        match reply {
                            Ok(bytes) => {
                                if let Ok(rmsg) = nbeep_core::ChatMessage::decode(&bytes, peer) {
                                    if self.dedup.accept(rmsg.sender_device, rmsg.seq) {
                                        if let nbeep_core::MessageBody::Text(t) = rmsg.body {
                                            chat.push_line(
                                                ChatLine {
                                                    mine: false,
                                                    text: nbeep_core::sanitize_message(&t),
                                                },
                                                &mut inv,
                                            );
                                        }
                                    }
                                }
                                self.status = format!("암호화 왕복 OK — seq={}", msg.seq);
                            }
                            Err(e) => self.status = format!("세션 오류: {e}"),
                        }
                    }
                }
                if chat.take_back() {
                    self.chat = None;
                    self.session = None; // 세션 드롭 = 링크 종료(봇은 다음 수신 대기로)
                    self.status =
                        "↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기".into();
                    inv.push(self.list.bounds());
                }
            } else {
                self.list.on_event(&ev, &mut inv);
                if let Some(peer) = self.list.take_activated() {
                    match self.open_session(peer) {
                        Ok((session, decision)) => {
                            let title = self.table.get(peer).map_or_else(
                                || format!("{peer:?}"),
                                |e| e.name.as_str().to_string(),
                            );
                            let mut chat = ChatViewWidget::new(title);
                            chat.set_scale(self.scale, &mut inv);
                            chat.set_bounds(self.list.bounds(), &mut inv);
                            self.chat = Some(chat);
                            self.session = Some(session);
                            self.status = match decision {
                                nbeep_core::TrustDecision::FirstContact => {
                                    "Noise 세션 수립 — 첫 접촉(TOFU 핀 고정) · Esc = 목록".into()
                                }
                                d => format!("Noise 세션 수립 — {d:?} · Esc = 목록"),
                            };
                            self.refresh_rows(&mut inv); // 배지 갱신(핀 반영)
                        }
                        Err(e) => self.status = format!("연결 실패: {e}"),
                    }
                }
            }
            if !inv.is_empty() {
                if let Some(w) = &self.window {
                    w.request_redraw();
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
                if let Some(w) = &self.window {
                    w.request_redraw();
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
            peer: nbeep_core::PeerId,
        ) -> Result<(LiveSession, nbeep_core::TrustDecision), String> {
            use nbeep_net::Transport as _;
            let link = self.transport.connect(peer).map_err(|e| format!("{e:?}"))?;
            let noise = nbeep_crypto::NoiseSession::initiate(link, &self.identity)
                .map_err(|e| e.to_string())?;
            let est = nbeep_core::TrustedSession::wrap(noise, &mut self.trust)
                .map_err(|e| e.to_string())?;
            Ok((nbeep_core::MuxSession::new(est.session), est.decision))
        }

        fn bar_h(&self) -> i32 {
            (26.0 * self.scale).round() as i32
        }

        fn relayout(&mut self, w: u32, h: u32) {
            let mut inv = Invalidations::default();
            let width = i32::try_from(w).unwrap_or(i32::MAX);
            let height = i32::try_from(h).unwrap_or(i32::MAX) - self.bar_h();
            self.list.set_scale(self.scale, &mut inv);
            self.list
                .set_bounds(Rect::new(0, 0, width, height.max(0)), &mut inv);
        }

        fn now_ms(&self) -> u64 {
            u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
        }
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let attrs = Window::default_attributes().with_title("Nexa Beep");
            let window = Rc::new(el.create_window(attrs).unwrap());
            window.set_ime_allowed(true); // 한글 타입어헤드 — IME 커밋 문자를 받는다(조합 UI는 M3-3)
            self.scale = window.scale_factor() as f32;
            let size = window.inner_size();
            let context = softbuffer::Context::new(window.clone()).unwrap();
            let surface = SbSurface::new(&context, window.clone()).unwrap();
            self.window = Some(window);
            self.surface = Some(surface);
            self.relayout(size.width, size.height);
        }

        fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
            // 발견 이벤트 폴링 — 봇은 시작 시 join하므로 첫 배치에서 다 접힌다.
            // 실물 네트워크의 상시 깨우기(EventLoopProxy)는 M2-7에서.
            self.poll_discovery();
        }

        fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => el.exit(),
                WindowEvent::Resized(size) => {
                    self.relayout(size.width, size.height);
                    if let Some(win) = &self.window {
                        win.request_redraw();
                    }
                }
                WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                    // 모니터 이동·배율 변경(FR-U-6) — 레이아웃 전체 재계산.
                    self.scale = scale_factor as f32;
                    if let Some(win) = self.window.clone() {
                        let size = win.inner_size();
                        self.relayout(size.width, size.height);
                        win.request_redraw();
                    }
                }
                WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                    // IME가 확정한 문자열(한글 등) — 타입어헤드로 라우팅.
                    let now_ms = self.now_ms();
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.route(InputEvent::Char { c, now_ms });
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.cursor = (position.x as i32, position.y as i32);
                    let (x, y) = self.cursor;
                    self.route(InputEvent::MouseMove { x, y });
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    let (x, y) = self.cursor;
                    self.route(InputEvent::MouseDown {
                        x,
                        y,
                        shift: false,
                        primary: false,
                    });
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let d = match delta {
                        MouseScrollDelta::LineDelta(_, y) => (y * 120.0) as i32,
                        MouseScrollDelta::PixelDelta(p) => (p.y * 120.0 / 38.0) as i32,
                    };
                    if d != 0 {
                        self.route(InputEvent::Wheel { delta: d });
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state != ElementState::Pressed {
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
                        self.route(InputEvent::Key {
                            key,
                            shift: false,
                            primary: false,
                        });
                        return;
                    }
                    // 타입어헤드 — 인쇄 가능 문자·Backspace(IME 조합은 M3-3).
                    if let WKey::Named(NamedKey::Backspace) = &event.logical_key {
                        let now_ms = self.now_ms();
                        self.route(InputEvent::Char { c: '\u{8}', now_ms });
                        return;
                    }
                    if let WKey::Character(text) = &event.logical_key {
                        let now_ms = self.now_ms();
                        if let Some(c) = text.chars().next() {
                            if !c.is_control() {
                                self.route(InputEvent::Char { c, now_ms });
                            }
                        }
                    }
                }
                WindowEvent::RedrawRequested => {
                    let bar_h = self.bar_h();
                    let pad = (8.0 * self.scale).round() as i32;
                    let text_dy = (bar_h - (14.0 * self.scale) as i32) / 2;
                    let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
                        return;
                    };
                    let size = window.inner_size();
                    if let (Some(w), Some(h)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                    {
                        surface.resize(w, h).unwrap();
                        let mut buffer = surface.buffer_mut().unwrap();
                        let mut px = nbeep_gfx::Surface::new(
                            &mut buffer,
                            size.width as usize,
                            size.height as usize,
                        );
                        px.fill(self.theme.window_bg);
                        let mut ctx = RasterCtx::new(&mut px, &self.font).with_scale(self.scale);
                        if let Some(chat) = &self.chat {
                            chat.paint(&mut ctx, &self.theme);
                        } else {
                            self.list.paint(&mut ctx, &self.theme);
                        }
                        // 하단 상태바 — Enter/타입어헤드 동작이 눈에 보이게.
                        let h = i32::try_from(size.height).unwrap_or(i32::MAX);
                        let w = i32::try_from(size.width).unwrap_or(i32::MAX);
                        let bar = Rect::new(0, h - bar_h, w, bar_h);
                        ctx.select_font(nbeep_ui::FontSlot::Status, false);
                        ctx.text_opaque(
                            bar.x + pad,
                            bar.y + text_dy,
                            bar,
                            &self.status,
                            self.theme.text_dim,
                            self.theme.chrome_bg,
                        );
                        buffer.present().unwrap();
                    }
                }
                _ => {}
            }
        }
    }

    /// 창을 띄우고 이벤트 루프를 돈다(닫으면 종료).
    pub(crate) fn run() {
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
        let list = PeerListWidget::new();

        let event_loop = EventLoop::new().unwrap();
        let mut app = App {
            window: None,
            surface: None,
            font,
            theme: Theme::dark(),
            list,
            chat: None,
            identity,
            seq: nbeep_core::Sequencer::new(),
            transport,
            discovery,
            table: nbeep_core::PeerTable::new(60_000),
            trust: nbeep_core::MemoryTrustStore::new(),
            session: None,
            dedup: nbeep_core::DedupIndex::new(),
            cursor: (0, 0),
            started: Instant::now(),
            scale: 1.0,
            status: "↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기".into(),
        };
        event_loop.run_app(&mut app).unwrap();
    }
}
