//! 신뢰 핀 세그먼트 — TOFU 핀의 암호화 영속(M2-5a · FR-S-47 · R-17 해소).
//!
//! [ADR-0005] 결정 그대로:
//! - **핀 목록은 암호화**(§4-2 정정 08-09) — 키 자체는 공개값이지만 "내가 그 키를
//!   안다"는 사실이 곧 **인간관계 목록**이다(H-1 동기화 폴더·H-4 폴더째 전달).
//! - **키 계층**(§3 · DR-20 V1-4) — 마스터 키는 **무작위 생성**, 래핑 키(기기 신원 키
//!   에서 KDF)로 **한 겹 감싼다**. 직접 파생 금지 — 보호 수준 전환(승격 ①②) 시
//!   마스터를 다시 감싸기만 하면 되고 본문 재암호화가 없다.
//! - **기록 본문과 분리된 작은 세그먼트**(§4-2) — 핀은 세션이 서기 전에 필요하다.
//! - ★ **fail-closed**(§4-2) — 못 읽으면(손상·다른 키) 전부 `Unverified`로 취급하고
//!   "잠김"을 보고한다. **"핀이 없다 = 처음 보는 상대"로 조용히 넘어가면 재핀으로
//!   이력이 오염된다.** 잠긴 동안은 파일을 덮어쓰지 않는다(원본 보존).
//!
//! ## 파일 포맷 v1 (`trust.seg`)
//!
//! ```text
//! [magic 4B "NBTS"][ver 1B]
//! [salt 16B]                                  ← 래핑 KDF 솔트(기기별)
//! [wrap_nonce 12B][wrapped_master 48B]        ← AEAD(마스터 32B) · aad = 앞 21B
//! [body_nonce 12B][body ct ..]                ← AEAD(레코드 평문) · aad = 앞 81B
//! ```
//!
//! 레코드 평문: `count u32 BE` · 반복 `{ peer 32B · level u8 · blocked u8 ·
//! name_count u8 · { len u16 BE · UTF-8 }* }`. 저장은 **변경 즉시**(write-through) —
//! 핀 사건은 드물어 디바운스가 불필요하고, 크래시에도 마지막 핀이 남는 쪽이 안전하다.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use nbeep_core::{
    DisplayName, MemoryTrustStore, PeerId, PinRecord, TrustDecision, TrustLevel, TrustStore,
};
use std::io;
use std::path::PathBuf;

const MAGIC: [u8; 4] = *b"NBTS";
/// v2(08-15) = v1 + 레코드 꼬리(목록 고정 fav · 최근 접속/대화 unix ms).
/// v1 파일도 읽는다(꼬리 없음 = 기본값 — 전방·후방 관용, groupfile v2와 같은 문법).
const VER: u8 = 2;
const VER_V1: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// 헤더(매직+버전+솔트) 길이 — 래핑 AEAD의 aad.
const HEAD: usize = 4 + 1 + SALT_LEN;
/// 래핑부까지 길이 — 본문 AEAD의 aad.
const WRAPPED_END: usize = HEAD + NONCE_LEN + 32 + TAG_LEN;

/// 열기 결과 — 앱이 상태바·배지에 그대로 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustLoad {
    /// 파일 없음 — 새 저장소(첫 실행).
    Fresh,
    /// 정상 로드(핀 레코드 수).
    Loaded(usize),
    /// ★ 손상·키 불일치 — **신뢰 목록 잠김**(fail-closed · 파일 보존 · 영속 중단).
    Locked,
    /// ★ 잠긴 옛 세그먼트를 `<이름>.locked`로 **보관 이동**하고 새로 시작(08-19 —
    /// 잠긴 채 메모리 전용으로 돌면 이번 세션 핀이 조용히 증발한다. 삭제 아님).
    Archived,
}

/// 파일 기반 TOFU 저장 — [`MemoryTrustStore`]를 감싸 **변경 즉시** 암호화 저장한다.
/// 도메인 로직은 전부 위임이라 core와 갈라질 수 없다(도메인 코드 변경 0).
#[derive(Debug)]
pub struct FileTrustStore {
    inner: MemoryTrustStore,
    path: PathBuf,
    /// 래핑 키 원료(기기 신원 키 32B). 승격 ①②는 이 원료만 바뀐다.
    wrap_secret: [u8; 32],
    salt: [u8; SALT_LEN],
    /// 마스터 저장 키 — 무작위 생성·래핑 보관(DR-20 V1-4).
    master: [u8; 32],
    /// 잠김(fail-closed) — 참이면 메모리로만 동작하고 파일을 건드리지 않는다.
    locked: bool,
    /// 마지막 저장 실패 여부(IO — 다음 변경에서 재시도).
    write_failed: bool,
    /// 메타(최근 접속·대화)의 마지막 영속 시각 — **60초 스로틀**(발견 비컨마다
    /// 파일을 다시 쓰지 않는다 · 핀·차단·이름은 여전히 즉시 저장).
    meta_saved: Option<std::time::Instant>,
}

impl FileTrustStore {
    /// **부팅용** — 열되, 현 신원으로 못 여는 세그먼트는 `<이름>.locked(-N)`로
    /// **옆으로 보관**(삭제 아님)하고 새 저장소로 시작한다(그룹판과 같은 문법 —
    /// [`crate::FileGroupStore::open_or_archive`] 주석 참조). ⚠ **임시 신원 부팅
    /// (키 파일 손상)에는 쓰지 말 것** — 진짜 주인의 세그먼트를 밀어내면 안 된다.
    #[must_use]
    pub fn open_or_archive(path: PathBuf, wrap_secret: [u8; 32]) -> (Self, TrustLoad) {
        let (store, load) = Self::open(path.clone(), wrap_secret);
        if load != TrustLoad::Locked {
            return (store, load);
        }
        let Some(arch) = crate::archive_name(&path) else {
            return (store, load); // 보관 이름 소진 — 현행 유지(보존 우선)
        };
        if std::fs::rename(&path, &arch).is_err() {
            return (store, load); // 못 옮기면 현행 유지(보존 우선)
        }
        let (fresh, _) = Self::open(path, wrap_secret);
        (fresh, TrustLoad::Archived)
    }

    /// 세그먼트를 열거나 새로 준비한다. 잠김이어도 저장소는 동작한다(메모리 전용).
    #[must_use]
    pub fn open(path: PathBuf, wrap_secret: [u8; 32]) -> (Self, TrustLoad) {
        match std::fs::read(&path) {
            Ok(bytes) => match Self::parse(&bytes, &wrap_secret) {
                Some((salt, master, records)) => {
                    let n = records.len();
                    (
                        Self {
                            inner: MemoryTrustStore::from_records(records),
                            path,
                            wrap_secret,
                            salt,
                            master,
                            locked: false,
                            write_failed: false,
                            meta_saved: None,
                        },
                        TrustLoad::Loaded(n),
                    )
                }
                None => (
                    // ★ 잠김 — 파일은 보존(덮어쓰기 금지), 전부 Unverified로 동작.
                    Self {
                        inner: MemoryTrustStore::new(),
                        path,
                        wrap_secret,
                        salt: [0; SALT_LEN],
                        master: [0; 32],
                        locked: true,
                        write_failed: false,
                        meta_saved: None,
                    },
                    TrustLoad::Locked,
                ),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut salt = [0u8; SALT_LEN];
                let mut master = [0u8; 32];
                // OS CSPRNG 실패는 진행 불가급이지만, 여기서 죽는 대신 잠김으로 강등한다.
                let ok = getrandom::getrandom(&mut salt).is_ok()
                    && getrandom::getrandom(&mut master).is_ok();
                (
                    Self {
                        inner: MemoryTrustStore::new(),
                        path,
                        wrap_secret,
                        salt,
                        master,
                        locked: !ok,
                        write_failed: false,
                        meta_saved: None,
                    },
                    if ok {
                        TrustLoad::Fresh
                    } else {
                        TrustLoad::Locked
                    },
                )
            }
            Err(_) => (
                Self {
                    inner: MemoryTrustStore::new(),
                    path,
                    wrap_secret,
                    salt: [0; SALT_LEN],
                    master: [0; 32],
                    locked: true,
                    write_failed: false,
                    meta_saved: None,
                },
                TrustLoad::Locked,
            ),
        }
    }

    /// 잠김 여부(fail-closed 표시용 — "신뢰 목록 잠김").
    #[must_use]
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// 직전 저장 IO 실패 여부(다음 변경에서 자동 재시도).
    #[must_use]
    pub fn write_failed(&self) -> bool {
        self.write_failed
    }

    // ── 위임(비영속 조회) ──

    /// [`MemoryTrustStore::record_name`] 위임 + 즉시 저장.
    pub fn record_name(&mut self, peer: PeerId, name: DisplayName) {
        self.inner.record_name(peer, name);
        self.persist();
    }

    /// [`MemoryTrustStore::name_conflict`] 위임.
    #[must_use]
    pub fn name_conflict(&self, peer: PeerId, name: &DisplayName) -> Option<PeerId> {
        self.inner.name_conflict(peer, name)
    }

    /// [`MemoryTrustStore::names`] 위임.
    #[must_use]
    pub fn names(&self, peer: PeerId) -> &[DisplayName] {
        self.inner.names(peer)
    }

    /// [`MemoryTrustStore::set_fav`] 위임 + 즉시 저장(목록 고정 — 08-15).
    pub fn set_fav(&mut self, peer: PeerId, fav: bool) {
        if self.inner.set_fav(peer, fav) {
            self.persist();
        }
    }

    /// [`MemoryTrustStore::fav`] 위임.
    #[must_use]
    pub fn fav(&self, peer: PeerId) -> bool {
        self.inner.fav(peer)
    }

    /// [`MemoryTrustStore::note_seen`] 위임 + **60초 스로틀 저장**(발견 비컨이
    /// 잦다 — 메타만으로 파일을 자주 다시 쓰지 않는다).
    pub fn note_seen(&mut self, peer: PeerId, unix_ms: u64) {
        if self.inner.note_seen(peer, unix_ms) {
            self.persist_meta_throttled();
        }
    }

    /// [`MemoryTrustStore::note_chat`] 위임 + 60초 스로틀 저장.
    pub fn note_chat(&mut self, peer: PeerId, unix_ms: u64) {
        if self.inner.note_chat(peer, unix_ms) {
            self.persist_meta_throttled();
        }
    }

    /// [`MemoryTrustStore::meta`] 위임 — (최근 접속, 최근 대화).
    #[must_use]
    pub fn meta(&self, peer: PeerId) -> (u64, u64) {
        self.inner.meta(peer)
    }

    /// 메타 저장 스로틀 — 마지막 저장 후 60초 지났을 때만(그 사이 변경은 다음
    /// 저장 계기(핀·이름·60초 경과)에 자연 합류한다).
    fn persist_meta_throttled(&mut self) {
        let due = self.meta_saved.is_none_or(|t| t.elapsed().as_secs() >= 60);
        if due {
            self.persist();
            self.meta_saved = Some(std::time::Instant::now());
        }
    }

    /// [`MemoryTrustStore::export`] 위임 — 핀 스냅샷(읽기 전용).
    /// 부팅 시 "연결됐던 상대" 열람의 단일 통로다(08-14 프로필 캐시 복원) —
    /// 핀은 세션 성립에서만 생기므로 이 목록이 곧 연결 이력이다.
    #[must_use]
    pub fn export(&self) -> Vec<PinRecord> {
        self.inner.export()
    }

    // ── 직렬화·암호화 ──

    fn wrap_key(salt: &[u8; SALT_LEN], secret: &[u8; 32]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"nexa-beep/adr-0005/wrap-v1");
        h.update(salt);
        h.update(secret);
        h.finalize().into()
    }

    fn parse(
        bytes: &[u8],
        secret: &[u8; 32],
    ) -> Option<([u8; SALT_LEN], [u8; 32], Vec<PinRecord>)> {
        if bytes.len() < WRAPPED_END + NONCE_LEN + TAG_LEN || bytes[..4] != MAGIC {
            return None;
        }
        let ver = bytes[4];
        if ver != VER && ver != VER_V1 {
            return None;
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&bytes[5..HEAD]);
        let wrap = ChaCha20Poly1305::new(&Self::wrap_key(&salt, secret).into());
        let master_pt = wrap
            .decrypt(
                Nonce::from_slice(&bytes[HEAD..HEAD + NONCE_LEN]),
                Payload {
                    msg: &bytes[HEAD + NONCE_LEN..WRAPPED_END],
                    aad: &bytes[..HEAD],
                },
            )
            .ok()?;
        let master: [u8; 32] = master_pt.try_into().ok()?;
        let body = ChaCha20Poly1305::new(&master.into())
            .decrypt(
                Nonce::from_slice(&bytes[WRAPPED_END..WRAPPED_END + NONCE_LEN]),
                Payload {
                    msg: &bytes[WRAPPED_END + NONCE_LEN..],
                    aad: &bytes[..WRAPPED_END],
                },
            )
            .ok()?;
        let records = decode_records(ver, &body)?;
        Some((salt, master, records))
    }

    /// 스냅샷 저장 — 실패는 치명적이지 않다(플래그만 · 다음 변경에서 재시도).
    /// 잠김이면 **아무것도 쓰지 않는다**(손상 원본 보존).
    fn persist(&mut self) {
        if self.locked {
            return;
        }
        self.write_failed = self.write_atomic().is_err();
    }

    fn write_atomic(&self) -> io::Result<()> {
        use std::io::Write as _;
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(&MAGIC);
        out.push(VER);
        out.extend_from_slice(&self.salt);
        let rng_err = || io::Error::other("OS 난수 실패");
        let aead_err = || io::Error::other("AEAD 봉인 실패");
        // 래핑부 — 마스터를 래핑 키로 감싼다.
        let wrap = ChaCha20Poly1305::new(&Self::wrap_key(&self.salt, &self.wrap_secret).into());
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).map_err(|_| rng_err())?;
        let wrapped = wrap
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &self.master,
                    aad: &out[..HEAD],
                },
            )
            .map_err(|_| aead_err())?;
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&wrapped);
        // 본문 — 레코드 평문을 마스터로 봉인.
        let body_pt = encode_records(&self.inner.export());
        let mut bnonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut bnonce).map_err(|_| rng_err())?;
        let body = ChaCha20Poly1305::new(&self.master.into())
            .encrypt(
                Nonce::from_slice(&bnonce),
                Payload {
                    msg: &body_pt[..],
                    aad: &out[..WRAPPED_END],
                },
            )
            .map_err(|_| aead_err())?;
        out.extend_from_slice(&bnonce);
        out.extend_from_slice(&body);
        // 원자적 교체(F-2와 같은 정책 — PID temp → sync → 덮어쓰기 rename).
        if let Some(dir) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&out)?;
            f.sync_all()?;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }
}

impl TrustStore for FileTrustStore {
    fn on_session(&mut self, peer: PeerId) -> TrustDecision {
        let d = self.inner.on_session(peer);
        // 첫 접촉(핀 신설)·미검증→핀 승격이 일어났을 수 있다 — 즉시 저장.
        if d == TrustDecision::FirstContact {
            self.persist();
        }
        d
    }

    fn level(&self, peer: PeerId) -> TrustLevel {
        self.inner.level(peer)
    }

    fn verify(&mut self, peer: PeerId) {
        self.inner.verify(peer);
        self.persist();
    }

    fn unverify(&mut self, peer: PeerId) {
        self.inner.unverify(peer);
        self.persist();
    }

    fn forget(&mut self, peer: PeerId) {
        self.inner.forget(peer);
        self.persist();
    }

    fn block(&mut self, peer: PeerId) {
        self.inner.block(peer);
        self.persist();
    }

    fn is_blocked(&self, peer: PeerId) -> bool {
        self.inner.is_blocked(peer)
    }
}

// ── 레코드 코덱(평문 — AEAD 안쪽) ──

fn encode_records(records: &[PinRecord]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 * records.len() + 4);
    out.extend_from_slice(
        &u32::try_from(records.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for r in records {
        out.extend_from_slice(r.peer.as_bytes());
        out.push(match r.level {
            TrustLevel::Unverified => 0,
            TrustLevel::Pinned => 1,
            TrustLevel::FingerprintVerified => 2,
        });
        out.push(u8::from(r.blocked));
        let names = &r.names[..r.names.len().min(255)];
        out.push(u8::try_from(names.len()).unwrap_or(255));
        for n in names {
            let b = n.as_str().as_bytes();
            let len = u16::try_from(b.len()).unwrap_or(u16::MAX);
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(&b[..usize::from(len)]);
        }
        // v2 꼬리(08-15) — 목록 고정 + 최근 접속/대화(unix ms).
        out.push(u8::from(r.fav));
        out.extend_from_slice(&r.last_seen.to_be_bytes());
        out.extend_from_slice(&r.last_chat.to_be_bytes());
    }
    out
}

fn decode_records(ver: u8, bytes: &[u8]) -> Option<Vec<PinRecord>> {
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let s = bytes.get(*p..*p + n)?;
        *p += n;
        Some(s)
    };
    let count = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
    let mut out = Vec::with_capacity(count.min(4096) as usize);
    for _ in 0..count {
        let peer = PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?);
        let level = match take(&mut p, 1)?[0] {
            0 => TrustLevel::Unverified,
            1 => TrustLevel::Pinned,
            2 => TrustLevel::FingerprintVerified,
            _ => return None,
        };
        let blocked = take(&mut p, 1)?[0] != 0;
        let nn = take(&mut p, 1)?[0];
        let mut names = Vec::with_capacity(usize::from(nn));
        for _ in 0..nn {
            let len = usize::from(u16::from_be_bytes(take(&mut p, 2)?.try_into().ok()?));
            let s = std::str::from_utf8(take(&mut p, len)?).ok()?;
            names.push(DisplayName::parse(s).ok()?);
        }
        // v2 꼬리(08-15) — v1 파일은 기본값(0/false)으로 관용.
        let (fav, last_seen, last_chat) = if ver >= 2 {
            let fav = take(&mut p, 1)?[0] != 0;
            let seen = u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?);
            let chat = u64::from_be_bytes(take(&mut p, 8)?.try_into().ok()?);
            (fav, seen, chat)
        } else {
            (false, 0, 0)
        };
        out.push(PinRecord {
            peer,
            level,
            blocked,
            names,
            fav,
            last_seen,
            last_chat,
        });
    }
    (p == bytes.len()).then_some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-trustseg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("trust.seg")
    }
    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    /// R-17의 핵심 계약 — 핀 → 재시작(재오픈) → `Known(Pinned)`(재핀 아님).
    #[test]
    fn pin_survives_reopen() {
        let path = tmp("survive");
        let secret = [7u8; 32];
        {
            let (mut ts, load) = FileTrustStore::open(path.clone(), secret);
            assert_eq!(load, TrustLoad::Fresh);
            assert_eq!(ts.on_session(pid(1)), TrustDecision::FirstContact);
            ts.record_name(pid(1), DisplayName::parse("bob").unwrap());
            ts.verify(pid(2));
            ts.block(pid(3));
            assert!(!ts.write_failed(), "저장 성공");
        }
        let (mut ts, load) = FileTrustStore::open(path, secret);
        assert_eq!(load, TrustLoad::Loaded(3));
        assert_eq!(
            ts.on_session(pid(1)),
            TrustDecision::Known(TrustLevel::Pinned),
            "재시작 후에도 아는 키 — MITM 창이 닫힌다"
        );
        assert_eq!(ts.names(pid(1)), &[DisplayName::parse("bob").unwrap()]);
        assert_eq!(ts.level(pid(2)), TrustLevel::FingerprintVerified);
        assert_eq!(ts.on_session(pid(3)), TrustDecision::Blocked);
    }

    /// export = 재시작 후 "연결됐던 상대" 열람 통로(08-14 프로필 캐시 복원) —
    /// 핀·이름·차단이 스냅샷에 그대로 실려야 부팅 복원이 차단 필터를 걸 수 있다.
    #[test]
    fn export_carries_pins_names_and_block_after_reopen() {
        let path = tmp("export");
        let secret = [9u8; 32];
        {
            let (mut ts, _) = FileTrustStore::open(path.clone(), secret);
            let _ = ts.on_session(pid(1));
            ts.record_name(pid(1), DisplayName::parse("영희 노트북").unwrap());
            ts.block(pid(2));
        }
        let (ts, _) = FileTrustStore::open(path, secret);
        let recs = ts.export();
        let r1 = recs.iter().find(|r| r.peer == pid(1)).unwrap();
        assert_eq!(
            r1.names.last(),
            Some(&DisplayName::parse("영희 노트북").unwrap())
        );
        assert!(!r1.blocked);
        assert!(recs.iter().find(|r| r.peer == pid(2)).unwrap().blocked);
    }

    /// fail-closed — 다른 기기 키(래핑 원료)로는 열리지 않고 **잠김**이 보고된다.
    /// 잠긴 동안 변경해도 파일이 덮어써지지 않는다(원본 보존).
    #[test]
    fn wrong_secret_locks_and_preserves_file() {
        let path = tmp("lock");
        {
            let (mut ts, _) = FileTrustStore::open(path.clone(), [1u8; 32]);
            ts.on_session(pid(1));
        }
        let before = std::fs::read(&path).unwrap();
        let (mut ts, load) = FileTrustStore::open(path.clone(), [2u8; 32]);
        assert_eq!(load, TrustLoad::Locked);
        assert!(ts.locked());
        assert_eq!(ts.level(pid(1)), TrustLevel::Unverified, "전부 미검증 취급");
        ts.on_session(pid(9)); // 메모리로는 동작
        assert_eq!(std::fs::read(&path).unwrap(), before, "원본 무손상");
    }

    /// 손상 파일도 잠김(원본 보존) — 조용한 재핀 오염이 없다.
    #[test]
    fn corrupt_file_locks() {
        let path = tmp("corrupt");
        std::fs::write(&path, b"NBTSgarbage-not-a-segment").unwrap();
        let (ts, load) = FileTrustStore::open(path.clone(), [1u8; 32]);
        assert_eq!(load, TrustLoad::Locked);
        assert!(ts.locked());
        assert!(std::fs::read(&path).unwrap().starts_with(b"NBTS"));
    }

    /// 파일은 평문 이름을 노출하지 않는다(§4-2 — 인간관계 목록 보호).
    #[test]
    fn file_does_not_leak_names_or_peers() {
        let path = tmp("leak");
        let (mut ts, _) = FileTrustStore::open(path.clone(), [1u8; 32]);
        ts.record_name(pid(1), DisplayName::parse("홍길동의 맥북").unwrap());
        let raw = std::fs::read(&path).unwrap();
        let name_utf8 = "홍길동의 맥북".as_bytes();
        assert!(
            !raw.windows(name_utf8.len()).any(|w| w == name_utf8),
            "이름 평문 노출 금지"
        );
        let peer_bytes = pid(1);
        assert!(
            !raw.windows(32).any(|w| w == peer_bytes.as_bytes()),
            "PeerId 평문 노출 금지"
        );
    }

    /// v1 파일 관용(08-15 · v2) — 꼬리(고정·시각) 없는 레코드를 기본값으로 읽는다.
    /// 업그레이드 첫 실행이 기존 핀을 잃으면 안 된다(R-17과 같은 무게).
    #[test]
    fn v1_records_read_with_default_tail() {
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(pid(3).as_bytes());
        b.push(1); // Pinned
        b.push(0); // blocked 아님
        b.push(0); // 이름 0개
        let recs = decode_records(VER_V1, &b).unwrap();
        assert_eq!(recs.len(), 1);
        assert!(!recs[0].fav);
        assert_eq!((recs[0].last_seen, recs[0].last_chat), (0, 0));
        // 같은 바이트를 v2로 읽으면 길이 부족 = 거부(버전이 코덱을 가른다).
        assert!(decode_records(VER, &b).is_none());
    }

    /// 레코드 코덱 왕복(경계 — 이름 0·다수·비ASCII).
    #[test]
    fn record_codec_roundtrip() {
        let records = vec![
            PinRecord {
                peer: pid(1),
                level: TrustLevel::Pinned,
                blocked: false,
                names: vec![],
                fav: false,
                last_seen: 0,
                last_chat: 0,
            },
            PinRecord {
                peer: pid(2),
                level: TrustLevel::FingerprintVerified,
                blocked: true,
                names: vec![
                    DisplayName::parse("bob").unwrap(),
                    DisplayName::parse("鮑勃").unwrap(),
                ],
                fav: true,
                last_seen: 1_723_000_000_000,
                last_chat: 42,
            },
        ];
        let enc = encode_records(&records);
        assert_eq!(decode_records(VER, &enc).unwrap(), records);
        // 꼬리 쓰레기는 거부(길이 정합).
        let mut bad = enc;
        bad.push(0);
        assert!(decode_records(VER, &bad).is_none());
    }
}
