//! **전송 배치 패널** — 발신 대기·진행 창(`Role::Sending`)의 본체(M4-2d · 08-19).
//!
//! 종전엔 메시지 1줄 + 파일명이 박힌 버튼 하나였다(다중 파일이면 1개만 보이고,
//! 파일명이 잘렸다). 이 위젯은 **배치 전체**를 보인다:
//! - **고정 헤더** = 파일 개수 + 총 용량 + 승인 대기 안내 + **전체 제어**(일시정지·취소)
//! - **스크롤 목록** = 파일마다 이름(문자 단위 word-wrap) + 용량 + 상태 + **행별 제어**
//!
//! 제어는 일반 버튼이 아니라 **카세트 테이프식 아이콘**(사용자 확정 08-19 —
//! [`crate::icons::xfer`]). 상단 아이콘 = 배치 전체, 행 아이콘 = 그 파일만.
//!
//! 이 위젯은 전송 큐를 모른다 — 호스트가 [`SendFileRow`] 목록을 넘기고, 위젯은
//! **사용자 의도**([`SendAction`])만 보고한다(계층 분리 · `QuarantineWidget`과 동형).

use crate::controls::ScrollBars;
use crate::draw::{DrawCtx, FontSlot};
use crate::event::InputEvent;
use crate::geom::{Point, Rect};
use crate::theme::{Color, IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use nbeep_core::{t, tf, Msg};

/// 한 파일의 전송 상태(목록 행) — 아이콘 제어 조합을 정한다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SendStatus {
    /// 대기(큐 or 상대 승인 대기).
    Waiting,
    /// 전송 중(백분율).
    Active { pct: u8 },
    /// 일시정지(활성 멈춤 or 큐 보류).
    Paused,
    /// 완료.
    Done,
    /// 실패·취소.
    Failed,
}

/// 목록 행 — 호스트가 큐/배치에서 만들어 넘긴다.
#[derive(Clone, Debug)]
pub struct SendFileRow {
    /// 파일명(표시용).
    pub name: String,
    /// 크기(bytes).
    pub size: u64,
    /// 상태.
    pub status: SendStatus,
}

/// 사용자 의도 — 호스트가 큐/전송 조작으로 옮긴다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SendAction {
    /// 배치 전체 취소(상단).
    CancelAll,
    /// 배치 전체 일시정지(상단).
    PauseAll,
    /// 배치 전체 재개(상단).
    ResumeAll,
    /// 그 파일만 취소(행).
    Cancel(usize),
    /// 그 파일만 일시정지(행).
    Pause(usize),
    /// 그 파일만 재개(행).
    Resume(usize),
}

/// 사람이 읽는 크기(GiB까지 — 대용량 전송 UX).
fn human(bytes: u64) -> String {
    const K: u64 = 1024;
    match bytes {
        b if b >= K * K * K => format!("{:.1}GiB", b as f64 / (K * K * K) as f64),
        b if b >= K * K => format!("{:.1}MiB", b as f64 / (K * K) as f64),
        b if b >= K => format!("{:.1}KiB", b as f64 / K as f64),
        b => format!("{b}B"),
    }
}

/// 상태 라벨 + 색.
fn status_label(status: SendStatus, theme: &Theme) -> (String, Color) {
    match status {
        SendStatus::Waiting => (t(Msg::XferStWaiting).into(), theme.text_dim),
        SendStatus::Active { pct } => (format!("{} {pct}%", t(Msg::XferStActive)), theme.accent),
        SendStatus::Paused => (t(Msg::XferStPaused).into(), theme.warn),
        SendStatus::Done => (t(Msg::XferStDone).into(), theme.ok),
        SendStatus::Failed => (t(Msg::XferStFailed).into(), theme.danger),
    }
}

/// 문자 단위 줄바꿈(공백 없는 긴 파일명 대응 · `chat_view`와 동형 규칙).
fn wrap(ctx: &mut dyn DrawCtx, text: &str, max_w: i32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    let mut line_w = 0;
    let mut last_space: Option<usize> = None;
    for c in text.chars() {
        let cw = ctx.text_width(&c.to_string());
        if line_w + cw > max_w && !line.is_empty() {
            if let Some(bi) = last_space {
                let rest = line.split_off(bi);
                out.push(std::mem::take(&mut line));
                line = rest.trim_start().to_string();
                line_w = ctx.text_width(&line);
                last_space = None;
            } else {
                out.push(std::mem::take(&mut line));
                line_w = 0;
            }
        }
        if c == ' ' {
            last_space = Some(line.len());
        }
        line.push(c);
        line_w += cw;
    }
    out.push(line);
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// 전송 배치 패널.
pub struct SendBatchWidget {
    bounds: Rect,
    scale: f32,
    rows: Vec<SendFileRow>,
    bars: ScrollBars,
    off_y: i32,
    /// 마지막 페인트가 계산한 목록 총 높이·뷰포트(이벤트가 재계산 없이 쓴다).
    content_h: std::cell::Cell<i32>,
    vp: std::cell::Cell<Rect>,
    /// 마지막 페인트가 남긴 아이콘 히트 영역(클릭 판정 · `chat_view` 배너와 동형).
    hits: std::cell::RefCell<Vec<(Rect, SendAction)>>,
    cursor: (i32, i32),
    action: Option<SendAction>,
}

impl std::fmt::Debug for SendBatchWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendBatchWidget")
            .field("rows", &self.rows.len())
            .field("off_y", &self.off_y)
            .finish()
    }
}

impl SendBatchWidget {
    /// 새 패널.
    #[must_use]
    pub fn new(rows: Vec<SendFileRow>) -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            rows,
            bars: ScrollBars::new(),
            off_y: 0,
            content_h: std::cell::Cell::new(0),
            vp: std::cell::Cell::new(Rect::default()),
            hits: std::cell::RefCell::new(Vec::new()),
            cursor: (-1, -1),
            action: None,
        }
    }

    /// 목록 갱신(진행률·상태 변화 · 호스트가 매 이벤트에 태운다).
    pub fn set_rows(&mut self, rows: Vec<SendFileRow>, inv: &mut Invalidations) {
        self.rows = rows;
        inv.push(self.bounds);
    }

    /// 배율.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        if (self.scale - scale).abs() > f32::EPSILON {
            self.scale = scale;
            inv.push(self.bounds);
        }
    }

    /// 사용자 의도 회수(1회성).
    pub fn take_action(&mut self) -> Option<SendAction> {
        self.action.take()
    }

    /// 스크롤 틱(스크롤바 자동 숨김 · 호스트 about_to_wait).
    pub fn tick(&mut self, now_ms: u64, inv: &mut Invalidations) {
        if self.bars.tick(now_ms) {
            inv.push(self.vp.get());
        }
    }

    /// 상단 제어 아이콘의 (rect, action) 목록 — 페인트·이벤트 공용.
    fn top_controls(&self) -> Vec<(Rect, SendAction, &'static [u8])> {
        let s = self.scale;
        let b = self.bounds;
        let icon = (20.0 * s) as i32;
        let pad = (12.0 * s) as i32;
        let gap = (10.0 * s) as i32;
        let cy = b.y + pad;
        let mut out = Vec::new();
        // 오른쪽 끝 = 전체 취소.
        let cancel = Rect::new(b.right() - pad - icon, cy, icon, icon);
        out.push((cancel, SendAction::CancelAll, crate::icons::xfer::CANCEL_ALPHA));
        // 그 왼쪽 = 전체 일시정지/재개(상태에 따라).
        let any_pausable = self
            .rows
            .iter()
            .any(|r| matches!(r.status, SendStatus::Waiting | SendStatus::Active { .. }));
        let any_paused = self
            .rows
            .iter()
            .any(|r| matches!(r.status, SendStatus::Paused));
        let pr = Rect::new(cancel.x - gap - icon, cy, icon, icon);
        if any_pausable {
            out.push((pr, SendAction::PauseAll, crate::icons::xfer::PAUSE_ALPHA));
        } else if any_paused {
            out.push((pr, SendAction::ResumeAll, crate::icons::xfer::PLAY_ALPHA));
        }
        out
    }

    /// 아이콘을 테마색으로 틴트해 그린다(96×96 알파 → 정사각 dst).
    fn draw_icon(ctx: &mut dyn DrawCtx, dst: Rect, alpha: &'static [u8], color: Color, clip: Rect) {
        let img = IconImage::from_alpha_tinted(96, 96, alpha, color.rgb());
        ctx.image_scaled(dst, &img, clip);
    }

    /// 한 행의 제어 아이콘 (rect, action, alpha) — 상태별 조합.
    fn row_controls(&self, i: usize, row_rect: Rect) -> Vec<(Rect, SendAction, &'static [u8])> {
        let s = self.scale;
        let icon = (18.0 * s) as i32;
        let gap = (8.0 * s) as i32;
        let pad = (10.0 * s) as i32;
        let cy = row_rect.y + (row_rect.h - icon) / 2;
        let x_cancel = row_rect.right() - pad - icon;
        let x_pp = x_cancel - gap - icon;
        let mut out = Vec::new();
        match self.rows[i].status {
            SendStatus::Waiting | SendStatus::Active { .. } => {
                out.push((
                    Rect::new(x_pp, cy, icon, icon),
                    SendAction::Pause(i),
                    crate::icons::xfer::PAUSE_ALPHA,
                ));
                out.push((
                    Rect::new(x_cancel, cy, icon, icon),
                    SendAction::Cancel(i),
                    crate::icons::xfer::CANCEL_ALPHA,
                ));
            }
            SendStatus::Paused => {
                out.push((
                    Rect::new(x_pp, cy, icon, icon),
                    SendAction::Resume(i),
                    crate::icons::xfer::PLAY_ALPHA,
                ));
                out.push((
                    Rect::new(x_cancel, cy, icon, icon),
                    SendAction::Cancel(i),
                    crate::icons::xfer::CANCEL_ALPHA,
                ));
            }
            SendStatus::Done | SendStatus::Failed => {}
        }
        out
    }

    /// 헤더 높이(px).
    fn header_h(&self) -> i32 {
        let s = self.scale;
        (56.0 * s) as i32
    }
}

impl Widget for SendBatchWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        if let InputEvent::MouseMove { x, y } = ev {
            self.cursor = (*x, *y);
            inv.push(self.vp.get());
        }
        // 스크롤바·휠 먼저(뷰포트·내용 높이는 마지막 페인트값).
        let vp = self.vp.get();
        let content_h = self.content_h.get();
        let (_, noy, _) = self
            .bars
            .on_event(ev, vp, vp.w, content_h, 0, self.off_y, self.scale);
        if noy != self.off_y {
            self.off_y = noy;
            inv.push(vp);
        }
        // 클릭(MouseUp) = 아이콘 히트 판정.
        if let InputEvent::MouseUp { x, y, .. } = ev {
            let p = Point { x: *x, y: *y };
            let hit = self
                .hits
                .borrow()
                .iter()
                .find(|(r, _)| r.contains(p))
                .map(|(_, a)| *a);
            if let Some(a) = hit {
                self.action = Some(a);
                inv.push(self.bounds);
            }
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let s = self.scale;
        let b = self.bounds;
        let pad = (12.0 * s) as i32;
        ctx.fill_rect(b, theme.panel_bg);
        let mut hits: Vec<(Rect, SendAction)> = Vec::new();

        // ── 고정 헤더 ──
        let lineh = ctx.text_height() + (4.0 * s) as i32;
        let count = self.rows.len();
        let total: u64 = self.rows.iter().map(|r| r.size).sum();
        let summary = tf(Msg::XferBatchSummary, &[&count.to_string(), &human(total)]);
        ctx.select_font(FontSlot::Base, true);
        ctx.text(b.x + pad, b.y + pad, b, &summary, theme.text);
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            b.x + pad,
            b.y + pad + lineh,
            b,
            t(Msg::XferWaitApproval),
            theme.text_dim,
        );
        for (r, a, alpha) in self.top_controls() {
            let hover = r.contains(Point {
                x: self.cursor.0,
                y: self.cursor.1,
            });
            let color = if matches!(a, SendAction::CancelAll) {
                if hover {
                    theme.danger
                } else {
                    theme.text_dim
                }
            } else if hover {
                theme.accent
            } else {
                theme.text_dim
            };
            Self::draw_icon(ctx, r, alpha, color, b);
            hits.push((r, a));
        }
        // 구분선.
        let hh = self.header_h();
        ctx.fill_rect(Rect::new(b.x, b.y + hh - 1, b.w, 1), theme.border);

        // ── 스크롤 목록 ──
        let vp = Rect::new(b.x, b.y + hh, b.w, b.h - hh);
        self.vp.set(vp);
        let icon = (18.0 * s) as i32;
        let gap = (8.0 * s) as i32;
        let ctrl_w = icon * 2 + gap * 2 + pad; // 행 오른쪽 제어 열 폭
        let text_w = (vp.w - pad * 2 - ctrl_w).max((40.0 * s) as i32);
        let mut y = vp.y - self.off_y;
        for (i, row) in self.rows.iter().enumerate() {
            let name_lines = wrap(ctx, &row.name, text_w);
            let name_h = i32::try_from(name_lines.len()).unwrap_or(1) * lineh;
            let row_h = name_h + lineh + pad;
            let row_rect = Rect::new(vp.x, y, vp.w, row_h);
            // 뷰포트에 걸치는 행만 그린다.
            if row_rect.bottom() > vp.y && row_rect.y < vp.bottom() {
                // hover 배경(행).
                let row_hover = row_rect.contains(Point {
                    x: self.cursor.0,
                    y: self.cursor.1,
                }) && vp.contains(Point {
                    x: self.cursor.0,
                    y: self.cursor.1,
                });
                if row_hover {
                    ctx.fill_rect(
                        Rect::new(vp.x, row_rect.y, vp.w, row_h),
                        theme.panel_bg_alt,
                    );
                }
                // 파일명(word-wrap).
                ctx.select_font(FontSlot::Base, false);
                let mut ty = row_rect.y + (6.0 * s) as i32;
                for ln in &name_lines {
                    ctx.text(vp.x + pad, ty, vp, ln, theme.text);
                    ty += lineh;
                }
                // 용량 · 상태.
                let (st_txt, st_col) = status_label(row.status, theme);
                let size_txt = human(row.size);
                ctx.text(vp.x + pad, ty, vp, &size_txt, theme.text_dim);
                let sw = ctx.text_width(&size_txt);
                ctx.text(vp.x + pad + sw + (10.0 * s) as i32, ty, vp, &st_txt, st_col);
                // 행 제어 아이콘.
                for (r, a, alpha) in self.row_controls(i, row_rect) {
                    let hover = r.contains(Point {
                        x: self.cursor.0,
                        y: self.cursor.1,
                    });
                    let color = if matches!(a, SendAction::Cancel(_)) {
                        if hover {
                            theme.danger
                        } else {
                            theme.text_dim
                        }
                    } else if hover {
                        theme.accent
                    } else {
                        theme.text_dim
                    };
                    Self::draw_icon(ctx, r, alpha, color, vp);
                    hits.push((r, a));
                }
                // 행 구분선.
                ctx.fill_rect(
                    Rect::new(vp.x + pad, row_rect.bottom() - 1, vp.w - pad * 2, 1),
                    theme.border,
                );
            }
            y += row_h;
        }
        let content_h = y - (vp.y - self.off_y);
        self.content_h.set(content_h);
        self.bars
            .paint(ctx, theme, vp, vp.w, content_h, 0, self.off_y, s);

        *self.hits.borrow_mut() = hits;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn rows() -> Vec<SendFileRow> {
        vec![
            SendFileRow {
                name: "VMware-Fusion-25H2-24995814_universal.dmg".into(),
                size: 4_512_000_000,
                status: SendStatus::Active { pct: 12 },
            },
            SendFileRow {
                name: "report-final.pdf".into(),
                size: 12_000_000,
                status: SendStatus::Waiting,
            },
            SendFileRow {
                name: "photo.png".into(),
                size: 480_000,
                status: SendStatus::Paused,
            },
        ]
    }

    #[test]
    fn human_size_reaches_gib() {
        assert_eq!(human(4_512_000_000), "4.2GiB");
        assert_eq!(human(12_000_000), "11.4MiB");
        assert_eq!(human(480_000), "468.8KiB");
    }

    #[test]
    fn active_and_waiting_expose_pause_all() {
        let w = SendBatchWidget::new(rows());
        // 상단 = 취소 전체 + (활성/대기 있으니) 일시정지 전체.
        let top: Vec<SendAction> = w.top_controls().into_iter().map(|(_, a, _)| a).collect();
        assert!(top.contains(&SendAction::CancelAll));
        assert!(top.contains(&SendAction::PauseAll));
        assert!(!top.contains(&SendAction::ResumeAll));
    }

    #[test]
    fn all_paused_exposes_resume_all() {
        let mut r = rows();
        for row in &mut r {
            row.status = SendStatus::Paused;
        }
        let w = SendBatchWidget::new(r);
        let top: Vec<SendAction> = w.top_controls().into_iter().map(|(_, a, _)| a).collect();
        assert!(top.contains(&SendAction::ResumeAll));
        assert!(!top.contains(&SendAction::PauseAll));
    }

    #[test]
    fn paused_row_offers_resume_not_pause() {
        let w = SendBatchWidget::new(rows());
        let rr = Rect::new(0, 0, 400, 60);
        let acts: Vec<SendAction> = w.row_controls(2, rr).into_iter().map(|(_, a, _)| a).collect();
        assert!(acts.contains(&SendAction::Resume(2)));
        assert!(acts.contains(&SendAction::Cancel(2)));
        assert!(!acts.contains(&SendAction::Pause(2)));
    }

    #[test]
    fn active_row_offers_pause_and_cancel() {
        let w = SendBatchWidget::new(rows());
        let rr = Rect::new(0, 0, 400, 60);
        let acts: Vec<SendAction> = w.row_controls(0, rr).into_iter().map(|(_, a, _)| a).collect();
        assert!(acts.contains(&SendAction::Pause(0)));
        assert!(acts.contains(&SendAction::Cancel(0)));
    }
}
