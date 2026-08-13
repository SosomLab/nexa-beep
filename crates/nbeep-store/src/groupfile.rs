//! 그룹 세그먼트 — 로컬 그룹의 암호화 영속(M5-1 · FR-G-1 "재시작 후 유지").
//!
//! [`trustfile`](crate::trustfile)과 같은 결이다 — **그룹 이름·구성원 목록은 곧
//! 인간관계 목록**이라(ADR-0005 §4-2의 핀 목록과 같은 민감도) 평문으로 두지 않는다.
//! 컨테이너 구조(키 계층·fail-closed·원자적 쓰기)도 동일하고 매직·레코드 코덱만 다르다.
//! (같은 구조의 2번째 사용 — 3번째 사용자(M2-5b 기록 저장)가 생기면 컨테이너를 추출한다
//! · [docs/13 §12] 중복 규칙.)
//!
//! ## 파일 포맷 v1 (`groups.seg`)
//!
//! ```text
//! [magic 4B "NBGS"][ver 1B]
//! [salt 16B]                                  ← 래핑 KDF 솔트(기기별)
//! [wrap_nonce 12B][wrapped_master 48B]        ← AEAD(마스터 32B) · aad = 앞 21B
//! [body_nonce 12B][body ct ..]                ← AEAD(레코드 평문) · aad = 앞 81B
//! ```
//!
//! 레코드 평문: `next u32 BE` · `count u32 BE` · 반복 `{ id u32 BE ·
//! name(len u16 BE · UTF-8) · member_count u32 BE · { peer 32B }* }`.
//! 저장은 **변경 즉시**(write-through) — 그룹 편집은 드물어 디바운스가 불필요하고,
//! 크래시에도 마지막 편집이 남는 쪽이 안전하다(trust.seg와 같은 판단).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use nbeep_core::group::{Group, GroupId, GroupStore};
use nbeep_core::{DisplayName, PeerId};
use std::io;
use std::path::PathBuf;

const MAGIC: [u8; 4] = *b"NBGS";
const VER: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// 헤더(매직+버전+솔트) 길이 — 래핑 AEAD의 aad.
const HEAD: usize = 4 + 1 + SALT_LEN;
/// 래핑부까지 길이 — 본문 AEAD의 aad.
const WRAPPED_END: usize = HEAD + NONCE_LEN + 32 + TAG_LEN;

/// 열기 결과 — 앱이 상태바에 그대로 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupLoad {
    /// 파일 없음 — 새 저장소(첫 실행).
    Fresh,
    /// 정상 로드(그룹 수).
    Loaded(usize),
    /// ★ 손상·키 불일치 — **그룹 목록 잠김**(fail-closed · 파일 보존 · 영속 중단).
    Locked,
}

/// 파일 기반 그룹 저장 — [`GroupStore`]를 감싸 **변경 즉시** 암호화 저장한다.
/// 도메인 로직은 전부 위임(도메인 코드 변경 0 — trustfile과 같은 문법).
#[derive(Debug)]
pub struct FileGroupStore {
    inner: GroupStore,
    path: PathBuf,
    /// 래핑 키 원료(기기 신원 키 32B — trust.seg와 같은 원료).
    wrap_secret: [u8; 32],
    salt: [u8; SALT_LEN],
    /// 마스터 저장 키 — 무작위 생성·래핑 보관.
    master: [u8; 32],
    /// 잠김(fail-closed) — 참이면 메모리로만 동작하고 파일을 건드리지 않는다.
    locked: bool,
    /// 마지막 저장 실패 여부(IO — 다음 변경에서 재시도).
    write_failed: bool,
}

impl FileGroupStore {
    /// 세그먼트를 열거나 새로 준비한다. 잠김이어도 저장소는 동작한다(메모리 전용).
    #[must_use]
    pub fn open(path: PathBuf, wrap_secret: [u8; 32]) -> (Self, GroupLoad) {
        match std::fs::read(&path) {
            Ok(bytes) => match Self::parse(&bytes, &wrap_secret) {
                Some((salt, master, store)) => {
                    let n = store.list().len();
                    (
                        Self {
                            inner: store,
                            path,
                            wrap_secret,
                            salt,
                            master,
                            locked: false,
                            write_failed: false,
                        },
                        GroupLoad::Loaded(n),
                    )
                }
                None => (
                    // ★ 잠김 — 파일은 보존(덮어쓰기 금지), 이번 실행은 메모리 전용.
                    Self {
                        inner: GroupStore::new(),
                        path,
                        wrap_secret,
                        salt: [0; SALT_LEN],
                        master: [0; 32],
                        locked: true,
                        write_failed: false,
                    },
                    GroupLoad::Locked,
                ),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut salt = [0u8; SALT_LEN];
                let mut master = [0u8; 32];
                let ok = getrandom::getrandom(&mut salt).is_ok()
                    && getrandom::getrandom(&mut master).is_ok();
                (
                    Self {
                        inner: GroupStore::new(),
                        path,
                        wrap_secret,
                        salt,
                        master,
                        locked: !ok,
                        write_failed: false,
                    },
                    if ok {
                        GroupLoad::Fresh
                    } else {
                        GroupLoad::Locked
                    },
                )
            }
            Err(_) => (
                Self {
                    inner: GroupStore::new(),
                    path,
                    wrap_secret,
                    salt: [0; SALT_LEN],
                    master: [0; 32],
                    locked: true,
                    write_failed: false,
                },
                GroupLoad::Locked,
            ),
        }
    }

    /// 잠김 여부(fail-closed 표시용 — "그룹 목록 잠김").
    #[must_use]
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// 직전 저장 IO 실패 여부(다음 변경에서 자동 재시도).
    #[must_use]
    pub fn write_failed(&self) -> bool {
        self.write_failed
    }

    // ── 변경 위임 + 즉시 저장 ──

    /// [`GroupStore::create`] 위임 + 즉시 저장.
    pub fn create(&mut self, name: DisplayName) -> GroupId {
        let id = self.inner.create(name);
        self.persist();
        id
    }

    /// [`GroupStore::rename`] 위임 + 즉시 저장.
    pub fn rename(&mut self, id: GroupId, name: DisplayName) -> bool {
        let ok = self.inner.rename(id, name);
        if ok {
            self.persist();
        }
        ok
    }

    /// [`GroupStore::add_member`] 위임 + 즉시 저장.
    pub fn add_member(&mut self, id: GroupId, peer: PeerId) -> bool {
        let ok = self.inner.add_member(id, peer);
        if ok {
            self.persist();
        }
        ok
    }

    /// [`GroupStore::remove_member`] 위임 + 즉시 저장.
    pub fn remove_member(&mut self, id: GroupId, peer: PeerId) -> bool {
        let ok = self.inner.remove_member(id, peer);
        if ok {
            self.persist();
        }
        ok
    }

    /// [`GroupStore::delete`] 위임 + 즉시 저장.
    pub fn delete(&mut self, id: GroupId) -> bool {
        let ok = self.inner.delete(id);
        if ok {
            self.persist();
        }
        ok
    }

    // ── 조회 위임 ──

    /// [`GroupStore::get`] 위임.
    #[must_use]
    pub fn get(&self, id: GroupId) -> Option<&Group> {
        self.inner.get(id)
    }

    /// [`GroupStore::list`] 위임.
    #[must_use]
    pub fn list(&self) -> &[(GroupId, Group)] {
        self.inner.list()
    }

    // ── 직렬화·암호화(트러스트 세그먼트와 같은 컨테이너 · 매직만 다름) ──

    fn wrap_key(salt: &[u8; SALT_LEN], secret: &[u8; 32]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"nexa-beep/m5-1/group-wrap-v1");
        h.update(salt);
        h.update(secret);
        h.finalize().into()
    }

    fn parse(bytes: &[u8], secret: &[u8; 32]) -> Option<([u8; SALT_LEN], [u8; 32], GroupStore)> {
        if bytes.len() < WRAPPED_END + NONCE_LEN + TAG_LEN || bytes[..4] != MAGIC || bytes[4] != VER
        {
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
        let store = decode_store(&body)?;
        Some((salt, master, store))
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
        let body_pt = encode_store(&self.inner);
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

// ── 레코드 코덱(평문 — AEAD 안쪽) ──

fn encode_store(store: &GroupStore) -> Vec<u8> {
    let (next, list) = store.export();
    let mut out = Vec::with_capacity(64 * list.len() + 8);
    out.extend_from_slice(&next.to_be_bytes());
    out.extend_from_slice(&u32::try_from(list.len()).unwrap_or(u32::MAX).to_be_bytes());
    for (id, name, members) in &list {
        out.extend_from_slice(&id.0.to_be_bytes());
        let b = name.as_str().as_bytes();
        let len = u16::try_from(b.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&b[..usize::from(len)]);
        out.extend_from_slice(
            &u32::try_from(members.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for m in members {
            out.extend_from_slice(m.as_bytes());
        }
    }
    out
}

fn decode_store(bytes: &[u8]) -> Option<GroupStore> {
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let s = bytes.get(*p..*p + n)?;
        *p += n;
        Some(s)
    };
    let next = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
    let count = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
    let mut list = Vec::with_capacity(count.min(4096) as usize);
    for _ in 0..count {
        let id = GroupId(u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?));
        let len = usize::from(u16::from_be_bytes(take(&mut p, 2)?.try_into().ok()?));
        let name = DisplayName::parse(std::str::from_utf8(take(&mut p, len)?).ok()?).ok()?;
        let mc = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
        let mut members = Vec::with_capacity(mc.min(4096) as usize);
        for _ in 0..mc {
            members.push(PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?));
        }
        list.push((id, name, members));
    }
    (p == bytes.len()).then(|| GroupStore::from_export(next, list))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nbeep-groupseg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("groups.seg")
    }
    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn name(s: &str) -> DisplayName {
        DisplayName::parse(s).unwrap()
    }

    /// FR-G-1의 핵심 계약 — 그룹 생성·구성원 편집 → 재시작(재오픈) → 그대로.
    #[test]
    fn groups_survive_reopen() {
        let path = tmp("survive");
        let secret = [7u8; 32];
        let gid = {
            let (mut gs, load) = FileGroupStore::open(path.clone(), secret);
            assert_eq!(load, GroupLoad::Fresh);
            let g = gs.create(name("개발팀"));
            gs.add_member(g, pid(1));
            gs.add_member(g, pid(2));
            gs.remove_member(g, pid(2));
            let _trash = gs.create(name("임시"));
            assert!(gs.delete(_trash));
            assert!(!gs.write_failed(), "저장 성공");
            g
        };
        let (mut gs, load) = FileGroupStore::open(path, secret);
        assert_eq!(load, GroupLoad::Loaded(1));
        let g = gs.get(gid).unwrap();
        assert_eq!(g.name.as_str(), "개발팀");
        assert_eq!(g.members(), vec![pid(1)]);
        // 재오픈 후 새 그룹 id가 기존과 충돌하지 않는다(next 승계).
        let g2 = gs.create(name("영업팀"));
        assert_ne!(g2, gid);
    }

    /// fail-closed — 다른 기기 키로는 열리지 않고 잠김 · 잠긴 동안 파일 불변.
    #[test]
    fn wrong_secret_locks_and_preserves_file() {
        let path = tmp("lock");
        {
            let (mut gs, _) = FileGroupStore::open(path.clone(), [1u8; 32]);
            let g = gs.create(name("g"));
            gs.add_member(g, pid(1));
        }
        let before = std::fs::read(&path).unwrap();
        let (mut gs, load) = FileGroupStore::open(path.clone(), [2u8; 32]);
        assert_eq!(load, GroupLoad::Locked);
        assert!(gs.locked());
        assert!(gs.list().is_empty(), "메모리 전용 빈 상태");
        let _ = gs.create(name("x")); // 메모리로는 동작
        assert_eq!(std::fs::read(&path).unwrap(), before, "원본 무손상");
    }

    /// 파일은 그룹 이름·구성원 PeerId를 평문으로 노출하지 않는다(인간관계 목록 보호).
    #[test]
    fn file_does_not_leak_names_or_peers() {
        let path = tmp("leak");
        let (mut gs, _) = FileGroupStore::open(path.clone(), [1u8; 32]);
        let g = gs.create(name("비밀 프로젝트팀"));
        gs.add_member(g, pid(9));
        let raw = std::fs::read(&path).unwrap();
        let n = "비밀 프로젝트팀".as_bytes();
        assert!(!raw.windows(n.len()).any(|w| w == n), "이름 평문 노출 금지");
        assert!(
            !raw.windows(32).any(|w| w == pid(9).as_bytes()),
            "PeerId 평문 노출 금지"
        );
    }

    /// 코덱 왕복 + 꼬리 쓰레기 거부.
    #[test]
    fn store_codec_roundtrip() {
        let mut s = GroupStore::new();
        let a = s.create(name("a"));
        s.add_member(a, pid(1));
        s.add_member(a, pid(2));
        let _b = s.create(name("한국어 그룹"));
        let enc = encode_store(&s);
        let back = decode_store(&enc).unwrap();
        assert_eq!(back.export(), s.export());
        let mut bad = enc;
        bad.push(0);
        assert!(decode_store(&bad).is_none());
    }
}
