//! nexa-conf — 설정 직렬화·영속 표준 (ADR-0011 · DR-27 · FR-P-8~12).
//!
//! **재사용 크레이트다** — 도메인 타입·타이머 API·경로 싱글톤·UI에 의존하지 않는다
//! (docs/28 §5). 앱이 소유하는 것: 키·기본값·검증(Entry 레지스트리) · `tick(now)` 호출 ·
//! 저장 경로 결정. 이 크레이트가 소유하는 것: 포맷 · 미지 키 보존 · 저장 스케줄 판정 ·
//! 원자적 쓰기.
//!
//! ## 포맷 (docs/28 §4-1)
//! UTF-8 · 한 줄 = `key=value` · `#` 주석 · `=` 최초 1회 분할 · 버전은 주석이 아니라
//! **실제 키 `_schema`**(파서가 읽을 수 있어야 마이그레이션 디스패치가 선다).
//!
//! ## 과부하 방지 (docs/28 §3 — 요구 ②의 핵심)
//! 변경은 [`SaveScheduler::mark`]로 **플래그만** 세운다. 중간 상태를 저장하지 않으므로
//! "최종 요청만 남기고 버린다"가 **자료구조 없이** 성립한다(큐 없음 — NFR-B-6).
//! 발화 = `조용해진 지 quiet_ms` **OR** `첫 미저장 변경 후 max_delay_ms`(F-3 starvation 정정).
//! 시계는 호스트가 `Instant`로 주입한다 — ADR의 trait 스케치는 `mark()`가 무인자지만,
//! quiet 판정에 변경 시각이 필요해 **mark도 호스트 시계를 받는다**(시계 비소유 원칙 동일).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 현재 파일 스키마 버전(`_schema` 키로 기록).
pub const SCHEMA: u32 = 1;

/// 파싱 결과 — 실패하지 않는다(docs/28 §4-3: 손상 파일에도 앱이 뜬다).
#[derive(Debug, Default)]
pub struct Doc {
    /// `_schema` 키 값(없거나 손상 = 0).
    pub schema: u32,
    /// `key=value` 쌍(파일 순서 보존 · `_schema` 제외).
    pub pairs: Vec<(String, String)>,
}

/// 관용 파싱 — 주석(`#`)·빈 줄·`=` 없는 줄은 건너뛴다. CRLF 허용.
#[must_use]
pub fn parse(text: &str) -> Doc {
    let mut doc = Doc::default();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        if k == "_schema" {
            doc.schema = v.trim().parse().unwrap_or(0);
        } else {
            // 줄 분리자(U+2028/2029)로 저장된 개행을 복원(08-18 — 멀티라인 값
            // 왕복 · serialize의 역). 경로의 역슬래시와 충돌하지 않는다.
            let v = v.replace('\u{2028}', "\n").replace('\u{2029}', "\r");
            doc.pairs.push((k.to_string(), v));
        }
    }
    doc
}

/// 직렬화 — `_schema` 먼저, 아는 키(호출자 순서), 그 뒤 미지 키 **그대로 재방출**
/// (F-1 정정: 구버전이 저장해도 신버전 키가 살아남는다).
/// 값 속 개행은 **줄 분리자 U+2028/2029로 치환**해 한 줄 = 한 키 불변식을 지키되
/// **왕복 보존**한다(08-18 — 멀티라인 소개글 등). `str::lines()`는 U+2028/2029로
/// 줄을 나누지 않으므로 값이 한 물리 줄에 머문다. parse가 역치환한다. ★ `\n`→`\\n`
/// 이스케이프는 Windows 경로(`C:\new` → `\n`)와 충돌하므로 쓰지 않는다.
#[must_use]
pub fn serialize(known: &[(&str, &str)], unknown: &[(String, String)]) -> String {
    let mut out = String::with_capacity(64 + (known.len() + unknown.len()) * 24);
    out.push_str("_schema=");
    out.push_str(&SCHEMA.to_string());
    out.push('\n');
    let mut line = |k: &str, v: &str| {
        out.push_str(k);
        out.push('=');
        for ch in v.chars() {
            out.push(match ch {
                '\n' => '\u{2028}',
                '\r' => '\u{2029}',
                c => c,
            });
        }
        out.push('\n');
    };
    for (k, v) in known {
        line(k, v);
    }
    for (k, v) in unknown {
        line(k, v);
    }
    out
}

// ---------------------------------------------------------------- 원자적 쓰기

/// temp 이름 충돌 방지용 프로세스 내 일련번호(동시 인스턴스는 PID로 갈린다).
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 원자적 쓰기(F-2 정정) — PID·일련 접미 temp → `sync_all` → **덮어쓰기 rename**
/// (Unix rename·Windows `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` — std가 보장) →
/// 부모 디렉터리 fsync(Unix — rename 내구성 · Windows는 해당 API 없음).
/// 실패 시 temp를 지우고 기존 파일은 건드리지 않는다.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    fs::create_dir_all(&dir)?;
    let base = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("settings.cfg");
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{base}.{}.{seq}.tmp", std::process::id()));
    let write = (|| -> io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()
    })();
    if let Err(e) = write {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

// ------------------------------------------------------------ 저장 스케줄러

/// 저장 과부하 방지(docs/28 §3) — 변경 N회 → 쓰기 1회.
///
/// 발화 조건은 **OR**: `now - 마지막 변경 ≥ quiet` **또는**
/// `now - 첫 미저장 변경 ≥ max_delay`(연속 변경 starvation 방지 · F-3).
/// `tick`/`flush_now`가 참을 돌려주면 dirty가 소비된다(mem::take와 같은 한 동작 수거).
#[derive(Debug)]
pub struct SaveScheduler {
    quiet: Duration,
    max_delay: Duration,
    last_change: Option<Instant>,
    first_unsaved: Option<Instant>,
}

impl SaveScheduler {
    /// 기본 quiet 1s · max_delay 10s (docs/28 §3-3 · Q-28-2에서 실사용 후 조정).
    #[must_use]
    pub fn new(quiet_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            quiet: Duration::from_millis(quiet_ms),
            max_delay: Duration::from_millis(max_delay_ms),
            last_change: None,
            first_unsaved: None,
        }
    }

    /// 변경 발생 — 플래그만 세운다(직렬화·값 복사 없음).
    pub fn mark(&mut self, now: Instant) {
        self.last_change = Some(now);
        self.first_unsaved.get_or_insert(now);
    }

    /// 미저장 변경이 있는가(발화 조건과 무관).
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.last_change.is_some()
    }

    /// 지금 저장할 때인가 — 참이면 dirty를 소비한다(저장은 호출자 몫).
    pub fn tick(&mut self, now: Instant) -> bool {
        let (Some(last), Some(first)) = (self.last_change, self.first_unsaved) else {
            return false;
        };
        let fire =
            now.duration_since(last) >= self.quiet || now.duration_since(first) >= self.max_delay;
        if fire {
            self.last_change = None;
            self.first_unsaved = None;
        }
        fire
    }

    /// 종료 등 — 조건 무시 강제 수거(S-1). 참 = 저장할 것이 있다.
    pub fn flush_now(&mut self) -> bool {
        let dirty = self.last_change.is_some();
        self.last_change = None;
        self.first_unsaved = None;
        dirty
    }
}

// ------------------------------------------------------------------- 스토어

/// 파일 1개 = 스토어 1개 — 경로·미지 키·직전 저장분(S-3 중복 쓰기 방지)·스케줄러.
///
/// 값은 앱이 소유한다(Entry 레지스트리). 스토어는 앱이 `known` 쌍을 넘겨줄 때만
/// 디스크를 만진다 — 두 경로(주기·종료)가 **같은 직렬화**를 쓴다(S-2).
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    unknown: Vec<(String, String)>,
    last_saved: Option<String>,
    /// 저장 스케줄러 — 호스트가 mark/tick/flush_now를 부른다.
    pub sched: SaveScheduler,
}

impl Store {
    /// 파일을 읽고(없으면 빈 문서) 파싱된 쌍을 돌려준다. 호출자는 아는 키를 적용하고
    /// **모르는 키는 [`Store::keep_unknown`]으로 되돌려준다**(F-1 보존 계약).
    #[must_use]
    pub fn open(path: PathBuf, quiet_ms: u64, max_delay_ms: u64) -> (Self, Doc) {
        let doc = parse(&fs::read_to_string(&path).unwrap_or_default());
        (
            Self {
                path,
                unknown: Vec::new(),
                last_saved: None,
                sched: SaveScheduler::new(quiet_ms, max_delay_ms),
            },
            doc,
        )
    }

    /// 모르는 키 보존 등록(직렬화 시 그대로 재방출).
    pub fn keep_unknown(&mut self, key: String, value: String) {
        self.unknown.push((key, value));
    }

    /// 저장 경로.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 스냅샷 저장 — 직전 저장분과 같으면 **쓰지 않는다**(S-3 · Ok(false)).
    pub fn save(&mut self, known: &[(&str, &str)]) -> io::Result<bool> {
        let text = serialize(known, &self.unknown);
        if self.last_saved.as_deref() == Some(text.as_str()) {
            return Ok(false);
        }
        write_atomic(&self.path, &text)?;
        self.last_saved = Some(text);
        Ok(true)
    }
}

// -------------------------------------------------------------- 경로 (FR-P-3)

/// 디렉터리 쓰기 가능 검사 — PID 접미 프로브 파일 생성→삭제(포터블 판정).
/// 디렉터리가 없으면 만들어 본다(만들 수 없으면 쓰기 불가).
#[must_use]
pub fn dir_writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".probe.{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 사용자 설정 폴더(폴백) — 앱 이름은 **인자로** 받는다(경로 비소유 · 싱글톤 금지).
/// Windows `%APPDATA%\{app}` · macOS `~/Library/Application Support/{app}` ·
/// 그 외 `$XDG_CONFIG_HOME/{app}` 또는 `~/.config/{app}`.
#[must_use]
pub fn user_config_dir(app: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join(app))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support")
                .join(app)
        })
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join(app))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nexa-conf-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    /// T-1: 변경 N회 → 발화 1회(가짜 시계).
    #[test]
    fn n_changes_one_fire() {
        let mut s = SaveScheduler::new(1000, 10_000);
        let t0 = Instant::now();
        for i in 0..5 {
            s.mark(at(t0, i * 100));
            assert!(!s.tick(at(t0, i * 100 + 50)), "조용 전 발화 금지");
        }
        assert!(s.tick(at(t0, 1400 + 1000)), "조용해지면 발화");
        assert!(!s.tick(at(t0, 60_000)), "dirty 소비 후 재발화 없음");
    }

    /// T-2: quiet보다 빠른 연속 변경에도 max_delay 초과 시 발화(starvation 없음).
    #[test]
    fn max_delay_beats_starvation() {
        let mut s = SaveScheduler::new(1000, 10_000);
        let t0 = Instant::now();
        let mut fired = 0;
        for i in 0..25 {
            s.mark(at(t0, i * 500)); // 0.5s 간격 = quiet 1s를 영원히 못 채움
            if s.tick(at(t0, i * 500 + 400)) {
                fired += 1;
            }
        }
        assert_eq!(fired, 1, "12.5초 동안 정확히 1회(10s 상한)");
    }

    /// T-3·T-9: flush_now가 마지막 변경을 수거하고, 수거는 소비다.
    #[test]
    fn flush_now_consumes() {
        let mut s = SaveScheduler::new(1000, 10_000);
        assert!(!s.flush_now(), "변경 없음 = 저장할 것 없음");
        s.mark(Instant::now());
        assert!(s.dirty());
        assert!(s.flush_now());
        assert!(!s.dirty() && !s.flush_now(), "이중 소비 불가");
    }

    /// T-4: 값이 같으면 쓰지 않는다(S-3).
    #[test]
    fn identical_snapshot_skips_write() {
        let d = tmpdir("t4");
        let (mut st, _) = Store::open(d.join("s.cfg"), 1000, 10_000);
        let known = [("a", "1"), ("b", "x")];
        assert!(st.save(&known).unwrap(), "첫 저장은 쓴다");
        assert!(!st.save(&known).unwrap(), "동일 스냅샷은 건너뛴다");
        assert!(st.save(&[("a", "2"), ("b", "x")]).unwrap());
        let _ = fs::remove_dir_all(&d);
    }

    /// T-5: 미지 키가 왕복 후에도 남아 있다(F-1).
    #[test]
    fn unknown_keys_survive_roundtrip() {
        let d = tmpdir("t5");
        let p = d.join("s.cfg");
        fs::write(&p, "_schema=1\nknown=old\nfuture.key=keepme\n").unwrap();
        let (mut st, doc) = Store::open(p.clone(), 1000, 10_000);
        assert_eq!(doc.schema, 1);
        for (k, v) in doc.pairs {
            if k != "known" {
                st.keep_unknown(k, v);
            }
        }
        st.save(&[("known", "new")]).unwrap();
        let re = parse(&fs::read_to_string(&p).unwrap());
        assert!(re.pairs.contains(&("future.key".into(), "keepme".into())));
        assert!(re.pairs.contains(&("known".into(), "new".into())));
        let _ = fs::remove_dir_all(&d);
    }

    /// T-6: 쓰레기 파일에도 파싱이 실패하지 않는다.
    #[test]
    fn garbage_parses_leniently() {
        let doc = parse("# 주석\n\n===\n키만\nk=v=w\n  \n_schema=쓰레기\nx = y\r\n");
        assert_eq!(doc.schema, 0, "손상 스키마 = 0");
        assert_eq!(doc.pairs.len(), 2);
        assert_eq!(doc.pairs[0], ("k".to_string(), "v=w".to_string()));
        assert_eq!(doc.pairs[1], ("x".to_string(), " y".to_string()));
    }

    /// T-8: 저장 실패(rename 불가) 시 기존 파일이 온전하고 temp가 남지 않는다.
    #[test]
    fn failed_write_leaves_original_intact() {
        let d = tmpdir("t8");
        let orig = d.join("s.cfg");
        write_atomic(&orig, "_schema=1\na=1\n").unwrap();
        // 대상 경로를 디렉터리로 만들어 rename을 강제로 실패시킨다.
        let blocked = d.join("dir.cfg");
        fs::create_dir_all(&blocked).unwrap();
        assert!(write_atomic(&blocked, "x=1\n").is_err());
        assert_eq!(fs::read_to_string(&orig).unwrap(), "_schema=1\na=1\n");
        let tmps: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmps.is_empty(), "temp 잔여 없음: {tmps:?}");
        let _ = fs::remove_dir_all(&d);
    }

    /// 개행이 값에 들어와도 ① 한 줄 = 한 키 불변식이 깨지지 않고(직렬화 결과는
    /// `_schema` 줄 + 키 줄 = 2줄) ② **왕복 보존**된다(08-18 — U+2028/2029).
    #[test]
    fn newline_in_value_round_trips() {
        let text = serialize(&[("k", "a\nb\rc")], &[]);
        assert_eq!(
            text.lines().count(),
            2,
            "값 속 개행이 물리 줄을 늘리지 않는다"
        );
        let doc = parse(&text);
        assert_eq!(doc.pairs, vec![("k".to_string(), "a\nb\rc".to_string())]);
    }

    /// ★ Windows 경로(역슬래시)는 개행 복원과 충돌하지 않는다(`\n`→개행 이스케이프
    /// 를 안 쓰는 이유). `C:\new` 가 `C:`+개행+`ew` 로 깨지지 않는다.
    #[test]
    fn backslash_path_survives() {
        let p = r"C:\new\readme.txt";
        let text = serialize(&[("path", p)], &[]);
        let doc = parse(&text);
        assert_eq!(doc.pairs, vec![("path".to_string(), p.to_string())]);
    }

    #[test]
    fn dir_writable_probe() {
        let d = tmpdir("probe");
        assert!(dir_writable(&d));
        let _ = fs::remove_dir_all(&d);
    }
}
