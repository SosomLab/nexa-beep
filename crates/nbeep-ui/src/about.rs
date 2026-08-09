//! **About 화면** — 브랜딩 이미지 + 앱 정보 + 링크(사용자 요청 08-09).
//!
//! 링크는 **표시만** 한다 — 자동/클릭 열기 없음(FR-S-14의 정신: 앱은 외부 실행을 하지 않는다).
//! 브랜딩 이미지는 [`crate::brand`] 임베드(RGBA)를 크게 그린다.

use crate::draw::{DrawCtx, FontSlot};
use crate::event::{InputEvent, Key};
use crate::geom::Rect;
use crate::theme::{IconImage, Theme};
use crate::widget::{Invalidations, Widget};
use std::rc::Rc;

/// About 정보(호스트가 채운다 — 버전은 bin의 `CARGO_PKG_VERSION`).
#[derive(Clone, Debug)]
pub struct AboutInfo {
    /// 앱 이름.
    pub app: String,
    /// 버전 문자열.
    pub version: String,
    /// 한 줄 소개.
    pub tagline: String,
    /// (라벨, URL) 링크 목록 — 표시 전용.
    pub links: Vec<(String, String)>,
}

/// About 화면 위젯.
#[derive(Debug)]
pub struct AboutWidget {
    bounds: Rect,
    scale: f32,
    info: AboutInfo,
    logo: Rc<IconImage>,
    back: bool,
}

impl AboutWidget {
    /// 정보로 만든다(로고 = 브랜딩 임베드).
    #[must_use]
    pub fn new(info: AboutInfo) -> Self {
        Self {
            bounds: Rect::default(),
            scale: 1.0,
            info,
            logo: Rc::new(IconImage::from_rgba(
                crate::brand::ICON_SIZE,
                crate::brand::ICON_SIZE,
                crate::brand::ICON_RGBA.to_vec(),
            )),
            back: false,
        }
    }

    /// 배율 지정.
    pub fn set_scale(&mut self, scale: f32, inv: &mut Invalidations) {
        self.scale = scale.max(0.5);
        inv.push(self.bounds);
    }

    /// Esc 닫기 요청(1회성).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    fn s(&self, v: i32) -> i32 {
        (v as f32 * self.scale).round() as i32
    }
}

impl Widget for AboutWidget {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn set_bounds(&mut self, bounds: Rect, inv: &mut Invalidations) {
        self.bounds = bounds;
        inv.push(bounds);
    }

    fn on_event(&mut self, ev: &InputEvent, _inv: &mut Invalidations) {
        if let InputEvent::Key {
            key: Key::Escape, ..
        } = *ev
        {
            self.back = true;
        }
    }

    fn paint(&self, ctx: &mut dyn DrawCtx, theme: &Theme) {
        let b = self.bounds;
        ctx.fill_rect(b, theme.panel_bg);

        // 로고(브랜딩) — 상단 중앙 96×96.
        let logo_d = self.s(96);
        let lx = b.x + (b.w - logo_d) / 2;
        let ly = b.y + self.s(28);
        ctx.image_scaled(Rect::new(lx, ly, logo_d, logo_d), &self.logo, b);

        // 앱 이름(굵게) + 버전.
        ctx.select_font(FontSlot::Base, true);
        let name_w = ctx.text_width(&self.info.app);
        let mut y = ly + logo_d + self.s(16);
        ctx.text(b.x + (b.w - name_w) / 2, y, b, &self.info.app, theme.text);
        y += self.s(26);
        ctx.select_font(FontSlot::Status, false);
        let ver = format!("v{}", self.info.version);
        let vw = ctx.text_width(&ver);
        ctx.text(b.x + (b.w - vw) / 2, y, b, &ver, theme.text_dim);
        y += self.s(24);
        let tw = ctx.text_width(&self.info.tagline);
        ctx.text(
            b.x + (b.w - tw) / 2,
            y,
            b,
            &self.info.tagline,
            theme.text_dim,
        );
        y += self.s(34);

        // 구분선.
        ctx.fill_rect(
            Rect::new(b.x + self.s(32), y, b.w - self.s(64), 1),
            theme.border,
        );
        y += self.s(18);

        // 링크(라벨 + URL 표시 전용 — 열기 없음).
        for (label, url) in &self.info.links {
            ctx.select_font(FontSlot::Base, false);
            ctx.text(b.x + self.s(40), y, b, label, theme.text);
            ctx.select_font(FontSlot::Status, false);
            ctx.text(b.x + self.s(40), y + self.s(20), b, url, theme.accent);
            y += self.s(48);
        }

        // 하단 저작자.
        ctx.select_font(FontSlot::Status, false);
        let c = "© SosomLab · Sangyong Bae";
        let cw = ctx.text_width(c);
        ctx.text(
            b.x + (b.w - cw) / 2,
            b.bottom() - self.s(30),
            b,
            c,
            theme.text_dim,
        );
    }
}
