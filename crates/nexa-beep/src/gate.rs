//! **무해화 게이트 합류** — 수신 완료물을 격리물로 만드는 조립 지점 공용 경로.
//!
//! CLI(`cli::chat`)와 GUI(`app`)가 **같은 함수**를 쓴다. 표현(println vs 대화 줄)만 다르고
//! 판정·봉인·저장은 한 곳이어야 한다 — 두 벌이면 한쪽만 강화되는 순간 게이트가 뚫린다.
//!
//! 순서([docs/11] ADR-0004 · FR-X-6 → FR-S-5): **SHA-256 재검증**(불일치 = 즉시 폐기) →
//! 위험 등급 판정 → `.beepq` 봉인 → 격리 디렉터리 저장. **실체화는 하지 않는다** —
//! 사용자 승인 후 별도 경로다(FR-S-9 · 자동 승인 경로 없음).

use nbeep_core::{PeerId, Received, RiskLevel};
use std::path::PathBuf;

/// GUI 노드의 격리 채널 — DR-18(PC 1대 = 노드 1개)의 그 노드.
pub(crate) const CH_GUI: &str = "gui";
/// CLI 헤드리스 도구(`--chat-*`·`--serve` 등)의 격리 채널 — 테스트·검증용 별도 노드.
pub(crate) const CH_CLI: &str = "cli";

/// 격리 보관 위치 — v1은 임시 폴더(영속 위치는 M2-5 저장소 확정 후).
///
/// **채널(하위 폴더)로 가른다** — 같은 PC에서 GUI와 CLI 도구를 함께 돌리면(테스트)
/// 서로 다른 노드의 수신물인데 한 폴더에 섞여, 남이 받은 파일이 내 격리함에 보였다
/// (사용자 지적 08-10). 신원(PeerId)별 분리가 정도이지만 신원이 아직 실행마다 새로
/// 생성되므로(영속은 M2-5·D-18), 그 전까지는 실행 채널로 가른다 — 재시작 후에도
/// 같은 채널이라 격리물이 계속 보인다(7일 보관과 양립).
#[must_use]
pub(crate) fn quarantine_root(channel: &str) -> PathBuf {
    // ★ 신원 폴더 소속(08-16 사용자 확정 — 종전 전역 temp는 같은 PC 다중
    //   인스턴스가 한 격리함을 공유했고(3신원 실기에서 발각 — 남의 격리물이
    //   보이고 Clear All이 남의 것까지 지웠다), OS temp 청소에 격리물이
    //   증발할 수 있었다(DR-4 위반 — 격리물은 승인 대기 중인 실데이터다).
    let root = crate::app::data_dir().join("quarantine").join(channel);
    migrate_legacy_quarantine(channel, &root);
    root
}

/// 구 전역 temp 격리함 → 신원 폴더 1회 이관(베스트 에포트 · 08-16). 공유
/// 위치라 남은 항목은 **먼저 여는 신원이 거둔다** — 격리 메타에 발신자만
/// 있고 수신 신원 표식이 없어 더 정밀한 귀속은 불가능하다(실기 데이터 한정
/// 한계 · 새 격리물은 처음부터 신원 폴더에 쌓인다).
fn migrate_legacy_quarantine(channel: &str, new_root: &std::path::Path) {
    let legacy = std::env::temp_dir()
        .join("nexa-beep-quarantine")
        .join(channel);
    let Ok(rd) = std::fs::read_dir(&legacy) else {
        return; // 구 격리함 없음(이미 이관됐거나 신규 설치)
    };
    let _ = std::fs::create_dir_all(new_root);
    for e in rd.flatten() {
        let from = e.path();
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = new_root.join(name);
        if to.exists() {
            continue; // 동시 이관 경합(다른 인스턴스가 먼저) — 그쪽이 진실
        }
        if std::fs::rename(&from, &to).is_err() {
            // 볼륨 경계(temp ≠ data 볼륨) — 복사 후 제거. 실패는 다음에 재시도.
            if std::fs::copy(&from, &to).is_ok() {
                let _ = std::fs::remove_file(&from);
            }
        }
    }
    let _ = std::fs::remove_dir(&legacy); // 비었을 때만 지워진다(경합 안전)
}

/// 격리 성공 결과 — 표시 계층이 그대로 렌더한다.
#[derive(Debug)]
pub(crate) struct Quarantined {
    /// 원본 파일명(표시용 — 실체화 시 정규화된다).
    pub name: String,
    /// 위험 등급.
    pub risk: RiskLevel,
    /// 확장자 주장과 매직 실체의 불일치(승인 화면 경고).
    pub mismatch: bool,
    /// `.beepq` 경로.
    pub path: PathBuf,
}

/// 격리 실패 사유 — 어느 쪽이든 **수신물은 남기지 않는다**.
#[derive(Debug)]
pub(crate) enum GateError {
    /// 선언 해시와 실측 해시 불일치(FR-X-6) — 전송 중 손상 또는 위조.
    HashMismatch,
    /// 격리 저장 실패(디스크·권한).
    Store(std::io::Error),
}

impl core::fmt::Display for GateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::HashMismatch => write!(f, "SHA-256 불일치 — 폐기"),
            Self::Store(e) => write!(f, "격리 저장 실패: {e}"),
        }
    }
}

/// 수신 완료물 → 격리물. 성공해도 **실체화는 하지 않는다**(승인 대기).
///
/// # Errors
/// [`GateError`] — 해시 불일치·저장 실패. 두 경우 모두 아무것도 남기지 않는다.
/// 격리물 봉투 도메인(ADR-0005 §4~ · 08-17 — sealed 문법의 도메인 분리 축).
pub(crate) const SEAL_QUARANTINE: &[u8] = b"quarantine-v1";

/// 격리물 파일 읽기 **관문**(08-17 — 평문 3면 조치 ①) — 봉인본은 개봉하고,
/// 구본(봉인 도입 전 평문 `.beepq`)은 그대로 돌려준다(관용 — 이관은 목록
/// 로드가 lazy 재봉인). 개봉 실패(다른 신원·손상·바꿔치기) = None(fail-closed).
pub(crate) fn read_beepq_bytes(path: &std::path::Path, secret: &[u8; 32]) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if nbeep_store::sealed::is_sealed(&bytes) {
        return nbeep_store::sealed::open(SEAL_QUARANTINE, secret, &bytes);
    }
    Some(bytes) // 구본 관용 — 매직 우연 충돌은 위 open 실패로 드러난다
}

pub(crate) fn quarantine_received(
    got: &Received,
    sender: PeerId,
    channel: &str,
    seal_secret: &[u8; 32],
) -> Result<Quarantined, GateError> {
    use nbeep_safe::{classify, Beepq, Meta, QuarantineDir};

    // ① 재검증 — 선언과 실측이 다르면 즉시 폐기(부분·위조물 잔존 금지).
    let actual = nbeep_crypto::sha256(&got.bytes);
    if actual != got.declared_sha256 {
        return Err(GateError::HashMismatch);
    }

    // ② 판정(확장자·매직 fail-closed).
    let name = String::from_utf8_lossy(&got.name).into_owned();
    let v = classify(&name, &got.bytes);

    // ③ 봉인 + 격리 저장.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let meta = Meta {
        orig_name: got.name.clone(),
        declared_ext: nbeep_safe::risk::extension_of(&name),
        declared_mime: String::new(),
        detected_kind: v.detected.name().into(),
        risk: v.risk,
        sender,
        received_at: now,
        expires_at: now + 7 * 24 * 3600,
        // 검사(§6 · ADR-0004) — 디스크 기록 전 버퍼 단계. 포트 뒤라(DR-21)
        // Windows AMSI가 들어와도 이 줄은 안 바뀐다. 지금은 3-OS Unavailable.
        scan: nbeep_plat::scan::scan(&name, &got.bytes),
        xfer: String::new(),
    };
    let sealed = Beepq::seal(&got.bytes, actual, &meta);
    // ★ 디스크 봉인(08-17 — 종전 .beepq는 무결성 봉인뿐 **평문**이었다):
    //   신원 파생 키(D-18 §3 계층)로 AEAD — 이 신원의 이 앱만 연다(같은 PC
    //   다른 계정·다른 신원·백업 유출 전부 fail-closed). 봉인 실패 = 저장 포기
    //   (평문으로 쓰는 폴백은 없다 — 원칙이 무너진다).
    let sealed = nbeep_store::sealed::seal(SEAL_QUARANTINE, seal_secret, &sealed)
        .map_err(GateError::Store)?;
    let path = QuarantineDir::open(quarantine_root(channel))
        .and_then(|q| q.save(&actual, &sealed))
        .map_err(GateError::Store)?;

    Ok(Quarantined {
        name,
        risk: v.risk,
        mismatch: v.mismatch,
        path,
    })
}

/// 프로필 이미지 캐시 봉투 도메인(08-17 — 평문 3면 조치 ③).
pub(crate) const SEAL_PROFILE_CACHE: &[u8] = b"profile-cache-v1";

/// PII 봉인 사이드카 도메인 + 대상 키(08-17 — 평문 3면 조치 ②).
pub(crate) const SEAL_PII: &[u8] = b"pii-v1";
pub(crate) const PII_KEYS: &[&str] = &["profile.email", "profile.phone"];

/// 대화 기록 봉투 도메인(ADR-0005 §4 · M2-5b — 저장 암호화 A 단일).
pub(crate) const SEAL_HISTORY: &[u8] = b"history-v1";
