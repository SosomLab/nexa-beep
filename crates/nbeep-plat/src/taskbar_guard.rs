//! GAP-042 — winit 0.30.13 `TaskbarCreated` 교착 우회(Windows 전용 · 앱 쪽 보완 · 출처 nexa-dir3 docs/27 · 11-05 이식).
//!
//! **결함**(상류 winit `platform_impl/windows/event_loop.rs:2455-2456` · PR #4563은 master(0.31)에만 병합 · 0.30.13이 0.30 마지막 판):
//! 작업 표시줄이 다시 만들어질 때(RDP 재접속 · explorer 재시작) 셸이 등록 메시지 `TaskbarCreated`를 모든 최상위 창에
//! 브로드캐스트한다. winit은 이 메시지를 받으면 **창 상태 `Mutex` 가드를 쥔 채** `ITaskbarList::AddTab`을 부르고, 그 안의
//! `SendMessageW`(작업 표시줄 창)가 동기 대기한다. 대기 중 들어온 테마 변경 후처리(uxtheme `SetFrameTheme` → `SetWindowPos`)가
//! 같은 창에 `WM_WINDOWPOSCHANGING`을 동기 전달하고, winit이 같은 가드를 다시 잠그려다 영구 대기 → 창이 "응답 없음"
//! (nexa-dir 설치본 0.29.0 실사례 · 미니덤프 + cdb로 확정 — 같은 winit 판을 쓰는 이 앱도 같은 결함을 가진다).
//!
//! **우회**: 창을 만든 직후 `SetWindowSubclass`(comctl32 · 인박스)로 서브클래스 프로시저를 단다. 서브클래스는 클래스
//! WndProc(winit)보다 **먼저** 메시지를 받으므로 `TaskbarCreated`만 `DefWindowProcW`로 돌려보내 winit의 처리(= AddTab)를
//! 건너뛴다. 이 앱은 `skip_taskbar`를 쓰지 않으므로 AddTab은 필요 없고(탐색기가 다시 뜨면 최상위 창은 스스로 작업 표시줄에
//! 오른다) 동작 차이가 없다. 나머지 메시지는 `DefSubclassProc`로 그대로 흘려보낸다. `WM_NCDESTROY`에서 서브클래스를 뗀다.
//! 트레이 창(`tray.rs`)은 winit 창이 아니라 자체 WndProc이므로 대상이 아니다(TaskbarCreated를 받아 아이콘을 재등록해야 한다).
//!
//! **적용 규칙**: winit 창은 `nexa-beep` `app.rs`의 `new_window` 깔때기 **한 곳**에서만 만들고, 그 깔때기가 바로 [`install`]을 부른다
//! (정적 가드 시험 `app::taskbar_guard_tests` — `create_window(` 호출이 깔때기 1곳뿐인지 대조).
//! **제거 조건**: winit을 #4563이 든 판(0.31 이후)으로 올리면 이 모듈 · 깔때기의 호출 · 가드 시험을 지운다.
//!
//! 바인딩은 다른 모듈과 같이 raw `extern "system"`(외부 Win32 바인딩 크레이트 의존 0 · DR-21 · NFR-B-5).

/// 창에 서브클래스를 단다 — 창을 만든 스레드(UI 스레드)에서 바로 부른다. 반환 = 성공 여부.
/// `hwnd` = winit `RawWindowHandle::Win32`의 `hwnd.get()`. Windows 밖 = 항상 `false`(호출부는 조용히 넘긴다).
pub fn install(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        win::install(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

#[cfg(windows)]
mod win {
    use std::sync::OnceLock;

    type Hwnd = *mut core::ffi::c_void;
    type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(hwnd: Hwnd, proc: SubclassProc, id: usize, data: usize) -> i32;
        fn RemoveWindowSubclass(hwnd: Hwnd, proc: SubclassProc, id: usize) -> i32;
        fn DefSubclassProc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize;
    }
    #[link(name = "user32")]
    extern "system" {
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
    }

    const WM_NCDESTROY: u32 = 0x0082;
    /// 서브클래스 식별자("NBTB") — 같은 창에 두 번 달아도 comctl32가 같은 (proc, id) 쌍은 갱신만 한다.
    const SUBCLASS_ID: usize = 0x4e42_5442;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    /// `RegisterWindowMessageW("TaskbarCreated")` — 세션 안에서 고정(셸과 같은 값 · 0xC000~0xFFFF). 실패 = 0.
    pub(super) fn taskbar_created_msg() -> u32 {
        static MSG: OnceLock<u32> = OnceLock::new();
        // SAFETY: 널 종료 문자열 포인터 · 호출 동안 살아 있다.
        *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
    }

    pub(super) fn install(hwnd: isize) -> bool {
        // 등록 실패(0)면 걸러 낼 메시지가 없으니 달지 않는다(종전 그대로).
        if taskbar_created_msg() == 0 {
            return false;
        }
        // SAFETY: 살아 있는 창의 HWND · 프로시저는 'static · 참조 자료 없음(0).
        unsafe { SetWindowSubclass(hwnd as Hwnd, guard_proc, SUBCLASS_ID, 0) != 0 }
    }

    unsafe extern "system" fn guard_proc(
        hwnd: Hwnd,
        msg: u32,
        w: usize,
        l: isize,
        id: usize,
        _data: usize,
    ) -> isize {
        if msg == taskbar_created_msg() {
            // winit의 처리(가드 쥔 채 AddTab 동기 대기)를 건너뛴다 — winit도 결국 DefWindowProc로 끝낸다.
            return unsafe { DefWindowProcW(hwnd, msg, w, l) };
        }
        if msg == WM_NCDESTROY {
            // SAFETY: 창 소멸 마지막 메시지 · 같은 (proc, id)로 뗀다 · 이후 DefSubclassProc 호출은 허용된 패턴.
            unsafe {
                RemoveWindowSubclass(hwnd, guard_proc, id);
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, w, l) }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicU32, Ordering};

        type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

        #[repr(C)]
        struct WndClassW {
            style: u32,
            wnd_proc: WndProc,
            cls_extra: i32,
            wnd_extra: i32,
            instance: Hwnd,
            icon: Hwnd,
            cursor: Hwnd,
            background: Hwnd,
            menu_name: *const u16,
            class_name: *const u16,
        }

        #[link(name = "user32")]
        extern "system" {
            fn RegisterClassW(class: *const WndClassW) -> u16;
            fn UnregisterClassW(class_name: *const u16, instance: Hwnd) -> i32;
            #[allow(clippy::too_many_arguments)]
            fn CreateWindowExW(
                ex_style: u32,
                class_name: *const u16,
                window_name: *const u16,
                style: u32,
                x: i32,
                y: i32,
                w: i32,
                h: i32,
                parent: Hwnd,
                menu: Hwnd,
                instance: Hwnd,
                param: *mut core::ffi::c_void,
            ) -> Hwnd;
            fn SendMessageW(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize;
            fn DestroyWindow(hwnd: Hwnd) -> i32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleW(name: *const u16) -> Hwnd;
        }

        const HWND_MESSAGE: isize = -3;
        const WM_APP: u32 = 0x8000;

        /// 클래스 WndProc(= winit 자리)에 닿은 메시지 수 — TaskbarCreated · WM_APP+1 각각.
        static SEEN_TASKBAR: AtomicU32 = AtomicU32::new(0);
        static SEEN_APP: AtomicU32 = AtomicU32::new(0);

        unsafe extern "system" fn class_proc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize {
            if msg == taskbar_created_msg() {
                SEEN_TASKBAR.fetch_add(1, Ordering::SeqCst);
            } else if msg == WM_APP + 1 {
                SEEN_APP.fetch_add(1, Ordering::SeqCst);
            }
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }

        /// 메시지 전용 창(화면 없음)에 서브클래스를 달면 `TaskbarCreated`는 클래스 WndProc에 닿지 않고, 다른 메시지는
        /// 그대로 닿는다. 소멸(`WM_NCDESTROY`) 뒤에도 패닉 없이 끝난다. 사용자 화면에 창을 띄우지 않는다(헤드리스 규칙).
        #[test]
        fn taskbar_created_is_swallowed_and_other_messages_pass_through() {
            let class = wide("nbeep-taskbar-guard-test");
            let empty = wide("");
            // SAFETY: 이 시험 전용 클래스 · 메시지 전용 창 · 같은 스레드에서 만들고 지운다.
            unsafe {
                let instance = GetModuleHandleW(core::ptr::null());
                let wc = WndClassW {
                    style: 0,
                    wnd_proc: class_proc,
                    cls_extra: 0,
                    wnd_extra: 0,
                    instance,
                    icon: core::ptr::null_mut(),
                    cursor: core::ptr::null_mut(),
                    background: core::ptr::null_mut(),
                    menu_name: core::ptr::null(),
                    class_name: class.as_ptr(),
                };
                assert_ne!(RegisterClassW(&wc), 0, "RegisterClassW");
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    empty.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE as Hwnd,
                    core::ptr::null_mut(),
                    instance,
                    core::ptr::null_mut(),
                );
                assert!(!hwnd.is_null(), "CreateWindowExW");
                let msg = taskbar_created_msg();
                assert!(msg >= 0xC000, "등록 메시지 범위");

                // 가드 전: 클래스 WndProc에 닿는다(= winit이 받던 상태).
                SendMessageW(hwnd, msg, 0, 0);
                assert_eq!(SEEN_TASKBAR.load(Ordering::SeqCst), 1);

                assert!(install(hwnd as isize), "SetWindowSubclass");
                // 가드 뒤: TaskbarCreated는 걸러지고 다른 메시지는 통과.
                SendMessageW(hwnd, msg, 0, 0);
                SendMessageW(hwnd, WM_APP + 1, 0, 0);
                assert_eq!(SEEN_TASKBAR.load(Ordering::SeqCst), 1, "걸러져야 한다");
                assert_eq!(SEEN_APP.load(Ordering::SeqCst), 1, "통과해야 한다");
                // 두 번 달아도 갱신만(같은 proc · id) — 여전히 1건.
                assert!(install(hwnd as isize));
                SendMessageW(hwnd, msg, 0, 0);
                assert_eq!(SEEN_TASKBAR.load(Ordering::SeqCst), 1);

                assert_ne!(DestroyWindow(hwnd), 0, "DestroyWindow");
                assert_ne!(
                    UnregisterClassW(class.as_ptr(), instance),
                    0,
                    "UnregisterClassW"
                );
            }
        }
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    #[test]
    fn install_is_a_no_op_off_windows() {
        assert!(!super::install(0x1234));
    }
}
