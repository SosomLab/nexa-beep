//! **파일명 정규화** — 실체화 직전 이름 무해화([docs/11 §4] 실체화 절차 2).
//!
//! 수신 파일명은 **공격 벡터**다: RLO(U+202E)로 `exe`를 `txt`처럼 보이게 뒤집고,
//! 경로 구분자로 폴더를 탈출하고, `CON`·`NUL` 같은 Windows 장치명으로 오동작을 유발하고,
//! `:`로 NTFS ADS에 숨는다. 여기서는 **표시가 아니라 실체화용 이름**을 만든다 —
//! 원본 바이트는 [`crate::Meta::orig_name`]에 그대로 남는다(감사용).
//!
//! 순수 함수 — 파일시스템 접근 없음. 충돌 회피(뒤 번호)는 어댑터(슬라이스 4) 몫.

/// 정규화 결과 이름의 최대 길이(문자 수 · 확장자 포함).
pub const MAX_NAME_CHARS: usize = 120;

/// 비어 버린 이름의 대체값.
const FALLBACK: &str = "file";

/// Windows 예약 장치명(대소문자 무관 · **확장자를 떼고** 비교 — `CON.txt`도 예약).
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// 제거 대상 — 방향 제어(RLO 계열)·0폭·BOM.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        // bidi 제어(LRE·RLE·PDF·LRO·RLO · LRI·RLI·FSI·PDI).
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        // 0폭(ZWSP·ZWNJ·ZWJ·LRM·RLM) · BOM.
        | '\u{200B}'..='\u{200F}' | '\u{FEFF}'
    )
}

/// 치환 대상 — 경로 구분자·ADS 구분자·OS 금지 문자.
fn is_separator_like(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
}

/// 실체화용 파일명 정규화 — 항상 쓸 수 있는 이름을 돌려준다(빈 결과 = `file`).
///
/// 규칙([docs/11 §4]): RLO·제어문자·0폭 **제거** · 경로/ADS 구분자 `_` 치환 ·
/// Windows 예약 장치명 앞에 `_` · 끝의 점·공백 제거 · [`MAX_NAME_CHARS`] 상한
/// (자를 땐 **확장자를 보존**).
#[must_use]
pub fn sanitize_filename(raw: &str) -> String {
    // 1) 문자 단위 필터 — 보이지 않는 것 제거, 구분자류·제어문자 치환.
    let mut name: String = raw
        .chars()
        .filter(|&c| !is_invisible(c))
        .map(|c| {
            if is_separator_like(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();

    // 2) 끝의 점·공백 제거(Windows가 조용히 떼어 다른 파일이 된다).
    while name.ends_with(['.', ' ']) {
        name.pop();
    }
    // 선두 공백도 정리.
    let name = name.trim_start().to_string();

    // 3) 비었으면 대체.
    let mut name = if name.is_empty() {
        FALLBACK.to_string()
    } else {
        name
    };

    // 4) 예약 장치명 회피 — 확장자를 뗀 몸통으로 비교(`CON`·`CON.txt` 모두).
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }

    // 5) 길이 상한 — 확장자 보존(확장자가 비정상적으로 길면 그쪽도 잘린다).
    if name.chars().count() > MAX_NAME_CHARS {
        let (stem_part, ext_part) = match name.rsplit_once('.') {
            Some((st, ex)) if !st.is_empty() => (st.to_string(), format!(".{ex}")),
            _ => (name.clone(), String::new()),
        };
        let ext_keep: String = ext_part.chars().take(MAX_NAME_CHARS / 2).collect();
        let stem_budget = MAX_NAME_CHARS - ext_keep.chars().count();
        let stem_keep: String = stem_part.chars().take(stem_budget).collect();
        name = format!("{stem_keep}{ext_keep}");
        // 절단으로 끝 점이 노출될 수 있다 — 한 번 더 정리.
        while name.ends_with(['.', ' ']) {
            name.pop();
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlo_spoof_is_neutralized() {
        // "invoice_\u{202E}txt.exe" — RLO로 "exe.txt"처럼 보이던 이름.
        let out = sanitize_filename("invoice_\u{202E}txt.exe");
        assert_eq!(
            out, "invoice_txt.exe",
            "RLO 제거 = 실체(exe)가 그대로 보인다"
        );
    }

    #[test]
    fn separators_and_ads_become_underscore() {
        assert_eq!(sanitize_filename("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(sanitize_filename("a\\b\\c.txt"), "a_b_c.txt");
        assert_eq!(
            sanitize_filename("doc.txt:hidden.exe"),
            "doc.txt_hidden.exe"
        );
    }

    #[test]
    fn windows_reserved_names_prefixed() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("Lpt9.log"), "_Lpt9.log");
        assert_eq!(
            sanitize_filename("console.txt"),
            "console.txt",
            "부분 일치 아님"
        );
    }

    #[test]
    fn trailing_dots_spaces_and_empty() {
        assert_eq!(sanitize_filename("report.pdf. . ."), "report.pdf");
        assert_eq!(sanitize_filename("   "), "file");
        assert_eq!(
            sanitize_filename("\u{202E}\u{200B}"),
            "file",
            "보이지 않는 것만 = 대체"
        );
    }

    #[test]
    fn control_chars_replaced_zero_width_removed() {
        assert_eq!(sanitize_filename("a\u{0}b\nc.txt"), "a_b_c.txt");
        assert_eq!(sanitize_filename("na\u{200B}me.txt"), "name.txt");
    }

    #[test]
    fn length_capped_with_extension_preserved() {
        let long = format!("{}.tar.gz", "가".repeat(300));
        let out = sanitize_filename(&long);
        assert!(out.chars().count() <= MAX_NAME_CHARS);
        assert!(out.ends_with(".gz"), "확장자 보존: {out}");
    }
}
