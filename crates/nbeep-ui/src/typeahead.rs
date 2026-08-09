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
    /// 한글 **직접 조합기**(두벌식 · IME 탈피 — [`crate::hangul`]). 자모는 여기서 조합되고
    /// 완성 글자만 `buf`로 넘어간다. 타임아웃/ESC 리셋이 결정적이다.
    composer: crate::hangul::Composer,
    /// IME 조합 중 텍스트(확정 전 · 실시간 매칭용). 확정(`push`)·소거 시 비운다.
    preedit: String,
    /// 타임아웃 초기화 시점의 **묵은 조합 텍스트**. macOS IME 세션(marked text)은 앱이 강제로
    /// 못 버리므로, 이후 Preedit/Commit에서 이 접두사를 벗겨내 "김"+"최"="김최" 유입을 막는다.
    stale: String,
    last_ms: u64,
    timeout_ms: u64,
}

impl TypeAhead {
    /// 타임아웃(ms)으로 생성.
    #[must_use]
    pub fn new(timeout_ms: u64) -> Self {
        TypeAhead {
            buf: String::new(),
            composer: crate::hangul::Composer::new(),
            preedit: String::new(),
            stale: String::new(),
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
        let mut s = format!("{}{}", self.buf, self.preedit);
        if let Some(p) = self.composer.preview() {
            s.push(p);
        }
        s
    }

    /// **IME 조합 중 텍스트 갱신** — 확정 전에도 실시간 매칭한다(한글 "김" 조합 즉시 이동).
    /// 반환 접두사 = 확정 버퍼 + 조합 텍스트. 조합이 비면 확정 버퍼만.
    pub fn set_preedit(&mut self, text: &str, now_ms: u64) -> Query {
        // 조합 시작도 활동으로 간주(타임아웃 리셋).
        if now_ms.saturating_sub(self.last_ms) > self.timeout_ms {
            self.buf.clear();
        }
        self.last_ms = now_ms;
        // 묵은 세션 처리: 이어진 조합("김최")이면 접두사를 벗기고, 새 조합("ㅊ"…)이면 stale 폐기.
        let text = if self.stale.is_empty() {
            text
        } else if let Some(rest) = text.strip_prefix(self.stale.as_str()) {
            rest
        } else {
            self.stale.clear();
            text
        };
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
        if !self.buf.is_empty() || !self.preedit.is_empty() || self.composer.is_composing() {
            self.last_ms = now_ms;
        }
    }

    /// 버퍼 소거(조합 포함).
    pub fn clear(&mut self) {
        self.buf.clear();
        self.preedit.clear();
        self.stale.clear();
        self.composer.reset();
    }

    /// 문자 입력(확정). 타임아웃이 지났으면 새 접두사로 시작.
    /// **반복 키 자동 순환 없음**(사용자 확정 — 순환은 ↑↓ 방향키 전용). 입력은 항상 누적된다.
    pub fn push(&mut self, c: char, now_ms: u64) -> Query {
        self.preedit.clear(); // 확정 문자 도착 = IME 조합 종료(레거시 경로)
                              // 묵은 IME 조합의 확정분은 소비만(레거시 stale — 목록은 이제 직접 조합이라 거의 안 탄다).
        if !self.stale.is_empty() {
            if self.stale.starts_with(c) {
                let n = c.len_utf8();
                self.stale.drain(..n);
                self.last_ms = now_ms;
                return Query {
                    prefix: self.composing(),
                    include_caret: true,
                };
            }
            self.stale.clear();
        }
        let was_empty = self.buf.is_empty() && !self.composer.is_composing();
        let expired = was_empty || now_ms.saturating_sub(self.last_ms) > self.timeout_ms;
        self.last_ms = now_ms;
        if expired {
            self.buf.clear();
            self.composer.reset();
        }
        // 자모는 직접 조합기로(완성 글자만 buf에), 그 외는 조합 확정 후 그대로.
        self.buf.push_str(&self.composer.feed(c));
        Query {
            prefix: self.composing(),
            include_caret: !expired, // 새 접두사 = 캐럿 다음부터
        }
    }

    /// Backspace — 접두사 축소 후 재평가. 비었으면 `None`(버퍼 종료·HUD 소거).
    pub fn backspace(&mut self, now_ms: u64) -> Option<Query> {
        self.preedit.clear();
        self.stale.clear();
        let timed_out = now_ms.saturating_sub(self.last_ms) > self.timeout_ms;
        if timed_out {
            self.buf.clear();
            self.composer.reset();
            return None;
        }
        self.last_ms = now_ms;
        // 조합 중이면 자모 단위, 아니면 완성 글자 단위(사용자 확정).
        if !self.composer.backspace() {
            self.buf.pop();
        }
        let p = self.composing();
        if p.is_empty() {
            None
        } else {
            Some(Query {
                prefix: p,
                include_caret: true,
            })
        }
    }

    /// 주기 점검 — 타임아웃 경과 시 버퍼 소거(**조합 중 텍스트 포함**). 소거했으면 `true`.
    /// buf뿐 아니라 preedit도 봐야 한다 — 한글 조합("김")은 확정 전이라 buf가 비어 있다(HUD가
    /// 안 사라지던 버그).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let active =
            !self.buf.is_empty() || !self.preedit.is_empty() || self.composer.is_composing();
        if active && now_ms.saturating_sub(self.last_ms) > self.timeout_ms {
            self.buf.clear();
            self.composer.reset(); // 직접 조합 = 리셋이 곧 결정적 초기화
                                   // (레거시 IME 경로) 조합 중이던 텍스트는 세션에 남을 수 있어 접두사 제거용으로 기억.
            self.stale = std::mem::take(&mut self.preedit);
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
    fn stale_session_prefix_is_stripped() {
        // "김" 조합 중 타임아웃 → IME 세션의 marked text는 살아 있다.
        let mut t = TypeAhead::new(1000);
        t.set_preedit("김", 0);
        assert!(t.tick(1500), "타임아웃 소거");
        assert_eq!(t.composing(), "", "HUD 비움");
        // 세션이 이어져 "김최"가 와도 접두사를 벗겨 "최"만 반영(사용자 버그 재현 케이스).
        assert_eq!(t.set_preedit("김ㅊ", 1600).prefix, "ㅊ");
        assert_eq!(t.set_preedit("김최", 1700).prefix, "최");
        // 확정 "김최"(문자 2개로 도착) → '김'은 stale 소비, '최'만 버퍼로.
        t.push('김', 1800);
        t.push('최', 1850);
        assert_eq!(t.text(), "최", "stale '김' 소비 · '최'만 확정");
    }

    #[test]
    fn fresh_session_after_timeout_drops_stale() {
        // 타임아웃 후 IME가 실제로 새 세션을 시작하면(ㅊ부터) stale은 즉시 폐기.
        let mut t = TypeAhead::new(1000);
        t.set_preedit("김", 0);
        t.tick(1500);
        assert_eq!(t.set_preedit("ㅊ", 1600).prefix, "ㅊ", "새 조합 = 그대로");
        assert_eq!(t.set_preedit("최", 1700).prefix, "최");
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
    fn repeated_key_accumulates_no_auto_cycle() {
        // 순환은 ↑↓ 전용(사용자 확정) — 같은 키 반복도 그대로 누적된다.
        let mut t = TypeAhead::new(1000);
        t.push('b', 0);
        let q = t.push('b', 300);
        assert_eq!(q.prefix, "bb");
        assert!(q.include_caret, "누적 = 확장 매치");
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
