//! 민감 값 마스킹 — `Debug`/`Display`가 내용을 노출하지 않게([docs/13] §7).
//!
//! **규약**: 개인키·세션 키·메시지 본문·파일 경로 등 민감 타입은 `Debug`/`Display`를 **수동 구현**해
//! 실제 값을 찍지 않는다(로그·계측·크래시 리포트로 새는 것을 원천 차단). `#[derive(Debug)]` 금지.
//!
//! 이 모듈은 그 수동 구현을 **한 줄로** 만들어 주는 재사용 헬퍼다. 앞으로 `nbeep-crypto`의 키,
//! `nbeep-core`의 메시지 본문이 이걸 쓴다(공개키 지문 `PeerId`/`UserId`는 비밀이 아니므로 별개 — 짧은 hex).

use core::fmt;

/// 바이트 슬라이스를 **길이만** 드러내고 내용은 가리는 `Debug` 래퍼.
///
/// ```
/// use nbeep_core::redact::Redacted;
/// let key = [0xABu8; 32];
/// assert_eq!(format!("{:?}", Redacted(&key)), "[32 bytes redacted]");
/// ```
pub struct Redacted<'a>(pub &'a [u8]);

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{} bytes redacted]", self.0.len())
    }
}

impl fmt::Display for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// 문자열/텍스트를 **문자 수만** 드러내는 `Debug` 래퍼(내용 없음).
///
/// ```
/// use nbeep_core::redact::RedactedText;
/// assert_eq!(format!("{:?}", RedactedText("비밀 메시지")), "[6 chars redacted]");
/// ```
pub struct RedactedText<'a>(pub &'a str);

impl fmt::Debug for RedactedText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{} chars redacted]", self.0.chars().count())
    }
}

impl fmt::Display for RedactedText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_bytes_show_length_only() {
        let secret = [0x42u8; 16];
        let s = format!("{:?}", Redacted(&secret));
        assert_eq!(s, "[16 bytes redacted]");
        assert!(!s.contains("42"), "내용 노출 금지: {s}");
    }

    #[test]
    fn redacted_bytes_empty() {
        assert_eq!(format!("{:?}", Redacted(&[])), "[0 bytes redacted]");
    }

    #[test]
    fn redacted_text_shows_char_count_not_bytes() {
        // 한글은 UTF-8 3바이트지만 문자 수로 센다(길이 유추도 최소화).
        let s = format!("{:?}", RedactedText("안녕"));
        assert_eq!(s, "[2 chars redacted]");
        assert!(!s.contains("안"), "내용 노출 금지: {s}");
    }

    #[test]
    fn display_matches_debug() {
        let secret = [1u8; 8];
        assert_eq!(
            format!("{}", Redacted(&secret)),
            format!("{:?}", Redacted(&secret))
        );
    }
}
