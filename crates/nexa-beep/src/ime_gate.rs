//! **ImeGate — IME 중재 순수 상태기계**(M3-1e ② · [34 §4-4 G3](../../docs/34-hangul-input-issues.md)).
//!
//! `app.rs`에 흩어져 있던 IME 경합 지식(조합 게이트 · 자모/비자모 보류-판정 · 유출
//! 조합기 · 키다운 잔향 억제 · 이동 키 보류-재생 · 프리에딧 보존)을 **한 타입**으로
//! 모은다. 입력 = OS가 실제로 배달한 이벤트 스트림, 출력 = 버퍼에 가할 명령([`Out`]).
//!
//! **순수 모듈이다** — 창·렌더·OS를 모른다(창 식별자는 제네릭 `W`). 그래서
//! 08-13~14 트레이스로 실측한 이벤트 순서를 **그대로 재생하는 회귀 테스트**가 가능하다
//! (아래 `replay_*` 테스트 — "고쳤다"의 정의가 *그 순서를 재생해도 버퍼가 맞다*가 된다).
//!
//! 호출 순서 계약(호스트 = `app.rs`)은 기존 이벤트 처리 순서를 그대로 따른다:
//! keydown이면 [`ImeGate::keydown_gate`] → (통과 시) [`ImeGate::flush_pending`] →
//! [`ImeGate::leak_intercept`] → (문자 키면) [`ImeGate::route_char`]. IME 이벤트는
//! [`ImeGate::preedit`]/[`ImeGate::commit`], 포커스 이탈은 [`ImeGate::focus_out`],
//! 주기 틱은 [`ImeGate::tick`].

// commit_now(G2 저장 트리거 배선 전)만 대기 — 그때 dead_code 제거.
// unreachable_pub: 바이너리 크레이트 내부 모듈이라 pub이 밖에 안 닿는다(문서 의도용).
#![allow(dead_code, unreachable_pub)]

use nbeep_ui::event::Key;
use nbeep_ui::hangul::{self, Composer};

/// 보류 판정 유예(ms) — 이 시간 안에 Ime 이벤트가 안 오면 진짜 입력으로 방출(H-2·④).
const PENDING_MS: u64 = 150;
/// 확정 직후 같은 문자 키다운 = 이중 배달 잔향으로 보는 창(ms · H-15).
const ECHO_MS: u64 = 120;
/// 프리에딧 소거 → 포커스 이탈 순서 역전을 흡수하는 스태시 유효기간(ms · H-9).
const STASH_MS: u64 = 300;

/// 버퍼에 가할 명령 — 호스트가 순서대로 적용한다(라우팅·표시만, 상태 판단 없음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out<W> {
    /// 확정 문자를 그 창의 입력에 넣는다.
    Char(W, char),
    /// 보류했던 이동 키를 재생한다(shift, primary 순).
    Key(W, Key, bool, bool),
    /// 조합 표시(프리에딧 밑줄) 갱신 — 유출 조합 미리보기 합성 포함. 빈 = 소거.
    Preedit(W, String),
}

/// [`ImeGate::keydown_gate`] 판정 — 이 keydown을 계속 처리할지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatePass {
    /// 계속(일반 경로 — 단축키·문자 라우팅으로).
    Continue,
    /// 소비됨(조합 중 IME 소유·보류 등록·잔향 억제) — 더 처리하지 않는다.
    Swallowed,
}

/// keydown의 게이트 판정에 필요한 요약(호스트가 winit 이벤트에서 추린다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyIn {
    /// 인쇄 가능 문자(자모 포함).
    Char(char),
    /// 이동 키(조합 중 보류-재생 대상 — ←→↑↓·Home/End).
    Arrow(Key),
    /// 그 외(이름 있는 키 — Enter·Esc·Backspace 등. 게이트는 조합 중이면 삼킨다).
    Other,
}

/// IME 중재 상태기계. `W` = 창 식별자(Copy — 테스트는 `u32`).
#[derive(Debug)]
pub struct ImeGate<W: Copy + Eq> {
    /// IME 조합 중(Preedit 비어있지 않음) — keydown 이중 유입 차단 근거(H-1).
    composing: bool,
    /// 마지막 프리에딧(포커스 이탈 보존 — H-9).
    preedit_text: String,
    /// Commit 없는 소거분 스태시(소거→이탈 순서 역전 대비 · H-9).
    stash: Option<(String, u64)>,
    /// 조합 종료 시각 — Windows Esc 잔향 차단(WIME-6)용. 호스트가 조회.
    cleared_ms: Option<u64>,
    /// 보류 문자(창, 문자, 시각) — 자모(H-2 유출/중복 판정) · 비자모(H-11 '?' 판정).
    pending: Option<(W, char, u64, bool)>,
    /// 조합 중 눌린 이동 키(H-16) — Commit 직후 재생.
    pending_arrow: Option<(W, Key, bool, bool)>,
    /// 확정 끝 문자 잔향(H-15 — 같은 키 이중 배달 1회 소비).
    echo: Option<(char, u64)>,
    /// 수동 확정분(창, 본문, 시각 — H-24): focus_out이 확정한 본문을 IME가 늦은
    /// Commit으로 또 보내면 잔향 — 1초 안 같은 창·같은 본문 1회 삼킨다.
    selfcommit: Option<(W, String, u64)>,
    /// 유출 자모 로컬 조합기(H-10·H-14).
    leak: Composer,
    /// 유출 조합이 진행 중인 창.
    leak_win: Option<W>,
    /// keytap 관측 링(문자, 시각 — G1): winit보다 먼저 보는 **순서의 원천**.
    raw: std::collections::VecDeque<(char, u64)>,
    /// 배달 증거 링(문자, 시각 — G1): keydown·commit 어느 경로든 버퍼에 닿은 ASCII.
    /// raw 대조에서 "이미 배달됨"을 가려 **프록시 지연 재주입**을 막는다.
    delivered: std::collections::VecDeque<(char, u64)>,
}

impl<W: Copy + Eq> Default for ImeGate<W> {
    fn default() -> Self {
        Self {
            composing: false,
            preedit_text: String::new(),
            stash: None,
            cleared_ms: None,
            pending: None,
            pending_arrow: None,
            echo: None,
            selfcommit: None,
            leak: Composer::new(),
            leak_win: None,
            raw: std::collections::VecDeque::new(),
            delivered: std::collections::VecDeque::new(),
        }
    }
}

impl<W: Copy + Eq> ImeGate<W> {
    /// 새 게이트.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 조합 중인가(호스트의 표시 판단용).
    #[must_use]
    pub fn composing(&self) -> bool {
        self.composing
    }

    /// 조합 종료 시각(비소비 조회 — G1 보충 주입의 "조합 직후" 판정).
    #[must_use]
    pub fn cleared_at(&self) -> Option<u64> {
        self.cleared_ms
    }

    /// 조합 종료 시각을 소비 조건으로 조회(WIME-6 Esc 잔향 — 1회성 take).
    pub fn take_cleared_if(&mut self, now: u64, within_ms: u64) -> bool {
        if let Some(t) = self.cleared_ms {
            if now.saturating_sub(t) < within_ms {
                self.cleared_ms = None;
                return true;
            }
        }
        false
    }

    // ── G1 · keytap 대조(순서 보존 주입) ────────────────────────────────────

    /// keytap 관측(무수식 ASCII keydown — winit보다 먼저 도착 순서대로).
    pub fn observe_raw(&mut self, c: char, t: u64) {
        self.raw.push_back((c, t));
        if self.raw.len() > 8 {
            self.raw.pop_front();
        }
    }

    /// 배달 증거 기록(내부 — Out::Char로 나가는 ASCII 전부).
    fn note_delivered(&mut self, c: char, t: u64) {
        if c.is_ascii() && !c.is_ascii_control() {
            self.delivered.push_back((c, t));
            if self.delivered.len() > 16 {
                self.delivered.pop_front();
            }
        }
    }

    /// 방출 직전 봉인 — Out::Char 전부를 배달 증거로 기록한 뒤 그대로 돌려준다.
    fn seal(&mut self, outs: Vec<Out<W>>, t: u64) -> Vec<Out<W>> {
        for o in &outs {
            if let Out::Char(_, c) = o {
                self.note_delivered(*c, t);
            }
        }
        outs
    }

    /// 이 문자가 최근(±800ms) 배달됐는가 — 프록시 지연 재주입 방지의 핵심.
    fn was_delivered(&self, c: char, t: u64) -> bool {
        self.delivered
            .iter()
            .any(|&(dc, dt)| dc == c && dt.abs_diff(t) < 800)
    }

    /// ★ keydown 도달 시 raw 대조(G1 개정 2차): 현재 문자를 만나기 전의 잔여 중
    /// **배달 증거가 없고 조합 직후인 것만** = winit이 삼킨 키 → 현재 키보다 먼저
    /// 주입한다(순서 보존). 배달 증거가 있는 잔여 = 프록시 지연 잔재 → 조용히 소진.
    pub fn reconcile_raw(&mut self, id: W, cur: char, now: u64, ime_on: bool) -> Vec<Out<W>> {
        let mut outs = Vec::new();
        if !cur.is_ascii() || self.composing {
            return outs;
        }
        while let Some(&(rc, rt)) = self.raw.front() {
            if rc == cur {
                self.raw.pop_front(); // 정상 도달 — 대조 소진
                break;
            }
            self.raw.pop_front();
            let after_ime = self
                .cleared_ms
                .is_some_and(|ct| rt >= ct && rt.saturating_sub(ct) < 2_000);
            if after_ime && !self.was_delivered(rc, rt) {
                outs.extend(self.route_char(id, rc, now, ime_on));
            }
        }
        outs
    }

    /// 틱 폴백(G1): 뒤따르는 키가 없어 대조가 못 정산한 250ms 경과 잔여를 주입.
    pub fn reconcile_stale(&mut self, id: W, now: u64, ime_on: bool) -> Vec<Out<W>> {
        let mut outs = Vec::new();
        while let Some(&(rc, rt)) = self.raw.front() {
            if now.saturating_sub(rt) < 250 {
                break;
            }
            self.raw.pop_front();
            let after_ime = self
                .cleared_ms
                .is_some_and(|ct| rt >= ct && rt.saturating_sub(ct) < 2_000);
            if after_ime && !self.was_delivered(rc, rt) {
                outs.extend(self.route_char(id, rc, now, ime_on));
            }
        }
        outs
    }

    // ── keydown 경로 ────────────────────────────────────────────────────────

    /// ① 게이트: 조합 중 keydown 소유권 판정 + 확정 직후 잔향 억제.
    /// `Swallowed`면 호스트는 이 keydown을 더 처리하지 않는다.
    pub fn keydown_gate(
        &mut self,
        id: W,
        key: KeyIn,
        now: u64,
        shift: bool,
        primary: bool,
    ) -> GatePass {
        // 잔향 억제(H-15) — 방금 IME가 확정한 그 문자의 keydown 이중 배달(1회 소비).
        if let KeyIn::Char(c) = key {
            if !hangul::is_jamo(c) {
                if let Some((ec, t)) = self.echo {
                    if ec == c && now.saturating_sub(t) < ECHO_MS {
                        self.echo = None;
                        return GatePass::Swallowed;
                    }
                }
            }
        }
        if !self.composing {
            return GatePass::Continue;
        }
        // 조합 중 — 키는 IME 소유(H-1). 단 이동 키(H-16)와 비자모 문자(H-11)는
        // IME가 재방출하지 않을 수 있어 보류한다.
        match key {
            KeyIn::Arrow(k) => {
                self.pending_arrow = Some((id, k, shift, primary));
            }
            KeyIn::Char(c) if !primary && !hangul::is_jamo(c) && !c.is_control() => {
                self.pending = Some((id, c, now, true));
            }
            _ => {}
        }
        GatePass::Swallowed
    }

    /// ② 보류 방출: 새 keydown이 왔는데(또는 틱 유예 경과) Ime 이벤트가 안 붙었다.
    /// `ime_on_window` = 대상 창이 IME 켠 입력(목록 제외) — 자모를 leak 조합기로.
    pub fn flush_pending(&mut self, _now: u64) -> Vec<Out<W>> {
        let Some((id, c, _, ime_on)) = self.pending.take() else {
            return Vec::new();
        };
        if hangul::is_jamo(c) && ime_on {
            let outs = self.leak_feed(id, c);
            return self.seal(outs, _now);
        }
        self.seal(vec![Out::Char(id, c)], _now)
    }

    /// 틱(~5Hz): 보류 유예(150ms) 경과분 방출. 조합 중엔 비자모 보류를 유지한다
    /// (Commit 판정까지 — H-11).
    pub fn tick(&mut self, now: u64) -> Vec<Out<W>> {
        if self.composing {
            return Vec::new();
        }
        if let Some((_, _, t, _)) = self.pending {
            if now.saturating_sub(t) >= PENDING_MS {
                return self.flush_pending(now);
            }
        }
        Vec::new()
    }

    /// ③ 유출 조합 개입(로컬 조합 중일 때만): 백스페이스/Esc는 조합기 몫,
    /// 자모는 보류-판정 경로로, 그 외 키는 조합을 먼저 확정한다.
    /// 반환 = (소비 여부, 명령들). 소비면 호스트는 이 keydown을 더 처리하지 않는다.
    pub fn leak_intercept(&mut self, key: KeyIn, primary: bool) -> (bool, Vec<Out<W>>) {
        if !self.leak.is_composing() {
            return (false, Vec::new());
        }
        if primary {
            return (false, self.flush_leak());
        }
        match key {
            KeyIn::Other => (false, self.flush_leak()), // Enter·Backspace 등 — 확정 후 통과
            KeyIn::Arrow(_) => (false, self.flush_leak()),
            KeyIn::Char(c) if hangul::is_jamo(c) => (false, Vec::new()), // 보류-판정으로
            KeyIn::Char(_) => (false, self.flush_leak()),
        }
    }

    /// 유출 조합 취소(Esc) — 조합 중 글자를 버리고 소비한다(없으면 false).
    pub fn leak_cancel(&mut self) -> (bool, Vec<Out<W>>) {
        if !self.leak.is_composing() {
            return (false, Vec::new());
        }
        self.leak.reset();
        let out = self
            .leak_win
            .take()
            .map(|w| vec![Out::Preedit(w, String::new())])
            .unwrap_or_default();
        (true, out)
    }

    /// 유출 조합 중 Backspace — 자모 단위로 지우고 소비한다(없으면 false).
    pub fn leak_backspace(&mut self) -> (bool, Vec<Out<W>>) {
        if !self.leak.is_composing() {
            return (false, Vec::new());
        }
        self.leak.backspace();
        let p = self.leak.preview().map(String::from).unwrap_or_default();
        let out = self
            .leak_win
            .map(|w| vec![Out::Preedit(w, p)])
            .unwrap_or_default();
        if !self.leak.is_composing() {
            self.leak_win = None;
        }
        (true, out)
    }

    /// ④ 문자 라우팅: 자모 = 보류-판정 등록(IME 켠 창), 그 외 = 즉시 라우팅.
    pub fn route_char(&mut self, id: W, c: char, now: u64, ime_on_window: bool) -> Vec<Out<W>> {
        if hangul::is_jamo(c) && ime_on_window {
            self.pending = Some((id, c, now, true));
            return Vec::new();
            // (자모 보류 — 배달은 flush 시점에 봉인된다.)
        }
        self.seal(vec![Out::Char(id, c)], now)
    }

    // ── IME 이벤트 경로 ─────────────────────────────────────────────────────

    /// Preedit 도착 — 보류 자모 판정(H-14: 프리에딧이 그 자모로 시작할 때만 중복
    /// 폐기, 아니면 유출 → leak 합류) · 조합 추적 · 표시 문자열 합성.
    pub fn preedit(&mut self, id: W, text: &str, now: u64) -> Vec<Out<W>> {
        let mut outs = Vec::new();
        if let Some((pid, c, _, _)) = self.pending {
            if hangul::is_jamo(c) {
                self.pending = None;
                if !text.starts_with(c) {
                    outs.extend(self.leak_feed(pid, c));
                }
            }
        }
        if !text.is_empty() {
            // 조합이 계속된다 = 보류 이동 키는 IME가 내부에서 쓴 것(H-16 폐기 규칙).
            self.pending_arrow = None;
        }
        let was = self.composing;
        self.composing = !text.is_empty();
        if was && !self.composing {
            self.cleared_ms = Some(now);
        }
        if text.is_empty() && !self.preedit_text.is_empty() {
            let prev = std::mem::take(&mut self.preedit_text);
            self.stash = Some((prev, now));
        } else {
            self.preedit_text = text.to_string();
        }
        // 표시 — 같은 창에서 유출 조합 중이면 미리보기를 앞에 붙인다(H-14 과도기).
        let shown = if self.leak_win == Some(id) {
            let mut s = self.leak.preview().map(String::from).unwrap_or_default();
            s.push_str(text);
            s
        } else {
            text.to_string()
        };
        outs.push(Out::Preedit(id, shown));
        outs
    }

    /// Commit 도착 — 낱개 자모 Commit은 leak 조합을 잇고(H-14), 아니면 유출 확정 →
    /// 본문 라우팅 → 비자모 보류 판정(H-11) → 이동 키 재생(H-16) 순서.
    pub fn commit(&mut self, id: W, text: &str, now: u64, ime_on_window: bool) -> Vec<Out<W>> {
        let mut outs = Vec::new();
        // 수동 확정 잔향(H-24) — focus_out이 이미 합류시킨 본문의 늦은 Commit은 1회 삼킨다.
        if let Some((sid, ref stext, st)) = self.selfcommit {
            if sid == id && stext == text && now.saturating_sub(st) < 1_000 {
                self.selfcommit = None;
                self.composing = false;
                self.preedit_text.clear();
                self.stash = None;
                self.cleared_ms = Some(now);
                return outs;
            }
        }
        self.selfcommit = None;
        self.composing = false;
        self.preedit_text.clear();
        self.stash = None;
        self.cleared_ms = Some(now);
        // 낱개 자모 Commit(전환 직후 경합 — H-14): leak에 feed해 음절을 잇는다.
        if ime_on_window && !text.is_empty() && text.chars().all(hangul::is_jamo) {
            self.pending = None; // 같은 키의 키다운 중복 폐기
            if self.leak_win.is_some_and(|w| w != id) {
                outs.extend(self.flush_leak());
            }
            for c in text.chars() {
                outs.extend(self.leak_feed(id, c));
            }
            return self.seal(outs, now);
        }
        // 위젯 조합 표시 소거(H-23 — 포커스 이탈형 확정은 소거 Preedit가 안 온다).
        outs.push(Out::Preedit(id, String::new()));
        // 유출 조합분이 확정 본문보다 앞선 입력이다.
        outs.extend(self.flush_leak());
        // 보류 판정(H-11) — 자모는 무조건 중복 폐기 · 비자모는 본문 대조.
        let leftover = match self.pending.take() {
            Some((pid, c, _, _)) if pid == id && !hangul::is_jamo(c) && !text.contains(c) => {
                Some(c)
            }
            _ => None,
        };
        for c in text.chars().filter(|c| !c.is_control()) {
            outs.push(Out::Char(id, c));
        }
        if let Some(c) = leftover {
            outs.push(Out::Char(id, c));
        }
        // 잔향 기억(H-15) + 이동 키 재생(H-16).
        self.echo = text
            .chars()
            .rev()
            .find(|c| !c.is_control())
            .map(|c| (c, now));
        if let Some((pid, k, shift, primary)) = self.pending_arrow.take() {
            if pid == id {
                outs.push(Out::Key(id, k, shift, primary));
            }
        }
        self.seal(outs, now)
    }

    /// 포커스 이탈 — 유출 조합·조합 중 프리에딧을 확정 합류(H-9 · 스태시 300ms).
    pub fn focus_out(&mut self, id: W, now: u64) -> Vec<Out<W>> {
        let mut outs = self.flush_leak();
        let mut text = std::mem::take(&mut self.preedit_text);
        if text.is_empty() {
            if let Some((s, t)) = self.stash.take() {
                if now.saturating_sub(t) < STASH_MS {
                    text = s;
                }
            }
        }
        if !text.is_empty() {
            self.composing = false;
            self.cleared_ms = Some(now);
            self.selfcommit = Some((id, text.clone(), now)); // H-24 늦은 Commit 잔향 대비
            outs.push(Out::Preedit(id, String::new()));
            for c in text.chars().filter(|c| !c.is_control()) {
                outs.push(Out::Char(id, c));
            }
        }
        self.seal(outs, now)
    }

    /// 저장 트리거 직전 확정(G2 — "저장 트리거 전 조합 확정"): 유출 조합 + 조합 중
    /// 프리에딧을 지금 확정 합류시킨다. 포커스 이탈과 같은 규칙, 스태시는 안 본다.
    /// 수동 확정이므로 **늦은 Commit 잔향 가드(H-24)도 동일하게** 건다.
    pub fn commit_now(&mut self, id: W, now: u64) -> Vec<Out<W>> {
        let mut outs = self.flush_leak();
        let text = std::mem::take(&mut self.preedit_text);
        if !text.is_empty() {
            self.composing = false;
            self.cleared_ms = Some(now);
            self.selfcommit = Some((id, text.clone(), now));
            outs.push(Out::Preedit(id, String::new()));
            for c in text.chars().filter(|c| !c.is_control()) {
                outs.push(Out::Char(id, c));
            }
        }
        self.seal(outs, now)
    }

    // ── 내부 ────────────────────────────────────────────────────────────────

    /// 유출 자모를 로컬 조합기에 합류(완성 글자는 즉시 라우팅 · 미리보기 갱신).
    fn leak_feed(&mut self, id: W, c: char) -> Vec<Out<W>> {
        let mut outs = Vec::new();
        if self.leak_win.is_some_and(|w| w != id) {
            outs.extend(self.flush_leak());
        }
        for oc in self.leak.feed(c).chars() {
            outs.push(Out::Char(id, oc));
        }
        self.leak_win = Some(id);
        let p = self.leak.preview().map(String::from).unwrap_or_default();
        outs.push(Out::Preedit(id, p));
        outs
    }

    /// 유출 조합 확정 — 조합 중 글자를 입력에 합류시키고 미리보기를 지운다.
    fn flush_leak(&mut self) -> Vec<Out<W>> {
        let Some(id) = self.leak_win.take() else {
            return Vec::new();
        };
        let out = self.leak.flush();
        let mut outs = vec![Out::Preedit(id, String::new())];
        if let Some(c) = out {
            outs.push(Out::Char(id, c));
        }
        outs
    }
}

// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 1;

    /// 호스트 호출 순서 계약대로 이벤트를 재생하고 버퍼·재생 키를 조립하는 드라이버.
    struct Driver {
        gate: ImeGate<u32>,
        now: u64,
        buf: String,
        keys: Vec<Key>,
    }

    impl Driver {
        fn new() -> Self {
            Self {
                gate: ImeGate::new(),
                now: 0,
                buf: String::new(),
                keys: Vec::new(),
            }
        }
        fn apply(&mut self, outs: Vec<Out<u32>>) {
            for o in outs {
                match o {
                    Out::Char(_, c) => {
                        if c == '\u{8}' {
                            self.buf.pop();
                        } else {
                            self.buf.push(c);
                        }
                    }
                    Out::Key(_, k, _, _) => self.keys.push(k),
                    Out::Preedit(..) => {}
                }
            }
        }
        /// keydown(문자) — app.rs 순서: gate → flush_pending → leak_intercept → route_char.
        /// keytap 관측(프록시 — 실제로는 winit보다 먼저 도착).
        fn raw(&mut self, c: char) {
            self.gate.observe_raw(c, self.now + 1);
        }
        /// keydown(문자) — app.rs 순서: reconcile → gate → flush → leak → route.
        fn key_char(&mut self, c: char) {
            self.now += 30;
            let outs = self.gate.reconcile_raw(W, c, self.now, true);
            self.apply(outs);
            if self
                .gate
                .keydown_gate(W, KeyIn::Char(c), self.now, false, false)
                == GatePass::Swallowed
            {
                return;
            }
            let outs = self.gate.flush_pending(self.now);
            self.apply(outs);
            let (consumed, outs) = self.gate.leak_intercept(KeyIn::Char(c), false);
            self.apply(outs);
            if consumed {
                return;
            }
            let outs = self.gate.route_char(W, c, self.now, true);
            self.apply(outs);
        }
        /// keydown(이동 키).
        fn key_arrow(&mut self, k: Key) {
            self.now += 30;
            if self
                .gate
                .keydown_gate(W, KeyIn::Arrow(k), self.now, false, false)
                == GatePass::Swallowed
            {
                return;
            }
            let outs = self.gate.flush_pending(self.now);
            self.apply(outs);
            let (_, outs) = self.gate.leak_intercept(KeyIn::Arrow(k), false);
            self.apply(outs);
            self.keys.push(k);
        }
        /// keydown(Enter 등 기타) — 소비 안 되면 키 목록에 기록.
        fn key_other(&mut self) {
            self.now += 30;
            if self
                .gate
                .keydown_gate(W, KeyIn::Other, self.now, false, false)
                == GatePass::Swallowed
            {
                return;
            }
            let outs = self.gate.flush_pending(self.now);
            self.apply(outs);
            let (_, outs) = self.gate.leak_intercept(KeyIn::Other, false);
            self.apply(outs);
            self.keys.push(Key::Enter);
        }
        fn preedit(&mut self, t: &str) {
            self.now += 10;
            let outs = self.gate.preedit(W, t, self.now);
            self.apply(outs);
        }
        fn commit(&mut self, t: &str) {
            self.now += 10;
            let outs = self.gate.commit(W, t, self.now, true);
            self.apply(outs);
        }
    }

    /// ★ G1 재생 — 소비된 첫 1byte가 **다음 키보다 먼저** 주입된다(순서 보존).
    /// 프록시 지연(raw가 keydown보다 늦게 도착) 순서 그대로 재생.
    #[test]
    fn replay_g1_consumed_injected_in_order() {
        let mut d = Driver::new();
        d.preedit("나");
        d.preedit("");
        d.commit("나");
        d.raw('!'); // 소비됨 — keydown 없음
        d.key_char('@'); // '@' keydown이 자기 raw보다 먼저(프록시 지연)
        d.raw('@');
        d.key_char('#');
        d.raw('#');
        assert_eq!(d.buf, "나!@#", "유실 복구 + 정순");
    }

    /// ★ G1 회귀 — "가나다123 → 가나다1223" 이중 입력(프록시 지연 재주입) 방지.
    #[test]
    fn replay_g1_no_double_from_proxy_lag() {
        let mut d = Driver::new();
        d.preedit("다");
        d.preedit("");
        d.commit("다");
        d.raw('1'); // 소비
        d.key_char('2');
        d.raw('2'); // 지연 도착
        d.key_char('3');
        d.raw('3');
        let outs = d.gate.reconcile_stale(W, d.now + 400, true);
        d.apply(outs);
        assert_eq!(d.buf, "다123", "재주입 없이 정확히 한 번씩");
    }

    /// G1 — 평시(조합 없음)엔 어떤 주입도 없다.
    #[test]
    fn replay_g1_idle_never_injects() {
        let mut d = Driver::new();
        d.raw('x');
        d.key_char('x');
        d.raw('y');
        d.key_char('y');
        let outs = d.gate.reconcile_stale(W, d.now + 400, true);
        d.apply(outs);
        assert_eq!(d.buf, "xy");
    }

    /// ★ S1 재생(08-14 2차 수집 트레이스 원문 순서) — 전환 직후 첫 키 유출:
    /// key="ㄴ" → preedit="ㅏ"(둘째 키부터) → 낱개 commit="ㅏ" → 둘째 음절 정상.
    /// 기대 버퍼 = "ab나다" (H-2·H-10·H-14 복원 실증의 박제).
    #[test]
    fn replay_s1_first_key_leak_recovers_syllable() {
        let mut d = Driver::new();
        d.key_char('a');
        d.key_char('b');
        d.key_char('ㄴ'); // 유출 keydown(보류로)
        d.preedit("ㅏ"); // IME는 둘째 키부터 — 보류 ㄴ은 leak 합류
        d.preedit("ㅏ");
        d.preedit("");
        d.commit("ㅏ"); // 낱개 자모 확정 → leak: ㄴ+ㅏ = "나"(조합 중)
        d.preedit("");
        d.preedit("ㄷ");
        d.preedit("다");
        d.preedit("다");
        d.preedit("");
        d.commit("다"); // 일반 확정 — leak "나" 선행 합류 후 "다"
        assert_eq!(d.buf, "ab나다");
    }

    /// ★ E3 재생 — 조합("홍") 후 1byte: OS가 배달해 준 것은 전부 버퍼에 닿아야 한다
    /// (세션당 1회 소비는 게이트 밖(winit 경계) — 게이트는 도달분을 잃지 않는다).
    #[test]
    fn replay_e3_digit_after_composition_is_kept() {
        let mut d = Driver::new();
        d.key_char('ㅎ');
        d.preedit("ㅎ");
        d.preedit("호");
        d.preedit("홍");
        d.preedit("");
        d.commit("홍");
        d.preedit("");
        d.key_char('1'); // E3에서 실제 도달한 1개
        assert_eq!(d.buf, "홍1");
    }

    /// ★ S3 재생 — "다" 조합 중 Enter: commit이 Enter보다 먼저 도달(실측 순서).
    /// 기대 = 버퍼 "나다" 완성 후 Enter 통과(1-Enter 확정+전송).
    #[test]
    fn replay_s3_commit_precedes_enter() {
        let mut d = Driver::new();
        d.preedit("ㄴ");
        d.preedit("나");
        d.preedit("");
        d.commit("나");
        d.preedit("");
        d.preedit("다");
        d.preedit("");
        d.commit("다");
        d.preedit("");
        d.key_other(); // Enter — 조합 아님 → 통과
        assert_eq!(d.buf, "나다");
        assert_eq!(d.keys, vec![Key::Enter], "Enter가 확정 뒤 도달·통과");
    }

    /// H-11 재생 — 조합 중 '?': keydown 보류 → commit 본문에 없으면 뒤에 방출.
    #[test]
    fn replay_h11_pending_punct_released_after_commit() {
        let mut d = Driver::new();
        d.preedit("껀");
        d.key_char('?'); // 조합 중 — 보류(Swallowed)
        d.preedit("");
        d.commit("껀"); // 본문에 '?' 없음 → 방출
        assert_eq!(d.buf, "껀?");
    }

    /// H-15 재생 — commit=" " 직후 같은 스페이스 keydown = 잔향 1회 소비(연타는 통과).
    #[test]
    fn replay_h15_space_echo_swallowed_once() {
        let mut d = Driver::new();
        d.preedit(" ");
        d.preedit("");
        d.commit(" ");
        d.key_char(' '); // 잔향 — 소비
        d.key_char(' '); // 진짜 연타 — 통과
        assert_eq!(d.buf, "  ", "확정 1 + 연타 1 = 2칸(잔향만 소거)");
    }

    /// H-16 재생 — 조합 중 ← 보류 → commit 직후 재생.
    #[test]
    fn replay_h16_arrow_pending_replayed_after_commit() {
        let mut d = Driver::new();
        d.preedit("지");
        d.key_arrow(Key::Left); // 조합 중 — 보류(Swallowed)
        assert!(d.keys.is_empty(), "조합 중엔 이동하지 않는다");
        d.preedit("");
        d.commit("지");
        assert_eq!(d.buf, "지");
        assert_eq!(d.keys, vec![Key::Left], "확정 직후 재생");
    }

    /// H-9 재생 — 조합 중 포커스 이탈(소거 Preedit 선행 순서 포함): 확정 합류.
    #[test]
    fn replay_h9_focus_out_commits_preedit_via_stash() {
        let mut d = Driver::new();
        d.preedit("지");
        d.preedit(""); // OS가 소거를 먼저 보냄(순서 역전) — 스태시로
        d.now += 50;
        let outs = d.gate.focus_out(W, d.now);
        d.apply(outs);
        assert_eq!(d.buf, "지", "스태시 300ms 안 = 확정 합류");
    }

    /// ★ H-24 재생("나다다") — 포커스 이탈 수동 확정 뒤 늦은 Commit(같은 본문)은
    /// 1회 잔향으로 삼킨다(다른 본문·1초 초과는 정상 입력).
    #[test]
    fn replay_h24_late_commit_after_manual_flush_is_swallowed_once() {
        let mut d = Driver::new();
        d.preedit("나");
        d.preedit("");
        d.commit("나");
        d.preedit("다"); // "다" 조합 중 포커스 이탈(작업표시줄 클릭류)
        d.now += 50;
        let outs = d.gate.focus_out(W, d.now);
        d.apply(outs);
        assert_eq!(d.buf, "나다", "수동 확정 합류");
        d.commit("다"); // IME의 늦은 Commit — 잔향
        assert_eq!(d.buf, "나다", "이중 입력('나다다') 방지");
        d.preedit("다");
        d.preedit("");
        d.commit("다"); // 그 다음 진짜 입력은 정상
        assert_eq!(d.buf, "나다다");
    }

    /// G2 — commit_now 수동 확정 뒤 **늦은 Commit 잔향**도 H-24처럼 1회 삼킨다.
    #[test]
    fn commit_now_swallows_late_commit_echo() {
        let mut d = Driver::new();
        d.preedit("다"); // 조합 중 저장 트리거(클릭 등)
        let outs = d.gate.commit_now(W, d.now + 10);
        d.apply(outs);
        assert_eq!(d.buf, "다", "수동 확정 합류");
        d.commit("다"); // IME의 늦은 Commit — 잔향
        assert_eq!(d.buf, "다", "이중 입력 방지(H-24 문법 공유)");
    }

    /// G2 — 저장 트리거 직전 확정(commit_now): 조합 중 음절이 값에 포함된다.
    #[test]
    fn commit_now_flushes_composing_syllable() {
        let mut d = Driver::new();
        d.preedit("나");
        d.preedit("");
        d.commit("나");
        d.preedit("다"); // "다" 조합 중 — 저장 트리거 발생 가정
        let outs = d.gate.commit_now(W, d.now + 10);
        d.apply(outs);
        assert_eq!(d.buf, "나다", "저장 트리거 전 조합 확정(G2)");
    }

    /// 틱 유예 — Ime 이벤트가 끝내 안 오면 150ms 후 유출 자모가 leak으로 방출.
    #[test]
    fn tick_flushes_stale_pending_into_leak() {
        let mut d = Driver::new();
        d.key_char('ㅁ'); // 보류
        assert_eq!(d.buf, "");
        let outs = d.gate.tick(d.now + PENDING_MS);
        d.apply(outs);
        // 자모 하나는 조합 중(미리보기)이라 버퍼는 아직 비어 있고, 비자모가 오면 확정.
        d.key_char('.');
        assert_eq!(d.buf, "ㅁ.", "leak 확정 후 문자");
    }
}
