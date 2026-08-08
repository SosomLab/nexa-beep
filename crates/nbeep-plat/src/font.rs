//! 시스템 폰트 발견 — 플랫폼 계층의 몫(ADR-0001 §4 — 폰트 열거·매칭은 OS마다 다르다).
//!
//! **임베드하지 않는다**(ADR-0001 결정 3). v1 최소 경로: OS별 **한글 포함 UI 폰트 후보 목록**을
//! 순서대로 찾아 첫 존재를 쓴다. 정식 폰트 열거·폴백 체인은 M3-3에서 확장.
//!
//! ## 메모리 매핑(사용자 확정 08-08 — R-15 해소)
//!
//! `fs::read`는 폰트 파일 전체를 힙에 복사한다 — macOS 한글 TTC는 **55MB**, Windows 맑은 고딕
//! 12.8MB로 유휴 RSS 예산(NFR-B-1 ≤30MB)을 잠식한다. **mmap**은 파일 백드 페이지라 실제로
//! 읽은 글리프 페이지만 상주하고, 메모리 압박 시 OS가 회수한다(더티 페이지 없음 — 스왑 아닌 폐기).
//!
//! 매핑은 **의도적으로 누수**한다(`Box::leak`) — 폰트는 프로세스 수명 자원이라(로드 1회·앱 종료까지
//! 사용) 해제 시점이 없고, `&'static [u8]`이 되어 자기참조 없이 `gfx`로 넘어간다.

use memmap2::Mmap;
use std::fs::File;
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

/// 시스템 UI 폰트를 **메모리 매핑**해 바이트와 TTC 인덱스를 돌려준다.
/// 후보가 하나도 없으면 `None`(호출 측이 "폰트 없음" 오류 UI — 조용히 죽지 않는다).
#[must_use]
pub fn system_ui_font() -> Option<(&'static [u8], u32)> {
    for &(path, index) in CANDIDATES {
        if !Path::new(path).exists() {
            continue;
        }
        let Ok(file) = File::open(path) else { continue };
        // SAFETY: mmap의 UB 조건은 "매핑 중 파일이 절단·변경"이다. 시스템 폰트 파일은
        // OS 배포본의 읽기 전용 자산으로 실행 중 변경되지 않는다(변경 = OS 업데이트 재부팅).
        let Ok(mmap) = (unsafe { Mmap::map(&file) }) else {
            continue;
        };
        if mmap.len() < 1000 {
            continue; // 빈/깨진 파일 — 다음 후보
        }
        // 프로세스 수명 자원 — 의도적 누수(모듈 문서 참조).
        let leaked: &'static Mmap = Box::leak(Box::new(mmap));
        return Some((&leaked[..], index));
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
