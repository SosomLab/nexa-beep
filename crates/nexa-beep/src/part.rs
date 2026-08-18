//! **부분 수신물 보존·복원**(M4-10a · D-31 확정 08-18) — 중단된 파일 수신의
//! 재개 원료. [docs/37] 설계 그대로:
//!
//! - 위치 = 격리함 하위 `parts/`(신원 폴더 소속 — 격리함 교훈 08-16). 파일명 =
//!   **전체 SHA-256 hex**`.part` — 재개 후보 판정 키가 그대로 이름이다(같은
//!   파일 = 같은 이름 = 자연 dedup).
//! - **봉인 저장**(`sealed` · 도메인 분리) — 부분물도 수신 데이터다(평문 3면
//!   원칙 08-17). 개봉 실패 = 없는 것(fail-closed).
//! - **격리함 목록에 보이지 않는다**(미완성은 존재만 — 봉투 원리). 완성 시에만
//!   기존 `.beepq` 경로로 승격되므로 무해화 게이트는 **무변경**.
//! - 수명 = 72h · 총량 1GiB(초과 시 오래된 것부터 — D-31 확정값).
//! - 부분 해시는 저장하지 않는다 — **재개 시점에 보존 바이트를 재해시**한다
//!   (설계의 "스트리밍 중간 상태 저장"과 검증 등가 · 직렬화 취약점 없음 ·
//!   한 번의 추가 패스는 재개 빈도에서 무시 가능).
//!
//! 평문 내부 배치(봉인 전): `[ver 1B][id 16B][size 8B][sender 32B][name_len 2B][name][bytes…]`

use nbeep_core::{xfer::PartialXfer, PeerId};
use std::path::PathBuf;

/// 재개 후보 **조회 키**(08-18 지연 해시 개정) — Offer의 sha가 0(지연)이라
/// 전체 해시로 부분물을 찾을 수 없다. 키 = sha256(name ‖ size ‖ sender):
/// 결정적 식별자일 뿐이고, **진위는 발신측 prefix 해시 대조**(M4-10b)가
/// 가른다(어긋나면 0부터 — 키 충돌·사칭이 손상으로 이어질 수 없다).
pub(crate) fn resume_key(name: &str, size: u64, sender: PeerId) -> [u8; 32] {
    let mut buf = Vec::with_capacity(name.len() + 8 + 32);
    buf.extend_from_slice(name.as_bytes());
    buf.extend_from_slice(&size.to_le_bytes());
    buf.extend_from_slice(sender.as_bytes());
    nbeep_crypto::sha256(&buf)
}

/// 봉인 도메인(도메인 분리 — 다른 파일 종류와 바꿔치기 차단).
const SEAL_PART: &[u8] = b"xfer-part-v1";
/// 내부 배치 버전.
const VER: u8 = 1;
/// 보존 기한(72h · D-31).
const RETAIN_SECS: u64 = 72 * 3600;
/// 총량 상한(1 GiB · D-31) — 초과 시 오래된 것부터.
const TOTAL_CAP: u64 = 1024 * 1024 * 1024;

fn parts_dir(channel: &str) -> PathBuf {
    crate::gate::quarantine_root(channel).join("parts")
}

fn hex(sha: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in sha {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// 부분물 봉인 저장 — 실패는 조용히 버린다(재개는 최적화 · 전송 정합성과 무관).
pub(crate) fn save_partial(channel: &str, secret: &[u8; 32], sender: PeerId, p: &PartialXfer) {
    let mut plain = Vec::with_capacity(1 + 16 + 8 + 32 + 2 + p.name.len() + p.bytes.len());
    plain.push(VER);
    plain.extend_from_slice(&p.id);
    plain.extend_from_slice(&p.size.to_le_bytes());
    plain.extend_from_slice(sender.as_bytes());
    let nlen = u16::try_from(p.name.len()).unwrap_or(u16::MAX);
    plain.extend_from_slice(&nlen.to_le_bytes());
    plain.extend_from_slice(&p.name[..nlen as usize]);
    plain.extend_from_slice(&p.bytes);
    let Ok(sealed) = nbeep_store::sealed::seal(SEAL_PART, secret, &plain) else {
        return;
    };
    let dir = parts_dir(channel);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let key = resume_key(&String::from_utf8_lossy(&p.name), p.size, sender);
    let _ = std::fs::write(dir.join(format!("{}.part", hex(&key))), sealed);
}

/// 재개 후보 조회 — 같은 sha의 부분물이 있고 **선언 크기가 같으며** 아직
/// 미완(보존 < size)일 때만 보존 바이트를 돌려준다. 조건 미달·개봉 실패 =
/// None + **그 파일 폐기**(어긋난 보존분은 재개 원료가 아니다).
pub(crate) fn load_partial(
    channel: &str,
    secret: &[u8; 32],
    sha: &[u8; 32],
    size: u64,
) -> Option<Vec<u8>> {
    let path = parts_dir(channel).join(format!("{}.part", hex(sha)));
    let sealed = std::fs::read(&path).ok()?;
    let plain = nbeep_store::sealed::open(SEAL_PART, secret, &sealed).or_else(|| {
        let _ = std::fs::remove_file(&path);
        None
    })?;
    let parsed = (|| {
        if plain.len() < 1 + 16 + 8 + 32 + 2 || plain[0] != VER {
            return None;
        }
        let psize = u64::from_le_bytes(plain[17..25].try_into().ok()?);
        let nlen = u16::from_le_bytes([plain[57], plain[58]]) as usize;
        let body = plain.get(59 + nlen..)?;
        if psize != size || body.is_empty() || body.len() as u64 >= size {
            return None;
        }
        Some(body.to_vec())
    })();
    if parsed.is_none() {
        let _ = std::fs::remove_file(&path); // 크기 불일치·손상 = 폐기
    }
    parsed
}

/// 보존 바이트 수만(M4-10c — 승인 창 "이어받기 N%" 표시용). 검증 규칙은
/// [`load_partial`]과 동일(전체 개봉 — 승인 창 열림 빈도라 비용 무해).
pub(crate) fn partial_len(
    channel: &str,
    secret: &[u8; 32],
    sha: &[u8; 32],
    size: u64,
) -> Option<u64> {
    load_partial(channel, secret, sha, size).map(|b| b.len() as u64)
}

/// 부분물 제거 — 완료(승격)·사용자 취소(의사 표시 — 재개 제안 금지) 시.
pub(crate) fn remove_partial(channel: &str, sha: &[u8; 32]) {
    let _ = std::fs::remove_file(parts_dir(channel).join(format!("{}.part", hex(sha))));
}

/// 수명 정리(부팅 1회) — 72h 초과 폐기 + 총량 1GiB 초과 시 오래된 것부터.
pub(crate) fn sweep_partials(channel: &str) {
    let dir = parts_dir(channel);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    // (mtime, size, path) — 기한 지난 것 즉시 삭제, 나머지는 총량 판정용 수집.
    let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "part") {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let mtime = md.modified().unwrap_or(now);
        let age = now.duration_since(mtime).unwrap_or_default();
        if age.as_secs() > RETAIN_SECS {
            let _ = std::fs::remove_file(&path);
        } else {
            kept.push((mtime, md.len(), path));
        }
    }
    let total: u64 = kept.iter().map(|(_, s, _)| *s).sum();
    if total <= TOTAL_CAP {
        return;
    }
    kept.sort_by_key(|(t, _, _)| *t); // 오래된 것부터
    let mut over = total - TOTAL_CAP;
    for (_, s, path) in kept {
        if std::fs::remove_file(&path).is_ok() {
            over = over.saturating_sub(s);
            if over == 0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(sha: u8, size: u64, got: usize) -> PartialXfer {
        PartialXfer {
            id: [1u8; 16],
            size,
            sha256: [sha; 32],
            name: b"f.bin".to_vec(),
            bytes: vec![0xAB; got],
        }
    }

    /// 저장 → 같은 sha+size 조회 = 보존 바이트 왕복 · 다른 size = 폐기.
    #[test]
    fn partial_roundtrip_and_size_mismatch_discards() {
        let ch = format!("t-part-{}", std::process::id());
        let secret = [9u8; 32];
        let sender = PeerId::from_bytes([2u8; 32]);
        save_partial(&ch, &secret, sender, &part(5, 100, 40));
        let key = resume_key("f.bin", 100, sender);
        assert_eq!(
            load_partial(&ch, &secret, &key, 100).as_deref(),
            Some(vec![0xAB; 40].as_slice()),
            "같은 (이름·크기·발신자) 키 = 보존 바이트"
        );
        // 다른 크기 선언 = 다른 키 = 미스(같은 키에 다른 size 내부 불일치도 폐기).
        save_partial(&ch, &secret, sender, &part(6, 100, 40));
        assert!(load_partial(&ch, &secret, &resume_key("f.bin", 200, sender), 200).is_none());
        // 다른 신원(secret) = 개봉 불가(fail-closed) — 파일도 폐기.
        save_partial(&ch, &secret, sender, &part(7, 100, 40));
        assert!(load_partial(&ch, &[1u8; 32], &key, 100).is_none());
        let _ = std::fs::remove_dir_all(crate::gate::quarantine_root(&ch));
    }

    /// remove = 즉시 소멸(완료·취소 경로).
    #[test]
    fn remove_partial_deletes() {
        let ch = format!("t-partrm-{}", std::process::id());
        let secret = [9u8; 32];
        let sender = PeerId::from_bytes([2u8; 32]);
        save_partial(&ch, &secret, sender, &part(8, 50, 10));
        let key = resume_key("f.bin", 50, sender);
        remove_partial(&ch, &key);
        assert!(load_partial(&ch, &secret, &key, 50).is_none());
        let _ = std::fs::remove_dir_all(crate::gate::quarantine_root(&ch));
    }
}
