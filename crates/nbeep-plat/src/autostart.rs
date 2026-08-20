//! **로그인 시 자동 실행** — OS별 사용자 수준 등록(설정 `app.autostart` · 기본 on).
//!
//! 전부 **T0(무권한)** 경로다(DR-1·DR-4 — 관리자 권한·설치자를 요구하는 순간 정체성이
//! 깨진다): Windows = HKCU `Run` 레지스트리 값 · macOS = `~/Library/LaunchAgents` plist ·
//! Linux = XDG `~/.config/autostart` `.desktop`. 외부 크레이트 없음(레지스트리는
//! advapi32 직접 링크 — launch.rs kernel32와 같은 규약).
//!
//! **경로는 부팅마다 재동기화한다**(호출자 = `apply_boot_settings`) — 포터블 채널(DR-4)은
//! 실행 파일 위치가 옮겨질 수 있어, 등록된 옛 경로가 조용히 죽는다. 켜져 있으면 현재
//! `current_exe()`로 덮어쓰고, 꺼져 있으면 등록을 걷어낸다(멱등 — 없으면 no-op).

use std::io;
use std::path::Path;

/// Windows Run 값 이름.
#[cfg(windows)]
const APP_NAME: &str = "NexaBeep";
/// macOS LaunchAgent 라벨(역DNS — 조직 규약).
#[cfg(any(target_os = "macos", test))]
const MAC_LABEL: &str = "com.sosomlab.nexa-beep";

/// 자동 실행 등록을 **설정값과 동기화**한다 — `enabled`면 현재 실행 파일 경로로
/// 등록(덮어쓰기), 아니면 등록 제거(없어도 성공). 실패는 호출자가 화면에 알린다
/// (조용한 실패 금지 — 설정값 자체는 유지돼 다음 부팅·토글에서 재시도된다).
pub fn apply(enabled: bool) -> io::Result<()> {
    if enabled {
        let exe = std::env::current_exe()?;
        register(&exe)
    } else {
        unregister()
    }
}

// ── Windows — HKCU\…\Run 값(레지스트리 · 재부팅 불요·즉시 유효) ──────────────

#[cfg(windows)]
fn register(exe: &Path) -> io::Result<()> {
    // 경로에 공백이 있어도 하나의 명령으로 읽히게 따옴표로 감싼다.
    let cmd = format!("\"{}\"", exe.display());
    reg::set_run_value(APP_NAME, &cmd)
}

#[cfg(windows)]
fn unregister() -> io::Result<()> {
    reg::delete_run_value(APP_NAME)
}

#[cfg(windows)]
mod reg {
    //! advapi32 직접 링크(의존 0 규약 — launch.rs `#[link]`와 동형). HKCU 한정이라
    //! 권한 상승이 없다.
    use std::io;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            hkey: isize,
            sub_key: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            sam_desired: u32,
            security: *const core::ffi::c_void,
            result: *mut isize,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            hkey: isize,
            value_name: *const u16,
            reserved: u32,
            value_type: u32,
            data: *const u8,
            data_len: u32,
        ) -> i32;
        fn RegDeleteValueW(hkey: isize, value_name: *const u16) -> i32;
        fn RegCloseKey(hkey: isize) -> i32;
    }

    // 핸들 상수는 부호 확장된 포인터 값(winreg.h) — u32 → i32 → isize 순으로 굳힌다.
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_SZ: u32 = 1;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }

    /// Run 키를 열어(없으면 생성 — HKCU라 항상 가능해야 정상) 닫힘 보장과 함께
    /// `f`를 수행한다.
    fn with_run_key(f: impl FnOnce(isize) -> i32) -> io::Result<()> {
        let sub = wide(RUN_KEY);
        let mut hkey: isize = 0;
        // SAFETY: 유효한 널 종료 wide 문자열과 출력 핸들 포인터만 넘긴다. 성공 시
        // 핸들은 아래에서 반드시 RegCloseKey로 닫는다.
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                std::ptr::null(),
                0, // REG_OPTION_NON_VOLATILE
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut hkey,
                std::ptr::null_mut(),
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(rc));
        }
        let rc = f(hkey);
        // SAFETY: 위에서 성공적으로 연 핸들.
        unsafe { RegCloseKey(hkey) };
        match rc {
            ERROR_SUCCESS => Ok(()),
            // 삭제 대상 부재 = 이미 원하는 상태(멱등).
            ERROR_FILE_NOT_FOUND => Ok(()),
            e => Err(io::Error::from_raw_os_error(e)),
        }
    }

    pub(super) fn set_run_value(name: &str, cmd: &str) -> io::Result<()> {
        let name_w = wide(name);
        let data = wide(cmd);
        with_run_key(|hkey| {
            let bytes = data.len() * 2; // UTF-16 단위 → 바이트(널 포함)
            // SAFETY: REG_SZ 데이터로 널 종료 wide 버퍼와 그 바이트 길이를 넘긴다.
            unsafe {
                RegSetValueExW(
                    hkey,
                    name_w.as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr().cast(),
                    bytes as u32,
                )
            }
        })
    }

    pub(super) fn delete_run_value(name: &str) -> io::Result<()> {
        let name_w = wide(name);
        // SAFETY: 유효한 키 핸들과 널 종료 값 이름.
        with_run_key(|hkey| unsafe { RegDeleteValueW(hkey, name_w.as_ptr()) })
    }
}

// ── macOS — ~/Library/LaunchAgents plist(RunAtLoad · 무권한) ─────────────────

#[cfg(target_os = "macos")]
fn register(exe: &Path) -> io::Result<()> {
    let f = mac_plist_path()?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&f, plist_content(&exe.display().to_string()))
}

#[cfg(target_os = "macos")]
fn unregister() -> io::Result<()> {
    remove_if_exists(&mac_plist_path()?)
}

#[cfg(target_os = "macos")]
fn mac_plist_path() -> io::Result<std::path::PathBuf> {
    let home = crate::paths::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{MAC_LABEL}.plist")))
}

/// launchd LaunchAgent — `RunAtLoad`가 로그인 시 1회 실행. 경로는 XML 텍스트
/// 노드라 이스케이프 필수(사용자 폴더 이름은 임의 문자열이다).
/// (빌더는 순수 함수 — 계약 테스트는 전 OS에서 돈다.)
#[cfg(any(target_os = "macos", test))]
fn plist_content(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{MAC_LABEL}</string>
    <key>ProgramArguments</key>
    <array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
        xml_escape(exe)
    )
}

#[cfg(any(target_os = "macos", test))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ── Linux — XDG autostart .desktop(무권한 · 데스크톱 환경 공통) ──────────────

#[cfg(all(unix, not(target_os = "macos")))]
fn register(exe: &Path) -> io::Result<()> {
    let f = linux_desktop_path()?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&f, desktop_content(&exe.display().to_string()))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unregister() -> io::Result<()> {
    remove_if_exists(&linux_desktop_path()?)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_desktop_path() -> io::Result<std::path::PathBuf> {
    // XDG 규약 — 설정 홈이 지정돼 있으면 그쪽, 아니면 ~/.config.
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| crate::paths::home_dir().map(|h| h.join(".config")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    Ok(base.join("autostart").join("nexa-beep.desktop"))
}

/// freedesktop autostart 항목. `Exec`는 셸이 아니라 필드 코드 파서가 읽는다 —
/// 공백 대비 상시 따옴표 + 예약 문자 백슬래시 이스케이프.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn desktop_content(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Nexa Beep\nExec={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exec_quote(exe)
    )
}

#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn exec_quote(exe: &str) -> String {
    let escaped: String = exe
        .chars()
        .map(|c| match c {
            '"' | '\\' | '$' | '`' => format!("\\{c}"),
            _ => c.to_string(),
        })
        .collect();
    format!("\"{escaped}\"")
}

#[cfg(unix)]
fn remove_if_exists(f: &Path) -> io::Result<()> {
    match std::fs::remove_file(f) {
        Ok(()) => Ok(()),
        // 부재 = 이미 원하는 상태(멱등).
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

// 미지 타깃 — 컴파일은 되게, 동작은 정직하게(지원 안 함).
#[cfg(not(any(windows, unix)))]
fn register(_exe: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "autostart not supported on this platform",
    ))
}

#[cfg(not(any(windows, unix)))]
fn unregister() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 콘텐츠 빌더는 순수 함수 — OS 상태를 건드리지 않고 계약을 박제한다.
    // (레지스트리·홈 폴더 변조는 테스트에서 하지 않는다 — 사용자 환경이다.)

    #[test]
    fn plist_escapes_xml_and_carries_label() {
        let p = plist_content("/Users/a&b/nexa <beep>");
        assert!(p.contains("<string>/Users/a&amp;b/nexa &lt;beep&gt;</string>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>"));
        assert!(p.contains(MAC_LABEL));
    }

    #[test]
    fn desktop_quotes_exec_with_spaces_and_reserved() {
        let d = desktop_content(r#"/opt/my apps/nexa"beep"#);
        assert!(d.contains("Exec=\"/opt/my apps/nexa\\\"beep\""));
        assert!(d.starts_with("[Desktop Entry]\n"));
        assert!(d.contains("X-GNOME-Autostart-enabled=true"));
    }

    #[test]
    fn exec_quote_escapes_shell_reserved() {
        assert_eq!(exec_quote(r"/a/$b`c\d"), "\"/a/\\$b\\`c\\\\d\"");
    }

    #[test]
    fn xml_escape_passes_plain_paths_through() {
        assert_eq!(xml_escape("/usr/local/bin/nexa-beep"), "/usr/local/bin/nexa-beep");
    }
}
