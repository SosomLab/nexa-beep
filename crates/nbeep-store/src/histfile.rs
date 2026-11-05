//! 대화 기록 세그먼트 v2 — **증분 프레임**(RL-7 2단 · 11-05 · [ADR-0005 §4] "세그먼트 구조"의 실현).
//!
//! 종전(v1) = 스레드 전량을 한 봉투([`crate::sealed`])로 — 메시지 1건마다 전량 재봉인·전량 쓰기(세션 O(n²)). 1단(디바운스)이
//! 횟수를 줄였고, 2단인 이 모듈은 **쓰는 양**을 새 레코드만으로 줄인다.
//!
//! ```text
//! file = MAGIC "NBH2" ‖ ver u8 ‖ frame*
//! frame = len u32 BE ‖ NBSE 봉투(sealed · 도메인 = history · 키 = 대화별 데이터 키)
//! frame 평문 = 기록 레코드 열(앱 `encode_history` 조각) — 레코드는 자기 구분자라 프레임 평문을 **이어붙여 한 번에** 디코드한다.
//! ```
//!
//! - **추가** = 새 레코드만 봉인해 `append` + `sync_all`(O(new)). **앞부분이 바뀌면**(배달 ack · 수신확인 · 보관 상한으로 앞줄 밀림)
//!   호출자가 전체 재작성([`write_fresh`])한다 — 판정 재료는 [`SegState`](쓴 평문 길이 + 그 digest).
//! - 프레임이 [`COMPACT_FRAMES`] 이상이면 호출자가 재작성한다(읽기 상한 · 파일 비대 방지).
//! - **꼬리 부분 프레임**(append 중 크래시)은 [`open`]이 `valid_len`으로 알려 주고 호출자가 잘라낸다 — 그대로 두면 뒤에 붙는 프레임을
//!   영원히 못 읽는다. 프레임 하나가 AEAD에 실패하면 거기서 멈춘다(fail-soft · 앞 프레임은 살린다 · 같은 신원이면 일어나지 않는다).
//! - 봉투 원리: 헤더·길이만 평문 · 내용은 프레임 안.

use std::io::{self, Write as _};
use std::path::Path;

/// 파일 매직.
pub const MAGIC: [u8; 4] = *b"NBH2";
const VER: u8 = 1;
const HEAD: usize = 5;
/// 프레임 수 상한 — 이 이상이면 호출자가 전량 재작성해 하나로 접는다.
pub const COMPACT_FRAMES: u32 = 32;

/// 마지막으로 **쓴 평문**의 모양 — 다음 쓰기에서 "앞부분 불변 = append 가능" 판정 재료.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegState {
    /// 지금까지 파일에 든 평문 바이트 수(프레임 평문 합).
    pub bytes: usize,
    /// 그 평문의 SHA-256.
    pub digest: [u8; 32],
    /// 프레임 수.
    pub frames: u32,
}

/// 열기 결과.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// 프레임 평문을 이어붙인 것(디코드는 호출자).
    pub plain: Vec<u8>,
    /// 읽어 낸 프레임 수.
    pub frames: u32,
    /// 온전한 프레임까지의 파일 길이 — 이보다 파일이 길면 꼬리가 깨진 것(잘라낸다).
    pub valid_len: usize,
}

/// v2 파일인가(매직 + 버전).
#[must_use]
pub fn is_v2(raw: &[u8]) -> bool {
    raw.len() >= HEAD && raw[..4] == MAGIC && raw[4] == VER
}

/// SHA-256.
#[must_use]
pub fn digest(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

/// 열기 — v2가 아니면 `None`(호출자가 v1 경로로). 프레임은 순서대로 개봉하고 실패·꼬리 손상에서 멈춘다.
#[must_use]
pub fn open(domain: &[u8], key: &[u8; 32], raw: &[u8]) -> Option<Opened> {
    if !is_v2(raw) {
        return None;
    }
    let mut plain = Vec::new();
    let mut frames = 0u32;
    let mut pos = HEAD;
    while pos + 4 <= raw.len() {
        let len = u32::from_be_bytes(raw[pos..pos + 4].try_into().ok()?) as usize;
        let end = match pos.checked_add(4).and_then(|p| p.checked_add(len)) {
            Some(e) if e <= raw.len() => e,
            _ => break, // 꼬리 부분 프레임(append 중 크래시) — 여기까지가 온전
        };
        let Some(p) = crate::sealed::open(domain, key, &raw[pos + 4..end]) else {
            break; // 개봉 실패 — 앞 프레임은 살린다(fail-soft)
        };
        plain.extend_from_slice(&p);
        frames += 1;
        pos = end;
    }
    Some(Opened {
        plain,
        frames,
        valid_len: pos,
    })
}

fn frame(domain: &[u8], key: &[u8; 32], plain: &[u8]) -> io::Result<Vec<u8>> {
    let env = crate::sealed::seal(domain, key, plain)?;
    let mut out = Vec::with_capacity(4 + env.len());
    out.extend_from_slice(
        &u32::try_from(env.len())
            .map_err(|_| io::Error::other("프레임 과대"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&env);
    Ok(out)
}

/// 전량 재작성 — 헤더 + 프레임 1개 · 원자적(temp → rename · 0600).
pub fn write_fresh(path: &Path, domain: &[u8], key: &[u8; 32], plain: &[u8]) -> io::Result<()> {
    let mut out = Vec::with_capacity(HEAD + plain.len() + 64);
    out.extend_from_slice(&MAGIC);
    out.push(VER);
    out.extend_from_slice(&frame(domain, key, plain)?);
    crate::privfile::write_atomic(path, &out)
}

/// 프레임 추가 — 파일 끝에 봉인 프레임 하나를 붙이고 `sync_all`. 파일이 없거나 v2가 아니면 [`write_fresh`]로 간다.
pub fn append(path: &Path, domain: &[u8], key: &[u8; 32], plain: &[u8]) -> io::Result<()> {
    let head = std::fs::File::open(path).and_then(|mut f| {
        use std::io::Read as _;
        let mut h = [0u8; HEAD];
        f.read_exact(&mut h)?;
        Ok(h)
    });
    if !head.as_ref().is_ok_and(|h| is_v2(h)) {
        return write_fresh(path, domain, key, plain);
    }
    let fr = frame(domain, key, plain)?;
    let mut f = std::fs::OpenOptions::new().append(true).open(path)?;
    f.write_all(&fr)?;
    f.sync_all()
}

/// 꼬리 잘라내기(깨진 부분 프레임 제거) — 길이가 이미 같으면 아무 일도 없다.
pub fn truncate_to(path: &Path, len: usize) -> io::Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    if f.metadata()?.len() != len as u64 {
        f.set_len(len as u64)?;
        f.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::path::PathBuf;

    const D: &[u8] = b"history-v1";
    const K: [u8; 32] = [9u8; 32];

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-histfile-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("x.seg")
    }

    #[test]
    fn fresh_then_appends_concatenate_in_order() {
        let p = tmp("append");
        write_fresh(&p, D, &K, b"AAA").unwrap();
        append(&p, D, &K, b"BB").unwrap();
        append(&p, D, &K, b"C").unwrap();
        let raw = std::fs::read(&p).unwrap();
        let o = open(D, &K, &raw).unwrap();
        assert_eq!(o.plain, b"AAABBC");
        assert_eq!(o.frames, 3);
        assert_eq!(o.valid_len, raw.len());
        assert!(!raw.windows(3).any(|w| w == b"AAA"), "평문 누출 0");
        // 다른 키 = 프레임 0(fail-soft · 파일은 v2로 인식).
        let o2 = open(D, &[1u8; 32], &raw).unwrap();
        assert_eq!(o2.frames, 0);
        assert!(o2.plain.is_empty());
        assert_eq!(o2.valid_len, HEAD);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn torn_tail_is_reported_and_truncatable() {
        let p = tmp("torn");
        write_fresh(&p, D, &K, b"first").unwrap();
        append(&p, D, &K, b"second").unwrap();
        let good_len = std::fs::read(&p).unwrap().len();
        // 크래시 모사 — 다음 프레임의 길이 접두 + 절반만 남았다.
        let mut raw = std::fs::read(&p).unwrap();
        let fr = frame(D, &K, b"third").unwrap();
        raw.extend_from_slice(&fr[..fr.len() / 2]);
        std::fs::write(&p, &raw).unwrap();
        let o = open(D, &K, &raw).unwrap();
        assert_eq!(o.plain, b"firstsecond");
        assert_eq!(o.valid_len, good_len);
        truncate_to(&p, o.valid_len).unwrap();
        assert_eq!(std::fs::read(&p).unwrap().len(), good_len);
        append(&p, D, &K, b"third").unwrap();
        let o = open(D, &K, &std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(o.plain, b"firstsecondthird");
        assert_eq!(o.frames, 3);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn append_on_legacy_or_missing_file_rewrites_fresh() {
        let p = tmp("legacy");
        // v1(단일 NBSE 봉투)인 파일에 append → v2로 재작성(옛 내용은 호출자가 평문에 합쳐 넘긴다).
        std::fs::write(&p, crate::sealed::seal(D, &K, b"old").unwrap()).unwrap();
        assert!(
            open(D, &K, &std::fs::read(&p).unwrap()).is_none(),
            "v1 = None"
        );
        append(&p, D, &K, b"oldnew").unwrap();
        let o = open(D, &K, &std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(o.plain, b"oldnew");
        assert_eq!(o.frames, 1);
        let q = p.with_file_name("missing.seg");
        append(&q, D, &K, b"z").unwrap();
        assert_eq!(
            open(D, &K, &std::fs::read(&q).unwrap()).unwrap().plain,
            b"z"
        );
        assert_eq!(digest(b"a"), digest(b"a"));
        assert_ne!(digest(b"a"), digest(b"b"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
