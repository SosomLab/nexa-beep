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
use nbeep_core::avatar::AvatarChoice;
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
    /// 해석된 표시 이름(이니셜 아바타 원료 — "auto"의 실제 값은 호스트만 안다).
    pub resolved_name: String,
    /// 아바타 색 시드(내 키 지문).
    pub seed: Vec<u8>,
    /// 내 사진(M4-5 — 호스트가 imgdec로 디코드·원형 마스크해 넘긴다). 없으면 이니셜.
    pub avatar: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 아바타 선택 원문(`profile.avatar` — [`AvatarChoice`] 직렬). 사진(`image_path`)이
    /// 있으면 사진이 우선한다(선택 UI에서 스와치를 고르면 사진을 비운다 — 상호 배타).
    pub avatar_choice: String,
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
    resolved_name: String,
    seed: Vec<u8>,
    avatar: Option<std::rc::Rc<crate::theme::IconImage>>,
    /// 아바타 선택 원문(스와치 클릭으로 바뀐다 · 사진이 있으면 사진 우선).
    avatar_choice: String,
    /// 내장 12간지(키, 그림) — 스와치·프리뷰 공용(new에서 1회 해석).
    builtins: Vec<(String, std::rc::Rc<crate::theme::IconImage>)>,
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
            resolved_name: v.resolved_name.clone(),
            seed: v.seed.clone(),
            avatar: v.avatar.clone(),
            avatar_choice: v.avatar_choice.clone(),
            builtins: crate::avatar_assets::builtins()
                .into_iter()
                .map(|b| (b.key, std::rc::Rc::new(b.image)))
                .collect(),
            changes: Vec::new(),
            pick_image: false,
            closed: false,
        }
    }

    /// 스와치 목록 — [이니셜, 없음] + 내장 12간지(설정 저장값과 1:1).
    fn swatch_values(&self) -> Vec<String> {
        let mut v = vec!["initials".to_string(), "none".to_string()];
        v.extend(self.builtins.iter().map(|(k, _)| format!("b:{k}")));
        v
    }

    /// i번째 스와치 rect(한 줄 가로 배열 — 14개가 폭 안에 들어온다: 14×28-4 = 388 ≤ 408).
    fn swatch_rect(&self, i: usize) -> Rect {
        let d = self.s(24);
        let gap = self.s(4);
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let x = self.bounds.x + self.s(16) + (d + gap) * i as i32;
        Rect::new(x, self.bounds.y + self.s(172), d, d)
    }

    /// IME 조합 중 문자열(08-13) — 초점 필드만 받는다(TextBox가 스스로 거른다 ·
    /// 빈 문자열은 소거라 전 필드에 전달).
    pub fn set_preedit(&mut self, text: &str, inv: &mut Invalidations) {
        self.name.set_preedit(text, inv);
        self.email.set_preedit(text, inv);
        self.phone.set_preedit(text, inv);
    }

    /// 우클릭 편집 메뉴 행동(1회성 — 08-13 전수 검사).
    pub fn take_edit_ctx(&mut self) -> Option<crate::controls::EditCtxAction> {
        self.name
            .take_edit_ctx()
            .or_else(|| self.email.take_edit_ctx())
            .or_else(|| self.phone.take_edit_ctx())
    }

    /// 클립보드 텍스트 유무 주입(우클릭 시점 — 붙여넣기 항목 활성 근거).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.name.set_clipboard_has_text(yes);
        self.email.set_clipboard_has_text(yes);
        self.phone.set_clipboard_has_text(yes);
    }

    /// 사진 미리보기 교체(호스트 — 이미지 선택 직후 imgdec 결과 반영).
    pub fn set_avatar(
        &mut self,
        img: Option<std::rc::Rc<crate::theme::IconImage>>,
        inv: &mut Invalidations,
    ) {
        self.avatar = img;
        inv.push(self.bounds);
    }

    /// 바뀐 (설정 키, 값) 회수(1회성) — 호스트가 설정 적용 깔때기에 넘긴다.
    pub fn take_changes(&mut self) -> Vec<(&'static str, String)> {
        std::mem::take(&mut self.changes)
    }

    /// 이미지 선택 요청(1회성 · 선택… 버튼) — 호스트가 피커를 연다.
    /// 선택 복사(① 08-13) — 포커스된 입력 필드에서만 나온다(비포커스는 None).
    #[must_use]
    pub fn clipboard_copy(&self) -> Option<String> {
        self.name
            .copy_selection()
            .or_else(|| self.email.copy_selection())
            .or_else(|| self.phone.copy_selection())
    }

    /// 선택 잘라내기(①) — 포커스된 필드에서만.
    pub fn clipboard_cut(&mut self, inv: &mut Invalidations) -> Option<String> {
        self.name
            .cut_selection(inv)
            .or_else(|| self.email.cut_selection(inv))
            .or_else(|| self.phone.cut_selection(inv))
    }

    /// 붙여넣기(①) — 포커스된 필드만 받는다(TextBox가 비포커스면 무시).
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        self.name.paste(text, inv);
        self.email.paste(text, inv);
        self.phone.paste(text, inv);
    }

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
        // (y=172 = 아바타 스와치 행 — swatch_rect가 계산 · 아래 행들은 +40 시프트)
        // 이미지 행(라벨은 paint) — 버튼 우측.
        self.choose_img.set_bounds(
            Rect::new(b.right() - pad - bw, b.y + self.s(214), bw, field_h),
            inv,
        );
        // 입력 3종 — 라벨 아래 필드.
        let fw = b.w - pad * 2;
        self.name
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(274), fw, field_h), inv);
        self.email
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(332), fw, field_h), inv);
        self.phone
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(390), fw, field_h), inv);
        // 공개 토글 3종 — 라벨 왼쪽·토글 오른쪽 끝.
        let sw_h = self.s(26);
        self.sw_basic
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(436), fw, sw_h), inv);
        self.sw_email
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(468), fw, sw_h), inv);
        self.sw_phone
            .set_bounds(Rect::new(b.x + pad, b.y + self.s(500), fw, sw_h), inv);
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
            // 우클릭 메뉴가 열려 있으면 Esc는 메뉴 몫(메뉴만 닫는다) — 창 닫기가
            // 가로채면 메뉴를 키보드로 못 닫는다(08-13 실기).
            let popup =
                self.name.popup_open() || self.email.popup_open() || self.phone.popup_open();
            if !popup {
                self.closed = true;
                return;
            }
        }
        if let InputEvent::MouseDown { x, y, .. } = *ev {
            let p = Point { x, y };
            // 아바타 스와치(08-14 — 이니셜·없음·12간지) — 클릭 = 즉시 선택 보고.
            // 사진과는 상호 배타: 스와치를 고르면 image_path를 비운다(선택이 보여야 한다).
            for (i, val) in self.swatch_values().into_iter().enumerate() {
                if self.swatch_rect(i).contains(p) {
                    self.avatar_choice = val.clone();
                    self.avatar = None; // 사진 미리보기 해제(호스트도 image_path "" 반영)
                    if !self.image_path.is_empty() {
                        self.image_path.clear();
                        self.changes.push(("profile.image_path", String::new()));
                    }
                    self.changes.push(("profile.avatar", val));
                    inv.push(self.bounds);
                    return;
                }
            }
            // 포커스는 **배타** — 클릭된 컨트롤만 갖는다(08-14 실기: Choose… 버튼이
            // 묵은 포커스를 쥔 채라 텍스트박스 Enter가 **버튼 클릭으로도** 처리되고
            // 포커스 링도 남았다. 브로드캐스트 전달 구조에선 포커스 단일성이 이중
            // 처리를 막는 유일한 문이다 — 갤러리는 처음부터 전 컨트롤 배타였다).
            self.name.set_focused(self.name.bounds().contains(p));
            self.email.set_focused(self.email.bounds().contains(p));
            self.phone.set_focused(self.phone.bounds().contains(p));
            self.choose_img.set_focused(self.choose_img.bounds().contains(p));
            self.sw_basic.set_focused(self.sw_basic.bounds().contains(p));
            self.sw_email.set_focused(self.sw_email.bounds().contains(p));
            self.sw_phone.set_focused(self.sw_phone.bounds().contains(p));
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
        // 큰 원형 아바타(목록 40의 3배 = 120 · 사용자 요청 08-11) — 이니셜 가상 이미지.
        // 사진을 골라도 픽셀 렌더는 M4-5(imgdec) 후 — 그때까지 이니셜 + 파일명 표기.
        let d = self.s(120);
        let av = Rect::new(b.x + (b.w - d) / 2, b.y + self.s(40), d, d);
        // 프리뷰 우선순위(08-14): 사진 > 내장 12간지 > 없음(빈 원) > 이니셜.
        let choice = AvatarChoice::parse(&self.avatar_choice);
        let builtin_img = match &choice {
            AvatarChoice::Builtin(k) => self
                .builtins
                .iter()
                .find(|(bk, _)| bk == k)
                .map(|(_, i)| i.clone()),
            _ => None,
        };
        if let Some(img) = &self.avatar {
            // 선택된 사진(M4-5 imgdec — 원형 마스크 완료본) — 동그랗고 큰 미리보기.
            ctx.image_scaled(av, img, b);
        } else if let Some(img) = builtin_img {
            crate::avatar::draw_builtin(ctx, av, &img, &self.seed);
        } else if matches!(choice, AvatarChoice::None) {
            crate::avatar::draw_avatar(ctx, av, "", &self.seed, 34.0); // 빈 원
        } else {
            let ini_src = {
                // 조합 중 글자까지 반영(display_text) — 필드와 아바타가 같은 것을
                // 보여야 "입력이 안 됐다"로 오독하지 않는다(08-13 실기).
                let typed = self.name.display_text();
                if typed.trim().is_empty() {
                    self.resolved_name.clone()
                } else {
                    typed
                }
            };
            crate::avatar::draw_avatar(ctx, av, &ini_src, &self.seed, 34.0);
        }
        // ── 아바타 스와치 행(08-14) — [이니셜][없음][12간지 …] · 현재 선택은 링 ──
        let cur = if self.avatar.is_some() {
            None // 사진 사용 중 — 스와치 어느 것도 "현재"가 아니다
        } else {
            Some(choice.to_setting())
        };
        for (i, val) in self.swatch_values().into_iter().enumerate() {
            let r = self.swatch_rect(i);
            match val.as_str() {
                "initials" => {
                    let ini_src = {
                        let typed = self.name.display_text();
                        if typed.trim().is_empty() {
                            self.resolved_name.clone()
                        } else {
                            typed
                        }
                    };
                    crate::avatar::draw_avatar(ctx, r, &ini_src, &self.seed, 0.0);
                }
                "none" => crate::avatar::draw_avatar(ctx, r, "", &self.seed, 0.0),
                v => {
                    if let Some((_, img)) =
                        self.builtins.iter().find(|(k, _)| format!("b:{k}") == v)
                    {
                        crate::avatar::draw_builtin(ctx, r, img, &self.seed);
                    }
                }
            }
            if cur.as_deref() == Some(val.as_str()) {
                // 선택 링 — 위치 신호(색만으로 가르지 않는다 규약과 정합).
                let ring = Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4);
                ctx.stroke_round_rect(ring, ring.w / 2, theme.accent, 2.0);
            }
        }
        // 이미지 행 — 라벨 + 파일명(버튼 왼쪽까지 클립).
        ctx.select_font(FontSlot::Base, false);
        ctx.text(
            b.x + pad,
            b.y + self.s(218),
            b,
            t(Msg::ProfileImage),
            theme.text,
        );
        ctx.select_font(FontSlot::Status, false);
        let clip = Rect::new(
            b.x + pad,
            b.y + self.s(214),
            (self.choose_img.bounds().x - self.s(8) - (b.x + pad + self.s(110))).max(0),
            self.s(28),
        );
        ctx.text(
            b.x + pad + self.s(110),
            b.y + self.s(220),
            clip,
            &self.image_label(),
            theme.text_dim,
        );
        // 필드 라벨.
        let labels = [
            (Msg::DisplayNameLabel, self.s(256)),
            (Msg::FieldEmail, self.s(314)),
            (Msg::FieldPhone, self.s(372)),
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
            let dy = self.s(538) + i as i32 * self.s(16);
            ctx.text(b.x + pad, b.y + dy, b, line, theme.text_dim);
        }
        // 우클릭 편집 메뉴 — 모든 자식 뒤에 재도색(08-13 실기: Email 필드가
        // Display name 메뉴의 가운데를 덮었다).
        self.name.paint_popup(ctx, theme);
        self.email.paint_popup(ctx, theme);
        self.phone.paint_popup(ctx, theme);
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

    /// 08-14 실기 — Choose… 클릭 후 이름 필드를 편집하고 Enter를 치면 **버튼이
    /// 묵은 포커스로 또 클릭**됐다(피커 재오픈). 포커스는 배타여야 한다.
    #[test]
    fn enter_in_textbox_does_not_click_stale_focused_button() {
        let (mut w, mut inv) = widget();
        // Choose… 클릭 — 버튼이 포커스를 얻고 피커 요청 1회.
        let r = w.choose_img.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: r.x + 2,
                y: r.y + 2,
                shift: false,
                primary: true,
            },
            &mut inv,
        );
        w.on_event(&InputEvent::MouseUp { x: r.x + 2, y: r.y + 2 }, &mut inv);
        assert!(w.take_pick_image(), "버튼 클릭 = 피커 요청");
        // 이름 필드 클릭 — 버튼 포커스는 **해제**되어야 한다(배타).
        let n = w.name.bounds();
        w.on_event(
            &InputEvent::MouseDown {
                x: n.x + 2,
                y: n.y + 2,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(
            !w.choose_img.is_focused(),
            "텍스트박스 클릭 후 버튼 포커스 잔존(포커스 링·Enter 이중 처리의 원인)"
        );
        // Enter = 이름 확정만 — 버튼 클릭(피커 재요청)이 되면 안 된다.
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(!w.take_pick_image(), "Enter가 묵은 포커스 버튼을 누르면 안 된다");
        assert!(
            w.take_changes()
                .iter()
                .any(|(k, _)| *k == "profile.display_name"),
            "Enter = 표시 이름 확정 보고"
        );
    }

    /// 아바타 스와치(08-14) — 클릭 = 선택 보고 · **사진과 상호 배타**(image_path 소거).
    #[test]
    fn swatch_click_reports_choice_and_clears_photo() {
        let mut w = ProfileWidget::new(&ProfileValues {
            display_name: "auto".into(),
            image_path: "C:/pics/me.png".into(),
            avatar_choice: "initials".into(),
            ..ProfileValues::default()
        });
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 440, 610), &mut inv);
        let r = w.swatch_rect(2); // [이니셜][없음] 다음 = 첫 12간지(rat)
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
        assert!(
            ch.contains(&("profile.avatar", "b:rat".to_string())),
            "스와치 클릭 = 선택 보고: {ch:?}"
        );
        assert!(
            ch.contains(&("profile.image_path", String::new())),
            "사진과 배타 — image_path 소거: {ch:?}"
        );
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
