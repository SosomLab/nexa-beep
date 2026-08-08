//! 텍스트 스택 통합(SP-1c 채택 검증) — 시스템 폰트 로드 → gfx 래스터가 3-OS CI에서 돈다.
//!
//! 조립 지점(bin)의 통합 테스트인 이유: `gfx`는 파일을 못 읽고(플랫폼 중립) `plat`은 gfx를
//! 모른다 — 둘을 잇는 곳은 조립 지점뿐이다(docs/13 §2-2).

use nbeep_gfx::{Color, Font, Surface};

fn load_font() -> Font {
    let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트");
    Font::from_static(data, index).expect("폰트 파싱")
}

fn coverage(text: &str) -> u64 {
    let font = load_font();
    let mut buf = vec![0u32; 200 * 40];
    let mut s = Surface::new(&mut buf, 200, 40);
    font.draw_text(&mut s, 2.0, 28.0, 16.0, Color(0x00FF_FFFF), text);
    buf.iter().map(|&p| u64::from(p & 0xFF)).sum()
}

#[test]
fn ascii_text_rasterizes_on_all_ci_targets() {
    // 3-OS 러너 전부에서 라틴 폰트는 존재한다(docs/18 — plat 후보 목록의 폴백).
    assert!(coverage("Nexa Beep 123") > 0, "ASCII 래스터 커버리지");
}

#[test]
fn measure_matches_draw_direction() {
    let font = load_font();
    let w = font.measure("Beep", 16.0);
    assert!(w > 10.0 && w < 100.0, "실측 폭이 상식 범위: {w}");
    assert!(font.line_height(16.0) > 10.0);
}

// 한글 커버리지는 CJK 폰트가 보장되는 타깃에서만 단언한다(ubuntu CI 러너엔 CJK가 없다).
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn korean_text_rasterizes_where_cjk_font_guaranteed() {
    let font = load_font();
    assert!(font.covers('한'), "시스템 한글 폰트 커버리지");
    assert!(coverage("한글 렌더링") > 0, "한글 래스터 커버리지");
}
