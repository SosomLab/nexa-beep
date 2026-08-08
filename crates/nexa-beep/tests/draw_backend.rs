//! RasterCtx(DrawCtx 백엔드) 통합 — 위젯 어휘가 실제 픽셀로 닫히는지(M3-1 슬라이스 1).

use nbeep_gfx::{Font, Surface};
use nbeep_ui::{Color, DrawCtx, RasterCtx, Rect};

fn font() -> Font {
    let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트");
    Font::from_static(data, index).expect("폰트 파싱")
}

fn px(buf: &[u32], w: usize, x: usize, y: usize) -> u32 {
    buf[y * w + x]
}

#[test]
fn round_rect_fills_center_and_rounds_corners() {
    let f = font();
    let mut buf = vec![0u32; 40 * 40];
    {
        let mut s = Surface::new(&mut buf, 40, 40);
        let mut ctx = RasterCtx::new(&mut s, &f);
        ctx.fill_round_rect(Rect::new(4, 4, 32, 32), 10, Color(0x00FF_FFFF));
    }
    assert_eq!(px(&buf, 40, 20, 20), 0x00FF_FFFF, "중앙 불투명");
    assert_eq!(px(&buf, 40, 5, 5), 0, "코너는 라운드로 비어 있다");
    assert_eq!(px(&buf, 40, 20, 5), 0x00FF_FFFF, "변 중앙은 채워진다");
}

#[test]
fn stroke_leaves_interior_empty() {
    let f = font();
    let mut buf = vec![0u32; 40 * 40];
    {
        let mut s = Surface::new(&mut buf, 40, 40);
        let mut ctx = RasterCtx::new(&mut s, &f);
        ctx.stroke_round_rect(Rect::new(4, 4, 32, 32), 6, Color(0x00FF_FFFF), 2.0);
    }
    assert_eq!(px(&buf, 40, 20, 20), 0, "내부 비어 있음");
    assert!(
        px(&buf, 40, 20, 4) > 0 || px(&buf, 40, 20, 5) > 0,
        "테두리 픽셀 존재"
    );
}

#[test]
fn polyline_draws_check_mark() {
    let f = font();
    let mut buf = vec![0u32; 30 * 30];
    {
        let mut s = Surface::new(&mut buf, 30, 30);
        let mut ctx = RasterCtx::new(&mut s, &f);
        ctx.polyline(&[(5, 15), (12, 22), (25, 7)], Color(0x00FF_FFFF), 2.0);
    }
    let lit = buf.iter().filter(|&&p| p > 0).count();
    assert!(lit > 20, "✓ 스트로크 픽셀: {lit}");
    assert_eq!(px(&buf, 30, 27, 27), 0, "바깥은 깨끗");
}

#[test]
fn text_opaque_clips_to_row_rect() {
    let f = font();
    let mut buf = vec![0u32; 60 * 30];
    {
        let mut s = Surface::new(&mut buf, 60, 30);
        let mut ctx = RasterCtx::new(&mut s, &f);
        // 행 rect는 (0,8)~(40,24) — 텍스트가 길어도 rect 밖은 배경 그대로.
        ctx.text_opaque(
            2,
            10,
            Rect::new(0, 8, 40, 16),
            "WWWWWWWWWWWW",
            Color(0x00FF_FFFF),
            Color(0x0020_2020),
        );
    }
    assert_eq!(px(&buf, 60, 50, 15), 0, "클립 오른쪽 밖은 미접촉");
    assert_eq!(px(&buf, 60, 39, 9), 0x0020_2020, "클립 안 배경 채움");
    assert_eq!(px(&buf, 60, 10, 2), 0, "클립 위쪽 밖 미접촉");
}
