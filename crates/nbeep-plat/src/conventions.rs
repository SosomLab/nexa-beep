//! OS 동작 관례 어댑터(M3-1b · [docs/14 §6] `PlatformConventions`) — **시각은
//! macOS, 동작은 OS 네이티브**(DR-16)의 "동작" 절반을 한 곳에 모은다.
//!
//! 종전엔 수정 키 판정(`cfg!(macos)` 리터럴)과 표준 단축키 문자 매핑이 앱
//! 이벤트 루프 안에 하드코딩돼, 단축키 하나를 추가하려면 판정 지점을 찾아
//! 헤매야 했다. **매핑은 여기, 행동(디스패치)은 앱** — 행동은 앱 도메인이라
//! 여기로 끌어오지 않는다(DR-21 이음새 규칙과 같은 결).
//!
//! 스크롤 관성·창 닫기 관례도 이 모듈 몫이지만: 관성은 mac이 OS 이벤트로
//! 제공(모멘텀 스크롤이 휠 이벤트로 들어온다)하고 Win/Linux 합성 관성은 실기
//! 튜닝이 필요해 보류, 닫기 차등(mac 빨간 버튼 = 앱 유지)은 M3-2d 결정지다.

/// 표준 단축키 — **뜻**의 열거(키 조합이 아니라). 매핑은 [`std_accel`] 한 곳.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StdAccel {
    /// 설정 열기(⌘/Ctrl+,).
    Settings,
    /// 갤러리(⌘/Ctrl+G).
    Gallery,
    /// 전체 선택(⌘/Ctrl+A).
    SelectAll,
    /// 복사(⌘/Ctrl+C).
    Copy,
    /// 잘라내기(⌘/Ctrl+X).
    Cut,
    /// 붙여넣기(⌘/Ctrl+V).
    Paste,
    /// 파일 제안 수락(⌘/Ctrl+Y).
    AcceptOffer,
    /// 파일 제안 거절(⌘/Ctrl+N).
    RejectOffer,
    /// 주소 직접 입력(⌘/Ctrl+K · M3-16).
    AddEndpoint,
}

/// 주(수정) 키가 ⌘(super)인 OS인가 — mac만. 나머지는 Ctrl.
/// (종전 앱 루프의 `cfg!(target_os = "macos")` 리터럴을 한 곳으로.)
#[must_use]
pub fn primary_is_super() -> bool {
    cfg!(target_os = "macos")
}

/// 주 키가 눌린 상태에서 온 문자(물리 키 우선 매핑·대소 무관)를 표준 단축키로.
///
/// 호출측 규약: 물리 키(KeyA 등)를 문자로 내린 값이 우선이고, 물리 매핑이
/// 없으면 logical 문자를 그대로 준다([docs/27 §8] — 한글 자판에서 ⌘A의
/// logical은 "ㅁ"이라 물리 우선이 필수).
#[must_use]
pub fn std_accel(ch: &str) -> Option<StdAccel> {
    match ch.to_ascii_lowercase().as_str() {
        "," => Some(StdAccel::Settings),
        "g" => Some(StdAccel::Gallery),
        "a" => Some(StdAccel::SelectAll),
        "c" => Some(StdAccel::Copy),
        "x" => Some(StdAccel::Cut),
        "v" => Some(StdAccel::Paste),
        "y" => Some(StdAccel::AcceptOffer),
        "n" => Some(StdAccel::RejectOffer),
        "k" => Some(StdAccel::AddEndpoint),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 매핑 전수 — 대소 무관 · 미지 문자 = None(단축키 아님 = 입력으로 흘림).
    #[test]
    fn accel_map_is_case_insensitive_and_closed() {
        for (ch, want) in [
            (",", StdAccel::Settings),
            ("g", StdAccel::Gallery),
            ("A", StdAccel::SelectAll),
            ("C", StdAccel::Copy),
            ("x", StdAccel::Cut),
            ("V", StdAccel::Paste),
            ("y", StdAccel::AcceptOffer),
            ("N", StdAccel::RejectOffer),
            ("k", StdAccel::AddEndpoint),
        ] {
            assert_eq!(std_accel(ch), Some(want), "{ch}");
        }
        assert_eq!(std_accel("q"), None);
        assert_eq!(std_accel("ㅁ"), None, "logical 한글 = 단축키 아님");
    }

    #[test]
    fn primary_follows_os() {
        assert_eq!(primary_is_super(), cfg!(target_os = "macos"));
    }
}
