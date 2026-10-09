//! **라이선스 화면**(docs/50 P4 · D-33-7 · 도움말 ▸ 라이선스…) — 출처: nexa-sql `crates/nexa-sql/src/license_win.rs`.
//!
//! 위 = 상태 띠 + 표(파일 · ID · 사용자 · 종류/등급 · 기한 · 빌드일 · 기기 코드) · 가운데 = 요청 코드(이름 · 이메일 ·
//! [요청 코드 복사] · 보낼 이메일은 링크 = 클릭하면 복사) · 아래 = [라이선스 파일 열기…] \[제거\] \[닫기\] + 결과 한 줄.
//!
//! beep 관례(nbeep-ui)대로 **winit 없는 위젯**이다(sql 판은 창까지 소유) — 창 생성·`Role`·클립보드·파일 선택은 호스트 몫이고,
//! 위젯은 [`LicenseWidget::take_action`] 1회성 행동만 낸다(`AddrPromptWidget`과 같은 꼴). 판정·설치의 단일 원천은 호스트의
//! [`nbeep_license::Licensing`] — 위젯은 보기([`LicView`])만 받는다. 보기는 [`license_view`]가 만든다(호스트가 열 때·설치·제거 뒤 호출).
//!
//! 버튼 이름은 발급 메일 안내와 **글자까지 같아야** 한다(nexa-license `presets.rs` nexa-beep 분기 — 10-09 협업 합의):
//! 도움말 ▸ 라이선스… · [라이선스 파일 열기…] · [요청 코드 복사] · 상태 줄 licensed.

use std::cell::Cell;

use crate::controls::{Button, Control as _, TextBox};
use crate::draw::{ellipsize_middle, DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::{Color, Theme};
use crate::widget::{Invalidations, Widget};
use nbeep_core::{t, tf, Msg};
use nbeep_license::{Invalid, LicenseState, Licensing, RequestMeta, PRODUCT};

/// 상태 띠의 톤.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LicTone {
    /// 무료(파일 없음) — 정상 상태라 경고가 아니다(D-33-6 · 비상업 = 무료).
    #[default]
    Neutral,
    /// 정식.
    Ok,
    /// 무효 · 기한 지남 · 만료.
    Warn,
}

/// 호스트가 그릴 때 넘기는 보기(라이선스 상태의 단일 원천은 `Licensing`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LicView {
    /// 상태 한 줄.
    pub state: String,
    /// 상태 띠 톤.
    pub tone: LicTone,
    /// (라벨, 값) 표.
    pub rows: Vec<(String, String)>,
    /// 요청 코드 미리보기(기기 ID 없으면 `None` = [요청 코드 복사] 끔).
    pub request: Option<String>,
    /// 요청 코드를 보내는 이메일(링크 · 클릭 = 복사).
    pub contact: String,
}

/// 위젯이 내는 1회성 행동(호스트가 처리한다).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LicAction {
    /// 요청 코드를 만들어 클립보드로(이름 · 이메일 = 메타 · trim 끝).
    CopyRequest {
        /// 이름(빈 값 = 메타 생략).
        name: String,
        /// 이메일(빈 값 = 메타 생략).
        email: String,
    },
    /// 보낼 이메일 주소를 클립보드로(링크 클릭).
    CopyContact(String),
    /// 라이선스 파일을 고른다(파일 선택 = 호스트 · P3 nexa-dlg).
    OpenFile,
    /// 사용자 폴더의 라이선스 제거.
    Remove,
    /// 창 닫기(Esc · \[닫기\]).
    Close,
}

/// 무효 사유 → 짧은 문구.
#[must_use]
pub fn invalid_reason(i: Invalid) -> &'static str {
    match i {
        Invalid::Signature => t(Msg::LicWhySignature),
        Invalid::Product => t(Msg::LicWhyProduct),
        Invalid::Machine => t(Msg::LicWhyMachine),
        Invalid::NoRootKey => t(Msg::LicWhyNoRoot),
        Invalid::Format | Invalid::DuplicateKeys | Invalid::Malformed => t(Msg::LicWhyFormat),
    }
}

/// 상태 → (한 줄, 톤). 설치 거부 안내([`Msg::LicNoteRejected`])의 사유로도 쓴다.
#[must_use]
pub fn state_text(s: &LicenseState) -> (String, LicTone) {
    match s {
        LicenseState::Free => (t(Msg::LicStateFree).to_string(), LicTone::Neutral),
        LicenseState::Licensed(l) => (tf(Msg::LicStateLicensed, &[&l.tier]), LicTone::Ok),
        LicenseState::Invalid(i) => (
            tf(Msg::LicStateInvalid, &[invalid_reason(*i)]),
            LicTone::Warn,
        ),
        LicenseState::Outdated(l) => (
            tf(Msg::LicStateOutdated, &[&l.updates_until]),
            LicTone::Warn,
        ),
        LicenseState::Expired(l) => (tf(Msg::LicStateExpired, &[&l.expires]), LicTone::Warn),
    }
}

/// `Licensing` → 보기(호스트가 창을 열 때 · 설치·제거·`refresh` 변화 뒤에 부른다). 기기 ID 조회가 있어 자주 도는 길(그리기)에서는 부르지 않는다.
#[must_use]
pub fn license_view(l: &Licensing) -> LicView {
    let (state, tone) = state_text(l.state());
    let dash = |s: &str| {
        if s.is_empty() {
            "-".to_string()
        } else {
            s.to_string()
        }
    };
    let mut rows = vec![(
        t(Msg::LicRowFile).to_string(),
        l.path()
            .map_or_else(|| "-".to_string(), |p| p.display().to_string()),
    )];
    if let Some(lic) = l.state().license() {
        rows.push((t(Msg::LicRowId).to_string(), lic.id.clone()));
        rows.push((t(Msg::LicRowLicensee).to_string(), dash(&lic.licensee)));
        rows.push((
            t(Msg::LicRowTier).to_string(),
            format!("{} / {}", lic.kind, dash(&lic.tier)),
        ));
        rows.push((
            t(Msg::LicRowTerm).to_string(),
            if lic.expires.is_empty() {
                t(Msg::LicTermForever).to_string()
            } else {
                lic.expires.clone()
            },
        ));
        if !lic.updates_until.is_empty() {
            rows.push((t(Msg::LicRowUpdates).to_string(), lic.updates_until.clone()));
        }
    }
    rows.push((
        t(Msg::LicRowBuild).to_string(),
        format!("{} · v{}", PRODUCT.build_date, PRODUCT.version),
    ));
    rows.push((
        t(Msg::LicRowMachine).to_string(),
        Licensing::machine_code().unwrap_or_else(|| "-".to_string()),
    ));
    LicView {
        state,
        tone,
        rows,
        request: Licensing::request_code(&RequestMeta::default()),
        contact: nbeep_license::LICENSE_CONTACT.to_string(),
    }
}

// ── 치수(논리 px · 배율을 곱한다) ──
const PAD: i32 = 16;
const BANNER_H: i32 = 30;
const ROW_H: i32 = 22;
const FIELD_H: i32 = 28;
const BTN_H: i32 = 30;
const GAP: i32 = 8;
const NAME_W: i32 = 150;
const EMAIL_W: i32 = 210;

/// 라벨 폭 추정(논리 px) — 레이아웃은 그리기 문맥 없이 정해지므로 글자 수로 잰다(ASCII 8 · 그 밖 15 + 여백 28 · 최소 88).
fn label_w(s: &str) -> i32 {
    let w: i32 = s.chars().map(|c| if c.is_ascii() { 8 } else { 15 }).sum();
    (w + 28).max(88)
}

/// 라이선스 화면 위젯.
#[derive(Debug)]
pub struct LicenseWidget {
    bounds: Rect,
    scale: f32,
    view: LicView,
    name: TextBox,
    email: TextBox,
    btn_copy: Button,
    btn_open: Button,
    btn_remove: Button,
    btn_close: Button,
    /// 결과 한 줄(설치됨 · 거부 · 복사됨) · 경고 여부.
    note: Option<(String, bool)>,
    action: Option<LicAction>,
    /// 이메일 링크 자리(마지막 페인트 — 글 폭은 그릴 때 안다) · hover.
    link: Cell<Rect>,
    link_hover: bool,
}

impl Default for LicenseWidget {
    fn default() -> Self {
        Self::new(LicView::default())
    }
}

impl LicenseWidget {
    /// 보기로 만든다(이름 입력에 포커스).
    #[must_use]
    pub fn new(view: LicView) -> Self {
        let mut name = TextBox::new(t(Msg::LicReqName));
        name.set_focused(true);
        let mut w = Self {
            bounds: Rect::default(),
            scale: 1.0,
            view,
            name,
            email: TextBox::new(t(Msg::LicReqEmail)),
            btn_copy: Button::new(t(Msg::LicBtnCopyReq)),
            btn_open: Button::new(t(Msg::LicBtnOpen)),
            btn_remove: Button::new(t(Msg::LicBtnRemove)),
            btn_close: Button::new(t(Msg::LicBtnClose)),
            note: None,
            action: None,
            link: Cell::new(Rect::default()),
            link_hover: false,
        };
        let has = w.view.request.is_some();
        w.btn_copy.set_enabled(has);
        w
    }

    /// 보기 교체(설치·제거·상태 변화 뒤). 높이가 바뀔 수 있다 — 호스트는 [`Self::desired_height`]로 창을 맞춘다.
    pub fn set_view(&mut self, view: LicView, inv: &mut Invalidations) {
        self.btn_copy.set_enabled(view.request.is_some());
        self.view = view;
        self.relayout(inv);
        inv.push(self.bounds);
    }

    /// 지금 보기(시험·덤프).
    #[must_use]
    pub fn view(&self) -> &LicView {
        &self.view
    }

    /// 결과 한 줄(호스트: 설치·거부·복사·제거 결과).
    pub fn set_note(&mut self, text: impl Into<String>, warn: bool, inv: &mut Invalidations) {
        self.note = Some((text.into(), warn));
        inv.push(self.bounds);
    }

    /// 결과 한 줄(시험).
    #[must_use]
    pub fn note(&self) -> Option<&(String, bool)> {
        self.note.as_ref()
    }

    /// 1회성 행동.
    pub fn take_action(&mut self) -> Option<LicAction> {
        self.action.take()
    }

    /// 배율 지정 — 내부 컨트롤 전파.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        for tb in [&mut self.name, &mut self.email] {
            tb.set_scale(self.scale);
        }
        for b in [
            &mut self.btn_copy,
            &mut self.btn_open,
            &mut self.btn_remove,
            &mut self.btn_close,
        ] {
            b.set_scale(self.scale);
        }
        self.relayout(inv);
    }

    /// 내용이 다 들어가는 높이(물리 px · 행 수가 정한다) — 호스트가 창을 열 때·[`Self::set_view`] 뒤에 쓴다.
    #[must_use]
    pub fn desired_height(&self) -> i32 {
        let rows = i32::try_from(self.view.rows.len()).unwrap_or(0);
        let logical = PAD
            + BANNER_H
            + 10
            + rows * ROW_H
            + 18
            + ROW_H
            + 6
            + FIELD_H
            + 8
            + ROW_H
            + ROW_H
            + 12
            + BTN_H
            + PAD;
        self.s(logical)
    }

    // ── 포커스된 입력 상자 위임(호스트 = IME·클립보드 · AddrPromptWidget과 같은 꼴) ──

    fn focused_box(&mut self) -> Option<&mut TextBox> {
        if self.name.is_focused() {
            Some(&mut self.name)
        } else if self.email.is_focused() {
            Some(&mut self.email)
        } else {
            None
        }
    }

    /// IME 조합 중 문자열(포커스된 상자).
    pub fn set_preedit(&mut self, text: &str, inv: &mut Invalidations) {
        if let Some(tb) = self.focused_box() {
            tb.set_preedit(text, inv);
        }
    }

    /// 우클릭 편집 메뉴 행동(1회성 · 두 상자 중 낸 쪽).
    pub fn take_edit_ctx(&mut self) -> Option<crate::controls::EditCtxAction> {
        self.name
            .take_edit_ctx()
            .or_else(|| self.email.take_edit_ctx())
    }

    /// 클립보드 텍스트 유무 주입(우클릭 시점 — 붙여넣기 항목 활성 근거).
    pub fn set_clipboard_has_text(&mut self, yes: bool) {
        self.name.set_clipboard_has_text(yes);
        self.email.set_clipboard_has_text(yes);
    }

    /// 선택 복사(OS 클립보드 쓰기는 호스트).
    #[must_use]
    pub fn clipboard_copy(&self) -> Option<String> {
        if self.name.is_focused() {
            self.name.copy_selection()
        } else if self.email.is_focused() {
            self.email.copy_selection()
        } else {
            None
        }
    }

    /// 선택 잘라내기.
    pub fn clipboard_cut(&mut self, inv: &mut Invalidations) -> Option<String> {
        self.focused_box().and_then(|tb| tb.cut_selection(inv))
    }

    /// 붙여넣기(호스트가 읽은 텍스트).
    pub fn clipboard_paste(&mut self, text: &str, inv: &mut Invalidations) {
        if let Some(tb) = self.focused_box() {
            tb.paste(text, inv);
        }
    }

    /// 창이 포커스를 잃었다 — 버튼의 눌림·hover 잔상을 지운다.
    pub fn clear_transient(&mut self, inv: &mut Invalidations) {
        for b in [
            &mut self.btn_copy,
            &mut self.btn_open,
            &mut self.btn_remove,
            &mut self.btn_close,
        ] {
            b.clear_transient();
        }
        inv.push(self.bounds);
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }

    /// 표 아래 요청 구역의 위쪽 y(제목 줄).
    fn request_y(&self) -> i32 {
        let rows = i32::try_from(self.view.rows.len()).unwrap_or(0);
        self.bounds.y + self.s(PAD + BANNER_H + 10 + rows * ROW_H + 18)
    }

    fn relayout(&mut self, inv: &mut Invalidations) {
        let b = self.bounds;
        let pad = self.s(PAD);
        // 요청 구역: 제목 줄 아래 [이름][이메일][요청 코드 복사].
        let fy = self.request_y() + self.s(ROW_H + 6);
        let fh = self.s(FIELD_H);
        let mut x = b.x + pad;
        self.name
            .set_bounds(Rect::new(x, fy, self.s(NAME_W), fh), inv);
        x += self.s(NAME_W + GAP);
        self.email
            .set_bounds(Rect::new(x, fy, self.s(EMAIL_W), fh), inv);
        x += self.s(EMAIL_W + GAP);
        let cw = self.s(label_w(t(Msg::LicBtnCopyReq)));
        self.btn_copy.set_bounds(Rect::new(x, fy, cw, fh), inv);
        // 버튼 행 = 아래 고정(sql 사용자 09-27 "설명 밑에 버튼").
        let bh = self.s(BTN_H);
        let by = b.bottom() - pad - bh;
        let mut bx = b.x + pad;
        for (btn, label) in [
            (&mut self.btn_open, Msg::LicBtnOpen),
            (&mut self.btn_remove, Msg::LicBtnRemove),
            (&mut self.btn_close, Msg::LicBtnClose),
        ] {
            let w = (label_w(t(label)) as f32 * self.scale).round() as i32;
            btn.set_bounds(Rect::new(bx, by, w, bh), inv);
            bx += w + (GAP as f32 * self.scale).round() as i32;
        }
    }

    /// 버튼 행 오른쪽 끝 x(결과 한 줄 자리).
    fn note_x(&self) -> i32 {
        self.btn_close.bounds().right() + self.s(12)
    }

    fn take_clicks(&mut self) {
        if self.btn_copy.take_clicked() {
            self.action = Some(LicAction::CopyRequest {
                name: self.name.text().trim().to_string(),
                email: self.email.text().trim().to_string(),
            });
        } else if self.btn_open.take_clicked() {
            self.action = Some(LicAction::OpenFile);
        } else if self.btn_remove.take_clicked() {
            self.action = Some(LicAction::Remove);
        } else if self.btn_close.take_clicked() {
            self.action = Some(LicAction::Close);
        }
    }

    fn popup_open(&self) -> bool {
        self.name.popup_open() || self.email.popup_open()
    }
}

impl Widget for LicenseWidget {
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
            // Esc = 닫기(우클릭 메뉴가 열려 있으면 메뉴 몫).
            InputEvent::Key {
                key: Key::Escape, ..
            } if !self.popup_open() => {
                self.action = Some(LicAction::Close);
                return;
            }
            InputEvent::MouseMove { x, y } => {
                let over = self.link.get().contains(Point { x, y });
                if over != self.link_hover {
                    self.link_hover = over;
                    inv.push(self.link.get());
                }
            }
            InputEvent::MouseDown { x, y, .. } => {
                let p = Point { x, y };
                // 이메일 링크 = 주소 복사(sql 사용자 09-27).
                if self.link.get().contains(p) && !self.view.contact.is_empty() {
                    self.action = Some(LicAction::CopyContact(self.view.contact.clone()));
                    return;
                }
                // 포커스 링은 하나 — 상자 밖을 누르면 둘 다 놓는다(버튼 누름 포함).
                if !self.popup_open() {
                    let on_name = self.name.bounds().contains(p);
                    let on_email = self.email.bounds().contains(p);
                    self.name.set_focused(on_name);
                    self.email.set_focused(on_email);
                }
            }
            _ => {}
        }
        // 마우스 라우팅: 누름은 커서 아래 컨트롤에만 · 뗌·이동은 전부(nexa-ctl = 뗄 때 확정).
        // 키·문자·편집 명령은 포커스된 상자만.
        match *ev {
            InputEvent::MouseDown { x, y, .. }
            | InputEvent::RightDown { x, y }
            | InputEvent::DoubleClick { x, y, .. } => {
                let p = Point { x, y };
                for tb in [&mut self.name, &mut self.email] {
                    if tb.bounds().contains(p) || tb.popup_open() {
                        tb.on_event(ev, inv);
                    }
                }
                for b in [
                    &mut self.btn_copy,
                    &mut self.btn_open,
                    &mut self.btn_remove,
                    &mut self.btn_close,
                ] {
                    if b.bounds().contains(p) {
                        b.on_event(ev, inv);
                    }
                }
            }
            InputEvent::MouseUp { .. } | InputEvent::MouseMove { .. } => {
                for tb in [&mut self.name, &mut self.email] {
                    tb.on_event(ev, inv);
                }
                for b in [
                    &mut self.btn_copy,
                    &mut self.btn_open,
                    &mut self.btn_remove,
                    &mut self.btn_close,
                ] {
                    b.on_event(ev, inv);
                }
            }
            InputEvent::Wheel { .. }
            | InputEvent::HWheel { .. }
            | InputEvent::MiddleDown { .. }
            | InputEvent::XButton { .. } => {}
            // Enter는 한 줄 상자에서 확정만(전송·복사로 쓰지 않는다 — 실수로 요청 코드가 나가지 않게).
            _ => {
                if let Some(tb) = self.focused_box() {
                    tb.on_event(ev, inv);
                    let _ = tb.take_committed();
                }
            }
        }
        self.take_clicks();
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);
        ctx.select_font(FontSlot::Base, false);
        let th = ctx.text_height();
        let pad = self.s(PAD);
        // 상태 띠.
        let (band, fg): (Color, Color) = match self.view.tone {
            LicTone::Ok => (theme.ok, theme.ok),
            LicTone::Warn => (theme.warn, theme.warn),
            LicTone::Neutral => (theme.accent, theme.text),
        };
        let banner = Rect::new(b.x + pad, b.y + pad, b.w - pad * 2, self.s(BANNER_H));
        ctx.fill_rect_alpha(banner, band, 0.12);
        let st = ellipsize_middle(ctx, &self.view.state, banner.w - self.s(16));
        ctx.text(
            banner.x + self.s(8),
            banner.y + (banner.h - th) / 2,
            banner,
            &st,
            fg,
        );
        // 표.
        let mut y = banner.bottom() + self.s(10);
        let label_w = self
            .view
            .rows
            .iter()
            .map(|(k, _)| ctx.text_width(k))
            .max()
            .unwrap_or(0)
            + self.s(16);
        for (k, v) in &self.view.rows {
            ctx.text(b.x + pad, y, b, k, theme.text_dim);
            let vr = Rect::new(
                b.x + pad + label_w,
                y,
                b.w - pad * 2 - label_w,
                self.s(ROW_H),
            );
            let v = ellipsize_middle(ctx, v, vr.w);
            ctx.text(vr.x, y, vr, &v, theme.text);
            y += self.s(ROW_H);
        }
        y += self.s(8);
        ctx.fill_rect(Rect::new(b.x + pad, y, b.w - pad * 2, 1), theme.border);
        // 요청 구역 제목: "… {이메일} …" — 이메일은 링크(강조색 · hover 밑줄 · 클릭 = 복사).
        let y = self.request_y();
        let tpl = tf(Msg::LicReqTitle, &["\u{1}"]);
        let (pre, post) = tpl.split_once('\u{1}').unwrap_or((tpl.as_str(), ""));
        let mut tx = b.x + pad;
        ctx.text(tx, y, b, pre, theme.text);
        tx += ctx.text_width(pre);
        let lw = ctx.text_width(&self.view.contact);
        ctx.text(tx, y, b, &self.view.contact, theme.accent);
        if self.link_hover {
            ctx.fill_rect(Rect::new(tx, y + th - self.s(2), lw, 1), theme.accent);
        }
        self.link.set(Rect::new(tx, y, lw, th + self.s(2)));
        ctx.text(tx + lw, y, b, post, theme.text);
        // 입력 · 복사 버튼.
        self.name.paint(ctx, theme);
        self.email.paint(ctx, theme);
        self.btn_copy.paint(ctx, theme);
        // 요청 코드 미리보기 + 안내(버튼 행을 넘지 않는 온전한 줄만).
        let floor = self.btn_open.bounds().y - self.s(10);
        let mut y = self.name.bounds().bottom() + self.s(8);
        let code = self
            .view
            .request
            .clone()
            .unwrap_or_else(|| t(Msg::LicNoMachine).to_string());
        ctx.select_font(FontSlot::Status, false);
        let code = ellipsize_middle(ctx, &code, b.w - pad * 2);
        if y + th <= floor {
            ctx.text(b.x + pad, y, b, &code, theme.text_dim);
        }
        y += self.s(ROW_H);
        if y + th <= floor {
            ctx.text(b.x + pad, y, b, t(Msg::LicHint), theme.text_dim);
        }
        // 버튼 행 + 결과 한 줄(버튼 오른쪽 같은 줄 — 세로 공간을 쓰지 않는다).
        self.btn_open.paint(ctx, theme);
        self.btn_remove.paint(ctx, theme);
        self.btn_close.paint(ctx, theme);
        if let Some((n, warn)) = &self.note {
            ctx.select_font(FontSlot::Base, false);
            let nx = self.note_x();
            let nw = (b.right() - pad - nx).max(0);
            let br = self.btn_close.bounds();
            let n = ellipsize_middle(ctx, n, nw);
            ctx.text(
                nx,
                br.y + (br.h - th) / 2,
                Rect::new(nx, br.y, nw, br.h),
                &n,
                if *warn { theme.danger } else { theme.ok },
            );
        }
        // 우클릭 편집 메뉴 = 최상위.
        self.name.paint_popup(ctx, theme);
        self.email.paint_popup(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbeep_license::License;

    fn view(rows: usize, request: bool) -> LicView {
        LicView {
            state: "Free".into(),
            tone: LicTone::Neutral,
            rows: (0..rows)
                .map(|i| (format!("k{i}"), format!("v{i}")))
                .collect(),
            request: request.then(|| "NEXAREQ1.AAAA.BBBB".to_string()),
            contact: "a@b.c".into(),
        }
    }

    fn widget(request: bool) -> (LicenseWidget, Invalidations) {
        crate::controls::button::set_default_click_guard_ms(0);
        let mut w = LicenseWidget::new(view(3, request));
        let mut inv = Invalidations::default();
        w.set_bounds(Rect::new(0, 0, 640, 420), &mut inv);
        (w, inv)
    }

    /// nexa-ctl 컨트롤은 뗄 때 확정 — 누름+뗌 쌍(settings.rs 시험과 같은 도우미).
    fn click(w: &mut LicenseWidget, r: Rect, inv: &mut Invalidations) {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        w.on_event(
            &InputEvent::MouseDown {
                x,
                y,
                shift: false,
                primary: false,
            },
            inv,
        );
        w.on_event(&InputEvent::MouseUp { x, y }, inv);
    }

    fn type_str(w: &mut LicenseWidget, s: &str, inv: &mut Invalidations) {
        for c in s.chars() {
            w.on_event(&InputEvent::Char { c, now_ms: 0 }, inv);
        }
    }

    #[test]
    fn buttons_emit_actions_once() {
        let (mut w, mut inv) = widget(true);
        for (b, want) in [
            (w.btn_open.bounds(), LicAction::OpenFile),
            (w.btn_remove.bounds(), LicAction::Remove),
            (w.btn_close.bounds(), LicAction::Close),
        ] {
            click(&mut w, b, &mut inv);
            assert_eq!(w.take_action(), Some(want));
            assert_eq!(w.take_action(), None, "1회성");
        }
    }

    #[test]
    fn copy_request_carries_trimmed_name_and_email() {
        let (mut w, mut inv) = widget(true);
        let r = w.name.bounds();
        click(&mut w, r, &mut inv);
        type_str(&mut w, " 홍길동 ", &mut inv);
        let r = w.email.bounds();
        click(&mut w, r, &mut inv);
        type_str(&mut w, "a@b.c ", &mut inv);
        assert!(
            w.email.is_focused() && !w.name.is_focused(),
            "포커스 링 하나"
        );
        let r = w.btn_copy.bounds();
        click(&mut w, r, &mut inv);
        assert_eq!(
            w.take_action(),
            Some(LicAction::CopyRequest {
                name: "홍길동".into(),
                email: "a@b.c".into()
            })
        );
    }

    #[test]
    fn copy_disabled_without_machine_id() {
        let (mut w, mut inv) = widget(false);
        let r = w.btn_copy.bounds();
        click(&mut w, r, &mut inv);
        assert_eq!(w.take_action(), None, "기기 ID 없음 = 요청 코드 불가");
        // 보기가 바뀌면 다시 켜진다.
        w.set_view(view(3, true), &mut inv);
        let r = w.btn_copy.bounds();
        click(&mut w, r, &mut inv);
        assert!(matches!(
            w.take_action(),
            Some(LicAction::CopyRequest { .. })
        ));
    }

    #[test]
    fn escape_closes_and_enter_does_not_copy() {
        let (mut w, mut inv) = widget(true);
        w.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.take_action(), None, "Enter로 요청 코드가 나가지 않는다");
        w.on_event(
            &InputEvent::Key {
                key: Key::Escape,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert_eq!(w.take_action(), Some(LicAction::Close));
    }

    #[test]
    fn contact_link_click_copies_address() {
        let (mut w, mut inv) = widget(true);
        // 링크 자리는 그릴 때 정해진다 — 시험은 자리를 직접 놓는다.
        w.link.set(Rect::new(100, 200, 80, 18));
        click(&mut w, Rect::new(100, 200, 80, 18), &mut inv);
        assert_eq!(
            w.take_action(),
            Some(LicAction::CopyContact("a@b.c".into()))
        );
    }

    #[test]
    fn desired_height_grows_with_rows_and_scale() {
        let (mut w, mut inv) = widget(true);
        let h3 = w.desired_height();
        w.set_view(view(8, true), &mut inv);
        let h8 = w.desired_height();
        assert_eq!(h8 - h3, 5 * ROW_H);
        w.set_scale(2.0, &mut inv);
        assert_eq!(w.desired_height(), h8 * 2);
    }

    #[test]
    fn layout_keeps_controls_inside_and_apart() {
        let (w, _) = widget(true);
        let b = w.bounds();
        let all = [
            w.name.bounds(),
            w.email.bounds(),
            w.btn_copy.bounds(),
            w.btn_open.bounds(),
            w.btn_remove.bounds(),
            w.btn_close.bounds(),
        ];
        for r in all {
            assert!(r.x >= b.x && r.right() <= b.right(), "{r:?}");
            assert!(r.y >= b.y && r.bottom() <= b.bottom(), "{r:?}");
        }
        for (i, a) in all.iter().enumerate() {
            for c in &all[i + 1..] {
                assert!(!a.intersects(c), "{a:?} ∩ {c:?}");
            }
        }
        assert!(
            w.btn_open.bounds().y > w.name.bounds().bottom(),
            "버튼 행 = 아래"
        );
    }

    fn lic(tier: &str, updates: &str, expires: &str) -> License {
        License {
            id: "NBL-2026-000001".into(),
            tier: tier.into(),
            updates_until: updates.into(),
            expires: expires.into(),
            licensee: String::new(),
            kind: nbeep_license::Kind::User,
            features: std::collections::BTreeSet::new(),
            machines: Vec::new(),
            issued: String::new(),
            seats: 0,
            seat_mode: None,
            key_id: String::new(),
            max_major: None,
            max_version: String::new(),
        }
    }

    #[test]
    fn state_text_tones_and_args() {
        nbeep_core::set_lang(nbeep_core::Lang::En);
        assert_eq!(state_text(&LicenseState::Free).1, LicTone::Neutral);
        let (s, tone) = state_text(&LicenseState::Licensed(lic("pro", "", "")));
        assert_eq!(tone, LicTone::Ok);
        assert!(s.contains("pro"), "{s}");
        let (s, tone) = state_text(&LicenseState::Invalid(Invalid::Machine));
        assert_eq!(tone, LicTone::Warn);
        assert!(s.contains(t(Msg::LicWhyMachine)), "{s}");
        let (s, _) = state_text(&LicenseState::Outdated(lic("pro", "2026-01-01", "")));
        assert!(s.contains("2026-01-01"), "{s}");
        let (s, _) = state_text(&LicenseState::Expired(lic("pro", "", "2025-12-31")));
        assert!(s.contains("2025-12-31"), "{s}");
    }

    #[test]
    fn license_view_free_has_file_build_machine_rows() {
        nbeep_core::set_lang(nbeep_core::Lang::En);
        let dir = std::env::temp_dir().join(format!("nbeep-licview-{}", std::process::id()));
        let l = Licensing::open(vec![dir.join("u")]);
        let v = license_view(&l);
        assert_eq!(v.tone, LicTone::Neutral);
        let labels: Vec<&str> = v.rows.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            labels,
            [
                t(Msg::LicRowFile),
                t(Msg::LicRowBuild),
                t(Msg::LicRowMachine)
            ]
        );
        assert_eq!(v.rows[0].1, "-");
        assert!(v.rows[1].1.starts_with(PRODUCT.build_date));
        assert_eq!(v.contact, nbeep_license::LICENSE_CONTACT);
        assert_eq!(v.request.is_some(), Licensing::machine_code().is_some());
    }

    #[test]
    fn button_labels_match_issuer_mail_wording() {
        // 발급 메일(nexa-license presets nexa-beep 분기)과 글자까지 같아야 한다 — 바꾸면 presets도 같이.
        assert_eq!(
            nbeep_core::tr(nbeep_core::Lang::Ko, Msg::LicBtnOpen),
            "라이선스 파일 열기…"
        );
        assert_eq!(
            nbeep_core::tr(nbeep_core::Lang::Ko, Msg::LicBtnCopyReq),
            "요청 코드 복사"
        );
        assert_eq!(
            nbeep_core::tr(nbeep_core::Lang::En, Msg::LicBtnOpen),
            "Open license file…"
        );
        assert_eq!(
            nbeep_core::tr(nbeep_core::Lang::En, Msg::LicBtnCopyReq),
            "Copy request code"
        );
        assert_eq!(
            nbeep_core::tr(nbeep_core::Lang::Ko, Msg::LicMenu),
            "라이선스…"
        );
        assert_eq!(
            nbeep_core::tr(nbeep_core::Lang::En, Msg::LicMenu),
            "License…"
        );
    }
}
