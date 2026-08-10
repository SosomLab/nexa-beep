//! OS 클립보드 **텍스트** 어댑터(DR-21 — 외부 기술은 이음새 뒤에).
//!
//! ui 계층은 OS를 모른다 — 위젯은 문자열만 내놓고/받고, 호출자(bin)가 이 모듈로
//! OS 클립보드와 오간다. **Windows만 실물**(CF_UNICODETEXT · Win32 직접 호출 —
//! 런타임 의존 0 유지). macOS(NSPasteboard)·Linux(X11/Wayland)는 objc·디스플레이
//! 서버 연동이 필요해 **의도된 보류**(M3-1b OS 동작 어댑터) — 미지원 환경은
//! 조용히 실패하는 대신 `None`/`false`로 정직하게 알린다.

/// 클립보드의 텍스트를 읽는다. 미지원 OS·비텍스트·잠금 실패면 `None`.
#[must_use]
pub fn get_text() -> Option<String> {
    imp::get_text()
}

/// 클립보드에 텍스트를 쓴다. 성공 여부 반환(미지원 OS는 `false`).
pub fn set_text(text: &str) -> bool {
    imp::set_text(text)
}

#[cfg(target_os = "windows")]
mod imp {
    use core::ffi::c_void;

    type Handle = *mut c_void;
    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(hwnd: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> Handle;
        fn SetClipboardData(format: u32, mem: Handle) -> Handle;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(mem: Handle) -> *mut c_void;
        fn GlobalUnlock(mem: Handle) -> i32;
        fn GlobalFree(mem: Handle) -> Handle;
    }

    pub(super) fn get_text() -> Option<String> {
        // SAFETY: Win32 클립보드 규약 그대로 — Open 성공 시 반드시 Close,
        // GetClipboardData 핸들은 시스템 소유(해제 금지), Lock/Unlock 짝 유지.
        unsafe {
            if OpenClipboard(core::ptr::null_mut()) == 0 {
                return None;
            }
            let h = GetClipboardData(CF_UNICODETEXT);
            let out = if h.is_null() {
                None
            } else {
                let p = GlobalLock(h).cast::<u16>();
                if p.is_null() {
                    None
                } else {
                    let mut len = 0usize;
                    while *p.add(len) != 0 {
                        len += 1;
                    }
                    let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, len));
                    GlobalUnlock(h);
                    Some(s)
                }
            };
            CloseClipboard();
            out
        }
    }

    pub(super) fn set_text(text: &str) -> bool {
        let wide: Vec<u16> = text.encode_utf16().chain(core::iter::once(0)).collect();
        // SAFETY: GMEM_MOVEABLE 메모리에 UTF-16(널 종단)을 채워 SetClipboardData에
        // 넘기면 **소유권이 시스템으로 이전**된다(성공 시 GlobalFree 금지 · 실패 시 해제).
        unsafe {
            if OpenClipboard(core::ptr::null_mut()) == 0 {
                return false;
            }
            let mut ok = false;
            if EmptyClipboard() != 0 {
                let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
                if !h.is_null() {
                    let p = GlobalLock(h);
                    if p.is_null() {
                        GlobalFree(h);
                    } else {
                        core::ptr::copy_nonoverlapping(wide.as_ptr(), p.cast::<u16>(), wide.len());
                        GlobalUnlock(h);
                        if SetClipboardData(CF_UNICODETEXT, h).is_null() {
                            GlobalFree(h);
                        } else {
                            ok = true;
                        }
                    }
                }
            }
            CloseClipboard();
            ok
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    /// 의도된 보류 — macOS/Linux 클립보드는 M3-1b(OS 동작 어댑터)에서.
    pub fn get_text() -> Option<String> {
        None
    }
    pub fn set_text(_text: &str) -> bool {
        false
    }
}
