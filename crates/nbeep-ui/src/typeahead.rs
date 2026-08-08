//! 타입어헤드 버퍼 — 피어 목록 키보드 탐색(FR-U-4)의 접두사 입력.
//!
//! `nexa-dir2/crates/nexa-gui/src/typeahead.rs` 이식([docs/12 §A]) — 시각 주입으로 순수
//! 로직·전 플랫폼 테스트. 누적/타임아웃 리셋/반복 단일키 cycle/Backspace. **매칭 자체는
//! 소비 위젯이 한다**(목록마다 매칭 대상이 다르다 — 피어 목록은 표시 이름).

/// 기본 타임아웃(ms) — 사용자 확정 2000. 설정에서 변경 가능(`ui.typeahead_timeout`).
pub const TYPEAHEAD_TIMEOUT_MS: u64 = 2000;

/// 입력 결과 — 검색 접두사와 시작점 규칙.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Query {
    /// 검색 접두사.
    pub prefix: String,
    /// `true` = 접두사 확장(현재 캐럿 행 포함해 재평가), `false` = 새 입력/반복(다음 매치부터).
    pub include_caret: bool,
}

#[derive(Debug)]
pub struct TypeAhead {
    buf: String,
    /// IME 조합 중 텍스트(확정 전 · 실시간 매칭용). 확정(`push`)·소거 시 비운다.
    preedit: String,
    last_ms: u64,
    timeout_ms: u64,
}

impl TypeAhead {
    /// 타임아웃(ms)으로 생성.
    #[must_use]
    pub fn new(timeout_ms: u64) -> Self {
        TypeAhead {
            buf: String::new(),
            preedit: String::new(),
            last_ms: 0,
            timeout_ms,
        }
    }

    /// 현재 확정 버퍼(테스트·내부용).
    #[must_use]
    pub fn text(&self) -> &str {
        &self.buf
    }

    /// 확정 버퍼 + 조합 중 텍스트(HUD 표시·매칭 접두사). 빈 값 = 비활성.
    #[must_use]
    pub fn composing(&self) -> String {
        format!("{}{}", self.buf, self.preedit)
    }

    /// **IME 조합 중 텍스트 갱신** — 확정 전에도 실시간 매칭한다(한글 "김" 조합 즉시 이동).
    /// 반환 접두사 = 확정 버퍼 + 조합 텍스트. 조합이 비면 확정 버퍼만.
    pub fn set_preedit(&mut self, text: &str, now_ms: u64) -> Query {
        // 조합 시작도 활동으로 간주(타임아웃 리셋).
        if now_ms.saturating_sub(self.last_ms) > self.timeout_ms {
            self.buf.clear();
        }
        self.last_ms = now_ms;
        self.preedit = text.to_string();
        Query {
            prefix: self.composing(),
            include_caret: true, // 조합이 자라며 현재 매치 유지·재평가
        }
    }

    /// 입력 리셋 타임아웃 변경(설정 — 원본 "Type-ahead input reset (ms)").
    pub fn set_timeout(&mut self, ms: u64) {
        self.timeout_ms = ms.max(1);
    }

    /// 활동 갱신 — 버퍼가 살아 있으면 타임아웃 기준 시각을 지금으로 리셋(↑↓ 순환 중 유지).
    pub fn touch(&mut self, now_ms: u64) {
        if !self.buf.is_empty() || !self.preedit.is_empty() {
            self.last_ms = now_ms;
        }
    }

    /// 버퍼 소거(조합 포함).
    pub fn clear(&mut self) {
        self.buf.clear();
        self.preedit.clear();
    }

    /// 문자 입력(확정). 타임아웃이 지났으면 새 접두사로 시작.
    /// 반복 단일키(`r`,`r`,…)는 누적하지 않고 같은 접두사의 **다음 매치로 cycle**(탐색기 규약).
    pub fn push(&mut self, c: char, now_ms: u64) -> Query {
        self.preedit.clear(); // 확정 문자 도착 = 조합 종료
        let expired = self.buf.is_empty() || now_ms.saturating_sub(self.last_ms) > self.timeout_ms;
        self.last_ms = now_ms;
        if expired {
            self.buf.clear();
            self.buf.push(c);
            return Query {
                prefix: self.buf.clone(),
                include_caret: false, // 새 접두사 = 캐럿 다음부터
            };
        }
        let single_repeat = self.buf.chars().count() == 1 && self.buf.starts_with(c);
        if single_repeat {
            Query {
                prefix: self.buf.clone(),
                include_caret: false, // cycle = 다음 매치
            }
        } else {
            self.buf.push(c);
            Query {
                prefix: self.buf.clone(),
                include_caret: true, // 확장 = 현재 행이 여전히 매치면 유지
            }
        }
    }

    /// Backspace — 접두사 축소 후 재평가. 비었으면 `None`(버퍼 종료·HUD 소거).
    pub fn backspace(&mut self, now_ms: u64) -> Option<Query> {
        self.preedit.clear();
        if self.buf.is_empty() || now_ms.saturating_sub(self.last_ms) > self.timeout_ms {
            self.buf.clear();
            return None;
        }
        self.last_ms = now_ms;
        self.buf.pop();
        if self.buf.is_empty() {
            None
        } else {
            Some(Query {
                prefix: self.buf.clone(),
                include_caret: true,
            })
        }
    }

    /// 주기 점검 — 타임아웃 경과 시 버퍼 소거(**조합 중 텍스트 포함**). 소거했으면 `true`.
    /// buf뿐 아니라 preedit도 봐야 한다 — 한글 조합("김")은 확정 전이라 buf가 비어 있다(HUD가
    /// 안 사라지던 버그).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let active = !self.buf.is_empty() || !self.preedit.is_empty();
        if active && now_ms.saturating_sub(self.last_ms) > self.timeout_ms {
            self.buf.clear();
            self.preedit.clear();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preedit_matches_live_and_commit_carries_over() {
        let mut t = TypeAhead::new(1000);
        // 한글 조합: ㄱ → 기 → 김 (확정 전에도 접두사가 실시간으로 바뀐다).
        assert_eq!(t.set_preedit("ㄱ", 0).prefix, "ㄱ");
        assert_eq!(t.set_preedit("기", 50).prefix, "기");
        assert_eq!(t.set_preedit("김", 100).prefix, "김");
        assert_eq!(t.composing(), "김");
        // 확정(Space 등) → Char 도착 시 조합이 버퍼로 넘어가고 preedit 소거.
        let q = t.push('김', 150);
        assert_eq!(q.prefix, "김");
        assert_eq!(t.composing(), "김");
        assert_eq!(t.text(), "김", "확정 버퍼로");
    }

    #[test]
    fn accumulates_within_timeout_and_resets_after() {
        let mut t = TypeAhead::new(1000);
        assert_eq!(
            t.push('r', 0),
            Query {
                prefix: "r".into(),
                include_caret: false
            }
        );
        assert_eq!(
            t.push('e', 500),
            Query {
                prefix: "re".into(),
                include_caret: true
            }
        );
        // 1000ms 초과 → 새 접두사
        assert_eq!(
            t.push('x', 1600),
            Query {
                prefix: "x".into(),
                include_caret: false
            }
        );
    }

    #[test]
    fn single_key_repeat_cycles_instead_of_accumulating() {
        let mut t = TypeAhead::new(1000);
        t.push('r', 0);
        let q = t.push('r', 300);
        assert_eq!(q.prefix, "r");
        assert!(!q.include_caret, "반복 = 다음 매치로 cycle");
        // 다른 글자가 오면 누적으로 복귀
        assert_eq!(t.push('e', 600).prefix, "re");
    }

    #[test]
    fn backspace_shrinks_then_ends() {
        let mut t = TypeAhead::new(1000);
        t.push('a', 0);
        t.push('b', 100);
        assert_eq!(t.backspace(200).unwrap().prefix, "a");
        assert_eq!(t.backspace(300), None);
        assert_eq!(t.text(), "");
        assert_eq!(t.backspace(400), None); // 빈 버퍼 무시
    }

    #[test]
    fn tick_clears_only_after_timeout() {
        let mut t = TypeAhead::new(1000);
        t.push('a', 0);
        assert!(!t.tick(900));
        assert_eq!(t.text(), "a");
        assert!(t.tick(1100));
        assert_eq!(t.text(), "");
        assert!(!t.tick(2000)); // 이미 비어 있음
    }
}
