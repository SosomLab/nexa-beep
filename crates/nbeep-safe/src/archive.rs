//! **아카이브 안전 정책** — Zip Slip · 압축 폭탄 방어(FR-S-11 · [docs/04] T-5 · [docs/11] §3).
//!
//! 대전제: **자동 해제하지 않는다.** 이 모듈은 "풀어도 되는가"를 판정하는 **정책 계층**이고,
//! 압축 포맷 파싱은 하지 않는다 — 항목 서술([`EntryDesc`])을 받아 규칙을 적용한다.
//! 포맷 파서(zip 중앙 디렉터리 등)는 어댑터가 맡고, 파서를 바꿔도 이 규칙은 그대로다(DR-21).
//!
//! ## 규칙(전부 fail-closed — "애매하면 거부")
//!
//! | 검사 | 규칙 |
//! |---|---|
//! | **Zip Slip** | 절대 경로·`..`·드라이브 문자·UNC·링크 항목 **거부**. 정규화 결과가 목적 디렉터리의 **자식**이어야 한다 |
//! | **압축 폭탄** | 총 해제 크기·**압축률**·항목 수·중첩 깊이 상한 |
//! | **이름 일관성** | 로컬 헤더와 중앙 디렉터리 이름 불일치 = 거부(파서가 서로 다른 이름을 보는 순간 방어가 무의미) |
//! | **재귀** | 내부 항목도 **다시 게이트를 탄다** — 아카이브 안의 아카이브는 깊이로 센다 |
//!
//! 통과한 이름조차 실체화 시엔 [`crate::sanitize_filename`]을 또 거친다(이중 방어).

/// 아카이브 한 항목의 서술 — 파서가 채운다(이 모듈은 포맷을 모른다).
#[derive(Clone, Debug)]
pub struct EntryDesc {
    /// 중앙 디렉터리 기준 항목 이름(원본 그대로).
    pub name: String,
    /// 로컬 헤더 기준 이름(없으면 `None` — 일관성 검사 생략 불가 포맷용).
    pub local_name: Option<String>,
    /// 압축 크기(바이트).
    pub compressed: u64,
    /// 선언된 해제 크기(바이트) — **선언일 뿐**이라 실제 해제 시에도 상한을 다시 건다.
    pub uncompressed: u64,
    /// 디렉터리 항목인가.
    pub is_dir: bool,
    /// 심볼릭/하드 링크 항목인가(추출 시 대상 밖을 가리킬 수 있다).
    pub is_link: bool,
    /// 이 항목의 중첩 깊이(최상위 아카이브 = 1).
    pub depth: u32,
}

/// 상한 정책 — 기본값은 v1 보수치. 설정으로 조정 가능하게 열어 둔다.
#[derive(Clone, Copy, Debug)]
pub struct ArchivePolicy {
    /// 총 해제 크기 상한(기본 1 GiB).
    pub max_total_uncompressed: u64,
    /// 항목 하나의 압축률 상한(해제/압축 · 기본 200배).
    pub max_ratio: u64,
    /// 항목 수 상한(기본 10,000).
    pub max_entries: usize,
    /// 중첩 깊이 상한(기본 3 — 아카이브 안의 아카이브 안의 아카이브까지).
    pub max_depth: u32,
}

impl Default for ArchivePolicy {
    fn default() -> Self {
        Self {
            max_total_uncompressed: 1024 * 1024 * 1024,
            max_ratio: 200,
            max_entries: 10_000,
            max_depth: 3,
        }
    }
}

/// 거부 사유 — 사용자에게 **왜 막혔는지** 그대로 보여줄 수 있게 구체적으로 남긴다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveReject {
    /// 절대 경로(`/etc/passwd`, `C:\…`, `\\server\share`).
    AbsolutePath(String),
    /// 상위 탈출(`..`).
    PathEscape(String),
    /// 링크 항목(추출 대상 밖을 가리킬 수 있다).
    LinkEntry(String),
    /// 빈 이름·제어문자 등 해석 불가.
    BadName(String),
    /// 로컬 헤더와 중앙 디렉터리 이름 불일치.
    NameMismatch {
        /// 중앙 디렉터리 이름.
        central: String,
        /// 로컬 헤더 이름.
        local: String,
    },
    /// 총 해제 크기 초과.
    TotalTooLarge {
        /// 합계.
        total: u64,
        /// 상한.
        limit: u64,
    },
    /// 압축률 초과(폭탄 징후).
    RatioTooHigh {
        /// 항목 이름.
        name: String,
        /// 실측 압축률.
        ratio: u64,
        /// 상한.
        limit: u64,
    },
    /// 항목 수 초과.
    TooManyEntries {
        /// 항목 수.
        count: usize,
        /// 상한.
        limit: usize,
    },
    /// 중첩 깊이 초과.
    TooDeep {
        /// 깊이.
        depth: u32,
        /// 상한.
        limit: u32,
    },
}

impl core::fmt::Display for ArchiveReject {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AbsolutePath(n) => write!(f, "절대 경로 항목 거부: {n}"),
            Self::PathEscape(n) => write!(f, "상위 경로 탈출 거부: {n}"),
            Self::LinkEntry(n) => write!(f, "링크 항목 거부: {n}"),
            Self::BadName(n) => write!(f, "해석 불가 이름 거부: {n}"),
            Self::NameMismatch { central, local } => {
                write!(f, "이름 불일치 거부: 중앙={central} 로컬={local}")
            }
            Self::TotalTooLarge { total, limit } => {
                write!(f, "총 해제 크기 초과: {total} > {limit}")
            }
            Self::RatioTooHigh { name, ratio, limit } => {
                write!(f, "압축률 초과({name}): {ratio}배 > {limit}배")
            }
            Self::TooManyEntries { count, limit } => {
                write!(f, "항목 수 초과: {count} > {limit}")
            }
            Self::TooDeep { depth, limit } => write!(f, "중첩 깊이 초과: {depth} > {limit}"),
        }
    }
}

impl std::error::Error for ArchiveReject {}

/// 항목 이름을 **목적 디렉터리 안의 상대 경로**로 정규화한다(Zip Slip 방어).
///
/// 통과 조건: 절대 경로가 아니고, 어떤 구간도 `..`가 아니며, 남는 구간이 하나 이상.
/// `.`과 빈 구간은 버린다. 구분자는 `/`·`\` 둘 다 본다(윈도우 아카이브가 섞인다).
///
/// # Errors
/// [`ArchiveReject`] — 절대 경로·탈출·해석 불가.
pub fn safe_entry_path(name: &str) -> Result<String, ArchiveReject> {
    let n = name.trim();
    if n.is_empty() || n.chars().any(char::is_control) {
        return Err(ArchiveReject::BadName(name.to_string()));
    }
    // 절대 경로: POSIX 루트 · 윈도우 드라이브(`C:`) · UNC(`\\host`).
    let bytes = n.as_bytes();
    let win_drive = n.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    if n.starts_with('/') || n.starts_with('\\') || win_drive {
        return Err(ArchiveReject::AbsolutePath(name.to_string()));
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in n.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            // `..`는 정규화로 상쇄하지 않고 **거부**한다 — 상쇄를 허용하면
            // `a/../../x` 같은 입력에서 파서마다 결과가 갈린다(fail-closed).
            ".." => return Err(ArchiveReject::PathEscape(name.to_string())),
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return Err(ArchiveReject::BadName(name.to_string()));
    }
    Ok(parts.join("/"))
}

/// 항목 하나 검사 — 경로 안전성 + 링크 + 이름 일관성 + 압축률 + 깊이.
///
/// # Errors
/// [`ArchiveReject`].
pub fn check_entry(e: &EntryDesc, policy: &ArchivePolicy) -> Result<String, ArchiveReject> {
    if e.is_link {
        return Err(ArchiveReject::LinkEntry(e.name.clone()));
    }
    if e.depth > policy.max_depth {
        return Err(ArchiveReject::TooDeep {
            depth: e.depth,
            limit: policy.max_depth,
        });
    }
    let safe = safe_entry_path(&e.name)?;
    // 로컬 헤더 이름이 있으면 **정규화 결과가 같아야** 한다(둘이 다르면 방어가 무의미).
    if let Some(local) = &e.local_name {
        let local_safe = safe_entry_path(local)?;
        if local_safe != safe {
            return Err(ArchiveReject::NameMismatch {
                central: e.name.clone(),
                local: local.clone(),
            });
        }
    }
    // 압축률 — 압축 0B는 판정 불가라 해제 크기가 있으면 폭탄으로 본다(fail-closed).
    if !e.is_dir && e.uncompressed > 0 {
        // 압축 0B = 나눗셈 불가 → 최대치로 본다(폭탄 취급 · fail-closed).
        let ratio = e.uncompressed.checked_div(e.compressed).unwrap_or(u64::MAX);
        if ratio > policy.max_ratio {
            return Err(ArchiveReject::RatioTooHigh {
                name: e.name.clone(),
                ratio,
                limit: policy.max_ratio,
            });
        }
    }
    Ok(safe)
}

/// 아카이브 전체 검사 — 항목 수·총 해제 크기 + 각 항목. 통과 시 **안전한 상대 경로 목록**.
///
/// 목록 조회 단계에서 쓴다(해제하지 않는다 — FR-S-11). 통과했더라도 실제 해제는
/// 별도 승인 사항이고, 꺼낸 항목은 **각각 다시 무해화 게이트**를 탄다.
///
/// # Errors
/// [`ArchiveReject`] — 하나라도 걸리면 아카이브 전체를 거부한다.
pub fn check_archive(
    entries: &[EntryDesc],
    policy: &ArchivePolicy,
) -> Result<Vec<String>, ArchiveReject> {
    if entries.len() > policy.max_entries {
        return Err(ArchiveReject::TooManyEntries {
            count: entries.len(),
            limit: policy.max_entries,
        });
    }
    let mut total: u64 = 0;
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        out.push(check_entry(e, policy)?);
        total = total.saturating_add(e.uncompressed);
        if total > policy.max_total_uncompressed {
            return Err(ArchiveReject::TotalTooLarge {
                total,
                limit: policy.max_total_uncompressed,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> EntryDesc {
        EntryDesc {
            name: name.to_string(),
            local_name: None,
            compressed: 100,
            uncompressed: 1000,
            is_dir: false,
            is_link: false,
            depth: 1,
        }
    }

    #[test]
    fn zip_slip_variants_rejected() {
        // 고전 Zip Slip + 윈도우/UNC/역슬래시 변종 — 전부 거부.
        for bad in [
            "../../etc/passwd",
            "a/../../b",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "C:\\Windows\\system32\\evil.dll",
            "foo/../../../root/.ssh/authorized_keys",
        ] {
            assert!(safe_entry_path(bad).is_err(), "통과하면 안 된다: {bad}");
        }
    }

    #[test]
    fn benign_names_normalized() {
        assert_eq!(safe_entry_path("docs/a.txt").unwrap(), "docs/a.txt");
        assert_eq!(safe_entry_path("./docs//a.txt").unwrap(), "docs/a.txt");
        assert_eq!(
            safe_entry_path("dir\\sub\\file.txt").unwrap(),
            "dir/sub/file.txt",
            "윈도우 구분자도 같은 규칙"
        );
        assert_eq!(safe_entry_path("보고서.pdf").unwrap(), "보고서.pdf");
    }

    #[test]
    fn empty_and_control_names_rejected() {
        assert!(safe_entry_path("").is_err());
        assert!(safe_entry_path("   ").is_err());
        assert!(safe_entry_path("a\u{0}b").is_err());
        assert!(safe_entry_path("./.").is_err(), "남는 구간이 없다");
    }

    #[test]
    fn link_entries_rejected() {
        let mut e = entry("link");
        e.is_link = true;
        assert_eq!(
            check_entry(&e, &ArchivePolicy::default()),
            Err(ArchiveReject::LinkEntry("link".into()))
        );
    }

    #[test]
    fn name_mismatch_rejected_but_equivalent_forms_pass() {
        let p = ArchivePolicy::default();
        let mut e = entry("dir/a.txt");
        e.local_name = Some("dir/b.txt".into());
        assert!(matches!(
            check_entry(&e, &p),
            Err(ArchiveReject::NameMismatch { .. })
        ));
        // 같은 경로의 다른 표기(`./`·역슬래시)는 정규화 후 같으므로 통과.
        e.local_name = Some(".\\dir\\a.txt".into());
        assert_eq!(check_entry(&e, &p).unwrap(), "dir/a.txt");
    }

    #[test]
    fn compression_bomb_ratio_rejected() {
        let p = ArchivePolicy::default();
        let mut e = entry("bomb.bin");
        e.compressed = 1_000;
        e.uncompressed = 1_000_000; // 1000배
        assert!(matches!(
            check_entry(&e, &p),
            Err(ArchiveReject::RatioTooHigh { .. })
        ));
        // 압축 0B인데 해제 크기가 있으면 판정 불가 = 폭탄 취급(fail-closed).
        e.compressed = 0;
        assert!(matches!(
            check_entry(&e, &p),
            Err(ArchiveReject::RatioTooHigh { .. })
        ));
        // 상식적인 비율은 통과.
        e.compressed = 100_000;
        e.uncompressed = 1_000_000; // 10배
        assert!(check_entry(&e, &p).is_ok());
    }

    #[test]
    fn total_size_entry_count_and_depth_limits() {
        let p = ArchivePolicy {
            max_total_uncompressed: 10_000,
            max_ratio: 200,
            max_entries: 3,
            max_depth: 2,
        };
        // 항목 수 초과.
        let many: Vec<EntryDesc> = (0..4).map(|i| entry(&format!("f{i}"))).collect();
        assert!(matches!(
            check_archive(&many, &p),
            Err(ArchiveReject::TooManyEntries { .. })
        ));
        // 총 해제 크기 초과(1000B × 3 = 3000 통과 · 늘리면 거부).
        let ok: Vec<EntryDesc> = (0..3).map(|i| entry(&format!("f{i}"))).collect();
        assert_eq!(check_archive(&ok, &p).unwrap().len(), 3);
        let mut big = ok;
        big[0].uncompressed = 20_000;
        big[0].compressed = 10_000;
        assert!(matches!(
            check_archive(&big, &p),
            Err(ArchiveReject::TotalTooLarge { .. })
        ));
        // 중첩 깊이 초과.
        let mut deep = entry("inner.zip");
        deep.depth = 3;
        assert!(matches!(
            check_entry(&deep, &p),
            Err(ArchiveReject::TooDeep { .. })
        ));
    }

    #[test]
    fn one_bad_entry_rejects_whole_archive() {
        // 부분 통과가 없다 — 섞여 있으면 전체 거부(fail-closed).
        let entries = vec![entry("ok.txt"), entry("../escape.txt"), entry("ok2.txt")];
        assert!(check_archive(&entries, &ArchivePolicy::default()).is_err());
    }
}
