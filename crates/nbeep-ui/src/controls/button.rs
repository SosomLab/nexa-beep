//! 버튼 — 텍스트/이미지(옵션) · **이미지 버튼 모드** · 포커스 링 · 도움말(사용자 요청 08-08).
//!
//! - 일반 모드: **선행 이미지(옵션) + 텍스트(옵션)**. 텍스트가 없으면 이미지만, 이미지가 없으면
//!   텍스트만. 큰 이미지는 자동 축소되어 앞에 놓인다.
//! - 이미지 버튼 모드([`ButtonMode::Image`]): 이미지를 **버튼 크기에 맞춰 스케일**하고 넘치면
//!   버튼 영역으로 **잘라** 보여준다(Cover) 또는 버튼 안에 다 보이게(Contain).
//!
//! 공통 기능은 [`Control`] 기본 메서드로 상속([`super`]).

use super::{image_fit_contain, image_fit_cover, Control, ControlBase, HAlign};
use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::{Point, Rect};
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

const PAD: i32 = 8;
const RADIUS: i32 = 6;
const GAP: i32 = 6;

/// 이미지 버튼 맞춤 방식.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageFit {
    /// 버튼 안에 전부 보이게(비율 유지 · 여백).
    Contain,
    /// 버튼을 가득 채우고 넘치는 부분은 잘림(비율 유지 · 크롭).
    Cover,
}

/// 버튼 모드.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonMode {
    /// 선행 이미지(옵션) + 텍스트(옵션).
    Normal,
    /// 이미지 버튼 — 이미지를 버튼 크기에 맞춰 스케일·클립.
    Image(ImageFit),
}

/// 버튼 컨트롤(이미지 버튼 포함 — 별도 컨트롤로 나누지 않음 · 사용자 확정).
#[derive(Debug)]
pub struct Button {
    base: ControlBase,
    label: Option<String>,
    image: Option<Rc<IconImage>>,
    mode: ButtonMode,
    /// **이미지 맨 앞 고정**(옵션): true면 텍스트 정렬과 무관하게 이미지가 버튼 앞(pad)에
    /// 붙는다. false(기본)면 이미지+텍스트를 한 묶음으로 정렬(halign)한다.
    image_leading: bool,
    pressed: bool,
    clicked: bool,
}

impl Button {
    /// 텍스트 버튼.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            base: ControlBase::default(),
            label: Some(label.into()),
            image: None,
            mode: ButtonMode::Normal,
            image_leading: false,
            pressed: false,
            clicked: false,
        }
    }

    /// 이미지만 있는 버튼(텍스트 없음).
    #[must_use]
    pub fn icon(image: Rc<IconImage>) -> Self {
        Self {
            base: ControlBase::default(),
            label: None,
            image: Some(image),
            mode: ButtonMode::Normal,
            image_leading: false,
            pressed: false,
            clicked: false,
        }
    }

    /// 선행 이미지 지정(체이닝).
    #[must_use]
    pub fn with_image(mut self, image: Rc<IconImage>) -> Self {
        self.image = Some(image);
        self
    }

    /// 텍스트 지정(체이닝).
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// **이미지 버튼 모드**로 전환 — 이미지를 버튼 크기에 맞춰 스케일·클립.
    #[must_use]
    pub fn image_fill(mut self, fit: ImageFit) -> Self {
        self.mode = ButtonMode::Image(fit);
        self
    }

    /// **이미지 맨 앞 고정** 옵션(체이닝). 텍스트 정렬 규칙(사용자 확정):
    /// Left=이미지 뒤 · Center=버튼 전체 기준 중앙(이미지 무시) · Right=우측 정렬.
    #[must_use]
    pub fn image_front(mut self, on: bool) -> Self {
        self.image_leading = on;
        self
    }

    /// 이미지 맨 앞 고정 지정(런타임).
    pub fn set_image_front(&mut self, on: bool) {
        self.image_leading = on;
    }

    /// (Normal) 이미지/텍스트 x 배치 — 정렬·이미지 맨 앞 규칙의 **순수 계산**(테스트 근거).
    /// 반환: (이미지 x, 텍스트 x). 물리 px.
    #[allow(clippy::too_many_arguments)]
    fn normal_positions(
        b: Rect,
        pad: i32,
        gap: i32,
        icon: i32,
        label_w: i32,
        has_img: bool,
        has_label: bool,
        leading: bool,
        halign: HAlign,
    ) -> (Option<i32>, Option<i32>) {
        if leading && has_img {
            let img_x = b.x + pad;
            let text_x = has_label.then(|| match halign {
                HAlign::Left => img_x + icon + gap,
                HAlign::Center => b.x + (b.w - label_w) / 2, // 이미지 무시, 버튼 전체 기준
                HAlign::Right => b.right() - pad - label_w,
            });
            return (Some(img_x), text_x);
        }
        // 묶음 정렬: 이미지+텍스트(사이 gap은 둘 다 있을 때만 — 이미지 전용은 정확히 중앙).
        let group_w =
            if has_img { icon } else { 0 } + if has_img && has_label { gap } else { 0 } + label_w;
        let gx = match halign {
            HAlign::Left => b.x + pad,
            HAlign::Center => b.x + (b.w - group_w) / 2,
            HAlign::Right => b.right() - pad - group_w,
        };
        let img_x = has_img.then_some(gx);
        let text_x = has_label.then(|| {
            gx + if has_img {
                icon + if has_label { gap } else { 0 }
            } else {
                0
            }
        });
        (img_x, text_x)
    }

    /// 눌렸으면 `true`(1회성) — 호스트가 동작 실행.
    pub fn take_clicked(&mut self) -> bool {
        std::mem::take(&mut self.clicked)
    }
}

impl Control for Button {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
}

impl Widget for Button {
    fn bounds(&self) -> Rect {
        self.base.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.base.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, inv: &mut Invalidations) {
        match *ev {
            InputEvent::MouseDown { x, y, .. } => {
                let badge = self.help_badge_rect(self.base.bounds);
                if self.handle_help_click(x, y, badge) {
                    inv.push(self.base.bounds);
                    return;
                }
                if self.base.bounds.contains(Point { x, y }) {
                    self.pressed = true;
                    self.base.focused = true;
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::MouseUp { x, y } => {
                if self.pressed {
                    self.pressed = false;
                    if self.base.bounds.contains(Point { x, y }) {
                        self.clicked = true;
                    }
                    inv.push(self.base.bounds);
                }
            }
            InputEvent::Key { key, .. } if self.base.focused => {
                if matches!(key, Key::Enter | Key::Space) {
                    self.clicked = true;
                }
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.base.bounds;
        let radius = self.s(RADIUS);

        match self.mode {
            ButtonMode::Image(fit) => {
                // 이미지 버튼 — 배경 + 버튼 크기에 맞춘 이미지(클립).
                let bg = if self.pressed {
                    theme.sel_bg
                } else {
                    theme.field_bg
                };
                ctx.fill_round_rect(b, radius, bg);
                if let Some(img) = self.image.as_deref() {
                    let area = Rect::new(
                        b.x + self.s(2),
                        b.y + self.s(2),
                        b.w - self.s(4),
                        b.h - self.s(4),
                    );
                    let dst = match fit {
                        ImageFit::Contain => image_fit_contain(area, img.w as i32, img.h as i32),
                        ImageFit::Cover => image_fit_cover(area, img.w as i32, img.h as i32),
                    };
                    // clip = 버튼 영역 → Cover에서 넘치는 부분은 잘린다.
                    ctx.image_scaled(dst, img, area);
                }
                ctx.stroke_round_rect(b, radius, theme.border, 1.0);
            }
            ButtonMode::Normal => {
                let bg = if self.pressed {
                    theme.sel_bg
                } else {
                    theme.field_bg
                };
                ctx.fill_round_rect(b, radius, bg);
                ctx.stroke_round_rect(b, radius, theme.border, 1.0);

                // 아이콘 변 = 공용 단일 원천(콤보/Choose/트리와 동일 — 드리프트 방지).
                let icon = self.s(super::LEADING_ICON);
                ctx.select_font(FontSlot::Base, false);
                let th = ctx.text_height();
                let label_w = self.label.as_deref().map_or(0, |l| ctx.text_width(l));
                let (img_x, text_x) = Self::normal_positions(
                    b,
                    self.s(PAD),
                    self.s(GAP),
                    icon,
                    label_w,
                    self.image.is_some(),
                    self.label.is_some(),
                    self.image_leading,
                    self.halign(),
                );
                // 세로 배치 = VAlign(기본 중앙).
                let icon_y = self.align_y(b, icon, self.s(4));
                let text_y = self.align_y(b, th, self.s(4));
                if let (Some(x), Some(img)) = (img_x, self.image.as_deref()) {
                    let boxr = Rect::new(x, icon_y, icon, icon);
                    let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
                    ctx.image_scaled(fit, img, b);
                }
                if let (Some(x), Some(label)) = (text_x, self.label.as_deref()) {
                    ctx.text(x, text_y, b, label, theme.text);
                }
            }
        }

        self.draw_focus_ring(ctx, theme, b);
        let badge = self.help_badge_rect(b);
        self.draw_help_badge(ctx, theme, badge);
        self.draw_help_tip(ctx, theme, badge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::IconImage;

    fn img() -> Rc<IconImage> {
        Rc::new(IconImage::swatch(24, (0, 120, 255)))
    }
    fn btn(mut b: Button) -> (Button, Invalidations) {
        let mut inv = Invalidations::default();
        b.set_bounds(Rect::new(0, 0, 120, 32), &mut inv);
        (b, inv)
    }
    fn down(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseDown {
            x,
            y,
            shift: false,
            primary: false,
        }
    }
    fn up(x: i32, y: i32) -> InputEvent {
        InputEvent::MouseUp { x, y }
    }

    #[test]
    fn press_release_inside_clicks() {
        let (mut b, mut inv) = btn(Button::new("OK"));
        b.on_event(&down(10, 10), &mut inv);
        assert!(b.pressed);
        b.on_event(&up(10, 10), &mut inv);
        assert!(b.take_clicked(), "안에서 떼면 클릭");
        assert!(!b.take_clicked(), "1회성");
    }

    #[test]
    fn release_outside_does_not_click() {
        let (mut b, mut inv) = btn(Button::new("OK"));
        b.on_event(&down(10, 10), &mut inv);
        b.on_event(&up(500, 500), &mut inv);
        assert!(!b.take_clicked(), "밖에서 떼면 취소");
    }

    #[test]
    fn enter_clicks_when_focused() {
        let (mut b, mut inv) = btn(Button::new("OK"));
        b.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(!b.take_clicked(), "비포커스 무시");
        b.set_focused(true);
        b.on_event(
            &InputEvent::Key {
                key: Key::Enter,
                shift: false,
                primary: false,
            },
            &mut inv,
        );
        assert!(b.take_clicked());
    }

    #[test]
    fn icon_only_is_exactly_centered() {
        // 이미지 전용 = 보이지 않는 gap 없이 정중앙(사용자 확정).
        let b = Rect::new(0, 0, 100, 26);
        let (img_x, text_x) =
            Button::normal_positions(b, 8, 6, 13, 0, true, false, false, HAlign::Center);
        assert_eq!(img_x, Some((100 - 13) / 2), "gap 미포함 정중앙");
        assert_eq!(text_x, None);
    }

    #[test]
    fn image_front_text_alignment_rules() {
        let b = Rect::new(0, 0, 200, 26);
        // Left: 이미지 뒤 + gap.
        let (i, t) = Button::normal_positions(b, 8, 6, 13, 40, true, true, true, HAlign::Left);
        assert_eq!(i, Some(8));
        assert_eq!(t, Some(8 + 13 + 6));
        // Center: 이미지 무시, 버튼 전체 기준 중앙.
        let (_, t) = Button::normal_positions(b, 8, 6, 13, 40, true, true, true, HAlign::Center);
        assert_eq!(t, Some((200 - 40) / 2));
        // Right: 일반 우측 정렬.
        let (_, t) = Button::normal_positions(b, 8, 6, 13, 40, true, true, true, HAlign::Right);
        assert_eq!(t, Some(200 - 8 - 40));
    }

    #[test]
    fn group_alignment_includes_image() {
        // 이미지 앞 고정 미선택 = 이미지 포함 묶음으로 정렬(사용자 확정).
        let b = Rect::new(0, 0, 200, 26);
        let group = 13 + 6 + 40;
        let (i, t) = Button::normal_positions(b, 8, 6, 13, 40, true, true, false, HAlign::Right);
        assert_eq!(i, Some(200 - 8 - group));
        assert_eq!(t, Some(200 - 8 - group + 13 + 6));
        let (i, _) = Button::normal_positions(b, 8, 6, 13, 40, true, true, false, HAlign::Left);
        assert_eq!(i, Some(8));
    }

    #[test]
    fn constructors_set_mode_and_content() {
        let text = Button::new("Save");
        assert!(text.label.is_some() && text.image.is_none());
        assert_eq!(text.mode, ButtonMode::Normal);

        let icon_only = Button::icon(img());
        assert!(icon_only.label.is_none() && icon_only.image.is_some());

        let imgbtn = Button::icon(img()).image_fill(ImageFit::Cover);
        assert_eq!(imgbtn.mode, ButtonMode::Image(ImageFit::Cover));
    }
}
