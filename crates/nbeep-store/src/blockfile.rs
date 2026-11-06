//! 수신 제한 목록 세그먼트(`blocks.seg`) — [`BlockList`]의 봉인 영속([docs/38 §4] · BL-1 · 11-05).
//!
//! 설계 문서는 "trust.seg 확장"이라 했지만 **[`crate::sealed`] 봉투(도메인 분리 키)의 별도 파일**로 둔다 —
//! 같은 래핑 원료(기기 신원 키) · 같은 암호군 · 도메인 `blocklist-v1`이라 바꿔치기가 통하지 않고, trust.seg 포맷
//! 판을 올리지 않아 구버전 공존(v1~v3 관용 규칙)이 그대로다. IP·이름은 PII라 평문 3면 원칙 그대로 봉인한다.
//!
//! 규약은 trust.seg와 같다: **변경 즉시 저장**(write-through · 원자적 교체) · 못 읽으면 **잠김**(fail-closed · 메모리
//! 전용 · 파일 보존 — 조용히 빈 목록으로 시작해 덮어쓰면 차단이 증발한다).

use nbeep_core::{BlockAxis, BlockEntry, BlockList};
use std::io;
use std::path::PathBuf;

const DOMAIN: &[u8] = b"blocklist-v1";

/// 열기 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockLoad {
    /// 파일 없음 — 새 목록.
    Fresh,
    /// 정상 로드(항목 수).
    Loaded(usize),
    /// 손상·다른 신원 — 잠김(메모리 전용 · 파일 보존).
    Locked,
}

/// 파일 기반 차단 목록 — 읽기는 [`std::ops::Deref`]로 [`BlockList`] 그대로, 변경은 이 타입의 메서드로(저장 동반).
#[derive(Debug)]
pub struct FileBlockList {
    inner: BlockList,
    path: PathBuf,
    secret: [u8; 32],
    locked: bool,
    write_failed: bool,
}

impl std::ops::Deref for FileBlockList {
    type Target = BlockList;
    fn deref(&self) -> &BlockList {
        &self.inner
    }
}

impl FileBlockList {
    /// 열기 — 없으면 새로, 있으면 개봉, 실패면 잠김.
    #[must_use]
    pub fn open(path: PathBuf, secret: [u8; 32]) -> (Self, BlockLoad) {
        let mut me = Self {
            inner: BlockList::new(),
            path,
            secret,
            locked: false,
            write_failed: false,
        };
        let load = match std::fs::read(&me.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => BlockLoad::Fresh,
            Err(_) => {
                me.locked = true;
                BlockLoad::Locked
            }
            Ok(bytes) => match crate::sealed::open(DOMAIN, &secret, &bytes)
                .and_then(|p| BlockList::decode(&p))
            {
                Some(list) => {
                    let n = list.entries().len();
                    me.inner = list;
                    BlockLoad::Loaded(n)
                }
                None => {
                    me.locked = true;
                    BlockLoad::Locked
                }
            },
        };
        (me, load)
    }

    /// 잠김(fail-closed) 여부.
    #[must_use]
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// 마지막 저장이 실패했는가(IO — 다음 변경에서 재시도).
    #[must_use]
    pub fn write_failed(&self) -> bool {
        self.write_failed
    }

    /// 등재(교체 포함) — 바뀌면 저장.
    pub fn add(&mut self, entry: BlockEntry) -> bool {
        let changed = self.inner.add(entry);
        if changed {
            self.persist();
        }
        changed
    }

    /// 해제 — 있었으면 저장.
    pub fn remove(&mut self, axis: &BlockAxis) -> bool {
        let removed = self.inner.remove(axis);
        if removed {
            self.persist();
        }
        removed
    }

    /// 만료 정리 — 걷어낸 게 있으면 저장.
    pub fn purge_expired(&mut self, now_ms: u64) -> Vec<BlockEntry> {
        let gone = self.inner.purge_expired(now_ms);
        if !gone.is_empty() {
            self.persist();
        }
        gone
    }

    fn persist(&mut self) {
        if self.locked {
            return; // 원본 보존 — 잠긴 동안은 덮어쓰지 않는다.
        }
        self.write_failed = self.write_atomic().is_err();
    }

    fn write_atomic(&self) -> io::Result<()> {
        let sealed = crate::sealed::seal(DOMAIN, &self.secret, &self.inner.encode())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, &sealed)?;
        std::fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

impl Drop for FileBlockList {
    fn drop(&mut self) {
        // R-16 zeroize(11-06).
        crate::sealed::wipe(&mut self.secret);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use nbeep_core::PeerId;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-blockfile-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("blocks.seg")
    }

    fn pid(b: u8) -> PeerId {
        PeerId::from_bytes([b; 32])
    }

    #[test]
    fn entries_survive_reopen_and_wrong_secret_locks() {
        let p = tmp("reopen");
        let s = [3u8; 32];
        let (mut bl, load) = FileBlockList::open(p.clone(), s);
        assert_eq!(load, BlockLoad::Fresh);
        assert!(bl.add(BlockEntry::manual(BlockAxis::Peer(pid(1)), 10, None)));
        assert!(bl.add(BlockEntry::manual(
            BlockAxis::parse_ip("10.0.0.0/8").unwrap(),
            11,
            Some(1_000)
        )));
        assert!(!bl.write_failed());
        let (back, load) = FileBlockList::open(p.clone(), s);
        assert_eq!(load, BlockLoad::Loaded(2));
        assert!(back.is_peer_blocked(pid(1), 999_999));
        assert!(back.is_ip_blocked("10.9.9.9".parse().unwrap(), 500));
        // 다른 신원 = 잠김 · 파일 보존 · 쓰기 거부.
        let before = std::fs::read(&p).unwrap();
        let (mut other, load) = FileBlockList::open(p.clone(), [4u8; 32]);
        assert_eq!(load, BlockLoad::Locked);
        assert!(other.locked());
        other.add(BlockEntry::manual(BlockAxis::Peer(pid(9)), 1, None));
        assert_eq!(
            std::fs::read(&p).unwrap(),
            before,
            "잠긴 동안 덮어쓰지 않는다"
        );
        // 평문 누출 없음.
        assert!(
            !before.windows(4).any(|w| w == b"NBBL"),
            "본문 매직이 평문으로 보이면 봉인 실패"
        );
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn purge_persists_only_when_something_expired() {
        let p = tmp("purge");
        let s = [5u8; 32];
        let (mut bl, _) = FileBlockList::open(p.clone(), s);
        bl.add(BlockEntry::manual(
            BlockAxis::name("x").unwrap(),
            0,
            Some(100),
        ));
        let m1 = std::fs::metadata(&p).unwrap().len();
        assert!(bl.purge_expired(50).is_empty());
        assert_eq!(std::fs::metadata(&p).unwrap().len(), m1);
        assert_eq!(bl.purge_expired(100).len(), 1);
        let (back, load) = FileBlockList::open(p.clone(), s);
        assert_eq!(load, BlockLoad::Loaded(0));
        assert!(back.entries().is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
