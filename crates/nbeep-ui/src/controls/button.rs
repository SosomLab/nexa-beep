//! 버튼 — 텍스트/이미지(옵션) · **이미지 버튼 모드** · 포커스 링 · 도움말(사용자 요청 08-08).
//!
//! - 일반 모드: **선행 이미지(옵션) + 텍스트(옵션)**. 텍스트가 없으면 이미지만, 이미지가 없으면
//!   텍스트만. 큰 이미지는 자동 축소되어 앞에 놓인다.
//! - 이미지 버튼 모드([`ButtonMode::Image`]): 이미지를 **버튼 크기에 맞춰 스케일**하고 넘치면
//!   버튼 영역으로 **잘라** 보여준다(Cover) 또는 버튼 안에 다 보이게(Contain).
//!
//! 공통 기능은 [`Control`] 기본 메서드로 상속([`super`]).

use super::{image_fit_contain, image_fit_cover, Control, ControlBase};
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

                let cy = b.y + b.h / 2;
                // 아이콘 변 = 콤보 아이콘(18)에서 사방 1px씩 줄인 16(사용자 확정 · 가로·세로 −2px).
                let icon = self.s(16).min(b.h - self.s(PAD));
                let s16 = self.s(16);
                // 콘텐츠(아이콘 + 라벨) 폭을 재서 가운데 정렬.
                let mut icon_w = 0;
                if self.image.is_some() {
                    icon_w = icon + self.s(GAP);
                }
                ctx.select_font(FontSlot::Base, false);
                let label_w = self.label.as_deref().map_or(0, |l| ctx.text_width(l));
                let content = icon_w + label_w;
                let mut x = b.x + ((b.w - content) / 2).max(self.s(PAD));

                if let Some(img) = self.image.as_deref() {
                    let boxr = Rect::new(x, cy - icon / 2, icon, icon);
                    let fit = image_fit_contain(boxr, img.w as i32, img.h as i32);
                    ctx.image_scaled(fit, img, b);
                    x += icon + self.s(GAP);
                }
                if let Some(label) = self.label.as_deref() {
                    ctx.text(x, cy - s16 / 2, b, label, theme.text);
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
