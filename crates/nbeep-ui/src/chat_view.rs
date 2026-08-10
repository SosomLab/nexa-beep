//! 대화 화면 위젯 — 스레드 + 한 줄 입력(M3 첫 슬라이스 · M2 게이트 "고르면 대화가 된다"의 UI 절반).
//!
//! 메시지는 [`nbeep_core::safetext`]를 **통과한 것만** 담긴다(타입이 `SafeText` — 무해화 우회
//! 불가). 입력은 [`crate::edit::EditState`](캐럿·선택·char 단위) + **IME 프리에딧**(조합 중 밑줄
//! 표시 — M3-3). 확정 문자는 `Char`로, 조합 중 텍스트는 [`ChatViewWidget::set_preedit`]로 온다.
//!
//! 발신·복귀는 폴링(`take_outgoing`/`take_back`) — 위젯은 부모를 모른다.

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::safetext::{sanitize_message, SafeText};

/// 스레드 한 줄.
#[derive(Clone, Debug)]
pub struct ChatLine {
    /// 내가 보낸 것인가(정렬·색 구분).
    pub mine: bool,
    /// 본문 — 텍스트 또는 파일 전송 기록.
    pub body: ChatBody,
}

impl ChatLine {
    /// 텍스트 줄(이미 무해화된 타입만).
    #[must_use]
    pub fn text(mine: bool, text: SafeText) -> Self {
        Self {
            mine,
            body: ChatBody::Text(text),
        }
    }

    /// 파일 전송 항목 — `승인 대기` 상태로 시작한다.
    #[must_use]
    pub fn xfer(mine: bool, name: SafeText, size: u64) -> Self {
        Self {
            mine,
            body: ChatBody::Xfer(XferLine {
                name,
                size,
                state: XferLineState::Waiting,
            }),
        }
    }
}

/// 스레드 줄의 본문 종류.
#[derive(Clone, Debug)]
pub enum ChatBody {
    /// 무해화된 텍스트.
    Text(SafeText),
    /// 파일 전송 기록 — 진행 중엔 상태가 갱신되고, **완료 후에도 스레드에 남는다**
    /// (송신·수신 이력을 대화에서 본다 — 사용자 요청 08-10).
    Xfer(XferLine),
}

/// 파일 전송 스레드 항목.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XferLine {
    /// 무해화된 파일명(원격 제공 이름 — 표시 전 무해화 필수).
    pub name: SafeText,
    /// 전체 크기(바이트).
    pub size: u64,
    /// 현재 상태.
    pub state: XferLineState,
}

/// 파일 전송 항목의 상태 — `Waiting`/`Active`만 갱신 대상(종결 상태는 불변).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XferLineState {
    /// 승인 대기(발신 = 상대 승인 · 수신 = 내 승인).
    Waiting,
    /// 전송 중(누적 바이트).
    Active {
        /// 지금까지 오간 바이트.
        done: u64,
    },
    /// 완료(부가 설명 — 예: 격리·위험 등급).
    Done {
        /// 상태 뒤에 붙는 설명(비면 "완료"만).
        note: String,
    },
    /// 실패·거절·취소(사유).
    Failed {
        /// 사람이 읽는 사유.
        why: String,
    },
}

/// `lines`에서 **가장 오래된 미종결 전송 항목**(방향 일치)의 상태를 갱신한다.
///
/// 앞에서부터 찾는 이유 — 오퍼 큐가 FIFO라 결정·진행도 오래된 것부터 처리된다
/// (뒤에서 찾으면 대기 2건일 때 앞 건의 거절이 뒤 항목에 붙는다).
/// 종결(`Done`/`Failed`) 항목은 건드리지 않는다 — 완료 기록은 불변이고,
/// 같은 상대와의 새 전송은 새 항목으로 쌓인다. 갱신했으면 `true`.
pub fn update_xfer_in(lines: &mut [ChatLine], mine: bool, state: XferLineState) -> bool {
    for line in lines.iter_mut() {
        if line.mine != mine {
            continue;
        }
        if let ChatBody::Xfer(x) = &mut line.body {
            if matches!(
                x.state,
                XferLineState::Waiting | XferLineState::Active { .. }
            ) {
                x.state = state;
                return true;
            }
        }
    }
    false
}

/// 대화 화면 위젯.
#[derive(Debug)]
pub struct ChatViewWidget {
    bounds: Rect,
    /// 상대 표시 이름(헤더).
    title: String,
    lines: Vec<ChatLine>,
    input: crate::edit::EditState,
    /// IME 조합 중 텍스트(확정 전 — 밑줄 표시. 확정은 input에 삽입).
    preedit: String,
    scale: f32,
    outgoing: Option<SafeText>,
    back: bool,
    /// 스크롤 오프셋 — **하단(최신)에서 위로 밀어올린 줄 수**(0 = 최신 붙어 봄).
    scroll: usize,
    wheel: crate::event::WheelAccum,
    /// 진행 중 파일 전송(헤더 아래 진척 줄 · 사용자 요청 08-09).
    xfer: Option<crate::peer_list::XferProgress>,
}

/// 사람이 읽는 크기 표기.
fn human_bytes(b: u64) -> String {
    const K: u64 = 1024;
    match b {
        v if v >= K * K => format!("{:.1}MiB", v as f64 / (K * K) as f64),
        v if v >= K => format!("{:.1}KiB", v as f64 / K as f64),
        v => format!("{v}B"),
    }
}

impl ChatViewWidget {
    /// 진행 중 전송 상태 지정(`None` = 진행 없음 — 줄이 사라진다).
    pub fn set_xfer(
        &mut self,
        xfer: Option<crate::peer_list::XferProgress>,
        inv: &mut Invalidations,
    ) {
        if self.xfer != xfer {
            self.xfer = xfer;
            inv.push(self.bounds);
        }
    }

    /// 상대 이름으로 빈 대화 화면을 연다.
    #[must_use]
    pub fn new(title: String) -> Self {
        Self {
            bounds: Rect::default(),
            title,
            lines: Vec::new(),
            input: crate::edit::EditState::new(),
            preedit: String::new(),
            scale: 1.0,
            outgoing: None,
            back: false,
            scroll: 0,
            wheel: crate::event::WheelAccum::default(),
            xfer: None,
        }
    }

    /// 배율 지정(고DPI).
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        let scale = scale.max(0.5);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            inv.push(self.bounds);
        }
    }

    /// 스레드에 줄 추가(수신·발신 확정분 — 이미 무해화된 타입만 받는다).
    pub fn push_line(&mut self, line: ChatLine, inv: &mut Invalidations) {
        self.lines.push(line);
        self.scroll = 0; // 새 메시지 = 최신으로 스냅(표준 채팅 동작)
        inv.push(self.bounds);
    }

    /// 마지막 미종결 전송 항목(방향 일치)의 상태를 갱신한다([`update_xfer_in`]).
    pub fn update_xfer_line(
        &mut self,
        mine: bool,
        state: XferLineState,
        inv: &mut Invalidations,
    ) -> bool {
        let hit = update_xfer_in(&mut self.lines, mine, state);
        if hit {
            inv.push(self.bounds);
        }
        hit
    }

    /// 화면에 들어가는 스레드 줄 수(헤더·입력창 제외).
    fn visible_lines(&self) -> usize {
        let head_h = self.s(34);
        let input_h = self.s(40);
        let area = (self.bounds.h - head_h - input_h - self.s(6)).max(0);
        (area / self.s(28).max(1)) as usize
    }

    /// 스크롤 상한(위로 최대 얼마나) — 안 넘는 범위로 조인다.
    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.visible_lines())
    }

    /// 현재 스크롤 오프셋(테스트·표시).
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Enter로 확정된 발신 본문(1회성) — 조립 지점이 시퀀서·팬아웃에 넘긴다.
    pub fn take_outgoing(&mut self) -> Option<SafeText> {
        self.outgoing.take()
    }

    /// Esc 복귀 요청(1회성).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    /// 현재 입력 중 텍스트(테스트·HUD용).
    #[must_use]
    pub fn input(&self) -> String {
        self.input.text()
    }

    /// 입력 캐럿 위치(문자 인덱스) — 커서 렌더용.
    #[must_use]
    pub fn input_caret(&self) -> usize {
        self.input.caret()
    }

    /// IME 조합 중 텍스트 설정(빈 문자열 = 조합 종료·취소). 확정은 `on_event`의 Char로.
    pub fn set_preedit(&mut self, text: String, inv: &mut Invalidations) {
        self.preedit = text;
        inv.push(self.input_bar());
    }

    /// 조합 중 텍스트(테스트·렌더).
    #[must_use]
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    fn s(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    fn input_bar(&self) -> Rect {
        let h = self.s(40);
        Rect::new(self.bounds.x, self.bounds.bottom() - h, self.bounds.w, h)
    }
}

impl Widget for ChatViewWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        use crate::edit::EditKey;
        match *ev {
            InputEvent::Key {
                key: Key::Enter, ..
            } => {
                let text = self.input.text();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    // 발신 확정 — 무해화를 여기서 통과시킨다(스레드 타입이 SafeText).
                    self.outgoing = Some(sanitize_message(trimmed));
                    self.input.set_text("");
                    inv.push(self.bounds);
                }
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.back = true;
            }
            InputEvent::Key {
                key: Key::PageUp, ..
            } => {
                let page = self.visible_lines().max(1);
                self.scroll = (self.scroll + page).min(self.max_scroll());
                inv.push(self.bounds);
            }
            InputEvent::Key {
                key: Key::PageDown, ..
            } => {
                let page = self.visible_lines().max(1);
                self.scroll = self.scroll.saturating_sub(page);
                inv.push(self.bounds);
            }
            InputEvent::Key { key, shift, .. } => {
                // 캐럿 이동·선택(edit 모델). Enter/Esc는 위에서 처리됨.
                let ek = match key {
                    Key::Left => Some(EditKey::Left),
                    Key::Right => Some(EditKey::Right),
                    Key::Home => Some(EditKey::Home),
                    Key::End => Some(EditKey::End),
                    _ => None,
                };
                if let Some(ek) = ek {
                    self.input.key(ek, shift);
                    inv.push(self.input_bar());
                }
            }
            InputEvent::Wheel { delta } => {
                let lines = self.wheel.add(delta, 3);
                if lines != 0 {
                    // 위로(양수) = 과거로(scroll 증가).
                    let ns = if lines > 0 {
                        self.scroll + lines.unsigned_abs() as usize
                    } else {
                        self.scroll.saturating_sub(lines.unsigned_abs() as usize)
                    };
                    let clamped = ns.min(self.max_scroll());
                    if clamped != self.scroll {
                        self.scroll = clamped;
                        inv.push(self.bounds);
                    }
                }
            }
            InputEvent::SelectAll => {
                self.input.key(EditKey::SelectAll, false);
                inv.push(self.input_bar());
            }
            InputEvent::Char { c, .. } => {
                self.preedit.clear(); // 확정 문자 도착 = 조합 종료
                if c == '\u{8}' {
                    self.input.backspace();
                } else if !c.is_control() {
                    self.input.insert(c);
                }
                inv.push(self.input_bar());
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        ctx.fill_rect(self.bounds, theme.panel_bg);
        // 헤더.
        let head_h = self.s(34);
        let head = Rect::new(self.bounds.x, self.bounds.y, self.bounds.w, head_h);
        ctx.select_font(FontSlot::Base, false);
        ctx.text_opaque(
            head.x + self.s(12),
            head.y + self.s(7),
            head,
            &self.title,
            theme.text,
            theme.chrome_bg,
        );

        // 전송 진척 줄 — 헤더 바로 아래(대화 내용을 가리지 않는 자리).
        if let Some(xp) = self.xfer {
            let row = Rect::new(head.x, head.bottom(), head.w, self.s(22));
            ctx.fill_rect(row, theme.chrome_bg);
            ctx.select_font(FontSlot::Status, false);
            let sh = ctx.text_height();
            let pct = (xp.ratio() * 100.0).round() as u32;
            let label = format!(
                "{} {pct}% · {} / {} ({}/{} 파일)",
                if xp.sending { "전송" } else { "수신" },
                human_bytes(xp.done_bytes),
                human_bytes(xp.total_bytes),
                xp.done_files,
                xp.total_files
            );
            ctx.text(
                row.x + self.s(12),
                row.y + (row.h - sh) / 2,
                row,
                &label,
                theme.text_dim,
            );
            // 우측 소형 막대.
            let bar_w = self.s(90);
            let bar_h = self.s(5);
            let bx = row.right() - bar_w - self.s(12);
            let by = row.y + (row.h - bar_h) / 2;
            ctx.fill_round_rect(
                Rect::new(bx, by, bar_w, bar_h),
                bar_h / 2,
                theme.panel_bg_alt,
            );
            let fw = (bar_w as f32 * xp.ratio()).round() as i32;
            if fw > 0 {
                ctx.fill_round_rect(
                    Rect::new(bx, by, fw, bar_h),
                    bar_h / 2,
                    if xp.sending { theme.accent } else { theme.ok },
                );
            }
        }

        // 스레드 — 아래부터 최신(마지막 줄이 입력창 위).
        ctx.select_font(FontSlot::Message, false);
        let line_h = self.s(28);
        let input = self.input_bar();
        let mut y = input.y - self.s(6) - line_h;
        // 하단에서 scroll개를 건너뛴다(위로 올려본 만큼 과거로).
        for line in self.lines.iter().rev().skip(self.scroll) {
            if y < head.bottom() {
                break; // 화면 위쪽 밖
            }
            let fg = if line.mine { theme.text } else { theme.accent };
            let prefix = if line.mine {
                nbeep_core::t(nbeep_core::Msg::ChatPrefixMe)
            } else {
                nbeep_core::t(nbeep_core::Msg::ChatPrefixPeer)
            };
            let clip = Rect::new(self.bounds.x, y, self.bounds.w, line_h);
            match &line.body {
                ChatBody::Text(t) => {
                    let text = format!("{prefix}{}", t.as_str());
                    ctx.text(self.bounds.x + self.s(12), y, clip, &text, fg);
                }
                ChatBody::Xfer(x) => {
                    let dir = if line.mine { "전송" } else { "수신" };
                    let state = match &x.state {
                        XferLineState::Waiting => "승인 대기".to_string(),
                        XferLineState::Active { done } => {
                            let pct = if x.size > 0 {
                                (*done as f64 / x.size as f64 * 100.0).round() as u32
                            } else {
                                0
                            };
                            format!("{dir} {pct}% · {}", human_bytes(*done))
                        }
                        XferLineState::Done { note } if note.is_empty() => "완료".to_string(),
                        XferLineState::Done { note } => format!("완료 — {note}"),
                        XferLineState::Failed { why } => format!("실패 — {why}"),
                    };
                    let text = format!(
                        "{prefix}[파일] {} ({}) · {state}",
                        x.name.as_str(),
                        human_bytes(x.size)
                    );
                    ctx.text(self.bounds.x + self.s(12), y, clip, &text, fg);
                    // 진행 중엔 우측에 소형 막대(헤더 진척 줄과 같은 문법).
                    if let XferLineState::Active { done } = x.state {
                        let ratio = if x.size > 0 {
                            (done as f32 / x.size as f32).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let bar_w = self.s(70);
                        let bar_h = self.s(5);
                        let bx = clip.right() - bar_w - self.s(12);
                        let by = y + (line_h - bar_h) / 2;
                        ctx.fill_round_rect(
                            Rect::new(bx, by, bar_w, bar_h),
                            bar_h / 2,
                            theme.panel_bg_alt,
                        );
                        let fw = (bar_w as f32 * ratio).round() as i32;
                        if fw > 0 {
                            ctx.fill_round_rect(
                                Rect::new(bx, by, fw, bar_h),
                                bar_h / 2,
                                if line.mine { theme.accent } else { theme.ok },
                            );
                        }
                    }
                }
            }
            y -= line_h;
        }

        // 입력창.
        ctx.fill_rect(input, theme.field_bg);
        ctx.fill_rect(
            Rect::new(input.x, input.y, input.w, self.s(1)),
            theme.border,
        );
        ctx.select_font(FontSlot::Message, false);
        let text = self.input.text();
        let tx = input.x + self.s(10);
        let ty = input.y + self.s(7);
        if text.is_empty() && self.preedit.is_empty() {
            ctx.text(
                tx,
                ty,
                input,
                "메시지 입력… (Enter 전송 · Esc 목록)",
                theme.text_dim,
            );
            // 빈 입력창에도 **Beam 커서**를 선두에 표시(입력 가능 상태 표시).
            ctx.fill_rect(
                Rect::new(
                    tx,
                    input.y + self.s(6),
                    self.s(2).max(1),
                    input.h - self.s(12),
                ),
                theme.accent,
            );
        } else {
            let chars: Vec<char> = text.chars().collect();
            let upto = |ctx: &mut dyn DrawCtx, n: usize| -> i32 {
                let prefix: String = chars[..n].iter().collect();
                ctx.text_width(&prefix)
            };
            // 선택 하이라이트(텍스트 뒤에 먼저).
            if let Some((a, b)) = self.input.selection() {
                let x0 = tx + upto(ctx, a);
                let x1 = tx + upto(ctx, b);
                ctx.fill_rect(
                    Rect::new(
                        x0,
                        input.y + self.s(4),
                        (x1 - x0).max(1),
                        input.h - self.s(8),
                    ),
                    theme.sel_bg,
                );
            }
            ctx.text(tx, ty, input, &text, theme.text);
            let cx = tx + upto(ctx, self.input.caret());
            if self.preedit.is_empty() {
                // 폰트 실측 픽셀 커서 — 캐럿까지 폭만큼 오른쪽에 세로선.
                ctx.fill_rect(
                    Rect::new(
                        cx,
                        input.y + self.s(6),
                        self.s(2).max(1),
                        input.h - self.s(12),
                    ),
                    theme.accent,
                );
            } else {
                // IME 조합 중 — 캐럿 위치에 프리에딧을 accent 색 + 밑줄로(확정 전).
                ctx.text(cx, ty, input, &self.preedit, theme.accent);
                let pw = ctx.text_width(&self.preedit);
                ctx.fill_rect(
                    Rect::new(cx, ty + self.s(16), pw.max(1), self.s(2).max(1)),
                    theme.accent,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> (ChatViewWidget, Invalidations) {
        let mut w = ChatViewWidget::new("김철수".into());
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 400, 300), &mut inv);
        (w, inv)
    }
    fn ch(w: &mut ChatViewWidget, c: char, inv: &mut Invalidations) {
        w.on_event(&InputEvent::Char { c, now_ms: 0 }, inv);
    }
    fn enter(w: &mut ChatViewWidget, inv: &mut Invalidations) {
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            inv,
        );
    }

    fn key(w: &mut ChatViewWidget, k: Key, inv: &mut Invalidations) {
        w.on_event(
            &InputEvent::Key {
                key: k,
                shift: false,
                primary: false,
            },
            inv,
        );
    }

    fn wheel_up(w: &mut ChatViewWidget, inv: &mut Invalidations) {
        w.on_event(&InputEvent::Wheel { delta: 120 }, inv); // 1노치 위(과거)
    }

    #[test]
    fn scroll_reveals_history_and_new_msg_snaps_to_latest() {
        let (mut w, mut inv) = widget();
        // bounds 400x300 · 헤더30·입력34 → 스레드 영역 ~236 / 24 ≈ 9줄 가시.
        for i in 0..30 {
            w.push_line(
                ChatLine::text(true, nbeep_core::sanitize_message(&format!("m{i}"))),
                &mut inv,
            );
        }
        assert_eq!(w.scroll(), 0, "push 시 최신 스냅");
        // 휠 위로 여러 번 → 과거로 스크롤.
        for _ in 0..5 {
            wheel_up(&mut w, &mut inv);
        }
        assert!(w.scroll() > 0, "위로 스크롤됨: {}", w.scroll());
        // PgUp은 한 페이지씩.
        let before = w.scroll();
        w.on_event(
            &InputEvent::Key {
                key: Key::PageUp,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.scroll() > before);
        // 새 메시지 도착 = 최신으로 스냅.
        w.push_line(
            ChatLine::text(false, nbeep_core::sanitize_message("새 메시지")),
            &mut inv,
        );
        assert_eq!(w.scroll(), 0, "새 메시지 = 하단 스냅");
    }

    #[test]
    fn scroll_clamped_to_history() {
        let (mut w, mut inv) = widget();
        for i in 0..3 {
            w.push_line(
                ChatLine::text(true, nbeep_core::sanitize_message(&format!("m{i}"))),
                &mut inv,
            );
        }
        // 3줄뿐이고 화면이 더 크면 스크롤 불가(상한 0).
        for _ in 0..10 {
            wheel_up(&mut w, &mut inv);
        }
        assert_eq!(w.scroll(), 0, "짧은 대화는 스크롤 상한 0");
    }

    #[test]
    fn preedit_shown_then_commit_inserts() {
        let (mut w, mut inv) = widget();
        // 조합 중 "한" 프리에딧 표시(확정 아님 — input엔 없음).
        w.set_preedit("한".into(), &mut inv);
        assert_eq!(w.preedit(), "한");
        assert_eq!(w.input(), "", "조합 중엔 확정 텍스트 없음");
        // 확정(Commit) = Char 도착 → 프리에딧 클리어 + input 삽입.
        ch(&mut w, '한', &mut inv);
        assert_eq!(w.preedit(), "", "확정 시 조합 종료");
        assert_eq!(w.input(), "한");
    }

    #[test]
    fn empty_preedit_ends_composition() {
        let (mut w, mut inv) = widget();
        w.set_preedit("ㅎ".into(), &mut inv);
        w.set_preedit(String::new(), &mut inv); // 조합 취소
        assert_eq!(w.preedit(), "");
    }

    #[test]
    fn caret_moves_and_inserts_mid_string() {
        let (mut w, mut inv) = widget();
        for c in "helo".chars() {
            ch(&mut w, c, &mut inv);
        }
        // 캐럿을 'l'과 'o' 사이로: End에서 Left 1회 → "hel|o"에 'l' 삽입 → "hello"
        key(&mut w, Key::Left, &mut inv);
        ch(&mut w, 'l', &mut inv);
        assert_eq!(w.input(), "hello");
        assert_eq!(w.input_caret(), 4);
    }

    #[test]
    fn typing_and_enter_produces_sanitized_outgoing() {
        let (mut w, mut inv) = widget();
        for c in "안녕 bob".chars() {
            ch(&mut w, c, &mut inv);
        }
        assert_eq!(w.input(), "안녕 bob");
        enter(&mut w, &mut inv);
        let out = w.take_outgoing().expect("발신 확정");
        assert_eq!(out.as_str(), "안녕 bob");
        assert_eq!(w.input(), "", "전송 후 입력창 비움");
        assert!(w.take_outgoing().is_none(), "1회성");
    }

    #[test]
    fn outgoing_is_sanitized_rlo_stripped() {
        // 발신 경로도 무해화 — RLO를 타이핑(붙여넣기)해도 스레드 타입에 못 들어간다.
        let (mut w, mut inv) = widget();
        for c in "a\u{202E}b".chars() {
            ch(&mut w, c, &mut inv);
        }
        enter(&mut w, &mut inv);
        assert_eq!(w.take_outgoing().unwrap().as_str(), "ab");
    }

    #[test]
    fn empty_or_whitespace_input_does_not_send() {
        let (mut w, mut inv) = widget();
        enter(&mut w, &mut inv);
        assert!(w.take_outgoing().is_none());
        ch(&mut w, ' ', &mut inv);
        enter(&mut w, &mut inv);
        assert!(w.take_outgoing().is_none(), "공백만 = 미전송");
    }

    // ── 파일 전송 스레드 항목(사용자 요청 08-10 — 송수신 이력·진행률·완료 잔존) ──

    fn xfer_state(line: &ChatLine) -> &XferLineState {
        match &line.body {
            ChatBody::Xfer(x) => &x.state,
            ChatBody::Text(_) => panic!("전송 항목이어야 한다"),
        }
    }

    #[test]
    fn xfer_line_lifecycle_waiting_active_done() {
        let (mut w, mut inv) = widget();
        w.push_line(
            ChatLine::xfer(true, nbeep_core::sanitize_message("a.bin"), 100),
            &mut inv,
        );
        assert_eq!(*xfer_state(&w.lines[0]), XferLineState::Waiting);
        assert!(w.update_xfer_line(true, XferLineState::Active { done: 40 }, &mut inv));
        assert_eq!(*xfer_state(&w.lines[0]), XferLineState::Active { done: 40 });
        assert!(w.update_xfer_line(
            true,
            XferLineState::Done {
                note: String::new()
            },
            &mut inv
        ));
        // 종결 후엔 갱신 대상이 없다 — 완료 기록은 불변.
        assert!(
            !w.update_xfer_line(true, XferLineState::Active { done: 99 }, &mut inv),
            "종결 항목은 갱신 불가"
        );
        assert_eq!(
            *xfer_state(&w.lines[0]),
            XferLineState::Done {
                note: String::new()
            }
        );
    }

    #[test]
    fn xfer_update_matches_direction_only() {
        let (mut w, mut inv) = widget();
        w.push_line(
            ChatLine::xfer(false, nbeep_core::sanitize_message("recv.bin"), 10),
            &mut inv,
        );
        // 발신 방향 갱신은 수신 항목을 건드리지 않는다.
        assert!(!w.update_xfer_line(true, XferLineState::Active { done: 1 }, &mut inv));
        assert_eq!(*xfer_state(&w.lines[0]), XferLineState::Waiting);
    }

    #[test]
    fn xfer_update_fifo_two_open_offers() {
        // 수신 오퍼 2건 대기 — 결정은 큐 순서(FIFO)라 **앞 항목**부터 갱신돼야 한다.
        let (mut w, mut inv) = widget();
        w.push_line(
            ChatLine::xfer(false, nbeep_core::sanitize_message("first.bin"), 10),
            &mut inv,
        );
        w.push_line(
            ChatLine::xfer(false, nbeep_core::sanitize_message("second.bin"), 20),
            &mut inv,
        );
        assert!(w.update_xfer_line(
            false,
            XferLineState::Failed {
                why: "거절함".into()
            },
            &mut inv
        ));
        assert_eq!(
            *xfer_state(&w.lines[0]),
            XferLineState::Failed {
                why: "거절함".into()
            },
            "앞(오래된) 오퍼가 먼저 갱신"
        );
        assert_eq!(*xfer_state(&w.lines[1]), XferLineState::Waiting);
    }

    #[test]
    fn xfer_update_skips_text_and_terminal_hits_open() {
        let (mut w, mut inv) = widget();
        // [종결된 전송] [텍스트] [새 전송] — 갱신은 미종결 항목에만.
        w.push_line(
            ChatLine::xfer(true, nbeep_core::sanitize_message("old.bin"), 10),
            &mut inv,
        );
        assert!(w.update_xfer_line(
            true,
            XferLineState::Failed {
                why: "취소".into()
            },
            &mut inv
        ));
        w.push_line(
            ChatLine::text(true, nbeep_core::sanitize_message("중간 메시지")),
            &mut inv,
        );
        w.push_line(
            ChatLine::xfer(true, nbeep_core::sanitize_message("new.bin"), 20),
            &mut inv,
        );
        assert!(w.update_xfer_line(true, XferLineState::Active { done: 5 }, &mut inv));
        assert_eq!(
            *xfer_state(&w.lines[0]),
            XferLineState::Failed {
                why: "취소".into()
            },
            "이전 종결 기록은 그대로"
        );
        assert_eq!(*xfer_state(&w.lines[2]), XferLineState::Active { done: 5 });
    }

    #[test]
    fn xfer_push_snaps_to_latest() {
        let (mut w, mut inv) = widget();
        for i in 0..30 {
            w.push_line(
                ChatLine::text(true, nbeep_core::sanitize_message(&format!("m{i}"))),
                &mut inv,
            );
        }
        for _ in 0..5 {
            wheel_up(&mut w, &mut inv);
        }
        assert!(w.scroll() > 0);
        w.push_line(
            ChatLine::xfer(false, nbeep_core::sanitize_message("f.bin"), 1),
            &mut inv,
        );
        assert_eq!(w.scroll(), 0, "전송 항목도 새 줄 = 최신 스냅");
    }

    #[test]
    fn backspace_edits_and_escape_requests_back() {
        let (mut w, mut inv) = widget();
        ch(&mut w, 'h', &mut inv);
        ch(&mut w, 'i', &mut inv);
        ch(&mut w, '\u{8}', &mut inv);
        assert_eq!(w.input(), "h");
        assert!(!w.take_back());
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(w.take_back(), "Esc = 복귀 요청");
        assert!(!w.take_back(), "1회성");
    }
}
