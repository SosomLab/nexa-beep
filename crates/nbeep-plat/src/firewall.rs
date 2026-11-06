//! **방화벽 허용 등록 포트**(FW-1 · 11-06 · docs/06 §6 T1 "방화벽 허용" · FR-P-5 "이유를 설명하고 요청" · DR-14 T0 = 제품).
//!
//! Windows만: 사용자가 **설정 › 고급에서 눌렀을 때만** `ShellExecuteW("runas")`로 `netsh advfirewall` 규칙을 추가/제거한다
//! (UAC 1회 · 규칙 = 프로그램 경로 기준 인바운드 허용 · 포트 무관 · 이름 "Nexa Beep"). 거부·실패해도 앱은 그대로 동작한다
//! (T0). 콘솔 창은 `SW_HIDE`로 숨긴다(CLAUDE.md "콘솔 창 0"). 포터블은 exe 경로마다 규칙이 생기므로 폴더를 옮기면 다시 눌러야 한다.
//!
//! macOS = 서명·공증(M5-4a) 전엔 방화벽 대화상자가 정답 · Linux = 패키지가 ufw 앱 프로필·firewalld 서비스 정의를 **설치만**
//! 하고(열지 않는다 — Debian 관례 · 06 §6 규칙 2) 사용자가 `ufw allow nexa-beep` / `firewall-cmd --add-service=nexa-beep`로 켠다.

/// 요청 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwAction {
    /// 인바운드 허용 규칙 추가(같은 이름·경로의 옛 규칙은 먼저 지운다 — 멱등).
    Allow,
    /// 이 이름의 규칙 전부 제거.
    Remove,
}

/// 규칙 이름(개발용 `tools/firewall-allow.ps1`의 "Nexa Beep dev"와 분리).
pub const RULE_NAME: &str = "Nexa Beep";

/// 이 플랫폼에서 앱이 직접 요청할 수 있는가(Windows).
#[must_use]
pub fn supported() -> bool {
    cfg!(windows)
}

/// 관리자 권한으로 규칙 추가/제거를 **요청**한다 — 반환 = 승격 프로세스가 시작됐는가(UAC 거부·비지원 = `false`).
/// 결과(규칙이 실제로 생겼는가)는 비동기라 알 수 없다 — 사용자에게 "요청함"으로만 알린다.
pub fn request(action: FwAction) -> bool {
    imp::request(action)
}

/// `netsh` 명령줄(순수 함수 · 테스트) — `cmd /c` 인자. 경로는 큰따옴표로 감싼다.
#[must_use]
pub fn netsh_args(action: FwAction, exe: &str) -> String {
    let del = format!("netsh advfirewall firewall delete rule name=\"{RULE_NAME}\"");
    match action {
        FwAction::Remove => format!("/c {del}"),
        FwAction::Allow => format!(
            "/c {del} program=\"{exe}\" & netsh advfirewall firewall add rule name=\"{RULE_NAME}\" dir=in action=allow program=\"{exe}\" enable=yes profile=any"
        ),
    }
}

#[cfg(windows)]
mod imp {
    use super::{netsh_args, FwAction};

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    const SW_HIDE: i32 = 0;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    pub(super) fn request(action: FwAction) -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let exe = exe.to_string_lossy().into_owned();
        if exe.contains('"') {
            return false; // 인용 탈출 금지(경로에 따옴표가 있는 경우는 없다 — 방어)
        }
        let op = wide("runas");
        let file = wide("cmd.exe");
        let params = wide(&netsh_args(action, &exe));
        // SAFETY: 널 종료 UTF-16 버퍼 · 호출 동안 살아 있다 · 결과 HINSTANCE > 32 = 시작 성공.
        let r = unsafe {
            ShellExecuteW(
                0,
                op.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                SW_HIDE,
            )
        };
        r > 32
    }
}

#[cfg(not(windows))]
mod imp {
    use super::FwAction;
    pub(super) fn request(_action: FwAction) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netsh_args_are_idempotent_and_quoted() {
        let a = netsh_args(FwAction::Allow, r"C:\Apps\Nexa Beep\nexa-beep.exe");
        assert!(a.starts_with("/c netsh advfirewall firewall delete rule name=\"Nexa Beep\""));
        assert!(a.contains("add rule name=\"Nexa Beep\" dir=in action=allow program=\"C:\\Apps\\Nexa Beep\\nexa-beep.exe\" enable=yes profile=any"));
        let r = netsh_args(FwAction::Remove, "x");
        assert_eq!(
            r,
            "/c netsh advfirewall firewall delete rule name=\"Nexa Beep\""
        );
        assert_eq!(supported(), cfg!(windows));
    }
}
