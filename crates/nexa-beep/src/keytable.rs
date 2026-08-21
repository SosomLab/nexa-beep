//! **대화별 데이터 키 테이블**(크립토 셰레딩 · docs/17 §3·§7 · D-18 전체 확정 08-22).
//!
//! 기록 세그먼트를 신원 키 파생이 아니라 **대화별 무작위 데이터 키**로 봉인하고,
//! 그 키들을 이 작은 테이블(`data/keys.seg` — 신원 키로 봉인) 하나에 모은다.
//! **대화 삭제 = 키 폐기** — 세그먼트 파일을 지운 뒤 디스크에 바이트가 남아도
//! (SSD 웨어 레벨링·저널링) 키가 없으면 복호 불가능한 난수다. 테이블 자체는
//! 작아서 원자적 재작성의 옛 블록이 회수될 확률이 높지만, **보장하지는 않는다**
//! (docs/17 §7 그대로 — docs/40 알려진 한계에 명시).
//!
//! - 키 id = 세그먼트 파일 stem(`{peer.short()}` · `g-{uid.short()}`) — 파일과 1:1.
//! - **백업은 키 동반**(대화함 백업이 이 파일을 함께 복사 · 복원 = 병합) —
//!   백업해 둔 기록은 복원되고, **백업 없는 삭제만** 셰레딩된다(의미론 확정 08-22).
//! - 테이블 유실·손상 = 그 키로 봉인된 세그먼트를 영영 못 연다(fail-closed).
//!   손상 파일은 `.corrupt`로 비켜 두고 빈 테이블로 시작한다(조용한 삭제 금지).
//!
//! 평문 내부 배치: `[ver 1B][count u32 LE][{stem_len u8 ‖ stem ‖ key 32B}…]`

use std::collections::HashMap;
use std::path::PathBuf;

/// 봉인 도메인(도메인 분리).
const SEAL_KEYTABLE: &[u8] = b"keytable-v1";
const VER: u8 = 1;

/// 키 테이블 — 로드는 부팅 1회, 변경 시마다 원자적 재봉인 저장.
pub(crate) struct KeyTable {
    path: PathBuf,
    wrap: [u8; 32],
    map: HashMap<String, [u8; 32]>,
}

impl KeyTable {
    /// 자리 표시(부팅 로드 전) — 항목 0.
    pub(crate) fn empty() -> Self {
        Self {
            path: PathBuf::new(),
            wrap: [0u8; 32],
            map: HashMap::new(),
        }
    }

    /// 로드 — 없으면 빈 테이블. 개봉 실패(손상·다른 신원)는 `.corrupt`로 비켜
    /// 두고 빈 테이블(조용한 삭제 금지 — 사람이 되찾을 여지).
    pub(crate) fn load(path: PathBuf, wrap: [u8; 32]) -> Self {
        let mut t = Self {
            path,
            wrap,
            map: HashMap::new(),
        };
        let Ok(raw) = std::fs::read(&t.path) else {
            return t;
        };
        match nbeep_store::sealed::open(SEAL_KEYTABLE, &t.wrap, &raw) {
            Some(plain) => t.map = decode(&plain),
            None => {
                let mut aside = t.path.as_os_str().to_os_string();
                aside.push(".corrupt");
                let _ = std::fs::rename(&t.path, PathBuf::from(aside));
            }
        }
        t
    }

    /// 이 대화의 데이터 키 — 없으면 **무작위 생성 + 즉시 저장**(키가 세그보다
    /// 먼저 디스크에 있어야 크래시 창에서 세그만 남는 고아가 안 생긴다).
    pub(crate) fn get_or_create(&mut self, stem: &str) -> [u8; 32] {
        if let Some(k) = self.map.get(stem) {
            return *k;
        }
        // CSPRNG 32B — 그룹 uid 무작위 생성과 같은 관례(Identity = getrandom 유래).
        let key = *nbeep_crypto::Identity::generate().peer_id().as_bytes();
        self.map.insert(stem.to_string(), key);
        self.save();
        key
    }

    /// 조회만(생성 없음) — 개봉 경로용.
    pub(crate) fn get(&self, stem: &str) -> Option<[u8; 32]> {
        self.map.get(stem).copied()
    }

    /// ★ 셰레딩 — 키 폐기 + 즉시 재저장. 이 뒤로 그 세그먼트의 잔존 바이트는
    /// (신원 키가 있어도) 복호 불가.
    pub(crate) fn destroy(&mut self, stem: &str) {
        if self.map.remove(stem).is_some() {
            self.save();
        }
    }

    /// 백업 테이블 병합(대화함 복원 — 복원이 이긴다 규약과 정합: 같은 stem은
    /// **백업 키가 이긴다** — 복원된 세그 파일과 짝이어야 열리기 때문).
    pub(crate) fn merge_from(&mut self, backup: &std::path::Path) -> usize {
        let Ok(raw) = std::fs::read(backup) else {
            return 0;
        };
        let Some(plain) = nbeep_store::sealed::open(SEAL_KEYTABLE, &self.wrap, &raw) else {
            return 0; // 다른 신원의 백업 — fail-closed
        };
        let mut n = 0;
        for (stem, key) in decode(&plain) {
            self.map.insert(stem, key);
            n += 1;
        }
        if n > 0 {
            self.save();
        }
        n
    }

    /// 봉인 + 원자적 쓰기(record_history와 같은 tmp+rename 문법).
    fn save(&self) {
        if self.path.as_os_str().is_empty() {
            return; // empty() 자리 표시 — 저장처 없음
        }
        let mut plain = vec![VER];
        plain.extend_from_slice(&u32::try_from(self.map.len()).unwrap_or(0).to_le_bytes());
        for (stem, key) in &self.map {
            let sb = stem.as_bytes();
            let n = u8::try_from(sb.len()).unwrap_or(u8::MAX);
            plain.push(n);
            plain.extend_from_slice(&sb[..usize::from(n)]);
            plain.extend_from_slice(key);
        }
        let Ok(env) = nbeep_store::sealed::seal(SEAL_KEYTABLE, &self.wrap, &plain) else {
            return;
        };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));
        if std::fs::write(&tmp, &env).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

fn decode(b: &[u8]) -> HashMap<String, [u8; 32]> {
    let mut out = HashMap::new();
    if b.first().copied() != Some(VER) {
        return out;
    }
    let Some(cnt) = b
        .get(1..5)
        .map(|x| u32::from_le_bytes(x.try_into().unwrap_or([0; 4])))
    else {
        return out;
    };
    let mut i = 5usize;
    for _ in 0..cnt {
        let Some(&n) = b.get(i) else { break };
        let n = usize::from(n);
        let Some(stem) = b.get(i + 1..i + 1 + n) else {
            break;
        };
        let Some(key) = b.get(i + 1 + n..i + 33 + n) else {
            break;
        };
        out.insert(
            String::from_utf8_lossy(stem).into_owned(),
            <[u8; 32]>::try_from(key).unwrap_or([0; 32]),
        );
        i += 33 + n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nb-kt-{}-{name}.seg", std::process::id()))
    }

    /// 생성 → 재로드 왕복 · 같은 stem = 같은 키(안정) · 다른 stem = 다른 키.
    #[test]
    fn create_persists_and_reloads() {
        let p = tmp("rt");
        let _ = std::fs::remove_file(&p);
        let wrap = [7u8; 32];
        let (a, b2);
        {
            let mut t = KeyTable::load(p.clone(), wrap);
            a = t.get_or_create("alice");
            b2 = t.get_or_create("g-room");
            assert_eq!(t.get_or_create("alice"), a, "재호출 = 같은 키");
            assert_ne!(a, b2, "대화별 독립 키");
        }
        let t = KeyTable::load(p.clone(), wrap);
        assert_eq!(t.get("alice"), Some(a));
        assert_eq!(t.get("g-room"), Some(b2));
        let _ = std::fs::remove_file(&p);
    }

    /// ★ 셰레딩 계약 — destroy 후 재로드에도 키가 없다(세그 잔존 바이트 = 난수).
    #[test]
    fn destroy_removes_key_durably() {
        let p = tmp("shred");
        let _ = std::fs::remove_file(&p);
        let wrap = [8u8; 32];
        let key;
        {
            let mut t = KeyTable::load(p.clone(), wrap);
            key = t.get_or_create("victim");
            // 실제로 그 키로 봉인한 데이터가 destroy 후 안 열리는지까지 본다.
            let env = nbeep_store::sealed::seal(b"history-v1", &key, b"secret chat").unwrap();
            t.destroy("victim");
            assert!(t.get("victim").is_none());
            // 키를 잃었다 — 신원(wrap)으로도, 테이블로도 못 연다.
            assert!(nbeep_store::sealed::open(b"history-v1", &wrap, &env).is_none());
        }
        let t = KeyTable::load(p.clone(), wrap);
        assert!(t.get("victim").is_none(), "재로드에도 없음");
        let _ = key;
        let _ = std::fs::remove_file(&p);
    }

    /// 다른 신원 봉인·손상 = `.corrupt`로 비켜 두고 빈 테이블(조용한 삭제 금지).
    #[test]
    fn corrupt_table_is_set_aside_not_deleted() {
        let p = tmp("corrupt");
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"garbage-not-sealed").unwrap();
        let t = KeyTable::load(p.clone(), [9u8; 32]);
        assert!(t.get("x").is_none());
        assert!(!p.exists(), "원 파일은 비켜졌다");
        let aside = PathBuf::from(format!("{}.corrupt", p.display()));
        assert!(aside.exists(), "corrupt 보관");
        let _ = std::fs::remove_file(&aside);
    }

    /// 백업 병합 — 백업 키가 이긴다(복원 세그와 짝) · 다른 신원 백업 = 무시.
    #[test]
    fn merge_prefers_backup_keys_and_rejects_foreign() {
        let wrap = [3u8; 32];
        let main = tmp("main");
        let bak = tmp("bak");
        let _ = (std::fs::remove_file(&main), std::fs::remove_file(&bak));
        let bak_key;
        {
            let mut b = KeyTable::load(bak.clone(), wrap);
            bak_key = b.get_or_create("alice");
        }
        let mut m = KeyTable::load(main.clone(), wrap);
        let _local = m.get_or_create("alice"); // 로컬에 다른 키가 이미 있어도
        assert_eq!(m.merge_from(&bak), 1);
        assert_eq!(m.get("alice"), Some(bak_key), "백업 키가 이긴다");
        // 다른 신원의 백업은 개봉 불가 = 병합 0.
        let mut other = KeyTable::load(tmp("other"), [4u8; 32]);
        other.get_or_create("x");
        assert_eq!(m.merge_from(&tmp("other")), 0);
        for p in [main, bak, tmp("other")] {
            let _ = std::fs::remove_file(&p);
        }
    }
}
