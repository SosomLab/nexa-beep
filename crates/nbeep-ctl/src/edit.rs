//! 텍스트 편집 모델 — 캐럿·선택·삽입·삭제([docs/12 §A] `nexa-gui/edit.rs` 이식).
//!
//! **순수 로직**(레이아웃·히트테스트 없음 — 그건 위젯이 폰트로 실측). 문자(`char`) 단위 버퍼라
//! 한글·이모지 경계가 자연히 지켜진다(UTF-8 바이트 인덱싱의 함정 회피). **IME 연결 지점**(M3-3):
//! 조합 확정 문자열은 [`EditState::insert_str`], 프리에딧 표시는 위젯이 이 상태 위에 얹는다.
//!
//! 원본에서 뺀 것: paint 캐시 기반 클릭 히트테스트(위젯이 폰트 실측으로 대체) · 드래그 선택.

/// 이동/편집 키(플랫폼 중립 — [`crate::event::Key`]에서 번역).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    /// 캐럿 왼쪽(선택 있으면 왼쪽 가장자리로 접기).
    Left,
    /// 캐럿 오른쪽.
    Right,
    /// 줄 시작.
    Home,
    /// 줄 끝.
    End,
    /// 전체 선택.
    SelectAll,
    /// Delete(앞으로 삭제).
    DeleteForward,
}

/// 편집 상태 — 버퍼(char)·캐럿(`0..=len`)·선택(anchor↔caret).
#[derive(Clone, Debug, Default)]
pub struct EditState {
    buf: Vec<char>,
    caret: usize,
    anchor: Option<usize>,
    /// IME 조합 중 문자열(M3-1e ① — TextBox·대화 입력 공용). **표시 전용**:
    /// 편집 버퍼(`buf`)에 들어가지 않고, `display_text`가 캐럿 자리에 끼워 보인다.
    /// 확정 문자는 `insert`로 버퍼에 들어오고 조합은 끝난다(호출측이 preedit 비움).
    preedit: String,
}

impl EditState {
    /// 빈 상태.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 초기 텍스트(캐럿 끝). `select_all`이면 전체 선택으로 시작.
    #[must_use]
    pub fn with_text(text: &str, select_all: bool) -> Self {
        let buf: Vec<char> = text.chars().collect();
        let caret = buf.len();
        let anchor = (select_all && !buf.is_empty()).then_some(0);
        Self {
            buf,
            caret,
            anchor,
            preedit: String::new(),
        }
    }

    /// 현재 텍스트.
    #[must_use]
    pub fn text(&self) -> String {
        self.buf.iter().collect()
    }

    /// 비어 있는가.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// 캐럿 위치(문자 인덱스).
    #[must_use]
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// 선택 범위(정규화 `[a, b)`) — 없으면 `None`.
    #[must_use]
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.caret {
            return None;
        }
        Some((a.min(self.caret), a.max(self.caret)))
    }

    /// 선택 텍스트(복사용).
    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection()?;
        Some(self.buf[a..b].iter().collect())
    }

    /// 조합 중 문자열 지정(M3-1e ① 공용) — **H-25 규칙 내장**: 조합 시작(빈→비움
    /// 아님)에 선택이 있으면 그 선택을 삭제한다(OS 관례 — 선택 위 타이핑 = 대체 ·
    /// "선택 반전 + 조합 밑줄 병존"의 어리둥절한 화면 방지). 버퍼를 바꿨으면(선택
    /// 삭제) `true` — 호스트가 dirty 플래그를 갱신하는 근거.
    pub fn set_preedit(&mut self, text: &str) -> bool {
        let cut = if !text.is_empty() && self.selection().is_some() {
            self.delete_selection()
        } else {
            false
        };
        self.preedit = text.to_string();
        cut
    }

    /// 조합 중 문자열(표시·테스트).
    #[must_use]
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// **표시용 텍스트** — 조합 중 문자열을 캐럿 자리에 끼운 것(편집 버퍼 불변).
    /// 필드에 보이는 그대로가 필요한 곳(아바타 이니셜 미리보기 등)이 쓴다.
    #[must_use]
    pub fn display_text(&self) -> String {
        if self.preedit.is_empty() {
            return self.text();
        }
        let caret = self.caret.min(self.buf.len());
        let before: String = self.buf[..caret].iter().collect();
        let after: String = self.buf[caret..].iter().collect();
        format!("{before}{}{after}", self.preedit)
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.buf.drain(a..b);
        self.caret = a;
        self.anchor = None;
        true
    }

    /// 문자 하나 삽입(선택 있으면 대체).
    pub fn insert(&mut self, c: char) {
        self.delete_selection();
        self.buf.insert(self.caret, c);
        self.caret += 1;
    }

    /// 문자열 삽입(붙여넣기·IME 확정 — 선택 있으면 대체). 제어문자 필터는 호출자 몫.
    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        for c in s.chars() {
            self.buf.insert(self.caret, c);
            self.caret += 1;
        }
    }

    /// Backspace(선택 있으면 선택 삭제).
    pub fn backspace(&mut self) {
        if !self.delete_selection() && self.caret > 0 {
            self.caret -= 1;
            self.buf.remove(self.caret);
        }
    }

    /// 잘라내기(선택 텍스트 반환 후 삭제).
    pub fn cut(&mut self) -> Option<String> {
        let t = self.selected_text()?;
        self.delete_selection();
        Some(t)
    }

    /// 캐럿을 옮긴다 — `extend`면 기존 앵커를 유지해 범위가 늘어난다(드래그·Shift 이동).
    pub fn set_caret(&mut self, idx: usize, extend: bool) {
        let i = idx.min(self.buf.len());
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
        self.caret = i;
    }

    /// 범위를 직접 선택한다(더블클릭 단어 선택 등).
    pub fn set_selection(&mut self, from: usize, to: usize) {
        let n = self.buf.len();
        self.anchor = Some(from.min(n));
        self.caret = to.min(n);
    }

    /// 전체 교체(캐럿 끝·선택 해제).
    pub fn set_text(&mut self, text: &str) {
        self.buf = text.chars().collect();
        self.caret = self.buf.len();
        self.anchor = None;
    }

    /// 키 처리. 비Shift 이동 중 선택이 있으면 선택 가장자리로 접는다(표준 관례).
    pub fn key(&mut self, k: EditKey, shift: bool) {
        match k {
            EditKey::Left => {
                if let (false, Some((a, _))) = (shift, self.selection()) {
                    self.caret = a;
                    self.anchor = None;
                } else {
                    self.move_to(self.caret.saturating_sub(1), shift);
                }
            }
            EditKey::Right => {
                if let (false, Some((_, b))) = (shift, self.selection()) {
                    self.caret = b;
                    self.anchor = None;
                } else {
                    self.move_to((self.caret + 1).min(self.buf.len()), shift);
                }
            }
            EditKey::Home => self.move_to(0, shift),
            EditKey::End => self.move_to(self.buf.len(), shift),
            EditKey::SelectAll => {
                self.anchor = (!self.buf.is_empty()).then_some(0);
                self.caret = self.buf.len();
            }
            EditKey::DeleteForward => {
                if !self.delete_selection() && self.caret < self.buf.len() {
                    self.buf.remove(self.caret);
                }
            }
        }
    }

    fn move_to(&mut self, to: usize, shift: bool) {
        if shift {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
        self.caret = to;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_caret_advance() {
        let mut e = EditState::new();
        e.insert('h');
        e.insert('i');
        assert_eq!(e.text(), "hi");
        assert_eq!(e.caret(), 2);
    }

    #[test]
    fn hangul_is_char_wise_not_byte() {
        // UTF-8 3바이트 한글이 캐럿 1칸씩 — 바이트 인덱싱 함정 회피.
        let mut e = EditState::new();
        e.insert_str("한글");
        assert_eq!(e.caret(), 2);
        e.backspace();
        assert_eq!(e.text(), "한");
        assert_eq!(e.caret(), 1);
    }

    #[test]
    fn caret_move_left_right_home_end() {
        let mut e = EditState::with_text("abc", false);
        assert_eq!(e.caret(), 3);
        e.key(EditKey::Left, false);
        assert_eq!(e.caret(), 2);
        e.key(EditKey::Home, false);
        assert_eq!(e.caret(), 0);
        e.insert('X');
        assert_eq!(e.text(), "Xabc");
        e.key(EditKey::End, false);
        assert_eq!(e.caret(), 4);
    }

    #[test]
    fn shift_selection_then_type_replaces() {
        let mut e = EditState::with_text("hello", false);
        e.key(EditKey::Home, false);
        e.key(EditKey::Right, true); // select 'h'
        e.key(EditKey::Right, true); // select 'he'
        assert_eq!(e.selection(), Some((0, 2)));
        assert_eq!(e.selected_text().as_deref(), Some("he"));
        e.insert('X'); // 선택 대체
        assert_eq!(e.text(), "Xllo");
        assert_eq!(e.caret(), 1);
    }

    #[test]
    fn select_all_and_backspace_clears() {
        let mut e = EditState::with_text("data", false);
        e.key(EditKey::SelectAll, false);
        assert_eq!(e.selection(), Some((0, 4)));
        e.backspace();
        assert!(e.is_empty());
    }

    #[test]
    fn non_shift_left_collapses_selection_to_edge() {
        let mut e = EditState::with_text("abcd", false);
        e.key(EditKey::Home, false);
        e.key(EditKey::Right, true);
        e.key(EditKey::Right, true); // sel [0,2), caret=2
        e.key(EditKey::Left, false); // 비Shift Left = 선택 왼쪽 가장자리로 접기
        assert_eq!(e.caret(), 0);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn delete_forward_at_caret() {
        let mut e = EditState::with_text("abc", false);
        e.key(EditKey::Home, false);
        e.key(EditKey::DeleteForward, false);
        assert_eq!(e.text(), "bc");
        assert_eq!(e.caret(), 0);
    }

    #[test]
    fn insert_str_replaces_selection() {
        let mut e = EditState::with_text("world", true); // 전체 선택
        e.insert_str("hi"); // IME 확정 문자열이 선택 대체
        assert_eq!(e.text(), "hi");
    }
}
