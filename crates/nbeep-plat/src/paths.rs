//! **사용자 폴더 위치** — 실체화 기본 대상(다운로드 폴더 · [docs/11] §4 실체화 절차 1).
//!
//! 외부 크레이트 없이 환경 변수만 본다(런타임 의존 0 · DR-5). 표준 위치가 없거나
//! 만들 수 없으면 **호출자가 판단할 수 있게** `None`을 준다 — 임의 폴더에 몰래 쓰지 않는다.

use std::path::PathBuf;

/// 홈 디렉터리(`$HOME` · 윈도우 `%USERPROFILE%`).
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// 다운로드 폴더 — 실체화 기본 대상. 없으면 `None`(호출자가 대안을 정한다).
///
/// 리눅스의 XDG 사용자 폴더 설정(`user-dirs.dirs`)까지는 보지 않는다(v1) —
/// `$HOME/Downloads`가 없으면 `None`이고, 대상 폴더 선택 UI는 M4 후반.
#[must_use]
pub fn downloads_dir() -> Option<PathBuf> {
    let d = home_dir()?.join("Downloads");
    d.is_dir().then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_a_real_directory_when_present() {
        // 환경에 따라 없을 수 있다 — 있으면 반드시 실제 디렉터리여야 한다.
        if let Some(h) = home_dir() {
            assert!(h.is_dir(), "홈이 디렉터리가 아니다: {}", h.display());
        }
    }

    #[test]
    fn downloads_is_under_home_when_present() {
        if let (Some(h), Some(d)) = (home_dir(), downloads_dir()) {
            assert!(d.starts_with(&h), "다운로드가 홈 밖: {}", d.display());
            assert!(d.is_dir());
        }
    }
}
