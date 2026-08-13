//! 이니셜 아바타 — **가상 이미지**(사용자 확정 08-11): 표시 이름 앞 2글자를 원 안에
//! 벡터 렌더한다(비트맵 자산 없음 — SVG처럼 수식으로 그린다).
//!
//! 실제 사진 픽셀 렌더는 M4-5(imgdec 격리 디코드) 이후 — 수신 이미지를 본체에서
//! 디코드하지 않는다(R-5). 그때까지 프로필 이미지가 있어도 이니셜이 대신 선다.

use crate::draw::{DrawCtx, FontSlot};
use crate::geom::Rect;
use crate::theme::Color;

/// 배경 팔레트 — 시드(키 지문 등)로 안정 선택(같은 상대는 늘 같은 색).
const PALETTE: [Color; 8] = [
    Color(0x005E_81AC), // 청회
    Color(0x00A3_BE8C), // 녹
    Color(0x00D0_8770), // 주황
    Color(0x00B4_8EAD), // 보라
    Color(0x00BF_616A), // 적
    Color(0x00D9_A62E), // 황
    Color(0x0058_A6A6), // 청록
    Color(0x006C_7A96), // 회청
];

/// 이름에서 이니셜 추출 — 공백을 건너뛴 앞 2글자(한글·한자는 그대로 2자).
#[must_use]
pub fn initials(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .take(2)
        .collect()
}

/// 시드 → 팔레트 색(안정 해시).
#[must_use]
pub fn avatar_color(seed: &[u8]) -> Color {
    let idx = seed.iter().fold(0usize, |a, b| {
        a.wrapping_mul(31).wrapping_add(usize::from(*b))
    });
    PALETTE[idx % PALETTE.len()]
}

/// 원형 이니셜 아바타를 `rect`(정사각 권장)에 그린다.
///
/// `font_delta_logical` = 기본 글꼴 대비 크기 증분(논리 px) — 호출자가 아바타 지름에
/// 맞춰 준다(목록 소형 ≈ +6 · 프로필 대형 ≈ +34). 절대 크기를 박지 않는 증분 규약
/// (docs: `select_font_sized`)을 따른다.
pub fn draw_avatar(
    ctx: &mut dyn DrawCtx,
    rect: Rect,
    name: &str,
    seed: &[u8],
    font_delta_logical: f32,
) {
    ctx.fill_ellipse(rect, avatar_color(seed));
    let ini = initials(name);
    if ini.is_empty() {
        return;
    }
    ctx.select_font_sized(FontSlot::Base, true, font_delta_logical);
    let tw = ctx.text_width(&ini);
    let th = ctx.text_height();
    ctx.text(
        rect.x + (rect.w - tw) / 2,
        rect.y + (rect.h - th) / 2,
        rect,
        &ini,
        Color(0x00FF_FFFF),
    );
}

/// 내장(12간지) 아바타 — **시드 색 원 배경 + 투명 배경 일러스트**(08-14).
/// 그림은 mkavatars가 0.92 배율로 안착시켜 원 안에 들어온다. 원 배경을 까는 이유 =
/// 일러스트가 투명 배경이라 패널 위에 맨몸으로 뜨면 "아바타"가 아니라 "스티커"로
/// 보이고, 이니셜·사진(원형)과 시각 문법이 어긋난다.
pub fn draw_builtin(
    ctx: &mut dyn DrawCtx,
    rect: Rect,
    img: &crate::theme::IconImage,
    seed: &[u8],
) {
    ctx.fill_ellipse(rect, avatar_color(seed));
    ctx.image_scaled(rect, img, rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_skip_whitespace_and_cap_two() {
        assert_eq!(initials("홍 길동"), "홍길");
        assert_eq!(initials("bob"), "bo");
        assert_eq!(initials("★테스트단말"), "★테");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn color_is_stable_per_seed() {
        assert_eq!(avatar_color(b"abc").0, avatar_color(b"abc").0);
        // 같은 팔레트 안(공식 보증은 아님) — 최소한 패닉 없이 인덱싱.
        let _ = avatar_color(&[]);
    }
}
