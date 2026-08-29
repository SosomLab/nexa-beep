//! Wayland 정식 창 활성화(08-29 Linux 실기) — 트레이 "열기"·알림 클릭이 창을 **앞으로**.
//!
//! 배경: Wayland 클라이언트는 자기 창을 되살릴 수 없다. `xdg_activation_v1`은 **입력 서열이
//! 실린 토큰**이 있어야 컴포지터가 포커스를 준다 — 앱이 스스로 만든 토큰(winit
//! `request_user_attention`)은 서열이 없어 GNOME이 "앱이 준비되었습니다" 알림으로 강등한다.
//! GNOME appindicator 확장은 트레이 클릭 때 셸이 발급한 토큰을 SNI
//! `ProvideXdgActivationToken`으로 넘겨준다(좌클릭·메뉴 항목 둘 다) → 그 토큰으로
//! `activate(token, wl_surface)`를 부르면 진짜 포커스다.
//!
//! winit에는 외부 토큰을 쓰는 API가 없어 **winit이 연 wl_display를 빌려**(foreign display)
//! 별도 이벤트 큐로 레지스트리에서 `xdg_activation_v1`을 묶고 요청 하나만 보낸다.
//! 봉투 원리: 여기서 보는 것은 display·surface 포인터와 토큰 문자열뿐.

/// `display`·`surface` = winit 창의 raw 핸들(`RawDisplayHandle::Wayland`·`RawWindowHandle::Wayland`).
/// 성공(요청 송신·flush) = true. X11이거나 프로토콜 부재 = false(호출부가 폴백).
///
/// # Safety
/// 두 포인터는 살아 있는 winit 창의 `wl_display*`/`wl_surface*`여야 하고, winit 이벤트 루프
/// 스레드(메인)에서 불러야 한다(libwayland 큐 규약 — 다른 스레드의 읽기와 겹치지 않게).
#[cfg(target_os = "linux")]
pub unsafe fn activate(
    display: *mut core::ffi::c_void,
    surface: *mut core::ffi::c_void,
    token: &str,
) -> bool {
    imp::activate(display, surface, token)
}

/// 비-Linux 스텁 — 항상 false(호출부가 폴백).
///
/// # Safety
/// 포인터를 읽지 않는다(스텁) — 계약은 Linux 판과 동일하게 유지한다(호출부 cfg 분기 방지).
#[cfg(not(target_os = "linux"))]
pub unsafe fn activate(
    _display: *mut core::ffi::c_void,
    _surface: *mut core::ffi::c_void,
    _token: &str,
) -> bool {
    false
}

#[cfg(target_os = "linux")]
mod imp {
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{
        self, XdgActivationV1,
    };

    #[derive(Default)]
    struct St {
        act: Option<XdgActivationV1>,
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
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = ev
            {
                if interface == "xdg_activation_v1" && st.act.is_none() {
                    st.act = Some(reg.bind::<XdgActivationV1, _, _>(name, version.min(1), qh, ()));
                }
            }
        }
    }

    impl Dispatch<XdgActivationV1, ()> for St {
        fn event(
            _: &mut Self,
            _: &XdgActivationV1,
            _: xdg_activation_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    pub(super) unsafe fn activate(
        display: *mut core::ffi::c_void,
        surface: *mut core::ffi::c_void,
        token: &str,
    ) -> bool {
        if display.is_null() || surface.is_null() || token.is_empty() {
            return false;
        }
        // SAFETY: 호출 계약(살아 있는 winit wl_display) — 소유하지 않는 외부 디스플레이 래핑.
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let conn = Connection::from_backend(backend);
        let mut q = conn.new_event_queue::<St>();
        let qh = q.handle();
        let _reg = conn.display().get_registry(&qh, ());
        let mut st = St::default();
        if q.roundtrip(&mut st).is_err() {
            return false;
        }
        let Some(act) = st.act else { return false };
        // SAFETY: 호출 계약(살아 있는 winit wl_surface) — 외부 프록시 래핑(요청 전용).
        let Ok(id) = (unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }) else {
            act.destroy();
            return false;
        };
        let Ok(surf) = WlSurface::from_id(&conn, id) else {
            act.destroy();
            return false;
        };
        act.activate(token.to_string(), &surf);
        act.destroy();
        conn.flush().is_ok()
    }
}
