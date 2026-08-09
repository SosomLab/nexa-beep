//! CPU 래스터 백엔드 — [`DrawCtx`]를 `nbeep-gfx` 위에 구현(ADR-0001 B안의 실체).
//!
//! 원본(nexa-dir2)의 백엔드는 GDI/DirectWrite였다 — **같은 어휘를 우리 래스터라이저로 다시
//! 구현**하는 것이 이식의 핵심이다(위젯 코드는 백엔드를 모른다). AA 도형은 픽셀별
//! **부호 거리(SDF) 커버리지**로 그린다 — 1비트 리전 클립을 쓰지 않는다([docs/12 §B] `behind`
//! 함정의 근본 해소: 우리는 배경 위 진짜 블렌드가 된다).

use crate::draw::{DrawCtx, FontSlot};
use crate::geom::Rect;
use crate::theme::{Color, FontPrefs, SlotFont};
use nbeep_gfx::{Font, Surface, TextStyle};

/// [`Surface`] + [`Font`] 위의 [`DrawCtx`] 구현체.
#[allow(missing_debug_implementations)]
pub struct RasterCtx<'s, 'b, 'f> {
    surface: &'s mut Surface<'b>,
    font: &'f Font,
    /// 영역별 글꼴 설정(사용자 설정).
    prefs: FontPrefs,
    /// 현재 선택된 슬롯 글꼴(select_font로 전환).
    cur: SlotFont,
    /// 배율(고DPI — FR-U-6). 크기에 곱한다. 좌표는 이미 물리 px(호출자 몫).
    scale: f32,
}

impl<'s, 'b, 'f> RasterCtx<'s, 'b, 'f> {
    /// 표면과 폰트로 컨텍스트를 만든다(기본 슬롯 = Base).
    pub fn new(surface: &'s mut Surface<'b>, font: &'f Font) -> Self {
        let prefs = FontPrefs::default();
        Self {
            surface,
            font,
            prefs,
            cur: prefs.base,
            scale: 1.0,
        }
    }

    /// 사용자 글꼴 설정 지정.
    #[must_use]
    pub fn with_fonts(mut self, prefs: FontPrefs) -> Self {
        self.prefs = prefs;
        self.cur = prefs.base;
        self
    }

    /// 배율 지정(창의 scale factor — 텍스트 크기에 반영).
    #[must_use]
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale.max(0.5);
        self
    }

    /// 현재 슬롯의 물리 픽셀 크기.
    fn px_size(&self) -> f32 {
        self.cur.size * self.scale
    }

    fn cur_style(&self) -> TextStyle {
        TextStyle {
            bold: self.cur.bold,
            italic: self.cur.italic,
        }
    }

    fn clip_of(rect: Rect) -> (i32, i32, i32, i32) {
        (rect.x, rect.y, rect.right(), rect.bottom())
    }

    /// rect 영역을 픽셀별 SDF 커버리지로 채운다.
    fn coverage_fill(&mut self, rect: Rect, color: Color, dist: impl Fn(f32, f32) -> f32) {
        self.coverage_fill_alpha(rect, color, 1.0, dist);
    }

    /// [`Self::coverage_fill`]의 불투명도 변형 — 커버리지에 `alpha`를 곱한다(반투명 테두리).
    fn coverage_fill_alpha(
        &mut self,
        rect: Rect,
        color: Color,
        alpha: f32,
        dist: impl Fn(f32, f32) -> f32,
    ) {
        let a = alpha.clamp(0.0, 1.0);
        for py in rect.y..rect.bottom() {
            for px in rect.x..rect.right() {
                // 픽셀 중심 기준 거리 → 커버리지(0.5px 폭 안티에일리어싱).
                let d = dist(px as f32 + 0.5, py as f32 + 0.5);
                let cov = (0.5 - d).clamp(0.0, 1.0) * a;
                if cov > 0.0 {
                    self.surface.blend_px(px, py, color, cov);
                }
            }
        }
    }
}

/// 라운드 사각형 SDF — 중심 좌표계, 반코너 반경 `r`.
fn round_rect_sdf(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hw - r);
    let qy = (y - cy).abs() - (hh - r);
    let ax = qx.max(0.0);
    let ay = qy.max(0.0);
    (ax * ax + ay * ay).sqrt() + qx.max(qy).min(0.0) - r
}

/// 점-선분 거리.
fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let (ex, ey) = (ax + t * dx - px, ay + t * dy - py);
    (ex * ex + ey * ey).sqrt()
}

impl DrawCtx for RasterCtx<'_, '_, '_> {
    fn select_font(&mut self, slot: FontSlot, bold: bool) {
        self.cur = match slot {
            FontSlot::Base => self.prefs.base,
            FontSlot::PeerList => self.prefs.peerlist,
            FontSlot::Message => self.prefs.message,
            FontSlot::Status => self.prefs.status,
        };
        // 인자 bold는 슬롯 설정 위 강제 볼드(예: 강조 라벨).
        self.cur.bold |= bold;
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        if rect.is_empty() {
            return;
        }
        self.surface.fill_rect(
            rect.x,
            rect.y,
            rect.w.max(0) as u32,
            rect.h.max(0) as u32,
            color,
        );
    }

    fn text_opaque(&mut self, x: i32, y: i32, clip: Rect, text: &str, fg: Color, bg: Color) {
        self.fill_rect(clip, bg);
        self.text(x, y, clip, text, fg);
    }

    fn text(&mut self, x: i32, y: i32, clip: Rect, text: &str, fg: Color) {
        let size = self.px_size();
        let baseline = y as f32 + self.font.ascent(size);
        self.font.draw_styled(
            self.surface,
            x as f32,
            baseline,
            size,
            fg,
            text,
            Self::clip_of(clip),
            self.cur_style(),
        );
    }

    fn text_width(&mut self, text: &str) -> i32 {
        self.font.measure(text, self.px_size()).ceil() as i32
    }

    fn text_height(&mut self) -> i32 {
        self.font.text_box_height(self.px_size()).ceil() as i32
    }

    fn image(&mut self, x: i32, y: i32, img: &crate::theme::IconImage, clip: Rect) {
        self.surface.blend_image(x, y, img, Self::clip_of(clip));
    }

    fn image_scaled(&mut self, dst: Rect, img: &crate::theme::IconImage, clip: Rect) {
        self.surface
            .blend_image_scaled(dst.x, dst.y, dst.w, dst.h, img, Self::clip_of(clip));
    }

    fn fill_ellipse(&mut self, rect: Rect, color: Color) {
        if rect.is_empty() {
            return;
        }
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0,
        );
        let (rx, ry) = (rect.w as f32 / 2.0, rect.h as f32 / 2.0);
        self.coverage_fill(rect, color, move |x, y| {
            // 근사 SDF — 정규화 거리에 짧은 반경을 곱한다(원에 가까울수록 정확).
            let nx = (x - cx) / rx;
            let ny = (y - cy) / ry;
            ((nx * nx + ny * ny).sqrt() - 1.0) * rx.min(ry)
        });
    }

    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        if rect.is_empty() {
            return;
        }
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0,
        );
        let (hw, hh) = (rect.w as f32 / 2.0, rect.h as f32 / 2.0);
        let r = (radius as f32).min(hw).min(hh).max(0.0);
        self.coverage_fill(rect, color, move |x, y| {
            round_rect_sdf(x, y, cx, cy, hw, hh, r)
        });
    }

    fn fill_round_rect_alpha(&mut self, rect: Rect, radius: i32, color: Color, alpha: f32) {
        if rect.is_empty() {
            return;
        }
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0,
        );
        let (hw, hh) = (rect.w as f32 / 2.0, rect.h as f32 / 2.0);
        let r = (radius as f32).min(hw).min(hh).max(0.0);
        self.coverage_fill_alpha(rect, color, alpha, move |x, y| {
            round_rect_sdf(x, y, cx, cy, hw, hh, r)
        });
    }

    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, color: Color, width: f32) {
        if rect.is_empty() {
            return;
        }
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0,
        );
        let (hw, hh) = (rect.w as f32 / 2.0, rect.h as f32 / 2.0);
        let r = (radius as f32).min(hw).min(hh).max(0.0);
        let half_w = width / 2.0;
        // 외곽선은 경계 밖 half_w까지 나간다 — 순회 영역을 넓힌다.
        let pad = half_w.ceil() as i32 + 1;
        let area = Rect::new(
            rect.x - pad,
            rect.y - pad,
            rect.w + pad * 2,
            rect.h + pad * 2,
        );
        self.coverage_fill(area, color, move |x, y| {
            round_rect_sdf(x, y, cx, cy, hw, hh, r).abs() - half_w
        });
    }

    fn stroke_round_rect_alpha(
        &mut self,
        rect: Rect,
        radius: i32,
        color: Color,
        width: f32,
        alpha: f32,
    ) {
        if rect.is_empty() {
            return;
        }
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0,
        );
        let (hw, hh) = (rect.w as f32 / 2.0, rect.h as f32 / 2.0);
        let r = (radius as f32).min(hw).min(hh).max(0.0);
        let half_w = width / 2.0;
        let pad = half_w.ceil() as i32 + 1;
        let area = Rect::new(
            rect.x - pad,
            rect.y - pad,
            rect.w + pad * 2,
            rect.h + pad * 2,
        );
        self.coverage_fill_alpha(area, color, alpha, move |x, y| {
            round_rect_sdf(x, y, cx, cy, hw, hh, r).abs() - half_w
        });
    }

    fn polyline(&mut self, pts: &[(i32, i32)], color: Color, width: f32) {
        if pts.len() < 2 {
            return;
        }
        let half_w = width / 2.0;
        for seg in pts.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let pad = half_w.ceil() as i32 + 1;
            let x0 = a.0.min(b.0) - pad;
            let y0 = a.1.min(b.1) - pad;
            let area = Rect::new(x0, y0, (a.0.max(b.0) + pad) - x0, (a.1.max(b.1) + pad) - y0);
            let (ax, ay, bx, by) = (a.0 as f32, a.1 as f32, b.0 as f32, b.1 as f32);
            self.coverage_fill(area, color, move |x, y| {
                seg_dist(x, y, ax, ay, bx, by) - half_w
            });
        }
    }
}
