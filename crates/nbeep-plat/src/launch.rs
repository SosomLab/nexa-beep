//! 실행 환경 이음 — **부모 콘솔 연결**(CLI 출력 보전) · 기본 프로그램 열기.
//!
//! ★ 08-20 전환: 바이너리가 Windows에서 **windows 서브시스템**이 됐다(main.rs
//! `windows_subsystem`). 종전엔 콘솔 서브시스템 + `FreeConsole` 휴리스틱(M5-4d ·
//! `from_gui_shell`/`hide_gui_console`)으로 탐색기 실행의 콘솔을 뗐는데, **Windows 11
//! 기본 터미널(Windows Terminal)은 ConPTY 세션 창이 루트 프로세스 종료까지 남아**
//! detach로는 창이 닫히지 않는다(실기 08-20 — 빈 터미널 창이 앱 옆에 상주). 서브시스템
//! 전환으로 콘솔 창은 **아예 생기지 않고**, 터미널 실행의 CLI 출력은
//! [`attach_parent_console`]이 부모 콘솔에 붙어 보전한다.

/// **부모 콘솔에 붙는다**(Windows · GUI 서브시스템의 CLI 보전) — 터미널(cmd·pwsh·bash)에서
/// 불렸으면 그 콘솔로 stdout/stderr/stdin이 이어져 `--help`·검증 도구 출력이 그대로
/// 보인다. 탐색기 더블클릭·자동 실행(M3-25)처럼 부모 콘솔이 없으면 no-op — 창만 뜬다.
///
/// ⚠ 규약 둘: ① **첫 출력 전에 호출**(Rust std가 표준 핸들을 처음 사용할 때 잡는다)
/// ② AttachConsole은 **표준 핸들 셋을 콘솔로 덮으므로**, 리다이렉트(`> 파일`·파이프)로
/// 물려받은 유효 핸들은 붙기 전에 떠 뒀다가 되살린다 — 안 그러면 `--version > f`나
/// 테스트 하네스 캡처가 콘솔로 새 나간다. 비-Windows는 no-op(콘솔 개념이 없다).
pub fn attach_parent_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(pid: u32) -> i32;
            fn GetStdHandle(which: u32) -> isize;
            fn SetStdHandle(which: u32, handle: isize) -> i32;
        }
        const ATTACH_PARENT_PROCESS: u32 = u32::MAX; // (DWORD)-1
        const STD_INPUT_HANDLE: u32 = -10i32 as u32;
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        const STD_ERROR_HANDLE: u32 = -12i32 as u32;
        const INVALID_HANDLE: isize = -1;
        let keep = |which: u32| {
            // SAFETY: 표준 핸들 조회 — 소유권·부작용 없음.
            let h = unsafe { GetStdHandle(which) };
            (h != 0 && h != INVALID_HANDLE).then_some(h)
        };
        let held = [
            (STD_INPUT_HANDLE, keep(STD_INPUT_HANDLE)),
            (STD_OUTPUT_HANDLE, keep(STD_OUTPUT_HANDLE)),
            (STD_ERROR_HANDLE, keep(STD_ERROR_HANDLE)),
        ];
        // SAFETY: 부모 콘솔이 없으면 실패(반환 0)하고 부작용도 없다.
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
            return;
        }
        for (which, h) in held {
            if let Some(h) = h {
                // SAFETY: 이 프로세스가 물려받은(여전히 유효한) 핸들을 되돌린다.
                unsafe { SetStdHandle(which, h) };
            }
        }
    }
}

/// **대화형 CLI의 콘솔 확보**(Windows) — cmd·PowerShell은 GUI 서브시스템 앱을
/// **기다리지 않는다**: 프롬프트가 즉시 돌아오고, 셸과 앱이 **한 콘솔의 입력을 나눠
/// 읽어** 키가 증발한다(08-20 실기 — `--chat-live` 입력 경합·"바로 종료된 것처럼"
/// 보임). 부모가 그런 셸이고 stdin이 실제 콘솔이면 **자기 콘솔 창을 새로 열어**
/// 옮긴다. bash(서브시스템 무관 대기)·파이프/리다이렉트(mintty·CI·스크립트)는
/// 그대로 둔다 — stdin이 콘솔이 아니면 no-op. 반환 = 새 창으로 옮겼는가.
///
/// ⚠ 호출은 **그 모드의 첫 출력 전에**(옮긴 뒤의 출력이 새 콘솔에 모이도록).
#[must_use]
pub fn own_console_for_interactive() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(which: u32) -> isize;
            fn GetConsoleMode(handle: isize, mode: *mut u32) -> i32;
            fn FreeConsole() -> i32;
            fn AllocConsole() -> i32;
            fn SetConsoleTitleW(title: *const u16) -> i32;
        }
        const STD_INPUT_HANDLE: u32 = -10i32 as u32;
        // stdin이 실제 콘솔인가 — 파이프·파일(mintty·CI·스크립트)이면 경합이 없다.
        let mut mode = 0u32;
        // SAFETY: 표준 핸들 조회 + 모드 조회 — 콘솔이 아니면 0 반환뿐.
        let is_console = unsafe {
            let h = GetStdHandle(STD_INPUT_HANDLE);
            h != 0 && h != -1 && GetConsoleMode(h, &mut mode) != 0
        };
        if !is_console {
            return false;
        }
        // 기다리지 않는 셸만 옮긴다 — bash류(대기)는 이 창에서 그대로가 낫다.
        let Some(parent) = parent_process_name() else {
            return false;
        };
        if !matches!(
            parent.as_str(),
            "cmd.exe" | "powershell.exe" | "pwsh.exe" | "powershell_ise.exe"
        ) {
            return false;
        }
        // 옛 콘솔(셸 프롬프트 자리)에 이유 한 줄 — 조용히 창을 바꾸지 않는다.
        eprintln!(
            "ℹ 대화형 모드 — 새 콘솔 창에서 계속합니다 ({parent}는 GUI 앱을 기다리지 \
             않아 입력이 섞입니다 · Git Bash는 이 창에서 그대로 동작)"
        );
        let title: Vec<u16> = "Nexa Beep — 터미널 대화"
            .encode_utf16()
            .chain([0])
            .collect();
        // SAFETY: 콘솔 detach 후 새 콘솔 할당 — 실패해도(극히 드묾) 부작용은
        // "콘솔 없음"이며, 이후 읽기 실패로 모드가 스스로 끝난다(fail-soft).
        unsafe {
            FreeConsole();
            if AllocConsole() == 0 {
                return false;
            }
            SetConsoleTitleW(title.as_ptr());
        }
        true
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 부모 프로세스의 실행 파일 이름(소문자 · Windows) — Toolhelp 스냅숏 2회 조회.
/// 실패는 `None`(호출자가 보수적으로 판단 — 옮기지 않는 쪽).
#[cfg(windows)]
fn parent_process_name() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        fn Process32FirstW(snap: isize, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snap: isize, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetCurrentProcessId() -> u32;
    }
    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }
    const TH32CS_SNAPPROCESS: u32 = 0x2;
    // SAFETY: 스냅숏 핸들은 아래에서 반드시 닫는다. 엔트리는 dw_size를 채워 넘긴다.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == -1 || snap == 0 {
            return None;
        }
        let mut entry = std::mem::zeroed::<ProcessEntry32W>();
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let me = GetCurrentProcessId();
        let mut parent_pid = None;
        let mut ok = Process32FirstW(snap, &mut entry);
        while ok != 0 {
            if entry.th32_process_id == me {
                parent_pid = Some(entry.th32_parent_process_id);
                break;
            }
            ok = Process32NextW(snap, &mut entry);
        }
        let name = parent_pid.and_then(|pp| {
            entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry);
            while ok != 0 {
                if entry.th32_process_id == pp {
                    let len = entry
                        .sz_exe_file
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.sz_exe_file.len());
                    return Some(
                        String::from_utf16_lossy(&entry.sz_exe_file[..len]).to_lowercase(),
                    );
                }
                ok = Process32NextW(snap, &mut entry);
            }
            None
        });
        CloseHandle(snap);
        name
    }
}

/// 파일·폴더를 **OS 기본 연결 프로그램**으로 연다(M3-22 "로그 보기" — `.log`는
/// 기본 편집기·폴더는 탐색기). 성공 = 스폰 성공(열림 보장까지는 아님 — fail-soft).
/// Windows `cmd /c start`(연결 프로그램 해석은 셸 몫) · mac `open` · Linux `xdg-open`.
#[must_use]
pub fn open_path(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // start의 첫 인자는 창 제목 자리 — 빈 제목을 줘야 경로가 제목으로 안 삼켜진다.
        // CREATE_NO_WINDOW: 본체가 windows 서브시스템(08-20)이라 콘솔이 없고, 그 상태로
        // 콘솔 앱 cmd를 그냥 스폰하면 콘솔 창이 번쩍인다. 열리는 대상(편집기·탐색기)의
        // 창은 별개 프로세스라 영향 없다.
        std::process::Command::new("cmd")
            .args(["/c", "start", ""])
            .arg(path)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn().is_ok()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .is_ok()
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_console_noop_without_console_stdin() {
        // 테스트 하네스 stdin은 콘솔이 아니다(파이프/널) — 옮기지 않아야 한다.
        // (스크립트·CI·mintty 경로의 무회귀 박제 — 옮기면 캡처가 끊긴다.)
        assert!(!own_console_for_interactive());
    }

    #[cfg(windows)]
    #[test]
    fn parent_name_resolves_without_panic() {
        // 하네스 부모는 환경마다 다르다(cargo·pwsh·bash) — 값이 아니라 무패닉 + 형식만.
        if let Some(n) = parent_process_name() {
            assert!(!n.is_empty());
            assert_eq!(n, n.to_lowercase());
        }
    }

    #[test]
    fn attach_is_safe_everywhere() {
        // 테스트 하네스는 콘솔/파이프가 다양하다 — 어떤 환경에서든 **패닉 없이**
        // 지나가고, 캡처된 stdout이 콘솔로 새지 않아야 한다(핸들 보전 규약).
        attach_parent_console();
        println!("still-captured"); // 하네스 캡처가 살아 있으면 이 줄은 콘솔에 안 샌다
    }
}
