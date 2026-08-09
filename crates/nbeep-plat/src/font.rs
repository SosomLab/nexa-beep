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

/// OS별 **고정폭** 후보(사용자 요청 08-09 — 숫자 폭이 변해 화면이 떨리는 것을 막는다).
#[cfg(target_os = "macos")]
const MONO_CANDIDATES: &[(&str, u32)] = &[
    ("/System/Library/Fonts/Menlo.ttc", 0),
    ("/System/Library/Fonts/SFNSMono.ttf", 0),
    ("/System/Library/Fonts/Monaco.ttf", 0),
];

#[cfg(target_os = "windows")]
const MONO_CANDIDATES: &[(&str, u32)] = &[
    ("C:\\Windows\\Fonts\\consola.ttf", 0),
    ("C:\\Windows\\Fonts\\cour.ttf", 0),
];

#[cfg(target_os = "linux")]
const MONO_CANDIDATES: &[(&str, u32)] = &[
    ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0),
    (
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        0,
    ),
    ("/usr/share/fonts/opentype/noto/NotoSansMono-Regular.ttf", 0),
];

/// 사용자 폰트를 찾을 디렉터리(앞이 우선 — 사용자 설치본이 시스템보다 먼저).
#[cfg(target_os = "macos")]
const FONT_DIRS: &[&str] = &["~/Library/Fonts", "/Library/Fonts", "/System/Library/Fonts"];
#[cfg(target_os = "windows")]
const FONT_DIRS: &[&str] = &["C:\\Windows\\Fonts"];
#[cfg(target_os = "linux")]
const FONT_DIRS: &[&str] = &[
    "~/.local/share/fonts",
    "~/.fonts",
    "/usr/share/fonts",
    "/usr/local/share/fonts",
];

/// 비교용 정규화 — 공백·하이픈·언더바를 지우고 소문자로("D2 Coding" == "d2coding").
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// 경로를 mmap해 바이트를 얻는다(실패 = `None`).
fn map_font(path: &Path) -> Option<&'static [u8]> {
    let file = File::open(path).ok()?;
    // SAFETY: system_ui_font와 같은 근거 — 폰트 파일은 실행 중 변경되지 않는 읽기 전용 자산.
    let mmap = unsafe { Mmap::map(&file) }.ok()?;
    if mmap.len() < 1000 {
        return None;
    }
    let leaked: &'static Mmap = Box::leak(Box::new(mmap));
    Some(&leaked[..])
}

/// 홈(`~`) 확장.
fn expand(dir: &str) -> std::path::PathBuf {
    if let Some(rest) = dir.strip_prefix("~/") {
        if let Some(h) = crate::paths::home_dir() {
            return h.join(rest);
        }
    }
    std::path::PathBuf::from(dir)
}

/// **패밀리 이름으로 폰트 파일을 찾는다** — 파일명 어간을 정규화 비교한다.
///
/// ⚠️ 정직한 한계: 폰트 내부 `name` 테이블을 읽지 않고 **파일명으로 맞춘다**(파서를
/// 들이지 않기 위한 선택 — DR-5). 그래서 파일명과 표시 패밀리명이 다른 폰트는 못 찾는다.
/// 대신 부분 일치까지 허용해 `D2Coding` → `D2Coding-Ver1.3.2.ttf`는 찾는다.
#[must_use]
pub fn find_font_by_family(family: &str) -> Option<(&'static [u8], u32)> {
    let want = norm(family);
    if want.is_empty() {
        return None;
    }
    for dir in FONT_DIRS {
        let Ok(rd) = std::fs::read_dir(expand(dir)) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"));
            if !ext_ok {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let got = norm(stem);
            // 정확 일치 우선, 그다음 접두 일치(버전 접미가 붙은 파일명 대응).
            if got == want || got.starts_with(&want) {
                if let Some(bytes) = map_font(&path) {
                    return Some((bytes, 0));
                }
            }
        }
    }
    None
}

/// OS 기본 **고정폭** 폰트(없으면 `None` — 호출 측이 UI 폰트로 폴백).
#[must_use]
pub fn system_mono_font() -> Option<(&'static [u8], u32)> {
    for &(path, index) in MONO_CANDIDATES {
        let p = Path::new(path);
        if p.exists() {
            if let Some(bytes) = map_font(p) {
                return Some((bytes, index));
            }
        }
    }
    None
}

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

    #[test]
    fn family_normalization_ignores_case_space_dash() {
        assert_eq!(norm("D2 Coding"), "d2coding");
        assert_eq!(norm("Liberation-Mono"), "liberationmono");
        assert_eq!(norm("  Menlo  "), "menlo");
    }

    #[test]
    fn unknown_family_is_none_not_panic() {
        assert!(find_font_by_family("이런폰트는없다12345").is_none());
        assert!(
            find_font_by_family("").is_none(),
            "빈 이름 = 조회하지 않는다"
        );
    }

    #[test]
    fn mono_font_is_available_on_this_os() {
        // 실측 — 이 기기에 고정폭 폰트가 실제로 있는가(없으면 폴백 경로를 타야 한다).
        let got = system_mono_font();
        #[cfg(target_os = "macos")]
        assert!(got.is_some(), "macOS는 Menlo/SF Mono가 있어야 한다");
        let _ = got;
    }
}
