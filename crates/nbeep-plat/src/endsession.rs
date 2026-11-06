//! **Windows GUI 세션 종료 훅**(M1-8x 잔여 · R-16 · FR-P-7 "Windows 콘솔/세션 종료" · 11-06) — `windows` 서브시스템 GUI(탐색기
//! 실행)는 콘솔 제어 이벤트를 받지 못하므로, 주 창에 comctl32 서브클래스를 달아 `WM_QUERYENDSESSION`/`WM_ENDSESSION`을 받아
//! **종료 플래그**(`shutdown::Shutdown::flag`)를 세운다. 루프의 `about_to_wait`가 플래그를 보고 `el.exit()` → `exiting()`
//! (기록 flush · Bye · 설정 저장)로 나간다.
//!
//! - `WM_QUERYENDSESSION`: 플래그 + **TRUE(동의)** — 반환 뒤 루프가 돌아 정리한다(셸이 전 앱의 답을 모으는 동안 수 초 여유).
//! - `WM_ENDSESSION`(wParam TRUE): 플래그만(프로세스는 곧 끝난다 · 이미 정리됐으면 무해).
//! - 다른 OS·헤드리스: no-op `false`. 서브클래스 프로시저는 메인(창) 스레드에서 돌므로 앱 상태를 직접 만지지 않는다
//!   (winit이 App을 빌려 쓰는 중 — 플래그만이 안전한 통로).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 주 창에 훅 설치 — 성공 여부(Windows만 · 첫 설치의 플래그가 쓰인다).
pub fn install(hwnd: isize, flag: Arc<AtomicBool>) -> bool {
    #[cfg(windows)]
    {
        win::install(hwnd, flag)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, flag);
        false
    }
}

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    type Hwnd = *mut core::ffi::c_void;
    type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(hwnd: Hwnd, proc: SubclassProc, id: usize, data: usize) -> i32;
        fn RemoveWindowSubclass(hwnd: Hwnd, proc: SubclassProc, id: usize) -> i32;
        fn DefSubclassProc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize;
    }

    const WM_QUERYENDSESSION: u32 = 0x0011;
    const WM_ENDSESSION: u32 = 0x0016;
    const WM_NCDESTROY: u32 = 0x0082;
    /// 서브클래스 식별자("NBES") — taskbar_guard("NBTB")와 다른 id로 같은 창에 공존.
    const SUBCLASS_ID: usize = 0x4e42_4553;
    /// 종료 플래그(프로세스 1개 · 첫 설치가 정한다).
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

    pub(super) fn install(hwnd: isize, flag: Arc<AtomicBool>) -> bool {
        if hwnd == 0 {
            return false;
        }
        let _ = FLAG.set(flag);
        // SAFETY: 살아 있는 창의 HWND · 프로시저는 'static · 참조 자료 없음(0).
        unsafe { SetWindowSubclass(hwnd as Hwnd, session_proc, SUBCLASS_ID, 0) != 0 }
    }

    fn request() {
        if let Some(f) = FLAG.get() {
            f.store(true, Ordering::Relaxed);
        }
    }

    unsafe extern "system" fn session_proc(
        hwnd: Hwnd,
        msg: u32,
        w: usize,
        l: isize,
        id: usize,
        _data: usize,
    ) -> isize {
        match msg {
            WM_QUERYENDSESSION => {
                request();
                1 // TRUE = 종료에 동의 · 루프로 돌아가 정리
            }
            WM_ENDSESSION => {
                if w != 0 {
                    request();
                }
                DefSubclassProc(hwnd, msg, w, l)
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, session_proc, id);
                DefSubclassProc(hwnd, msg, w, l)
            }
            _ => DefSubclassProc(hwnd, msg, w, l),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn null_hwnd_is_refused_and_flag_request_is_idempotent() {
            let f = Arc::new(AtomicBool::new(false));
            assert!(!install(0, Arc::clone(&f)));
            let _ = FLAG.set(Arc::clone(&f));
            request();
            request();
            assert!(FLAG.get().is_some_and(|g| g.load(Ordering::Relaxed)));
        }
    }
}
