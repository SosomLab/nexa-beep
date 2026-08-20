//! **대화함(Conversations)** — 대화 기록 관리 창(M3-23 · 사용자 확정 08-20).
//!
//! 격리함([`crate::quarantine_view`])과 같은 문법의 관리 창: 목록 + 파괴적 행위는
//! **2단계 확인**. 행 = 저장된 대화 기록 하나(1:1 상대 또는 그룹 · `data/history/*.seg`).
//! 위젯은 **파일시스템을 모른다** — 행 구성·삭제·백업·복원 실행은 전부 호스트 몫이고,
//! 여기서는 의도([`CvAction`])만 1회성으로 보고한다.
//!
//! 확정 사양(08-20 수집): 행 = 아바타·이름·마지막 시각·**마지막 메시지 1줄**·파일
//! 크기 · **행 클릭 = 그 대화 열기** · 행 휴지통 = 2단계 삭제 · 하단 = 전체 백업
//! (sealed 그대로)·복원(중복 덮어쓰기 병합)·전체 삭제(2단계) · 상단 **이름 필터**.

use crate::controls::{Button, Control as _, TextBox};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::{t, tf, Msg};

/// 대화함 행 — 저장된 대화 기록 하나(표시용 스냅샷 · 진실은 호스트).
#[derive(Clone, Debug)]
pub struct CRow {
    /// 행동 라우팅 키 = 세그 파일 stem(`{peer.short()}` 또는 `g-{uid.short()}`).
    pub key: String,
    /// 그룹 기록인가(이름 옆 그룹 태그).
    pub is_group: bool,
    /// 표시 이름(상대 표시 이름 · 그룹 이름 · 미매핑은 지문).
    pub name: String,
    /// 마지막 기록 시각 라벨(호스트가 포맷).
    pub when: String,
    /// 마지막 메시지 1줄 미리보기(흐린 글씨 · 없으면 공백).
    pub preview: String,
    /// 세그 파일 크기(디스크 관리 관점 — 격리함 문법).
    pub size: u64,
    /// 아바타(실사진·내장 — 없으면 이니셜 원).
    pub avatar: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 아바타 보더 색(공개분).
    pub border: Option<(u8, u8, u8)>,
    /// 이니셜 원 색 시드(키 지문 바이트 — 안정 배정).
    pub seed: Vec<u8>,
}

/// 위젯이 보고하는 의도(1회성) — 실행은 호스트.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CvAction {
    /// 행 클릭 = 그 대화 열기(키 = 세그 stem).
    Open(String),
    /// 행 휴지통 2단계 확정 = 그 기록 삭제.
    Delete(String),
    /// 하단 전체 삭제 2단계 확정.
    DeleteAll,
    /// 전체 백업(sealed 그대로 — 폴더 선택은 호스트).
    Backup,
    /// 복원(폴더 선택·병합은 호스트).
    Restore,
}

const ROW_H: i32 = 56;
const FILTER_H: i32 = 46;
const BAR_H: i32 = 64;
/// 행 우측 휴지통 열 폭.
const TRASH_W: i32 = 40;

fn human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// 대화함 위젯.
#[derive(Debug)]
pub struct ConvboxWidget {
    bounds: Rect,
    scale: f32,
    rows: Vec<CRow>,
    filter: TextBox,
    /// 목록 스크롤(px · 0 = 맨 위).
    scroll: i32,
    /// 필터 결과 안 선택 인덱스(키보드 탐색).
    sel: usize,
    /// 행 삭제 2단계 — 무장된 키(다른 행 조작 = 해제).
    confirm_del: Option<String>,
    /// 전체 삭제 2단계.
    confirm_clear: bool,
    backup: Button,
    restore: Button,
    clear: Button,
    action: Option<CvAction>,
    back: bool,
    /// 호스트 결과 한 줄(백업/복원/삭제 결과) — 확인 안내보다 우선.
    message: Option<String>,
}

impl ConvboxWidget {
    /// 행 목록으로 만든다(필터에 포커스).
    #[must_use]
    pub fn new(rows: Vec<CRow>) -> Self {
        let mut filter = TextBox::new(t(Msg::CvFilterPh));
        filter.set_focused(true);
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            rows,
            filter,
            scroll: 0,
            sel: 0,
            confirm_del: None,
            confirm_clear: false,
            backup: Button::new(t(Msg::CvBackup)),
            restore: Button::new(t(Msg::CvRestore)),
            clear: Button::new(t(Msg::CvClear)),
            action: None,
            back: false,
            message: None,
        }
    }

    /// 행 교체(삭제·복원 뒤 호스트가 재구성) — 선택·확인 상태 리셋.
    pub fn set_rows(&mut self, rows: Vec<CRow>, inv: &mut Invalidations) {
        self.rows = rows;
        self.sel = 0;
        self.confirm_del = None;
        self.confirm_clear = false;
        self.clamp_scroll();
        inv.push(self.bounds);
    }

    /// 호스트 결과 문장(백업/복원/삭제 결과).
    pub fn set_message(&mut self, m: impl Into<String>, inv: &mut Invalidations) {
        self.message = Some(m.into());
        inv.push(self.bounds);
    }

    /// 배율 지정 — 내부 컨트롤 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.filter.set_scale(self.scale);
        self.backup.set_scale(self.scale);
        self.restore.set_scale(self.scale);
        self.clear.set_scale(self.scale);
        self.relayout(inv);
    }

    /// 의도 회수(1회성).
    pub fn take_action(&mut self) -> Option<CvAction> {
        self.action.take()
    }

    /// 닫기 요청(Esc — 확인 단계에서는 확인 취소가 먼저).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    // ── 필터 TextBox 배관(프롬프트 위젯들과 같은 문법) ──
    /// 선택 복사.
    #[must_use]
    pub fn clipboard_copy(&self) -> Option<String> {
        self.filter.copy_selection()
    }
    /// 선택 잘라내기.
    pub fn clipboard_cut(&mut self, inv: &mut Invalidations) -> Option<String> {
        self.filter.cut_selection(inv)
    }
    /// 붙여넣기.
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        self.filter.paste(text, inv);
    }
    /// IME 조합 중 문자열 표시.
    pub fn set_preedit(&mut self, text: &str, inv: &mut Invalidations) {
        self.filter.set_preedit(text, inv);
    }
    /// 우클릭 편집 메뉴 행동(1회성).
    pub fn take_edit_ctx(&mut self) -> Option<crate::controls::EditCtxAction> {
        self.filter.take_edit_ctx()
    }
    /// 클립보드 텍스트 유무 주입.
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.filter.set_clipboard_has_text(yes);
    }
    /// 편집 팝업이 열려 있는가(호스트 Esc/Enter 가드).
    #[must_use]
    pub fn popup_open(&self) -> bool {
        self.filter.popup_open()
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 필터를 통과한 행 인덱스(이름 부분 일치 · 대소문자 무시).
    fn filtered(&self) -> Vec<usize> {
        let q = self.filter.text().trim().to_lowercase();
        (0..self.rows.len())
            .filter(|&i| q.is_empty() || self.rows[i].name.to_lowercase().contains(&q))
            .collect()
    }

    fn list_rect(&self) -> Rect {
        let b = self.bounds;
        let top = self.s(FILTER_H);
        Rect::new(b.x, b.y + top, b.w, b.h - top - self.s(BAR_H))
    }

    fn bar_rect(&self) -> Rect {
        let b = self.bounds;
        Rect::new(b.x, b.bottom() - self.s(BAR_H), b.w, self.s(BAR_H))
    }

    /// 필터 결과 `vi`번째 행의 사각(스크롤 반영).
    fn row_rect(&self, vi: usize) -> Rect {
        let lr = self.list_rect();
        let rh = self.s(ROW_H);
        Rect::new(lr.x, lr.y + rh * vi as i32 - self.scroll, lr.w, rh)
    }

    fn trash_rect(&self, row: Rect) -> Rect {
        let w = self.s(TRASH_W);
        Rect::new(row.right() - w, row.y, w, row.h)
    }

    fn clamp_scroll(&mut self) {
        let n = self.filtered().len() as i32;
        let max = (n * self.s(ROW_H) - self.list_rect().h).max(0);
        self.scroll = self.scroll.clamp(0, max);
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(10);
        self.filter.set_bounds(
            Rect::new(b.x + pad, b.y + pad, b.w - pad * 2, self.s(28)),
            inv,
        );
        let bar = self.bar_rect();
        let (bw, bh) = (self.s(96), self.s(28));
        let by = bar.y + bar.h - bh - self.s(10);
        self.clear
            .set_bounds(Rect::new(bar.right() - pad - bw, by, bw, bh), inv);
        self.restore.set_bounds(
            Rect::new(bar.right() - pad - bw * 2 - self.s(8), by, bw, bh),
            inv,
        );
        self.backup.set_bounds(
            Rect::new(bar.right() - pad - bw * 3 - self.s(16), by, bw, bh),
            inv,
        );
    }

    /// 목록 조작 공통 — 확인 상태 해제(파괴적 확인은 다른 조작으로 흐르지 않는다).
    fn cancel_confirms(&mut self) {
        self.confirm_del = None;
        self.confirm_clear = false;
    }
}

impl Widget for ConvboxWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.relayout(inv);
        self.clamp_scroll();
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        self.backup.on_event(ev, inv);
        self.restore.on_event(ev, inv);
        self.clear.on_event(ev, inv);
        if self.backup.take_clicked() {
            self.cancel_confirms();
            self.action = Some(CvAction::Backup);
            return;
        }
        if self.restore.take_clicked() {
            self.cancel_confirms();
            self.action = Some(CvAction::Restore);
            return;
        }
        if self.clear.take_clicked() {
            self.confirm_del = None;
            self.message = None;
            if self.confirm_clear {
                self.confirm_clear = false;
                self.action = Some(CvAction::DeleteAll);
            } else {
                self.confirm_clear = true; // 1단계 — 안내는 paint가 그린다
            }
            inv.push(self.bounds);
            return;
        }
        match *ev {
            InputEvent::Wheel { delta } => {
                let lr = self.list_rect();
                // 커서 위치를 모르니 목록 전용 창에서는 휠 = 목록(필터엔 휠이 없다).
                let _ = lr;
                self.scroll -= delta * self.s(ROW_H) / 120;
                self.clamp_scroll();
                inv.push(self.bounds);
            }
            InputEvent::MouseDown { x, y, .. } => {
                let p = Point { x, y };
                self.filter.set_focused(self.filter.bounds().contains(p));
                if !self.list_rect().contains(p) {
                    return;
                }
                let vis = self.filtered();
                for (vi, &ri) in vis.iter().enumerate() {
                    let r = self.row_rect(vi);
                    if !r.contains(p) {
                        continue;
                    }
                    self.sel = vi;
                    self.confirm_clear = false;
                    self.message = None;
                    let key = self.rows[ri].key.clone();
                    if self.trash_rect(r).contains(p) {
                        // 휴지통 — 2단계(첫 클릭 = 무장 · 같은 행 재클릭 = 확정).
                        if self.confirm_del.as_deref() == Some(key.as_str()) {
                            self.confirm_del = None;
                            self.action = Some(CvAction::Delete(key));
                        } else {
                            self.confirm_del = Some(key);
                        }
                    } else if self.confirm_del.is_some() {
                        self.confirm_del = None; // 본문 클릭 = 확인 취소(오조작 방지)
                    } else {
                        self.action = Some(CvAction::Open(key));
                    }
                    inv.push(self.bounds);
                    return;
                }
                // 빈 곳 클릭 = 확인 해제.
                self.cancel_confirms();
                inv.push(self.bounds);
            }
            InputEvent::Key { key, .. } => match key {
                Key::Escape if !self.filter.popup_open() => {
                    if self.confirm_del.is_some() || self.confirm_clear {
                        self.cancel_confirms();
                        inv.push(self.bounds);
                    } else {
                        self.back = true;
                    }
                }
                Key::Up => {
                    self.sel = self.sel.saturating_sub(1);
                    self.cancel_confirms();
                    inv.push(self.bounds);
                }
                Key::Down => {
                    if self.sel + 1 < self.filtered().len() {
                        self.sel += 1;
                        self.cancel_confirms();
                    }
                    inv.push(self.bounds);
                }
                Key::Enter if !self.filter.popup_open() => {
                    // Enter = 선택 행 열기(파괴적 행위는 Enter로 확정하지 않는다 —
                    // 격리함과 같은 규약: 삭제는 클릭만).
                    if let Some(&ri) = self.filtered().get(self.sel) {
                        self.action = Some(CvAction::Open(self.rows[ri].key.clone()));
                    }
                }
                _ => {
                    self.filter.on_event(ev, inv);
                }
            },
            _ => {
                let before = self.filter.text();
                self.filter.on_event(ev, inv);
                if self.filter.text() != before {
                    // 필터 변경 = 결과 집합이 바뀐다 — 선택·스크롤 리셋.
                    self.sel = 0;
                    self.scroll = 0;
                    self.cancel_confirms();
                    inv.push(self.bounds);
                }
            }
        }
        let _ = self.filter.take_committed(); // Enter는 위(열기)에서 처리
    }

    #[allow(clippy::too_many_lines)]
    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        self.filter.paint(ctx, theme);

        let lr = self.list_rect();
        let vis = self.filtered();
        if vis.is_empty() {
            ctx.select_font(FontSlot::Base, false);
            let msg = t(Msg::CvEmpty);
            let w = ctx.text_width(msg);
            ctx.text(
                lr.x + (lr.w - w) / 2,
                lr.y + self.s(30),
                lr,
                msg,
                theme.text_dim,
            );
        }
        let rh = self.s(ROW_H);
        for (vi, &ri) in vis.iter().enumerate() {
            let r = self.row_rect(vi);
            if r.bottom() <= lr.y || r.y >= lr.bottom() {
                continue; // 화면 밖(스크롤)
            }
            let row = &self.rows[ri];
            let armed = self.confirm_del.as_deref() == Some(row.key.as_str());
            ctx.fill_rect(
                Rect::new(
                    r.x,
                    r.y.max(lr.y),
                    r.w,
                    (r.bottom().min(lr.bottom()) - r.y.max(lr.y)).max(0),
                ),
                if vi == self.sel {
                    theme.sel_bg
                } else {
                    theme.panel_bg
                },
            );
            // 아바타(원형 · 이니셜 폴백) — 목록과 같은 시각 문법.
            let av_d = self.s(34);
            let av = Rect::new(r.x + self.s(10), r.y + (rh - av_d) / 2, av_d, av_d);
            if let Some(img) = &row.avatar {
                ctx.fill_ellipse(av, crate::avatar::avatar_color(&row.seed));
                ctx.image_scaled(av, img, lr);
            } else {
                crate::avatar::draw_avatar(ctx, av, &row.name, &row.seed, 6.0);
            }
            if let Some((br, bg, bb)) = row.border {
                let c = crate::theme::Color(
                    (u32::from(br) << 16) | (u32::from(bg) << 8) | u32::from(bb),
                );
                ctx.stroke_ellipse(av, c, self.s(2).max(2) as f32);
            }
            let tx = av.right() + self.s(10);
            let trash = self.trash_rect(r);
            // 1행: 이름(+그룹 태그) · 우측 = 마지막 시각.
            ctx.select_font(FontSlot::Base, false);
            let name_clip = Rect::new(tx, r.y, trash.x - tx - self.s(90), rh);
            ctx.text(tx, r.y + self.s(7), name_clip, &row.name, theme.text);
            ctx.select_font(FontSlot::Status, false);
            let sh = ctx.text_height();
            if row.is_group {
                let tag = t(Msg::CvGroupTag);
                let nw = {
                    ctx.select_font(FontSlot::Base, false);
                    let w = ctx.text_width(&row.name).min(name_clip.w);
                    ctx.select_font(FontSlot::Status, false);
                    w
                };
                ctx.text(tx + nw + self.s(8), r.y + self.s(10), r, tag, theme.accent);
            }
            let ww = ctx.text_width(&row.when);
            ctx.text(
                trash.x - ww - self.s(6),
                r.y + self.s(10),
                r,
                &row.when,
                theme.text_dim,
            );
            // 2행: 미리보기 1줄(무장 시 = 삭제 확인 안내) · 우측 = 파일 크기.
            let size_txt = human(row.size);
            let sw = ctx.text_width(&size_txt);
            let pv_clip = Rect::new(tx, r.y, trash.x - tx - sw - self.s(14), rh);
            if armed {
                ctx.text(
                    tx,
                    r.y + rh - sh - self.s(8),
                    pv_clip,
                    t(Msg::CvDelConfirm),
                    theme.danger,
                );
            } else {
                ctx.text(
                    tx,
                    r.y + rh - sh - self.s(8),
                    pv_clip,
                    &row.preview,
                    theme.text_dim,
                );
            }
            ctx.text(
                trash.x - sw - self.s(6),
                r.y + rh - sh - self.s(8),
                r,
                &size_txt,
                theme.text_dim,
            );
            // 휴지통(자체 작도 — 뚜껑 + 몸통 + 손잡이). 무장 = 위험색.
            let tc = if armed { theme.danger } else { theme.text_dim };
            let cx = trash.x + trash.w / 2;
            let cy = r.y + rh / 2;
            let lid_w = self.s(14);
            ctx.fill_rect(
                Rect::new(cx - lid_w / 2, cy - self.s(7), lid_w, self.s(2)),
                tc,
            );
            ctx.fill_rect(
                Rect::new(cx - self.s(3), cy - self.s(9), self.s(6), self.s(2)),
                tc,
            );
            let body_w = self.s(10);
            let body = Rect::new(cx - body_w / 2, cy - self.s(4), body_w, self.s(11));
            ctx.fill_rect(body, tc);
            // 몸통 세로 줄(파낸 슬릿 2개 — 배경색).
            let bgc = if vi == self.sel {
                theme.sel_bg
            } else {
                theme.panel_bg
            };
            ctx.fill_rect(
                Rect::new(cx - self.s(2), body.y + self.s(2), 1, body.h - self.s(4)),
                bgc,
            );
            ctx.fill_rect(
                Rect::new(cx + self.s(1), body.y + self.s(2), 1, body.h - self.s(4)),
                bgc,
            );
            // 구분선.
            ctx.fill_rect(
                Rect::new(r.x + self.s(10), r.bottom() - 1, r.w - self.s(20), 1),
                theme.panel_bg_alt,
            );
        }

        // 하단 바 — 결과/확인 문장 + 버튼 3종.
        let bar = self.bar_rect();
        ctx.fill_rect(bar, theme.panel_bg_alt);
        ctx.select_font(FontSlot::Status, false);
        let note = if self.confirm_clear {
            t(Msg::CvClearConfirm).to_string()
        } else if let Some(m) = &self.message {
            m.clone()
        } else {
            let n = vis.len();
            tf(Msg::CvCount, &[&n.to_string()])
        };
        ctx.text(
            bar.x + self.s(12),
            bar.y + self.s(8),
            bar,
            &note,
            if self.confirm_clear {
                theme.danger
            } else {
                theme.text_dim
            },
        );
        self.backup.paint(ctx, theme);
        self.restore.paint(ctx, theme);
        self.clear.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn row(key: &str, name: &str) -> CRow {
        CRow {
            key: key.into(),
            is_group: key.starts_with("g-"),
            name: name.into(),
            when: "08-20 09:00".into(),
            preview: "마지막 메시지".into(),
            size: 1234,
            avatar: None,
            border: None,
            seed: vec![1, 2, 3, 4],
        }
    }

    fn widget(rows: Vec<CRow>) -> (ConvboxWidget, Invalidations) {
        let mut w = ConvboxWidget::new(rows);
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 560, 420), &mut inv);
        (w, inv)
    }

    fn click(x: i32, y: i32) -> [InputEvent; 2] {
        [
            InputEvent::MouseDown {
                x,
                y,
                shift: false,
                primary: false,
            },
            InputEvent::MouseUp { x, y },
        ]
    }

    fn send(w: &mut ConvboxWidget, evs: &[InputEvent]) {
        let mut inv = Invalidations::default();
        for e in evs {
            w.on_event(e, &mut inv);
        }
    }

    /// 행 본문 클릭 = 열기 · 휴지통은 2단계여야 한다.
    #[test]
    fn row_click_opens_and_trash_needs_two_steps() {
        let (mut w, _) = widget(vec![row("aaaa1111", "가나다"), row("g-bbbb2222", "테스트")]);
        // 첫 행 본문(필터 46 아래 + 행 중앙) 클릭 = Open.
        send(&mut w, &click(200, 46 + 28));
        assert_eq!(w.take_action(), Some(CvAction::Open("aaaa1111".into())));
        // 휴지통 첫 클릭 = 무장만(행동 없음).
        send(&mut w, &click(560 - 20, 46 + 28));
        assert_eq!(w.take_action(), None);
        // 같은 자리 재클릭 = 삭제 확정.
        send(&mut w, &click(560 - 20, 46 + 28));
        assert_eq!(w.take_action(), Some(CvAction::Delete("aaaa1111".into())));
    }

    /// 무장 후 본문 클릭 = 확인 취소(열기도 삭제도 아님 — 오조작 방지).
    #[test]
    fn body_click_cancels_armed_delete() {
        let (mut w, _) = widget(vec![row("aaaa1111", "가나다")]);
        send(&mut w, &click(560 - 20, 46 + 28)); // 무장
        send(&mut w, &click(200, 46 + 28)); // 본문 = 취소
        assert_eq!(w.take_action(), None);
        // 다시 본문 = 정상 열기.
        send(&mut w, &click(200, 46 + 28));
        assert_eq!(w.take_action(), Some(CvAction::Open("aaaa1111".into())));
    }

    /// 전체 삭제 버튼도 2단계 · Esc = 확인 취소(닫기 아님).
    #[test]
    fn clear_all_two_steps_and_escape_cancels() {
        let (mut w, _) = widget(vec![row("aaaa1111", "가나다")]);
        let clear_r = w.clear.bounds();
        let (cx, cy) = (clear_r.x + clear_r.w / 2, clear_r.y + clear_r.h / 2);
        send(&mut w, &click(cx, cy));
        assert_eq!(w.take_action(), None, "1단계 = 무장만");
        send(
            &mut w,
            &[InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            }],
        );
        assert!(!w.take_back(), "확인 중 Esc = 확인 취소(닫기 아님)");
        send(&mut w, &click(cx, cy));
        assert_eq!(w.take_action(), None, "취소 뒤 첫 클릭 = 다시 1단계");
        send(&mut w, &click(cx, cy));
        assert_eq!(w.take_action(), Some(CvAction::DeleteAll));
    }

    /// 이름 필터 = 부분 일치 · 필터 결과에서 클릭해도 올바른 키가 나간다.
    #[test]
    fn filter_narrows_and_click_targets_filtered_row() {
        let (mut w, _) = widget(vec![
            row("aaaa1111", "가나다"),
            row("bbbb2222", "sybae"),
            row("g-cccc3333", "테스트그룹"),
        ]);
        assert_eq!(w.filtered().len(), 3);
        let mut inv = Invalidations::default();
        for c in "sy".chars() {
            w.on_event(&InputEvent::Char { c, now_ms: 0 }, &mut inv);
        }
        assert_eq!(w.filtered(), vec![1], "필터 = 이름 부분 일치");
        // 필터 결과 첫 행 클릭 = 원본 2번째 행의 키.
        send(&mut w, &click(200, 46 + 28));
        assert_eq!(w.take_action(), Some(CvAction::Open("bbbb2222".into())));
    }

    /// 백업·복원 버튼 = 즉시 의도 보고(파괴적이 아니라 1단계).
    #[test]
    fn backup_restore_report_once() {
        let (mut w, _) = widget(vec![row("aaaa1111", "가나다")]);
        let b = w.backup.bounds();
        send(&mut w, &click(b.x + b.w / 2, b.y + b.h / 2));
        assert_eq!(w.take_action(), Some(CvAction::Backup));
        let r = w.restore.bounds();
        send(&mut w, &click(r.x + r.w / 2, r.y + r.h / 2));
        assert_eq!(w.take_action(), Some(CvAction::Restore));
        assert_eq!(w.take_action(), None, "1회성");
    }

    /// 빈 목록 = 행동 없음 · Esc = 닫기.
    #[test]
    fn empty_list_escape_closes() {
        let (mut w, _) = widget(vec![]);
        send(&mut w, &click(200, 46 + 28));
        assert_eq!(w.take_action(), None);
        send(
            &mut w,
            &[InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            }],
        );
        assert!(w.take_back());
    }
}
