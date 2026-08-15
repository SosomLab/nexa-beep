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
    /// 이번 프레임에 **캐럿을 그릴 것인가**(깜빡임 위상 · 08-13 사용자 요청).
    /// 위젯은 시계가 없으므로 호스트가 프레임마다 위상을 주입한다 — 포커스 창이
    /// 아니거나 어두운 위상이면 `false`. 기본 = 항상 표시(테스트 백엔드·정적 렌더).
    fn caret_on(&self) -> bool {
        true
    }
    /// 폰트 슬롯/장식 선택 — 이후의 `text*`/`text_width`에 적용. 기본 = no-op(단일 폰트 백엔드).
    fn select_font(&mut self, slot: FontSlot, bold: bool) {
        let _ = (slot, bold);
    }

    /// 슬롯 선택 + **크기 증분**(논리 px) — 제목처럼 "본문보다 조금 크게"를 표현할 때.
    /// 절대 크기를 박으면 사용자가 글꼴 크기를 바꿔도 제목만 그대로 남아 위계가 깨진다.
    /// 기본 = 증분 무시(단일 폰트 백엔드).
    fn select_font_sized(&mut self, slot: FontSlot, bold: bool, delta_px: f32) {
        let _ = delta_px;
        self.select_font(slot, bold);
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

    /// 문자 경계 **누적 폭**(08-14 성능) — `out[i]` = 앞 `i`글자 접두사의
    /// [`text_width`](Self::text_width)와 **동일 값**(0 포함 · 길이 = 문자수+1).
    /// 캐럿·선택 좌표의 원천이라 **값 동일이 계약**이다. 기본 구현 = 접두사
    /// 재측정(O(n²) — 종전 호출부 로직 그대로) · 렌더러는 단일 패스(O(n))로
    /// 오버라이드한다(매 페인트 실측이라 캐럿 깜빡임 상시 리페인트에서 비용이 컸다).
    fn text_prefix_widths(&mut self, text: &str, out: &mut Vec<i32>) {
        out.clear();
        out.push(0);
        let mut acc = String::new();
        for c in text.chars() {
            acc.push(c);
            out.push(self.text_width(&acc));
        }
    }

    /// 현재 글꼴의 텍스트 상자 높이(px · 어센트+디센트) — 세로 중앙 정렬 실측용.
    /// 기본 = 16(레거시 근사) — 실제 렌더러는 폰트 메트릭으로 오버라이드.
    fn text_height(&mut self) -> i32 {
        16
    }

    /// 삼각형을 단색 AA로 채운다(말풍선 꼬리 등 — 08-10). 기본 = no-op.
    fn fill_triangle(&mut self, a: (i32, i32), b: (i32, i32), c: (i32, i32), color: Color) {
        let _ = (a, b, c, color);
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

    /// 타원 **테두리 링** AA(08-14 — 아바타 보더). `width`px 밴드를 가장자리 **안쪽**에
    /// 그린다(rect 밖으로 나가지 않는다). 기본 = no-op.
    fn stroke_ellipse(&mut self, rect: Rect, color: Color, width: f32) {
        let _ = (rect, color, width);
    }

    /// 부채꼴(파이) AA 채움 — `rect` 내접 타원에서 **12시 = 0° · 시계 방향**으로
    /// `start_deg`부터 `sweep_deg`만큼. 반평면 2장의 교집합이라 **`sweep_deg` ≤ 180°만
    /// 보증**한다(M3-19 갭 링의 "파냄" 용도 — 그 이상이 필요하면 두 번 부른다). 기본 = no-op.
    fn fill_pie(&mut self, rect: Rect, start_deg: f32, sweep_deg: f32, color: Color) {
        let _ = (rect, start_deg, sweep_deg, color);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::theme::Color;

    /// 비선형 폭 목업 — 누적이 "선형 가정"을 하면 어긋나도록 폭 = 문자수²×3.
    struct Quirky;
    impl DrawCtx for Quirky {
        fn fill_rect(&mut self, _r: Rect, _c: Color) {}
        fn text_opaque(&mut self, _x: i32, _y: i32, _c: Rect, _t: &str, _f: Color, _b: Color) {}
        fn text(&mut self, _x: i32, _y: i32, _c: Rect, _t: &str, _f: Color) {}
        fn text_width(&mut self, text: &str) -> i32 {
            let n = i32::try_from(text.chars().count()).unwrap_or(i32::MAX);
            n * n * 3
        }
    }

    /// 계약(08-14): `out[i]` == `text_width(접두사 i)` · `out[0]` == 0 · 길이 = 문자수+1.
    /// 캐럿·선택 좌표의 원천이라 이 동일성이 곧 기능 불변의 근거다.
    #[test]
    fn prefix_widths_default_matches_prefix_text_width() {
        let mut ctx = Quirky;
        let text = "한a b글";
        let mut out = Vec::new();
        ctx.text_prefix_widths(text, &mut out);
        assert_eq!(out.len(), text.chars().count() + 1);
        assert_eq!(out[0], 0);
        for (i, w) in out.iter().enumerate() {
            let prefix: String = text.chars().take(i).collect();
            assert_eq!(*w, ctx.text_width(&prefix), "접두사 {i}");
        }
    }
}
