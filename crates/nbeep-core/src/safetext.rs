//! 메시지 본문 무해화 + 링크 추출 — M2-6(FR-S-13 · FR-S-14).
//!
//! [`crate::name::DisplayName`]과 목적은 같지만 규칙이 다르다 — **이름은 한 줄 라벨**이라 공백을
//! 한 칸으로 접지만, **메시지는 문단**이라 개행·탭을 보존해야 한다. 공통 위협(RLO 등 양방향
//! 오버라이드·0폭 문자·제어문자)만 같은 기준으로 중화한다.
//!
//! 링크는 **표시만** 한다 — 자동으로 열지 않고, 클릭 시 UI가 전체 URL을 보여주고 확인받는다
//! (FR-S-14). 이 모듈은 그 UI가 쓸 **탐지 결과**(범위·스킴)만 제공한다. 열기 API는 여기 없다.

/// 메시지 본문의 문자 수 상한(무해화 후). 초과분은 자른다 — 거부하면 대화가 끊긴다.
pub const MAX_MESSAGE_CHARS: usize = 16_384;

/// 무해화된 메시지 본문. [`sanitize_message`]로만 만들 수 있다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafeText {
    text: String,
    truncated: bool,
}

impl SafeText {
    /// 무해화된 문자열.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// 상한 초과로 잘렸는가 — UI가 "…(잘림)"을 표시할 근거.
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// 수신 메시지 본문을 표시용으로 무해화한다.
///
/// - 제거: **양방향 제어**(U+202A–202E · U+2066–2069) · **0폭**(U+200B–200D · U+FEFF) ·
///   비공백 제어문자(단 `\n`·`\t`는 보존 — 메시지는 문단이다).
/// - `\r\n`·`\r`은 `\n`으로 정규화한다.
/// - [`MAX_MESSAGE_CHARS`] 초과분은 자르고 [`SafeText::truncated`]에 표시한다.
#[must_use]
pub fn sanitize_message(raw: &str) -> SafeText {
    let mut out = String::with_capacity(raw.len().min(MAX_MESSAGE_CHARS * 4));
    let mut count = 0usize;
    let mut truncated = false;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        let ch = match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next(); // CRLF → LF
                }
                '\n'
            }
            c if is_neutralized(c) => continue,
            c => c,
        };
        if count >= MAX_MESSAGE_CHARS {
            truncated = true;
            break;
        }
        out.push(ch);
        count += 1;
    }
    SafeText {
        text: out,
        truncated,
    }
}

/// 중화(제거) 대상 — [`crate::name`]의 기준과 동일하되 `\n`·`\t`는 살린다.
fn is_neutralized(ch: char) -> bool {
    matches!(ch,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        | '\u{200B}'..='\u{200D}' | '\u{FEFF}'
    ) || (ch.is_control() && ch != '\n' && ch != '\t')
}

/// 본문 속 링크 하나 — UI가 "클릭 시 전체 URL 확인"을 그릴 재료(FR-S-14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    /// [`SafeText::as_str`] 기준 바이트 범위 시작.
    pub start: usize,
    /// 바이트 범위 끝(exclusive).
    pub end: usize,
}

/// 무해화된 본문에서 **http/https 링크만** 찾는다.
///
/// 스킴 화이트리스트다 — `file:`·`javascript:` 등은 링크로 취급하지 않는다(일반 텍스트로 표시).
/// 자동 열기는 어디에도 없다 — 이 함수는 범위만 보고한다.
#[must_use]
pub fn find_links(safe: &SafeText) -> Vec<LinkSpan> {
    let text = safe.as_str();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let Some(rel) = rest.find("http") else { break };
        let start = at + rel;
        let after = &text[start..];
        let scheme_len = if after.starts_with("https://") {
            8
        } else if after.starts_with("http://") {
            7
        } else {
            at = start + 4;
            continue;
        };
        // 단어 경계 확인 — "xhttp://…" 같은 붙은 꼬리는 링크가 아니다.
        if start > 0 {
            let prev = text[..start].chars().next_back().unwrap_or(' ');
            if prev.is_alphanumeric() {
                at = start + scheme_len;
                continue;
            }
        }
        let body = &text[start + scheme_len..];
        let end_rel = body
            .char_indices()
            .find(|(_, c)| c.is_whitespace() || is_url_terminator(*c))
            .map_or(body.len(), |(i, _)| i);
        if end_rel == 0 {
            at = start + scheme_len; // 스킴만 있고 본문 없음
            continue;
        }
        let end = start + scheme_len + end_rel;
        spans.push(LinkSpan { start, end });
        at = end;
    }
    spans
}

/// URL을 끝내는 것으로 취급하는 문자(문장 부호로 URL이 끝나는 흔한 표기).
fn is_url_terminator(c: char) -> bool {
    matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '}' | '`')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_paragraph_structure_unlike_names() {
        let s = sanitize_message("첫 줄\n\t들여쓴 둘째 줄\r\n셋째");
        assert_eq!(
            s.as_str(),
            "첫 줄\n\t들여쓴 둘째 줄\n셋째",
            "개행·탭 보존, CRLF 정규화"
        );
        assert!(!s.truncated());
    }

    #[test]
    fn strips_bidi_zero_width_and_controls() {
        // RLO로 "exe.txt"를 "txt.exe"처럼 보이게 하는 위장(FR-S-13의 원형)을 중화.
        let s = sanitize_message("파일: \u{202E}txt.gpj\u{202C} 끝\u{200B}\u{0007}");
        assert_eq!(s.as_str(), "파일: txt.gpj 끝");
    }

    #[test]
    fn truncates_at_cap_and_reports() {
        let long: String = "가".repeat(MAX_MESSAGE_CHARS + 5);
        let s = sanitize_message(&long);
        assert_eq!(s.as_str().chars().count(), MAX_MESSAGE_CHARS);
        assert!(s.truncated(), "잘림 사실을 UI에 알린다");
    }

    #[test]
    fn finds_only_http_https_links() {
        let s = sanitize_message(
            "보기: https://example.com/a?x=1 그리고 http://b.kr 끝. file:///etc javascript:x",
        );
        let links: Vec<&str> = find_links(&s)
            .iter()
            .map(|l| &s.as_str()[l.start..l.end])
            .collect();
        assert_eq!(
            links,
            vec!["https://example.com/a?x=1", "http://b.kr"],
            "화이트리스트 스킴만"
        );
    }

    #[test]
    fn link_boundaries_respect_punctuation_and_words() {
        let s = sanitize_message("(https://a.io) xhttps://not.link https:// 뿐");
        let links: Vec<&str> = find_links(&s)
            .iter()
            .map(|l| &s.as_str()[l.start..l.end])
            .collect();
        assert_eq!(
            links,
            vec!["https://a.io"],
            "괄호로 끝·붙은 꼬리 제외·빈 본문 제외"
        );
    }

    #[test]
    fn no_open_api_exists_only_spans() {
        // FR-S-14 구조 보증 — 이 모듈의 공개 표면은 탐지뿐이다(열기 API 부재는 컴파일 표면이 증명).
        let s = sanitize_message("https://a.io");
        let spans = find_links(&s);
        assert_eq!(spans[0], LinkSpan { start: 0, end: 12 });
    }
}
