//! 시맨틱 테마 토큰 — `nexa-dir2/crates/nexa-gui/src/theme.rs`의 **토큰 체계** 이식([docs/12 §B]).
//!
//! **색 하드코딩 금지 — 전 위젯이 테마를 인자로 받는다.** 다크/라이트가 값 교체만으로 끝난다
//! (FR-U-5). 토큰 키는 안정 계약 — rename 시 마이그레이션 표를 남긴다.
//! ⚠️ 값은 임시 팔레트다 — macOS 시각 언어 수치표(M3-1c) 확정 시 값만 교체한다.

pub use nbeep_gfx::{Color, IconImage};

/// 시맨틱 색 토큰. 파일 탐색기 전용 토큰(탭 바·헤더 등)은 이식에서 제외하고
/// 메신저에 필요한 공통 토큰 + `danger`(파일 승인 등 위험 행위 — [docs/12 §B])를 얹었다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Theme {
    /// 창 루트 배경.
    pub window_bg: Color,
    /// 메뉴·툴바 크롬.
    pub chrome_bg: Color,
    /// 목록·대화 패널.
    pub panel_bg: Color,
    /// 행 교대 음영.
    pub panel_bg_alt: Color,
    /// 수신 말풍선 배경(대화 — 발신은 `accent`). 패널과의 대비 확보:
    /// **다크 = 패널보다 밝게 · 라이트 = 패널보다 어둡게**(사용자 확정 08-10).
    pub bubble_peer: Color,
    /// 입력 필드.
    pub field_bg: Color,
    /// 경계선·스플리터.
    pub border: Color,
    /// 강조(선택 줄·링크·활성 표시).
    pub accent: Color,
    /// 포커스 링(모든 커스텀 컨트롤 공통 — 선택 시 밝은 반투명 테두리).
    pub focus_ring: Color,
    /// 본문 텍스트.
    pub text: Color,
    /// 보조 텍스트.
    pub text_dim: Color,
    /// 선택 행 배경(포커스).
    pub sel_bg: Color,
    /// 선택 행 배경(비포커스 — 무채색 관례).
    pub sel_bg_inactive: Color,
    /// 위험 행위(파일 실체화 승인·차단 등).
    pub danger: Color,
    /// 긍정 상태(대조 완료 등).
    pub ok: Color,
    /// 주의 상태(미검증 등).
    pub warn: Color,
    /// 다크 팔레트 여부.
    pub is_dark: bool,
}

impl Theme {
    /// 다크 팔레트(임시 — M3-1c에서 수치 확정).
    #[must_use]
    pub const fn dark() -> Self {
        Theme {
            window_bg: Color(0x0014_161A),
            chrome_bg: Color(0x001E_2228),
            panel_bg: Color(0x0019_1C21),
            panel_bg_alt: Color(0x001F_242B),
            bubble_peer: Color(0x0031_3947),
            field_bg: Color(0x0026_2B33),
            border: Color(0x0036_3C46),
            accent: Color(0x003D_8BFF),
            focus_ring: Color(0x007F_B4FF),
            text: Color(0x00D6_DAE0),
            text_dim: Color(0x008A_919C),
            sel_bg: Color(0x0024_405F),
            sel_bg_inactive: Color(0x002C_313A),
            danger: Color(0x00E5_534B),
            ok: Color(0x002E_A043),
            warn: Color(0x00B5_7C1E),
            is_dark: true,
        }
    }

    /// 라이트 팔레트(임시).
    #[must_use]
    pub const fn light() -> Self {
        Theme {
            window_bg: Color(0x00F6_F7F9),
            chrome_bg: Color(0x00EE_F1F5),
            panel_bg: Color(0x00FF_FFFF),
            panel_bg_alt: Color(0x00F5_F7FA),
            bubble_peer: Color(0x00E2_E7EE),
            field_bg: Color(0x00FF_FFFF),
            border: Color(0x00D5_DAE1),
            accent: Color(0x003D_8BFF),
            focus_ring: Color(0x00A9_CCFF),
            text: Color(0x001B_1F26),
            text_dim: Color(0x006B_7280),
            sel_bg: Color(0x00D8_E8FF),
            sel_bg_inactive: Color(0x00E6_E9EE),
            danger: Color(0x00D3_2F2F),
            ok: Color(0x001A_7F37),
            warn: Color(0x009A_6700),
            is_dark: false,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::dark()
    }
}

/// `#RRGGBB`/`RRGGBB` → 토큰 색(형식 오류 = `None`) — 색상 설정 파싱(08-10).
#[must_use]
pub fn color_from_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(h, 16).ok().map(Color)
}

/// 토큰 색 → `#RRGGBB`(대문자) — 설정 저장·표시 형식.
#[must_use]
pub fn color_to_hex(c: Color) -> String {
    format!("#{:06X}", c.0 & 0x00FF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_dark() {
        assert_eq!(Theme::default(), Theme::dark());
    }

    #[test]
    fn light_and_dark_share_shape_not_values() {
        assert_ne!(Theme::dark().panel_bg, Theme::light().panel_bg);
        assert_eq!(
            Theme::dark().accent,
            Theme::light().accent,
            "강조색은 공통(임시)"
        );
    }
}

/// 영역별 글꼴 설정 — 크기·굵기·기울임(사용자 설정 · FR-U 가독성).
///
/// 글꼴 **패밀리** 선택은 시스템 폰트 열거(M3-3 확장)가 필요해 v1은 시스템 1벌 위에서
/// 크기·faux 굵기/기울임만 조정한다([docs/14 §5] 시각 언어 수치는 M3-1c에서 확정).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SlotFont {
    /// 글자 크기(논리 px).
    pub size: f32,
    /// 굵게(faux).
    pub bold: bool,
    /// 기울임(faux).
    pub italic: bool,
}

/// 영역별 글꼴 설정(기본 UI·사용자 목록·대화 본문·상태바).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FontPrefs {
    /// 기본 UI(버튼·헤더·설정 등).
    pub base: SlotFont,
    /// 사용자(피어) 목록.
    pub peerlist: SlotFont,
    /// 대화 본문.
    pub message: SlotFont,
    /// 상태바·보조.
    pub status: SlotFont,
}

impl Default for FontPrefs {
    /// 기본값 — **가독성 위해 상향**(구 13/15/11 → 16/18/13). 사용자 목록은 기본 UI와 같은 16.
    fn default() -> Self {
        let plain = |size| SlotFont {
            size,
            bold: false,
            italic: false,
        };
        Self {
            base: plain(16.0),
            peerlist: plain(16.0),
            message: plain(18.0),
            status: plain(15.0), // 상태바 가독성 상향(13→15 · 사용자 확정 08-09)
        }
    }
}
