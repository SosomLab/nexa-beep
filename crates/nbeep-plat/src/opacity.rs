//! **창 투명도 포트**(nexa-dir3 DR-22 이식 · 11-06 · DR-21 이음새) — OS가 **창 전체**를 합성하는 알파 하나.
//! 소프트웨어 래스터·softbuffer·IOSurface present는 건드리지 않는다(알파 없는 `0x00RRGGBB` 그대로).
//!
//! | 타깃 | 수단 |
//! |---|---|
//! | Windows | `WS_EX_LAYERED` + `SetLayeredWindowAttributes(LWA_ALPHA)` · 불투명(255)이면 레이어드를 떼어 합성 비용 0 |
//! | macOS | `NSWindow.alphaValue`(NSView → window · objc2 msg_send · 추가 feature 없음) |
//! | Linux X11/XWayland | `_NET_WM_WINDOW_OPACITY`(CARDINAL · x11rb **지속 연결 + `check()`** — dir3 GAP-028: 요청마다 연결을 닫으면 버퍼의 쓰기가 유실) |
//! | Wayland 네이티브 | 미지원(`false`) — 호출부는 조용히 넘긴다 |
//!
//! 상한 90 %(그 이상은 창이 거의 안 보여 다시 끌 수 없다 · dir3 DR-22).

use std::ffi::c_void;

/// 투명도 상한(%).
pub const OPACITY_MAX_PCT: u8 = 90;

/// 투명도(%) → 알파(0 % = 255 불투명 · 90 % = 25). 범위 밖은 클램프.
#[must_use]
pub fn opacity_alpha(pct: i64) -> u8 {
    let p = pct.clamp(0, i64::from(OPACITY_MAX_PCT));
    u8::try_from(255 - (255 * p + 50) / 100).unwrap_or(255)
}

/// 창 핸들(raw-window-handle에서 호스트가 꺼낸다 — 이 크레이트는 winit을 모른다).
#[derive(Debug, Clone, Copy)]
pub enum WinHandle {
    /// Win32 HWND.
    Win32(isize),
    /// AppKit NSView 포인터(창은 view.window).
    AppKit(*mut c_void),
    /// X11 창 id(Xlib·Xcb 공용).
    X11(u32),
    /// Wayland 네이티브 등 — 적용 불가.
    Unsupported,
}

/// 창 전체 알파 적용 — 성공 여부(미지원·실패 = `false` · fail-soft).
pub fn apply(h: WinHandle, alpha: u8) -> bool {
    match h {
        WinHandle::Win32(hwnd) => imp::set_win32(hwnd, alpha),
        WinHandle::AppKit(view) => imp::set_mac(view, alpha),
        WinHandle::X11(win) => imp::set_x11(win, alpha),
        WinHandle::Unsupported => false,
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_LAYERED: isize = 0x0008_0000;
    const LWA_ALPHA: u32 = 0x2;
    #[link(name = "user32")]
    extern "system" {
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        fn SetLayeredWindowAttributes(hwnd: isize, key: u32, alpha: u8, flags: u32) -> i32;
    }
    pub(super) fn set_win32(hwnd: isize, alpha: u8) -> bool {
        if hwnd == 0 {
            return false;
        }
        // SAFETY: 유효한 HWND · 메인 스레드(창 소유 스레드) · 스타일 읽기/쓰기와 알파 설정뿐.
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if alpha == 255 {
                // 불투명 = 레이어드 해제(DWM 합성 비용 0 · softbuffer BitBlt 경로 그대로).
                if ex & WS_EX_LAYERED != 0 {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !WS_EX_LAYERED);
                }
                return true;
            }
            if ex & WS_EX_LAYERED == 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED);
            }
            SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA) != 0
        }
    }
    pub(super) fn set_mac(_v: *mut c_void, _a: u8) -> bool {
        false
    }
    pub(super) fn set_x11(_w: u32, _a: u8) -> bool {
        false
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use std::ffi::c_void;
    pub(super) fn set_win32(_h: isize, _a: u8) -> bool {
        false
    }
    pub(super) fn set_mac(view: *mut c_void, alpha: u8) -> bool {
        if view.is_null() {
            return false;
        }
        // SAFETY: winit이 준 NSView · 메인 스레드 · `window` 접근자와 `setAlphaValue:`뿐(AppKit 공개 API).
        unsafe {
            let view = view.cast::<AnyObject>();
            let win: *mut AnyObject = msg_send![&*view, window];
            if win.is_null() {
                return false;
            }
            let a = f64::from(alpha) / 255.0;
            let _: () = msg_send![&*win, setAlphaValue: a];
        }
        true
    }
    pub(super) fn set_x11(_w: u32, _a: u8) -> bool {
        false
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::c_void;
    use std::sync::{Mutex, OnceLock};
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// 지속 연결 + atom(dir3 GAP-028 — 요청마다 열고 닫으면 버퍼의 쓰기가 유실 · 끊기면 버리고 다음에 다시 연다).
    static LINK: OnceLock<Mutex<Option<(RustConnection, u32)>>> = OnceLock::new();

    fn with_link<R>(f: impl FnOnce(&RustConnection, u32) -> Option<R>) -> Option<R> {
        let m = LINK.get_or_init(|| Mutex::new(None));
        let mut g = m.lock().ok()?;
        if g.is_none() {
            let (c, _) = x11rb::connect(None).ok()?;
            let atom = c
                .intern_atom(false, b"_NET_WM_WINDOW_OPACITY")
                .ok()?
                .reply()
                .ok()?
                .atom;
            *g = Some((c, atom));
        }
        let r = g.as_ref().and_then(|(c, atom)| f(c, *atom));
        if r.is_none() {
            *g = None; // 연결 이상 — 다음 호출이 다시 연다
        }
        r
    }

    pub(super) fn set_win32(_h: isize, _a: u8) -> bool {
        false
    }
    pub(super) fn set_mac(_v: *mut c_void, _a: u8) -> bool {
        false
    }
    pub(super) fn set_x11(win: u32, alpha: u8) -> bool {
        with_link(|c, atom| {
            if alpha == 255 {
                c.delete_property(win, atom).ok()?.check().ok()?;
            } else {
                let v = u32::from(alpha) * 0x0101_0101;
                c.change_property32(PropMode::REPLACE, win, atom, AtomEnum::CARDINAL, &[v])
                    .ok()?
                    .check()
                    .ok()?;
            }
            c.flush().ok()?;
            Some(())
        })
        .is_some()
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod imp {
    use std::ffi::c_void;
    pub(super) fn set_win32(_h: isize, _a: u8) -> bool {
        false
    }
    pub(super) fn set_mac(_v: *mut c_void, _a: u8) -> bool {
        false
    }
    pub(super) fn set_x11(_w: u32, _a: u8) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// dir3 `window_transparency_follows_setting_and_boost`의 알파 표 그대로.
    #[test]
    fn alpha_table_and_clamp() {
        assert_eq!(opacity_alpha(0), 255);
        assert_eq!(opacity_alpha(50), 127);
        assert_eq!(opacity_alpha(90), 25);
        assert_eq!(opacity_alpha(99), 25, "상한 90 클램프");
        assert_eq!(opacity_alpha(-5), 255);
        assert!(!apply(WinHandle::Unsupported, 100));
    }
}
