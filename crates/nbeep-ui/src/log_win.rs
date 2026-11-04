//! 로그 창(LOG-1 · 11-04 · docs/52 §5 "로그 창" 후속) — 호스트의 **메모리 링 버퍼**(상태바 문구 + 상세 로그 `dev …` 줄)를
//! 보여 준다. 파일 로깅(`log.enabled`)이 꺼져 있어도 링은 돌아 "방금 무슨 일이 있었나"를 창에서 본다(종전 = `log.view`로
//! 파일을 외부 프로그램에 여는 길뿐). 검색 = 공백으로 나눈 토큰 **AND**(대소문자 무시) · 휠 스크롤 · 바닥에 있으면 새 줄을
//! 따라간다(위로 올리면 멈춤) · \[복사\] = 보이는(걸러진) 줄 전부를 호스트가 클립보드에 · \[닫기\]/Esc.
//! 데이터의 단일 원천은 호스트(`sync`로 링과 일련번호를 넘긴다 · 위젯은 표시·검색·스크롤만).
use nbeep_core::{t, Msg};

use crate::controls::{Button, Control as _, TextBox};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 위젯이 내는 1회성 행동(호스트가 처리).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogAction {
    /// \[복사\] — 걸러진 줄 전부(줄바꿈 연결). 클립보드는 호스트 몫(플랫폼 층).
    Copy(String),
    /// 닫기(Esc · \[닫기\]).
    Close,
}

const PAD: i32 = 12;
const SEARCH_H: i32 = 26;
const ROW_H: i32 = 18;
const BTN_H: i32 = 26;
const BTN_W: i32 = 88;
const GAP: i32 = 8;

/// 로그 창 위젯.
#[derive(Debug)]
pub struct LogWidget {
    bounds: Rect,
    scale: f32,
    /// 링 스냅숏(오래된 것 → 새 것).
    lines: Vec<String>,
    /// 마지막으로 받은 링 일련번호(같으면 `sync`는 no-op).
    seq: u64,
    /// 검색 결과 = `lines`의 인덱스.
    shown: Vec<usize>,
    search: TextBox,
    copy: Button,
    close: Button,
    /// 첫 보이는 행(`shown` 인덱스).
    scroll: usize,
    /// 바닥 따라가기(위로 올리면 꺼지고 바닥에 닿으면 켜진다).
    follow: bool,
    action: Option<LogAction>,
}

impl Default for LogWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl LogWidget {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            lines: Vec::new(),
            seq: 0,
            shown: Vec::new(),
            search: TextBox::new(t(Msg::LogWinSearch)).with_clearable(),
            copy: Button::new(t(Msg::LogWinCopy)),
            close: Button::new(t(Msg::BtnClose)),
            scroll: 0,
            follow: true,
            action: None,
        }
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 호스트 링과 맞춘다 — `seq`가 같으면 비용 0(틱마다 불러도 된다). 반환 = 바뀌었는가(다시 그릴 것).
    pub fn sync<'a, I>(&mut self, seq: u64, ring: I, inv: &mut Invalidations) -> bool
    where
        I: IntoIterator<Item = &'a String>,
    {
        if seq == self.seq {
            return false;
        }
        self.seq = seq;
        self.lines = ring.into_iter().cloned().collect();
        self.refilter();
        inv.push(self.bounds);
        true
    }

    #[must_use]
    pub fn take_action(&mut self) -> Option<LogAction> {
        self.action.take()
    }

    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        if (self.scale - scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = scale;
        self.search.set_scale(scale);
        self.copy.set_scale(scale);
        self.close.set_scale(scale);
        self.relayout(inv);
    }

    /// 걸러진 줄 수(시험·덤프).
    #[must_use]
    pub fn shown_len(&self) -> usize {
        self.shown.len()
    }

    /// 걸러진 줄 전부(복사 원문).
    #[must_use]
    pub fn shown_text(&self) -> String {
        self.shown
            .iter()
            .map(|&i| self.lines[i].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 검색 토큰 AND(대소문자 무시) — 토큰이 없으면 전부.
    fn refilter(&mut self) {
        let q = self.search.text();
        let tokens: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
        self.shown = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                tokens.is_empty() || {
                    let low = l.to_lowercase();
                    tokens.iter().all(|tk| low.contains(tk.as_str()))
                }
            })
            .map(|(i, _)| i)
            .collect();
        self.clamp_scroll();
    }

    fn rows_area(&self) -> Rect {
        let b = self.bounds;
        let pad = self.s(PAD);
        let top = b.y + pad + self.s(SEARCH_H) + self.s(GAP);
        let bottom = b.bottom() - pad - self.s(BTN_H) - self.s(GAP);
        Rect::new(
            b.x + pad,
            top,
            (b.w - pad * 2).max(0),
            (bottom - top).max(0),
        )
    }

    fn visible_rows(&self) -> usize {
        usize::try_from(self.rows_area().h / self.s(ROW_H).max(1)).unwrap_or(0)
    }

    fn max_scroll(&self) -> usize {
        self.shown.len().saturating_sub(self.visible_rows())
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        if self.follow {
            self.scroll = max;
        } else {
            self.scroll = self.scroll.min(max);
        }
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(PAD);
        self.search.set_bounds(
            Rect::new(
                b.x + pad,
                b.y + pad,
                (b.w - pad * 2).max(0),
                self.s(SEARCH_H),
            ),
            inv,
        );
        let by = b.bottom() - pad - self.s(BTN_H);
        let bw = self.s(BTN_W);
        self.close
            .set_bounds(Rect::new(b.right() - pad - bw, by, bw, self.s(BTN_H)), inv);
        self.copy.set_bounds(
            Rect::new(
                b.right() - pad - bw * 2 - self.s(GAP),
                by,
                bw,
                self.s(BTN_H),
            ),
            inv,
        );
        self.clamp_scroll();
    }

    fn take_clicks(&mut self) {
        if self.close.take_clicked() {
            self.action = Some(LogAction::Close);
        } else if self.copy.take_clicked() {
            self.action = Some(LogAction::Copy(self.shown_text()));
        }
    }
}

impl Widget for LogWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.relayout(inv);
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.action = Some(LogAction::Close);
                return;
            }
            InputEvent::Wheel { delta } => {
                // 휠 한 노치 = 3행. 위로 올리면 따라가기 해제 · 바닥에 닿으면 다시 켠다.
                let step = 3usize;
                if delta > 0 {
                    self.scroll = self.scroll.saturating_sub(step);
                    self.follow = false;
                } else if delta < 0 {
                    self.scroll = (self.scroll + step).min(self.max_scroll());
                }
                self.follow = self.scroll >= self.max_scroll();
                inv.push(self.bounds);
                return;
            }
            InputEvent::MouseDown { x, y, .. }
            | InputEvent::RightDown { x, y }
            | InputEvent::DoubleClick { x, y, .. } => {
                let p = Point { x, y };
                if self.search.bounds().contains(p) {
                    self.search.on_event(ev, inv);
                } else {
                    for b in [&mut self.copy, &mut self.close] {
                        if b.bounds().contains(p) {
                            b.on_event(ev, inv);
                        }
                    }
                }
            }
            InputEvent::MouseUp { .. } | InputEvent::MouseMove { .. } => {
                self.search.on_event(ev, inv);
                self.copy.on_event(ev, inv);
                self.close.on_event(ev, inv);
            }
            _ => {
                // 키·문자·IME = 검색 상자(창의 유일한 입력).
                self.search.on_event(ev, inv);
            }
        }
        if self.search.take_changed().is_some() {
            self.refilter();
            inv.push(self.bounds);
        }
        self.take_clicks();
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        self.search.paint(ctx, theme);
        let area = self.rows_area();
        ctx.fill_rect(area, theme.chrome_bg);
        ctx.fill_rect(Rect::new(area.x, area.y, area.w, 1), theme.border);
        ctx.fill_rect(
            Rect::new(area.x, area.bottom() - 1, area.w, 1),
            theme.border,
        );
        ctx.select_font(FontSlot::Mono, false);
        let rh = self.s(ROW_H);
        let th = ctx.text_height();
        if self.shown.is_empty() {
            ctx.select_font(FontSlot::Status, false);
            ctx.text(
                area.x + self.s(8),
                area.y + self.s(8),
                area,
                t(Msg::LogWinEmpty),
                theme.text_dim,
            );
        } else {
            let n = self.visible_rows();
            for (k, &i) in self.shown.iter().skip(self.scroll).take(n).enumerate() {
                let y = area.y + rh * i32::try_from(k).unwrap_or(0) + (rh - th) / 2;
                let line = &self.lines[i];
                // `dev ` 접두(상세 로그)는 흐리게 — 상태바 문구와 한눈에 갈린다.
                let color = if line.contains("] dev ") || line.starts_with("dev ") {
                    theme.text_dim
                } else {
                    theme.text
                };
                let clip = Rect::new(
                    area.x,
                    area.y + rh * i32::try_from(k).unwrap_or(0),
                    area.w,
                    rh,
                );
                ctx.text(area.x + self.s(6), y, clip, line, color);
            }
        }
        // 상태 줄(왼쪽 아래): 보이는 줄 / 전체 · 따라가기.
        ctx.select_font(FontSlot::Status, false);
        let status = format!(
            "{} / {}{}",
            self.shown.len(),
            self.lines.len(),
            if self.follow { "  ·  ⤓" } else { "" }
        );
        let sy = b.bottom() - self.s(PAD) - self.s(BTN_H) + (self.s(BTN_H) - th) / 2;
        ctx.text(b.x + self.s(PAD), sy, b, &status, theme.text_dim);
        self.copy.paint(ctx, theme);
        self.close.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (1..=n)
            .map(|i| {
                if i % 10 == 0 {
                    format!("[00:00:{i:02}] dev chat/decide send_text seq={i}")
                } else {
                    format!("[00:00:{i:02}] status line {i}")
                }
            })
            .collect()
    }

    /// 검색 = 토큰 AND(대소문자 무시) · 바닥 따라가기 · 복사 원문 = 걸러진 줄 전부.
    #[test]
    fn filter_follow_and_copy() {
        let mut w = LogWidget::new();
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 600, 200), &mut inv);
        let ls = lines(50);
        assert!(w.sync(1, ls.iter(), &mut inv));
        assert!(!w.sync(1, ls.iter(), &mut inv), "같은 일련번호 = no-op");
        assert_eq!(w.shown_len(), 50);
        assert!(
            w.follow && w.scroll == w.max_scroll(),
            "새 줄은 바닥을 따라간다"
        );
        // 위로 올리면 따라가기 해제 · 바닥에 닿으면 복귀.
        w.on_event(&InputEvent::Wheel { delta: 1 }, &mut inv);
        assert!(!w.follow);
        for _ in 0..40 {
            w.on_event(&InputEvent::Wheel { delta: -1 }, &mut inv);
        }
        assert!(w.follow);
        // 검색: "DEV send" → dev 줄 5개(대소문자 무시 · AND).
        w.search.set_text("DEV send");
        w.refilter();
        assert_eq!(w.shown_len(), 5);
        assert!(w.shown_text().lines().all(|l| l.contains("send_text")));
        w.search.set_text("send nothing-here");
        w.refilter();
        assert_eq!(w.shown_len(), 0);
        // Esc = 닫기.
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.take_action(), Some(LogAction::Close));
    }
}
