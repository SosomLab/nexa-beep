//! 파일 검사 어댑터(M4 §6 · FR-S-15) — [`nbeep_core::FileScanner`] 포트의 OS별 구현.
//!
//! - **Windows**: AMSI(`AmsiInitialize`/`AmsiScanBuffer` 직접 — clipboard·tray의
//!   "시스템 API 직접·의존 0" 선례)가 정공법. **구현은 Windows 실기 세션 몫**
//!   ([docs/11] §6 — 미설치·초기화 실패 = `Unavailable` 폴백 필수 · 08-16 자리만).
//! - **macOS/Linux**: 시스템이 스크립트 가능한 온디맨드 검사 API를 주지 않는다
//!   (XProtect는 비공개 · clamav는 외부 의존 = DR-5 위반) — `Unavailable` 고정.
//!
//! `Unavailable`은 실패가 아니다 — 격리 상태기계가 마찰을 올린다(`friction_raised`).

use nbeep_core::{FileScanner, NoScanner, ScanOutcome};

/// 이 OS의 기본 검사기. 지금은 3-OS 전부 [`NoScanner`](`Unavailable`) — Windows
/// AMSI 어댑터가 들어오면 여기서만 갈린다(호출부 무변경 · DR-21 이음새).
#[must_use]
pub fn platform_scanner() -> Box<dyn FileScanner> {
    Box::new(NoScanner)
}

/// 편의 — 기본 검사기로 1회 검사.
#[must_use]
pub fn scan(name: &str, bytes: &[u8]) -> ScanOutcome {
    platform_scanner().scan(name, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 검사기 부재 = `Unavailable`(fail-closed의 짝 — 게이트가 마찰을 올린다).
    /// Windows AMSI가 들어와도 이 폴백 계약은 유지돼야 한다.
    #[test]
    fn default_scanner_is_unavailable() {
        assert_eq!(scan("a.exe", b"MZ"), ScanOutcome::Unavailable);
    }
}
