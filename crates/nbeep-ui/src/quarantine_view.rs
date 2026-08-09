//! **격리함 화면** — 격리물 목록 + **등급별 마찰 승인**([docs/11] §7 · M4-3).
//!
//! 이 위젯은 파일시스템을 모른다 — 호스트가 `.beepq`를 읽어 [`QRow`]로 넘기고, 위젯은
//! **사용자 결정**([`QAction`])만 보고한다. 실체화·삭제는 호스트가 수행한다(계층 분리).
//!
//! ## 마찰 규칙([docs/11] §7 표 그대로)
//!
//! | 등급 | 화면 | 기본 버튼 |
//! |---|---|---|
//! | 🟢 데이터 | 1클릭 저장 | 저장 |
//! | 🟡 아카이브 | "자동으로 풀지 않습니다" 안내 | 저장 |
//! | 🟠 능동 문서 | 경고 + 보호된 보기 안내 | **취소** |
//! | 🔴 실행형 | **2단계**(고지 → 재확인) | **취소** |
//!
//! 공통: **형식 불일치 경고는 등급과 별개로 최상단** · 발신자 신뢰 상태를 같은 화면에 ·
//! 🔴은 기본 버튼이 항상 취소라 **Enter 연타로 통과되지 않는다**.

use crate::controls::{Button, Control as _};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::peer_list::badge;
use crate::theme::{Color, Theme};
use crate::widget::{Invalidations, Widget};
use nbeep_core::{t, Msg, RiskLevel, TrustLevel};

/// 목록 한 줄 — 호스트가 `.beepq` 메타에서 채운다.
#[derive(Clone, Debug)]
pub struct QRow {
    /// 원본 파일명(표시용).
    pub name: String,
    /// 위험 등급.
    pub risk: RiskLevel,
    /// 확장자 주장 ≠ 매직 실체(최상단 경고).
    pub mismatch: bool,
    /// 원본 크기(바이트).
    pub size: u64,
    /// 발신자 신뢰 상태(같은 화면에 표시 — [docs/11] §7 공통).
    pub trust: TrustLevel,
    /// `.beepq` 경로(호스트가 행을 되찾는 열쇠).
    pub path: String,
}

/// 사용자 결정 — 호스트가 실행한다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QAction {
    /// 실체화 승인(등급별 마찰을 이미 통과한 상태).
    Approve(String),
    /// 거부 — 격리물 삭제.
    Reject(String),
}

/// 행 높이(논리 px).
const ROW_H: i32 = 46;
/// 하단 버튼 바 높이.
const BAR_H: i32 = 44;

/// 등급 라벨 + 색.
fn risk_badge(risk: RiskLevel, theme: &Theme) -> (&'static str, Color) {
    match risk {
        RiskLevel::Executable => (t(Msg::RiskExec), theme.danger),
        RiskLevel::ActiveDocument => (t(Msg::RiskActive), theme.warn),
        RiskLevel::Archive => (t(Msg::RiskArchive), theme.accent),
        RiskLevel::Data => (t(Msg::RiskData), theme.ok),
    }
}

/// 등급별 안내 문구([docs/11] §7).
fn risk_note(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Executable => t(Msg::RiskExecNote),
        RiskLevel::ActiveDocument => t(Msg::RiskActiveNote),
        RiskLevel::Archive => t(Msg::RiskArchiveNote),
        RiskLevel::Data => t(Msg::RiskDataNote),
    }
}

/// 사람이 읽는 크기.
fn human(bytes: u64) -> String {
    const K: u64 = 1024;
    match bytes {
        b if b >= K * K => format!("{:.1}MiB", b as f64 / (K * K) as f64),
        b if b >= K => format!("{:.1}KiB", b as f64 / K as f64),
        b => format!("{b}B"),
    }
}

/// 격리함 위젯.
#[derive(Debug)]
pub struct QuarantineWidget {
    bounds: Rect,
    scale: f32,
    rows: Vec<QRow>,
    sel: usize,
    /// 🔴 실행형 2단계 확인 중(고지 → 재확인).
    confirming: bool,
    approve: Button,
    reject: Button,
    action: Option<QAction>,
    back: bool,
    /// 호스트가 넣는 결과 문장(승인·삭제 결과) — 등급 안내보다 우선 표시.
    message: Option<(String, bool)>,
    /// 이 세션에서 실체화한 경로 — 행에 "실체화됨" 태그. `.beepq`는 보존되므로
    /// 목록이 그대로라 **아무 일도 안 난 것처럼 보이던** 문제를 없앤다.
    done: std::collections::HashSet<String>,
}

impl QuarantineWidget {
    /// 행 목록으로 만든다.
    #[must_use]
    pub fn new(rows: Vec<QRow>) -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            rows,
            sel: 0,
            confirming: false,
            approve: Button::new(t(Msg::QApprove)),
            reject: Button::new(t(Msg::QReject)),
            action: None,
            back: false,
            message: None,
            done: std::collections::HashSet::new(),
        }
    }

    /// 호스트 결과 문장 표시(실패면 `is_error = true` — 위험색).
    pub fn set_message(
        &mut self,
        text: impl Into<String>,
        is_error: bool,
        inv: &mut Invalidations,
    ) {
        self.message = Some((text.into(), is_error));
        inv.push(self.bounds);
    }

    /// 이 경로가 실체화되었음을 표시한다(행 태그).
    pub fn mark_done(&mut self, path: impl Into<String>) {
        self.done.insert(path.into());
    }

    /// 행 갱신(승인·삭제 후 호스트가 다시 채운다).
    pub fn set_rows(&mut self, rows: Vec<QRow>, inv: &mut Invalidations) {
        self.sel = self.sel.min(rows.len().saturating_sub(1));
        self.rows = rows;
        self.confirming = false;
        inv.push(self.bounds);
    }

    /// 배율 지정.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.approve.set_scale(self.scale);
        self.reject.set_scale(self.scale);
        inv.push(self.bounds);
    }

    /// 사용자 결정(1회성).
    pub fn take_action(&mut self) -> Option<QAction> {
        self.action.take()
    }

    /// Esc 닫기 요청(1회성).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    fn row_rect(&self, i: usize) -> Rect {
        let rh = self.s(ROW_H);
        Rect::new(
            self.bounds.x,
            self.bounds.y + rh * i as i32,
            self.bounds.w,
            rh,
        )
    }

    fn selected(&self) -> Option<&QRow> {
        self.rows.get(self.sel)
    }

    /// 승인 누름 — 🔴 실행형은 **2단계**(첫 누름 = 고지, 두 번째 = 확정).
    fn press_approve(&mut self, inv: &mut Invalidations) {
        let Some(row) = self.selected() else { return };
        let two_step = matches!(row.risk, RiskLevel::Executable);
        if two_step && !self.confirming {
            self.confirming = true; // 고지 단계 — 아직 승인 아님
            inv.push(self.bounds);
            return;
        }
        self.action = Some(QAction::Approve(row.path.clone()));
        self.confirming = false;
        inv.push(self.bounds);
    }

    fn press_reject(&mut self, inv: &mut Invalidations) {
        if let Some(row) = self.selected() {
            self.action = Some(QAction::Reject(row.path.clone()));
            self.confirming = false;
            inv.push(self.bounds);
        }
    }

    fn bar_rect(&self) -> Rect {
        let h = self.s(BAR_H);
        Rect::new(self.bounds.x, self.bounds.bottom() - h, self.bounds.w, h)
    }
}

impl Widget for QuarantineWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        let bar = self.bar_rect();
        let bw = self.s(84);
        let bh = self.s(26);
        let by = bar.y + (bar.h - bh) / 2;
        self.approve.set_bounds(
            Rect::new(bar.right() - bw * 2 - self.s(18), by, bw, bh),
            inv,
        );
        self.reject
            .set_bounds(Rect::new(bar.right() - bw - self.s(10), by, bw, bh), inv);
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        self.approve.on_event(ev, inv);
        self.reject.on_event(ev, inv);
        if self.approve.take_clicked() {
            self.press_approve(inv);
            return;
        }
        if self.reject.take_clicked() {
            self.press_reject(inv);
            return;
        }
        match *ev {
            InputEvent::MouseDown { x, y, .. } => {
                let p = Point { x, y };
                for i in 0..self.rows.len() {
                    if self.row_rect(i).contains(p) {
                        if i != self.sel {
                            self.sel = i;
                            self.confirming = false; // 다른 행 = 확인 단계 취소
                            self.message = None;
                        }
                        inv.push(self.bounds);
                        break;
                    }
                }
            }
            InputEvent::Key { key, .. } => match key {
                Key::Escape => {
                    // 확인 단계에서는 Esc가 **승인 취소**(창 닫기 아님).
                    if self.confirming {
                        self.confirming = false;
                        inv.push(self.bounds);
                    } else {
                        self.back = true;
                    }
                }
                Key::Up => {
                    self.sel = self.sel.saturating_sub(1);
                    self.confirming = false;
                    inv.push(self.bounds);
                }
                Key::Down => {
                    if self.sel + 1 < self.rows.len() {
                        self.sel += 1;
                        self.confirming = false;
                    }
                    inv.push(self.bounds);
                }
                // ⚠️ Enter로는 승인하지 않는다 — 🔴 기본 버튼이 취소여야 하고(§7),
                // 연타 통과를 막으려면 승인은 **버튼 클릭**만 받는다.
                _ => {}
            },
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        if self.rows.is_empty() {
            ctx.select_font(FontSlot::Base, false);
            let msg = t(Msg::QEmpty);
            let w = ctx.text_width(msg);
            ctx.text(
                b.x + (b.w - w) / 2,
                b.y + self.s(40),
                b,
                msg,
                theme.text_dim,
            );
            return;
        }

        let rh = self.s(ROW_H);
        for (i, row) in self.rows.iter().enumerate() {
            let r = self.row_rect(i);
            if r.y >= self.bar_rect().y {
                break; // 버튼 바 침범 방지(스크롤은 후속)
            }
            ctx.fill_rect(
                r,
                if i == self.sel {
                    theme.sel_bg
                } else {
                    theme.panel_bg
                },
            );

            // 1행: 이름 + 크기.
            ctx.select_font(FontSlot::Base, false);
            let th = ctx.text_height();
            ctx.text(r.x + self.s(12), r.y + self.s(6), r, &row.name, theme.text);
            let size_txt = human(row.size);
            let sw = ctx.text_width(&size_txt);
            ctx.text(
                r.right() - sw - self.s(12),
                r.y + self.s(6),
                r,
                &size_txt,
                theme.text_dim,
            );
            if self.done.contains(&row.path) {
                ctx.select_font(FontSlot::Status, false);
                let tag = t(Msg::QDoneTag);
                let tw = ctx.text_width(tag);
                ctx.text(
                    r.right() - sw - tw - self.s(24),
                    r.y + self.s(8),
                    r,
                    tag,
                    theme.ok,
                );
                ctx.select_font(FontSlot::Base, false);
            }

            // 2행: 등급 칩 + 신뢰 배지 + 불일치 경고.
            let (rl, rc) = risk_badge(row.risk, theme);
            ctx.select_font(FontSlot::Status, false);
            let sh = ctx.text_height();
            let chip_w = ctx.text_width(rl) + self.s(14);
            let chip = Rect::new(
                r.x + self.s(12),
                r.y + rh - sh - self.s(12),
                chip_w,
                sh + self.s(6),
            );
            ctx.fill_round_rect(chip, chip.h / 2, rc);
            ctx.text(
                chip.x + self.s(7),
                chip.y + (chip.h - sh) / 2,
                chip,
                rl,
                theme.text,
            );
            let (tl, _) = badge(row.trust, theme);
            let mut x = chip.right() + self.s(8);
            ctx.text(x, chip.y + (chip.h - sh) / 2, r, tl, theme.text_dim);
            if row.mismatch {
                x += ctx.text_width(tl) + self.s(10);
                ctx.text(
                    x,
                    chip.y + (chip.h - sh) / 2,
                    r,
                    "⚠ 형식 불일치",
                    theme.danger,
                );
            }
            let _ = th;
        }

        // 하단 바 — 선택 행 안내(등급별) 또는 🔴 재확인 고지.
        let bar = self.bar_rect();
        ctx.fill_rect(bar, theme.chrome_bg);
        ctx.fill_rect(Rect::new(bar.x, bar.y, bar.w, 1), theme.border);
        ctx.select_font(FontSlot::Status, false);
        let sh = ctx.text_height();
        let (msg, color) = if self.confirming {
            (t(Msg::QConfirmExec).to_string(), theme.danger)
        } else if let Some((m, err)) = &self.message {
            (m.clone(), if *err { theme.danger } else { theme.ok })
        } else {
            self.selected()
                .map_or((String::new(), theme.text_dim), |r| {
                    (risk_note(r.risk).to_string(), theme.text_dim)
                })
        };
        ctx.text(
            bar.x + self.s(12),
            bar.y + (bar.h - sh) / 2,
            bar,
            &msg,
            color,
        );
        self.approve.paint(ctx, theme);
        self.reject.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, risk: RiskLevel) -> QRow {
        QRow {
            name: name.into(),
            risk,
            mismatch: false,
            size: 1234,
            trust: TrustLevel::Unverified,
            path: format!("/q/{name}.beepq"),
        }
    }

    fn widget(rows: Vec<QRow>) -> (QuarantineWidget, Invalidations) {
        let mut w = QuarantineWidget::new(rows);
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 600, 400), &mut inv);
        (w, inv)
    }

    fn click(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }
    fn key(key: Key) -> InputEvent {
        InputEvent::Key {
            key,
            shift: false,
            primary: false,
        }
    }
    /// 승인 버튼을 실제로 누른다(눌림→뗌).
    fn press_approve(w: &mut QuarantineWidget, inv: &mut Invalidations) {
        let b = w.approve.bounds();
        w.on_event(&click(b.x + 5, b.y + 5), inv);
        w.on_event(
            &InputEvent::MouseUp {
                x: b.x + 5,
                y: b.y + 5,
            },
            inv,
        );
    }

    #[test]
    fn data_file_approves_in_one_click() {
        let (mut w, mut inv) = widget(vec![row("a.txt", RiskLevel::Data)]);
        press_approve(&mut w, &mut inv);
        assert_eq!(
            w.take_action(),
            Some(QAction::Approve("/q/a.txt.beepq".into())),
            "🟢 = 1클릭"
        );
    }

    #[test]
    fn executable_requires_two_steps() {
        let (mut w, mut inv) = widget(vec![row("evil.exe", RiskLevel::Executable)]);
        press_approve(&mut w, &mut inv);
        assert!(w.take_action().is_none(), "🔴 첫 누름 = 고지만");
        assert!(w.confirming, "재확인 단계 진입");
        press_approve(&mut w, &mut inv);
        assert_eq!(
            w.take_action(),
            Some(QAction::Approve("/q/evil.exe.beepq".into())),
            "두 번째 누름 = 확정"
        );
    }

    #[test]
    fn escape_cancels_confirmation_before_closing() {
        let (mut w, mut inv) = widget(vec![row("evil.exe", RiskLevel::Executable)]);
        press_approve(&mut w, &mut inv);
        assert!(w.confirming);
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(!w.confirming, "Esc = 승인 취소");
        assert!(!w.take_back(), "확인 취소가 창 닫기로 새지 않는다");
        w.on_event(&key(Key::Escape), &mut inv);
        assert!(w.take_back(), "두 번째 Esc = 창 닫기");
    }

    #[test]
    fn enter_never_approves() {
        // §7: 🔴 기본 버튼은 항상 취소 — Enter 연타로 통과되면 안 된다.
        let (mut w, mut inv) = widget(vec![row("evil.exe", RiskLevel::Executable)]);
        for _ in 0..5 {
            w.on_event(&key(Key::Enter), &mut inv);
        }
        assert!(w.take_action().is_none());
        assert!(!w.confirming);
    }

    #[test]
    fn changing_row_resets_confirmation() {
        let (mut w, mut inv) = widget(vec![
            row("evil.exe", RiskLevel::Executable),
            row("b.txt", RiskLevel::Data),
        ]);
        press_approve(&mut w, &mut inv);
        assert!(w.confirming);
        w.on_event(&key(Key::Down), &mut inv);
        assert!(!w.confirming, "다른 행 선택 = 확인 단계 취소");
        // 이제 승인하면 새로 선택한 데이터 파일이 대상.
        press_approve(&mut w, &mut inv);
        assert_eq!(
            w.take_action(),
            Some(QAction::Approve("/q/b.txt.beepq".into()))
        );
    }

    #[test]
    fn reject_reports_selected_path() {
        let (mut w, mut inv) = widget(vec![row("a.txt", RiskLevel::Data)]);
        let b = w.reject.bounds();
        w.on_event(&click(b.x + 5, b.y + 5), &mut inv);
        w.on_event(
            &InputEvent::MouseUp {
                x: b.x + 5,
                y: b.y + 5,
            },
            &mut inv,
        );
        assert_eq!(
            w.take_action(),
            Some(QAction::Reject("/q/a.txt.beepq".into()))
        );
    }

    #[test]
    fn empty_list_has_no_action() {
        let (mut w, mut inv) = widget(Vec::new());
        press_approve(&mut w, &mut inv);
        assert!(w.take_action().is_none());
    }
}
