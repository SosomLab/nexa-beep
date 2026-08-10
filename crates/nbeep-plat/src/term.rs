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

#[cfg(unix)]
use std::io::Write as _;

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
            t.c_cc[libc::VMIN] = 1;
            t.c_cc[libc::VTIME] = 0;
            // SAFETY: 위와 동일.
            if unsafe { libc::tcsetattr(0, libc::TCSANOW, &t) } != 0 {
                return Self {
                    saved: None,
                    extended: false,
                };
            }
            // kitty 키보드 프로토콜(플래그 1 = disambiguate escape codes) 요청.
            // 지원하지 않는 터미널은 이 시퀀스를 조용히 무시한다.
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x1b[>1u");
            let _ = out.flush();
            Self {
                saved: Some(saved),
                extended: true,
            }
        }
        #[cfg(not(unix))]
        {
            // Windows 콘솔 raw 모드는 별도 슬라이스(R-16 콘솔 핸들러와 함께).
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
        if self.extended {
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x1b[<u"); // 확장 키 리포팅 해제
            let _ = out.flush();
        }
        #[cfg(unix)]
        if let Some(saved) = self.saved {
            // SAFETY: 진입 때 저장해 둔 원본 설정을 그대로 복원한다.
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &saved);
            }
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
    fn empty_input_needs_more() {
        assert_eq!(parse_key(b""), None);
    }
}
