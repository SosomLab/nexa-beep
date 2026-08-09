//! **파일시스템 격리 저장소** — `.beepq` 보관·적재·실체화([docs/11 §4~5]).
//!
//! 외부 세계(해시·OS 표식)는 **포트 뒤에**(DR-21): [`HashPort`]는 조립 지점이
//! `nbeep-crypto::sha256`을 꽂고, [`MarkPort`]는 플랫폼 어댑터(MotW·quarantine —
//! 다음 슬라이스)를 꽂는다. 이 모듈 자체는 std 파일시스템만 쓴다.
//!
//! 실체화 규칙(ADR-0004 §4): 이름 정규화 → **덮어쓰기 금지**(충돌 = 뒤 번호) →
//! 임시 쓰기 → **SHA-256 재검증** → fsync → 원자적 rename → **표식은 rename 뒤** →
//! 실행 비트 미부여 → `.beepq`는 보존(재실체화·감사).
//! 표식 실패는 **실체화를 막지 않되 결과에 명시**한다(조용히 넘어가지 않는다 — §5).

use crate::container::Beepq;
use crate::sanitize::sanitize_filename;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// SHA-256 포트 — 구현은 조립 지점이 꽂는다(`nbeep-crypto::sha256`).
pub trait HashPort {
    /// 데이터 전체의 SHA-256.
    fn sha256(&self, data: &[u8]) -> [u8; 32];
}

/// OS 격리 표식 결과 — 실패해도 실체화는 진행하되 **사용자에게 명시**([docs/11 §5]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkOutcome {
    /// 표식 부착됨.
    Applied,
    /// 이 플랫폼/볼륨은 지원하지 않음(FAT ADS 불가 · Linux 표준 없음 등).
    Unsupported,
}

/// OS 격리 표식 포트(MotW·`com.apple.quarantine` — 어댑터는 플랫폼 슬라이스).
pub trait MarkPort {
    /// 최종 위치의 파일에 표식을 붙인다 — **rename 이후에만** 호출된다.
    ///
    /// # Errors
    /// 표식 시도 자체가 실패한 경우(지원하지만 실패 — 결과에 명시).
    fn apply(&self, path: &Path) -> io::Result<MarkOutcome>;
}

/// 표식 없음 어댑터 — Linux 1차(실행 비트 미부여가 방어선) 및 테스트용.
#[derive(Debug, Default)]
pub struct NoopMark;

impl MarkPort for NoopMark {
    fn apply(&self, _path: &Path) -> io::Result<MarkOutcome> {
        Ok(MarkOutcome::Unsupported)
    }
}

/// 실체화 실패 사유.
#[derive(Debug)]
pub enum MaterializeError {
    /// SHA-256 재검증 불일치 — 격리 유지·롤백(상태 기계 `MaterializeFailed`).
    HashMismatch,
    /// 파일시스템 오류.
    Io(io::Error),
}

impl From<io::Error> for MaterializeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl core::fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::HashMismatch => write!(f, "SHA-256 재검증 불일치 — 격리 유지"),
            Self::Io(e) => write!(f, "실체화 I/O 오류: {e}"),
        }
    }
}

impl std::error::Error for MaterializeError {}

/// 실체화 성공 결과.
#[derive(Debug)]
pub struct Materialized {
    /// 최종 파일 경로.
    pub path: PathBuf,
    /// OS 표식 결과 — `Unsupported`/오류는 UI가 "표식 못 붙임"으로 표시.
    pub mark: Result<MarkOutcome, io::Error>,
}

/// `.beepq` 보관 디렉터리.
#[derive(Debug)]
pub struct QuarantineDir {
    root: PathBuf,
}

impl QuarantineDir {
    /// 루트 디렉터리로 연다(없으면 만든다).
    ///
    /// # Errors
    /// 디렉터리 생성 실패.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// 루트 경로.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `.beepq` 바이트를 보관한다 — 이름 = `content_sha256` 앞 16헥사(`.beepq`),
    /// 임시 쓰기 → fsync → 원자적 rename(부분 파일 잔존 금지).
    ///
    /// # Errors
    /// 쓰기 실패.
    pub fn save(&self, sha256: &[u8; 32], beepq_bytes: &[u8]) -> io::Result<PathBuf> {
        let hex: String = sha256[..8].iter().map(|b| format!("{b:02x}")).collect();
        let final_path = self.root.join(format!("{hex}.beepq"));
        let tmp = self.root.join(format!("{hex}.beepq.part"));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(beepq_bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &final_path)?;
        Ok(final_path)
    }

    /// 보관 중인 `.beepq` 목록(파일명 정렬).
    ///
    /// # Errors
    /// 디렉터리 읽기 실패.
    pub fn list(&self) -> io::Result<Vec<PathBuf>> {
        let mut out: Vec<PathBuf> = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "beepq"))
            .collect();
        out.sort();
        Ok(out)
    }

    /// 실체화 — 재결합·재검증 후 목적 폴더에 안전한 이름으로 내보낸다.
    /// 성공해도 `.beepq`는 지우지 않는다(보존 기간까지 — 재실체화·감사).
    ///
    /// # Errors
    /// [`MaterializeError::HashMismatch`] = 격리 유지(호출자가 상태 기계에
    /// `MaterializeFailed`를 넣는다) · 그 외 I/O.
    pub fn materialize(
        beepq: &Beepq,
        dest_dir: &Path,
        hash: &dyn HashPort,
        mark: &dyn MarkPort,
    ) -> Result<Materialized, MaterializeError> {
        // 1) 재결합 + SHA-256 재검증(불일치 = 즉시 중단·아무것도 안 쓴다).
        let original = beepq.unseal();
        if hash.sha256(&original) != beepq.content_sha256 {
            return Err(MaterializeError::HashMismatch);
        }

        // 2) 이름 정규화 + 충돌 회피(덮어쓰기 금지 — 뒤 번호).
        let wanted = sanitize_filename(&String::from_utf8_lossy(&beepq.meta.orig_name));
        fs::create_dir_all(dest_dir)?;
        let final_path = collision_free(dest_dir, &wanted);

        // 3) 임시 쓰기 → fsync → 원자적 rename.
        let tmp = final_path.with_extension("nbeep-part");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&original)?;
            f.sync_all()?;
        }
        // 실행 비트 미부여(Linux 포함 — [docs/11 §5] 1차 방어선).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
        }
        if let Err(e) = fs::rename(&tmp, &final_path) {
            let _ = fs::remove_file(&tmp); // 부분 파일 잔존 금지
            return Err(e.into());
        }

        // 4) 표식은 최종 위치에 쓴 **직후**(rename 전이면 유실된다 — §5).
        let mark = mark.apply(&final_path);
        Ok(Materialized {
            path: final_path,
            mark,
        })
    }
}

/// 충돌 없는 경로 — `name.ext` → `name (1).ext` → `name (2).ext` ….
fn collision_free(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((st, ex)) if !st.is_empty() => (st.to_string(), format!(".{ex}")),
        _ => (name.to_string(), String::new()),
    };
    for i in 1.. {
        let cand = dir.join(format!("{stem} ({i}){ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    unreachable!("정수 범위 안에서 반드시 빈 이름이 있다")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::Meta;
    use nbeep_core::{PeerId, RiskLevel, ScanOutcome};

    /// 테스트 해시 — 결정적 32B(진짜 SHA-256 아님 · 포트 계약만 검증).
    struct FakeHash;
    impl HashPort for FakeHash {
        fn sha256(&self, data: &[u8]) -> [u8; 32] {
            let mut out = [0u8; 32];
            for (i, b) in data.iter().enumerate() {
                out[i % 32] = out[i % 32].wrapping_add(*b);
            }
            out
        }
    }

    fn meta(name: &[u8]) -> Meta {
        Meta {
            orig_name: name.to_vec(),
            declared_ext: "txt".into(),
            declared_mime: "text/plain".into(),
            detected_kind: "unknown".into(),
            risk: RiskLevel::Data,
            sender: PeerId::from_bytes([7u8; 32]),
            received_at: 1_700_000_000,
            expires_at: 1_700_600_000,
            scan: ScanOutcome::Unavailable,
            xfer: String::new(),
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-safe-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_list_roundtrip_atomic_name() {
        let d = tmpdir("save");
        let q = QuarantineDir::open(&d).unwrap();
        let original = b"hello quarantine".to_vec();
        let sha = FakeHash.sha256(&original);
        let bytes = Beepq::seal(&original, sha, &meta(b"a.txt"));
        let p = q.save(&sha, &bytes).unwrap();
        assert!(p.file_name().unwrap().to_str().unwrap().ends_with(".beepq"));
        assert_eq!(q.list().unwrap(), vec![p.clone()]);
        let loaded = Beepq::open(&fs::read(&p).unwrap()).unwrap();
        assert_eq!(loaded.unseal(), original);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn materialize_verifies_hash_and_marks_after_rename() {
        let d = tmpdir("mat");
        let dest = d.join("downloads");
        let original = b"the payload".to_vec();
        let sha = FakeHash.sha256(&original);
        let name = "r\u{202E}txt.exe".as_bytes();
        let bq = Beepq::open(&Beepq::seal(&original, sha, &meta(name))).unwrap();
        let out = QuarantineDir::materialize(&bq, &dest, &FakeHash, &NoopMark).unwrap();
        // RLO 제거된 안전한 이름으로 실체화.
        assert_eq!(out.path.file_name().unwrap(), "rtxt.exe");
        assert_eq!(fs::read(&out.path).unwrap(), original);
        assert!(
            matches!(out.mark, Ok(MarkOutcome::Unsupported)),
            "Noop = 미지원 명시"
        );
        // 부분 파일(.nbeep-part)이 남지 않는다.
        assert!(fs::read_dir(&dest).unwrap().count() == 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn materialize_hash_mismatch_writes_nothing() {
        let d = tmpdir("mismatch");
        let dest = d.join("downloads");
        let original = b"data".to_vec();
        let mut bq = Beepq::open(&Beepq::seal(
            &original,
            FakeHash.sha256(&original),
            &meta(b"x.bin"),
        ))
        .unwrap();
        bq.body.push(b'!'); // 변조
        bq.original_size += 1;
        let err = QuarantineDir::materialize(&bq, &dest, &FakeHash, &NoopMark).unwrap_err();
        assert!(matches!(err, MaterializeError::HashMismatch));
        assert!(
            !dest.exists() || fs::read_dir(&dest).unwrap().count() == 0,
            "아무것도 안 쓴다"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn collision_gets_numbered_never_overwrites() {
        let d = tmpdir("coll");
        let dest = d.join("dl");
        let original = b"one".to_vec();
        let sha = FakeHash.sha256(&original);
        let bq = Beepq::open(&Beepq::seal(&original, sha, &meta(b"same.txt"))).unwrap();
        let a = QuarantineDir::materialize(&bq, &dest, &FakeHash, &NoopMark).unwrap();
        let b = QuarantineDir::materialize(&bq, &dest, &FakeHash, &NoopMark).unwrap();
        let c = QuarantineDir::materialize(&bq, &dest, &FakeHash, &NoopMark).unwrap();
        assert_eq!(a.path.file_name().unwrap(), "same.txt");
        assert_eq!(b.path.file_name().unwrap(), "same (1).txt");
        assert_eq!(c.path.file_name().unwrap(), "same (2).txt");
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn no_exec_bit_on_materialized_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmpdir("exec");
        let dest = d.join("dl");
        let original = b"#!/bin/sh\necho hi".to_vec();
        let sha = FakeHash.sha256(&original);
        let bq = Beepq::open(&Beepq::seal(&original, sha, &meta(b"run.sh"))).unwrap();
        let out = QuarantineDir::materialize(&bq, &dest, &FakeHash, &NoopMark).unwrap();
        let mode = fs::metadata(&out.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0, "실행 비트 미부여([docs/11 §5])");
        let _ = fs::remove_dir_all(&d);
    }
}
