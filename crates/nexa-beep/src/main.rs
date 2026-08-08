//! Nexa Beep 본체 — 진입·조립·생명주기.
//!
//! ⚠️ **M0-2 SP-1 스파이크 상태**([docs/07] §6 · [docs/15] "버릴 수 있는 실험").
//! 지금은 **빈 창 + 픽셀 버퍼 표시**만으로 예산(R-8)을 실측한다 — winit(창·입력·IME) +
//! softbuffer(픽셀 버퍼 present)는 ADR-0001 **P2**(얇은 창 라이브러리)의 후보다.
//! 실제 발견/대화 배선은 M1+, 창 코드의 `nbeep-plat` 이관은 M3.
//!
//! `--window` 인자로 창을 띄운다(기본은 스캐폴드 출력 — 헤드리스 CI 안전).

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

    #[derive(Default)]
    struct App {
        window: Option<Rc<Window>>,
        surface: Option<Surface>,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            let attrs = Window::default_attributes().with_title("Nexa Beep — SP-1");
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
                        // 어두운 회색으로 클리어(softbuffer 픽셀 = 0x00RRGGBB).
                        buffer.fill(0x0020_2124);
                        buffer.present().unwrap();
                    }
                }
                _ => {}
            }
        }
    }

    /// 이벤트 루프를 열고 창을 띄운다(닫으면 종료).
    pub(crate) fn run() {
        let event_loop = EventLoop::new().unwrap();
        let mut app = App::default();
        event_loop.run_app(&mut app).unwrap();
    }
}
