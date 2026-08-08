//! 표시 이름 — **무해화된** 사용자/노드 이름([docs/13] §4 "parse, don't validate" · FR-S-13).
//!
//! 이름은 신원이 아니다(신원은 [`crate::PeerId`]). 화면·목록에 뜨는 문자열일 뿐이며,
//! **양방향 오버라이드(RLO 등)·제어문자·0폭 문자를 제거**해 표시가 실제와 달라 보이는 위장을 막는다.
//! 생성에 성공했다는 사실이 곧 무해화 완료다 — 이후 도메인은 이름을 다시 검사하지 않는다.

use core::fmt;

/// 표시 이름의 문자 수 상한(무해화 후 기준).
pub const MAX_NAME_CHARS: usize = 64;

/// 이름 무해화 실패 사유.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    /// 무해화 후 빈 문자열(표시할 게 없음).
    Empty,
    /// 상한 초과.
    TooLong,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NameError::Empty => f.write_str("이름이 비어 있음(무해화 후)"),
            NameError::TooLong => f.write_str("이름이 너무 김"),
        }
    }
}

impl std::error::Error for NameError {}

/// 무해화된 표시 이름. `parse`로만 만들 수 있어 **항상 안전**하다.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DisplayName(String);

impl DisplayName {
    /// 원시 문자열을 무해화해 표시 이름을 만든다.
    ///
    /// 제거: **양방향 제어**(U+202A–202E · U+2066–2069) · **0폭**(U+200B–200D · U+FEFF) ·
    /// 기타 제어문자(개행·탭 포함). 앞뒤 공백은 트림하고 연속 공백은 한 칸으로 접는다.
    ///
    /// # Errors
    /// 무해화 후 비면 [`NameError::Empty`], 상한 초과면 [`NameError::TooLong`].
    pub fn parse(raw: &str) -> Result<Self, NameError> {
        let mut out = String::with_capacity(raw.len());
        let mut prev_space = false;
        for ch in raw.chars() {
            if is_stripped(ch) {
                continue;
            }
            if ch.is_whitespace() {
                if !out.is_empty() && !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
                continue;
            }
            out.push(ch);
            prev_space = false;
        }
        // 끝의 접힌 공백 제거.
        if out.ends_with(' ') {
            out.pop();
        }
        if out.is_empty() {
            return Err(NameError::Empty);
        }
        if out.chars().count() > MAX_NAME_CHARS {
            return Err(NameError::TooLong);
        }
        Ok(Self(out))
    }

    /// 무해화된 문자열.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DisplayName({:?})", self.0)
    }
}

/// 표시에서 통째로 제거하는 문자 — 양방향 제어·0폭·비공백 제어.
///
/// 공백류 제어(스페이스·탭·개행)는 여기서 제거하지 않고 [`DisplayName::parse`]의
/// 공백 분기가 **한 칸으로 접는다**(줄바꿈으로 표시를 왜곡하지 못하게).
fn is_stripped(ch: char) -> bool {
    matches!(ch,
        // 양방향 오버라이드/임베딩/아이솔레이트
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        // 0폭 공백·조이너·BOM
        | '\u{200B}'..='\u{200D}' | '\u{FEFF}'
    ) || (ch.is_control() && !ch.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_name_passes() {
        assert_eq!(
            DisplayName::parse("홍길동-맥북").unwrap().as_str(),
            "홍길동-맥북"
        );
    }

    #[test]
    fn rlo_is_stripped() {
        // "exe\u{202E}txt." — RLO로 확장자 위장 시도. 제거 후 그대로 순방향.
        let n = DisplayName::parse("report\u{202E}fdp.exe").unwrap();
        assert!(!n.as_str().contains('\u{202E}'), "RLO 제거");
        assert_eq!(n.as_str(), "reportfdp.exe");
    }

    #[test]
    fn zero_width_and_controls_stripped() {
        let n = DisplayName::parse("a\u{200B}b\tc\u{0007}d").unwrap();
        assert_eq!(n.as_str(), "ab cd"); // 탭은 공백으로, 0폭·벨은 제거
    }

    #[test]
    fn whitespace_trimmed_and_collapsed() {
        assert_eq!(
            DisplayName::parse("  홍  길동  ").unwrap().as_str(),
            "홍 길동"
        );
    }

    #[test]
    fn empty_after_sanitize_errors() {
        assert_eq!(
            DisplayName::parse("\u{202E}\u{200B}  ").unwrap_err(),
            NameError::Empty
        );
        assert_eq!(DisplayName::parse("").unwrap_err(), NameError::Empty);
    }

    #[test]
    fn too_long_errors() {
        let long = "가".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(DisplayName::parse(&long).unwrap_err(), NameError::TooLong);
        // 경계값은 통과.
        assert!(DisplayName::parse(&"가".repeat(MAX_NAME_CHARS)).is_ok());
    }
}
