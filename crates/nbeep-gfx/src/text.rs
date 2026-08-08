//! 텍스트 스택 최소 경로 — **ab_glyph**(SP-1c 실측 후 사용자 확정 08-08).
//!
//! 폰트 파싱(ttf-parser 계열)·글리프 래스터만 쓴다. **셰이핑 엔진은 v1에 없다** — 한글은
//! 완성형 음절이 cmap에 직접 있어 글리프 치환이 필요 없고(아랍어·인도계와 다른 점), v1 요구는
//! 한/영(FR-U-3)이다. 복잡 문자는 v2에서 이 모듈 뒤(DR-21 이음새)에 셰이핑을 추가한다.
//!
//! **폰트 바이트는 밖에서 온다** — 이 크레이트는 파일을 읽지 않는다(플랫폼 중립).
//! 시스템 폰트 경로 발견은 `nbeep-plat` 소관(ADR-0001 — 폰트 열거는 플랫폼 계층).

use crate::surface::{Color, Surface};
use ab_glyph::{Font as _, FontRef, ScaleFont as _};

/// 로드된 폰트 — **프로세스 수명 자원**(로드 1회 · 앱 종료까지 사용).
///
/// 바이트는 `&'static`이다 — `nbeep-plat`의 mmap(파일 백드 페이지 · 힙 0)이 정상 경로이고,
/// [`Font::from_bytes`]는 소유 바이트를 의도적으로 누수해 같은 표현으로 수렴한다(테스트·특수 경로용).
pub struct Font {
    inner: FontRef<'static>,
}

impl core::fmt::Debug for Font {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Font").finish_non_exhaustive()
    }
}

/// 폰트 로드 실패(파싱 불가·인덱스 없음).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontError;

/// 텍스트 스타일(faux 볼드·이탤릭).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TextStyle {
    /// 굵게(faux — 이중 그리기).
    pub bold: bool,
    /// 기울임(faux — 전단).
    pub italic: bool,
}

impl TextStyle {
    /// 스타일 없음.
    pub const PLAIN: Self = Self {
        bold: false,
        italic: false,
    };
}

impl Font {
    /// `'static` 폰트 바이트에서 로드한다(mmap 정상 경로). `index`는 TTC 컬렉션 인덱스.
    ///
    /// # Errors
    /// 파싱 불가·인덱스 범위 밖이면 [`FontError`].
    pub fn from_static(data: &'static [u8], index: u32) -> Result<Self, FontError> {
        FontRef::try_from_slice_and_index(data, index)
            .map(|inner| Self { inner })
            .map_err(|_| FontError)
    }

    /// 소유 바이트에서 로드 — **의도적 누수**로 `'static`화(폰트는 프로세스 수명 자원).
    ///
    /// # Errors
    /// 파싱 불가·인덱스 범위 밖이면 [`FontError`].
    pub fn from_bytes(data: Vec<u8>, index: u32) -> Result<Self, FontError> {
        Self::from_static(Box::leak(data.into_boxed_slice()), index)
    }

    /// 이 폰트가 문자의 글리프를 갖고 있는가(폴백 체인 판단 근거).
    #[must_use]
    pub fn covers(&self, ch: char) -> bool {
        self.inner.glyph_id(ch).0 != 0
    }

    /// `size`(px)에서의 줄 높이.
    #[must_use]
    pub fn line_height(&self, size: f32) -> f32 {
        let s = self.inner.as_scaled(size);
        s.ascent() - s.descent() + s.line_gap()
    }

    /// 텍스트 폭(px) — 그리지 않고 잰다(라벨 실측 정렬 — [docs/12 §B]).
    #[must_use]
    pub fn measure(&self, text: &str, size: f32) -> f32 {
        let s = self.inner.as_scaled(size);
        text.chars()
            .map(|c| s.h_advance(self.inner.glyph_id(c)))
            .sum()
    }

    /// `size`에서의 어센트(베이스라인 위 높이, px) — 상단 기준 배치를 베이스라인으로 변환.
    #[must_use]
    pub fn ascent(&self, size: f32) -> f32 {
        self.inner.as_scaled(size).ascent()
    }

    /// [`Font::draw_text`]의 클립 변형 — `clip = (x0, y0, x1, y1)` 밖 픽셀은 찍지 않는다
    /// (행 배경 안에서만 그리는 `text_opaque` 모델의 기초).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_clipped(
        &self,
        surface: &mut Surface<'_>,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
        text: &str,
        clip: (i32, i32, i32, i32),
    ) -> f32 {
        self.draw_styled(surface, x, y, size, color, text, clip, TextStyle::PLAIN)
    }

    /// [`Font::draw_text_clipped`]의 **스타일 변형** — 실제 볼드/이탤릭 폰트 파일 없이
    /// **faux 볼드**(x축 2회 그리기)·**faux 이탤릭**(베이스라인 위 거리 비례 전단)로 근사한다.
    /// 진짜 글꼴 패밀리·굵기 face는 폰트 열거(M3-3 확장)에서. 지금은 시스템 폰트 1벌 위 근사.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_styled(
        &self,
        surface: &mut Surface<'_>,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
        text: &str,
        clip: (i32, i32, i32, i32),
        style: TextStyle,
    ) -> f32 {
        let scaled = self.inner.as_scaled(size);
        let slant = if style.italic { 0.22 } else { 0.0 };
        let bold_pass = if style.bold { 2 } else { 1 };
        let mut pen = x;
        for ch in text.chars() {
            let gid = self.inner.glyph_id(ch);
            let glyph = gid.with_scale_and_position(size, ab_glyph::point(pen, y));
            if let Some(outlined) = scaled.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                if bounds.min.x as i32 >= clip.2 {
                    break;
                }
                let (ox, oy) = (bounds.min.x as i32, bounds.min.y as i32);
                outlined.draw(|gx, gy, cov| {
                    let py = oy + i32::try_from(gy).unwrap_or(i32::MAX);
                    // faux 이탤릭: 베이스라인 위로 갈수록 오른쪽으로 전단.
                    let shear = ((y - py as f32) * slant) as i32;
                    let base_px = ox + i32::try_from(gx).unwrap_or(i32::MAX) + shear;
                    for dx in 0..bold_pass {
                        let px = base_px + dx;
                        if px >= clip.0 && px < clip.2 && py >= clip.1 && py < clip.3 {
                            surface.blend_px(px, py, color, cov);
                        }
                    }
                });
            }
            pen += scaled.h_advance(gid);
        }
        pen - x
    }

    /// `(x, y)`를 **베이스라인 왼쪽 끝**으로 텍스트를 그린다. 그린 폭(px)을 돌려준다.
    ///
    /// 커버리지를 배경과 블렌드(안티에일리어싱). 표면 밖은 [`Surface`]가 클립한다.
    pub fn draw_text(
        &self,
        surface: &mut Surface<'_>,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
        text: &str,
    ) -> f32 {
        let scaled = self.inner.as_scaled(size);
        let mut pen = x;
        for ch in text.chars() {
            let gid = self.inner.glyph_id(ch);
            let glyph = gid.with_scale_and_position(size, ab_glyph::point(pen, y));
            if let Some(outlined) = scaled.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                let (ox, oy) = (bounds.min.x as i32, bounds.min.y as i32);
                outlined.draw(|gx, gy, cov| {
                    // 좌표 상한은 표면 클립이 보장 — i32 변환만 안전하게.
                    let px = ox + i32::try_from(gx).unwrap_or(i32::MAX);
                    let py = oy + i32::try_from(gy).unwrap_or(i32::MAX);
                    surface.blend_px(px, py, color, cov);
                });
            }
            pen += scaled.h_advance(gid);
        }
        pen - x
    }
}
