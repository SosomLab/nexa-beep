//! 내 기기 목록 창(10-09 사용자 요청 "내 기기 목록 전용 화면 — 기기 이름·마지막 접속·폐기").
//!
//! 같은 사용자 키로 묶인 기기(ADR-0015 A-1 **서명 기기 목록** · 호스트 `my_devices`)를 표로 보이고 기기마다
//! [폐기]를 둔다. **폐기 = Succession 부분 집합**: 설정의 "사용자 키 교체"(나만 남김)와 같은 경로로 사용자 키를
//! 바꾸되 **지목한 기기만 `revoked`** 에 넣고 나머지는 `devices`로 남긴다 — 남은 기기는 새 키 봉인본을 받아
//! "후계가 이긴다"로 따라온다(S2). 안전장치도 같다: 호스트가 **5초 무장 뒤 두 번째 클릭**에서 실행하고,
//! 그 사이 이 창의 노트가 뜻을 알려 준다. 데이터의 단일 원천은 호스트(`DevView`를 넘긴다 · 위젯은 표시·클릭만).
use nbeep_core::{t, Msg, PeerId};

use crate::controls::{Button, Control as _};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};

/// 기기 한 행(호스트가 채운다).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevRow {
    /// 기기 식별자(폐기 대상 지목용).
    pub peer: PeerId,
    /// 표시 이름(프로필 이름 → 발견 이름 → 지문 라벨).
    pub name: String,
    /// 짧은 지문(이름 뒤 흐리게).
    pub id_short: String,
    /// 이 PC 자신(폐기 불가 · "이 기기").
    pub is_me: bool,
    /// 지금 형제 세션이 살아 있다.
    pub online: bool,
    /// 마지막 접속 문구(접속 중 · N분 전 · 기록 없음).
    pub last_seen: String,
}

/// 창 보기 전체.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevView {
    /// 핸들(제목 줄).
    pub handle: String,
    /// 사용자 ID 짧은 표기.
    pub user_id: String,
    /// 기기 행(키 바이트 정렬 · 호스트 순서 그대로).
    pub rows: Vec<DevRow>,
}

/// 위젯이 내는 1회성 행동(호스트가 처리 — 폐기의 무장·실행은 호스트).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevAction {
    /// 행의 [폐기] 클릭(호스트: 첫 클릭 = 5초 무장 · 두 번째 = Succession 발행).
    Revoke(PeerId),
    /// 닫기(Esc · [닫기]).
    Close,
}

const PAD: i32 = 16;
const HEAD_H: i32 = 24;
const COL_H: i32 = 20;
const ROW_H: i32 = 34;
const BTN_H: i32 = 28;
const BTN_W: i32 = 88;
const NOTE_H: i32 = 40;

/// 내 기기 목록 위젯.
#[derive(Debug)]
pub struct DevicesWidget {
    bounds: Rect,
    scale: f32,
    view: DevView,
    revoke: Vec<Button>,
    close: Button,
    note: Option<(String, bool)>,
    action: Option<DevAction>,
}

impl Default for DevicesWidget {
    fn default() -> Self {
        Self::new(DevView::default())
    }
}

impl DevicesWidget {
    #[must_use]
    pub fn new(view: DevView) -> Self {
        let mut w = Self {
            bounds: Rect::default(),
            scale: 1.0,
            view: DevView::default(),
            revoke: Vec::new(),
            close: Button::new(t(Msg::DevBtnClose)),
            note: None,
            action: None,
        };
        let mut inv = Invalidations::default();
        w.set_view(view, &mut inv);
        w
    }

    /// 보기 교체 — 같은 내용이면 아무것도 하지 않는다(호스트가 틱마다 불러도 비용 0 · 재그리기 0).
    pub fn set_view(&mut self, view: DevView, inv: &mut Invalidations) {
        if view == self.view && self.revoke.len() == view.rows.len() {
            return;
        }
        self.revoke = view
            .rows
            .iter()
            .map(|r| {
                let mut b = Button::new(t(Msg::DevBtnRevoke));
                b.set_enabled(!r.is_me); // 자기 자신은 못 뺀다(그건 "사용자 키 교체"의 반대 — 다른 PC에서 한다)
                b.set_scale(self.scale);
                b
            })
            .collect();
        self.view = view;
        self.relayout(inv);
        inv.push(self.bounds);
    }

    #[must_use]
    pub fn view(&self) -> &DevView {
        &self.view
    }

    pub fn set_note(&mut self, text: impl Into<String>, warn: bool, inv: &mut Invalidations) {
        self.note = Some((text.into(), warn));
        inv.push(self.bounds);
    }

    pub fn clear_note(&mut self, inv: &mut Invalidations) {
        if self.note.take().is_some() {
            inv.push(self.bounds);
        }
    }

    pub fn take_action(&mut self) -> Option<DevAction> {
        self.action.take()
    }

    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        for b in &mut self.revoke {
            b.set_scale(self.scale);
        }
        self.close.set_scale(self.scale);
        self.relayout(inv);
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 내용 전체에 필요한 창 높이(px) — 행 수에 따라 호스트가 창을 맞춘다(라이선스 창 문법).
    #[must_use]
    pub fn desired_height(&self) -> i32 {
        let rows = i32::try_from(self.view.rows.len().max(1)).unwrap_or(1);
        self.s(PAD + HEAD_H + 8 + COL_H + 4 + rows * ROW_H + 10 + NOTE_H + 10 + BTN_H + PAD)
    }

    fn rows_top(&self) -> i32 {
        self.bounds.y + self.s(PAD + HEAD_H + 8 + COL_H + 4)
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(PAD);
        let (bw, bh, rh) = (self.s(BTN_W), self.s(BTN_H), self.s(ROW_H));
        let top = self.rows_top();
        for (i, btn) in self.revoke.iter_mut().enumerate() {
            let y = top + rh * i as i32 + (rh - bh) / 2;
            btn.set_bounds(Rect::new(b.right() - pad - bw, y, bw, bh), inv);
        }
        self.close.set_bounds(
            Rect::new(b.right() - pad - bw, b.bottom() - pad - bh, bw, bh),
            inv,
        );
    }

    fn take_clicks(&mut self) {
        for (i, b) in self.revoke.iter_mut().enumerate() {
            if b.take_clicked() {
                if let Some(r) = self.view.rows.get(i) {
                    self.action = Some(DevAction::Revoke(r.peer));
                }
            }
        }
        if self.close.take_clicked() {
            self.action = Some(DevAction::Close);
        }
    }
}

impl Widget for DevicesWidget {
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
                self.action = Some(DevAction::Close);
                return;
            }
            InputEvent::MouseDown { x, y, .. }
            | InputEvent::RightDown { x, y }
            | InputEvent::DoubleClick { x, y, .. } => {
                let p = Point { x, y };
                for b in self
                    .revoke
                    .iter_mut()
                    .chain(std::iter::once(&mut self.close))
                {
                    if b.bounds().contains(p) {
                        b.on_event(ev, inv);
                    }
                }
            }
            InputEvent::MouseUp { .. } | InputEvent::MouseMove { .. } => {
                for b in self
                    .revoke
                    .iter_mut()
                    .chain(std::iter::once(&mut self.close))
                {
                    b.on_event(ev, inv);
                }
            }
            _ => {}
        }
        self.take_clicks();
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let pad = self.s(PAD);
        // 제목 줄: 핸들 · ID
        ctx.select_font(FontSlot::Base, true);
        let title = if self.view.handle.is_empty() {
            t(Msg::DevTitle).to_string()
        } else {
            format!("{} · ID {}", self.view.handle, self.view.user_id)
        };
        ctx.text(b.x + pad, b.y + pad, b, &title, theme.text);
        // 열 머리
        ctx.select_font(FontSlot::Status, false);
        let col_y = b.y + self.s(PAD + HEAD_H + 8);
        let x_state = b.x + pad + (b.w - pad * 2) * 45 / 100;
        let x_seen = b.x + pad + (b.w - pad * 2) * 62 / 100;
        ctx.text(b.x + pad, col_y, b, t(Msg::DevColDevice), theme.text_dim);
        ctx.text(x_state, col_y, b, t(Msg::DevColState), theme.text_dim);
        ctx.text(x_seen, col_y, b, t(Msg::DevColSeen), theme.text_dim);
        // 행
        let top = self.rows_top();
        let rh = self.s(ROW_H);
        ctx.fill_rect(
            Rect::new(b.x + pad, top - self.s(2), b.w - pad * 2, 1),
            theme.border,
        );
        for (i, r) in self.view.rows.iter().enumerate() {
            let y = top + rh * i as i32 + (rh - ctx.text_height()) / 2;
            ctx.select_font(FontSlot::Base, false);
            let clip = Rect::new(
                b.x + pad,
                top + rh * i as i32,
                x_state - b.x - pad - self.s(8),
                rh,
            );
            ctx.text(b.x + pad, y, clip, &r.name, theme.text);
            let nw = ctx.text_width(&r.name);
            ctx.select_font(FontSlot::Status, false);
            ctx.text(
                b.x + pad + nw + self.s(8),
                y + self.s(1),
                clip,
                &r.id_short,
                theme.text_dim,
            );
            let (state, tone) = if r.is_me {
                (t(Msg::DevThisDevice).to_string(), theme.accent)
            } else if r.online {
                (t(Msg::DevOnline).to_string(), theme.ok)
            } else {
                (t(Msg::DevOffline).to_string(), theme.text_dim)
            };
            ctx.text(x_state, y + self.s(1), b, &state, tone);
            let seen_clip = Rect::new(
                x_seen,
                top + rh * i as i32,
                b.right() - pad - self.s(BTN_W) - self.s(8) - x_seen,
                rh,
            );
            ctx.text(
                x_seen,
                y + self.s(1),
                seen_clip,
                &r.last_seen,
                theme.text_dim,
            );
            ctx.fill_rect(
                Rect::new(b.x + pad, top + rh * (i as i32 + 1) - 1, b.w - pad * 2, 1),
                theme.border,
            );
        }
        for btn in &self.revoke {
            btn.paint(ctx, theme);
        }
        // 노트(무장 안내 · 결과) 또는 힌트
        ctx.select_font(FontSlot::Status, false);
        let rows_n = i32::try_from(self.view.rows.len().max(1)).unwrap_or(1);
        let ny = top + rh * rows_n + self.s(10);
        let (text, color) = match &self.note {
            Some((s, true)) => (s.as_str(), theme.danger),
            Some((s, false)) => (s.as_str(), theme.ok),
            None => (t(Msg::DevHint), theme.text_dim),
        };
        let lines =
            crate::settings::wrap_text(ctx, text, b.w - pad * 2 - self.s(BTN_W) - self.s(8), 2);
        for (k, line) in lines.iter().enumerate() {
            ctx.text(b.x + pad, ny + self.s(18) * k as i32, b, line, color);
        }
        self.close.paint(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u8) -> PeerId {
        PeerId::from_bytes([n; 32])
    }

    fn row(n: u8, me: bool) -> DevRow {
        DevRow {
            peer: pid(n),
            name: format!("dev{n}"),
            id_short: format!("{n:02x}"),
            is_me: me,
            online: me,
            last_seen: String::new(),
        }
    }

    #[test]
    fn revoke_button_disabled_for_self_and_height_grows_with_rows() {
        let one = DevicesWidget::new(DevView {
            handle: "h".into(),
            user_id: "u".into(),
            rows: vec![row(1, true)],
        });
        let two = DevicesWidget::new(DevView {
            handle: "h".into(),
            user_id: "u".into(),
            rows: vec![row(1, true), row(2, false)],
        });
        assert!(two.desired_height() > one.desired_height());
        assert!(!one.revoke[0].is_enabled());
        assert!(two.revoke[1].is_enabled());
    }

    #[test]
    fn same_view_is_noop() {
        let v = DevView {
            handle: "h".into(),
            user_id: "u".into(),
            rows: vec![row(1, true)],
        };
        let mut w = DevicesWidget::new(v.clone());
        let mut inv = Invalidations::default();
        w.set_view(v, &mut inv);
        assert!(inv.is_empty());
    }
}
