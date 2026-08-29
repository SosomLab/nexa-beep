//! Linux 클립보드 **자체 구현**(08-29 · TODO L-1) — 도구 스폰(`wl-paste`/`xclip`) 폐지.
//!
//! 배경: GNOME 기본 설치에는 `wl-copy`·`wl-paste`·`xclip` 셋 다 없다(Ubuntu 실측) →
//! Linux에서 복사·붙여넣기·클립보드 이미지 전송이 전부 죽어 있었다. 클립보드는 배포판이
//! 아니라 **디스플레이 서버 프로토콜** 층이라 두 축만 구현하면 어디서나 같다:
//!
//! - **Wayland** — winit이 연 `wl_display`를 빌려(`wlactivate` 선례 · foreign display) 별도
//!   큐를 **상주 스레드**에서 돌린다. 프로토콜은 두 갈래: ① `ext_data_control_v1`(있으면 —
//!   포커스·시리얼 불요 · Mutter 48+/wlroots/KWin) ② `wl_data_device`(코어 — 붙여넣기는
//!   포커스 있는 창에서만 · 복사는 **최근 입력 시리얼** 필요 → 같은 클라이언트의 두 번째
//!   `wl_keyboard`로 시리얼을 받는다(winit 것과 나란히 · 이벤트는 둘 다에게 온다)).
//!   상주인 이유: `selection` 이벤트는 **포커스 진입 때** 오고, 복사 원본은 `send` 요청을
//!   나중에 받아야 한다 — 붙여넣기 순간에만 큐를 만들면 둘 다 못 본다.
//!   요청은 어느 스레드에서나 보낼 수 있고(프록시 = Send+Sync) 이벤트만 워커 큐로 온다.
//! - **X11** — ICCCM 셀렉션(`x11rb` 순수 Rust · winit과 같은 판). 붙여넣기 = 요청 연결 1회
//!   (`ConvertSelection` → `SelectionNotify` · INCR 조립) · 복사 = 소유 스레드(`SelectionRequest`
//!   응답 · `SelectionClear`로 종료).
//!
//! 봉투 원리: 여기서 보는 것은 MIME 이름과 바이트뿐. 상한 64MiB(할당 폭탄) · 대기 5s(원본
//! 클라이언트가 안 쓰면 fail-closed) · 실패는 `None`/`false`로 정직하게(조용한 성공 금지).

use std::sync::{Arc, OnceLock};

/// 읽기 상한 — 이미지 상한(16MiB)보다 넉넉히, 할당 폭탄은 막는다.
const MAX_BYTES: usize = 64 << 20;
/// 원본 클라이언트가 파이프에 쓰길 기다리는 상한(ms).
const READ_TIMEOUT_MS: i32 = 5_000;

/// 텍스트로 받을 MIME 사다리(선호 순).
pub const TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "TEXT",
    "STRING",
];
/// 이미지 = PNG만(호출자가 서명 검사 · 다른 포맷은 imgdec 재인코딩 축이 따로).
pub const IMAGE_MIMES: &[&str] = &["image/png"];

/// Wayland 워커를 띄운다(1회 · 멱등). `display` = winit 창의 `wl_display*`(살아 있는 동안).
/// X11 세션·핸들 부재면 아무것도 하지 않는다(X11 경로는 자기 연결을 쓴다).
///
/// # Safety
/// `display`는 winit이 연 살아 있는 `wl_display*`여야 하고, 프로세스가 끝날 때까지 유효해야 한다
/// (winit 이벤트 루프는 앱 수명과 같다).
pub unsafe fn init_wayland(display: *mut core::ffi::c_void) {
    if display.is_null() {
        return;
    }
    let _ = WL.get_or_init(|| wl::spawn(display as usize));
}

static WL: OnceLock<Option<Arc<wl::Shared>>> = OnceLock::new();

fn wl_shared() -> Option<&'static Arc<wl::Shared>> {
    WL.get().and_then(|o| o.as_ref())
}

/// 클립보드에서 `mimes` 중 첫 가능한 형식의 바이트를 읽는다(Wayland → X11 순).
pub fn get(mimes: &[&str]) -> Option<Vec<u8>> {
    if let Some(s) = wl_shared() {
        // Wayland 세션이면 X11 폴백은 XWayland 클립보드(동기화되지만 이중 경로) — 안 간다.
        return wl::get(s, mimes);
    }
    if std::env::var_os("DISPLAY").is_some() {
        return x11::get(mimes);
    }
    None
}

/// `items`(MIME → 바이트)를 클립보드에 올린다. 성공 = 셀렉션 소유가 성립.
pub fn set(items: Vec<(String, Vec<u8>)>) -> bool {
    if let Some(s) = wl_shared() {
        return wl::set(s, items);
    }
    if std::env::var_os("DISPLAY").is_some() {
        return x11::set(items);
    }
    false
}

/// 자체 경로가 하나라도 살아 있는가(도구 스폰 폴백 판단용).
pub fn native_available() -> bool {
    wl_shared().is_some() || std::env::var_os("DISPLAY").is_some()
}

/// 텍스트 복사 항목(사다리 전부를 광고 — 받는 쪽이 고른다).
pub fn text_items(text: &str) -> Vec<(String, Vec<u8>)> {
    TEXT_MIMES
        .iter()
        .map(|m| ((*m).to_string(), text.as_bytes().to_vec()))
        .collect()
}

/// 선호 순으로 첫 일치 MIME(순수 — 회귀용).
fn pick<'a>(want: &[&'a str], have: &[String]) -> Option<&'a str> {
    want.iter().copied().find(|w| have.iter().any(|h| h == w))
}

/// 파이프 읽기(EOF까지 · 타임아웃·상한) — Wayland `receive` 공용.
fn read_pipe(fd: std::os::fd::OwnedFd) -> Option<Vec<u8>> {
    use std::io::Read as _;
    use std::os::fd::AsRawFd as _;
    let mut f = std::fs::File::from(fd);
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(READ_TIMEOUT_MS as u64);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None; // 원본이 안 쓴다 — 조용히 빈 값으로 성공하지 않는다.
        }
        let mut p = libc::pollfd {
            fd: f.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: 유효한 pollfd 1개.
        let r = unsafe { libc::poll(&mut p, 1, left.as_millis() as i32) };
        if r < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if r == 0 {
            return None;
        }
        match f.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => {
                if out.len() + n > MAX_BYTES {
                    return None;
                }
                out.extend_from_slice(&buf[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

fn make_pipe() -> Option<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd as _;
    let mut fds = [0i32; 2];
    // SAFETY: 길이 2 배열 · O_CLOEXEC(자식에 새지 않게).
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    // SAFETY: 방금 만든 유효한 fd 둘 — 소유권을 넘긴다.
    unsafe {
        Some((
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        ))
    }
}

// ── Wayland ────────────────────────────────────────────────────────────────

mod wl {
    use super::{make_pipe, pick, read_pipe};
    use std::os::fd::AsFd as _;
    use std::sync::{Arc, Mutex};
    use wayland_backend::client::Backend;
    use wayland_client::protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::WlDataDeviceManager,
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_keyboard::{self, WlKeyboard},
        wl_registry,
        wl_seat::{self, WlSeat},
    };
    use wayland_client::{
        delegate_noop, event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum,
    };
    use wayland_protocols::ext::data_control::v1::client::{
        ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
        ext_data_control_manager_v1::ExtDataControlManagerV1,
        ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
        ext_data_control_source_v1::{self, ExtDataControlSourceV1},
    };

    /// 오퍼별 광고 MIME(오퍼 객체의 사용자 데이터 — 컴포지터가 만든 자식 객체).
    #[derive(Default)]
    pub(super) struct OfferData(Mutex<Vec<String>>);

    #[derive(Clone)]
    enum Offer {
        Wl(WlDataOffer),
        Ext(ExtDataControlOfferV1),
    }
    impl Offer {
        fn mimes(&self) -> Vec<String> {
            let d = match self {
                Offer::Wl(o) => o.data::<OfferData>(),
                Offer::Ext(o) => o.data::<OfferData>(),
            };
            d.map(|d| d.0.lock().map(|v| v.clone()).unwrap_or_default())
                .unwrap_or_default()
        }
        fn receive(&self, mime: &str, fd: std::os::fd::BorrowedFd<'_>) {
            match self {
                Offer::Wl(o) => o.receive(mime.to_string(), fd),
                Offer::Ext(o) => o.receive(mime.to_string(), fd),
            }
        }
        fn destroy(&self) {
            match self {
                Offer::Wl(o) => o.destroy(),
                Offer::Ext(o) => o.destroy(),
            }
        }
    }

    #[derive(Clone)]
    enum Source {
        Wl(WlDataSource),
        Ext(ExtDataControlSourceV1),
    }
    impl Source {
        fn destroy(&self) {
            match self {
                Source::Wl(s) => s.destroy(),
                Source::Ext(s) => s.destroy(),
            }
        }
        fn id(&self) -> wayland_backend::client::ObjectId {
            match self {
                Source::Wl(s) => s.id(),
                Source::Ext(s) => s.id(),
            }
        }
    }

    #[derive(Default)]
    struct Inner {
        selection: Option<Offer>,
        serial: u32,
        // 내가 올린 복사본(source 이벤트 `send`가 여기서 읽는다).
        source: Option<Source>,
        copy: Vec<(String, Vec<u8>)>,
        // 바인딩(워커가 채운다).
        wl_mgr: Option<WlDataDeviceManager>,
        wl_dev: Option<WlDataDevice>,
        ext_mgr: Option<ExtDataControlManagerV1>,
        ext_dev: Option<ExtDataControlDeviceV1>,
        seat: Option<WlSeat>,
        keyboard: Option<WlKeyboard>,
        ready: bool,
    }

    pub(super) struct Shared {
        conn: Connection,
        qh: QueueHandle<St>,
        inner: Mutex<Inner>,
    }

    struct St {
        sh: Arc<Shared>,
    }

    fn set_selection(sh: &Shared, new: Option<Offer>) {
        let mut g = sh.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = g.selection.take() {
            old.destroy();
        }
        g.selection = new;
    }

    fn serve_send(sh: &Shared, mime: &str, fd: std::os::fd::OwnedFd) {
        use std::io::Write as _;
        let bytes = {
            let g = sh.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.copy
                .iter()
                .find(|(m, _)| m == mime)
                .map(|(_, b)| b.clone())
        };
        if let Some(b) = bytes {
            let mut f = std::fs::File::from(fd);
            let _ = f.write_all(&b); // 받는 쪽이 닫으면 EPIPE — 무시(SIGPIPE는 프로세스가 ignore).
        }
    }

    fn source_cancelled(sh: &Shared, id: wayland_backend::client::ObjectId) {
        let mut g = sh.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.source.as_ref().is_some_and(|s| s.id() == id) {
            if let Some(s) = g.source.take() {
                s.destroy();
            }
            g.copy.clear();
        }
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for St {
        fn event(
            st: &mut Self,
            reg: &wl_registry::WlRegistry,
            ev: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = ev
            else {
                return;
            };
            let mut g = st.sh.inner.lock().unwrap_or_else(|e| e.into_inner());
            match interface.as_str() {
                "ext_data_control_manager_v1" if g.ext_mgr.is_none() => {
                    g.ext_mgr = Some(reg.bind::<ExtDataControlManagerV1, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    ));
                }
                "wl_data_device_manager" if g.wl_mgr.is_none() => {
                    g.wl_mgr =
                        Some(reg.bind::<WlDataDeviceManager, _, _>(name, version.min(3), qh, ()));
                }
                "wl_seat" if g.seat.is_none() => {
                    g.seat = Some(reg.bind::<WlSeat, _, _>(name, version.min(5), qh, ()));
                }
                _ => {}
            }
        }
    }

    impl Dispatch<WlSeat, ()> for St {
        fn event(
            st: &mut Self,
            seat: &WlSeat,
            ev: wl_seat::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_seat::Event::Capabilities {
                capabilities: WEnum::Value(c),
            } = ev
            {
                let mut g = st.sh.inner.lock().unwrap_or_else(|e| e.into_inner());
                let has = c.contains(wl_seat::Capability::Keyboard);
                if has && g.keyboard.is_none() {
                    g.keyboard = Some(seat.get_keyboard(qh, ()));
                } else if !has {
                    if let Some(k) = g.keyboard.take() {
                        k.release();
                    }
                }
            }
        }
    }

    impl Dispatch<WlKeyboard, ()> for St {
        fn event(
            st: &mut Self,
            _: &WlKeyboard,
            ev: wl_keyboard::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            // 시리얼만 본다(키 값은 보지 않는다 — 봉투 원리 · winit이 입력을 처리한다).
            let serial = match ev {
                wl_keyboard::Event::Enter { serial, .. }
                | wl_keyboard::Event::Leave { serial, .. }
                | wl_keyboard::Event::Key { serial, .. }
                | wl_keyboard::Event::Modifiers { serial, .. } => serial,
                _ => return, // Keymap(fd는 OwnedFd — 드롭으로 닫힘)·RepeatInfo
            };
            st.sh.inner.lock().unwrap_or_else(|e| e.into_inner()).serial = serial;
        }
    }

    impl Dispatch<WlDataDevice, ()> for St {
        fn event(
            st: &mut Self,
            _: &WlDataDevice,
            ev: wl_data_device::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match ev {
                wl_data_device::Event::Selection { id } => {
                    set_selection(&st.sh, id.map(Offer::Wl));
                }
                // DnD 오퍼는 winit 몫 — 우리 큐로 온 사본은 즉시 버린다.
                wl_data_device::Event::Enter { id: Some(o), .. } => o.destroy(),
                _ => {}
            }
        }
        event_created_child!(St, WlDataDevice, [
            wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferData::default()),
        ]);
    }

    impl Dispatch<WlDataOffer, OfferData> for St {
        fn event(
            _: &mut Self,
            _: &WlDataOffer,
            ev: wl_data_offer::Event,
            d: &OfferData,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_data_offer::Event::Offer { mime_type } = ev {
                if let Ok(mut v) = d.0.lock() {
                    if v.len() < 256 {
                        v.push(mime_type);
                    }
                }
            }
        }
    }

    impl Dispatch<WlDataSource, ()> for St {
        fn event(
            st: &mut Self,
            src: &WlDataSource,
            ev: wl_data_source::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match ev {
                wl_data_source::Event::Send { mime_type, fd } => serve_send(&st.sh, &mime_type, fd),
                wl_data_source::Event::Cancelled => source_cancelled(&st.sh, src.id()),
                _ => {}
            }
        }
    }

    impl Dispatch<ExtDataControlDeviceV1, ()> for St {
        fn event(
            st: &mut Self,
            _: &ExtDataControlDeviceV1,
            ev: ext_data_control_device_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match ev {
                ext_data_control_device_v1::Event::Selection { id } => {
                    set_selection(&st.sh, id.map(Offer::Ext));
                }
                // 프라이머리 셀렉션(중클릭)은 쓰지 않는다 — 오퍼만 정리.
                ext_data_control_device_v1::Event::PrimarySelection { id: Some(o) } => o.destroy(),
                _ => {}
            }
        }
        event_created_child!(St, ExtDataControlDeviceV1, [
            ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, OfferData::default()),
        ]);
    }

    impl Dispatch<ExtDataControlOfferV1, OfferData> for St {
        fn event(
            _: &mut Self,
            _: &ExtDataControlOfferV1,
            ev: ext_data_control_offer_v1::Event,
            d: &OfferData,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let ext_data_control_offer_v1::Event::Offer { mime_type } = ev {
                if let Ok(mut v) = d.0.lock() {
                    if v.len() < 256 {
                        v.push(mime_type);
                    }
                }
            }
        }
    }

    impl Dispatch<ExtDataControlSourceV1, ()> for St {
        fn event(
            st: &mut Self,
            src: &ExtDataControlSourceV1,
            ev: ext_data_control_source_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match ev {
                ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                    serve_send(&st.sh, &mime_type, fd)
                }
                ext_data_control_source_v1::Event::Cancelled => source_cancelled(&st.sh, src.id()),
                _ => {}
            }
        }
    }

    delegate_noop!(St: ignore WlDataDeviceManager);
    delegate_noop!(St: ignore ExtDataControlManagerV1);

    /// 워커 기동 — 레지스트리·바인딩까지 동기(roundtrip 2회)로 마친 뒤 상주 디스패치.
    /// 실패(프로토콜 부재·큐 오류) = None(호출자는 X11/도구 폴백).
    pub(super) fn spawn(display: usize) -> Option<Arc<Shared>> {
        // SAFETY: 호출 계약(살아 있는 winit wl_display · 프로세스 수명) — 소유하지 않는 래핑.
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        let conn = Connection::from_backend(backend);
        let mut q = conn.new_event_queue::<St>();
        let qh = q.handle();
        let sh = Arc::new(Shared {
            conn: conn.clone(),
            qh: qh.clone(),
            inner: Mutex::new(Inner::default()),
        });
        let mut st = St { sh: sh.clone() };
        let _reg = conn.display().get_registry(&qh, ());
        // 1차 = 글로벌 목록 · 2차 = seat capabilities(키보드) 도착.
        if q.roundtrip(&mut st).is_err() || q.roundtrip(&mut st).is_err() {
            return None;
        }
        {
            let mut g = sh.inner.lock().unwrap_or_else(|e| e.into_inner());
            let seat = g.seat.clone()?;
            match (g.ext_mgr.clone(), g.wl_mgr.clone()) {
                (Some(m), _) => g.ext_dev = Some(m.get_data_device(&seat, &qh, ())),
                (None, Some(m)) => g.wl_dev = Some(m.get_data_device(&seat, &qh, ())),
                (None, None) => return None,
            }
            g.ready = true;
        }
        let _ = conn.flush();
        std::thread::Builder::new()
            .name("nbeep-wlclip".into())
            .spawn(move || {
                // 상주 디스패치 — 다른 스레드(winit)의 읽기와는 libwayland 큐 규약(prepare_read)으로 공존.
                while q.blocking_dispatch(&mut st).is_ok() {}
            })
            .ok()?;
        Some(sh)
    }

    pub(super) fn get(sh: &Shared, mimes: &[&str]) -> Option<Vec<u8>> {
        let offer = {
            let g = sh.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.selection.clone()?
        };
        let have = offer.mimes();
        let mime = pick(mimes, &have)?;
        let (rd, wr) = make_pipe()?;
        offer.receive(mime, wr.as_fd());
        let _ = sh.conn.flush();
        drop(wr); // 쓰기 끝은 원본 클라이언트만 잡는다 — 우리 쪽을 닫아야 EOF가 온다.
        read_pipe(rd)
    }

    pub(super) fn set(sh: &Shared, items: Vec<(String, Vec<u8>)>) -> bool {
        let mut g = sh.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !g.ready {
            return false;
        }
        if let Some(old) = g.source.take() {
            old.destroy();
        }
        let src = if let (Some(m), Some(d)) = (g.ext_mgr.clone(), g.ext_dev.clone()) {
            let s = m.create_data_source(&sh.qh, ());
            for (mime, _) in &items {
                s.offer(mime.clone());
            }
            d.set_selection(Some(&s));
            Source::Ext(s)
        } else if let (Some(m), Some(d)) = (g.wl_mgr.clone(), g.wl_dev.clone()) {
            if g.serial == 0 {
                return false; // 입력 시리얼 없이는 컴포지터가 거절한다(포커스 전).
            }
            let s = m.create_data_source(&sh.qh, ());
            for (mime, _) in &items {
                s.offer(mime.clone());
            }
            d.set_selection(Some(&s), g.serial);
            Source::Wl(s)
        } else {
            return false;
        };
        g.source = Some(src);
        g.copy = items;
        drop(g);
        sh.conn.flush().is_ok()
    }
}

// ── X11 ────────────────────────────────────────────────────────────────────

mod x11 {
    use super::{pick, MAX_BYTES, READ_TIMEOUT_MS};
    use std::os::fd::AsRawFd as _;
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{self, ConnectionExt as _};
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    struct Atoms {
        clipboard: xproto::Atom,
        targets: xproto::Atom,
        incr: xproto::Atom,
        prop: xproto::Atom,
    }

    fn atom(c: &RustConnection, name: &str) -> Option<xproto::Atom> {
        c.intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom)
    }

    fn atoms(c: &RustConnection) -> Option<Atoms> {
        Some(Atoms {
            clipboard: atom(c, "CLIPBOARD")?,
            targets: atom(c, "TARGETS")?,
            incr: atom(c, "INCR")?,
            prop: atom(c, "NBEEP_CLIP")?,
        })
    }

    fn open() -> Option<(RustConnection, xproto::Window, Atoms)> {
        let (c, scr) = x11rb::connect(None).ok()?;
        let root = c.setup().roots.get(scr)?.root;
        let win = c.generate_id().ok()?;
        c.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            xproto::WindowClass::INPUT_ONLY,
            0,
            &xproto::CreateWindowAux::new().event_mask(xproto::EventMask::PROPERTY_CHANGE),
        )
        .ok()?;
        let a = atoms(&c)?;
        Some((c, win, a))
    }

    /// 이벤트 1개(타임아웃) — poll로 소켓을 기다린 뒤 읽는다.
    fn next_event(c: &RustConnection, deadline: std::time::Instant) -> Option<Event> {
        loop {
            if let Ok(Some(e)) = c.poll_for_event() {
                return Some(e);
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            let mut p = libc::pollfd {
                fd: c.stream().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: 유효한 pollfd 1개.
            let r = unsafe { libc::poll(&mut p, 1, left.as_millis() as i32) };
            if r < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return None;
            }
            if r == 0 {
                return None;
            }
        }
    }

    fn read_prop(
        c: &RustConnection,
        win: xproto::Window,
        prop: xproto::Atom,
    ) -> Option<xproto::GetPropertyReply> {
        c.get_property(true, win, prop, xproto::AtomEnum::ANY, 0, u32::MAX)
            .ok()?
            .reply()
            .ok()
    }

    fn convert(
        c: &RustConnection,
        win: xproto::Window,
        a: &Atoms,
        target: xproto::Atom,
    ) -> Option<Vec<u8>> {
        c.convert_selection(win, a.clipboard, target, a.prop, x11rb::CURRENT_TIME)
            .ok()?;
        c.flush().ok()?;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(READ_TIMEOUT_MS as u64);
        loop {
            match next_event(c, deadline)? {
                Event::SelectionNotify(e) if e.requestor == win => {
                    if e.property == x11rb::NONE {
                        return None;
                    }
                    let r = read_prop(c, win, a.prop)?;
                    if r.type_ != a.incr {
                        return (r.value.len() <= MAX_BYTES).then_some(r.value);
                    }
                    // INCR — 소유자가 조각을 프로퍼티에 올릴 때마다 PropertyNotify(NewValue).
                    let mut out = Vec::new();
                    loop {
                        match next_event(c, deadline)? {
                            Event::PropertyNotify(p)
                                if p.window == win
                                    && p.atom == a.prop
                                    && p.state == xproto::Property::NEW_VALUE =>
                            {
                                let r = read_prop(c, win, a.prop)?;
                                if r.value.is_empty() {
                                    return Some(out);
                                }
                                if out.len() + r.value.len() > MAX_BYTES {
                                    return None;
                                }
                                out.extend_from_slice(&r.value);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn get(mimes: &[&str]) -> Option<Vec<u8>> {
        let (c, win, a) = open()?;
        // TARGETS로 소유자가 주는 형식을 먼저 본다(없는 형식 요청 = 대기만 한다).
        let t = convert(&c, win, &a, a.targets)?;
        let names: Vec<String> = t
            .chunks_exact(4)
            .filter_map(|b| {
                let atom = u32::from_ne_bytes([b[0], b[1], b[2], b[3]]);
                c.get_atom_name(atom).ok()?.reply().ok()
            })
            .map(|r| String::from_utf8_lossy(&r.name).into_owned())
            .collect();
        let mime = pick(mimes, &names)?;
        let target = atom(&c, mime)?;
        let out = convert(&c, win, &a, target)?;
        let _ = c.destroy_window(win);
        let _ = c.flush();
        Some(out)
    }

    /// 복사 = 셀렉션 소유 스레드. 다음 소유자(우리 자신의 재복사 포함)가 나타나면
    /// `SelectionClear`로 끝난다 — 스레드가 누적되지 않는다.
    pub(super) fn set(items: Vec<(String, Vec<u8>)>) -> bool {
        let Some((c, win, a)) = open() else {
            return false;
        };
        if c.set_selection_owner(win, a.clipboard, x11rb::CURRENT_TIME)
            .is_err()
            || c.flush().is_err()
        {
            return false;
        }
        let owner = c
            .get_selection_owner(a.clipboard)
            .ok()
            .and_then(|k| k.reply().ok())
            .map(|r| r.owner);
        if owner != Some(win) {
            return false;
        }
        let atoms: Vec<(xproto::Atom, Vec<u8>)> = items
            .into_iter()
            .filter_map(|(m, b)| Some((atom(&c, &m)?, b)))
            .collect();
        std::thread::Builder::new()
            .name("nbeep-x11clip".into())
            .spawn(move || {
                while let Ok(ev) = c.wait_for_event() {
                    match ev {
                        Event::SelectionClear(e) if e.owner == win => break,
                        Event::SelectionRequest(r) => {
                            let prop = if r.property == x11rb::NONE {
                                r.target
                            } else {
                                r.property
                            };
                            let ok = if r.target == a.targets {
                                let mut list: Vec<u32> = vec![a.targets];
                                list.extend(atoms.iter().map(|(t, _)| *t));
                                c.change_property32(
                                    xproto::PropMode::REPLACE,
                                    r.requestor,
                                    prop,
                                    xproto::AtomEnum::ATOM,
                                    &list,
                                )
                                .is_ok()
                            } else if let Some((_, b)) = atoms.iter().find(|(t, _)| *t == r.target)
                            {
                                c.change_property8(
                                    xproto::PropMode::REPLACE,
                                    r.requestor,
                                    prop,
                                    r.target,
                                    b,
                                )
                                .is_ok()
                            } else {
                                false
                            };
                            let n = xproto::SelectionNotifyEvent {
                                response_type: xproto::SELECTION_NOTIFY_EVENT,
                                sequence: 0,
                                time: r.time,
                                requestor: r.requestor,
                                selection: r.selection,
                                target: r.target,
                                property: if ok { prop } else { x11rb::NONE },
                            };
                            let _ =
                                c.send_event(false, r.requestor, xproto::EventMask::NO_EVENT, n);
                            let _ = c.flush();
                        }
                        _ => {}
                    }
                }
                let _ = c.destroy_window(win);
                let _ = c.flush();
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_prefers_caller_order_over_offer_order() {
        let have = vec!["STRING".to_string(), "text/plain;charset=utf-8".to_string()];
        assert_eq!(pick(TEXT_MIMES, &have), Some("text/plain;charset=utf-8"));
        assert_eq!(pick(IMAGE_MIMES, &have), None);
        let have = vec!["image/png".to_string()];
        assert_eq!(pick(IMAGE_MIMES, &have), Some("image/png"));
    }

    #[test]
    fn text_items_advertise_full_ladder() {
        let it = text_items("한글");
        assert_eq!(it.len(), TEXT_MIMES.len());
        assert!(it.iter().all(|(_, b)| b == "한글".as_bytes()));
    }

    /// 파이프 읽기 — EOF까지 모으고, 닫히지 않으면 타임아웃(None)이다.
    #[test]
    fn read_pipe_collects_until_eof() {
        use std::io::Write as _;
        let (rd, wr) = make_pipe().expect("pipe");
        let mut w = std::fs::File::from(wr);
        w.write_all(b"abc").unwrap();
        drop(w);
        assert_eq!(read_pipe(rd).as_deref(), Some(&b"abc"[..]));
    }
}
