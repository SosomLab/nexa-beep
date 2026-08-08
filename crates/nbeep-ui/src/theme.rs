//! 시맨틱 테마 토큰 — `nexa-dir2/crates/nexa-gui/src/theme.rs`의 **토큰 체계** 이식([docs/12 §B]).
//!
//! **색 하드코딩 금지 — 전 위젯이 테마를 인자로 받는다.** 다크/라이트가 값 교체만으로 끝난다
//! (FR-U-5). 토큰 키는 안정 계약 — rename 시 마이그레이션 표를 남긴다.
//! ⚠️ 값은 임시 팔레트다 — macOS 시각 언어 수치표(M3-1c) 확정 시 값만 교체한다.

pub use nbeep_gfx::Color;

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
    /// 입력 필드.
    pub field_bg: Color,
    /// 경계선·스플리터.
    pub border: Color,
    /// 강조(선택 줄·링크·활성 표시).
    pub accent: Color,
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
            field_bg: Color(0x0026_2B33),
            border: Color(0x0036_3C46),
            accent: Color(0x003D_8BFF),
            text: Color(0x00D6_DAE0),
            text_dim: Color(0x008A_919C),
            sel_bg: Color(0x0024_405F),
            sel_bg_inactive: Color(0x002C_313A),
            danger: Color(0x00E5_534B),
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
            field_bg: Color(0x00FF_FFFF),
            border: Color(0x00D5_DAE1),
            accent: Color(0x003D_8BFF),
            text: Color(0x001B_1F26),
            text_dim: Color(0x006B_7280),
            sel_bg: Color(0x00D8_E8FF),
            sel_bg_inactive: Color(0x00E6_E9EE),
            danger: Color(0x00D3_2F2F),
            is_dark: false,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::dark()
    }
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
