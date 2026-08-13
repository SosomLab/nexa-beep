//! **GUI를 띄울 수 있는 세션인가** — 창을 열기 전에 묻는다(사용자 요청 08-13).
//!
//! 창을 열 수 없는 자리에서 창을 열려고 하면 프로세스가 **그냥 죽는다.** 실기에서
//! 컨테이너에 무인자로 띄웠다가 `Exited (134)`와 `시스템 UI 폰트 없음`만 남은 적이 있고
//! ([26 §6](../../../docs/26-run-and-manual-test.md)), SSH·원격 PowerShell도 같은 자리다.
//! **왜 안 되는지와 대신 무엇을 쓸지**를 콘솔에 말하고 조용히 끝내는 게 맞다.
//!
//! ## 설계 — 판정과 관측을 나눈다
//!
//! OS API를 부르는 [`observe`]와, 사실만 보고 판단하는 [`decide`]를 분리했다.
//! 판정이 순수 함수라 **3-OS × 세션 종류 전부를 맥에서 테스트**할 수 있다(조건부 컴파일
//! 코드는 반대편을 실행해 볼 수 없다는 게 이 저장소의 반복된 함정이라 — [term.rs] 참조).
//!
//! ## 판단 근거 (OS별)
//!
//! | OS | 신호 | 근거 |
//! | --- | --- | --- |
//! | Windows | `GetProcessWindowStation` 이름 | 대화형 데스크톱은 **`WinSta0`** 뿐. 서비스·원격 PS(WinRM)·OpenSSH는 `Service-0x0-…$` 같은 별도 스테이션을 받는다. **RDP는 `WinSta0`이라 통과**(실제로 GUI가 된다) |
//! | macOS | `CGSessionCopyCurrentDictionary()` | 창 서버 세션에 붙어 있어야 non-NULL. SSH·데몬은 NULL — `launchctl managername`이 `Aqua`가 아닌 것과 같은 판정을 서브프로세스 없이 얻는다 |
//! | Linux·BSD | `$DISPLAY` / `$WAYLAND_DISPLAY` | 디스플레이 서버 주소가 없으면 붙을 데가 없다. `ssh -X`는 `DISPLAY`를 채우므로 **자동으로 통과**한다(전달이 실제로 되는지는 별개) |
//!
//! 신호를 못 얻으면(API 실패) **막지 않는다** — 오탐으로 정상 실행을 막는 쪽이 더 나쁘다.
//! 단 SSH가 확실하면 그때는 막는다(그 자리에서 창이 뜰 가능성은 없다).

/// 창을 못 여는 이유 — 사유마다 사용자가 할 일이 다르므로 나눠서 든다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoGui {
    /// Linux·BSD — 디스플레이 서버 주소가 없다.
    NoDisplayServer {
        /// SSH 세션이면 X11 전달을 안내한다.
        ssh: bool,
    },
    /// macOS — 창 서버에 붙을 수 없는 세션(SSH·데몬·`launchd` 백그라운드).
    NoAquaSession {
        /// SSH 세션이면 그렇게 말해 준다.
        ssh: bool,
    },
    /// Windows — 대화형 데스크톱이 없다(원격 PowerShell·OpenSSH·서비스).
    NoWindowStation {
        /// 관측된 윈도우 스테이션 이름(진단용 — 사용자에게 그대로 보여준다).
        station: String,
    },
    /// 창은 열 수 있는데 **글자를 그릴 폰트가 없다**(최소 컨테이너·폰트 미설치 서버).
    ///
    /// 세션 판정과 따로 두는 이유 — 사용자가 할 일이 다르다(여긴 폰트를 깔면 된다).
    /// 그전에는 `expect("시스템 UI 폰트 없음")` 패닉이었고, 릴리스는 `panic = "abort"`라
    /// **`Exited (134)`**(SIGABRT)로만 보였다 — 실기에서 원인 찾는 데 시간을 썼다.
    NoSystemFont,
}

impl NoGui {
    /// 콘솔에 그대로 낼 사람 말 — **왜 안 되는지 + 무엇을 쓸지**를 함께 준다.
    /// 안내 없이 끝내면 사용자는 앱이 깨진 줄 안다.
    #[must_use]
    pub fn message(&self) -> String {
        let alt = "이 자리에서는 터미널 단말을 쓰세요:\n  \
                   nexa-beep --chat-live <이름> [--port <N>]   # 목록에 뜨는 대화 단말\n  \
                   nexa-beep --discover-probe 8                # 발견만 관찰\n  \
                   nexa-beep --help                            # 전체 모드";
        match self {
            NoGui::NoDisplayServer { ssh: true } => format!(
                "창을 열 수 없습니다 — SSH 세션에 디스플레이가 없습니다($DISPLAY·$WAYLAND_DISPLAY 없음).\n\
                 X11 전달을 쓰려면 `ssh -X`(또는 `-Y`)로 다시 접속하세요.\n{alt}"
            ),
            NoGui::NoDisplayServer { ssh: false } => format!(
                "창을 열 수 없습니다 — 디스플레이 서버가 없습니다($DISPLAY·$WAYLAND_DISPLAY 없음).\n\
                 콘솔·컨테이너·헤드리스 서버에서는 창이 뜨지 않습니다.\n{alt}"
            ),
            NoGui::NoAquaSession { ssh } => format!(
                "창을 열 수 없습니다 — {} 창 서버(WindowServer)에 붙을 수 없습니다.\n\
                 macOS는 콘솔에 로그인한 세션에서만 창을 띄웁니다(화면 공유는 됩니다).\n{alt}",
                if *ssh {
                    "SSH 세션이라"
                } else {
                    "이 세션은"
                }
            ),
            NoGui::NoWindowStation { station } => format!(
                "창을 열 수 없습니다 — 대화형 데스크톱이 없는 세션입니다(윈도우 스테이션 = {station}).\n\
                 원격 PowerShell·OpenSSH·서비스 세션에서는 창이 뜨지 않습니다.\n\
                 원격에서 화면이 필요하면 **원격 데스크톱(RDP)** 으로 접속하세요 — 그건 창이 뜹니다.\n{alt}"
            ),
            NoGui::NoSystemFont => format!(
                "창을 열 수 없습니다 — 쓸 수 있는 시스템 UI 폰트를 못 찾았습니다.\n\
                 최소 설치 컨테이너·서버에 흔합니다. 폰트를 설치하면 이 문제는 사라집니다\n\
                 (예: Debian·Ubuntu `apt-get install -y fonts-dejavu-core`, 한글은 `fonts-nanum`).\n{alt}"
            ),
        }
    }
}

/// 세션에서 관측한 사실 — [`decide`]의 입력. 테스트가 직접 지어낼 수 있게 전부 공개.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionFacts {
    /// SSH 환경 변수가 보인다(`SSH_CONNECTION`·`SSH_TTY`·`SSH_CLIENT`).
    pub ssh: bool,
    /// unix — `$DISPLAY` 또는 `$WAYLAND_DISPLAY`가 비어 있지 않다.
    pub display: bool,
    /// macOS — 창 서버 세션에 붙어 있다. `None` = 확인 못 함.
    pub aqua: Option<bool>,
    /// Windows — 프로세스 윈도우 스테이션 이름. `None` = 확인 못 함.
    pub window_station: Option<String>,
    /// 글자를 그릴 시스템 UI 폰트를 찾았다.
    pub ui_font: bool,
}

/// 판정 대상 OS — [`decide`]를 3-OS 전부 테스트하려고 인자로 받는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// macOS.
    Macos,
    /// Windows.
    Windows,
    /// Linux·BSD 등 나머지 unix.
    Unix,
}

impl Os {
    /// 지금 컴파일된 대상.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Unix
        }
    }
}

/// 사실 → 판정(**순수 함수**).
///
/// # Errors
/// 창을 열 수 없다고 판단하면 사유를 돌려준다. 확신이 없으면 `Ok`다 —
/// 오탐으로 멀쩡한 실행을 막는 쪽이 더 나쁘다.
pub fn decide(os: Os, f: &SessionFacts) -> Result<(), NoGui> {
    session_ok(os, f)?;
    // 세션은 되는데 그릴 폰트가 없으면 그것도 못 여는 것이다 — 순서는 세션이 먼저다
    // (헤드리스는 둘 다 없는데, 그때 유용한 안내는 "터미널 단말을 쓰라" 쪽이다).
    if f.ui_font {
        Ok(())
    } else {
        Err(NoGui::NoSystemFont)
    }
}

/// OS별 세션 판정만(폰트는 [`decide`]가 뒤이어 본다).
fn session_ok(os: Os, f: &SessionFacts) -> Result<(), NoGui> {
    match os {
        // 대화형 데스크톱은 WinSta0 하나뿐. RDP도 WinSta0라 통과한다.
        Os::Windows => match f.window_station.as_deref() {
            Some(name) if name.eq_ignore_ascii_case("WinSta0") => Ok(()),
            Some(name) => Err(NoGui::NoWindowStation {
                station: name.to_string(),
            }),
            // API 실패 — SSH가 확실할 때만 막는다.
            None if f.ssh => Err(NoGui::NoWindowStation {
                station: "확인 불가(SSH)".into(),
            }),
            None => Ok(()),
        },
        Os::Macos => match f.aqua {
            Some(true) => Ok(()),
            Some(false) => Err(NoGui::NoAquaSession { ssh: f.ssh }),
            None if f.ssh => Err(NoGui::NoAquaSession { ssh: true }),
            None => Ok(()),
        },
        // ssh -X 는 DISPLAY 를 채운다 — 그래서 SSH 여부보다 DISPLAY 가 먼저다.
        Os::Unix if f.display => Ok(()),
        Os::Unix => Err(NoGui::NoDisplayServer { ssh: f.ssh }),
    }
}

/// 지금 세션을 관측한다(OS API·환경 변수).
#[must_use]
pub fn observe() -> SessionFacts {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    SessionFacts {
        ssh: ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
            .iter()
            .any(|k| var(k).is_some()),
        display: var("DISPLAY").is_some() || var("WAYLAND_DISPLAY").is_some(),
        aqua: aqua_session(),
        window_station: window_station(),
        ui_font: crate::font::system_ui_font().is_some(),
    }
}

/// 이 세션에서 창을 열 수 있는지 — 관측 + 판정을 한 번에.
///
/// # Errors
/// 창을 열 수 없으면 사유.
pub fn probe() -> Result<(), NoGui> {
    decide(Os::current(), &observe())
}

// ── macOS ────────────────────────────────────────────────────────────────────

/// 창 서버 세션 여부 — `CGSessionCopyCurrentDictionary()`가 NULL이면 붙을 수 없다.
/// SSH·`launchd` 백그라운드가 그 경우다(`launchctl managername`이 `Aqua`가 아닌 것과 같다).
#[cfg(target_os = "macos")]
fn aqua_session() -> Option<bool> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> *const core::ffi::c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const core::ffi::c_void);
    }
    // SAFETY: 인자 없는 조회 호출. 반환은 NULL이거나 **소유권 있는**(Copy 규약)
    // CFDictionary라 곧바로 해제한다 — 내용은 보지 않는다("봉투만 본다").
    unsafe {
        let d = CGSessionCopyCurrentDictionary();
        if d.is_null() {
            return Some(false);
        }
        CFRelease(d);
        Some(true)
    }
}

#[cfg(not(target_os = "macos"))]
fn aqua_session() -> Option<bool> {
    None
}

// ── Windows ──────────────────────────────────────────────────────────────────

/// 프로세스 윈도우 스테이션 이름 — 대화형이면 `WinSta0`.
///
/// 서비스·WinRM(원격 PowerShell)·OpenSSH 세션은 `Service-0x0-3e7$` 꼴을 받는다.
/// **RDP는 `WinSta0`** 이라 그대로 통과한다(실제로 창이 뜬다).
#[cfg(windows)]
fn window_station() -> Option<String> {
    type Handle = *mut core::ffi::c_void;
    /// `UOI_NAME` — 개체 이름을 달라는 질의 코드.
    const UOI_NAME: i32 = 2;

    #[link(name = "user32")]
    extern "system" {
        fn GetProcessWindowStation() -> Handle;
        fn GetUserObjectInformationW(
            h: Handle,
            index: i32,
            info: *mut core::ffi::c_void,
            len: u32,
            needed: *mut u32,
        ) -> i32;
    }

    // SAFETY: 핸들은 소유하지 않는 프로세스 스테이션(닫지 않는다). 버퍼 길이는
    // 바이트 수로 넘기고, 성공 시 needed(바이트) 안쪽만 UTF-16으로 읽는다.
    unsafe {
        let h = GetProcessWindowStation();
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 128];
        let mut needed: u32 = 0;
        let len_bytes = u32::try_from(core::mem::size_of_val(&buf)).unwrap_or(u32::MAX);
        let ok = GetUserObjectInformationW(
            h,
            UOI_NAME,
            buf.as_mut_ptr().cast(),
            len_bytes,
            &raw mut needed,
        );
        if ok == 0 {
            return None;
        }
        let chars = (needed as usize / 2).min(buf.len());
        let s: String = String::from_utf16_lossy(&buf[..chars]);
        let s = s.trim_end_matches('\0').to_string();
        (!s.is_empty()).then_some(s)
    }
}

#[cfg(not(windows))]
fn window_station() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 세션 테스트의 기본 — **폰트는 있다**(폰트 축은 따로 시험한다).
    fn facts() -> SessionFacts {
        SessionFacts {
            ui_font: true,
            ..SessionFacts::default()
        }
    }

    // ── Windows ──────────────────────────────────────────────────────────

    #[test]
    fn windows_interactive_desktop_passes() {
        let f = SessionFacts {
            window_station: Some("WinSta0".into()),
            ..facts()
        };
        assert_eq!(decide(Os::Windows, &f), Ok(()));
    }

    /// RDP도 `WinSta0`다 — 원격이라고 막으면 **되는 걸 막는 것**이다.
    #[test]
    fn windows_rdp_passes() {
        let f = SessionFacts {
            ssh: false,
            window_station: Some("winsta0".into()), // 대소문자 무관
            ..facts()
        };
        assert_eq!(decide(Os::Windows, &f), Ok(()));
    }

    /// 원격 PowerShell(WinRM)·서비스 — 별도 스테이션.
    #[test]
    fn windows_remote_powershell_blocked() {
        let f = SessionFacts {
            window_station: Some("Service-0x0-3e7$".into()),
            ..facts()
        };
        let e = decide(Os::Windows, &f).unwrap_err();
        assert_eq!(
            e,
            NoGui::NoWindowStation {
                station: "Service-0x0-3e7$".into()
            }
        );
        assert!(
            e.message().contains("원격 데스크톱"),
            "대안을 알려줘야 한다"
        );
    }

    #[test]
    fn windows_ssh_without_api_blocked() {
        let f = SessionFacts {
            ssh: true,
            window_station: None,
            ..facts()
        };
        assert!(decide(Os::Windows, &f).is_err());
    }

    /// API 실패 + SSH 아님 = **막지 않는다**(오탐이 더 나쁘다).
    #[test]
    fn windows_unknown_is_allowed() {
        assert_eq!(decide(Os::Windows, &facts()), Ok(()));
    }

    // ── macOS ────────────────────────────────────────────────────────────

    #[test]
    fn macos_console_session_passes() {
        let f = SessionFacts {
            aqua: Some(true),
            ..facts()
        };
        assert_eq!(decide(Os::Macos, &f), Ok(()));
    }

    #[test]
    fn macos_ssh_blocked() {
        let f = SessionFacts {
            ssh: true,
            aqua: Some(false),
            ..facts()
        };
        let e = decide(Os::Macos, &f).unwrap_err();
        assert_eq!(e, NoGui::NoAquaSession { ssh: true });
        assert!(e.message().contains("SSH 세션이라"));
    }

    /// 데몬·`launchd` 백그라운드 — SSH는 아니지만 창 서버가 없다.
    #[test]
    fn macos_daemon_blocked_without_ssh() {
        let f = SessionFacts {
            aqua: Some(false),
            ..facts()
        };
        assert_eq!(
            decide(Os::Macos, &f),
            Err(NoGui::NoAquaSession { ssh: false })
        );
    }

    #[test]
    fn macos_unknown_is_allowed() {
        assert_eq!(decide(Os::Macos, &facts()), Ok(()));
    }

    // ── Linux·BSD ────────────────────────────────────────────────────────

    #[test]
    fn unix_with_display_passes() {
        let f = SessionFacts {
            display: true,
            ..facts()
        };
        assert_eq!(decide(Os::Unix, &f), Ok(()));
    }

    /// ★ `ssh -X`는 DISPLAY를 채운다 — SSH여도 통과해야 한다.
    #[test]
    fn unix_ssh_with_x11_forwarding_passes() {
        let f = SessionFacts {
            ssh: true,
            display: true,
            ..facts()
        };
        assert_eq!(decide(Os::Unix, &f), Ok(()));
    }

    #[test]
    fn unix_ssh_without_display_blocked() {
        let f = SessionFacts {
            ssh: true,
            ..facts()
        };
        let e = decide(Os::Unix, &f).unwrap_err();
        assert_eq!(e, NoGui::NoDisplayServer { ssh: true });
        assert!(e.message().contains("ssh -X"), "전달 방법을 알려줘야 한다");
    }

    /// 컨테이너·헤드리스 서버 — 실기에서 `Exited (134)`로 죽던 자리(26 §6).
    #[test]
    fn unix_container_blocked() {
        let e = decide(Os::Unix, &facts()).unwrap_err();
        assert_eq!(e, NoGui::NoDisplayServer { ssh: false });
        assert!(e.message().contains("--chat-live"), "대안을 알려줘야 한다");
    }

    /// 모든 사유가 대안을 안내한다 — 안내 없이 끝내면 앱이 깨진 줄 안다.
    #[test]
    fn every_reason_offers_an_alternative() {
        for e in [
            NoGui::NoDisplayServer { ssh: true },
            NoGui::NoDisplayServer { ssh: false },
            NoGui::NoAquaSession { ssh: true },
            NoGui::NoAquaSession { ssh: false },
            NoGui::NoWindowStation {
                station: "X".into(),
            },
            NoGui::NoSystemFont,
        ] {
            let m = e.message();
            assert!(m.contains("창을 열 수 없습니다"), "{m}");
            assert!(m.contains("--chat-live"), "{m}");
            assert!(m.contains("--help"), "{m}");
        }
    }

    // ── 폰트 축 ──────────────────────────────────────────────────────────

    /// ★ 실기 재현 — 최소 컨테이너는 세션(DISPLAY)이 있어도 폰트가 없다.
    /// 그전에는 `expect` 패닉 → `panic = "abort"` → **`Exited (134)`** 로만 보였다.
    #[test]
    fn font_missing_blocks_even_with_display() {
        let f = SessionFacts {
            display: true,
            ui_font: false,
            ..SessionFacts::default()
        };
        assert_eq!(decide(Os::Unix, &f), Err(NoGui::NoSystemFont));
        assert!(decide(Os::Unix, &f)
            .unwrap_err()
            .message()
            .contains("fonts-dejavu-core"));
    }

    /// 세션이 먼저다 — 둘 다 없으면 "터미널 단말을 쓰라"가 더 쓸모 있는 안내다.
    #[test]
    fn session_reason_wins_over_font() {
        let f = SessionFacts::default(); // display 없음 + 폰트 없음
        assert_eq!(
            decide(Os::Unix, &f),
            Err(NoGui::NoDisplayServer { ssh: false })
        );
    }

    /// 폰트 축은 OS를 가리지 않는다.
    #[test]
    fn font_axis_applies_to_every_os() {
        for (os, f) in [
            (
                Os::Macos,
                SessionFacts {
                    aqua: Some(true),
                    ..SessionFacts::default()
                },
            ),
            (
                Os::Windows,
                SessionFacts {
                    window_station: Some("WinSta0".into()),
                    ..SessionFacts::default()
                },
            ),
            (
                Os::Unix,
                SessionFacts {
                    display: true,
                    ..SessionFacts::default()
                },
            ),
        ] {
            assert_eq!(decide(os, &f), Err(NoGui::NoSystemFont), "{os:?}");
        }
    }

    /// 실제 세션 관측이 패닉 없이 돌고, 이 맥(콘솔 로그인)에서는 통과한다.
    #[test]
    fn observe_runs_and_local_session_is_usable() {
        let f = observe();
        if cfg!(target_os = "macos") {
            assert_eq!(f.aqua, Some(true), "콘솔 세션이면 창 서버가 보인다");
            assert_eq!(probe(), Ok(()));
        }
    }
}
