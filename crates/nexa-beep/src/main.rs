//! Nexa Beep 본체 — 진입·조립·생명주기.
//!
//! ⚠️ **M0-2 SP-1 스파이크 상태**([docs/07] §6 · [docs/15] "버릴 수 있는 실험").
//! 지금은 **빈 창 + 픽셀 버퍼 표시**만으로 예산(R-8)을 실측한다 — winit(창·입력·IME) +
//! softbuffer(픽셀 버퍼 present)는 ADR-0001 **P2**(얇은 창 라이브러리)의 후보다.
//! 실제 발견/대화 배선은 M1+, 창 코드의 `nbeep-plat` 이관은 M3.
//!
//! `--window` 인자로 창을 띄운다(기본은 스캐폴드 출력 — 헤드리스 CI 안전).
//! M1-6: 창 = **데모 피어 목록**(PeerTable+TrustStore 실물 도메인 경로 · 미검증 배지).

// SP-1 스파이크 바이너리 — 창 초기화 경로의 unwrap 허용(버릴 수 있는 실험).
#![allow(clippy::unwrap_used)]

fn main() {
    let open_window = std::env::args().any(|a| a == "--window");
    if open_window {
        spike_window::run();
    } else {
        println!(
            "nexa-beep {} — scaffold (창은 `--window`, SP-1 예산 실측용)",
            env!("CARGO_PKG_VERSION")
        );
    }
}

/// SP-1 빈 창 — winit + softbuffer로 창 1개 + 어두운 배경 픽셀 버퍼 present.
mod spike_window {
    use std::num::NonZeroU32;
    use std::rc::Rc;
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::window::{Window, WindowId};

    type Surface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

    struct App {
        window: Option<Rc<Window>>,
        surface: Option<Surface>,
        font: nbeep_gfx::Font,
        rows: Vec<nbeep_ui::PeerRow>,
    }

    /// 데모 목록 — PeerTable·TrustStore **실물 도메인 경로**로 만든다(그리기만 데모가 아니라
    /// "발견 관측 → 목록 → 신뢰 배지"의 전체 조립을 검증). 실물 발견 배선은 M1-4.
    fn demo_rows() -> Vec<nbeep_ui::PeerRow> {
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
        trust.on_session(pid(1)); // 핀
        trust.on_session(pid(3));
        trust.verify(pid(3)); // SAS 대조 완료

        table
            .list()
            .into_iter()
            .map(|entry| {
                let t = trust.level(entry.peer);
                nbeep_ui::PeerRow { entry, trust: t }
            })
            .collect()
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let attrs = Window::default_attributes().with_title("Nexa Beep");
            let window = Rc::new(el.create_window(attrs).unwrap());
            let context = softbuffer::Context::new(window.clone()).unwrap();
            let surface = Surface::new(&context, window.clone()).unwrap();
            self.window = Some(window);
            self.surface = Some(surface);
        }

        fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => el.exit(),
                WindowEvent::RedrawRequested => {
                    let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
                        return;
                    };
                    let size = window.inner_size();
                    if let (Some(w), Some(h)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                    {
                        surface.resize(w, h).unwrap();
                        let mut buffer = surface.buffer_mut().unwrap();
                        // M1-6 최소 경로 — 데모 피어 목록(발견 배선은 M1-4에서 실물로 교체).
                        let mut px = nbeep_gfx::Surface::new(
                            &mut buffer,
                            size.width as usize,
                            size.height as usize,
                        );
                        nbeep_ui::render(&mut px, &self.font, &self.rows);
                        buffer.present().unwrap();
                    }
                }
                _ => {}
            }
        }
    }

    /// 이벤트 루프를 열고 창을 띄운다(닫으면 종료).
    pub(crate) fn run() {
        let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트 없음");
        let font = nbeep_gfx::Font::from_bytes(data, index).expect("폰트 파싱");
        let event_loop = EventLoop::new().unwrap();
        let mut app = App {
            window: None,
            surface: None,
            font,
            rows: demo_rows(),
        };
        event_loop.run_app(&mut app).unwrap();
    }
}
