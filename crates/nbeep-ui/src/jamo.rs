//! 한글 **자모열 검색**(10-09 · nexa-sql `nsql-core::hangul` 이식 — 설정 검색 "조합 중 글자도 맞는다").
//!
//! 질의와 대상을 둘 다 입력 순서의 자모열(초·중·종 · 겹모음·겹받침은 둘로)로 펴서 부분열 대조한다.
//! "ㄱ"·"기"·"긴" 모두 "긴급"에 맞는다. 한글이 없는 질의는 호출자가 보통의 부분 문자열 검색을 쓴다.

const CHO: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];
const JUNG: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ',
    'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];
const JONG: [char; 27] = [
    'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ', 'ㅁ',
    'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
/// 겹모음 → 두 모음.
const JUNG_SPLIT: [(char, char, char); 7] = [
    ('ㅘ', 'ㅗ', 'ㅏ'),
    ('ㅙ', 'ㅗ', 'ㅐ'),
    ('ㅚ', 'ㅗ', 'ㅣ'),
    ('ㅝ', 'ㅜ', 'ㅓ'),
    ('ㅞ', 'ㅜ', 'ㅔ'),
    ('ㅟ', 'ㅜ', 'ㅣ'),
    ('ㅢ', 'ㅡ', 'ㅣ'),
];
/// 겹받침 → 두 자음.
const JONG_SPLIT: [(char, char, char); 11] = [
    ('ㄳ', 'ㄱ', 'ㅅ'),
    ('ㄵ', 'ㄴ', 'ㅈ'),
    ('ㄶ', 'ㄴ', 'ㅎ'),
    ('ㄺ', 'ㄹ', 'ㄱ'),
    ('ㄻ', 'ㄹ', 'ㅁ'),
    ('ㄼ', 'ㄹ', 'ㅂ'),
    ('ㄽ', 'ㄹ', 'ㅅ'),
    ('ㄾ', 'ㄹ', 'ㅌ'),
    ('ㄿ', 'ㄹ', 'ㅍ'),
    ('ㅀ', 'ㄹ', 'ㅎ'),
    ('ㅄ', 'ㅂ', 'ㅅ'),
];

fn split2(table: &[(char, char, char)], c: char) -> Option<(char, char)> {
    table
        .iter()
        .find(|(k, _, _)| *k == c)
        .map(|(_, a, b)| (*a, *b))
}

/// 완성형 음절인가.
#[must_use]
pub fn is_syllable(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

/// 호환 자모(ㄱ~ㅣ)인가.
#[must_use]
pub fn is_compat_jamo(c: char) -> bool {
    ('\u{3131}'..='\u{318E}').contains(&c)
}

/// 한글(음절·자모)이 들어 있는가 — 자모열 검색을 쓸지 정하는 기준.
#[must_use]
pub fn has_hangul(s: &str) -> bool {
    s.chars().any(|c| is_syllable(c) || is_compat_jamo(c))
}

/// 대소문자 접기(글자 수가 변하지 않는 소문자화만).
fn fold_char(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// 글자 하나를 자모열로 편다(`fold`면 한글 아닌 글자는 소문자).
fn decompose_into(c: char, fold: bool, out: &mut Vec<char>) {
    if is_syllable(c) {
        let s = c as u32 - 0xAC00;
        let (ci, vi, ti) = (
            (s / 588) as usize,
            ((s % 588) / 28) as usize,
            (s % 28) as usize,
        );
        out.push(CHO[ci]);
        match split2(&JUNG_SPLIT, JUNG[vi]) {
            Some((a, b)) => out.extend([a, b]),
            None => out.push(JUNG[vi]),
        }
        if ti > 0 {
            match split2(&JONG_SPLIT, JONG[ti - 1]) {
                Some((a, b)) => out.extend([a, b]),
                None => out.push(JONG[ti - 1]),
            }
        }
    } else if let Some((a, b)) = split2(&JUNG_SPLIT, c).or_else(|| split2(&JONG_SPLIT, c)) {
        out.extend([a, b]);
    } else {
        out.push(if fold { fold_char(c) } else { c });
    }
}

/// 문자열 → 자모열(질의 준비용).
#[must_use]
pub fn decompose(s: &str, fold: bool) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        decompose_into(c, fold, &mut out);
    }
    out
}

/// `hay`에 자모열 `q`(이미 [`decompose`]한 질의 · `fold`도 같게)가 **글자 경계에서 시작**해 부분열로 들어 있는가.
#[must_use]
pub fn contains_jamo(hay: &str, q: &[char], fold: bool) -> bool {
    if q.is_empty() {
        return true;
    }
    let mut jam: Vec<char> = Vec::with_capacity(hay.len() * 2);
    let mut origin: Vec<usize> = Vec::with_capacity(hay.len() * 2);
    for (i, c) in hay.chars().enumerate() {
        let n0 = jam.len();
        decompose_into(c, fold, &mut jam);
        origin.extend(std::iter::repeat_n(i, jam.len() - n0));
    }
    let n = q.len();
    let mut j = 0usize;
    while j + n <= jam.len() {
        let at_boundary = j == 0 || origin[j] != origin[j - 1];
        if at_boundary && jam[j..j + n] == *q {
            return true;
        }
        j += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_syllables_and_compounds() {
        assert_eq!(decompose("가", false), vec!['ㄱ', 'ㅏ']);
        assert_eq!(decompose("긴", false), vec!['ㄱ', 'ㅣ', 'ㄴ']);
        assert_eq!(decompose("닭", false), vec!['ㄷ', 'ㅏ', 'ㄹ', 'ㄱ']);
        assert_eq!(decompose("과A", true), vec!['ㄱ', 'ㅗ', 'ㅏ', 'a']);
        assert_eq!(decompose("ㄳ", false), vec!['ㄱ', 'ㅅ']);
        assert!(has_hangul("x기"));
        assert!(has_hangul("ㄱ"));
        assert!(!has_hangul("abc"));
    }

    #[test]
    fn partial_syllable_matches_at_char_boundary() {
        let hay = "가나 기타 긴급 닭";
        for q in ["ㄱ", "기", "긴", "긴ㄱ", "긴급", "닭", "달", "ㄷㅏㄹㄱ"] {
            assert!(contains_jamo(hay, &decompose(q, true), true), "{q}");
        }
        // 글자 경계 안에서 시작하지 않는 자모열은 맞지 않는다("ㅏㄴ" = 가+나의 중간).
        assert!(!contains_jamo(hay, &decompose("ㅏㄴ", true), true));
        assert!(!contains_jamo(hay, &decompose("김", true), true));
        // 대소문자 접기.
        assert!(contains_jamo("Theme 테마", &decompose("THEME", true), true));
    }
}
