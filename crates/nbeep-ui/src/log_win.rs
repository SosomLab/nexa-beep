//! 로그 창(LOG-1 · 11-04 · docs/52 §5 "로그 창" 후속) — 호스트의 **메모리 링 버퍼**(상태바 문구 + 상세 로그 `dev …` 줄)를
//! 보여 준다. 파일 로깅(`log.enabled`)이 꺼져 있어도 링은 돌아 "방금 무슨 일이 있었나"를 창에서 본다(종전 = `log.view`로
//! 파일을 외부 프로그램에 여는 길뿐). 검색 = 공백으로 나눈 토큰 **AND**(대소문자 무시) · 휠 스크롤 · 바닥에 있으면 새 줄을
//! 따라간다(위로 올리면 멈춤) · \[복사\] = 보이는(걸러진) 줄 전부를 호스트가 클립보드에 · \[닫기\]/Esc.
//! 데이터의 단일 원천은 호스트(`sync`로 링과 일련번호를 넘긴다 · 위젯은 표시·검색·스크롤만).
//!
//! **필터 칩 행**(11-04 후속 · docs/52 §5 "층/수준 필터"): 검색 상자 아래 칩 = `[전체] [상태] app disc conn sess chat xfer
//! user relay store ui | decide detail trace | FAIL만`. 층 칩은 **끄고 켜는 집합**(상태 = `dev ` 접두가 없는 상태바 줄 ·
//! `[전체]` = 전부 켬) · 수준 칩은 `dev` 줄에만 적용(`층/수준` 토큰 — 라벨은 번역하지 않는다 · 파일·grep과 같은 글자) ·
//! `FAIL만` = `| FAIL ` 결과 줄만. 검색 토큰과 AND. 칩은 창 너비에 맞춰 줄바꿈(최소 너비 480에서 3줄).
use nbeep_core::devlog::{Layer, Level, Rec};
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
const CHIP_H: i32 = 20;
const CHIP_GAP: i32 = 4;
/// 칩 최소 너비(라벨이 길면 글자 수로 늘린다 · 폭 측정은 paint에서만 가능하므로 글자 수 추정).
const CHIP_MIN_W: i32 = 30;
const CHIP_PAD_X: i32 = 7;
/// 칩 글자 폭 추정(Status 폰트 · ASCII 7px · 한글 12px).
const CHIP_CH_W: i32 = 7;
const CHIP_CH_W_WIDE: i32 = 12;
/// 층·수준 묶음 사이 간격(구분선 자리).
const CHIP_SEP: i32 = 10;

/// 칩 종류 — 층(0 = 상태바 줄 · 1..=10 = `Layer::ALL`) · 수준(1..=3 = decide/detail/trace) · FAIL만 · 전체 켬.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chip {
    All,
    Layer(u8),
    Level(u8),
    Fail,
}

/// 줄 메타(sync 때 한 번 해석 · 필터는 이걸로만 판단).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineMeta {
    /// 0 = 상태바 줄 · 1..=10 = `Layer as u8 + 1`.
    layer: u8,
    /// 0 = basic(상태) · 1..=3.
    level: u8,
    /// `| FAIL ` 결과.
    fail: bool,
}

/// 링 원소 본문(LOG-2 2단계 · 11-07 — **완전 지연 포맷**): 상세 로그는 `Rec` 구조체 그대로 링에 들어오고, 텍스트는
/// 로그 창이 **소비할 때**(sync) 한 번 만든다 — 켠 상태의 문자열 비용이 생산 쪽에서 0(docs/52 §5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogBody {
    /// 상태바 문구(M3-22 · 게이트 밖).
    Status(String),
    /// 상세 로그 레코드(docs/52).
    Dev(Rec),
}

/// 링 한 줄 = 시각(h·m·s) + 본문.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// 로컬 시각 `[h, m, s]`.
    pub hms: [u8; 3],
    pub body: LogBody,
}

impl LogLine {
    /// 표시 텍스트 — `[HH:MM:SS] 문구` / `[HH:MM:SS] dev 층/수준 기능 …`(종전 생산 쪽 포맷과 같은 모양).
    #[must_use]
    pub fn render(&self) -> String {
        let [h, m, s] = self.hms;
        match &self.body {
            LogBody::Status(t) => format!("[{h:02}:{m:02}:{s:02}] {t}"),
            LogBody::Dev(r) => format!("[{h:02}:{m:02}:{s:02}] dev {}", r.render()),
        }
    }

    fn meta(&self) -> LineMeta {
        match &self.body {
            LogBody::Status(_) => LineMeta {
                layer: 0,
                level: 0,
                fail: false,
            },
            LogBody::Dev(r) => LineMeta {
                layer: r.layer() as u8 + 1,
                level: r.level() as u8, // Basic 0 · Decide 1 · Detail 2 · Trace 3
                fail: r.is_fail(),
            },
        }
    }
}

/// 로그 창 위젯.
#[derive(Debug)]
pub struct LogWidget {
    bounds: Rect,
    scale: f32,
    /// 링 스냅숏(오래된 것 → 새 것).
    lines: Vec<String>,
    /// `lines`와 같은 길이의 메타(층·수준·FAIL).
    meta: Vec<LineMeta>,
    /// 층 필터(0 = 상태 줄 · 1..=10 = 층) — 켜진 것만 보인다 · 기본 전부.
    layers: [bool; 11],
    /// 수준 필터(1..=3 · `dev` 줄에만) — 기본 전부.
    levels: [bool; 4],
    /// FAIL 결과 줄만.
    fail_only: bool,
    /// 칩 배치(relayout) — 클릭 판정·그리기.
    chips: Vec<(Rect, Chip)>,
    /// 칩 행 전체 높이(줄바꿈 포함 · rows_area가 그만큼 내려간다).
    chips_h: i32,
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
            meta: Vec::new(),
            layers: [true; 11],
            levels: [true; 4],
            fail_only: false,
            chips: Vec::new(),
            chips_h: 0,
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
    ///
    /// LOG-2 2단계(11-07): 텍스트는 여기서 만든다(완전 지연 포맷) · **증분** — 일련번호 차이만큼 꼬리만 렌더해 붙이고
    /// 링 길이를 넘는 머리를 버린다(종전 = 매번 2000줄 전체 복사·재파싱 · 18줄/s면 틱마다). 차이가 링 길이를 넘거나
    /// 첫 동기면 전체 재구성.
    pub fn sync<'a, I>(&mut self, seq: u64, ring: I, inv: &mut Invalidations) -> bool
    where
        I: IntoIterator<Item = &'a LogLine>,
    {
        if seq == self.seq {
            return false;
        }
        let fresh = usize::try_from(seq.wrapping_sub(self.seq)).unwrap_or(usize::MAX);
        let entries: Vec<&LogLine> = ring.into_iter().collect();
        if self.seq == 0 || fresh >= entries.len() || self.lines.len() + fresh < entries.len() {
            self.lines = entries.iter().map(|l| l.render()).collect();
            self.meta = entries.iter().map(|l| l.meta()).collect();
        } else {
            for l in &entries[entries.len() - fresh..] {
                self.lines.push(l.render());
                self.meta.push(l.meta());
            }
            let over = self.lines.len() - entries.len();
            if over > 0 {
                self.lines.drain(..over);
                self.meta.drain(..over);
            }
        }
        self.seq = seq;
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

    /// 층 필터(0 = 상태 줄 · 1..=10 = `Layer::ALL` 순) — 시험·스크립트.
    pub fn set_layer_on(&mut self, idx: usize, on: bool) {
        if let Some(v) = self.layers.get_mut(idx) {
            *v = on;
            self.refilter();
        }
    }

    /// 수준 필터(1..=3 = decide/detail/trace).
    pub fn set_level_on(&mut self, idx: usize, on: bool) {
        if let Some(v) = self.levels.get_mut(idx) {
            *v = on;
            self.refilter();
        }
    }

    /// FAIL 결과 줄만.
    pub fn set_fail_only(&mut self, on: bool) {
        self.fail_only = on;
        self.refilter();
    }

    /// 필터 전부 해제(`[전체]` 칩).
    pub fn reset_filters(&mut self) {
        self.layers = [true; 11];
        self.levels = [true; 4];
        self.fail_only = false;
        self.refilter();
    }

    fn chip_on(&self, chip: Chip) -> bool {
        match chip {
            Chip::All => {
                self.layers.iter().all(|b| *b) && self.levels.iter().all(|b| *b) && !self.fail_only
            }
            Chip::Layer(i) => self.layers[usize::from(i)],
            Chip::Level(i) => self.levels[usize::from(i)],
            Chip::Fail => self.fail_only,
        }
    }

    fn chip_label(chip: Chip) -> &'static str {
        match chip {
            Chip::All => t(Msg::LogWinAll),
            Chip::Layer(0) => t(Msg::LogWinStatusChip),
            Chip::Layer(i) => Layer::ALL[usize::from(i - 1)].label(),
            Chip::Level(1) => Level::Decide.label(),
            Chip::Level(2) => Level::Detail.label(),
            Chip::Level(_) => Level::Trace.label(),
            Chip::Fail => t(Msg::LogWinFailOnly),
        }
    }

    fn chip_list() -> Vec<Chip> {
        let mut v = vec![Chip::All];
        v.extend((0..=10u8).map(Chip::Layer));
        v.extend((1..=3u8).map(Chip::Level));
        v.push(Chip::Fail);
        v
    }

    fn toggle_chip(&mut self, chip: Chip) {
        match chip {
            Chip::All => self.reset_filters(),
            Chip::Layer(i) => {
                let i = usize::from(i);
                self.set_layer_on(i, !self.layers[i]);
            }
            Chip::Level(i) => {
                let i = usize::from(i);
                self.set_level_on(i, !self.levels[i]);
            }
            Chip::Fail => self.set_fail_only(!self.fail_only),
        }
    }

    /// 칩 너비 추정(글자 수 · paint 밖이라 실측 불가 — 넉넉히).
    fn chip_w(&self, chip: Chip) -> i32 {
        let label = Self::chip_label(chip);
        let w: i32 = label
            .chars()
            .map(|ch| {
                if ch.is_ascii() {
                    CHIP_CH_W
                } else {
                    CHIP_CH_W_WIDE
                }
            })
            .sum();
        self.s((w + CHIP_PAD_X * 2).max(CHIP_MIN_W))
    }

    /// 칩 흐름 배치 — 창 너비에 맞춰 줄바꿈 · 층 묶음 뒤와 수준 묶음 뒤에 구분 간격.
    fn layout_chips(&mut self) {
        let b = self.bounds;
        let pad = self.s(PAD);
        let x0 = b.x + pad;
        let right = b.right() - pad;
        let y0 = b.y + pad + self.s(SEARCH_H) + self.s(GAP);
        let (ch, gap, sep) = (self.s(CHIP_H), self.s(CHIP_GAP), self.s(CHIP_SEP));
        let mut x = x0;
        let mut y = y0;
        let mut out = Vec::new();
        for chip in Self::chip_list() {
            let w = self.chip_w(chip);
            if x > x0 && x + w > right {
                x = x0;
                y += ch + gap;
            }
            out.push((Rect::new(x, y, w, ch), chip));
            x += w + gap;
            if matches!(chip, Chip::Layer(10) | Chip::Level(3)) {
                x += sep;
            }
        }
        self.chips = out;
        self.chips_h = y + ch - y0;
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

    /// 검색 토큰 AND(대소문자 무시 · 토큰이 없으면 전부) × 층/수준/FAIL 칩.
    fn refilter(&mut self) {
        let q = self.search.text();
        let tokens: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
        let (layers, levels, fail_only) = (self.layers, self.levels, self.fail_only);
        self.shown = self
            .lines
            .iter()
            .zip(self.meta.iter())
            .enumerate()
            .filter(|(_, (l, m))| {
                layers[usize::from(m.layer)]
                    && (m.level == 0 || levels[usize::from(m.level)])
                    && (!fail_only || m.fail)
                    && (tokens.is_empty() || {
                        let low = l.to_lowercase();
                        tokens.iter().all(|tk| low.contains(tk.as_str()))
                    })
            })
            .map(|(i, _)| i)
            .collect();
        self.clamp_scroll();
    }

    fn rows_area(&self) -> Rect {
        let b = self.bounds;
        let pad = self.s(PAD);
        let top = b.y + pad + self.s(SEARCH_H) + self.s(GAP) + self.chips_h + self.s(GAP);
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
        self.layout_chips();
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
                let hit = self
                    .chips
                    .iter()
                    .find(|(r, _)| r.contains(p))
                    .map(|(_, c)| *c);
                if let Some(chip) = hit {
                    if matches!(ev, InputEvent::MouseDown { .. }) {
                        self.toggle_chip(chip);
                        inv.push(self.bounds);
                    }
                } else if self.search.bounds().contains(p) {
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
        // 필터 칩 — 켜짐 = accent 채움 · 꺼짐 = field_bg + 테두리 · 흐린 글자.
        ctx.select_font(FontSlot::Status, false);
        let cth = ctx.text_height();
        for (r, chip) in &self.chips {
            let on = self.chip_on(*chip);
            ctx.fill_rect(*r, if on { theme.accent } else { theme.field_bg });
            if !on {
                ctx.fill_rect(Rect::new(r.x, r.y, r.w, 1), theme.border);
                ctx.fill_rect(Rect::new(r.x, r.bottom() - 1, r.w, 1), theme.border);
                ctx.fill_rect(Rect::new(r.x, r.y, 1, r.h), theme.border);
                ctx.fill_rect(Rect::new(r.right() - 1, r.y, 1, r.h), theme.border);
            }
            let label = Self::chip_label(*chip);
            let tw = ctx.text_width(label);
            ctx.text(
                r.x + (r.w - tw) / 2,
                r.y + (r.h - cth) / 2,
                *r,
                label,
                if on { theme.window_bg } else { theme.text_dim },
            );
        }
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

    fn at(i: usize, body: LogBody) -> LogLine {
        LogLine {
            hms: [0, 0, u8::try_from(i % 60).unwrap_or(0)],
            body,
        }
    }
    fn dev(layer: &str, level: Level, func: &'static str) -> Rec {
        Rec::new(Layer::parse(layer).expect("층 이름"), level, func)
    }
    fn lines(n: usize) -> Vec<LogLine> {
        (1..=n)
            .map(|i| {
                if i % 10 == 0 {
                    at(
                        i,
                        LogBody::Dev(
                            dev("chat", Level::Decide, "send_text")
                                .arg("seq", i)
                                .ok("sent"),
                        ),
                    )
                } else {
                    at(i, LogBody::Status(format!("status line {i}")))
                }
            })
            .collect()
    }

    /// LOG-2 2단계(11-07): 증분 동기 — 꼬리만 붙이고 링 길이를 넘는 머리를 버린다 · 전체 재구성과 결과가 같다.
    #[test]
    fn sync_is_incremental_and_matches_full_rebuild() {
        let mut inc = LogWidget::new();
        let mut full = LogWidget::new();
        let mut inv = Invalidations::default();
        let mut ring: std::collections::VecDeque<LogLine> = lines(30).into();
        assert!(inc.sync(30, ring.iter(), &mut inv));
        // 5줄 추가(상한 30 유지 — 앞 5줄 탈락) · seq 35.
        for i in 31..=35 {
            ring.pop_front();
            ring.push_back(at(i, LogBody::Status(format!("status line {i}"))));
        }
        assert!(inc.sync(35, ring.iter(), &mut inv));
        assert!(full.sync(35, ring.iter(), &mut inv));
        assert_eq!(inc.lines, full.lines, "증분 = 전체 재구성");
        assert_eq!(inc.meta, full.meta);
        assert_eq!(inc.lines.len(), 30);
        assert!(inc.lines[0].contains("status line 6"));
        assert!(inc.lines[29].contains("status line 35"));
        // 차이가 링 길이를 넘으면(오래 닫아 둠) 전체 재구성.
        assert!(inc.sync(500, ring.iter(), &mut inv));
        assert_eq!(inc.lines, full.lines);
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

    /// 층/수준/FAIL 칩(11-04): 메타 해석 · 집합 필터 · 검색과 AND · `[전체]` 복귀 · 칩 클릭.
    #[test]
    fn layer_level_fail_chips() {
        let mut w = LogWidget::new();
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 760, 480), &mut inv);
        let ls: Vec<LogLine> = vec![
            at(1, LogBody::Status("연결됨".to_string())),
            at(
                2,
                LogBody::Dev(
                    dev("conn", Level::Decide, "start_connect")
                        .arg("peer", "5016194e")
                        .rule("latch", "free")
                        .ok("dial"),
                ),
            ),
            at(
                3,
                LogBody::Dev(
                    dev("conn", Level::Decide, "connect_failed")
                        .arg("peer", "b7147214")
                        .arg("why", "NotFound")
                        .fail("auto retry stopped"),
                ),
            ),
            at(
                4,
                LogBody::Dev(dev("chat", Level::Trace, "recv_frame").arg("seq", 9).ok("")),
            ),
            at(
                5,
                LogBody::Dev(dev("xfer", Level::Detail, "chunk").arg("n", 3).ok("")),
            ),
        ];
        assert!(w.sync(1, ls.iter(), &mut inv));
        assert_eq!(w.shown_len(), 5);
        assert_eq!(
            w.meta[1],
            LineMeta {
                layer: Layer::Conn as u8 + 1,
                level: 1,
                fail: false
            }
        );
        assert_eq!(w.meta[0].layer, 0, "상태바 줄 = 층 0");
        // 상태 줄 끄기 → 4 · conn 끄기 → 2(chat trace · xfer detail).
        w.set_layer_on(0, false);
        assert_eq!(w.shown_len(), 4);
        w.set_layer_on(usize::from(Layer::Conn as u8) + 1, false);
        assert_eq!(w.shown_len(), 2);
        // 수준: trace 끄기 → xfer detail 1줄.
        w.set_level_on(3, false);
        assert_eq!(w.shown_len(), 1);
        assert!(w.shown_text().contains("xfer/detail"));
        // 전체 복귀 → FAIL만 → 1줄(연결 실패) · 검색과 AND.
        w.reset_filters();
        assert_eq!(w.shown_len(), 5);
        w.set_fail_only(true);
        assert_eq!(w.shown_len(), 1);
        assert!(w.shown_text().contains("connect_failed"));
        w.search.set_text("nothing");
        w.refilter();
        assert_eq!(w.shown_len(), 0);
        w.search.set_text("");
        // 칩 클릭 = 토글(`[전체]` 칩 좌표로 복귀).
        let all = w
            .chips
            .iter()
            .find(|(_, c)| *c == Chip::All)
            .map(|(r, _)| *r)
            .unwrap();
        w.on_event(
            &InputEvent::MouseDown {
                x: all.x + 2,
                y: all.y + 2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(!w.fail_only);
        assert_eq!(w.shown_len(), 5);
        // 칩은 창 너비 안에서 줄바꿈 — 760이면 2줄 이하 · 행 영역은 칩 아래에서 시작.
        assert!(w.chips_h <= w.s(CHIP_H) * 2 + w.s(CHIP_GAP));
        assert!(w.rows_area().y > w.search.bounds().bottom() + w.chips_h);
    }
}
