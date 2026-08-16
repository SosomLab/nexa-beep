//! **터미널 raw 모드 + 확장 키 프로토콜** — CLI에서 Shift+Enter 같은 조합을 읽기 위한 계층.
//!
//! ## 왜 필요한가
//!
//! 터미널의 기본(cooked) 모드는 **줄 단위**로 넘겨주고, 그 과정에서 Enter와 Shift+Enter가
//! **똑같은 `\r`** 로 뭉개진다. 조합을 구분하려면 두 가지가 동시에 필요하다:
//! 1. **raw 모드** — 줄 조립·에코를 끄고 키를 바이트로 직접 받는다.
//! 2. **확장 키 리포팅** — 터미널이 Shift+Enter를 `ESC[13;2u`처럼 **따로 보고**해 줘야 한다.
//!    (kitty 키보드 프로토콜 `CSI > 1 u` · xterm `modifyOtherKeys`)
//!
//! ⚠️ **정직한 한계**: 2번은 터미널이 지원해야 한다. macOS Terminal.app처럼 지원하지 않는
//! 터미널에서는 Shift+Enter가 여전히 그냥 Enter로 온다 — 그래서 [`RawTerm::enter`]는
//! 지원 여부를 장담하지 않고, 호출 측은 **대체 입력 수단을 함께 제공**해야 한다.
//!
//! TTY가 아니면(파이프 입력 등) raw 모드를 켜지 않는다 — 자동화 스크립트가 그대로 돈다.

// ⚠️ `#[cfg(unix)]`를 붙이면 안 된다 — 아래 `Drop`은 **모든 플랫폼에서 컴파일**되고
// 거기서 `write_all`/`flush`를 쓴다. unix로 묶었다가 Windows 빌드가 깨졌다(08-11 CI).
// 맥에서 도는 게이트로는 잡히지 않는 종류라, 조건부 컴파일은 항상 반대편을 의심한다.
use std::io::Write as _;

/// 복원해야 할 터미널 상태(전역 1슬롯) — **Drop이 돌지 못하는 경로**(릴리스는
/// `panic = "abort"`라 되감기 없음 · 시그널 기본 동작 즉사)에서도 [`restore_now`]가
/// 여길 보고 되돌린다. 실기(08-13): kitty 플래그가 켜진 채 죽으면 그 pane의 Ctrl+C가
/// `9;5u`로 찍히고 인터럽트가 죽는다 — 상태는 셸이 아니라 **터미널 에뮬레이터**에 남는다.
#[cfg(unix)]
static ACTIVE: std::sync::Mutex<Option<libc::termios>> = std::sync::Mutex::new(None);
/// 패닉 훅은 프로세스에 1회만 설치한다.
/// ⚠️ unix 전용으로 묶는다 — 설치 지점이 `enter_with`의 unix 블록뿐이라, 안 묶으면
/// Windows에서 dead_code로 릴리스 빌드(-D warnings)가 깨진다(08-13 CI 실측 —
/// 파일 상단 "조건부 컴파일은 반대편을 의심한다"의 재발).
#[cfg(unix)]
static PANIC_HOOK: std::sync::Once = std::sync::Once::new();

/// **지금 즉시** 터미널을 복원한다(멱등 — 복원할 것이 없으면 no-op).
///
/// 정상 경로는 [`RawTerm`]의 `Drop`이 처리한다. 이 함수는 그 밖의 경로 몫이다:
/// 패닉 훅(`panic = "abort"`에서도 훅은 돈다) · 시그널 후 정리 · 이중 방어.
pub fn restore_now() {
    #[cfg(unix)]
    {
        let Ok(mut slot) = ACTIVE.lock() else { return };
        let Some(saved) = slot.take() else { return };
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[<u"); // kitty 확장 키 리포팅 pop
        let _ = out.flush();
        // SAFETY: 진입 때 저장해 둔 원본 설정을 그대로 복원한다.
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &saved);
        }
    }
}

/// raw 모드 진입/복원을 소유하는 가드. `Drop`에서 **반드시** 원래 설정으로 돌린다
/// (여기서 새면 사용자 셸이 망가진 채로 남는다).
#[derive(Debug)]
pub struct RawTerm {
    #[cfg(unix)]
    saved: Option<libc::termios>,
    /// 확장 키 리포팅을 켰는가(끌 때 되돌린다).
    extended: bool,
}

impl RawTerm {
    /// 표준 입력이 TTY면 raw 모드로 바꾸고 확장 키 리포팅을 요청한다.
    /// TTY가 아니거나 실패하면 **아무것도 바꾸지 않은 가드**를 돌려준다(호출 측은 그대로 진행).
    #[must_use]
    pub fn enter() -> Self {
        Self::enter_with(true)
    }

    /// **폴링 raw 모드** — `read`가 입력이 없으면 ~100ms 후 0을 돌려주고 돌아온다.
    ///
    /// 상대를 기다리는 동안처럼 **다른 일도 같이 봐야 하는 루프**에서 쓴다. 블로킹
    /// 모드로 기다리면 키 하나가 올 때까지 갇혀서, 그 사이 도착한 연결·종료 신호를
    /// 처리하지 못한다(그래서 대기 중에는 `/quit`도 Ctrl+D도 듣지 않았다 — 08-11).
    #[must_use]
    pub fn enter_polling() -> Self {
        Self::enter_with(false)
    }

    /// `blocking = false`면 `VMIN=0`·`VTIME=1`(0.1초)로 폴링 모드가 된다.
    #[must_use]
    fn enter_with(blocking: bool) -> Self {
        #[cfg(unix)]
        {
            if !is_tty() {
                return Self {
                    saved: None,
                    extended: false,
                };
            }
            let mut t: libc::termios = unsafe { core::mem::zeroed() };
            // SAFETY: 유효한 fd(0)와 우리가 소유한 구조체 포인터.
            if unsafe { libc::tcgetattr(0, &mut t) } != 0 {
                return Self {
                    saved: None,
                    extended: false,
                };
            }
            let saved = t;
            // 줄 조립(ICANON)·에코(ECHO)만 끈다. 신호(ISIG)는 남겨 Ctrl+C가 살아 있게 한다.
            t.c_lflag &= !(libc::ICANON | libc::ECHO);
            if blocking {
                t.c_cc[libc::VMIN] = 1;
                t.c_cc[libc::VTIME] = 0;
            } else {
                // VMIN=0·VTIME=1 — 0.1초 안에 온 것만 주고 없으면 0을 돌려준다.
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 1;
            }
            // SAFETY: 위와 동일.
            if unsafe { libc::tcsetattr(0, libc::TCSANOW, &t) } != 0 {
                return Self {
                    saved: None,
                    extended: false,
                };
            }
            // ★ 선제 청소(M1-8y ① · 08-17) — **SIGKILL은 어떤 훅도 못 돈다**:
            //   이전 실행이 kill -9로 죽었다면 kitty 플래그가 터미널 에뮬레이터에
            //   남아 있다(상태는 셸이 아니라 에뮬레이터 소유 — 08-13 실기 2회).
            //   진입 직전에 pop을 한 번 쏴서 잔존 상태를 걷어낸다. kitty 프로토콜은
            //   스택형이라 **빈 스택 pop은 무해**하고, 미지원 터미널은 무시한다.
            //   같은 TTY 다중 인스턴스의 pop 경합(M1-8y ③)도 이 청소가 흡수한다.
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x1b[<u");
            // kitty 키보드 프로토콜(플래그 1 = disambiguate escape codes) 요청.
            // 지원하지 않는 터미널은 이 시퀀스를 조용히 무시한다.
            let _ = out.write_all(b"\x1b[>1u");
            let _ = out.flush();
            // 전역 복원 레지스트리 + 패닉 훅(1회) — Drop이 못 도는 죽음(패닉 abort ·
            // 시그널 기본 동작)에서도 터미널이 kitty 모드로 남지 않게(실기 08-13).
            if let Ok(mut slot) = ACTIVE.lock() {
                *slot = Some(saved);
            }
            PANIC_HOOK.call_once(|| {
                let prev = std::panic::take_hook();
                std::panic::set_hook(Box::new(move |info| {
                    restore_now();
                    prev(info);
                }));
            });
            Self {
                saved: Some(saved),
                extended: true,
            }
        }
        #[cfg(not(unix))]
        {
            // Windows 콘솔 raw 모드는 별도 슬라이스(R-16 콘솔 핸들러와 함께).
            let _ = blocking;
            Self { extended: false }
        }
    }

    /// raw 모드가 실제로 켜졌는가(호출 측이 안내 문구를 바꾸는 근거).
    #[must_use]
    pub fn is_raw(&self) -> bool {
        #[cfg(unix)]
        {
            self.saved.is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

impl Drop for RawTerm {
    fn drop(&mut self) {
        // 정상 경로 복원 — 전역 레지스트리를 비우며 되돌린다(멱등 · 패닉 훅과 공유).
        #[cfg(unix)]
        if self.extended {
            restore_now();
        }
        // 비-unix에서 extended가 켜질 일은 아직 없지만(콘솔 raw는 R-16 후속),
        // 켜졌다면 pop만이라도 되돌린다.
        #[cfg(not(unix))]
        if self.extended {
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x1b[<u");
            let _ = out.flush();
        }
    }
}

/// 표준 입력이 터미널인가.
#[must_use]
pub fn is_tty() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: fd 0에 대한 조회만 한다.
        unsafe { libc::isatty(0) == 1 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// raw 모드에서 읽어 낸 키 한 벌.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermKey {
    /// 문자(UTF-8 조립 완료).
    Char(char),
    /// Enter(전송).
    Enter,
    /// **Shift+Enter**(줄바꿈) — 확장 키 리포팅이 있는 터미널에서만 온다.
    ShiftEnter,
    /// Backspace.
    Backspace,
    /// Ctrl+D(입력 종료).
    Eof,
    /// 그 외(무시).
    Other,
}

/// 입력 바이트 열에서 키를 하나 떼어 낸다 — `(키, 소비한 바이트 수)`.
/// 바이트가 모자라면 `None`(더 읽어야 한다).
///
/// 다루는 것: UTF-8 문자 · CR/LF · DEL/BS · Ctrl+D · **CSI u 시퀀스**
/// (`ESC [ 13 ; 2 u` = Shift+Enter). 그 외 이스케이프는 통째로 버린다.
#[must_use]
pub fn parse_key(buf: &[u8]) -> Option<(TermKey, usize)> {
    let first = *buf.first()?;
    match first {
        0x0D | 0x0A => Some((TermKey::Enter, 1)),
        0x7F | 0x08 => Some((TermKey::Backspace, 1)),
        0x04 => Some((TermKey::Eof, 1)),
        0x1B => {
            // CSI 시퀀스: ESC [ ... <final>
            if buf.len() < 2 {
                return None;
            }
            if buf[1] != b'[' {
                return Some((TermKey::Other, 2));
            }
            let mut i = 2;
            while i < buf.len() && !buf[i].is_ascii_alphabetic() && buf[i] != b'~' {
                i += 1;
            }
            if i >= buf.len() {
                return None; // 아직 끝나지 않았다
            }
            let final_byte = buf[i];
            let params = core::str::from_utf8(&buf[2..i]).unwrap_or("");
            let consumed = i + 1;
            // CSI u: `<code>;<modifiers>u` — 13 = Enter, 수식자 2 = Shift.
            if final_byte == b'u' {
                let mut it = params.split(';');
                let code = it.next().unwrap_or("").parse::<u32>().unwrap_or(0);
                let modifiers = it.next().unwrap_or("1").parse::<u32>().unwrap_or(1);
                if code == 13 {
                    // 수식자는 1 + 비트마스크(1=Shift). 2 = Shift만.
                    return Some((
                        if (modifiers - 1) & 1 == 1 {
                            TermKey::ShiftEnter
                        } else {
                            TermKey::Enter
                        },
                        consumed,
                    ));
                }
            }
            Some((TermKey::Other, consumed))
        }
        // 그 외 제어문자는 무시.
        c if c < 0x20 => Some((TermKey::Other, 1)),
        _ => {
            // UTF-8 문자 조립.
            let len = utf8_len(first);
            if buf.len() < len {
                return None;
            }
            core::str::from_utf8(&buf[..len])
                .ok()
                .and_then(|s| s.chars().next())
                .map_or(Some((TermKey::Other, 1)), |c| Some((TermKey::Char(c), len)))
        }
    }
}

/// UTF-8 선두 바이트 → 전체 길이.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_enter_and_shift_enter_are_distinguished() {
        assert_eq!(parse_key(b"\r"), Some((TermKey::Enter, 1)));
        // kitty/CSI u: ESC [ 13 ; 2 u = Shift+Enter.
        assert_eq!(
            parse_key(b"\x1b[13;2u"),
            Some((TermKey::ShiftEnter, 7)),
            "확장 리포팅이 있으면 구분된다"
        );
        // 수식자 없는 CSI u Enter는 그냥 Enter.
        assert_eq!(parse_key(b"\x1b[13u"), Some((TermKey::Enter, 5)));
    }

    #[test]
    fn utf8_multibyte_is_assembled() {
        let s = "한".as_bytes();
        assert_eq!(parse_key(s), Some((TermKey::Char('한'), 3)));
        // 조각만 있으면 더 기다린다.
        assert_eq!(parse_key(&s[..2]), None);
    }

    #[test]
    fn control_keys() {
        assert_eq!(parse_key(b"\x7f"), Some((TermKey::Backspace, 1)));
        assert_eq!(parse_key(b"\x08"), Some((TermKey::Backspace, 1)));
        assert_eq!(parse_key(b"\x04"), Some((TermKey::Eof, 1)));
        assert_eq!(parse_key(b"a"), Some((TermKey::Char('a'), 1)));
    }

    #[test]
    fn unknown_escape_is_consumed_not_stuck() {
        // 화살표 등 — 통째로 버리되 **소비량을 정확히** 돌려줘야 무한 루프가 안 난다.
        assert_eq!(parse_key(b"\x1b[A"), Some((TermKey::Other, 3)));
        assert_eq!(parse_key(b"\x1b[1;5C"), Some((TermKey::Other, 6)));
        // 미완 시퀀스는 더 읽는다.
        assert_eq!(parse_key(b"\x1b["), None);
    }

    #[test]
    fn ctrl_d_is_reported_so_callers_can_exit() {
        // 대기 구간·대화 구간 **양쪽**이 이걸로 빠져나간다(08-11 종료 경로 정리).
        assert_eq!(parse_key(b"\x04"), Some((TermKey::Eof, 1)));
    }

    #[test]
    fn polling_mode_guard_restores_like_blocking_one() {
        // TTY가 없는 CI에서는 둘 다 "아무것도 바꾸지 않은 가드"여야 한다 —
        // 여기서 터미널을 건드리면 테스트 러너의 셸이 망가진다.
        let a = RawTerm::enter_polling();
        let b = RawTerm::enter();
        if !is_tty() {
            assert!(
                !a.is_raw() && !b.is_raw(),
                "비-TTY에서는 raw로 들어가지 않는다"
            );
        }
    }

    #[test]
    fn empty_input_needs_more() {
        assert_eq!(parse_key(b""), None);
    }

    #[test]
    fn restore_now_is_idempotent_and_safe_without_tty() {
        // 패닉 훅·시그널 경로가 몇 번을 불러도, 복원할 것이 없어도 안전해야 한다
        // (실기 08-13 — kitty 프로토콜 누수의 이중 방어).
        restore_now();
        restore_now();
        let g = RawTerm::enter_polling();
        drop(g); // Drop 경로도 restore_now와 같은 레지스트리를 지난다(멱등 검증)
        restore_now();
    }
}
