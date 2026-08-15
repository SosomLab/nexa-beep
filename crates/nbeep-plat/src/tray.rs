//! 시스템 트레이/메뉴바 상주(M3-2 · FR-U-2 · 사용자 확정 08-15) — **플랫폼 어댑터**.
//!
//! 요구(사용자 확정): 아이콘 = **사용자 프로필 아바타**(RGBA — 호스트가 합성해 넘긴다) ·
//! 툴팁 = 표시 이름 · **우클릭 메뉴 = 이름 헤더(비활성) + 열기 + 종료** · 좌클릭 = 열기.
//! 라벨(열기/종료)은 호스트가 i18n으로 넘긴다 — 이 모듈은 앱 도메인을 모른다(DR-21).
//!
//! ## Windows 구현 노트 (분석 = journal/2026-08-15)
//!
//! - `Shell_NotifyIconW` 콜백은 **창 프로시저**로만 온다. winit 창의 wndproc은 winit
//!   소유라 건드리지 않는다 — **전용 스레드 + 보이지 않는 일반 창**을 만들어 거기서
//!   받고, 이벤트는 콜백(호스트가 `EventLoopProxy`로 감싼 것)으로 되돌린다(워커 문법).
//!   메시지 전용 창(HWND_MESSAGE)을 쓰지 않는 이유 = **TaskbarCreated 브로드캐스트를
//!   못 받는다**(explorer 재시작 시 아이콘 재등록 불가).
//! - 우클릭 메뉴는 **네이티브 `TrackPopupMenu`**(트레이는 OS 셸 영역 — ADR-0014
//!   "OS 네임스페이스를 여는 문은 OS 것"과 같은 경계 획정 · DR-6은 앱 창 안 원칙이라
//!   충돌 없음). `SetForegroundWindow` 선행 — 안 하면 바깥 클릭에 메뉴가 안 닫히는
//!   고전 버그(MSDN 명시).
//! - 아이콘 = RGBA→BGRA 32bpp + `CreateIconIndirect`(알파 존중 · 마스크는 형식상).
//!   갱신(NIM_MODIFY) 후 이전 HICON은 파괴(누수 방지).
//!
//! 비-Windows: 현재 스텁(`spawn` = None). macOS(NSStatusItem)는 mac PC 몫,
//! Linux(SNI)는 후속 슬라이스로 이 모듈에 붙는다.

/// 트레이에서 온 사용자 행동.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// 창 열기/복원(좌클릭 · 메뉴 "열기").
    Open,
    /// 앱 종료(메뉴 "종료").
    Quit,
}

/// 트레이 표시 내용 — 호스트가 만들어 넘긴다(이 모듈은 아바타 합성을 모른다).
#[derive(Clone, Debug, Default)]
pub struct TrayContent {
    /// 정사각 RGBA(straight alpha) — 권장 32×32.
    pub rgba: Vec<u8>,
    /// 한 변(px).
    pub side: u32,
    /// 툴팁(= 표시 이름 · 127자 초과는 절단).
    pub tooltip: String,
    /// 메뉴 헤더(비활성 — 표시 이름).
    pub name: String,
    /// "열기" 라벨(i18n — 호스트 주입).
    pub open_label: String,
    /// "종료" 라벨(i18n — 호스트 주입).
    pub quit_label: String,
}

/// 살아 있는 트레이 핸들 — 갱신 요청만 보낸다(실행은 트레이 스레드).
#[derive(Debug)]
pub struct TrayHandle {
    _priv: (),
}

#[cfg(windows)]
pub use win::spawn;

#[cfg(not(windows))]
/// 비-Windows 스텁 — 트레이 없음(호스트는 `ui.close_to_tray`를 무시해야 한다).
pub fn spawn<F: Fn(TrayEvent) + Send + Sync + 'static>(
    _content: TrayContent,
    _on_event: F,
) -> Option<TrayHandle> {
    None
}

#[cfg(not(windows))]
impl TrayHandle {
    /// 표시 내용 갱신(스텁 — 도달 불가).
    pub fn update(&self, _content: TrayContent) {}
}

#[cfg(windows)]
mod win {
    use super::{TrayContent, TrayEvent, TrayHandle};
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    type Handle = *mut core::ffi::c_void;
    type WndProc = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;

    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: WndProc,
        cls_extra: i32,
        wnd_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Handle,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: Point,
    }

    /// `NOTIFYICONDATAW`(V4 크기 — 레거시 콜백 시맨틱 사용: lParam = 마우스 메시지).
    #[repr(C)]
    struct NotifyIconDataW {
        cb_size: u32,
        hwnd: Handle,
        uid: u32,
        flags: u32,
        callback_message: u32,
        icon: Handle,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        version: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: [u8; 16],
        balloon_icon: Handle,
    }

    #[repr(C)]
    struct IconInfo {
        f_icon: i32,
        x_hotspot: u32,
        y_hotspot: u32,
        bm_mask: Handle,
        bm_color: Handle,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(class: *const WndClassW) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: *mut core::ffi::c_void,
        ) -> Handle;
        fn DefWindowProcW(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(msg: *mut Msg, hwnd: Handle, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn PostMessageW(hwnd: Handle, msg: u32, w: usize, l: isize) -> i32;
        fn PostQuitMessage(code: i32);
        fn SetForegroundWindow(hwnd: Handle) -> i32;
        fn CreatePopupMenu() -> Handle;
        fn AppendMenuW(menu: Handle, flags: u32, id: usize, label: *const u16) -> i32;
        fn TrackPopupMenu(
            menu: Handle,
            flags: u32,
            x: i32,
            y: i32,
            reserved: i32,
            hwnd: Handle,
            rect: *const core::ffi::c_void,
        ) -> i32;
        fn DestroyMenu(menu: Handle) -> i32;
        fn GetCursorPos(pt: *mut Point) -> i32;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn CreateIconIndirect(info: *const IconInfo) -> Handle;
        fn DestroyIcon(icon: Handle) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconDataW) -> i32;
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateBitmap(
            w: i32,
            h: i32,
            planes: u32,
            bits_per_px: u32,
            bits: *const core::ffi::c_void,
        ) -> Handle;
        fn DeleteObject(obj: Handle) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }

    const WM_APP_CALLBACK: u32 = 0x8000 + 1; // WM_APP+1 — Shell_NotifyIcon 콜백
    const WM_APP_UPDATE: u32 = 0x8000 + 2; // 호스트 갱신 요청(상태는 STATE에)
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_DESTROY: u32 = 0x0002;
    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    const NIF_MESSAGE: u32 = 0x01;
    const NIF_ICON: u32 = 0x02;
    const NIF_TIP: u32 = 0x04;
    const MF_STRING: u32 = 0x0000;
    const MF_GRAYED: u32 = 0x0001;
    const MF_SEPARATOR: u32 = 0x0800;
    const TPM_RETURNCMD: u32 = 0x0100;
    const TPM_RIGHTBUTTON: u32 = 0x0002;
    const CMD_OPEN: usize = 1;
    const CMD_QUIT: usize = 2;

    /// 공유 상태 — wndproc(정적 fn)과 핸들이 같은 내용을 본다. 트레이는 프로세스당
    /// 1개(앱 창 하나의 부속)라 전역이 곧 인스턴스다.
    static STATE: OnceLock<Mutex<TrayContent>> = OnceLock::new();
    static ON_EVENT: OnceLock<Box<dyn Fn(TrayEvent) + Send + Sync>> = OnceLock::new();
    static HWND: AtomicIsize = AtomicIsize::new(0);
    static PREV_ICON: AtomicIsize = AtomicIsize::new(0);
    static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn emit(ev: TrayEvent) {
        if let Some(cb) = ON_EVENT.get() {
            cb(ev);
        }
    }

    /// RGBA(straight) → HICON. 실패 시 null(아이콘 없이도 등록은 진행 — fail-soft).
    fn hicon_from_rgba(rgba: &[u8], side: u32) -> Handle {
        let px = (side * side) as usize;
        if side == 0 || rgba.len() < px * 4 {
            return core::ptr::null_mut();
        }
        // BGRA로 채널 교환(GDI 비트맵 순서).
        let mut bgra = Vec::with_capacity(px * 4);
        for p in rgba[..px * 4].chunks_exact(4) {
            bgra.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        // SAFETY: 32bpp 색 비트맵 + 형식상 마스크로 아이콘을 만든다. 생성물(비트맵)은
        // CreateIconIndirect가 복사하므로 즉시 파괴한다.
        unsafe {
            let side_i = i32::try_from(side).unwrap_or(32);
            let color = CreateBitmap(side_i, side_i, 1, 32, bgra.as_ptr().cast());
            if color.is_null() {
                return core::ptr::null_mut();
            }
            let mask = CreateBitmap(side_i, side_i, 1, 1, core::ptr::null());
            let info = IconInfo {
                f_icon: 1,
                x_hotspot: 0,
                y_hotspot: 0,
                bm_mask: mask,
                bm_color: color,
            };
            let icon = CreateIconIndirect(&info);
            DeleteObject(color);
            if !mask.is_null() {
                DeleteObject(mask);
            }
            icon
        }
    }

    /// 현재 STATE를 트레이에 반영(NIM_ADD/MODIFY 공용). 이전 아이콘은 파괴.
    fn apply_state(hwnd: Handle, op: u32) {
        let Some(state) = STATE.get() else { return };
        let c = match state.lock() {
            Ok(g) => g.clone(),
            Err(_) => return,
        };
        let icon = hicon_from_rgba(&c.rgba, c.side);
        let mut nid = NotifyIconDataW {
            cb_size: u32::try_from(core::mem::size_of::<NotifyIconDataW>()).unwrap_or(0),
            hwnd,
            uid: 1,
            flags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            callback_message: WM_APP_CALLBACK,
            icon,
            tip: [0u16; 128],
            state: 0,
            state_mask: 0,
            info: [0u16; 256],
            version: 0,
            info_title: [0u16; 64],
            info_flags: 0,
            guid: [0u8; 16],
            balloon_icon: core::ptr::null_mut(),
        };
        for (i, u) in c.tooltip.encode_utf16().take(127).enumerate() {
            nid.tip[i] = u;
        }
        // SAFETY: nid는 위에서 완전 초기화된 로컬 — 호출 동안만 참조된다.
        unsafe {
            Shell_NotifyIconW(op, &mut nid);
        }
        // 이전 HICON 파괴(누수 방지) — 새것을 슬롯에 보관.
        let prev = PREV_ICON.swap(icon as isize, Ordering::AcqRel);
        if prev != 0 {
            // SAFETY: 우리가 만든 HICON이며 트레이는 복사본을 쓴다(NIM 반영 후 파괴 안전).
            unsafe {
                DestroyIcon(prev as *mut _);
            }
        }
    }

    /// 우클릭 메뉴 — 이름 헤더(비활성) · 열기 · 종료. 네이티브 TrackPopupMenu.
    fn show_menu(hwnd: Handle) {
        let Some(state) = STATE.get() else { return };
        let (name, open_label, quit_label) = match state.lock() {
            Ok(g) => (g.name.clone(), g.open_label.clone(), g.quit_label.clone()),
            Err(_) => return,
        };
        let name_w = wide(&name);
        let open_w = wide(&open_label);
        let quit_w = wide(&quit_label);
        // SAFETY: 메뉴는 이 함수 안에서 만들고 파괴한다. SetForegroundWindow 선행은
        // TrackPopupMenu 관례(안 하면 바깥 클릭에 메뉴가 닫히지 않는다 — MSDN).
        unsafe {
            SetForegroundWindow(hwnd);
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            if !name.is_empty() {
                AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, name_w.as_ptr());
                AppendMenuW(menu, MF_SEPARATOR, 0, core::ptr::null());
            }
            AppendMenuW(menu, MF_STRING, CMD_OPEN, open_w.as_ptr());
            AppendMenuW(menu, MF_STRING, CMD_QUIT, quit_w.as_ptr());
            let mut pt = Point { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                pt.x,
                pt.y,
                0,
                hwnd,
                core::ptr::null(),
            );
            DestroyMenu(menu);
            match cmd as usize {
                CMD_OPEN => emit(TrayEvent::Open),
                CMD_QUIT => emit(TrayEvent::Quit),
                _ => {}
            }
        }
    }

    unsafe extern "system" fn wndproc(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
        match msg {
            WM_APP_CALLBACK => {
                // 레거시 시맨틱 — lParam = 마우스 메시지.
                #[allow(clippy::cast_sign_loss)]
                match l as u32 {
                    WM_LBUTTONUP => emit(TrayEvent::Open),
                    WM_RBUTTONUP => show_menu(hwnd),
                    _ => {}
                }
                0
            }
            WM_APP_UPDATE => {
                apply_state(hwnd, NIM_MODIFY);
                0
            }
            WM_DESTROY => {
                let mut nid = NotifyIconDataW {
                    cb_size: u32::try_from(core::mem::size_of::<NotifyIconDataW>()).unwrap_or(0),
                    hwnd,
                    uid: 1,
                    flags: 0,
                    callback_message: 0,
                    icon: core::ptr::null_mut(),
                    tip: [0u16; 128],
                    state: 0,
                    state_mask: 0,
                    info: [0u16; 256],
                    version: 0,
                    info_title: [0u16; 64],
                    info_flags: 0,
                    guid: [0u8; 16],
                    balloon_icon: core::ptr::null_mut(),
                };
                // SAFETY: 창 파괴 시 아이콘 제거 — nid는 로컬 완전 초기화.
                unsafe {
                    Shell_NotifyIconW(NIM_DELETE, &mut nid);
                    PostQuitMessage(0);
                }
                0
            }
            // explorer 재시작(TaskbarCreated 브로드캐스트) — 아이콘 재등록.
            m if TASKBAR_CREATED.get() == Some(&m) => {
                apply_state(hwnd, NIM_ADD);
                0
            }
            // SAFETY: 나머지는 기본 처리 위임.
            _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
        }
    }

    /// 트레이 스레드 기동 — 성공 시 핸들(갱신 통로). 프로세스당 1회(재호출 = None).
    pub fn spawn<F: Fn(TrayEvent) + Send + Sync + 'static>(
        content: TrayContent,
        on_event: F,
    ) -> Option<TrayHandle> {
        if STATE.set(Mutex::new(content)).is_err() {
            return None; // 이미 떠 있다
        }
        let _ = ON_EVENT.set(Box::new(on_event));
        std::thread::Builder::new()
            .name("tray".into())
            .spawn(|| {
                let class_name = wide("NexaBeepTray");
                // SAFETY: 클래스 등록 → 보이지 않는 일반 창 생성 → 메시지 루프.
                // 창을 만들지 못하면 스레드만 조용히 끝난다(fail-soft — 앱은 트레이
                // 없이 동작 · 호스트의 close_to_tray 가드는 HWND==0으로 판별).
                unsafe {
                    let instance = GetModuleHandleW(core::ptr::null());
                    let wc = WndClassW {
                        style: 0,
                        wnd_proc: wndproc,
                        cls_extra: 0,
                        wnd_extra: 0,
                        instance,
                        icon: core::ptr::null_mut(),
                        cursor: core::ptr::null_mut(),
                        background: core::ptr::null_mut(),
                        menu_name: core::ptr::null(),
                        class_name: class_name.as_ptr(),
                    };
                    if RegisterClassW(&wc) == 0 {
                        return;
                    }
                    let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(
                        wide("TaskbarCreated").as_ptr(),
                    ));
                    let hwnd = CreateWindowExW(
                        0,
                        class_name.as_ptr(),
                        class_name.as_ptr(),
                        0, // WS_OVERLAPPED · 표시 안 함
                        0,
                        0,
                        0,
                        0,
                        core::ptr::null_mut(),
                        core::ptr::null_mut(),
                        instance,
                        core::ptr::null_mut(),
                    );
                    if hwnd.is_null() {
                        return;
                    }
                    HWND.store(hwnd as isize, Ordering::Release);
                    apply_state(hwnd, NIM_ADD);
                    let mut msg = Msg {
                        hwnd: core::ptr::null_mut(),
                        message: 0,
                        w_param: 0,
                        l_param: 0,
                        time: 0,
                        pt: Point { x: 0, y: 0 },
                    };
                    while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })
            .ok()?;
        Some(TrayHandle { _priv: () })
    }

    impl TrayHandle {
        /// 표시 내용 갱신(아바타·이름 변경 동기) — 트레이 스레드가 반영한다.
        pub fn update(&self, content: TrayContent) {
            if let Some(state) = STATE.get() {
                if let Ok(mut g) = state.lock() {
                    *g = content;
                }
            }
            let hwnd = HWND.load(Ordering::Acquire);
            if hwnd != 0 {
                // SAFETY: 살아 있는 트레이 창으로 갱신 통지만 보낸다.
                unsafe {
                    PostMessageW(hwnd as *mut _, WM_APP_UPDATE, 0, 0);
                }
            }
        }
    }
}
