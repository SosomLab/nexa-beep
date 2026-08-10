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

#[cfg(all(test, target_os = "windows"))]
mod tests {
    #[test]
    fn clipboard_roundtrip_windows() {
        let marker = "nexa-beep 클립보드 검증 ✓ 한글/CRLF\r\n포함";
        if !super::set_text(marker) {
            // 다른 프로세스가 클립보드를 잠근 순간일 수 있다 — 환경 탓 실패로 만들지 않는다.
            return;
        }
        let got = super::get_text().expect("방금 쓴 텍스트를 읽어야 한다");
        assert_eq!(got, marker, "쓴 그대로 돌아와야 한다(UTF-16 왕복)");
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    //! macOS·Linux — **OS 기본 도구**를 파이프로 쓴다(외부 크레이트 0 · DR-5).
    //! macOS `pbcopy`/`pbpaste` · Linux는 Wayland(`wl-copy`/`wl-paste`)를 먼저 보고
    //! X11(`xclip`)로 폴백한다. 도구가 없으면 실패를 그대로 보고한다(조용히 성공한 척
    //! 하지 않는다 — 복사가 안 됐는데 됐다고 하면 사용자가 붙여넣기에서 잃는다).

    use std::io::Write as _;
    use std::process::{Command, Stdio};

    /// 명령을 실행해 표준 출력을 문자열로 받는다.
    fn read_from(cmd: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(cmd)
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// 명령에 표준 입력으로 텍스트를 밀어 넣는다.
    fn write_to(cmd: &str, args: &[&str], text: &str) -> bool {
        let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let wrote = child
            .stdin
            .as_mut()
            .is_some_and(|si| si.write_all(text.as_bytes()).is_ok());
        // stdin을 닫아야 도구가 입력 끝을 안다(닫지 않으면 wait에서 멈춘다).
        drop(child.stdin.take());
        wrote && child.wait().is_ok_and(|s| s.success())
    }

    #[cfg(target_os = "macos")]
    pub(super) fn get_text() -> Option<String> {
        read_from("pbpaste", &[])
    }
    #[cfg(target_os = "macos")]
    pub(super) fn set_text(text: &str) -> bool {
        write_to("pbcopy", &[], text)
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn get_text() -> Option<String> {
        read_from("wl-paste", &["--no-newline"])
            .or_else(|| read_from("xclip", &["-selection", "clipboard", "-o"]))
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn set_text(text: &str) -> bool {
        write_to("wl-copy", &[], text) || write_to("xclip", &["-selection", "clipboard"], text)
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod unix_tests {
    /// 실측 — 이 기기에서 실제로 왕복하는가(도구가 없으면 건너뛴다).
    #[test]
    fn roundtrip_on_this_machine() {
        let marker = "nexa-beep 클립보드 \"왕복\" 시험";
        if !super::set_text(marker) {
            eprintln!("(클립보드 도구 없음 — 건너뜀)");
            return;
        }
        let got = super::get_text().expect("쓴 뒤에는 읽혀야 한다");
        assert_eq!(got.trim_end(), marker, "쓴 그대로 돌아와야 한다");
    }
}
