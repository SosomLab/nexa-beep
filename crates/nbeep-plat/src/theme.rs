//! 시스템 테마(밝기) 조회 — `ui.theme = system`(08-29 사용자 요청 · Linux 실기).
//!
//! 봉투 원리: 여기서는 **"OS가 다크를 선호하는가"** 한 비트만 본다. 색·팔레트는 앱 몫.
//!
//! | OS | 조회 | 변경 감시 |
//! |---|---|---|
//! | Linux | xdg-desktop-portal `Settings.ReadOne(appearance/color-scheme)` (1=다크·2=라이트·0=무선호) → 폴백 `gsettings color-scheme` | 포털 `SettingChanged` 신호(zbus 스레드 → 콜백) |
//! | Windows | HKCU `Themes\Personalize\AppsUseLightTheme` DWORD(0 = 다크) | winit `WindowEvent::ThemeChanged` |
//! | macOS | `defaults read -g AppleInterfaceStyle`(= "Dark"면 다크 · 키 부재 = 라이트) | winit `WindowEvent::ThemeChanged` |
//!
//! 전부 **무권한·의존 추가 0**(zbus는 트레이가 이미 쓴다 · Windows는 advapi32 직접).
//! 판정 불가 = `None`(앱이 기본값을 고른다 — fail-soft).

/// OS가 다크 테마를 선호하면 `Some(true)`, 라이트면 `Some(false)`, 모르면 `None`.
pub fn system_prefers_dark() -> Option<bool> {
    imp::prefers_dark()
}

/// 시스템 테마 변경 감시 — 바뀔 때마다 `cb(새 판정)`. Linux(포털 신호)만 실물이고
/// 다른 OS는 no-op(winit `ThemeChanged`가 그 역할 — 창 이벤트라 여기 둘 게 없다).
pub fn watch<F: Fn(Option<bool>) + Send + 'static>(cb: F) {
    imp::watch(cb);
}

#[cfg(target_os = "linux")]
mod imp {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Value};

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const IFACE: &str = "org.freedesktop.portal.Settings";

    /// 포털 값 → 판정(1 = 다크 · 2 = 라이트 · 그 외 = 무선호).
    fn from_scheme(v: u32) -> Option<bool> {
        match v {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        }
    }

    /// `ReadOne`/`Read` 응답은 variant 안에 variant가 한 겹 더 있을 수 있다 — 벗겨 u32로.
    fn unwrap_u32(v: &Value<'_>) -> Option<u32> {
        match v {
            Value::U32(n) => Some(*n),
            Value::Value(inner) => unwrap_u32(inner),
            _ => None,
        }
    }

    fn portal_read() -> Option<bool> {
        let conn = Connection::session().ok()?;
        let proxy = Proxy::new(&conn, PORTAL, PATH, IFACE).ok()?;
        // ReadOne(포털 v2) → 실패 시 Read(구판 · (v) 이중 variant).
        let v: OwnedValue = proxy
            .call("ReadOne", &("org.freedesktop.appearance", "color-scheme"))
            .or_else(|_| proxy.call("Read", &("org.freedesktop.appearance", "color-scheme")))
            .ok()?;
        unwrap_u32(&v).and_then(from_scheme)
    }

    fn gsettings_read() -> Option<bool> {
        let out = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        if s.contains("prefer-dark") {
            Some(true)
        } else if s.contains("prefer-light") {
            Some(false)
        } else {
            None
        }
    }

    pub(super) fn prefers_dark() -> Option<bool> {
        portal_read().or_else(gsettings_read)
    }

    pub(super) fn watch<F: Fn(Option<bool>) + Send + 'static>(cb: F) {
        std::thread::Builder::new()
            .name("nb-theme-watch".into())
            .spawn(move || {
                let Ok(conn) = Connection::session() else {
                    return;
                };
                let Ok(proxy) = Proxy::new(&conn, PORTAL, PATH, IFACE) else {
                    return;
                };
                let Ok(sigs) = proxy.receive_signal("SettingChanged") else {
                    return;
                };
                for msg in sigs {
                    // 시그니처 (ssv): namespace · key · value.
                    let body = msg.body();
                    let Ok((ns, key, val)) = body.deserialize::<(String, String, Value<'_>)>()
                    else {
                        continue;
                    };
                    if ns == "org.freedesktop.appearance" && key == "color-scheme" {
                        cb(unwrap_u32(&val).and_then(from_scheme));
                    }
                }
            })
            .ok();
    }
}

#[cfg(windows)]
mod imp {
    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            hkey: isize,
            sub_key: *const u16,
            options: u32,
            sam_desired: u32,
            result: *mut isize,
        ) -> i32;
        fn RegQueryValueExW(
            hkey: isize,
            value_name: *const u16,
            reserved: *mut u32,
            value_type: *mut u32,
            data: *mut u8,
            data_len: *mut u32,
        ) -> i32;
        fn RegCloseKey(hkey: isize) -> i32;
    }
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const REG_DWORD: u32 = 4;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn prefers_dark() -> Option<bool> {
        let sub = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let name = wide("AppsUseLightTheme");
        let mut hkey: isize = 0;
        // SAFETY: 널 종료 wide 문자열 + 출력 핸들. 성공 시 아래에서 닫는다.
        if unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut hkey,
            )
        } != 0
        {
            return None;
        }
        let mut ty = 0u32;
        let mut data = [0u8; 4];
        let mut len = 4u32;
        // SAFETY: 열린 핸들 · 4바이트 버퍼와 길이 포인터 유효.
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut ty,
                data.as_mut_ptr(),
                &mut len,
            )
        };
        // SAFETY: 위에서 연 핸들.
        unsafe { RegCloseKey(hkey) };
        if rc != 0 || ty != REG_DWORD || len != 4 {
            return None;
        }
        Some(u32::from_le_bytes(data) == 0) // 0 = 앱 다크 모드
    }

    pub(super) fn watch<F: Fn(Option<bool>) + Send + 'static>(_cb: F) {}
}

#[cfg(target_os = "macos")]
mod imp {
    pub(super) fn prefers_dark() -> Option<bool> {
        let out = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .ok()?;
        // 키가 없으면 rc≠0(= 라이트) · "Dark"면 다크.
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim() == "Dark")
        } else {
            Some(false)
        }
    }

    pub(super) fn watch<F: Fn(Option<bool>) + Send + 'static>(_cb: F) {}
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod imp {
    pub(super) fn prefers_dark() -> Option<bool> {
        None
    }
    pub(super) fn watch<F: Fn(Option<bool>) + Send + 'static>(_cb: F) {}
}

#[cfg(test)]
mod tests {
    /// 판정 함수는 어느 환경에서도 패닉 없이 돌아온다(값은 환경 의존이라 단정 안 함).
    #[test]
    fn prefers_dark_never_panics() {
        let _ = super::system_prefers_dark();
    }

    /// (Linux 실기 전용 · `--ignored`) gsettings로 색 구성을 뒤집으면 포털 신호가 콜백으로
    /// 도달한다 — 08-29 실측용. 끝나면 원래 값으로 되돌린다.
    #[test]
    #[ignore = "실 GNOME 세션 필요 — cargo test -p nbeep-plat -- --ignored theme_watch"]
    #[cfg(target_os = "linux")]
    fn theme_watch_receives_portal_signal() {
        use std::process::Command;
        use std::sync::mpsc;
        let get = || {
            String::from_utf8_lossy(
                &Command::new("gsettings")
                    .args(["get", "org.gnome.desktop.interface", "color-scheme"])
                    .output()
                    .unwrap()
                    .stdout,
            )
            .trim()
            .to_string()
        };
        let set = |v: &str| {
            Command::new("gsettings")
                .args(["set", "org.gnome.desktop.interface", "color-scheme", v])
                .status()
                .unwrap()
        };
        let orig = get();
        let (tx, rx) = mpsc::channel();
        super::watch(move |d| {
            let _ = tx.send(d);
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        let flip = if orig.contains("prefer-dark") {
            "'default'"
        } else {
            "'prefer-dark'"
        };
        set(flip);
        let got = rx.recv_timeout(std::time::Duration::from_secs(3));
        set(&orig);
        let now = super::system_prefers_dark();
        eprintln!("orig={orig} flip={flip} signal={got:?} after_restore={now:?}");
        assert!(got.is_ok(), "포털 SettingChanged 신호 미도달");
        let expect_dark = flip.contains("prefer-dark");
        assert_eq!(got.unwrap(), Some(expect_dark));
    }
}
