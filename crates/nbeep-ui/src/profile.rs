//! **프로필 변경 화면**(M3-17 · 사용자 요청 08-11) — 프로필 이미지·표시 이름·이메일·
//! 전화번호 편집 + 항목별 공개 토글(DR-22 옵트인 · ADR-0008).
//!
//! 위젯은 저장·전송을 모른다 — 바뀐 (키, 값)만 [`ProfileWidget::take_changes`]로 내놓고,
//! 영속(SettingsState/nexa-conf)·재공지·프로필 교환은 호스트 몫이다. 이미지 선택도
//! [`ProfileWidget::take_pick_image`] 요청만 올리고 실제 파일 선택은 호스트 피커가 한다.

use crate::controls::{Button, Control as _, LabelSide, Switch, TextBox};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::Theme;
use crate::widget::{Invalidations, Widget};
use nbeep_core::i18n::{t, Msg};

/// 프로필 화면 초기값(호스트가 설정에서 읽어 넘긴다).
#[derive(Debug, Default, Clone)]
pub struct ProfileValues {
    /// 표시 이름("auto" = 정제된 호스트명 자동).
    pub display_name: String,
    /// 이메일(빈 값 = 미설정).
    pub email: String,
    /// 전화번호(빈 값 = 미설정).
    pub phone: String,
    /// 프로필 이미지 경로(빈 값 = 없음).
    pub image_path: String,
    /// 공개 여부 — 기본정보(사진·이름)/이메일/전화.
    pub share_basic: bool,
    /// 이메일 공개.
    pub share_email: bool,
    /// 전화번호 공개.
    pub share_phone: bool,
}

/// 프로필 변경 화면 위젯.
#[derive(Debug)]
pub struct ProfileWidget {
    bounds: Rect,
    scale: f32,
    name: TextBox,
    email: TextBox,
    phone: TextBox,
    choose_img: Button,
    sw_basic: Switch,
    sw_email: Switch,
    sw_phone: Switch,
    image_path: String,
    changes: Vec<(&'static str, String)>,
    pick_image: bool,
    closed: bool,
}

impl ProfileWidget {
    /// 현재 값으로 화면을 만든다.
    #[must_use]
    pub fn new(v: &ProfileValues) -> Self {
        let mut name = TextBox::new(t(Msg::NameAuto));
        if v.display_name != "auto" {
            name = TextBox::new(t(Msg::NameAuto)).with_text(&v.display_name);
        }
        name.set_focused(true);
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            name,
            email: TextBox::new("name@example.com").with_text(&v.email),
            phone: TextBox::new("010-0000-0000").with_text(&v.phone),
            choose_img: Button::new(t(Msg::ActChoose)),
            sw_basic: Switch::new(t(Msg::ShareBasic), v.share_basic)
                .with_label_side(LabelSide::Left),
            sw_email: Switch::new(t(Msg::ShareEmail), v.share_email)
                .with_label_side(LabelSide::Left),
            sw_phone: Switch::new(t(Msg::SharePhone), v.share_phone)
                .with_label_side(LabelSide::Left),
            image_path: v.image_path.clone(),
            changes: Vec::new(),
            pick_image: false,
            closed: false,
        }
    }

    /// 바뀐 (설정 키, 값) 회수(1회성) — 호스트가 설정 적용 깔때기에 넘긴다.
    pub fn take_changes(&mut self) -> Vec<(&'static str, String)> {
        std::mem::take(&mut self.changes)
    }

    /// 이미지 선택 요청(1회성 · 선택… 버튼) — 호스트가 피커를 연다.
    pub fn take_pick_image(&mut self) -> bool {
        std::mem::take(&mut self.pick_image)
    }

    /// 닫기 요청(1회성 · Esc).
    pub fn take_closed(&mut self) -> bool {
        std::mem::take(&mut self.closed)
    }

    /// 피커가 고른 이미지 경로 반영(호스트 호출) — 변경으로도 보고한다.
    pub fn set_image_path(&mut self, path: &str, inv: &mut Invalidations) {
        self.image_path = path.to_string();
        self.changes.push(("profile.image_path", path.to_string()));
        inv.push(self.bounds);
    }

    /// 배율 지정 — 내부 컨트롤 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        self.name.set_scale(self.scale);
        self.email.set_scale(self.scale);
        self.phone.set_scale(self.scale);
        self.choose_img.set_scale(self.scale);
        self.sw_basic.set_scale(self.scale);
        self.sw_email.set_scale(self.scale);
        self.sw_phone.set_scale(self.scale);
        self.relayout(inv);
    }

    fn s(&self, v: i32) -> i32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let r = (v as f32 * self.scale).round() as i32;
        r
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(16);
        let field_h = self.s(28);
        let bw = self.s(80);
        // 이미지 행(라벨은 paint) — 버튼 우측.
        self.choose_img.set_bounds(
            Rect::new(b.right() - pad - bw, b.y + self.s(44), bw, field_h),
            inv,
        );
        // 입력 3종 — 라벨 아래 필드.
        let fw = b.w - pad * 2;
        self.name
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(104), fw, field_h), inv);
        self.email
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(162), fw, field_h), inv);
        self.phone
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(220), fw, field_h), inv);
        // 공개 토글 3종 — 라벨 왼쪽·토글 오른쪽 끝.
        let sw_h = self.s(26);
        self.sw_basic
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(266), fw, sw_h), inv);
        self.sw_email
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(298), fw, sw_h), inv);
        self.sw_phone
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(330), fw, sw_h), inv);
    }

    /// 표시용 이미지 파일명(경로 말고 이름만 — 좁은 창).
    fn image_label(&self) -> String {
        if self.image_path.is_empty() {
            "(없음)".to_string()
        } else {
            std::path::Path::new(&self.image_path)
                .file_name()
                .map_or_else(
                    || self.image_path.clone(),
                    |n| n.to_string_lossy().into_owned(),
                )
        }
    }
}

impl Widget for ProfileWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        self.relayout(inv);
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        if matches!(
            *ev,
            InputEvent::Key {
                key: Key::Escape,
                ..
            }
        ) {
            self.closed = true;
            return;
        }
        if let InputEvent::MouseDown { x, y, .. } = *ev {
            let p = Point { x, y };
            self.name.set_focused(self.name.bounds().contains(p));
            self.email.set_focused(self.email.bounds().contains(p));
            self.phone.set_focused(self.phone.bounds().contains(p));
        }
        self.choose_img.on_event(ev, inv);
        if self.choose_img.take_clicked() {
            self.pick_image = true;
            return;
        }
        for (sw, key) in [
            (&mut self.sw_basic, "profile.share.basic"),
            (&mut self.sw_email, "profile.share.email"),
            (&mut self.sw_phone, "profile.share.phone"),
        ] {
            sw.on_event(ev, inv);
            if let Some(on) = sw.take_toggled() {
                self.changes
                    .push((key, if on { "on" } else { "off" }.to_string()));
            }
        }
        // 텍스트 필드 — Enter 확정 시에만 보고(설정 Face와 같은 규약).
        for (tbx, key, auto_when_empty) in [
            (&mut self.name, "profile.display_name", true),
            (&mut self.email, "profile.email", false),
            (&mut self.phone, "profile.phone", false),
        ] {
            tbx.on_event(ev, inv);
            if let Some(v) = tbx.take_committed() {
                let v = v.trim().to_string();
                let out = if v.is_empty() && auto_when_empty {
                    "auto".to_string()
                } else {
                    v
                };
                self.changes.push((key, out));
            }
            let _ = tbx.take_changed();
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        let pad = self.s(16);
        ctx.select_font(FontSlot::Base, true);
        ctx.text(
            b.x + pad,
            b.y + self.s(14),
            b,
            t(Msg::ProfileTitle),
            theme.text,
        );
        // 이미지 행 — 라벨 + 파일명(버튼 왼쪽까지 클립).
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            b.x + pad,
            b.y + self.s(48),
            b,
            t(Msg::ProfileImage),
            theme.text,
        );
        ctx.select_font(FontSlot::Status, false);
        let clip = Rect::new(
            b.x + pad,
            b.y + self.s(44),
            (self.choose_img.bounds().x - self.s(8) - (b.x + pad + self.s(110))).max(0),
            self.s(28),
        );
        ctx.text(
            b.x + pad + self.s(110),
            b.y + self.s(50),
            clip,
            &self.image_label(),
            theme.text_dim,
        );
        // 필드 라벨.
        let labels = [
            (Msg::DisplayNameLabel, self.s(86)),
            (Msg::FieldEmail, self.s(144)),
            (Msg::FieldPhone, self.s(202)),
        ];
        for (m, dy) in labels {
            ctx.select_font(FontSlot::Status, false);
            ctx.text(b.x + pad, b.y + dy, b, t(m), theme.text_dim);
        }
        self.name.paint(ctx, theme);
        self.email.paint(ctx, theme);
        self.phone.paint(ctx, theme);
        self.choose_img.paint(ctx, theme);
        self.sw_basic.paint(ctx, theme);
        self.sw_email.paint(ctx, theme);
        self.sw_phone.paint(ctx, theme);
        // 안내 — 브로드캐스트 미포함·연결 상대 한정(워드랩 2줄).
        ctx.select_font(FontSlot::Status, false);
        let avail = b.w - pad * 2;
        let lines = crate::settings::wrap_text(ctx, t(Msg::ProfileShareNote), avail, 2);
        for (i, line) in lines.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let dy = self.s(368) + i as i32 * self.s(16);
            ctx.text(b.x + pad, b.y + dy, b, line, theme.text_dim);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn widget() -> (ProfileWidget, Invalidations) {
        let mut w = ProfileWidget::new(&ProfileValues {
            display_name: "auto".into(),
            ..ProfileValues::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 440, 430), &mut inv);
        (w, inv)
    }

    /// 토글 클릭 → (키, on/off) 변경 보고.
    #[test]
    fn share_toggle_reports_change() {
        let (mut w, mut inv) = widget();
        let r = w.sw_basic.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        let ch = w.take_changes();
        assert_eq!(ch, vec![("profile.share.basic", "on".to_string())]);
        assert!(w.take_changes().is_empty(), "1회성");
    }

    /// 표시 이름 Enter 확정 — 빈 값은 "auto"로 정규화.
    #[test]
    fn empty_name_commits_auto() {
        let (mut w, mut inv) = widget();
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        let ch = w.take_changes();
        assert_eq!(ch, vec![("profile.display_name", "auto".to_string())]);
    }

    /// 이미지 선택 요청 · 경로 반영이 변경으로 보고된다.
    #[test]
    fn image_pick_roundtrip() {
        let (mut w, mut inv) = widget();
        let r = w.choose_img.bounds();
        let (cx, cy) = (r.x + 2, r.y + 2);
        w.on_event(
            &InputEvent::MouseDown {
                x: cx,
                y: cy,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        w.on_event(&InputEvent::MouseUp { x: cx, y: cy }, &mut inv);
        assert!(w.take_pick_image(), "선택 요청");
        w.set_image_path("C:/pics/me.png", &mut inv);
        assert_eq!(
            w.take_changes(),
            vec![("profile.image_path", "C:/pics/me.png".to_string())]
        );
        assert_eq!(w.image_label(), "me.png");
    }
}
