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
//!
//! ★ **인스턴스별 슬롯(08-29)** — 등록 이름에 **실행 파일 경로 해시**를 붙인다
//! (`NexaBeep-<tag>` · `com.sosomlab.nexa-beep.<tag>` · `nexa-beep-<tag>.desktop`).
//! 종전엔 3-OS 전부 이름이 하나라, 같은 PC의 여러 인스턴스(설치본 + 포터블·다중
//! 신원 — 신원은 실행 파일 옆 `data/`에 붙으니 **인스턴스 = 실행 파일 경로**)가
//! OS 슬롯 하나를 두고 줄다리기했다: 설정 off인 개발 빌드가 부팅 동기화에서 설치본의
//! 등록을 지우고, 다음 인스턴스는 그 부재를 "외부 삭제"로 오판해 설정까지 내리는
//! 도미노(Linux 실기 08-29). 이제 각 인스턴스는 **자기 슬롯만** 관측·등록·삭제한다.
//! 옛 고정 이름 등록은 그 내용이 내 실행 파일을 가리킬 때만 내 것으로 인정해 걷어낸다
//! (1회 이관) — 남의 것이면 손대지 않는다.

use std::io;
use std::path::Path;

/// Windows Run 값 이름(접두 — 뒤에 `-<tag>`).
#[cfg(any(windows, test))]
const APP_NAME: &str = "NexaBeep";
/// macOS LaunchAgent 라벨(역DNS — 조직 규약 · 뒤에 `.<tag>`).
#[cfg(any(target_os = "macos", test))]
const MAC_LABEL: &str = "com.sosomlab.nexa-beep";

/// 실행 파일 경로 → 슬롯 태그(FNV-1a 64 · 16 hex). 경로가 곧 인스턴스다(포터블 =
/// 실행 파일 옆 `data/` · 설치본 = 기본 폴더 — 둘 다 경로로 갈린다). 외부 의존 0.
pub fn instance_tag(exe: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in exe.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// 등록 내용에 담긴 실행 파일 경로가 `exe`와 같은가(옛 고정 이름 등록의 소유 판정).
/// 이름 하나를 여럿이 쓰던 시절의 등록은 **내 경로를 가리킬 때만** 내 것이다.
fn owned_by(content_path: Option<String>, exe: &Path) -> bool {
    content_path.is_some_and(|p| Path::new(&p) == exe)
}

/// 자동 실행 등록을 **설정값과 동기화**한다 — `enabled`면 현재 실행 파일 경로로
/// 등록(덮어쓰기), 아니면 등록 제거(없어도 성공). 실패는 호출자가 화면에 알린다
/// (조용한 실패 금지 — 설정값 자체는 유지돼 다음 부팅·토글에서 재시도된다).
pub fn apply(enabled: bool) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    if enabled {
        register(&exe)
    } else {
        unregister(&exe)
    }
}

/// 자동 실행 등록이 OS에 **실재하는지** 관측한다(설정값과 무관) — 사용자는
/// 레지스트리 편집기·정리 도구로 앱 밖에서 등록을 지울 수 있고, 그 의사는
/// 존중해야 한다(무조건 재등록 = 사용자와 앱의 줄다리기). 미지 타깃은 false.
pub fn is_registered() -> bool {
    std::env::current_exe()
        .map(|exe| os_registered(&exe))
        .unwrap_or(false)
}

/// 부팅 동기화 판정(순수) — 설정값·"등록해 둔 적 있음" 마커·OS 관측의 곱.
/// 외부 삭제 존중은 **마커가 있을 때만**: 첫 실행·마커 없는 구버전 업그레이드는
/// 등록 부재가 삭제가 아니라 "아직"이므로 재등록이 맞다.
#[derive(Debug, PartialEq, Eq)]
pub enum BootSync {
    /// 켬 — 현재 경로로 (재)등록한다(포터블 이동 재동기화 포함).
    Register,
    /// 끔 — 등록을 걷어낸다(멱등).
    Unregister,
    /// 켬인데 등록해 둔 것이 밖에서 사라졌다 — 재등록하지 않는다.
    /// 호출자는 설정을 끔으로 내려 화면과 실제를 일치시킨다.
    RespectRemoval,
}

pub fn boot_sync(want_on: bool, was_registered: bool, os_registered: bool) -> BootSync {
    if !want_on {
        BootSync::Unregister
    } else if was_registered && !os_registered {
        BootSync::RespectRemoval
    } else {
        BootSync::Register
    }
}

// ── Windows — HKCU\…\Run 값(레지스트리 · 재부팅 불요·즉시 유효) ──────────────

#[cfg(any(windows, test))]
fn win_value_name(exe: &Path) -> String {
    format!("{APP_NAME}-{}", instance_tag(exe))
}

/// Run 값 데이터(`"C:\\…\\nexa-beep.exe"`) → 경로. 따옴표는 register가 감싼 것.
#[cfg(any(windows, test))]
fn win_cmd_path(cmd: &str) -> String {
    cmd.trim().trim_matches('"').to_string()
}

#[cfg(windows)]
fn register(exe: &Path) -> io::Result<()> {
    // 경로에 공백이 있어도 하나의 명령으로 읽히게 따옴표로 감싼다.
    let cmd = format!("\"{}\"", exe.display());
    reg::set_run_value(&win_value_name(exe), &cmd)?;
    // 옛 고정 이름이 내 경로를 가리키면 1회 이관(남의 것이면 보존).
    if owned_by(reg::run_value(APP_NAME).map(|c| win_cmd_path(&c)), exe) {
        reg::delete_run_value(APP_NAME)?;
    }
    Ok(())
}

#[cfg(windows)]
fn unregister(exe: &Path) -> io::Result<()> {
    reg::delete_run_value(&win_value_name(exe))?;
    if owned_by(reg::run_value(APP_NAME).map(|c| win_cmd_path(&c)), exe) {
        reg::delete_run_value(APP_NAME)?;
    }
    Ok(())
}

#[cfg(windows)]
fn os_registered(exe: &Path) -> bool {
    reg::has_run_value(&win_value_name(exe))
        || owned_by(reg::run_value(APP_NAME).map(|c| win_cmd_path(&c)), exe)
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

    // 핸들 상수는 부호 확장된 포인터 값(winreg.h) — u32 → i32 → isize 순으로 굳힌다.
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_QUERY_VALUE: u32 = 0x0001;
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

    /// 값 **읽기**(REG_SZ 문자열) — 부재·오류·비문자열 = None. 옛 고정 이름 등록의
    /// 소유 판정용(내 경로를 가리키는가).
    pub(super) fn run_value(name: &str) -> Option<String> {
        let sub = wide(RUN_KEY);
        let name_w = wide(name);
        let mut hkey: isize = 0;
        // SAFETY: 유효한 널 종료 wide 문자열과 출력 핸들 포인터. 성공 시 아래에서 닫는다.
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut hkey,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        // 1차 = 길이 조회, 2차 = 데이터 수신(그 사이 커진 값은 오류 → None).
        let mut ty: u32 = 0;
        let mut len: u32 = 0;
        // SAFETY: 열린 키 핸들 + 널 종료 값 이름 · 데이터 NULL = 길이만 받는다.
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name_w.as_ptr(),
                std::ptr::null_mut(),
                &mut ty,
                std::ptr::null_mut(),
                &mut len,
            )
        };
        let out = if rc == ERROR_SUCCESS && ty == REG_SZ && (2..=1 << 16).contains(&len) {
            let mut buf = vec![0u16; (len as usize).div_ceil(2)];
            // SAFETY: buf는 len 바이트 이상이고 len은 buf 크기를 넘지 않는다.
            let rc = unsafe {
                RegQueryValueExW(
                    hkey,
                    name_w.as_ptr(),
                    std::ptr::null_mut(),
                    &mut ty,
                    buf.as_mut_ptr().cast(),
                    &mut len,
                )
            };
            (rc == ERROR_SUCCESS).then(|| {
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                String::from_utf16_lossy(&buf[..n])
            })
        } else {
            None
        };
        // SAFETY: 위에서 성공적으로 연 핸들.
        unsafe { RegCloseKey(hkey) };
        out
    }

    /// 값 **존재 관측**(읽기 전용 — 데이터는 안 받는다: lpData·lpcbData 둘 다
    /// NULL이면 존재 시 ERROR_SUCCESS만 돌아온다). 키·값 부재·오류 = false.
    pub(super) fn has_run_value(name: &str) -> bool {
        let sub = wide(RUN_KEY);
        let name_w = wide(name);
        let mut hkey: isize = 0;
        // SAFETY: 유효한 널 종료 wide 문자열과 출력 핸들 포인터. 성공 시 아래에서 닫는다.
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                sub.as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut hkey,
            )
        };
        if rc != ERROR_SUCCESS {
            return false;
        }
        // SAFETY: 열린 키 핸들 + 널 종료 값 이름 · 출력은 전부 NULL(존재 판정만).
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name_w.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        // SAFETY: 위에서 성공적으로 연 핸들.
        unsafe { RegCloseKey(hkey) };
        rc == ERROR_SUCCESS
    }
}

// ── macOS — ~/Library/LaunchAgents plist(RunAtLoad · 무권한) ─────────────────

#[cfg(target_os = "macos")]
fn register(exe: &Path) -> io::Result<()> {
    let label = mac_label(exe);
    let f = mac_plist_path(&label)?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&f, plist_content(&label, &exe.display().to_string()))?;
    remove_legacy_if_mine(&mac_plist_path(MAC_LABEL)?, plist_exe, exe)
}

#[cfg(target_os = "macos")]
fn unregister(exe: &Path) -> io::Result<()> {
    remove_if_exists(&mac_plist_path(&mac_label(exe))?)?;
    remove_legacy_if_mine(&mac_plist_path(MAC_LABEL)?, plist_exe, exe)
}

#[cfg(target_os = "macos")]
fn os_registered(exe: &Path) -> bool {
    mac_plist_path(&mac_label(exe)).is_ok_and(|p| p.is_file())
        || mac_plist_path(MAC_LABEL).is_ok_and(|p| legacy_is_mine(&p, plist_exe, exe))
}

#[cfg(any(target_os = "macos", test))]
fn mac_label(exe: &Path) -> String {
    format!("{MAC_LABEL}.{}", instance_tag(exe))
}

#[cfg(target_os = "macos")]
fn mac_plist_path(label: &str) -> io::Result<std::path::PathBuf> {
    let home = crate::paths::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist")))
}

/// plist 본문 → ProgramArguments 첫 문자열(우리가 쓴 형식만 — 다른 형식은 None).
#[cfg(any(target_os = "macos", test))]
fn plist_exe(content: &str) -> Option<String> {
    let rest = content.split("<key>ProgramArguments</key>").nth(1)?;
    let s = rest.split("<string>").nth(1)?.split("</string>").next()?;
    Some(xml_unescape(s))
}

#[cfg(any(target_os = "macos", test))]
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// launchd LaunchAgent — `RunAtLoad`가 로그인 시 1회 실행. 경로는 XML 텍스트
/// 노드라 이스케이프 필수(사용자 폴더 이름은 임의 문자열이다).
/// (빌더는 순수 함수 — 계약 테스트는 전 OS에서 돈다.)
#[cfg(any(target_os = "macos", test))]
fn plist_content(label: &str, exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>{label}</string>
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
    let f = linux_desktop_path(&linux_file_name(exe))?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&f, desktop_content(&exe.display().to_string()))?;
    remove_legacy_if_mine(&linux_desktop_path(LEGACY_DESKTOP)?, desktop_exe, exe)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unregister(exe: &Path) -> io::Result<()> {
    remove_if_exists(&linux_desktop_path(&linux_file_name(exe))?)?;
    remove_legacy_if_mine(&linux_desktop_path(LEGACY_DESKTOP)?, desktop_exe, exe)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn os_registered(exe: &Path) -> bool {
    linux_desktop_path(&linux_file_name(exe)).is_ok_and(|p| p.is_file())
        || linux_desktop_path(LEGACY_DESKTOP).is_ok_and(|p| legacy_is_mine(&p, desktop_exe, exe))
}

/// 옛 고정 파일명(인스턴스 슬롯 도입 전).
#[cfg(any(all(unix, not(target_os = "macos")), test))]
const LEGACY_DESKTOP: &str = "nexa-beep.desktop";

#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn linux_file_name(exe: &Path) -> String {
    format!("nexa-beep-{}.desktop", instance_tag(exe))
}

/// `.desktop` 본문 → `Exec=` 경로(우리가 쓴 형식 — 따옴표·백슬래시 이스케이프 역변환).
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn desktop_exe(content: &str) -> Option<String> {
    let line = content
        .lines()
        .find_map(|l| l.strip_prefix("Exec="))?
        .trim();
    let inner = line.strip_prefix('"').and_then(|l| l.strip_suffix('"'))?;
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => out.push(it.next()?),
            _ => out.push(c),
        }
    }
    Some(out)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_desktop_path(file: &str) -> io::Result<std::path::PathBuf> {
    // XDG 규약 — 설정 홈이 지정돼 있으면 그쪽, 아니면 ~/.config.
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| crate::paths::home_dir().map(|h| h.join(".config")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    Ok(base.join("autostart").join(file))
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

/// 옛 고정 이름 파일이 **내 실행 파일**을 가리키는가(없음·못 읽음·남의 것 = false).
#[cfg(unix)]
fn legacy_is_mine(f: &Path, parse: fn(&str) -> Option<String>, exe: &Path) -> bool {
    std::fs::read_to_string(f)
        .ok()
        .is_some_and(|c| owned_by(parse(&c), exe))
}

/// 옛 고정 이름 등록이 내 것이면 걷어낸다(슬롯 이관 · 남의 것이면 보존).
#[cfg(unix)]
fn remove_legacy_if_mine(
    f: &Path,
    parse: fn(&str) -> Option<String>,
    exe: &Path,
) -> io::Result<()> {
    if legacy_is_mine(f, parse, exe) {
        remove_if_exists(f)
    } else {
        Ok(())
    }
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
fn unregister(_exe: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn os_registered(_exe: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // 콘텐츠 빌더는 순수 함수 — OS 상태를 건드리지 않고 계약을 박제한다.
    // (레지스트리·홈 폴더 변조는 테스트에서 하지 않는다 — 사용자 환경이다.)

    #[test]
    fn plist_escapes_xml_and_carries_label() {
        let label = mac_label(Path::new("/Users/a&b/nexa <beep>"));
        let p = plist_content(&label, "/Users/a&b/nexa <beep>");
        assert!(p.contains("<string>/Users/a&amp;b/nexa &lt;beep&gt;</string>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>"));
        assert!(p.contains(&format!("<string>{label}</string>")));
        // 본문 → 경로 역변환(옛 등록 소유 판정의 근거).
        assert_eq!(plist_exe(&p).as_deref(), Some("/Users/a&b/nexa <beep>"));
    }

    // 인스턴스 슬롯 — 경로가 다르면 이름이 다르고(줄다리기 없음), 같으면 안정.
    #[test]
    fn instance_tag_is_stable_and_path_distinct() {
        let a = Path::new("/usr/bin/nexa-beep");
        let b = Path::new("/home/u/.nexa-beep-multi/A/nexa-beep");
        assert_eq!(instance_tag(a), instance_tag(a));
        assert_ne!(instance_tag(a), instance_tag(b));
        assert_eq!(instance_tag(a).len(), 16);
        assert_ne!(linux_file_name(a), linux_file_name(b));
        assert_ne!(mac_label(a), mac_label(b));
        assert_ne!(win_value_name(a), win_value_name(b));
        assert_ne!(linux_file_name(a), LEGACY_DESKTOP);
    }

    // 옛 고정 이름 등록의 소유 판정 — 내 경로일 때만 내 것.
    #[test]
    fn legacy_ownership_is_by_exact_path() {
        let mine = Path::new("/opt/my apps/nexa\"beep");
        let d = desktop_content(&mine.display().to_string());
        assert_eq!(
            desktop_exe(&d).as_deref(),
            Some(r#"/opt/my apps/nexa"beep"#)
        );
        assert!(owned_by(desktop_exe(&d), mine));
        assert!(!owned_by(desktop_exe(&d), Path::new("/usr/bin/nexa-beep")));
        assert!(!owned_by(None, mine));
        assert_eq!(
            win_cmd_path(r#""C:\Apps\nexa-beep.exe""#),
            r"C:\Apps\nexa-beep.exe"
        );
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
        assert_eq!(
            xml_escape("/usr/local/bin/nexa-beep"),
            "/usr/local/bin/nexa-beep"
        );
    }

    // 부팅 동기화 판정 — 외부 삭제 존중은 "등록해 둔 적 있음"일 때만.
    #[test]
    fn boot_sync_respects_external_removal_only_after_prior_registration() {
        // 끔 = 언제나 걷어내기(관측·마커 무관).
        assert_eq!(boot_sync(false, true, true), BootSync::Unregister);
        assert_eq!(boot_sync(false, false, false), BootSync::Unregister);
        // 켬 + 등록 실재 = 경로 재동기화 재등록(포터블 이동).
        assert_eq!(boot_sync(true, true, true), BootSync::Register);
        // 켬 + 마커 없음 + 등록 없음 = 첫 실행·구버전 업그레이드 → 등록.
        assert_eq!(boot_sync(true, false, false), BootSync::Register);
        // 켬 + 마커 있음 + 등록 없음 = 사용자가 밖에서 지웠다 → 존중.
        assert_eq!(boot_sync(true, true, false), BootSync::RespectRemoval);
        // 켬 + 마커 없음 + 등록 실재(수기 등록) = 경로 최신화 재등록.
        assert_eq!(boot_sync(true, false, true), BootSync::Register);
    }
}
