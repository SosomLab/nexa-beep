//! 세션 배지 실루엣(M3-19 · [14 §12]) — 상태별 파냄 픽셀 + 배율 비율 불변 + 스위치 off.
//!
//! 색만으로 4상태를 나르던 배지가 실루엣(빈 링·갭 링·찬 원·막대)으로도 갈리는지
//! **실제 픽셀**로 단언한다. 기하가 지름 비율 고정이라 D=12·D=22 두 배율에서
//! 같은 판정이 나와야 한다.

use nbeep_gfx::{Font, Surface};
use nbeep_ui::{draw_link_badge, LinkState, Rect, Theme};

fn font() -> Font {
    let (data, index) = nbeep_plat::font::system_ui_font().expect("시스템 UI 폰트");
    Font::from_static(data, index).expect("폰트 파싱")
}

/// 32×32 검정 버퍼에 배지 하나를 그려 돌려준다.
fn render(link: LinkState, shape: bool, spin: u8, dot: Rect) -> Vec<u32> {
    let f = font();
    let th = Theme::dark();
    let mut buf = vec![0u32; 32 * 32];
    {
        let mut s = Surface::new(&mut buf, 32, 32);
        let mut ctx = nbeep_ui::RasterCtx::new(&mut s, &f);
        draw_link_badge(&mut ctx, dot, &th, link, false, shape, spin);
    }
    buf
}

fn px(buf: &[u32], x: usize, y: usize) -> u32 {
    buf[y * 32 + x]
}

/// D=12 표준 배지 — 중심 (16,16).
const DOT: Rect = Rect {
    x: 10,
    y: 10,
    w: 12,
    h: 12,
};

#[test]
fn idle_is_empty_ring() {
    let th = Theme::dark();
    let buf = render(LinkState::Idle, true, 0, DOT);
    // 구멍(반경 3 안) = 파낸 panel_bg · 링 밴드(반경 5) = 상태 색.
    assert_eq!(px(&buf, 16, 16), th.panel_bg.0, "중심은 파냈다");
    assert_eq!(px(&buf, 16, 11), th.text_dim.0, "링 밴드는 상태 색");
}

#[test]
fn active_is_filled_disc() {
    let th = Theme::dark();
    let buf = render(LinkState::Active, true, 0, DOT);
    assert_eq!(px(&buf, 16, 16), th.ok.0, "중심까지 꽉 찬 원(현행 유지)");
}

#[test]
fn lost_has_center_bar_knockout() {
    let th = Theme::dark();
    let buf = render(LinkState::Lost, true, 0, DOT);
    assert_eq!(px(&buf, 16, 16), th.panel_bg.0, "가로 막대 = 파냄");
    assert_eq!(px(&buf, 16, 11), th.danger.0, "막대 위 디스크는 상태 색");
}

#[test]
fn connecting_gap_rotates_with_spin() {
    let th = Theme::dark();
    // spin 0 = 12시부터 시계로 90°(우상단 사분면)가 갭.
    let buf = render(LinkState::Connecting, true, 0, DOT);
    assert_eq!(px(&buf, 19, 13), th.panel_bg.0, "우상단 갭은 파냈다");
    assert_eq!(px(&buf, 13, 19), th.accent.0, "반대편 밴드는 상태 색");
    assert_eq!(px(&buf, 16, 16), th.panel_bg.0, "구멍도 파냈다(링이다)");
    // spin 1 = 갭이 우하단으로 90° 회전.
    let buf = render(LinkState::Connecting, true, 1, DOT);
    assert_eq!(px(&buf, 19, 19), th.panel_bg.0, "갭이 우하단으로 이동");
    assert_eq!(px(&buf, 19, 13), th.accent.0, "우상단은 다시 밴드");
}

#[test]
fn ratio_holds_at_double_scale() {
    // D=22(중심 동일 16,16) — 구멍 반경 6 · 밴드 6..11: 비율 고정이면 같은 판정.
    let th = Theme::dark();
    let dot = Rect {
        x: 5,
        y: 5,
        w: 22,
        h: 22,
    };
    let buf = render(LinkState::Idle, true, 0, dot);
    assert_eq!(px(&buf, 16, 12), th.panel_bg.0, "반경 4 = 구멍(<6)");
    assert_eq!(px(&buf, 16, 8), th.text_dim.0, "반경 8 = 밴드(6..11)");
    let buf = render(LinkState::Lost, true, 0, dot);
    assert_eq!(px(&buf, 16, 16), th.panel_bg.0, "막대 파냄은 배율 무관");
    assert_eq!(px(&buf, 16, 9), th.danger.0, "막대 밖 디스크");
}

#[test]
fn switch_off_restores_plain_disc() {
    let th = Theme::dark();
    for (link, color) in [
        (LinkState::Idle, th.text_dim.0),
        (LinkState::Connecting, th.accent.0),
        (LinkState::Lost, th.danger.0),
        (LinkState::Active, th.ok.0),
    ] {
        let buf = render(link, false, 0, DOT);
        assert_eq!(px(&buf, 16, 16), color, "off = 종전 채운 원({link:?})");
    }
}
