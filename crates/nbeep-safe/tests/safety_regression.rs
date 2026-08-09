//! **안전 회귀 게이트** — [docs/18] "안전 송수신 검증" 4항목을 CI가 매번 확인한다.
//!
//! 보안 상시 관찰(TODO §9)에서 *"CI 자동 회귀가 없으면 트리거가 사람 기억에 의존한다"* 를
//! 급소로 짚었다. 이 파일이 그 의존을 없앤다 — **파일 경로를 수정하면 여기가 먼저 깨진다**.
//!
//! | # | [docs/18] 항목 | 여기서 보는 것 |
//! |---|---|---|
//! | 1 | 격리물이 실행 불가한가 | 실체화 파일에 **실행 비트 0**(unix) · 크레이트에 실행/열기 API 부재 |
//! | 2 | 복원 후 표식이 남는가 | `MarkPort`가 **rename 이후** 호출되고 결과가 보고되는가 |
//! | 3 | Zip Slip/압축폭탄 거부 | 대표 공격 입력이 전부 거부되는가 |
//! | 4 | RLO 파일명 무해화 | 스푸핑 문자가 제거되어 실체가 드러나는가 |
//!
//! 단위 테스트가 각 모듈 안에도 있지만, 여기서는 **조립된 경로**(봉인→저장→실체화)로
//! 확인한다 — 모듈이 각자 옳아도 이어 붙인 순서가 틀리면 게이트는 뚫린다.

use nbeep_core::{PeerId, RiskLevel, ScanOutcome};
use nbeep_safe::{
    check_archive, classify, sanitize_filename, ArchivePolicy, Beepq, EntryDesc, HashPort,
    MarkOutcome, MarkPort, Meta, QuarantineDir,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// 결정적 해시 — 실제 SHA-256 대신 계약만 본다(crypto 의존 없이 safe 단독 검증).
struct TestHash;
impl HashPort for TestHash {
    fn sha256(&self, data: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in data.iter().enumerate() {
            out[i % 32] = out[i % 32].wrapping_add(*b);
        }
        out
    }
}

/// 표식 어댑터 스파이 — **호출 시점에 파일이 최종 위치에 있었는지** 기록한다.
struct SpyMark {
    calls: AtomicUsize,
    existed_at_call: AtomicUsize,
}
impl SpyMark {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            existed_at_call: AtomicUsize::new(0),
        }
    }
}
impl MarkPort for SpyMark {
    fn apply(&self, path: &Path) -> std::io::Result<MarkOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if path.exists() {
            self.existed_at_call.fetch_add(1, Ordering::SeqCst);
        }
        Ok(MarkOutcome::Applied)
    }
}

fn meta(name: &[u8], risk: RiskLevel) -> Meta {
    Meta {
        orig_name: name.to_vec(),
        declared_ext: String::new(),
        declared_mime: String::new(),
        detected_kind: "unknown".into(),
        risk,
        sender: PeerId::from_bytes([3u8; 32]),
        received_at: 1_700_000_000,
        expires_at: 1_700_600_000,
        scan: ScanOutcome::Unavailable,
        xfer: String::new(),
    }
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nbeep-safety-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("임시 폴더");
    d
}

/// ①+② 조립 경로 — 봉인→격리 저장→실체화까지 한 번에 태우고 게이트 조건을 전부 본다.
#[test]
fn materialized_file_is_not_executable_and_mark_runs_after_rename() {
    let d = tmpdir("exec");
    let dest = d.join("dl");
    // 실행형 내용(셔뱅)에 실행형 이름 — 가장 위험한 조합.
    let original = b"#!/bin/sh\nrm -rf /\n".to_vec();
    let sha = TestHash.sha256(&original);
    let bq = Beepq::open(&Beepq::seal(
        &original,
        sha,
        &meta(b"payload.sh", RiskLevel::Executable),
    ))
    .expect("봉인 왕복");

    let spy = SpyMark::new();
    let out = QuarantineDir::materialize(&bq, &dest, &TestHash, &spy).expect("실체화");

    // ① 실행 비트 0 — 승인해도 "바로 실행되는 파일"이 되지 않는다.
    // ⚠️ 정직한 한계: 기본 umask(0644)로도 이 단언은 통과한다. 이 검사가 잡는 것은
    // **나중에 "원본 모드 보존" 같은 변경이 들어오는 것**이다(회귀 방지용 못).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out.path)
            .expect("메타")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0, "실행 비트가 붙었다({mode:o})");
    }

    // ② 표식은 **최종 위치에 쓴 뒤** 호출돼야 한다(먼저 붙이면 rename에서 유실).
    assert_eq!(spy.calls.load(Ordering::SeqCst), 1, "표식 1회 호출");
    assert_eq!(
        spy.existed_at_call.load(Ordering::SeqCst),
        1,
        "표식 호출 시점에 파일이 최종 위치에 없었다 — rename 이전 호출"
    );
    assert!(matches!(out.mark, Ok(MarkOutcome::Applied)));

    // 임시 조각이 남지 않는다.
    let leftovers: Vec<_> = std::fs::read_dir(&dest)
        .expect("dest")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "nbeep-part"))
        .collect();
    assert!(leftovers.is_empty(), "부분 파일 잔존: {leftovers:?}");

    let _ = std::fs::remove_dir_all(&d);
}

/// ④ RLO 등 스푸핑 파일명이 실체화 이름에서 무해화되는가(조립 경로 기준).
#[test]
fn rlo_spoofed_name_is_neutralized_on_materialize() {
    let d = tmpdir("rlo");
    let dest = d.join("dl");
    let original = b"MZ\x90\x00".to_vec();
    let sha = TestHash.sha256(&original);
    // 보이기엔 "…txt", 실제로는 exe — RLO가 확장자를 뒤집어 보여 준다.
    let spoof = "invoice_\u{202E}txt.exe".as_bytes();
    let bq = Beepq::open(&Beepq::seal(
        &original,
        sha,
        &meta(spoof, RiskLevel::Executable),
    ))
    .expect("봉인 왕복");

    let out =
        QuarantineDir::materialize(&bq, &dest, &TestHash, &nbeep_safe::NoopMark).expect("실체화");
    let name = out
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .expect("이름")
        .to_string();
    assert!(
        !name.chars().any(|c| ('\u{202A}'..='\u{202E}').contains(&c)),
        "방향 제어 문자가 남았다: {name:?}"
    );
    assert!(
        name.ends_with(".exe"),
        "실체(exe)가 이름에 드러나야 한다: {name}"
    );
    // 원본 바이트는 감사용으로 보존된다(표시 이름과 별개).
    assert_eq!(bq.meta.orig_name, spoof);

    let _ = std::fs::remove_dir_all(&d);
}

/// ④' 이름 무해화 단위 규칙 — 경로 구분자·예약 장치명이 실체화 이름에 살아남지 않는다.
#[test]
fn path_separators_and_reserved_names_never_survive() {
    for raw in [
        "../../etc/passwd",
        "a/b/c.txt",
        "doc.txt:hidden.exe",
        "CON",
        "aux.log",
    ] {
        let out = sanitize_filename(raw);
        assert!(
            !out.contains('/') && !out.contains('\\') && !out.contains(':'),
            "구분자 잔존: {raw} → {out}"
        );
        let stem = out.split('.').next().unwrap_or("").to_ascii_lowercase();
        assert!(
            !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul"),
            "예약 장치명 그대로: {raw} → {out}"
        );
    }
}

/// ③ Zip Slip — 대표 공격 입력이 전부 거부되는가.
#[test]
fn zip_slip_inputs_are_rejected() {
    let policy = ArchivePolicy::default();
    for bad in [
        "../../etc/passwd",
        "/etc/passwd",
        "C:\\Windows\\evil.dll",
        "\\\\srv\\share\\x",
        "ok/../../escape",
    ] {
        let e = EntryDesc {
            name: bad.into(),
            local_name: None,
            compressed: 100,
            uncompressed: 1_000,
            is_dir: false,
            is_link: false,
            depth: 1,
        };
        assert!(
            nbeep_safe::check_entry(&e, &policy).is_err(),
            "통과하면 안 된다: {bad}"
        );
    }
}

/// ③ 압축 폭탄 — 비율·총량·개수·깊이 상한이 살아 있는가.
#[test]
fn compression_bombs_are_rejected() {
    let policy = ArchivePolicy::default();
    let bomb = EntryDesc {
        name: "bomb.bin".into(),
        local_name: None,
        compressed: 1_000,
        uncompressed: 10_000_000_000, // 1천만 배
        is_dir: false,
        is_link: false,
        depth: 1,
    };
    assert!(nbeep_safe::check_entry(&bomb, &policy).is_err(), "압축률");

    // 총량 — 상한을 넘기는 조합.
    let chunk = EntryDesc {
        name: "part".into(),
        local_name: None,
        compressed: 100_000_000,
        uncompressed: 800_000_000,
        is_dir: false,
        is_link: false,
        depth: 1,
    };
    let many = vec![chunk.clone(), chunk];
    assert!(check_archive(&many, &policy).is_err(), "총 해제 크기");

    // 깊이 — 중첩 아카이브.
    let deep = EntryDesc {
        name: "inner.zip".into(),
        local_name: None,
        compressed: 100,
        uncompressed: 1_000,
        is_dir: false,
        is_link: false,
        depth: policy.max_depth + 1,
    };
    assert!(nbeep_safe::check_entry(&deep, &policy).is_err(), "깊이");
}

/// 위험 판정이 **이름 속임수에 넘어가지 않는가** — 게이트 입구의 마지막 방어.
#[test]
fn renamed_executable_is_still_executable_grade() {
    let v = classify("보고서.pdf", b"MZ\x90\x00rest");
    assert_eq!(v.risk, RiskLevel::Executable, "매직이 실체를 잡아야 한다");
    assert!(v.mismatch, "형식 불일치 경고가 떠야 한다");
    // 매직이 없는 형식은 확장자가 유일한 방어선.
    assert_eq!(
        classify("shortcut.lnk", b"anything").risk,
        RiskLevel::Executable
    );
}

/// 해시가 어긋나면 **아무것도 쓰지 않는다**(FR-X-6 → 실체화 차단).
#[test]
fn tampered_content_never_reaches_disk() {
    let d = tmpdir("tamper");
    let dest = d.join("dl");
    let original = b"clean".to_vec();
    let mut bq = Beepq::open(&Beepq::seal(
        &original,
        TestHash.sha256(&original),
        &meta(b"x.bin", RiskLevel::Data),
    ))
    .expect("봉인 왕복");
    bq.body.push(b'!'); // 변조
    bq.original_size += 1;

    assert!(
        QuarantineDir::materialize(&bq, &dest, &TestHash, &nbeep_safe::NoopMark).is_err(),
        "변조물이 실체화됐다"
    );
    let wrote_anything = std::fs::read_dir(&dest)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    assert!(!wrote_anything, "실패 경로가 파일을 남겼다");

    let _ = std::fs::remove_dir_all(&d);
}
