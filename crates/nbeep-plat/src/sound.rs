//! 소리 포트(M3-8 · ADR-0010 §4-3 · DR-21) — **인박스 수단만 · 신규 의존 0**.
//!
//! | 타깃 | 수단 |
//! |---|---|
//! | Windows | `MessageBeep`(user32 — 사용자 설정 알림음) |
//! | macOS | `AudioServicesPlaySystemSound(kSystemSoundID_UserPreferredAlert)`(AudioToolbox) |
//! | Linux | **없음** — 표준 인박스 오디오가 없고 libcanberra는 LGPL(DR-12). 소리는 알림 데몬 힌트가 낸다
//!   ([`crate::notify::Note::urgent`]) · 데몬이 없으면 호스트가 "이 환경에서는 소리를 낼 수 없습니다"를 1회 고지 |
//!
//! 호출자는 설정(`notify.sound`)·신뢰 게이트(미검증 무음)를 **먼저** 판정하고 Alarm일 때만 부른다.

/// 긴급 알림음 1회 — 재생을 시도했으면 `true`(Linux = `false` · 데몬 힌트 경로).
pub fn play_alert() -> bool {
    imp::play_alert()
}

/// 이 플랫폼이 앱 자체로 소리를 낼 수 있는가(Linux `false`).
#[must_use]
pub fn available() -> bool {
    imp::AVAILABLE
}

#[cfg(windows)]
mod imp {
    pub(super) const AVAILABLE: bool = true;
    /// 사용자 설정의 "느낌표" 알림음(제어판 소리 구성 존중 · 소리 꺼짐이면 무음).
    const MB_ICONEXCLAMATION: u32 = 0x0000_0030;
    #[link(name = "user32")]
    extern "system" {
        fn MessageBeep(utype: u32) -> i32;
    }
    pub(super) fn play_alert() -> bool {
        // SAFETY: 인자 하나짜리 Win32 호출 · 전역 상태 없음.
        unsafe { MessageBeep(MB_ICONEXCLAMATION) != 0 }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    pub(super) const AVAILABLE: bool = true;
    /// `kSystemSoundID_UserPreferredAlert` — 시스템 설정의 경고음(사용자 선택 존중).
    const USER_PREFERRED_ALERT: u32 = 0x0000_1000;
    #[link(name = "AudioToolbox", kind = "framework")]
    extern "C" {
        fn AudioServicesPlaySystemSound(sound_id: u32);
    }
    pub(super) fn play_alert() -> bool {
        // SAFETY: 상수 시스템 사운드 ID 재생 · 비동기 · 실패는 무음.
        unsafe { AudioServicesPlaySystemSound(USER_PREFERRED_ALERT) };
        true
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub(super) const AVAILABLE: bool = false;
    pub(super) fn play_alert() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn availability_matches_platform() {
        assert_eq!(
            super::available(),
            cfg!(any(windows, target_os = "macos")),
            "Linux = 데몬 힌트 경로(자체 소리 없음)"
        );
    }
}
