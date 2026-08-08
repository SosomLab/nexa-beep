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
        /// 열린 대화(None = 목록 화면). 스레드·세션 배선은 M2-7 — 지금은 발신 도메인 경로만.
        chat: Option<ChatViewWidget>,
        /// 내 기기 신원(발신 봉투 sender_device) — 실물 키(Noise와 동일 경로).
        me: nbeep_core::PeerId,
        seq: nbeep_core::Sequencer,
        cursor: (i32, i32),
        started: Instant,
        /// 창 배율(고DPI — FR-U-6). 모니터 이동·설정 변경 시 갱신.
        scale: f32,
        /// 하단 상태바 문구(Enter 활성화 피드백 — 대화 열기는 M2-7).
        status: String,
    }

    /// 데모 목록 — **실물 도메인 경로**(PeerTable 관측→병합→TrustStore 판정)로 만든다.
    /// 실물 발견(M1-4)은 이 관측 공급원만 교체한다.
    fn demo_rows() -> Vec<PeerRow> {
        use nbeep_core::{
            DisplayName, MemoryTrustStore, MonoInstant, PeerId, PeerTable, SourceId, TrustStore,
        };
        let pid = |b: u8| {
            let mut a = [0u8; 32];
            a[0] = b;
            PeerId::from_bytes(a)
        };
        let name = |s: &str| DisplayName::parse(s).unwrap();
        let mut table = PeerTable::new(10_000);
        let mut trust = MemoryTrustStore::new();
        let now = MonoInstant(0);

        table.observe(pid(1), name("김철수의 MacBook"), SourceId(0), now);
        table.observe(pid(1), name("김철수의 MacBook"), SourceId(1), now); // 다중 경로 병합
        table.observe(pid(2), name("DESKTOP-A7X3"), SourceId(0), now);
        table.observe(pid(3), name("이영희 (개발2팀)"), SourceId(0), now);
        table.observe(pid(4), name("박민준"), SourceId(0), now);
        table.observe(pid(5), name("bob-linux"), SourceId(0), now);
        trust.on_session(pid(1)); // 핀
        trust.on_session(pid(3));
        trust.verify(pid(3)); // SAS 대조 완료
        trust.on_session(pid(4));

        table
            .list()
            .into_iter()
            .map(|entry| {
                let t = trust.level(entry.peer);
                PeerRow { entry, trust: t }
            })
            .collect()
    }

    impl App {
        fn route(&mut self, ev: InputEvent) {
            let mut inv = Invalidations::default();
            if let Some(chat) = &mut self.chat {
                chat.on_event(&ev, &mut inv);
                if let Some(text) = chat.take_outgoing() {
                    // 발신 도메인 경로 — 시퀀서가 seq를 발급하고 봉투를 만든다.
                    // 전송(fanout)은 실물 세션 배선(M1-4·M2-7)에서 — 지금은 스레드 확정만.
                    let msg = nbeep_core::ChatMessage {
                        sender_device: self.me,
                        seq: self.seq.issue(),
                        body: nbeep_core::MessageBody::Text(text.as_str().to_string()),
                    };
                    self.status = format!("발신 봉투 seq={} — 전송 배선은 M2-7", msg.seq);
                    chat.push_line(ChatLine { mine: true, text }, &mut inv);
                }
                if chat.take_back() {
                    self.chat = None;
                    self.status =
                        "↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기".into();
                    inv.push(self.list.bounds());
                }
            } else {
                self.list.on_event(&ev, &mut inv);
                if let Some(peer) = self.list.take_activated() {
                    // 목록 → 대화 전환. 상대 이름은 데모 행에서 찾는다.
                    let title = demo_rows()
                        .into_iter()
                        .find(|r| r.entry.peer == peer)
                        .map_or_else(
                            || format!("{peer:?}"),
                            |r| r.entry.name.as_str().to_string(),
                        );
                    let mut chat = ChatViewWidget::new(title);
                    chat.set_scale(self.scale, &mut inv);
                    chat.set_bounds(self.list.bounds(), &mut inv);
                    self.chat = Some(chat);
                    self.status = "메시지 입력 · Enter 전송 · Esc = 목록".into();
                }
            }
            if !inv.is_empty() {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
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
        let mut list = PeerListWidget::new();
        let mut inv = Invalidations::default();
        list.set_rows(demo_rows(), &mut inv);

        let event_loop = EventLoop::new().unwrap();
        let me = nbeep_crypto::Identity::generate().peer_id(); // 실물 키 경로(발신 봉투용)
        let mut app = App {
            window: None,
            surface: None,
            font,
            theme: Theme::dark(),
            list,
            chat: None,
            me,
            seq: nbeep_core::Sequencer::new(),
            cursor: (0, 0),
            started: Instant::now(),
            scale: 1.0,
            status: "↑↓ 이동 · 타이핑 = 이름 점프(한글 가능) · Enter = 대화 열기".into(),
        };
        event_loop.run_app(&mut app).unwrap();
    }
}
