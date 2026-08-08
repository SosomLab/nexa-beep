//! 시스템 폰트 발견 — 플랫폼 계층의 몫(ADR-0001 §4 — 폰트 열거·매칭은 OS마다 다르다).
//!
//! **임베드하지 않는다**(ADR-0001 결정 3 — CJK 포함 시 수 MB · 재배포 라이선스 부담).
//! v1 최소 경로: OS별 **한글 포함 UI 폰트 후보 목록**을 순서대로 찾아 첫 존재를 쓴다.
//! 정식 폰트 열거·폴백 체인(사용자 지정 포함)은 M3-3에서 확장한다.

use std::path::Path;

/// OS별 후보(경로, TTC 인덱스) — 앞이 우선.
#[cfg(target_os = "macos")]
const CANDIDATES: &[(&str, u32)] = &[
    ("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0), // 한글 UI 표준
    ("/System/Library/Fonts/Helvetica.ttc", 0),        // 라틴 폴백
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[(&str, u32)] = &[
    ("C:\\Windows\\Fonts\\malgun.ttf", 0), // 맑은 고딕(한글 UI 표준)
    ("C:\\Windows\\Fonts\\segoeui.ttf", 0), // 라틴 폴백
];

#[cfg(target_os = "linux")]
const CANDIDATES: &[(&str, u32)] = &[
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2), // 인덱스 2 = KR
    ("/usr/share/fonts/truetype/nanum/NanumGothic.ttf", 0),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0), // 라틴 폴백(CI 러너 포함)
];

/// 시스템 UI 폰트 바이트와 TTC 인덱스. 후보가 하나도 없으면 `None`
/// (호출 측이 "폰트 없음" 오류 UI — 조용히 죽지 않는다).
#[must_use]
pub fn system_ui_font() -> Option<(Vec<u8>, u32)> {
    for &(path, index) in CANDIDATES {
        if Path::new(path).exists() {
            if let Ok(data) = std::fs::read(path) {
                return Some((data, index));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_font_exists_on_supported_targets() {
        // 3-OS CI 전부에서 후보 중 하나는 존재해야 한다(러너 폰트 포함 — docs/18).
        let (data, _idx) = system_ui_font().expect("지원 OS에 UI 폰트 후보 없음");
        assert!(data.len() > 1000, "폰트 파일 실체");
    }
}
