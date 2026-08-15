//! OS 알림 어댑터(M3-8 최소 슬라이스 · FR-U-15 — 08-15 사용자 요청 "알림 3-OS").
//!
//! **범위**: 배너/토스트 표시 + 소리 유무(DR-25 신뢰 게이트 — 미검증은 소리 없음).
//! 등급 3종·강등표·수신자 릴레이는 🔴 D-23 확정 후(M2-4b와 함께) — 여기는 그 밑의
//! OS 표시 계층만이다. 내용 정책(미리보기 무해화·파일명 금지 — FR-S-41)은 호출자 몫.
//!
//! - **macOS** = `osascript display notification` 스폰(시스템 기본 도구 — `pbcopy`·
//!   `arp` 선례). 번들 밖 바이너리도 동작한다(UserNotifications는 번들·서명 요구라
//!   포터블(DR-4)에서 불가 — M5-4a 공증 후 재검토).
//! - **Linux** = `notify-send` 스폰(libnotify 도구 · 대부분의 데스크톱에 존재).
//!   없으면 조용히 실패(fail-soft — 알림은 보조 채널이다).
//! - **Windows** = 이 모듈이 아니라 **트레이 풍선**(`tray::TrayHandle::notify` —
//!   Shell_NotifyIcon NIF_INFO · 이미 떠 있는 아이콘 재사용 · AppUserModelID가
//!   필요한 WinRT 토스트는 설치본 전용이라 T0/포터블 원칙과 안 맞는다).

/// 알림 한 건 — 제목·본문·무음 여부(신뢰 게이트: 미검증 = `silent`).
#[derive(Debug)]
pub struct Note<'a> {
    /// 제목(발신자 표시 이름 등).
    pub title: &'a str,
    /// 본문(호출자가 무해화·절단을 끝낸 문자열).
    pub body: &'a str,
    /// 소리 억제(DR-25 — 미검증 발신자).
    pub silent: bool,
}

/// 알림을 띄운다 — 성공 여부(false = 이 OS 경로 없음/실패 · fail-soft).
#[must_use]
pub fn notify(n: &Note<'_>) -> bool {
    imp::notify(n)
}

/// AppleScript 문자열 이스케이프(mac) — 역슬래시·따옴표만이 특수문자다.
/// 스폰 인자로 스크립트를 넘기므로 셸 이스케이프는 불필요(Command = no shell).
#[cfg(any(target_os = "macos", test))]
fn escape_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            // 개행은 본문 줄바꿈으로 유효 — 그대로 둔다.
            _ => out.push(c),
        }
    }
    out
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Note;

    pub(super) fn notify(n: &Note<'_>) -> bool {
        let mut script = format!(
            "display notification \"{}\" with title \"{}\"",
            super::escape_applescript(n.body),
            super::escape_applescript(n.title),
        );
        if !n.silent {
            script.push_str(" sound name \"default\"");
        }
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::Note;

    pub(super) fn notify(n: &Note<'_>) -> bool {
        let mut cmd = std::process::Command::new("notify-send");
        cmd.arg("--app-name=Nexa Beep");
        if n.silent {
            // 데몬 재량 힌트(관용) — 미지원이면 무시된다(무해).
            cmd.arg("--hint=boolean:suppress-sound:true");
        }
        cmd.arg(n.title).arg(n.body);
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::Note;

    pub(super) fn notify(_n: &Note<'_>) -> bool {
        false // Windows = 트레이 풍선 경로(tray::TrayHandle::notify) · 기타 OS = 없음
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn applescript_escaping_neutralizes_quotes_and_backslashes() {
        // 수신 본문이 스크립트를 탈출하면 임의 osascript 실행이 된다 — 필수 회귀.
        assert_eq!(super::escape_applescript(r#"a"b\c"#), r#"a\"b\\c"#,);
        assert_eq!(super::escape_applescript("줄\n바꿈"), "줄\n바꿈");
    }
}
