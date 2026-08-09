//! 드로잉 어휘 — 위젯이 그리는 최소 인터페이스([docs/14 §2]).
//!
//! `nexa-dir2/crates/nexa-gui/src/draw.rs` 이식([docs/12 §A]) — **이미 백엔드 교체를 전제로
//! 검증된 추상**이다(원본은 GDI/DirectWrite, 우리는 CPU 래스터라이저 [`crate::raster`]).
//! dir2 전용 어휘(터미널 셀·아이콘·이미지)는 제외 — 이미지는 M4에서 `imgdec` 격리 경유로
//! 별도 설계(FR-S-12), 아이콘은 위젯 셋과 함께.
//!
//! 규약: **래스터 호출은 구현체에만 존재** — 위젯·컨트롤은 이 인터페이스만 쓴다(DR-21의 UI판).

use crate::geom::Rect;
use crate::theme::Color;

/// 폰트 슬롯 — 위젯이 페인트 시작에 자신의 슬롯을 선택한다(상태 공유 · 순서 무관 보장).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FontSlot {
    /// 기본 UI(메뉴·버튼·설정).
    #[default]
    Base,
    /// 사용자(피어) 목록.
    PeerList,
    /// 대화 본문.
    Message,
    /// 상태바·보조.
    Status,
    /// **고정폭** — 시각·수치처럼 폭이 흔들리면 안 되는 표시(크기는 Base와 공유).
    Mono,
}

/// 위젯의 그리기 어휘. 기본 구현이 있는 메서드는 백엔드가 미구현해도 된다(테스트 백엔드).
pub trait DrawCtx {
    /// 폰트 슬롯/장식 선택 — 이후의 `text*`/`text_width`에 적용. 기본 = no-op(단일 폰트 백엔드).
    fn select_font(&mut self, slot: FontSlot, bold: bool) {
        let _ = (slot, bold);
    }

    /// rect를 단색으로 불투명하게 채운다.
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// `clip`을 `bg`로 채우면서 텍스트를 `(x, y)`(왼쪽 위)에 그린다 — 행 배경+텍스트 1회 호출
    /// (원본 GDI `ETO_OPAQUE` 모델의 실증을 계승). `clip` 초과분은 잘린다.
    fn text_opaque(&mut self, x: i32, y: i32, clip: Rect, text: &str, fg: Color, bg: Color);

    /// 배경 없이 텍스트만 — 선택 하이라이트 위 겹쳐 그리기. 1회 호출(경계 이음새 방지).
    fn text(&mut self, x: i32, y: i32, clip: Rect, text: &str, fg: Color);

    /// 텍스트 렌더 폭(px) — 우측 정렬·라벨 실측 정렬용.
    fn text_width(&mut self, text: &str) -> i32;

    /// 현재 글꼴의 텍스트 상자 높이(px · 어센트+디센트) — 세로 중앙 정렬 실측용.
    /// 기본 = 16(레거시 근사) — 실제 렌더러는 폰트 메트릭으로 오버라이드.
    fn text_height(&mut self) -> i32 {
        16
    }

    /// RGBA 이미지 아이콘을 `(x, y)`(좌상단)에 알파 블렌드 — `clip` 밖은 잘린다. 기본 = no-op.
    fn image(&mut self, x: i32, y: i32, img: &crate::theme::IconImage, clip: Rect) {
        let _ = (x, y, img, clip);
    }

    /// RGBA 이미지를 `dst`로 **스케일**해 블렌드(큰 이미지 축소·이미지 버튼) — `clip` 밖은 잘린다.
    /// 기본 = no-op.
    fn image_scaled(&mut self, dst: Rect, img: &crate::theme::IconImage, clip: Rect) {
        let _ = (dst, img, clip);
    }

    /// 원/타원 AA 채움. 기본 = no-op.
    fn fill_ellipse(&mut self, rect: Rect, color: Color) {
        let _ = (rect, color);
    }

    /// 라운드 사각형 AA 채움. 기본 = no-op.
    fn fill_round_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        let _ = (rect, radius, color);
    }

    /// 라운드 사각형 AA 채움 + **불투명도**(`alpha` 0..=1 — 반투명 스크롤바 등).
    /// 기본 = 알파 무시하고 [`Self::fill_round_rect`] 위임(테스트 백엔드).
    fn fill_round_rect_alpha(&mut self, rect: Rect, radius: i32, color: Color, alpha: f32) {
        let _ = alpha;
        self.fill_round_rect(rect, radius, color);
    }

    /// 라운드 사각형 AA 외곽선(폭 `width`px). 기본 = no-op.
    fn stroke_round_rect(&mut self, rect: Rect, radius: i32, color: Color, width: f32) {
        let _ = (rect, radius, color, width);
    }

    /// 라운드 사각형 AA 외곽선 + **불투명도**(`alpha` 0..=1 — 포커스 링 반투명 테두리).
    /// 기본 = 알파 무시하고 [`Self::stroke_round_rect`] 위임(테스트 백엔드).
    fn stroke_round_rect_alpha(
        &mut self,
        rect: Rect,
        radius: i32,
        color: Color,
        width: f32,
        alpha: f32,
    ) {
        let _ = alpha;
        self.stroke_round_rect(rect, radius, color, width);
    }

    /// 꺾은선(✓·셰브론 등) — 둥근 캡, 폭 `width`px AA. 기본 = no-op.
    fn polyline(&mut self, pts: &[(i32, i32)], color: Color, width: f32) {
        let _ = (pts, color, width);
    }
}
