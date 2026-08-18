//! 실행 출처 판정 — **GUI(탐색기/Dock)에서 인자 없이 열렸는가**(M5-4d · 배포 실기 08-11).
//!
//! 콘솔 서브시스템 바이너리를 배포하면, 사용자가 탐색기에서 더블클릭할 때 인자가 없어
//! 스캐폴드 안내만 찍고 끝난다 — 눈에는 "콘솔이 번쩍이고 사라짐"으로 보인다(첫 배포에서
//! 손에 쥔 경험이 이것). macOS는 `.app/Contents/MacOS/` 경로로 번들 실행을 알아채 창 모드로
//! 돌렸는데(main.rs), 그 판정이 macOS 전용이라 Windows엔 대응이 없었다. 이 모듈이 그 대칭을
//! 채운다: **Windows에서 "탐색기가 새로 띄운 콘솔"** 을 감지한다.

/// GUI 셸(탐색기 등)이 **새 콘솔을 할당해** 이 프로세스를 띄웠는가.
///
/// Windows에서만 참이 될 수 있다. 판정은 [`GetConsoleProcessList`](https://learn.microsoft.com/windows/console/getconsoleprocesslist)
/// 휴리스틱(Raymond Chen) — 붙어 있는 프로세스가 **자기 하나뿐**(반환 1)이면 탐색기가 이 앱을
/// 위해 콘솔을 새로 만든 것이고, 기존 터미널(cmd·pwsh)에서 부른 경우는 셸까지 2개 이상이다.
/// macOS/Linux는 항상 `false`(각자의 판정은 호출자 몫 — 예: macOS 번들 경로).
#[must_use]
pub fn from_gui_shell() -> bool {
    imp()
}

#[cfg(windows)]
fn imp() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    }
    let mut buf = [0u32; 2];
    // SAFETY: 출력 버퍼(길이 2)와 그 길이를 넘긴다 — 커널이 최대 count개만 채우고 총계를
    // 반환한다. 핸들·소유권 없음. 콘솔이 없으면 0(그 경우 GUI 판정도 아니다).
    let n = unsafe { GetConsoleProcessList(buf.as_mut_ptr(), buf.len() as u32) };
    n == 1
}

#[cfg(not(windows))]
fn imp() -> bool {
    false
}

/// GUI 실행 시 **탐색기가 붙여 준 콘솔 창을 닫는다**(Windows). 우리는 콘솔 서브시스템
/// 바이너리라, 창 모드로 가더라도 그 콘솔이 GUI 창 옆에 남는다 — 여기서 떼어낸다.
/// 붙어 있는 프로세스가 자기뿐일 때(=[`from_gui_shell`] 참) 호출해야 안전하다:
/// `FreeConsole`이 마지막 detach가 되어 콘솔 창이 사라진다. 비-Windows는 no-op.
pub fn hide_gui_console() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn FreeConsole() -> i32;
        }
        // SAFETY: 인자 없는 콘솔 detach. 실패해도(콘솔 없음 등) 부작용 없다.
        unsafe {
            FreeConsole();
        }
    }
}

/// 파일·폴더를 **OS 기본 연결 프로그램**으로 연다(M3-22 "로그 보기" — `.log`는
/// 기본 편집기·폴더는 탐색기). 성공 = 스폰 성공(열림 보장까지는 아님 — fail-soft).
/// Windows `cmd /c start`(연결 프로그램 해석은 셸 몫) · mac `open` · Linux `xdg-open`.
#[must_use]
pub fn open_path(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        // start의 첫 인자는 창 제목 자리 — 빈 제목을 줘야 경로가 제목으로 안 삼켜진다.
        std::process::Command::new("cmd")
            .args(["/c", "start", ""])
            .arg(path)
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
    fn returns_bool_without_panic() {
        // 테스트 하네스는 콘솔/파이프가 다양하다 — 값이 아니라 **패닉 없이 판정**됨을 본다.
        let _ = from_gui_shell();
        // 비-Windows는 계약상 항상 false.
        #[cfg(not(windows))]
        assert!(!from_gui_shell());
    }
}
