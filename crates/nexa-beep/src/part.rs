//! **부분 수신물 보존·복원**(M4-10a · D-31 확정 08-18 · **M4-12 ④ 개정 11-07 — 스풀이 곧 부분물**).
//!
//! - 위치 = 격리함 하위 `parts/`(신원 폴더 소속). 파일명 = **재개 키 hex**`.part` — 재개 후보 판정 키가 그대로 이름.
//! - `.part` = 수신 중 스풀 그 자체([`crate::spool::SpoolSink`] · NBSE v2 스트림 · 도메인 `quarantine-v2` = 격리물과
//!   **같은 봉투**). 종전처럼 세션 종료 때 메모리 부분물을 따로 봉인해 쓰는 일이 없다(= S 버퍼 없음) · 완료는 꼬리를
//!   이어 쓰고 `.beepq`로 rename.
//! - 사이드카 `.part.meta`(도메인 `xfer-part-meta-v2`) = `[ver 1B][id 16B][size 8B][sender 32B][name_len 2B][name]` —
//!   재개 판정(선언 크기 일치)과 승인 창 "이어받기 N%" 표시용. 본문은 들지 않는다.
//! - 재개 = 스트림을 다시 열어 마지막 청크를 되돌리고 이어 쓴다 · 프리픽스 해시는 **재개 시점에 보존 바이트를
//!   재해시**(설계 그대로 — 직렬화 취약점 없음).
//! - 격리함 목록에 보이지 않는다(미완성은 존재만 — 봉투 원리) · 수명 = 72h · 총량 1GiB(초과 시 오래된 것부터).

use nbeep_core::{PeerId, XferId};
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

/// 사이드카 봉인 도메인(도메인 분리 — 다른 파일 종류와 바꿔치기 차단).
const SEAL_PART_META: &[u8] = b"xfer-part-meta-v2";
/// 사이드카 내부 배치 버전.
const META_VER: u8 = 2;
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

/// `.part` 경로.
pub(crate) fn part_path(channel: &str, key: &[u8; 32]) -> PathBuf {
    parts_dir(channel).join(format!("{}.part", hex(key)))
}

fn meta_path(channel: &str, key: &[u8; 32]) -> PathBuf {
    parts_dir(channel).join(format!("{}.part.meta", hex(key)))
}

/// 사이드카 쓰기(스풀 생성 때).
pub(crate) fn write_part_meta(
    channel: &str,
    secret: &[u8; 32],
    key: &[u8; 32],
    id: XferId,
    size: u64,
    sender: PeerId,
    name: &[u8],
) -> std::io::Result<()> {
    let nlen = u16::try_from(name.len()).unwrap_or(u16::MAX);
    let mut plain = Vec::with_capacity(1 + 16 + 8 + 32 + 2 + usize::from(nlen));
    plain.push(META_VER);
    plain.extend_from_slice(&id);
    plain.extend_from_slice(&size.to_le_bytes());
    plain.extend_from_slice(sender.as_bytes());
    plain.extend_from_slice(&nlen.to_le_bytes());
    plain.extend_from_slice(&name[..usize::from(nlen)]);
    let sealed = nbeep_store::sealed::seal(SEAL_PART_META, secret, &plain)?;
    std::fs::create_dir_all(parts_dir(channel))?;
    std::fs::write(meta_path(channel, key), sealed)
}

/// 사이드카 읽기 → 선언 크기(손상·개봉 실패 = None).
fn read_part_meta_size(channel: &str, secret: &[u8; 32], key: &[u8; 32]) -> Option<u64> {
    let sealed = std::fs::read(meta_path(channel, key)).ok()?;
    let plain = nbeep_store::sealed::open(SEAL_PART_META, secret, &sealed)?;
    if plain.len() < 1 + 16 + 8 + 32 + 2 || plain[0] != META_VER {
        return None;
    }
    Some(u64::from_le_bytes(plain[17..25].try_into().ok()?))
}

/// 사이드카만 제거(완료 → `.beepq` 승격 때).
pub(crate) fn remove_part_meta(channel: &str, key: &[u8; 32]) {
    let _ = std::fs::remove_file(meta_path(channel, key));
}

/// 재개 후보 조회(M4-10 · ④) — 같은 키의 `.part`가 있고 **선언 크기가 같으며** 아직 미완일 때 스풀을 다시 연다.
/// 조건 미달·개봉 실패 = None + **그 파일 폐기**(어긋난 보존분은 재개 원료가 아니다).
pub(crate) fn open_resume(
    channel: &'static str,
    secret: &[u8; 32],
    key: &[u8; 32],
    size: u64,
) -> Option<crate::spool::Resume> {
    if read_part_meta_size(channel, secret, key) != Some(size) {
        remove_partial(channel, key);
        return None;
    }
    let Some((sink, prefix_sha)) = crate::spool::SpoolSink::reopen(channel, secret, key) else {
        remove_partial(channel, key);
        return None;
    };
    let prefix_len = sink.body_len();
    if prefix_len == 0 || prefix_len >= size {
        sink.discard();
        return None;
    }
    Some(crate::spool::Resume {
        sink,
        prefix_len,
        prefix_sha,
    })
}

/// 보존 바이트 수만(M4-10c — 승인 창 "이어받기 N%" 표시용) — 스트림 기하에서(본문을 읽지 않는다). 검증 규칙은
/// [`open_resume`]과 같되 폐기는 하지 않는다(표시 전용).
pub(crate) fn partial_len(
    channel: &str,
    secret: &[u8; 32],
    key: &[u8; 32],
    size: u64,
) -> Option<u64> {
    if read_part_meta_size(channel, secret, key) != Some(size) {
        return None;
    }
    let f = std::fs::File::open(part_path(channel, key)).ok()?;
    let len = f.metadata().ok()?.len();
    let r = nbeep_store::sealed_stream::StreamReader::open(
        crate::spool::SEAL_QUARANTINE_V2,
        secret,
        f,
        len,
    )?;
    let body = r.plain_len().checked_sub(nbeep_safe::HEAD_V2 as u64)?;
    (body > 0 && body < size).then_some(body)
}

/// 부분물 제거(`.part` + 사이드카) — 완료(승격)·사용자 취소(의사 표시 — 재개 제안 금지)·손상 시.
pub(crate) fn remove_partial(channel: &str, key: &[u8; 32]) {
    let _ = std::fs::remove_file(part_path(channel, key));
    let _ = std::fs::remove_file(meta_path(channel, key));
}

/// 수명 정리(부팅 1회) — 72h 초과 폐기 + 총량 1GiB 초과 시 오래된 것부터 · 짝 잃은 사이드카 정리.
pub(crate) fn sweep_partials(channel: &str) {
    let dir = parts_dir(channel);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    // (mtime, size, path) — 기한 지난 것 즉시 삭제, 나머지는 총량 판정용 수집.
    let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let mut metas: Vec<PathBuf> = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        match path.extension().and_then(|x| x.to_str()) {
            Some("meta") => {
                metas.push(path);
                continue;
            }
            Some("part") => {}
            _ => continue,
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
    if total > TOTAL_CAP {
        kept.sort_by_key(|(t, _, _)| *t); // 오래된 것부터
        let mut over = total - TOTAL_CAP;
        for (_, s, path) in &kept {
            if std::fs::remove_file(path).is_ok() {
                over = over.saturating_sub(*s);
                if over == 0 {
                    break;
                }
            }
        }
    }
    // 짝(.part) 없는 사이드카는 지운다(완료 승격·수명 삭제 뒤 잔재).
    for m in metas {
        let part = m.with_extension(""); // "{key}.part.meta" → "{key}.part"
        if !part.exists() {
            let _ = std::fs::remove_file(&m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 사이드카 왕복 · 크기 불일치 = None · 제거 = 즉시 소멸.
    #[test]
    fn part_meta_roundtrip_size_check_and_remove() {
        let ch = format!("t-partmeta-{}", std::process::id());
        let secret = [9u8; 32];
        let sender = PeerId::from_bytes([2u8; 32]);
        let key = resume_key("f.bin", 100, sender);
        write_part_meta(&ch, &secret, &key, [1u8; 16], 100, sender, b"f.bin").unwrap();
        assert_eq!(read_part_meta_size(&ch, &secret, &key), Some(100));
        assert!(
            partial_len(&ch, &secret, &key, 200).is_none(),
            "선언 크기 불일치"
        );
        assert!(partial_len(&ch, &secret, &key, 100).is_none(), ".part 없음");
        assert!(
            read_part_meta_size(&ch, &[1u8; 32], &key).is_none(),
            "다른 신원 = 개봉 불가"
        );
        remove_partial(&ch, &key);
        assert!(read_part_meta_size(&ch, &secret, &key).is_none());
        let _ = std::fs::remove_dir_all(crate::gate::quarantine_root(&ch));
    }
}
