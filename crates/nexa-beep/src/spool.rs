//! **격리 스풀 싱크**(M4-12 ①④ · 11-07 · 사용자 결정 11-06 "`.part` = 격리 도메인 통일").
//!
//! 수신 청크를 **NBSE v2 스트림**([`nbeep_store::sealed_stream`] · 도메인 `quarantine-v2`)으로 곧장 디스크에 쓴다 —
//! 메모리 = 청크 1개(256KiB) + 해시 상태. 스트림 평문의 첫 8B는 NXBQ v2 머리([`nbeep_safe::head_v2`])라 **스풀 파일이
//! 그대로 격리물의 앞부분**이다: 완료 = 꼬리(sha·size·메타)를 이어 쓰고 `.beepq`로 rename · 중단 = 스트림을 닫아
//! `.part`로 두고(재개 = [`StreamWriter::reopen`]) · 폐기 = 파일 삭제. docs/04 §4 "평문 스풀 금지"가 그대로 성립한다
//! (수신 중에도 디스크엔 봉인본뿐).
//!
//! 봉투 원리: 이 모듈은 바이트를 쓰고 해시만 센다 — 판정(등급·검사·아카이브)은 게이트([`crate::gate`])가 **완료 뒤**
//! 스풀에서 필요한 범위만 읽어 한다.

use std::fs::File;
use std::io;
use std::path::PathBuf;

use nbeep_core::xfer::{SinkSpec, XferError, XferSink};
use nbeep_core::PeerId;
use nbeep_safe::{head_v2, TailV2, FLAG_SEALED, HEAD_V2};
use nbeep_store::sealed_stream::{StreamReader, StreamWriter};

/// 격리 스풀/격리물 v2 봉투 도메인(`.part`와 `.beepq`가 **같은** 도메인 — 완료가 rename 한 번).
pub(crate) const SEAL_QUARANTINE_V2: &[u8] = b"quarantine-v2";

/// 스풀 싱크 — [`XferSink`] 구현.
pub(crate) struct SpoolSink {
    /// 열린 스트림(finalize/park/discard가 가져간다).
    w: Option<StreamWriter<File>>,
    /// `.part` 경로.
    path: PathBuf,
    /// 본문(NXBQ 머리 제외) 증분 해시(M4-12 ③).
    hasher: nbeep_crypto::Sha256Stream,
    /// 본문 길이(= 스트림 평문 − 머리 8B).
    body_len: u64,
    secret: [u8; 32],
    /// 재개 키(사이드카·삭제용).
    key: [u8; 32],
    channel: &'static str,
    /// finalize/park 됐으면 Drop이 파일을 지우지 않는다.
    kept: bool,
}

impl std::fmt::Debug for SpoolSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpoolSink")
            .field("path", &self.path)
            .field("body_len", &self.body_len)
            .finish_non_exhaustive()
    }
}

impl SpoolSink {
    /// 새 스풀 — NXBQ v2 머리를 쓰고 `.part.meta` 사이드카(id·size·발신자·이름)를 남긴다(재개 판정용).
    pub(crate) fn create(
        channel: &'static str,
        secret: &[u8; 32],
        sender: PeerId,
        spec: &SinkSpec<'_>,
    ) -> io::Result<Self> {
        let key = crate::part::resume_key(&String::from_utf8_lossy(spec.name), spec.size, sender);
        let path = crate::part::part_path(channel, &key);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::part::write_part_meta(channel, secret, &key, spec.id, spec.size, sender, spec.name)?;
        let f = File::create(&path)?;
        let mut w = StreamWriter::new(SEAL_QUARANTINE_V2, secret, f)?;
        w.write(&head_v2(FLAG_SEALED))?;
        Ok(Self {
            w: Some(w),
            path,
            hasher: nbeep_crypto::Sha256Stream::new(),
            body_len: 0,
            secret: *secret,
            key,
            channel,
            kept: false,
        })
    }

    /// 닫힌 `.part`를 **이어 쓰기**로 다시 연다(M4-12 ④) — 본문을 한 번 훑어 해시 상태를 되살린다(설계대로 "재개 시점에
    /// 보존 바이트를 재해시"). 반환 = (싱크, 프리픽스 해시). 본문이 비었거나 손상 = None(호출자가 폐기).
    pub(crate) fn reopen(
        channel: &'static str,
        secret: &[u8; 32],
        key: &[u8; 32],
    ) -> Option<(Self, [u8; 32])> {
        let path = crate::part::part_path(channel, key);
        let (w, plain) = StreamWriter::reopen(SEAL_QUARANTINE_V2, secret, &path).ok()?;
        let body_len = plain.checked_sub(HEAD_V2 as u64)?;
        if body_len == 0 {
            return None;
        }
        let mut me = Self {
            w: Some(w),
            path,
            hasher: nbeep_crypto::Sha256Stream::new(),
            body_len,
            secret: *secret,
            key: *key,
            channel,
            kept: false,
        };
        // 본문 재해시 — 확정 청크(파일) + 대기 버퍼(writer).
        let mut off = 0u64;
        let step = 1u64 << 20;
        while off < body_len {
            let want = usize::try_from((body_len - off).min(step)).ok()?;
            let part = me.body_range(off, want)?;
            if part.is_empty() {
                return None;
            }
            me.hasher.update(&part);
            off += part.len() as u64;
        }
        let prefix_sha = me.hasher.clone().finalize();
        Some((me, prefix_sha))
    }

    /// 본문 범위 읽기(머리 제외 오프셋) — 확정 청크는 별도 핸들로 복호, 대기 버퍼는 writer에서. 범위 밖은 잘린다.
    pub(crate) fn body_range(&mut self, off: u64, len: usize) -> Option<Vec<u8>> {
        let w = self.w.as_ref()?;
        let committed = w.committed_len();
        let poff = off.checked_add(HEAD_V2 as u64)?;
        let pend = poff
            .saturating_add(len as u64)
            .min(HEAD_V2 as u64 + self.body_len);
        if poff >= pend {
            return Some(Vec::new());
        }
        let mut out = Vec::with_capacity(usize::try_from(pend - poff).ok()?);
        if poff < committed {
            let f = File::open(&self.path).ok()?;
            let disk_len = f.metadata().ok()?.len();
            let mut r =
                StreamReader::open_unfinished(SEAL_QUARANTINE_V2, &self.secret, f, disk_len)?;
            let take = usize::try_from(pend.min(committed) - poff).ok()?;
            out.extend_from_slice(&r.read_range(poff, take)?);
        }
        if pend > committed {
            let start = usize::try_from(poff.max(committed) - committed).ok()?;
            let end = usize::try_from(pend - committed).ok()?;
            out.extend_from_slice(w.pending().get(start..end)?);
        }
        Some(out)
    }

    /// 본문 길이.
    pub(crate) fn body_len(&self) -> u64 {
        self.body_len
    }

    /// 지금까지의 본문 해시(M4-12 ③).
    pub(crate) fn body_digest(&self) -> [u8; 32] {
        self.hasher.clone().finalize()
    }

    /// 완료 — 꼬리를 이어 쓰고 스트림을 닫는다. 반환 = `.part` 경로(게이트가 `.beepq`로 rename).
    pub(crate) fn finalize(mut self, tail: &TailV2) -> io::Result<PathBuf> {
        let mut w = self.w.take().ok_or_else(|| io::Error::other("닫힌 스풀"))?;
        w.write(&tail.encode())?;
        let f = w.finish()?;
        f.sync_all()?;
        self.kept = true;
        crate::part::remove_part_meta(self.channel, &self.key);
        Ok(self.path.clone())
    }

    /// 중단 보존(세션 종료) — 스트림을 닫아 `.part`로 둔다(사이드카는 create 때 이미).
    pub(crate) fn park(mut self) -> io::Result<()> {
        let w = self.w.take().ok_or_else(|| io::Error::other("닫힌 스풀"))?;
        if self.body_len == 0 {
            return Ok(()); // 받은 게 없으면 보존 가치 없음 — Drop이 지운다
        }
        w.finish()?;
        self.kept = true;
        Ok(())
    }

    /// 폐기(해시 불일치·취소) — 파일·사이드카 삭제.
    pub(crate) fn discard(self) {
        drop(self); // kept=false → Drop
    }
}

impl Drop for SpoolSink {
    fn drop(&mut self) {
        if !self.kept {
            drop(self.w.take());
            crate::part::remove_partial(self.channel, &self.key);
        }
    }
}

impl XferSink for SpoolSink {
    fn write(&mut self, data: &[u8]) -> Result<(), XferError> {
        let w = self.w.as_mut().ok_or(XferError::Sink)?;
        w.write(data).map_err(|_| XferError::Sink)?;
        self.hasher.update(data);
        self.body_len += data.len() as u64;
        Ok(())
    }
    fn len(&self) -> u64 {
        self.body_len
    }
    fn matches_at(&mut self, offset: u64, data: &[u8]) -> Result<bool, XferError> {
        let have = self.body_range(offset, data.len()).ok_or(XferError::Sink)?;
        Ok(have == data)
    }
    fn digest(&mut self) -> Option<[u8; 32]> {
        Some(self.body_digest())
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// 재개 원료(M4-10 · ④) — 다시 연 스풀 + 프리픽스 길이·해시(Accept 와이어에 싣는다 · 발신측이 대조).
pub(crate) struct Resume {
    pub sink: SpoolSink,
    pub prefix_len: u64,
    pub prefix_sha: [u8; 32],
}

impl std::fmt::Debug for Resume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resume")
            .field("prefix_len", &self.prefix_len)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use nbeep_safe::Meta;

    fn spec<'a>(name: &'a [u8], size: u64) -> SinkSpec<'a> {
        SinkSpec {
            id: [3u8; 16],
            size,
            sha256: [0u8; 32],
            name,
        }
    }

    fn meta(name: &[u8]) -> Meta {
        Meta {
            orig_name: name.to_vec(),
            declared_ext: "bin".into(),
            declared_mime: String::new(),
            detected_kind: "unknown".into(),
            risk: nbeep_core::RiskLevel::Data,
            sender: PeerId::from_bytes([5u8; 32]),
            received_at: 1,
            expires_at: 2,
            scan: nbeep_core::ScanOutcome::Unavailable,
            xfer: String::new(),
        }
    }

    /// 스풀 왕복(M4-12 ①~④): 쓰기 → 중간 범위 읽기(확정+대기 걸침) → 중단 보존(.part) → partial_len → 재개(해시 복원) →
    /// 완료(꼬리) → QFile v2로 열어 본문·메타·해시가 맞는다 · 폐기는 파일을 지운다.
    #[test]
    fn spool_write_park_resume_finalize_roundtrip() {
        let ch: &'static str =
            Box::leak(format!("t-spool-{}", std::process::id()).into_boxed_str());
        let secret = [9u8; 32];
        let sender = PeerId::from_bytes([5u8; 32]);
        let total: usize = 300 * 1024 + 7; // 청크(256KiB) 경계를 넘는 크기
        let data: Vec<u8> = (0..total).map(|i| (i % 241) as u8).collect();
        let name = b"big.bin";
        let key = crate::part::resume_key("big.bin", total as u64, sender);
        let mut s = SpoolSink::create(ch, &secret, sender, &spec(name, total as u64)).unwrap();
        let cut = 270 * 1024;
        s.write(&data[..cut]).unwrap();
        assert_eq!(s.len(), cut as u64);
        // 확정 청크(256KiB)와 대기 버퍼에 걸친 범위.
        assert_eq!(
            s.body_range(256 * 1024 - 10, 100).unwrap(),
            &data[256 * 1024 - 10..256 * 1024 + 90]
        );
        assert!(s.matches_at(0, &data[..cut]).unwrap());
        assert!(!s.matches_at(0, &[0xFF; 16]).unwrap());
        assert_eq!(s.body_digest(), nbeep_crypto::sha256(&data[..cut]));
        s.park().unwrap();
        assert_eq!(
            crate::part::partial_len(ch, &secret, &key, total as u64),
            Some(cut as u64),
            "보존 길이"
        );
        let r = crate::part::open_resume(ch, &secret, &key, total as u64).unwrap();
        assert_eq!(r.prefix_len, cut as u64);
        assert_eq!(r.prefix_sha, nbeep_crypto::sha256(&data[..cut]));
        let mut s = r.sink;
        s.write(&data[cut..]).unwrap();
        assert_eq!(s.body_digest(), nbeep_crypto::sha256(&data));
        let tail = TailV2 {
            content_sha256: nbeep_crypto::sha256(&data),
            original_size: total as u64,
            meta: meta(name),
        };
        let p = s.finalize(&tail).unwrap();
        assert!(
            crate::part::partial_len(ch, &secret, &key, total as u64).is_none(),
            "사이드카 제거"
        );
        let mut q = crate::gate::QFile::open(&p, &secret).unwrap();
        assert_eq!(q.size(), total as u64);
        assert_eq!(q.meta().orig_name, name.to_vec());
        assert_eq!(q.body_range(100, 50).unwrap(), &data[100..150]);
        assert!(q.verify(), "스트림 해시 = 꼬리 sha");
        let mut all = Vec::new();
        q.for_each_body_chunk(|c| {
            all.extend_from_slice(c);
            Ok(())
        })
        .unwrap();
        assert_eq!(all, data);
        // 폐기 = 파일 삭제.
        let s2 = SpoolSink::create(ch, &secret, sender, &spec(b"x.bin", 10)).unwrap();
        let p2 = s2.path.clone();
        s2.discard();
        assert!(!p2.exists());
        let _ = std::fs::remove_dir_all(crate::gate::quarantine_root(ch));
    }
}
