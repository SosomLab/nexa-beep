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
//! Linux = **SNI(StatusNotifierItem · D-Bus) 구현**(아래 `sni` — 실기는 Linux 환경 몫).
//! macOS = **메뉴바 NSStatusItem**(아래 `mac` — M3-2b · AppKit 메인 스레드 강제).

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

#[cfg(target_os = "linux")]
pub use sni::spawn;

#[cfg(target_os = "macos")]
pub use mac::spawn;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
/// 스텁(기타 OS) — 트레이 없음(호스트는 `ui.close_to_tray`를 무시해야 한다).
pub fn spawn<F: Fn(TrayEvent) + Send + Sync + 'static>(
    _content: TrayContent,
    _on_event: F,
) -> Option<TrayHandle> {
    None
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
impl TrayHandle {
    /// 표시 내용 갱신(스텁 — 도달 불가).
    pub fn update(&self, _content: TrayContent) {}
}

/// macOS — **메뉴바 NSStatusItem** 어댑터(M3-2b · 사용자 확정 08-15).
///
/// - **AppKit은 메인 스레드 강제**다 — `spawn`/`update`는 winit 메인 루프에서만
///   불린다(호스트의 apply_boot_settings·refresh_tray가 그 자리). 메인 스레드가
///   아니면 `None`(fail-soft — Windows의 "워처 부재"와 같은 강도).
/// - 아이콘 = 호스트가 합성한 아바타 RGBA → `NSBitmapImageRep` → `NSImage`
///   (표시 18×18pt — 32px 원본이라 레티나에서 2x로 선명).
/// - 메뉴 = **좌/우클릭 공통**(mac 관례 — 분석 표 08-15): 이름 헤더(비활성) ·
///   구분선 · 열기 · 종료. 액션은 선언 클래스(`NbeepTrayTarget`)의 셀렉터로 받아
///   콜백(호스트 `EventLoopProxy`)에 넘긴다.
/// - 상태(NSStatusItem 등 — `Send` 아님)는 **스레드 로컬**에 산다. `TrayHandle`은
///   표식일 뿐이고, 메인 스레드 밖 `update`는 조용히 무시된다(도달 경로 없음).
#[cfg(target_os = "macos")]
mod mac {
    use super::{TrayContent, TrayEvent, TrayHandle};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{declare_class, msg_send_id, mutability, sel, ClassType, DeclaredClass};
    use objc2_app_kit::{
        NSBitmapImageRep, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
        NSVariableStatusItemLength,
    };
    use objc2_foundation::{MainThreadMarker, NSSize, NSString};
    use std::cell::RefCell;
    use std::sync::OnceLock;

    /// 이벤트 콜백(호스트 프록시 래퍼) — 액션 셀렉터에서 부른다.
    static ON_EVENT: OnceLock<Box<dyn Fn(TrayEvent) + Send + Sync>> = OnceLock::new();

    struct State {
        item: Retained<NSStatusItem>,
        header: Retained<NSMenuItem>,
        open: Retained<NSMenuItem>,
        quit: Retained<NSMenuItem>,
        _target: Retained<Target>,
    }

    thread_local! {
        /// 메인 스레드 전용 상태(NSStatusItem은 Send가 아니다).
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    declare_class!(
        /// 메뉴 액션 수신자 — 셀렉터를 콜백으로 잇는 것 외에 아무것도 모른다.
        struct Target;

        unsafe impl ClassType for Target {
            type Super = NSObject;
            type Mutability = mutability::InteriorMutable;
            const NAME: &'static str = "NbeepTrayTarget";
        }

        impl DeclaredClass for Target {
            type Ivars = ();
        }

        unsafe impl Target {
            #[method(nbeepTrayOpen:)]
            fn nbeep_tray_open(&self, _sender: Option<&AnyObject>) {
                if let Some(f) = ON_EVENT.get() {
                    f(TrayEvent::Open);
                }
            }

            #[method(nbeepTrayQuit:)]
            fn nbeep_tray_quit(&self, _sender: Option<&AnyObject>) {
                if let Some(f) = ON_EVENT.get() {
                    f(TrayEvent::Quit);
                }
            }
        }
    );

    /// RGBA(straight) → NSImage(18×18pt 표시 · 원본 해상도 유지 = 레티나 2x).
    fn image_from_rgba(rgba: &[u8], side: u32) -> Option<Retained<NSImage>> {
        if side == 0 || rgba.len() != (side as usize) * (side as usize) * 4 {
            return None;
        }
        unsafe {
            let rep: Option<Retained<NSBitmapImageRep>> = msg_send_id![
                NSBitmapImageRep::alloc(),
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
                pixelsWide: side as isize,
                pixelsHigh: side as isize,
                bitsPerSample: 8_isize,
                samplesPerPixel: 4_isize,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: &*NSString::from_str("NSDeviceRGBColorSpace"),
                bytesPerRow: (side * 4) as isize,
                bitsPerPixel: 32_isize,
            ];
            let rep = rep?;
            let data = rep.bitmapData();
            if data.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), data, rgba.len());
            let img = NSImage::initWithSize(NSImage::alloc(), NSSize::new(18.0, 18.0));
            img.addRepresentation(&rep);
            Some(img)
        }
    }

    fn apply(mtm: MainThreadMarker, state: &State, content: &TrayContent) {
        unsafe {
            if let Some(btn) = state.item.button(mtm) {
                if let Some(img) = image_from_rgba(&content.rgba, content.side) {
                    btn.setImage(Some(&img));
                }
                btn.setToolTip(Some(&NSString::from_str(&content.tooltip)));
            }
            state.header.setTitle(&NSString::from_str(&content.name));
            state
                .open
                .setTitle(&NSString::from_str(&content.open_label));
            state
                .quit
                .setTitle(&NSString::from_str(&content.quit_label));
        }
    }

    /// 메뉴바 상주 시작 — **메인 스레드에서만**(아니면 None · fail-soft).
    pub fn spawn<F: Fn(TrayEvent) + Send + Sync + 'static>(
        content: TrayContent,
        on_event: F,
    ) -> Option<TrayHandle> {
        let mtm = MainThreadMarker::new()?; // AppKit 계약 — 메인 스레드 증명
        let _ = ON_EVENT.set(Box::new(on_event));

        let target: Retained<Target> = unsafe { msg_send_id![Target::alloc(), init] };
        unsafe {
            let bar = NSStatusBar::systemStatusBar();
            let item = bar.statusItemWithLength(NSVariableStatusItemLength);

            let menu = NSMenu::new(mtm);
            let header = NSMenuItem::new(mtm);
            header.setEnabled(false); // 이름 헤더(비활성 — Windows와 동일 문법)
            menu.addItem(&header);
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            let open = NSMenuItem::new(mtm);
            open.setAction(Some(sel!(nbeepTrayOpen:)));
            open.setTarget(Some(&target));
            menu.addItem(&open);
            let quit = NSMenuItem::new(mtm);
            quit.setAction(Some(sel!(nbeepTrayQuit:)));
            quit.setTarget(Some(&target));
            menu.addItem(&quit);
            // mac 관례 — 좌/우클릭 모두 메뉴(분석 표 08-15). 메뉴를 달면 둘 다 연다.
            item.setMenu(Some(&menu));

            let state = State {
                item,
                header,
                open,
                quit,
                _target: target,
            };
            apply(mtm, &state, &content);
            STATE.with(|s| *s.borrow_mut() = Some(state));
        }
        Some(TrayHandle { _priv: () })
    }

    impl TrayHandle {
        /// 표시 내용 갱신 — 메인 스레드에서만 실제 반영(밖이면 무시 · 도달 경로 없음).
        pub fn update(&self, content: TrayContent) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            STATE.with(|s| {
                if let Some(state) = s.borrow().as_ref() {
                    apply(mtm, state, &content);
                }
            });
        }
    }
}

/// Linux — **StatusNotifierItem(SNI · D-Bus)** 어댑터(M3-2c · 사용자 확정 08-15
/// "리눅스까지 여기서 구현"). ⚠ **이 PC에서 실기 불가** — 컴파일·CI 크로스 체크로
/// 검증하고, 데스크톱 실기(KDE/GNOME+확장)는 추후 Linux 환경에서.
///
/// - 세션 버스에 `org.kde.StatusNotifierItem`(아이콘·툴팁·Activate=열기)과
///   `com.canonical.dbusmenu`(`/MenuBar` — 이름 헤더·열기·종료)를 **서빙**하고
///   `StatusNotifierWatcher`에 등록한다. **워처가 없으면(GNOME 확장 부재 등) None**
///   (fail-soft — 호스트의 `close_to_tray` 가드가 "트레이 없으면 종전 종료"를 지킨다).
/// - 아이콘 = `IconPixmap`(ARGB32 **네트워크 바이트 순서** — SNI 규격) · 갱신 =
///   `NewIcon`/`NewToolTip`/`NewTitle` + dbusmenu `LayoutUpdated` 신호.
/// - 의존 = `zbus`(MIT · 순수 Rust — 시스템 런타임 요구 없음 · Linux 타깃 한정.
///   원장 docs/10 §3).
#[cfg(target_os = "linux")]
mod sni {
    use super::{TrayContent, TrayEvent, TrayHandle};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, OnceLock};
    use zbus::blocking::Connection;
    use zbus::zvariant::{ObjectPath, OwnedValue, Value};

    static STATE: OnceLock<Mutex<TrayContent>> = OnceLock::new();
    static ON_EVENT: OnceLock<Box<dyn Fn(TrayEvent) + Send + Sync>> = OnceLock::new();
    static CONN: OnceLock<Connection> = OnceLock::new();
    /// dbusmenu 레이아웃 리비전(갱신마다 증가 — 호스트가 재조회).
    static MENU_REV: AtomicU32 = AtomicU32::new(1);

    /// SNI 픽스맵 — `(w, h, ARGB32)` 목록(규격 시그니처 `a(iiay)`).
    type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
    /// SNI 툴팁 — `(아이콘명, 픽스맵, 제목, 본문)`.
    type ToolTip = (String, Pixmaps, String, String);
    /// dbusmenu 항목 — `(id, 속성, 자식)`.
    type MenuNode = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

    const ITEM_PATH: &str = "/StatusNotifierItem";
    const MENU_PATH: &str = "/MenuBar";
    /// 메뉴 항목 id — 1 = 이름 헤더(비활성) · 2 = 구분선 · 3 = 열기 · 4 = 종료.
    const ID_HEADER: i32 = 1;
    const ID_SEP: i32 = 2;
    const ID_OPEN: i32 = 3;
    const ID_QUIT: i32 = 4;

    fn emit(ev: TrayEvent) {
        if let Some(cb) = ON_EVENT.get() {
            cb(ev);
        }
    }

    fn state() -> TrayContent {
        STATE
            .get()
            .and_then(|m| m.lock().ok().map(|g| g.clone()))
            .unwrap_or_default()
    }

    /// RGBA(straight) → SNI `IconPixmap`(ARGB32 · 네트워크 바이트 순서 = [A,R,G,B]).
    fn argb_pixmap() -> Pixmaps {
        let c = state();
        let px = (c.side as usize) * (c.side as usize);
        if c.side == 0 || c.rgba.len() < px * 4 {
            return Vec::new();
        }
        let mut argb = Vec::with_capacity(px * 4);
        for p in c.rgba[..px * 4].chunks_exact(4) {
            argb.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
        }
        let side = i32::try_from(c.side).unwrap_or(32);
        vec![(side, side, argb)]
    }

    /// `Value` → `OwnedValue`(실패 = None — fd 없는 값이라 실질 불가).
    fn ov<'a>(v: impl Into<Value<'a>>) -> Option<OwnedValue> {
        v.into().try_to_owned().ok()
    }

    /// 메뉴 항목 속성 집합.
    fn item_props(id: i32) -> HashMap<String, OwnedValue> {
        let c = state();
        let mut m = HashMap::new();
        let mut put = |k: &str, v: Option<OwnedValue>| {
            if let Some(v) = v {
                m.insert(k.to_string(), v);
            }
        };
        match id {
            ID_HEADER => {
                put("label", ov(c.name));
                put("enabled", ov(false));
            }
            ID_SEP => put("type", ov("separator")),
            ID_OPEN => put("label", ov(c.open_label)),
            ID_QUIT => put("label", ov(c.quit_label)),
            _ => {}
        }
        m
    }

    /// (id, 속성, 자식 없음) 구조 — dbusmenu 항목 `(ia{sv}av)`.
    fn item_value(id: i32) -> Option<OwnedValue> {
        ov((id, item_props(id), Vec::<OwnedValue>::new()))
    }

    /// `org.kde.StatusNotifierItem` — 호스트(트레이 영역)가 읽는다.
    struct Item;

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Item {
        #[zbus(property)]
        fn category(&self) -> String {
            "ApplicationStatus".into()
        }
        #[zbus(property)]
        fn id(&self) -> String {
            "nexa-beep".into()
        }
        #[zbus(property)]
        fn title(&self) -> String {
            state().name
        }
        #[zbus(property)]
        fn status(&self) -> String {
            "Active".into()
        }
        #[zbus(property)]
        fn icon_name(&self) -> String {
            String::new() // 픽스맵만 쓴다(테마 아이콘 없음 — 아바타가 곧 아이콘)
        }
        #[zbus(property)]
        fn icon_pixmap(&self) -> Pixmaps {
            argb_pixmap()
        }
        #[zbus(property)]
        fn tool_tip(&self) -> ToolTip {
            (String::new(), Vec::new(), state().tooltip, String::new())
        }
        #[zbus(property)]
        fn menu(&self) -> ObjectPath<'static> {
            ObjectPath::from_static_str_unchecked(MENU_PATH)
        }
        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false // 좌클릭 = Activate(열기) · 메뉴는 우클릭(호스트 관례)
        }
        fn activate(&self, _x: i32, _y: i32) {
            emit(TrayEvent::Open);
        }
        fn secondary_activate(&self, _x: i32, _y: i32) {
            emit(TrayEvent::Open);
        }
        fn context_menu(&self, _x: i32, _y: i32) {
            // 메뉴 렌더는 dbusmenu를 읽는 호스트 몫 — 여기 올 일은 드물다(no-op).
        }
        fn scroll(&self, _delta: i32, _orientation: String) {}
    }

    /// `com.canonical.dbusmenu` — 최소 구현(정적 4항목 · 클릭 이벤트만).
    struct Menu;

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl Menu {
        #[zbus(property)]
        fn version(&self) -> u32 {
            3
        }
        #[zbus(property)]
        fn status(&self) -> String {
            "normal".into()
        }
        #[zbus(property)]
        fn text_direction(&self) -> String {
            "ltr".into()
        }
        #[zbus(property)]
        fn icon_theme_path(&self) -> Vec<String> {
            Vec::new()
        }

        /// 레이아웃 — 루트(0) 요청에만 4항목을 자식으로 준다(깊이 1 고정 메뉴).
        fn get_layout(
            &self,
            parent_id: i32,
            _recursion_depth: i32,
            _property_names: Vec<String>,
        ) -> zbus::fdo::Result<(u32, MenuNode)> {
            let rev = MENU_REV.load(Ordering::Relaxed);
            let children = if parent_id == 0 {
                [ID_HEADER, ID_SEP, ID_OPEN, ID_QUIT]
                    .into_iter()
                    .filter_map(item_value)
                    .collect()
            } else {
                Vec::new()
            };
            Ok((rev, (0, HashMap::new(), children)))
        }

        fn get_group_properties(
            &self,
            ids: Vec<i32>,
            _property_names: Vec<String>,
        ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
            ids.into_iter().map(|id| (id, item_props(id))).collect()
        }

        fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
            item_props(id).remove(&name).ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs(format!("항목 {id}에 속성 {name} 없음"))
            })
        }

        /// 클릭 처리 — 열기/종료만 의미가 있다.
        fn event(&self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
            if event_id == "clicked" {
                match id {
                    ID_OPEN => emit(TrayEvent::Open),
                    ID_QUIT => emit(TrayEvent::Quit),
                    _ => {}
                }
            }
        }

        fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
            for (id, event_id, _d, _t) in events {
                if event_id == "clicked" {
                    match id {
                        ID_OPEN => emit(TrayEvent::Open),
                        ID_QUIT => emit(TrayEvent::Quit),
                        _ => {}
                    }
                }
            }
            Vec::new()
        }

        fn about_to_show(&self, _id: i32) -> bool {
            false
        }

        fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
            (Vec::new(), Vec::new())
        }
    }

    /// 세션 버스에 서빙 + 워처 등록. 실패(버스 부재·워처 부재)는 None — fail-soft.
    pub fn spawn<F: Fn(TrayEvent) + Send + Sync + 'static>(
        content: TrayContent,
        on_event: F,
    ) -> Option<TrayHandle> {
        if STATE.set(Mutex::new(content)).is_err() {
            return None; // 이미 떠 있다
        }
        let _ = ON_EVENT.set(Box::new(on_event));
        let conn = Connection::session().ok()?;
        conn.object_server().at(ITEM_PATH, Item).ok()?;
        conn.object_server().at(MENU_PATH, Menu).ok()?;
        let unique = conn.unique_name()?.to_string();
        // 워처 등록 — 이게 없으면 트레이를 그릴 호스트가 없다(fail-soft: None).
        conn.call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.kde.StatusNotifierWatcher"),
            "RegisterStatusNotifierItem",
            &unique,
        )
        .ok()?;
        let _ = CONN.set(conn);
        Some(TrayHandle { _priv: () })
    }

    impl TrayHandle {
        /// 표시 내용 갱신 — 신호(NewIcon 등)로 호스트가 재조회한다.
        pub fn update(&self, content: TrayContent) {
            if let Some(s) = STATE.get() {
                if let Ok(mut g) = s.lock() {
                    *g = content;
                }
            }
            let rev = MENU_REV.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(conn) = CONN.get() {
                let iface = "org.kde.StatusNotifierItem";
                for sig in ["NewIcon", "NewToolTip", "NewTitle"] {
                    let _ = conn.emit_signal(
                        None::<zbus::names::BusName<'_>>,
                        ITEM_PATH,
                        iface,
                        sig,
                        &(),
                    );
                }
                let _ = conn.emit_signal(
                    None::<zbus::names::BusName<'_>>,
                    MENU_PATH,
                    "com.canonical.dbusmenu",
                    "LayoutUpdated",
                    &(rev, 0i32),
                );
            }
        }
    }
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
    const WM_APP_BALLOON: u32 = 0x8000 + 3; // 풍선 알림 요청(내용은 BALLOON에 · M3-8)
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_DESTROY: u32 = 0x0002;
    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    const NIF_MESSAGE: u32 = 0x01;
    const NIF_ICON: u32 = 0x02;
    const NIF_TIP: u32 = 0x04;
    const NIF_INFO: u32 = 0x10; // 풍선(info/info_title/info_flags 유효 · M3-8)
    const NIIF_INFO: u32 = 0x01;
    const NIIF_NOSOUND: u32 = 0x10; // 신뢰 게이트 — 미검증은 소리 없음(DR-25)
    /// 풍선 클릭(레거시 콜백 lParam) — 알림 클릭 = 앱 열기(08-15 사용자 실기 후속).
    const NIN_BALLOONUSERCLICK: u32 = 0x0405;
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
    /// 대기 중 풍선(M3-8) — (제목, 본문, 무음). 마지막 것만 유효(폭주는 호스트 스로틀).
    static BALLOON: Mutex<Option<(String, String, bool)>> = Mutex::new(None);

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

    /// 대기 중 풍선을 표시(M3-8) — NIF_INFO만 갱신(아이콘·툴팁 불변). 제목 63자·
    /// 본문 255자 절단(u16 셀 마지막은 NUL). 무음 = NIIF_NOSOUND(DR-25 미검증).
    fn show_balloon(hwnd: Handle) {
        let Some((title, body, silent)) = BALLOON.lock().ok().and_then(|mut g| g.take()) else {
            return;
        };
        let mut nid = NotifyIconDataW {
            cb_size: u32::try_from(core::mem::size_of::<NotifyIconDataW>()).unwrap_or(0),
            hwnd,
            uid: 1,
            flags: NIF_INFO,
            callback_message: 0,
            icon: core::ptr::null_mut(),
            tip: [0u16; 128],
            state: 0,
            state_mask: 0,
            info: [0u16; 256],
            version: 0,
            info_title: [0u16; 64],
            info_flags: NIIF_INFO | if silent { NIIF_NOSOUND } else { 0 },
            guid: [0u8; 16],
            balloon_icon: core::ptr::null_mut(),
        };
        for (i, u) in title.encode_utf16().take(63).enumerate() {
            nid.info_title[i] = u;
        }
        for (i, u) in body.encode_utf16().take(255).enumerate() {
            nid.info[i] = u;
        }
        // SAFETY: 살아 있는 트레이 아이콘(uid 1)의 풍선 필드만 수정.
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &mut nid);
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
                    // 풍선 알림 클릭 = 창 복원(트레이 좌클릭과 같은 경로 — M3-8).
                    NIN_BALLOONUSERCLICK => emit(TrayEvent::Open),
                    _ => {}
                }
                0
            }
            WM_APP_UPDATE => {
                apply_state(hwnd, NIM_MODIFY);
                0
            }
            WM_APP_BALLOON => {
                show_balloon(hwnd);
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
                    let _ = TASKBAR_CREATED
                        .set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
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

        /// 풍선 알림(M3-8 · Windows = 트레이가 알림 채널) — 트레이 스레드가 표시.
        pub fn notify(&self, title: &str, body: &str, silent: bool) {
            if let Ok(mut g) = BALLOON.lock() {
                *g = Some((title.to_string(), body.to_string(), silent));
            }
            let hwnd = HWND.load(Ordering::Acquire);
            if hwnd != 0 {
                // SAFETY: 살아 있는 트레이 창으로 표시 요청만 보낸다.
                unsafe {
                    PostMessageW(hwnd as *mut _, WM_APP_BALLOON, 0, 0);
                }
            }
        }
    }
}
