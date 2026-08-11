//! **OS 격리 표식** — 실체화 직후 파일에 "외부에서 온 것" 표식([docs/11 §5]).
//!
//! macOS: `com.apple.quarantine` 확장 속성 직접 기록 — Gatekeeper·XProtect가 첫 실행/열기
//! 시 검사를 발동한다. 값 형식은 `플래그;16진 시각;에이전트명;`(UUID 항은 생략 가능 —
//! LaunchServices DB 참조가 없을 때의 관례).
//! Windows(M4-3 · 08-11): **`Zone.Identifier` NTFS 대체 데이터 스트림(ADS) 직접 기록** —
//! SmartScreen·첨부 파일 관리자·Office 보호 보기가 이 표식으로 발동한다. `ZoneId=3`
//! (인터넷 영역) = 브라우저 다운로드와 같은 취급. `IAttachmentExecute` COM 대신 ADS
//! 직접 기록을 택했다 — 의존 0·T0 무권한·표식 내용이 결정적(검증 가능). ⚠️ ADS는
//! NTFS 전용 — FAT/exFAT 볼륨(USB 등)에선 쓰기가 실패하고 **오류를 그대로 보고**한다
//! (macOS xattr 미지원 볼륨과 같은 정책 — 조용히 삼키지 않는다).
//! Linux xattr은 별도 슬라이스(⏸ — 비지원 플랫폼 폴백은 "미지원"을 **명시**한다 · §5 공통 규칙).
//!
//! 어댑터 함수만 노출한다 — 도메인 포트(`nbeep-safe::MarkPort`)에는 조립 지점(bin)이 꽂는다
//! (DR-21 — 이 크레이트는 도메인을 모른다).

use std::io;
use std::path::Path;

/// 표식 적용 결과 — `true` = 부착됨 · `false` = 이 플랫폼은 미지원(사용자에게 명시).
///
/// # Errors
/// 지원 플랫폼에서 부착 시도가 실패한 경우(예: xattr 미지원 볼륨) — 역시 명시 대상.
pub fn apply_quarantine_mark(path: &Path) -> io::Result<bool> {
    imp(path)
}

#[cfg(target_os = "macos")]
fn imp(path: &Path) -> io::Result<bool> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    // 플래그 0081 = kLSQuarantineTypeOtherDownload 관례(다운로드 유래·미승인).
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let value = format!("0081;{secs:x};Nexa Beep;");

    let cpath = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "경로에 NUL"))?;
    let name = c"com.apple.quarantine";
    // options 0 — 심링크 추적 기본(격리 대상은 방금 rename한 실파일).
    let rc = unsafe {
        libc::setxattr(
            cpath.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if rc == 0 {
        Ok(true)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn imp(path: &Path) -> io::Result<bool> {
    use std::ffi::OsString;
    // MotW = 파일의 `Zone.Identifier` ADS. 경로 뒤에 `:스트림명`을 붙이면 NTFS가
    // 대체 스트림으로 연다(별도 API 불필요 — 첨부 파일 관리자(urlmon)가 쓰는 형식
    // 그대로 CRLF). ZoneId=3 = URLZONE_INTERNET.
    let mut ads = OsString::from(path.as_os_str());
    ads.push(":Zone.Identifier");
    std::fs::write(&ads, b"[ZoneTransfer]\r\nZoneId=3\r\n")?;
    Ok(true)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn imp(_path: &Path) -> io::Result<bool> {
    // Linux xattr는 별도 슬라이스(⏸) — 미지원을 명시적으로 보고.
    Ok(false)
}

#[cfg(all(test, windows))]
mod win_tests {
    use super::*;

    /// ADS를 실제로 읽어 확인(실측 — 추정 금지). 파일 본문은 그대로여야 한다.
    #[test]
    fn motw_ads_is_written_and_readable() {
        let dir = std::env::temp_dir().join(format!("nbeep-plat-motw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("marked.txt");
        std::fs::write(&f, b"payload").unwrap();

        assert!(apply_quarantine_mark(&f).unwrap(), "Windows = 부착");
        // 스트림 직접 읽기 — PowerShell `Get-Content -Stream Zone.Identifier`와 동일 실체.
        let mut ads = std::ffi::OsString::from(f.as_os_str());
        ads.push(":Zone.Identifier");
        let v = std::fs::read_to_string(&ads).expect("ADS 존재");
        assert!(
            v.contains("[ZoneTransfer]") && v.contains("ZoneId=3"),
            "{v}"
        );
        // 본문 무손상 + 기존 표식 위에 다시 표식(멱등)도 성립.
        assert_eq!(std::fs::read(&f).unwrap(), b"payload");
        assert!(apply_quarantine_mark(&f).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    /// getxattr로 실제 값을 읽어 확인(실측 — 추정 금지).
    fn read_xattr(path: &Path) -> Option<String> {
        let cpath = CString::new(path.as_os_str().as_bytes()).ok()?;
        let name = c"com.apple.quarantine";
        let mut buf = vec![0u8; 256];
        let n = unsafe {
            libc::getxattr(
                cpath.as_ptr(),
                name.as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                0,
                0,
            )
        };
        (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }

    #[test]
    fn mark_is_written_and_readable() {
        let dir = std::env::temp_dir().join(format!("nbeep-plat-q-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("marked.txt");
        std::fs::write(&f, b"payload").unwrap();

        assert!(apply_quarantine_mark(&f).unwrap(), "macOS = 부착");
        let val = read_xattr(&f).expect("xattr 존재");
        assert!(val.starts_with("0081;"), "플래그: {val}");
        assert!(val.contains("Nexa Beep"), "에이전트명: {val}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
