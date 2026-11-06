//! **NBSE v2 — 청크 스트림 봉투**(M4-12 ①② · 11-07 · 사용자 결정 11-06 "NBSE v2 청크 스트림").
//!
//! [`sealed`](crate::sealed)(v1)는 파일 전체를 AEAD 1개로 봉인하므로 봉인·개봉에 **원본 크기(S)의 버퍼**가 필요하다.
//! 큰 수신 파일(256MiB)은 그 자체로 예산 게이트(DR-5 유휴 RSS ≤30MB)와 어긋나므로, v2는 평문을 **고정 크기 청크**
//! (기본 256KiB)로 잘라 청크마다 AEAD 하고, 쓰기는 들어오는 대로(스풀) · 읽기는 청크 단위 임의 접근으로 한다.
//! docs/04 §4 "평문 스풀 금지"라 스풀 파일이 곧 이 형식이다(수신 중에도 디스크에 평문이 없다).
//!
//! 형식:
//! ```text
//! head  = "NBSE" ‖ ver(1)=2 ‖ salt(16) ‖ nonce_base(8) ‖ chunk_log2(1)        (30B)
//! chunk = ct(len) ‖ tag(16)      len = CHUNK(비마지막) · 0..=CHUNK(마지막)
//! nonce_i = nonce_base ‖ i(u32 BE)       AAD_i = head ‖ domain ‖ i(u32 BE) ‖ last(1B)
//! key   = SHA-256("nexa-beep/adr-0005/sealed-v2" ‖ domain ‖ salt ‖ wrap_secret)
//! ```
//! - **절단 방지**: 마지막 청크만 `last=1`을 AAD에 넣는다. 파일이 뒤에서 잘리면 읽는 쪽이 마지막이라 믿는 청크의
//!   태그가 맞지 않아(AAD 불일치) **fail-closed**. 재배열·중복도 인덱스가 AAD에 있어 막힌다.
//! - **재개**(`.part` = 스풀 · M4-12 ④): 끝난 스트림을 다시 열어 마지막 청크를 복호해 버퍼로 되돌리고 그 자리부터
//!   이어 쓴다([`StreamWriter::reopen`]) — 평문 전체를 다시 만들지 않는다.
//! - 도메인·비밀은 v1과 같은 계약(도메인이 키 유도와 AAD 양쪽에).

use std::io::{self, Read, Seek, SeekFrom, Write};

use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};

const MAGIC: [u8; 4] = *b"NBSE";
const VER: u8 = 2;
const SALT_LEN: usize = 16;
const BASE_LEN: usize = 8;
/// 머리 길이(30B).
pub const HEAD: usize = 4 + 1 + SALT_LEN + BASE_LEN + 1;
const TAG: usize = 16;
/// 기본 청크(평문) 크기 — 256KiB(log2 = 18). 머리에 기록되므로 읽는 쪽은 파일마다 따른다.
pub const DEFAULT_CHUNK_LOG2: u8 = 18;

fn derive_key(domain: &[u8], salt: &[u8; SALT_LEN], secret: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"nexa-beep/adr-0005/sealed-v2");
    h.update(domain);
    h.update(salt);
    h.update(secret);
    h.finalize().into()
}

/// v2 스트림 봉투인가(머리 매직·버전).
#[must_use]
pub fn is_stream(bytes: &[u8]) -> bool {
    bytes.len() >= HEAD && bytes[..4] == MAGIC && bytes[4] == VER
}

/// 머리 해석 결과 — 청크 기하.
#[derive(Clone, Debug)]
struct Head {
    raw: [u8; HEAD],
    key: [u8; 32],
    base: [u8; BASE_LEN],
    chunk: usize,
}

impl Head {
    fn parse(raw: [u8; HEAD], domain: &[u8], secret: &[u8; 32]) -> Option<Self> {
        if raw[..4] != MAGIC || raw[4] != VER {
            return None;
        }
        let salt: [u8; SALT_LEN] = raw[5..5 + SALT_LEN].try_into().ok()?;
        let base: [u8; BASE_LEN] = raw[5 + SALT_LEN..5 + SALT_LEN + BASE_LEN].try_into().ok()?;
        let log2 = raw[HEAD - 1];
        if !(12..=24).contains(&log2) {
            return None; // 4KiB..16MiB 밖 = 손상
        }
        Some(Self {
            raw,
            key: derive_key(domain, &salt, secret),
            base,
            chunk: 1usize << log2,
        })
    }

    fn nonce(&self, i: u32) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[..BASE_LEN].copy_from_slice(&self.base);
        n[BASE_LEN..].copy_from_slice(&i.to_be_bytes());
        n
    }

    fn aad(&self, domain: &[u8], i: u32, last: bool) -> Vec<u8> {
        let mut a = Vec::with_capacity(HEAD + domain.len() + 5);
        a.extend_from_slice(&self.raw);
        a.extend_from_slice(domain);
        a.extend_from_slice(&i.to_be_bytes());
        a.push(u8::from(last));
        a
    }

    /// 파일 길이 → (청크 수, 평문 길이). 0 < 나머지 < TAG 는 손상.
    fn geometry(&self, file_len: u64) -> Option<(u32, u64)> {
        let body = file_len.checked_sub(HEAD as u64)?;
        let full_sz = (self.chunk + TAG) as u64;
        let full = body / full_sz;
        let rem = body % full_sz;
        let (n, last_plain) = if rem == 0 {
            if full == 0 {
                return None; // 청크 0개 = 마지막 청크조차 없음(finish 전) → 미완
            }
            (full, self.chunk as u64)
        } else if rem < TAG as u64 {
            return None;
        } else {
            (full + 1, rem - TAG as u64)
        };
        let n = u32::try_from(n).ok()?;
        let plain = (u64::from(n) - 1) * self.chunk as u64 + last_plain;
        Some((n, plain))
    }
}

/// 스트림 쓰기 — 들어오는 대로 청크 단위로 봉인해 내보낸다(메모리 = 청크 1개).
pub struct StreamWriter<W: Write> {
    w: W,
    head: Head,
    domain: Vec<u8>,
    cipher: ChaCha20Poly1305,
    buf: Vec<u8>,
    next: u32,
    /// 지금까지 내보낸(확정된) 평문 길이 + 버퍼.
    plain_len: u64,
    finished: bool,
}

impl<W: Write> std::fmt::Debug for StreamWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamWriter")
            .field("next", &self.next)
            .field("plain_len", &self.plain_len)
            .field("pending", &self.buf.len())
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl<W: Write> StreamWriter<W> {
    /// 새 스트림 — 머리를 즉시 쓴다(salt·nonce_base 매번 새로).
    ///
    /// # Errors
    /// OS 난수·쓰기 실패.
    pub fn new(domain: &[u8], secret: &[u8; 32], mut w: W) -> io::Result<Self> {
        Self::with_chunk_log2(domain, secret, DEFAULT_CHUNK_LOG2, &mut w).map(|head| {
            let cipher = ChaCha20Poly1305::new(&head.key.into());
            Self {
                w,
                head,
                domain: domain.to_vec(),
                cipher,
                buf: Vec::new(),
                next: 0,
                plain_len: 0,
                finished: false,
            }
        })
    }

    fn with_chunk_log2(domain: &[u8], secret: &[u8; 32], log2: u8, w: &mut W) -> io::Result<Head> {
        let mut raw = [0u8; HEAD];
        raw[..4].copy_from_slice(&MAGIC);
        raw[4] = VER;
        getrandom::getrandom(&mut raw[5..5 + SALT_LEN + BASE_LEN])
            .map_err(|_| io::Error::other("OS 난수 실패"))?;
        raw[HEAD - 1] = log2;
        w.write_all(&raw)?;
        Head::parse(raw, domain, secret).ok_or_else(|| io::Error::other("머리 구성 실패"))
    }

    /// 지금까지 받은 평문 길이(확정분 + 버퍼).
    #[must_use]
    pub fn plain_len(&self) -> u64 {
        self.plain_len
    }

    /// 평문 추가 — 청크가 차면 즉시 봉인해 내보낸다.
    ///
    /// # Errors
    /// AEAD·쓰기 실패(finish 뒤 호출은 오류).
    pub fn write(&mut self, mut data: &[u8]) -> io::Result<()> {
        if self.finished {
            return Err(io::Error::other("finish 뒤 쓰기"));
        }
        while !data.is_empty() {
            let room = self.head.chunk - self.buf.len();
            let take = room.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            self.plain_len += take as u64;
            if self.buf.len() == self.head.chunk {
                self.flush_chunk(false)?;
            }
        }
        Ok(())
    }

    fn flush_chunk(&mut self, last: bool) -> io::Result<()> {
        let i = self.next;
        let aad = self.head.aad(&self.domain, i, last);
        let nonce = self.head.nonce(i);
        self.cipher
            .encrypt_in_place(Nonce::from_slice(&nonce), &aad, &mut self.buf)
            .map_err(|_| io::Error::other("AEAD 봉인 실패"))?;
        self.w.write_all(&self.buf)?;
        self.buf.clear();
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| io::Error::other("청크 수 상한"))?;
        Ok(())
    }

    /// 마지막 청크(비어 있어도)를 `last=1`로 봉인해 스트림을 닫는다 — 이 뒤로만 완전한 파일이다.
    ///
    /// # Errors
    /// AEAD·쓰기·flush 실패.
    pub fn finish(mut self) -> io::Result<W> {
        if self.finished {
            return Ok(self.w);
        }
        self.flush_chunk(true)?;
        self.finished = true;
        self.w.flush()?;
        Ok(self.w)
    }

    /// 버퍼(아직 봉인 안 된 꼬리) 사본 — 재개·중복 프리픽스 대조용.
    #[must_use]
    pub fn pending(&self) -> &[u8] {
        &self.buf
    }

    /// 확정된(봉인돼 나간) 평문 길이.
    #[must_use]
    pub fn committed_len(&self) -> u64 {
        self.plain_len - self.buf.len() as u64
    }
}

impl StreamWriter<std::fs::File> {
    /// 끝난 스트림을 **이어 쓰기**로 다시 연다(M4-12 ④ `.part` 재개): 마지막 청크를 복호해 버퍼로 되돌리고 그 청크
    /// 자리부터 다시 쓴다(인덱스·nonce 재사용은 **같은 평문 접두**에 한해 안전 — 그 청크는 통째로 다시 봉인되며,
    /// 이어 쓰는 내용은 종전 버퍼 뒤에만 붙는다). 반환 = (writer, 지금까지의 평문 길이).
    ///
    /// # Errors
    /// 열기·해석·복호 실패(= 손상 · fail-closed).
    pub fn reopen(
        domain: &[u8],
        secret: &[u8; 32],
        path: &std::path::Path,
    ) -> io::Result<(Self, u64)> {
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let len = f.metadata()?.len();
        let mut raw = [0u8; HEAD];
        f.read_exact(&mut raw)?;
        let head =
            Head::parse(raw, domain, secret).ok_or_else(|| io::Error::other("v2 머리 아님"))?;
        let (n, plain) = head
            .geometry(len)
            .ok_or_else(|| io::Error::other("청크 기하 손상"))?;
        let cipher = ChaCha20Poly1305::new(&head.key.into());
        // 마지막 청크 복호 → 버퍼.
        let last_i = n - 1;
        let off = HEAD as u64 + u64::from(last_i) * (head.chunk + TAG) as u64;
        f.seek(SeekFrom::Start(off))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let aad = head.aad(domain, last_i, true);
        cipher
            .decrypt_in_place(Nonce::from_slice(&head.nonce(last_i)), &aad, &mut buf)
            .map_err(|_| io::Error::other("마지막 청크 복호 실패"))?;
        f.set_len(off)?;
        f.seek(SeekFrom::End(0))?;
        let mut w = Self {
            w: f,
            head,
            domain: domain.to_vec(),
            cipher,
            buf,
            next: last_i,
            plain_len: plain,
            finished: false,
        };
        // 되돌린 버퍼가 꽉 찬 청크였으면 바로 확정(비마지막)해 둔다.
        if w.buf.len() == w.head.chunk {
            w.flush_chunk(false)?;
        }
        Ok((w, plain))
    }
}

/// 스트림 읽기 — 청크 단위 임의 접근(메모리 = 청크 1개 · 범위 읽기는 걸친 청크만).
pub struct StreamReader<R: Read + Seek> {
    r: R,
    head: Head,
    domain: Vec<u8>,
    cipher: ChaCha20Poly1305,
    chunks: u32,
    plain_len: u64,
}

impl<R: Read + Seek> std::fmt::Debug for StreamReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamReader")
            .field("chunks", &self.chunks)
            .field("plain_len", &self.plain_len)
            .finish_non_exhaustive()
    }
}

impl<R: Read + Seek> StreamReader<R> {
    /// 열기 — 머리·기하 검증(내용 검증은 청크를 읽을 때 · fail-closed). `len` = 파일 전체 길이.
    #[must_use]
    pub fn open(domain: &[u8], secret: &[u8; 32], mut r: R, len: u64) -> Option<Self> {
        let mut raw = [0u8; HEAD];
        r.seek(SeekFrom::Start(0)).ok()?;
        r.read_exact(&mut raw).ok()?;
        let head = Head::parse(raw, domain, secret)?;
        let (chunks, plain_len) = head.geometry(len)?;
        let cipher = ChaCha20Poly1305::new(&head.key.into());
        Some(Self {
            r,
            head,
            domain: domain.to_vec(),
            cipher,
            chunks,
            plain_len,
        })
    }

    /// 평문 전체 길이.
    #[must_use]
    pub fn plain_len(&self) -> u64 {
        self.plain_len
    }

    /// 청크 수.
    #[must_use]
    pub fn chunks(&self) -> u32 {
        self.chunks
    }

    /// 청크 평문 크기.
    #[must_use]
    pub fn chunk_size(&self) -> usize {
        self.head.chunk
    }

    /// i번째 청크 복호(태그·AAD 검증 · 마지막 청크 플래그 포함) — 실패 = None(손상·절단·바꿔치기).
    #[must_use]
    pub fn read_chunk(&mut self, i: u32) -> Option<Vec<u8>> {
        if i >= self.chunks {
            return None;
        }
        let last = i == self.chunks - 1;
        let off = HEAD as u64 + u64::from(i) * (self.head.chunk + TAG) as u64;
        let ct_len = if last {
            let last_plain = self.plain_len - u64::from(i) * self.head.chunk as u64;
            usize::try_from(last_plain).ok()? + TAG
        } else {
            self.head.chunk + TAG
        };
        self.r.seek(SeekFrom::Start(off)).ok()?;
        let mut buf = vec![0u8; ct_len];
        self.r.read_exact(&mut buf).ok()?;
        let aad = self.head.aad(&self.domain, i, last);
        self.cipher
            .decrypt_in_place(Nonce::from_slice(&self.head.nonce(i)), &aad, &mut buf)
            .ok()?;
        Some(buf)
    }

    /// 평문 범위 `[off, off+len)` — 걸친 청크만 읽는다(범위 밖은 잘라 돌려준다 · 손상 = None).
    #[must_use]
    pub fn read_range(&mut self, off: u64, len: usize) -> Option<Vec<u8>> {
        let end = off.saturating_add(len as u64).min(self.plain_len);
        if off >= end {
            return Some(Vec::new());
        }
        let cs = self.head.chunk as u64;
        let mut out = Vec::with_capacity(usize::try_from(end - off).ok()?);
        let mut pos = off;
        while pos < end {
            let i = u32::try_from(pos / cs).ok()?;
            let c = self.read_chunk(i)?;
            let start = usize::try_from(pos - u64::from(i) * cs).ok()?;
            let take = (c.len() - start).min(usize::try_from(end - pos).ok()?);
            out.extend_from_slice(&c[start..start + take]);
            pos += take as u64;
        }
        Some(out)
    }

    /// 처음부터 끝까지 청크마다 `f` — 검증·실체화·해시 스트리밍. 손상 = Err(fail-closed · 이미 넘긴 청크는
    /// 호출자가 버린다).
    ///
    /// # Errors
    /// 복호 실패(손상) 또는 `f`의 오류.
    pub fn for_each_chunk(&mut self, mut f: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        for i in 0..self.chunks {
            let c = self
                .read_chunk(i)
                .ok_or_else(|| io::Error::other("청크 복호 실패(손상·절단)"))?;
            f(&c)?;
        }
        Ok(())
    }

    /// 소유권 반환.
    pub fn into_inner(self) -> R {
        self.r
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::io::Cursor;

    const SECRET: [u8; 32] = [7u8; 32];
    const DOM: &[u8] = b"quarantine-v2";

    fn seal_all(plain: &[u8]) -> Vec<u8> {
        let mut w = StreamWriter::new(DOM, &SECRET, Vec::new()).unwrap();
        // 들쭉날쭉한 조각으로 써서 경계 처리를 흔든다.
        let mut i = 0;
        let mut step = 1;
        while i < plain.len() {
            let e = (i + step).min(plain.len());
            w.write(&plain[i..e]).unwrap();
            i = e;
            step = (step * 3 + 7) % 100_000 + 1;
        }
        w.finish().unwrap()
    }

    fn open_all(bytes: &[u8]) -> Option<Vec<u8>> {
        let mut r = StreamReader::open(DOM, &SECRET, Cursor::new(bytes), bytes.len() as u64)?;
        let mut out = Vec::new();
        r.for_each_chunk(|c| {
            out.extend_from_slice(c);
            Ok(())
        })
        .ok()?;
        Some(out)
    }

    /// 왕복(빈 파일 · 청크 경계 정확히 · 경계 ±1 · 여러 청크) + 범위 읽기 + 기하.
    #[test]
    fn roundtrip_sizes_and_ranges() {
        let chunk = 1usize << DEFAULT_CHUNK_LOG2;
        for n in [0usize, 1, chunk - 1, chunk, chunk + 1, 3 * chunk + 123] {
            let plain: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            let sealed = seal_all(&plain);
            assert!(is_stream(&sealed));
            assert_eq!(open_all(&sealed).unwrap(), plain, "n={n}");
            let mut r = StreamReader::open(DOM, &SECRET, Cursor::new(&sealed), sealed.len() as u64)
                .unwrap();
            assert_eq!(r.plain_len(), n as u64);
            if n > 0 {
                let off = (n / 3) as u64;
                let got = r.read_range(off, chunk + 5).unwrap();
                let end = (off as usize + chunk + 5).min(n);
                assert_eq!(got, plain[off as usize..end], "범위 읽기 n={n}");
                assert_eq!(
                    r.read_range(n as u64 + 10, 4).unwrap(),
                    Vec::<u8>::new(),
                    "범위 밖 = 빈"
                );
            }
        }
    }

    /// fail-closed: 절단(마지막 청크 소실) · 비트 뒤집기 · 다른 도메인/비밀 · 청크 재배열.
    #[test]
    fn truncation_tamper_and_wrong_key_fail_closed() {
        let chunk = 1usize << DEFAULT_CHUNK_LOG2;
        let plain: Vec<u8> = (0..2 * chunk + 10).map(|i| (i % 253) as u8).collect();
        let sealed = seal_all(&plain);
        // 마지막(짧은) 청크를 잘라내면 청크 1이 마지막으로 보이지만 last 플래그가 달라 실패.
        let cut = &sealed[..HEAD + 2 * (chunk + TAG)];
        assert!(open_all(cut).is_none(), "절단 = fail-closed");
        // 비트 뒤집기.
        let mut bad = sealed.clone();
        bad[HEAD + 5] ^= 1;
        assert!(open_all(&bad).is_none());
        // 다른 도메인·비밀.
        let r = StreamReader::open(b"other", &SECRET, Cursor::new(&sealed), sealed.len() as u64);
        assert!(r.is_some_and(|mut r| r.read_chunk(0).is_none()));
        let r = StreamReader::open(DOM, &[8u8; 32], Cursor::new(&sealed), sealed.len() as u64);
        assert!(r.is_some_and(|mut r| r.read_chunk(0).is_none()));
        // 청크 0·1 맞바꾸기(같은 길이) — 인덱스가 AAD에 있어 실패.
        let mut swapped = sealed.clone();
        let (a, b) = (HEAD, HEAD + chunk + TAG);
        let tmp = swapped[a..a + chunk + TAG].to_vec();
        swapped.copy_within(b..b + chunk + TAG, a);
        swapped[b..b + chunk + TAG].copy_from_slice(&tmp);
        assert!(open_all(&swapped).is_none());
        // 끝나지 않은 스트림(머리만) = 기하 없음.
        assert!(
            StreamReader::open(DOM, &SECRET, Cursor::new(&sealed[..HEAD]), HEAD as u64).is_none()
        );
    }

    /// 재개(M4-12 ④): 끝난 스트림을 다시 열어 이어 쓰면 전체 평문이 이어진다 — 마지막 청크가 꽉 찼든 비었든.
    #[test]
    fn reopen_appends_after_finish() {
        let chunk = 1usize << DEFAULT_CHUNK_LOG2;
        let dir = std::env::temp_dir().join(format!("nbeep-sstream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for first in [0usize, 5, chunk, chunk + 1, 2 * chunk] {
            let p = dir.join(format!("s{first}.bin"));
            let a: Vec<u8> = (0..first).map(|i| (i % 7) as u8).collect();
            let b: Vec<u8> = (0..chunk + 9).map(|i| (i % 11) as u8 + 100).collect();
            let f = std::fs::File::create(&p).unwrap();
            let mut w = StreamWriter::new(DOM, &SECRET, f).unwrap();
            w.write(&a).unwrap();
            w.finish().unwrap();
            let (mut w, got) = StreamWriter::reopen(DOM, &SECRET, &p).unwrap();
            assert_eq!(got, first as u64, "재개 시 평문 길이");
            w.write(&b).unwrap();
            w.finish().unwrap();
            let bytes = std::fs::read(&p).unwrap();
            let mut want = a.clone();
            want.extend_from_slice(&b);
            assert_eq!(open_all(&bytes).unwrap(), want, "first={first}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
