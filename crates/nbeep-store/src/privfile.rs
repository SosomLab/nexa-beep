//! 소유자 전용 파일 쓰기(09-05) — 봉인 세그먼트·핀·사이드카가 공유하는 한 벌.
//!
//! 배경: nexa-clip이 공용 크레이트 `nexa-conf`의 `write_atomic`이 **umask 기본(0644/0664)**
//! 으로 파일을 만드는 것을 발견해 전달했다(clip 전달문 32 A-1 · [docs/44 §7]). beep을
//! 실측하니 `settings.cfg`뿐 아니라 `trust.seg`·`keys.seg`·`profile.sec`·`server.pin`·
//! `groups.seg`·`history/*.seg`도 **664**였다 — 전부 `nexa-conf`를 타지 않는 **별도 쓰기
//! 경로**(파일마다 temp+rename을 손으로 짠 것)라 한 곳을 고쳐도 남는 구조였다.
//! 내용은 AEAD 봉인이라 노출은 암호문에 그치지만, 그룹 쓰기 비트(664)는 같은 그룹의
//! 다른 계정이 **훼손·삭제**할 수 있게 하고(fail-closed라 결과는 데이터 소실), 크기·갱신
//! 시각은 그대로 보인다. `identity.key`(keyfile — 이미 0600)와 같은 수준으로 맞춘다.
//!
//! - [`write_atomic`] — 0600 temp(Unix) → `sync_all` → 덮어쓰기 rename → 실패 시 temp 제거.
//! - [`write_atomic_fast`] · [`sync_fast`] — 자주 쓰는 파일(대화 기록)용: macOS에서 `F_FULLFSYNC` 대신 `fsync`.
//! - [`tighten`] — 이미 있는 파일의 그룹/타인 비트를 걷어낸다(부팅 1회 · no-op이 기본).
//!
//! Windows는 사용자 프로필 ACL이 같은 역할이라 모드 개념이 없다(no-op).

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

/// 소유자 전용(0600 · Unix)으로 **원자적** 쓰기 — PID 접미 temp → `sync_all` → rename.
/// 실패 시 temp를 지우고 기존 파일은 건드리지 않는다. 부모 디렉터리는 만든다.
///
/// # Errors
/// 디렉터리 생성·temp 열기·쓰기·동기화·rename 실패 시 `io::Error`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, bytes, fs::File::sync_all)
}

/// [`write_atomic`]과 같되 동기화가 [`sync_fast`] — **자주 쓰는 파일**(대화 기록)용.
///
/// # Errors
/// [`write_atomic`]과 같다.
pub fn write_atomic_fast(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, bytes, sync_fast)
}

/// 자주 쓰는 파일의 동기화(11-22) — macOS는 `fsync(2)`, 그 밖은 `sync_all`.
///
/// 배경: Rust의 `sync_all`은 macOS에서 `fcntl(F_FULLFSYNC)`(드라이브 캐시까지 비움)라 한 번에 ≈20ms가 든다
/// (11-21 실측 · 같은 기기 `fsync` 0.2~0.3ms). 대화 기록은 메시지가 오갈 때마다(디바운스 300ms~1.5s) **메인
/// 스레드**에서 저장되므로 그만큼 화면·입력이 멎는다. `fsync`는 데이터를 드라이브까지 넘기되 드라이브 자체
/// 캐시의 비움은 기다리지 않는다 — 앱·OS가 죽는 경우는 그대로 안전하고(APFS는 crash-consistent · 원자적
/// rename은 옛 파일 아니면 새 파일), **갑작스러운 전원 차단** 때만 마지막 저장분(직전 ≤1.5s의 대화 줄)이 옛
/// 내용으로 남을 수 있다(SQLite의 macOS 기본과 같은 선택). 신원 키·핀·그룹 명부처럼 드물게 쓰고 잃으면 안
/// 되는 파일은 `F_FULLFSYNC`를 쓰는 [`write_atomic`]을 그대로 쓴다.
///
/// # Errors
/// 동기화 시스템 콜 실패 시 `io::Error`.
pub fn sync_fast(f: &fs::File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd as _;
        extern "C" {
            fn fsync(fd: std::ffi::c_int) -> std::ffi::c_int;
        }
        // SAFETY: `f`가 살아 있는 동안 fd는 유효한 열린 파일이다 · fsync는 fd만 읽는다.
        if unsafe { fsync(f.as_raw_fd()) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        f.sync_all()
    }
}

fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    sync: fn(&fs::File) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let write = (|| -> io::Result<()> {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600); // rename이 temp의 모드를 최종 파일로 나른다
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        sync(&f)
    })();
    if let Err(e) = write {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// 기존 파일을 소유자 전용(0600)으로 죈다 — 없거나 거부되면 조용히 지나간다(기능을 막지
/// 않는다). 이미 0600이면 시스템 콜 없이 끝난다. Windows no-op.
pub fn tighten(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if let Ok(meta) = fs::metadata(path) {
            if meta.is_file() && meta.permissions().mode() & 0o077 != 0 {
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// 데이터 폴더의 **알려진 비밀 파일**을 부팅 1회 죈다 — 0600 보장 이전 판이 남긴 664를
/// 다음 저장을 기다리지 않고 정리한다. 대상 = 폴더 바로 아래 `*.seg`·`*.sec`·`*.pin`·
/// `*.key`와 `history/`·`pending/`의 `*.seg`. 목록 밖(격리물·이미지 캐시)은 건드리지 않는다.
pub fn tighten_data_dir(dir: &Path) {
    fn sweep(dir: &Path, exts: &[&str]) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext_ok = p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.contains(&x));
            if ext_ok && p.is_file() {
                tighten(&p);
            }
        }
    }
    sweep(dir, &["seg", "sec", "pin", "key"]);
    sweep(&dir.join("history"), &["seg"]);
    sweep(&dir.join("pending"), &["seg"]);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-privfile-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn write_then_read_roundtrip_and_no_temp_left() {
        let d = tmpdir("rt");
        let p = d.join("sub").join("x.seg");
        write_atomic(&p, b"hello").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"hello");
        write_atomic(&p, b"again").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"again", "덮어쓰기 rename");
        let leftovers: Vec<_> = fs::read_dir(d.join("sub"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp 잔여 없음: {leftovers:?}");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn failed_rename_keeps_original_and_removes_temp() {
        let d = tmpdir("fail");
        let orig = d.join("a.seg");
        write_atomic(&orig, b"keep").unwrap();
        let blocked = d.join("dir.seg");
        fs::create_dir_all(&blocked).unwrap();
        assert!(
            write_atomic(&blocked, b"x").is_err(),
            "대상이 디렉터리 = rename 실패"
        );
        assert_eq!(fs::read(&orig).unwrap(), b"keep");
        let tmps: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(tmps.is_empty(), "temp 잔여 없음: {tmps:?}");
        let _ = fs::remove_dir_all(d);
    }

    /// 새로 쓴 파일은 소유자만 읽는다(09-05 · clip A-1 계열).
    #[cfg(unix)]
    #[test]
    fn written_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tmpdir("mode");
        let p = d.join("trust.seg");
        write_atomic(&p, b"sealed").unwrap();
        let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode {mode:o}");
        let _ = fs::remove_dir_all(d);
    }

    /// 예전 판이 남긴 664는 부팅 스윕이 죄고, 목록 밖 파일은 건드리지 않는다.
    #[cfg(unix)]
    #[test]
    fn tighten_data_dir_fixes_known_files_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tmpdir("sweep");
        fs::create_dir_all(d.join("history")).unwrap();
        let loose = |p: &Path| {
            fs::write(p, b"x").unwrap();
            fs::set_permissions(p, fs::Permissions::from_mode(0o664)).unwrap();
        };
        let a = d.join("trust.seg");
        let b = d.join("history").join("ab12cd34.seg");
        let c = d.join("me.wire.png"); // 목록 밖 — 그대로
        loose(&a);
        loose(&b);
        loose(&c);
        tighten_data_dir(&d);
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&a), 0o600);
        assert_eq!(mode(&b), 0o600);
        assert_eq!(mode(&c), 0o664, "목록 밖은 불변");
        let _ = fs::remove_dir_all(d);
    }
}
