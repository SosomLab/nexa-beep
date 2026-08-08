//! 픽셀 표면 — CPU 래스터라이저의 캔버스(ADR-0001 B안).
//!
//! `0x00RR_GGBB` u32 버퍼를 빌려 그린다(softbuffer 픽셀 형식과 동일 — 변환 없이 present).
//! 모든 그리기는 **표면 경계로 클립**된다 — 밖을 찍는 코드는 존재할 수 없다(패닉 대신 무시가
//! 아니라, 좌표를 잘라 정확히 안쪽만 쓴다).

/// `0x00RR_GGBB` 색.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    /// 채널 분해.
    #[must_use]
    pub fn rgb(self) -> (u8, u8, u8) {
        (
            ((self.0 >> 16) & 0xFF) as u8,
            ((self.0 >> 8) & 0xFF) as u8,
            (self.0 & 0xFF) as u8,
        )
    }

    /// 채널 합성.
    #[must_use]
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Color((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
    }

    /// 두 색을 `t`(0..=1)로 선형 보간한다(포커스 링 밝게 섞기 등).
    #[must_use]
    pub fn lerp(self, other: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let (r0, g0, b0) = self.rgb();
        let (r1, g1, b1) = other.rgb();
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Color::from_rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1))
    }
}

/// RGBA 이미지 아이콘 — **투명 배경 지원**(straight alpha, `[R,G,B,A]` 행 우선).
///
/// 글꼴 글리프가 아니라 진짜 래스터 이미지다. PNG 등 파일은 상위 계층에서 디코드해
/// [`IconImage::from_rgba`]로 넘긴다(UI/gfx는 파일 포맷을 모른다). 데모용 생성자
/// [`IconImage::swatch`]는 투명 배경의 라운드 사각형 아이콘을 코드로 만든다.
#[derive(Clone)]
pub struct IconImage {
    /// 폭(px).
    pub w: u32,
    /// 높이(px).
    pub h: u32,
    /// `w*h*4` 길이의 RGBA(straight alpha).
    pub rgba: Vec<u8>,
}

impl core::fmt::Debug for IconImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IconImage")
            .field("w", &self.w)
            .field("h", &self.h)
            .finish_non_exhaustive()
    }
}

impl IconImage {
    /// RGBA 바이트로 만든다(예: PNG 디코더 출력).
    ///
    /// # Panics
    /// `rgba.len() != w*h*4`면 패닉(구성 오류).
    #[must_use]
    pub fn from_rgba(w: u32, h: u32, rgba: Vec<u8>) -> Self {
        assert_eq!(rgba.len(), (w * h * 4) as usize, "RGBA 길이 불일치");
        Self { w, h, rgba }
    }

    /// 데모용 — **투명 배경의 라운드 사각형** 아이콘(앱 아이콘 느낌). 모서리는 알파 0.
    #[must_use]
    pub fn swatch(size: u32, (r, g, b): (u8, u8, u8)) -> Self {
        let s = size as f32;
        let radius = s / 4.0;
        let (cx, cy) = (s / 2.0, s / 2.0);
        let hw = s / 2.0;
        let mut rgba = vec![0u8; (size * size * 4) as usize];
        for py in 0..size {
            for px in 0..size {
                let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
                // 라운드 사각형 SDF → 0.5px AA 커버리지.
                let qx = (x - cx).abs() - (hw - radius);
                let qy = (y - cy).abs() - (hw - radius);
                let (ax, ay) = (qx.max(0.0), qy.max(0.0));
                let d = (ax * ax + ay * ay).sqrt() + qx.max(qy).min(0.0) - radius;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                let i = ((py * size + px) * 4) as usize;
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = (cov * 255.0).round() as u8;
            }
        }
        Self {
            w: size,
            h: size,
            rgba,
        }
    }
}

/// 빌린 픽셀 버퍼 위의 그리기 표면.
// Debug 미파생 — 픽셀 버퍼 덤프는 로그 오염(폭·높이만 의미 있음).
#[allow(missing_debug_implementations)]
pub struct Surface<'a> {
    buf: &'a mut [u32],
    width: usize,
    height: usize,
}

impl<'a> Surface<'a> {
    /// `width * height` 길이의 버퍼를 감싼다.
    ///
    /// # Panics
    /// 버퍼 길이가 `width * height`보다 짧으면 패닉(구성 오류 — 조립 지점에서만 발생 가능).
    #[must_use]
    pub fn new(buf: &'a mut [u32], width: usize, height: usize) -> Self {
        assert!(buf.len() >= width * height, "버퍼가 크기보다 작다");
        Self { buf, width, height }
    }

    /// 표면 폭(px).
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// 표면 높이(px).
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// 전체를 한 색으로 채운다.
    pub fn fill(&mut self, color: Color) {
        self.buf[..self.width * self.height].fill(color.0);
    }

    /// 사각형 채우기 — 표면 밖은 잘린다(음수·초과 좌표 안전).
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = usize::try_from(i64::from(x) + i64::from(w))
            .unwrap_or(0)
            .min(self.width);
        let y1 = usize::try_from(i64::from(y) + i64::from(h))
            .unwrap_or(0)
            .min(self.height);
        for row in y0..y1 {
            self.buf[row * self.width + x0..row * self.width + x1].fill(color.0);
        }
    }

    /// 픽셀 하나를 커버리지(0.0~1.0)로 배경과 블렌드 — 글리프 안티에일리어싱의 기초.
    pub fn blend_px(&mut self, x: i32, y: i32, color: Color, coverage: f32) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.width || y >= self.height {
            return;
        }
        let a = coverage.clamp(0.0, 1.0);
        let idx = y * self.width + x;
        let bg = Color(self.buf[idx]).rgb();
        let fg = color.rgb();
        let mix = |b: u8, f: u8| -> u32 {
            let v = f32::from(b) * (1.0 - a) + f32::from(f) * a;
            // 0..=255 범위 내 — 반올림 후 안전 캐스팅.
            u32::from((v + 0.5) as u8)
        };
        self.buf[idx] = (mix(bg.0, fg.0) << 16) | (mix(bg.1, fg.1) << 8) | mix(bg.2, fg.2);
    }

    /// RGBA 이미지를 `dst`(x,y,w,h)로 **스케일**해 알파 블렌드한다(nearest 샘플링 · 큰 이미지 축소용).
    /// `clip`(반열림) 밖은 건너뛴다.
    #[allow(clippy::too_many_arguments)]
    pub fn blend_image_scaled(
        &mut self,
        dx: i32,
        dy: i32,
        dw: i32,
        dh: i32,
        img: &IconImage,
        clip: (i32, i32, i32, i32),
    ) {
        if dw <= 0 || dh <= 0 || img.w == 0 || img.h == 0 {
            return;
        }
        for row in 0..dh {
            for col in 0..dw {
                let (px, py) = (dx + col, dy + row);
                if px < clip.0 || px >= clip.2 || py < clip.1 || py >= clip.3 {
                    continue;
                }
                // 목적 픽셀 → 원본 픽셀(nearest).
                let sx = (col * img.w as i32 / dw).clamp(0, img.w as i32 - 1);
                let sy = (row * img.h as i32 / dh).clamp(0, img.h as i32 - 1);
                let i = ((sy * img.w as i32 + sx) * 4) as usize;
                let a = img.rgba[i + 3];
                if a == 0 {
                    continue;
                }
                let color = Color::from_rgb(img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]);
                self.blend_px(px, py, color, f32::from(a) / 255.0);
            }
        }
    }

    /// RGBA 이미지를 `(x, y)`(좌상단)에 알파 블렌드한다. `clip = (x0,y0,x1,y1)`(반열림) 밖은 건너뛴다.
    pub fn blend_image(&mut self, x: i32, y: i32, img: &IconImage, clip: (i32, i32, i32, i32)) {
        for row in 0..img.h as i32 {
            for col in 0..img.w as i32 {
                let (px, py) = (x + col, y + row);
                if px < clip.0 || px >= clip.2 || py < clip.1 || py >= clip.3 {
                    continue;
                }
                let i = ((row * img.w as i32 + col) * 4) as usize;
                let a = img.rgba[i + 3];
                if a == 0 {
                    continue;
                }
                let color = Color::from_rgb(img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]);
                self.blend_px(px, py, color, f32::from(a) / 255.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swatch_has_transparent_corners_and_opaque_center() {
        let img = IconImage::swatch(16, (255, 0, 0));
        assert_eq!(img.rgba.len(), 16 * 16 * 4);
        // 모서리(0,0)는 투명(라운드 밖), 중앙은 불투명.
        assert_eq!(img.rgba[3], 0, "모서리 알파 0(투명 배경)");
        let c = ((8 * 16 + 8) * 4) as usize;
        assert!(img.rgba[c + 3] > 250, "중앙 불투명");
        assert_eq!(img.rgba[c], 255, "색 반영");
    }

    #[test]
    fn blend_image_respects_clip_and_alpha() {
        let mut buf = vec![0u32; 4 * 4];
        // 2x1 이미지: 불투명 흰색 + 완전 투명.
        let img = IconImage::from_rgba(2, 1, vec![255, 255, 255, 255, 0, 0, 0, 0]);
        {
            let mut s = Surface::new(&mut buf, 4, 4);
            s.blend_image(0, 0, &img, (0, 0, 4, 4));
            s.blend_image(0, 2, &img, (0, 0, 4, 1)); // clip 높이 1 → row2는 밖
        }
        assert_eq!(buf[0], 0x00FF_FFFF, "불투명 픽셀 반영");
        assert_eq!(buf[1], 0, "투명 픽셀 무변");
        assert_eq!(buf[2 * 4], 0, "clip 밖 무변");
    }

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut buf = vec![0u32; 4 * 4];
        let mut s = Surface::new(&mut buf, 4, 4);
        // 표면을 벗어나는 사각형 — 안쪽만 칠해진다(패닉·OOB 없음).
        s.fill_rect(-2, -2, 4, 4, Color(0xFF0000));
        s.fill_rect(3, 3, 10, 10, Color(0x00FF00));
        assert_eq!(buf[0], 0xFF0000, "좌상 클립");
        assert_eq!(buf[4 + 1], 0xFF0000);
        assert_eq!(buf[2 * 4 + 2], 0, "중앙 미접촉");
        assert_eq!(buf[3 * 4 + 3], 0x00FF00, "우하 클립");
    }

    #[test]
    fn blend_px_mixes_and_ignores_out_of_bounds() {
        let mut buf = vec![0u32; 2 * 2];
        let mut s = Surface::new(&mut buf, 2, 2);
        s.blend_px(0, 0, Color(0xFFFFFF), 0.5);
        s.blend_px(-1, 0, Color(0xFFFFFF), 1.0); // 무시
        s.blend_px(5, 5, Color(0xFFFFFF), 1.0); // 무시
        let (r, g, b) = Color(buf[0]).rgb();
        assert!(r > 120 && r < 135, "절반 블렌드: {r}");
        assert_eq!((r, g, b), (r, r, r), "회색");
        assert_eq!(buf[1], 0);
    }

    #[test]
    fn full_coverage_is_opaque() {
        let mut buf = vec![0u32; 1];
        let mut s = Surface::new(&mut buf, 1, 1);
        s.blend_px(0, 0, Color(0x0012_3456), 1.0);
        assert_eq!(buf[0], 0x0012_3456);
    }
}
