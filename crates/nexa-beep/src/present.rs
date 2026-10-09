//! **화면 내보내기 한 곳**(10-10 · nexa-sql `present.rs` 이식 — docs/62 §2) — 모든 창이 이 타입으로 픽셀을 낸다.
//!
//! - 기본 = macOS **IOSurface**(`nexa-sys::layer_present::LayerPresenter` — 표면 풀 3장 · 할당 0 · 색 맞춤은 합성기) ·
//!   그 밖 OS = `softbuffer`(종전 그대로). 만들기에 실패하면 조용히 `softbuffer`로 돌아간다.
//! - 왜: softbuffer 0.4 CoreGraphics 뒷단은 프레임마다 새 버퍼를 할당하고 DeviceRGB `CGImage`를 넘겨 CoreAnimation이
//!   **프레임마다 전체를 CPU 색 변환**한다 — beep 실측(Intel Retina · 1600×1200) present 15~17ms = 프레임의 절반 이상.
//! - A/B 스위치: 환경변수 `NEXA_MAC_PRESENT=softbuffer`(강제 종전 경로) — 실측·회귀 비교용. 창을 만들 때 정해진다.
//!
//! 모양은 `softbuffer::Surface`와 같다(`resize` → `buffer_mut` → 그리기 → `present`) — 창 코드가 뒷단을 모른다(DR-21).

use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use winit::window::Window;

type Soft = softbuffer::Surface<Rc<Window>, Rc<Window>>;

enum Kind {
    Soft {
        surface: Soft,
        /// 컨텍스트는 표면보다 오래 살아야 한다.
        _ctx: softbuffer::Context<Rc<Window>>,
    },
    Layer {
        p: nexa_sys::layer_present::LayerPresenter,
        win: Rc<Window>,
    },
}

/// 창 하나의 내보내기.
pub(crate) struct Presenter {
    kind: Kind,
    /// 마지막 프레임을 연 시각 — 유휴 해제 판정(`trim_if_idle`).
    last_frame: std::time::Instant,
}

/// 이만큼 프레임이 없으면 IOSurface 풀을 앞 장만 남기고 놓는다(DR-5 — 유휴 20→36MB 실측의 처방 · 창당 +5MB만 남는다).
pub(crate) const IDLE_TRIM: std::time::Duration = std::time::Duration::from_millis(1500);

impl std::fmt::Debug for Presenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.backend())
    }
}

/// 그릴 버퍼(`[u32]` = `0x00RRGGBB` · 폭 × 높이). 다 그린 뒤 [`Buffer::present`].
pub(crate) enum Buffer<'a> {
    Soft(softbuffer::Buffer<'a, Rc<Window>, Rc<Window>>),
    Layer(nexa_sys::layer_present::Frame<'a>),
}

/// IOSurface 경로를 쓸 것인가 — macOS **이고** 강제 끔(`softbuffer`)이 아닐 때(순수 판정 · 테스트).
pub(crate) fn use_layer(macos: bool, forced_off: bool) -> bool {
    macos && !forced_off
}

fn forced_soft() -> bool {
    std::env::var_os("NEXA_MAC_PRESENT").is_some_and(|v| v == "softbuffer")
}

impl Presenter {
    /// 창에 붙인다. 실패 = `Err(사유)`.
    pub(crate) fn new(win: Rc<Window>) -> Result<Presenter, String> {
        if use_layer(cfg!(target_os = "macos"), forced_soft()) {
            if let Some(p) = layer_for(&win) {
                return Ok(Presenter {
                    kind: Kind::Layer { p, win },
                    last_frame: std::time::Instant::now(),
                });
            }
        }
        let ctx = softbuffer::Context::new(win.clone())
            .map_err(|e| format!("softbuffer context failed: {e}"))?;
        let surface = softbuffer::Surface::new(&ctx, win)
            .map_err(|e| format!("softbuffer surface failed: {e}"))?;
        Ok(Presenter {
            kind: Kind::Soft { surface, _ctx: ctx },
            last_frame: std::time::Instant::now(),
        })
    }

    pub(crate) fn resize(&mut self, w: NonZeroU32, h: NonZeroU32) -> Result<(), ()> {
        match &mut self.kind {
            Kind::Soft { surface, .. } => surface.resize(w, h).map_err(|_| ()),
            Kind::Layer { p, win } => {
                if p.resize(w.get(), h.get(), win.scale_factor()) {
                    Ok(())
                } else {
                    Err(())
                }
            }
        }
    }

    pub(crate) fn buffer_mut(&mut self) -> Result<Buffer<'_>, ()> {
        self.last_frame = std::time::Instant::now();
        match &mut self.kind {
            Kind::Soft { surface, .. } => surface.buffer_mut().map(Buffer::Soft).map_err(|_| ()),
            Kind::Layer { p, .. } => p.frame().map(Buffer::Layer).ok_or(()),
        }
    }

    /// 유휴 해제 — 마지막 프레임 뒤 `IDLE_TRIM`이 지났으면 IOSurface 풀을 앞 장만 남긴다(softbuffer = 0). 반환 = 놓은 장 수.
    pub(crate) fn trim_if_idle(&mut self) -> usize {
        match &mut self.kind {
            Kind::Layer { p, .. } if self.last_frame.elapsed() >= IDLE_TRIM => p.trim_idle(),
            _ => 0,
        }
    }

    /// 뒷단 이름(진단·계측 표기).
    pub(crate) fn backend(&self) -> &'static str {
        match self.kind {
            Kind::Soft { .. } => "softbuffer",
            Kind::Layer { .. } => "iosurface",
        }
    }
}

impl Buffer<'_> {
    pub(crate) fn present(self) -> Result<(), ()> {
        match self {
            Buffer::Soft(b) => b.present().map_err(|_| ()),
            Buffer::Layer(f) => {
                f.present();
                Ok(())
            }
        }
    }
}

impl Deref for Buffer<'_> {
    type Target = [u32];
    fn deref(&self) -> &[u32] {
        match self {
            Buffer::Soft(b) => b,
            Buffer::Layer(f) => f.pixels(),
        }
    }
}

impl DerefMut for Buffer<'_> {
    fn deref_mut(&mut self) -> &mut [u32] {
        match self {
            Buffer::Soft(b) => b,
            Buffer::Layer(f) => f.pixels_mut(),
        }
    }
}

fn layer_for(win: &Rc<Window>) -> Option<nexa_sys::layer_present::LayerPresenter> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = win.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(h) => {
            // SAFETY: winit이 준 NSView 포인터 · 메인 스레드(창 생성 지점) · 레이어는 프레젠터가 소유.
            unsafe { nexa_sys::layer_present::LayerPresenter::new(h.ns_view.as_ptr()) }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn use_layer_mcdc() {
        use super::use_layer as f;
        assert!(f(true, false));
        assert!(!f(false, false), "macOS가 아니면 끔");
        assert!(!f(true, true), "NEXA_MAC_PRESENT=softbuffer 면 끔");
    }
}
