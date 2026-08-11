//! 신원 키 파일 — 기기 장기 신원의 영속(M2-5a · ADR-0005 §3 기본 A의 전제).
//!
//! 신원이 실행마다 새로 나면 상대의 TOFU 핀이 매번 깨진다 — 핀 영속(R-17)은
//! **내 신원 영속과 한 몸**이다. 포맷 v1:
//!
//! ```text
//! [magic 4B "NBK1"][개인키 32B][공개키 32B]   — 총 68B
//! ```
//!
//! - 파일은 **평문**이다 — 이 키가 곧 래핑 키의 원료라 자기 자신을 감쌀 수 없다
//!   (ADR-0005 §3 기본 A의 구조적 한계 = H-5 · 승격 ①②가 이걸 보완한다).
//! - Unix에선 0600으로 만든다. Windows는 사용자 프로필 ACL이 같은 역할.
//! - **손상 파일은 덮어쓰지 않는다**(fail-closed) — 새 신원을 조용히 만들면
//!   상대의 핀에서 나는 다른 사람이 되고, 원본 복구 기회도 사라진다.

use crate::Identity;
use std::io;
use std::path::Path;

/// 키 파일 매직(v1).
const MAGIC: [u8; 4] = *b"NBK1";
/// 파일 총 길이.
const LEN: usize = 4 + 64;

/// 키 파일을 읽거나, 없으면 새로 만들어 저장한다. 반환 `bool` = 새로 생성했는가.
///
/// # Errors
/// - 파일이 있는데 손상(길이·매직 불일치): `InvalidData` — **덮어쓰지 않는다**.
/// - 그 외 IO 실패(권한·디스크).
pub fn load_or_generate(path: &Path) -> io::Result<(Identity, bool)> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.len() != LEN || bytes[..4] != MAGIC {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "신원 키 파일 손상(길이/매직) — 덮어쓰지 않음",
                ));
            }
            let mut key = [0u8; 64];
            key.copy_from_slice(&bytes[4..]);
            Ok((Identity::from_key_bytes(&key), false))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let id = Identity::generate();
            write_new(path, &id)?;
            Ok((id, true))
        }
        Err(e) => Err(e),
    }
}

/// 새 키 파일 기록 — temp 생성(Unix 0600) → 덮어쓰기 rename(원자적).
fn write_new(path: &Path, id: &Identity) -> io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600); // 소유자 외 읽기 금지 — 개인키다.
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&MAGIC)?;
        f.write_all(&id.key_bytes())?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-keyfile-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 생성 → 재로드가 같은 신원(PeerId)을 돌려준다 — 영속의 핵심 계약.
    #[test]
    fn generate_then_reload_keeps_identity() {
        let d = tmpdir("roundtrip");
        let p = d.join("identity.key");
        let (a, created) = load_or_generate(&p).unwrap();
        assert!(created, "첫 호출 = 생성");
        let (b, created2) = load_or_generate(&p).unwrap();
        assert!(!created2, "둘째 호출 = 로드");
        assert_eq!(a.peer_id(), b.peer_id(), "재시작해도 같은 신원");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 손상 파일은 오류로 알리고 **덮어쓰지 않는다**(fail-closed).
    #[test]
    fn corrupt_file_is_not_overwritten() {
        let d = tmpdir("corrupt");
        let p = d.join("identity.key");
        std::fs::write(&p, b"garbage").unwrap();
        let e = load_or_generate(&p).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&p).unwrap(), b"garbage", "원본 보존");
        let _ = std::fs::remove_dir_all(&d);
    }
}
