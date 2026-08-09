//! 한글 2벌식 **직접 조합기**(사용자 확정 08-09 — 타입어헤드 IME 탈피).
//!
//! macOS IME는 조합 세션(marked text)의 시작·유지·확정이 전부 OS 소관이라, 한영 전환 직후
//! 첫 키 유실·타임아웃 후 묵은 조합 유입("김최") 같은 경합을 앱이 완전히 제어할 수 없었다
//! (실측 재현 4종 — journal 08-09). 그래서 **목록 타입어헤드는 IME를 끄고 raw 자모를 받아
//! 여기서 직접 조합한다** — 세션·preedit·stale이 없어져 타임아웃/ESC 초기화가 결정적이 된다.
//!
//! 표준 두벌식 오토마타: 초성→중성→종성, 겹모음(ㅗ+ㅏ=ㅘ)·겹받침(ㄹ+ㄱ=ㄺ),
//! **도깨비불**(받침이 다음 모음의 초성으로 이동: 갂+ㅣ → 가+끼 아님 · 김+ㅊㅗㅣ → 김최).
//! 백스페이스는 **자모 단위**(조합 중), 완성 글자는 밖(버퍼)에서 글자 단위로 지운다.

/// 초성 19자(유니코드 순).
const CHO: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];
/// 중성 21자(유니코드 순).
const JUNG: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ',
    'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];
/// 종성 27자(인덱스 1..=27 — 0은 받침 없음).
const JONG: [char; 27] = [
    'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ', 'ㅁ',
    'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
/// 겹모음: (기존, 추가) → 결합.
const JUNG_COMPOUND: [(char, char, char); 7] = [
    ('ㅗ', 'ㅏ', 'ㅘ'),
    ('ㅗ', 'ㅐ', 'ㅙ'),
    ('ㅗ', 'ㅣ', 'ㅚ'),
    ('ㅜ', 'ㅓ', 'ㅝ'),
    ('ㅜ', 'ㅔ', 'ㅞ'),
    ('ㅜ', 'ㅣ', 'ㅟ'),
    ('ㅡ', 'ㅣ', 'ㅢ'),
];
/// 겹받침: (기존, 추가) → 결합.
const JONG_COMPOUND: [(char, char, char); 11] = [
    ('ㄱ', 'ㅅ', 'ㄳ'),
    ('ㄴ', 'ㅈ', 'ㄵ'),
    ('ㄴ', 'ㅎ', 'ㄶ'),
    ('ㄹ', 'ㄱ', 'ㄺ'),
    ('ㄹ', 'ㅁ', 'ㄻ'),
    ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'),
    ('ㄹ', 'ㅌ', 'ㄾ'),
    ('ㄹ', 'ㅍ', 'ㄿ'),
    ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];

fn cho_idx(c: char) -> Option<usize> {
    CHO.iter().position(|&x| x == c)
}
fn jung_idx(c: char) -> Option<usize> {
    JUNG.iter().position(|&x| x == c)
}
fn jong_idx(c: char) -> Option<usize> {
    JONG.iter().position(|&x| x == c).map(|i| i + 1)
}
fn is_consonant(c: char) -> bool {
    cho_idx(c).is_some() || jong_idx(c).is_some()
}
fn is_vowel(c: char) -> bool {
    jung_idx(c).is_some()
}

/// 이 문자가 조합기가 다루는 자모인가(두벌식 키보드 원시 입력).
#[must_use]
pub fn is_jamo(c: char) -> bool {
    is_consonant(c) || is_vowel(c)
}

fn compose(cho: char, jung: char, jong: Option<char>) -> char {
    let c = cho_idx(cho).unwrap_or(0) as u32;
    let v = jung_idx(jung).unwrap_or(0) as u32;
    let t = jong.and_then(jong_idx).unwrap_or(0) as u32;
    char::from_u32(0xAC00 + c * 588 + v * 28 + t).unwrap_or('?')
}

fn jung_combine(a: char, b: char) -> Option<char> {
    JUNG_COMPOUND
        .iter()
        .find(|&&(x, y, _)| x == a && y == b)
        .map(|&(_, _, z)| z)
}
fn jong_combine(a: char, b: char) -> Option<char> {
    JONG_COMPOUND
        .iter()
        .find(|&&(x, y, _)| x == a && y == b)
        .map(|&(_, _, z)| z)
}
/// 겹받침 분해(도깨비불·백스페이스용): ㄺ → (ㄹ, ㄱ).
fn jong_split(j: char) -> Option<(char, char)> {
    JONG_COMPOUND
        .iter()
        .find(|&&(_, _, z)| z == j)
        .map(|&(x, y, _)| (x, y))
}

/// 조합 중 상태.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum State {
    #[default]
    Empty,
    /// 초성만(ㄱ).
    Cho(char),
    /// 모음만(ㅣ — 초성 없는 시작).
    Jung(char),
    /// 초성+중성(기).
    ChoJung(char, char),
    /// 초성+중성+종성(김).
    Full(char, char, char),
}

/// 두벌식 조합기 — 자모를 먹여 완성 글자를 뱉고, 조합 중 미리보기를 제공한다.
#[derive(Debug, Default)]
pub struct Composer {
    state: State,
}

impl Composer {
    /// 새 조합기.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 조합 중인가.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.state != State::Empty
    }

    /// 조합 중 미리보기(0~1글자) — 표시용.
    #[must_use]
    pub fn preview(&self) -> Option<char> {
        match self.state {
            State::Empty => None,
            State::Cho(c) | State::Jung(c) => Some(c),
            State::ChoJung(c, v) => Some(compose(c, v, None)),
            State::Full(c, v, j) => Some(compose(c, v, Some(j))),
        }
    }

    /// 조합 상태를 버린다(타임아웃·ESC — 결정적 초기화).
    pub fn reset(&mut self) {
        self.state = State::Empty;
    }

    /// 조합 중 글자를 확정 문자로 꺼내며 상태를 비운다.
    pub fn flush(&mut self) -> Option<char> {
        let out = self.preview();
        self.state = State::Empty;
        out
    }

    /// 자모 하나를 먹인다. **확정된 글자들**(0~2자)을 돌려준다(조합 중 글자는 [`Composer::preview`]).
    /// 자모가 아닌 문자는 조합을 확정(flush)한 뒤 그대로 돌려보낸다.
    pub fn feed(&mut self, ch: char) -> String {
        let mut out = String::new();
        if !is_jamo(ch) {
            if let Some(p) = self.flush() {
                out.push(p);
            }
            out.push(ch);
            return out;
        }
        if is_vowel(ch) {
            match self.state {
                State::Empty => self.state = State::Jung(ch),
                State::Jung(v) => {
                    if let Some(vv) = jung_combine(v, ch) {
                        self.state = State::Jung(vv);
                    } else {
                        out.push(v);
                        self.state = State::Jung(ch);
                    }
                }
                State::Cho(c) => self.state = State::ChoJung(c, ch),
                State::ChoJung(c, v) => {
                    if let Some(vv) = jung_combine(v, ch) {
                        self.state = State::ChoJung(c, vv);
                    } else {
                        out.push(compose(c, v, None));
                        self.state = State::Jung(ch);
                    }
                }
                State::Full(c, v, j) => {
                    // 도깨비불 — 받침(또는 겹받침 뒤쪽)이 다음 글자의 초성으로 이동.
                    if let Some((keep, moved)) = jong_split(j) {
                        out.push(compose(c, v, Some(keep)));
                        self.state = State::ChoJung(moved, ch);
                    } else if cho_idx(j).is_some() {
                        out.push(compose(c, v, None));
                        self.state = State::ChoJung(j, ch);
                    } else {
                        // 초성이 될 수 없는 받침 — 글자 확정 후 모음 단독 시작.
                        out.push(compose(c, v, Some(j)));
                        self.state = State::Jung(ch);
                    }
                }
            }
        } else {
            // 자음.
            match self.state {
                State::Empty => self.state = State::Cho(ch),
                State::Cho(c) => {
                    out.push(c); // 자음 연타 = 앞 자음 확정(두벌식 — 겹자음은 시프트 입력)
                    self.state = State::Cho(ch);
                }
                State::Jung(v) => {
                    out.push(v);
                    self.state = State::Cho(ch);
                }
                State::ChoJung(c, v) => {
                    if jong_idx(ch).is_some() {
                        self.state = State::Full(c, v, ch);
                    } else {
                        out.push(compose(c, v, None)); // ㄸ 등 받침 불가 자음
                        self.state = State::Cho(ch);
                    }
                }
                State::Full(c, v, j) => {
                    if let Some(jj) = jong_combine(j, ch) {
                        self.state = State::Full(c, v, jj);
                    } else {
                        out.push(compose(c, v, Some(j)));
                        self.state = State::Cho(ch);
                    }
                }
            }
        }
        out
    }

    /// 자모 단위 백스페이스 — 조합 중이면 마지막 자모를 떼고 `true`, 아니면 `false`.
    pub fn backspace(&mut self) -> bool {
        match self.state {
            State::Empty => false,
            State::Cho(_) | State::Jung(_) => {
                self.state = State::Empty;
                true
            }
            State::ChoJung(c, _) => {
                self.state = State::Cho(c);
                true
            }
            State::Full(c, v, j) => {
                self.state = match jong_split(j) {
                    Some((keep, _)) => State::Full(c, v, keep),
                    None => State::ChoJung(c, v),
                };
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_all(cm: &mut Composer, jamo: &str) -> String {
        let mut out = String::new();
        for c in jamo.chars() {
            out.push_str(&cm.feed(c));
        }
        out
    }
    fn done(cm: &mut Composer, jamo: &str) -> String {
        let mut out = type_all(cm, jamo);
        if let Some(p) = cm.flush() {
            out.push(p);
        }
        out
    }

    #[test]
    fn basic_syllables() {
        assert_eq!(done(&mut Composer::new(), "ㄱㅣㅁ"), "김");
        assert_eq!(done(&mut Composer::new(), "ㅊㅗㅣ"), "최"); // ㅗ+ㅣ=ㅚ… 아니고 ㅊㅗㅣ→최(ㅚ)
        assert_eq!(done(&mut Composer::new(), "ㅎㅏㄴㄱㅡㄹ"), "한글");
        assert_eq!(done(&mut Composer::new(), "ㄱㅏㅂㅅ"), "값"); // 겹받침 ㅄ
        assert_eq!(done(&mut Composer::new(), "ㅇㅣ"), "이");
    }

    #[test]
    fn dokkaebi_final_moves_to_next_cho() {
        // 김 + ㅊㅗㅣ = "김최"가 아니라... 각 글자 경계: ㄱㅣㅁ ㅊㅗㅣ → 김최.
        assert_eq!(done(&mut Composer::new(), "ㄱㅣㅁㅊㅗㅣ"), "김최");
        // 도깨비불: ㄱㅏㄱ + ㅣ → 가기.
        assert_eq!(done(&mut Composer::new(), "ㄱㅏㄱㅣ"), "가기");
        // 겹받침 분해: 앉+ㅏ → 안자.
        assert_eq!(done(&mut Composer::new(), "ㅇㅏㄴㅈㅏ"), "안자");
    }

    #[test]
    fn user_repro_cases_are_deterministic() {
        // 사용자 재현 케이스(08-09): 조합기는 상태가 자기 소유라 언제 리셋해도 결정적이다.
        // 1) bob → (타임아웃 = 밖에서 버퍼 소거·composer.reset) → 김.
        let mut cm = Composer::new();
        assert_eq!(done(&mut cm, "ㄱㅣㅁ"), "김");
        // 2) 최 → 리셋 → 김 → 리셋 → 이.
        cm.reset();
        assert_eq!(done(&mut cm, "ㅊㅗㅣ"), "최");
        cm.reset();
        assert_eq!(done(&mut cm, "ㄱㅣㅁ"), "김");
        cm.reset();
        assert_eq!(done(&mut cm, "ㅇㅣ"), "이");
        // 리셋 직후 잔여 상태 없음 = "김최" 유형 불가능.
    }

    #[test]
    fn backspace_is_jamo_wise_while_composing() {
        let mut cm = Composer::new();
        type_all(&mut cm, "ㄱㅣㅁ");
        assert_eq!(cm.preview(), Some('김'));
        assert!(cm.backspace());
        assert_eq!(cm.preview(), Some('기'));
        assert!(cm.backspace());
        assert_eq!(cm.preview(), Some('ㄱ'));
        assert!(cm.backspace());
        assert_eq!(cm.preview(), None);
        assert!(!cm.backspace(), "빈 상태 = 미처리(밖에서 글자 단위)");
    }

    #[test]
    fn non_jamo_flushes_and_passes_through() {
        let mut cm = Composer::new();
        let mut out = type_all(&mut cm, "ㄱㅣ");
        out.push_str(&cm.feed('b'));
        assert_eq!(out, "기b", "조합 확정 후 ASCII 통과");
        assert!(!cm.is_composing());
    }
}
