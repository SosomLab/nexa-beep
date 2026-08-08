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
}

#[cfg(test)]
mod tests {
    use super::*;

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
