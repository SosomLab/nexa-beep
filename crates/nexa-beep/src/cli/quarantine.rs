//! 무해화 종단 실측(`--quarantine-demo`) — 판정→봉인→격리→상태 기계→실체화→OS 표식.

/// **M4-1 종단 실측 데모** — 봉인→격리 저장→상태 기계→실체화→OS 표식을 실파일로 관측.
///
/// ⚠️ 데모 전용: 승인(Approve)을 자동 수행한다 — 제품 경로에는 자동 승인이 없다(FR-S-9 ·
/// `nbeep-safe::state`가 타입으로 보장). 조립 지점 어댑터(DR-21): 해시 = `nbeep-crypto::sha256`,
/// 표식 = `nbeep-plat::quarantine`(macOS 실물 · 그 외 미지원 명시).
pub(crate) fn quarantine_demo(src: &std::path::Path) {
    use nbeep_safe::{
        classify, friction_raised, step, Beepq, HashPort, MarkOutcome, MarkPort, Meta, QEvent,
        QState, QuarantineDir,
    };

    /// 실물 SHA-256 어댑터.
    struct CryptoHash;
    impl HashPort for CryptoHash {
        fn sha256(&self, data: &[u8]) -> [u8; 32] {
            nbeep_crypto::sha256(data)
        }
    }
    /// OS 격리 표식 어댑터(macOS = com.apple.quarantine).
    struct OsMark;
    impl MarkPort for OsMark {
        fn apply(&self, path: &std::path::Path) -> std::io::Result<MarkOutcome> {
            Ok(if nbeep_plat::quarantine::apply_quarantine_mark(path)? {
                MarkOutcome::Applied
            } else {
                MarkOutcome::Unsupported
            })
        }
    }

    let original = match std::fs::read(src) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("읽기 실패: {e}");
            return;
        }
    };
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());

    // ① 위험 등급 판정(확장자·매직 fail-closed).
    let v = classify(&name, &original);
    println!(
        "① 판정: risk={:?} (확장자={:?} · 매직={:?}/{}) 불일치 경고={}",
        v.risk,
        v.by_ext,
        v.by_magic,
        v.detected.name(),
        v.mismatch
    );

    // ② 봉인(.beepq) + 격리 저장.
    let sha = CryptoHash.sha256(&original);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let meta = Meta {
        orig_name: name.clone().into_bytes(),
        declared_ext: nbeep_safe::risk::extension_of(&name),
        declared_mime: String::new(),
        detected_kind: v.detected.name().into(),
        risk: v.risk,
        sender: nbeep_core::PeerId::from_bytes([0u8; 32]), // 데모 — 실전은 세션 상대
        received_at: now,
        expires_at: now + 7 * 24 * 3600,
        scan: nbeep_core::ScanOutcome::Unavailable,
        xfer: String::new(),
    };
    let sealed = Beepq::seal(&original, sha, &meta);
    let qdir = QuarantineDir::open(std::env::temp_dir().join("nexa-beep-quarantine")).unwrap();
    let qpath = qdir.save(&sha, &sealed).unwrap();
    println!("② 격리 저장: {} ({}B)", qpath.display(), sealed.len());

    // ③ 상태 기계 — 수신 완료(해시 OK) → 검사(백신 어댑터 없음 = Unavailable) → 승인(데모).
    let mut st = QState::Receiving;
    for ev in [
        QEvent::ReceiveComplete { hash_ok: true },
        QEvent::Inspect(nbeep_core::ScanOutcome::Unavailable),
    ] {
        st = step(st, ev).unwrap();
    }
    println!(
        "③ 상태: {st:?} (마찰 상승={} — 검사 못 함)",
        friction_raised(st)
    );
    st = step(st, QEvent::Approve).unwrap(); // ⚠️ 데모 한정 자동 — 제품은 사용자 버튼만

    // ④ 실체화 + OS 표식(rename 직후).
    let opened = Beepq::open(&std::fs::read(&qpath).unwrap()).unwrap();
    let dest = std::env::temp_dir().join("nexa-beep-materialized");
    match QuarantineDir::materialize(&opened, &dest, &CryptoHash, &OsMark) {
        Ok(m) => {
            st = step(st, QEvent::MaterializeOk).unwrap();
            println!(
                "④ 실체화: {} · 표식={:?} · 상태={st:?}",
                m.path.display(),
                m.mark
            );
            println!("   (.beepq는 보존 — 재실체화·감사: {})", qpath.display());
        }
        Err(e) => {
            st = step(st, QEvent::MaterializeFailed).unwrap();
            println!("④ 실체화 실패: {e} → 롤백 상태={st:?}");
        }
    }
}
