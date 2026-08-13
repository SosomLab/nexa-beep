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
/// 슬롯별 글꼴 **페이스** 묶음 — 없는 슬롯은 [`FontSet::base`]로 폴백한다.
///
/// 크기·굵기는 [`FontPrefs`]가, **얼굴은 여기가** 정한다. 고정폭 슬롯([`FontSet::mono`])은
/// 숫자 폭이 변해 화면이 떨리는 것을 막는 자리다(사용자 요청 08-09).
#[allow(missing_debug_implementations)]
pub struct FontSet<'f> {
    /// 기본 얼굴(항상 있어야 한다 — 나머지의 폴백).
    pub base: &'f Font,
    /// 사용자 목록.
    pub peerlist: Option<&'f Font>,
    /// 대화 본문.
    pub message: Option<&'f Font>,
    /// 상태바.
    pub status: Option<&'f Font>,
    /// 고정폭(시각·수치 표시).
    pub mono: Option<&'f Font>,
}

impl<'f> FontSet<'f> {
    /// 기본 얼굴 하나로 만든다(전 슬롯 동일).
    #[must_use]
    pub fn single(base: &'f Font) -> Self {
        Self {
            base,
            peerlist: None,
            message: None,
            status: None,
            mono: None,
        }
    }

    /// 슬롯의 얼굴(없으면 기본).
    #[must_use]
    pub fn face(&self, slot: FontSlot) -> &'f Font {
        match slot {
            FontSlot::Base => Some(self.base),
            FontSlot::PeerList => self.peerlist,
            FontSlot::Message => self.message,
            FontSlot::Status => self.status,
            FontSlot::Mono => self.mono,
        }
        .unwrap_or(self.base)
    }
}

#[allow(missing_debug_implementations)]
pub struct RasterCtx<'s, 'b, 'f> {
    surface: &'s mut Surface<'b>,
    fonts: FontSet<'f>,
    /// 현재 슬롯의 얼굴(select_font가 갱신).
    font: &'f Font,
    /// 영역별 글꼴 설정(사용자 설정).
    prefs: FontPrefs,
    /// 현재 선택된 슬롯 글꼴(select_font로 전환).
    cur: SlotFont,
    /// 배율(고DPI — FR-U-6). 크기에 곱한다. 좌표는 이미 물리 px(호출자 몫).
    scale: f32,
    /// 광학 크기 보정(고정폭 전용 · 08-10) — 같은 px에서 숫자가 차지하는 높이를
    /// 기본 얼굴과 일치시키는 배수(Consolas는 숫자가 커서 <1.0). 다른 슬롯은 1.0.
    mono_mult: f32,
    /// 이번 프레임의 캐럿 표시 위상(08-13 — 깜빡임). 호스트가 시각·포커스로 주입.
    caret_on: bool,
}

impl<'s, 'b, 'f> RasterCtx<'s, 'b, 'f> {
    /// 표면과 폰트로 컨텍스트를 만든다(기본 슬롯 = Base).
    pub fn new(surface: &'s mut Surface<'b>, font: &'f Font) -> Self {
        Self::with_font_set(surface, FontSet::single(font))
    }

    /// 슬롯별 얼굴을 가진 컨텍스트(기본 슬롯 = Base).
    pub fn with_font_set(surface: &'s mut Surface<'b>, fonts: FontSet<'f>) -> Self {
        let prefs = FontPrefs::default();
        let font = fonts.base;
        Self {
            surface,
            fonts,
            font,
            prefs,
            cur: prefs.base,
            scale: 1.0,
            mono_mult: 1.0,
            caret_on: true,
        }
    }

    /// 이번 프레임의 캐럿 표시 위상 지정(깜빡임 — 호스트가 시각·포커스로 계산).
    #[must_use]
    pub fn with_caret_on(mut self, on: bool) -> Self {
        self.caret_on = on;
        self
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

/// 텍스트를 (폴백 필요 여부, 연속 구간)으로 나눈다 — 글리프 폴백용(08-10).
/// 공백은 앞 구간에 붙인다(구간 난립 방지 — 공백 폭 차이는 미세해 무시).
fn split_runs(text: &str, has: impl Fn(char) -> bool) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        if c == ' ' {
            if let Some((_, run)) = out.last_mut() {
                run.push(c);
                continue;
            }
        }
        let fb = !has(c);
        match out.last_mut() {
            Some((lf, run)) if *lf == fb => run.push(c),
            _ => out.push((fb, c.to_string())),
        }
    }
    out
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
    fn caret_on(&self) -> bool {
        self.caret_on
    }

    fn select_font(&mut self, slot: FontSlot, bold: bool) {
        self.cur = match slot {
            FontSlot::Base => self.prefs.base,
            FontSlot::PeerList => self.prefs.peerlist,
            FontSlot::Message => self.prefs.message,
            FontSlot::Status => self.prefs.status,
            // 고정폭은 별도 크기 설정이 없다(얼굴만 지정). 크기는 **Status를 따른다** —
            // 쓰이는 곳이 보조 정보 줄(설명문 13px 사이)이라 Base(16px)면 혼자 커 보인다
            // (사용자 지적 08-10 — Consolas 밀도까지 겹쳐 차이가 도드라졌다).
            FontSlot::Mono => self.prefs.status,
        };
        self.font = self.fonts.face(slot);
        // 광학 크기 보정 — 같은 px라도 고정폭 숫자는 본문보다 커 보인다(사용자 지적
        // 08-10 2차: px를 맞춰도 차이가 심하다). 숫자 '0'의 실측 외곽 높이 비로 맞춘다.
        self.mono_mult = if matches!(slot, FontSlot::Mono)
            && !core::ptr::eq(self.font as *const Font, self.fonts.base as *const Font)
        {
            (self.fonts.base.digit_height(100.0) / self.font.digit_height(100.0)).clamp(0.75, 1.15)
        } else {
            1.0
        };
        // 인자 bold는 슬롯 설정 위 강제 볼드(예: 강조 라벨).
        self.cur.bold |= bold;
    }

    fn select_font_sized(&mut self, slot: FontSlot, bold: bool, delta_px: f32) {
        self.select_font(slot, bold);
        // 증분은 슬롯 크기 **위에** 얹는다 — 사용자가 글꼴 크기를 키우면 제목도 같이 큰다.
        self.cur.size = (self.cur.size + delta_px).max(1.0);
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

    fn fill_triangle(&mut self, a: (i32, i32), b: (i32, i32), c: (i32, i32), color: Color) {
        let (a, b, c) = (
            (a.0 as f32, a.1 as f32),
            (b.0 as f32, b.1 as f32),
            (c.0 as f32, c.1 as f32),
        );
        // 세 반평면의 부호 거리 — 무게중심에서 부호를 재 감김 방향을 정규화한다.
        let edge = |p: (f32, f32), q: (f32, f32), x: f32, y: f32| {
            let (ex, ey) = (q.0 - p.0, q.1 - p.1);
            let len = (ex * ex + ey * ey).sqrt().max(f32::EPSILON);
            ((x - p.0) * ey - (y - p.1) * ex) / len
        };
        let cx = (a.0 + b.0 + c.0) / 3.0;
        let cy = (a.1 + b.1 + c.1) / 3.0;
        let s = if edge(a, b, cx, cy) < 0.0 { -1.0 } else { 1.0 };
        let rect = Rect::new(
            a.0.min(b.0).min(c.0).floor() as i32 - 1,
            a.1.min(b.1).min(c.1).floor() as i32 - 1,
            (a.0.max(b.0).max(c.0) - a.0.min(b.0).min(c.0)).ceil() as i32 + 2,
            (a.1.max(b.1).max(c.1) - a.1.min(b.1).min(c.1)).ceil() as i32 + 2,
        );
        // 내부 = 세 거리 모두 양수(정규화 후) → dist = -(최솟값): 내부 음수·외부 양수.
        self.coverage_fill(rect, color, move |x, y| {
            -(s * edge(a, b, x, y))
                .min(s * edge(b, c, x, y))
                .min(s * edge(c, a, x, y))
        });
    }

    fn text(&mut self, x: i32, y: i32, clip: Rect, text: &str, fg: Color) {
        let size = self.px_size();
        // 광학 보정 — 슬롯 얼굴(고정폭)은 보정 크기로, 폴백(기본 얼굴)은 명목 크기로.
        let own = size * self.mono_mult;
        // 슬롯 얼굴에 없는 글자는 **기본 얼굴로 폴백**(08-10 — 고정폭 Consolas에 한글이
        // 없어 두부(□)가 나던 문제. 베이스라인은 공유 = 줄이 안 흔들린다).
        if core::ptr::eq(self.font, self.fonts.base) || text.chars().all(|c| self.font.has_glyph(c))
        {
            let baseline = y as f32 + self.font.ascent(own);
            self.font.draw_styled(
                self.surface,
                x as f32,
                baseline,
                own,
                fg,
                text,
                Self::clip_of(clip),
                self.cur_style(),
            );
            return;
        }
        let baseline = y as f32 + self.font.ascent(own).max(self.fonts.base.ascent(size));
        let mut fx = x as f32;
        for (fallback, run) in split_runs(text, |c| self.font.has_glyph(c)) {
            let (f, s): (&Font, f32) = if fallback {
                (self.fonts.base, size)
            } else {
                (self.font, own)
            };
            f.draw_styled(
                self.surface,
                fx,
                baseline,
                s,
                fg,
                &run,
                Self::clip_of(clip),
                self.cur_style(),
            );
            fx += f.measure(&run, s);
        }
    }

    fn text_width(&mut self, text: &str) -> i32 {
        let size = self.px_size();
        let own = size * self.mono_mult;
        if core::ptr::eq(self.font, self.fonts.base) || text.chars().all(|c| self.font.has_glyph(c))
        {
            return self.font.measure(text, own).ceil() as i32;
        }
        split_runs(text, |c| self.font.has_glyph(c))
            .into_iter()
            .map(|(fallback, run)| {
                if fallback {
                    self.fonts.base.measure(&run, size)
                } else {
                    self.font.measure(&run, own)
                }
            })
            .sum::<f32>()
            .ceil() as i32
    }

    fn text_height(&mut self) -> i32 {
        if (self.mono_mult - 1.0).abs() > f32::EPSILON {
            // 고정폭 줄엔 한글 폴백(기본 얼굴·명목 크기)이 섞인다 — 그 기준으로 센터링.
            return self.fonts.base.text_box_height(self.px_size()).ceil() as i32;
        }
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

    fn stroke_ellipse(&mut self, rect: Rect, color: Color, width: f32) {
        if rect.is_empty() || width <= 0.0 {
            return;
        }
        let (cx, cy) = (
            rect.x as f32 + rect.w as f32 / 2.0,
            rect.y as f32 + rect.h as f32 / 2.0,
        );
        let (rx, ry) = (rect.w as f32 / 2.0, rect.h as f32 / 2.0);
        self.coverage_fill(rect, color, move |x, y| {
            let nx = (x - cx) / rx;
            let ny = (y - cy) / ry;
            let d = ((nx * nx + ny * ny).sqrt() - 1.0) * rx.min(ry); // 음수 = 안쪽
            // 가장자리 안쪽 width 밴드(-width..0) — 링 SDF.
            (d + width / 2.0).abs() - width / 2.0
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
