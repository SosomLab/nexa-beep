//! **상태바 로깅**(M3-22 · 사용자 요청 08-16 · `log.enabled` 기본 off 확정 08-18) —
//! 상태바에 스쳐 지나가는 상태값을 파일로 남긴다(유일한 진단 창구가 휘발되던 것).
//!
//! 1급 요구 = **본 기능을 절대 블록하지 않는다**:
//! - **bounded 채널 + 가득 차면 드롭**(백프레셔 금지 — 드롭은 카운트만 남겨 다음
//!   기록 때 `dropped N` 한 줄로 밝힌다 · fail-soft).
//! - 소비는 **전용 스레드**(BufWriter + 2초 주기 flush + 종료 flush). 스레드가
//!   죽어도 앱 무해(상태바는 그대로 — 로그만 멎는다).
//! - 회전·정리도 로거 스레드 몫(메인 무영향 불변식).
//!
//! 파일 = `data/logs/beep-YYYYMMDD.log`(**신원 폴더 소속** — 격리함 교훈 08-16) ·
//! 날짜가 바뀌면 새 파일 · 시작 시 보존 일수(`log.retain_days`)·총량 상한
//! (`log.max_total_mb`, 오래된 것부터) 정리. **봉투 원리**: 로그는 상태바 문구만
//! (이미 사용자 노출 텍스트 — 키·전문이 새로 들어오지 않게 `set_status`가 관문).

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};

/// 큐 상한 — 초당 수십 건 수준의 상태 변화에 넉넉하고, 폭주 시엔 드롭이 정답.
const QUEUE_CAP: usize = 256;
/// flush 주기(ms) — 크래시 직전 로그가 목적이라 너무 길면 의미가 준다.
const FLUSH_MS: u64 = 2_000;

/// 종료 신호 겸 메시지.
enum Cmd {
    Line(String),
    Stop,
}

/// 로거 핸들 — 드롭/`stop` 시 flush 후 정지(설정 off = hot-swap).
pub(crate) struct StatusLog {
    tx: SyncSender<Cmd>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl StatusLog {
    /// 기동 — 디렉터리 준비 실패 시 None(fail-soft · 상태바로 알림은 호출자 몫).
    /// `prefix` = 파일명 머리(`beep`·`netmon` — 같은 폴더를 나눠 쓰되 정리는
    /// 자기 프리픽스만 건드린다).
    pub(crate) fn start(
        dir: PathBuf,
        prefix: &'static str,
        retain_days: u64,
        max_total_mb: u64,
    ) -> Option<Self> {
        std::fs::create_dir_all(&dir).ok()?;
        let (tx, rx) = sync_channel::<Cmd>(QUEUE_CAP);
        let join = std::thread::Builder::new()
            .name("statuslog".into())
            .spawn(move || run(&dir, prefix, retain_days, max_total_mb, &rx))
            .ok()?;
        Some(Self {
            tx,
            join: Some(join),
        })
    }

    /// 한 줄 기록 — **논블로킹**: 큐가 가득이면 버린다(드롭 계상은 스레드가).
    pub(crate) fn log(&self, line: &str) {
        match self.tx.try_send(Cmd::Line(line.to_string())) {
            Ok(()) | Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {}
        }
    }

    /// 정지 — flush 후 스레드 합류(설정 off·종료 훅).
    pub(crate) fn stop(mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for StatusLog {
    fn drop(&mut self) {
        // stop을 못 부른 경로(패닉 되감기 등) — 최선의 flush 시도.
        let _ = self.tx.send(Cmd::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn today(unix: u64) -> (u32, u32, u32) {
    let t = nbeep_plat::clock::local_time(unix);
    (t.y, t.mo, t.d)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn file_for(dir: &std::path::Path, prefix: &str, ymd: (u32, u32, u32)) -> PathBuf {
    dir.join(format!("{prefix}-{:04}{:02}{:02}.log", ymd.0, ymd.1, ymd.2))
}

/// 로거 스레드 본체 — 열기·회전·flush·드롭 계상 전부 여기(메인 무영향).
fn run(
    dir: &std::path::Path,
    prefix: &str,
    retain_days: u64,
    max_total_mb: u64,
    rx: &Receiver<Cmd>,
) {
    sweep(dir, prefix, retain_days, max_total_mb);
    let mut day = today(unix_now());
    let open = |ymd: (u32, u32, u32)| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_for(dir, prefix, ymd))
            .ok()
            .map(std::io::BufWriter::new)
    };
    let mut out = open(day);
    let mut last_flush = std::time::Instant::now();
    let mut dropped_note = 0u64; // 큐 포화로 잃은 건수(다음 줄에 밝힌다)
    loop {
        // flush 주기를 지키기 위한 타임아웃 수신.
        let cmd = rx.recv_timeout(std::time::Duration::from_millis(FLUSH_MS));
        let now = unix_now();
        let ymd = today(now);
        if ymd != day {
            // 날짜 전환 = 새 파일 + 그때마다 정리(자정 넘긴 상주).
            if let Some(o) = &mut out {
                let _ = o.flush();
            }
            sweep(dir, prefix, retain_days, max_total_mb);
            day = ymd;
            out = open(day);
        }
        match cmd {
            Ok(Cmd::Line(line)) => {
                if let Some(o) = &mut out {
                    if dropped_note > 0 {
                        let _ = writeln!(o, "[--:--:--] (dropped {dropped_note})");
                        dropped_note = 0;
                    }
                    let hms = nbeep_plat::clock::local_hms(now).hms();
                    if writeln!(o, "[{hms}] {line}").is_err() {
                        out = None; // 디스크 실패 = 조용히 멎는다(fail-soft)
                    }
                } else {
                    dropped_note += 1;
                }
            }
            Ok(Cmd::Stop) => {
                if let Some(o) = &mut out {
                    let _ = o.flush();
                }
                return;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(o) = &mut out {
                    let _ = o.flush();
                }
                return;
            }
        }
        if last_flush.elapsed().as_millis() as u64 >= FLUSH_MS {
            if let Some(o) = &mut out {
                let _ = o.flush();
            }
            last_flush = std::time::Instant::now();
        }
    }
}

/// 보존 정리 — `retain_days` 지난 파일 삭제 + 총량 `max_total_mb` 초과 시
/// 오래된 것부터(part.rs sweep과 같은 문법). **자기 프리픽스만** — 같은 폴더의
/// 다른 로그(beep/netmon)를 서로 지우지 않는다.
fn sweep(dir: &std::path::Path, prefix: &str, retain_days: u64, max_total_mb: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let head = format!("{prefix}-");
    for e in rd.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "log") {
            continue;
        }
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(&head))
        {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let mtime = md.modified().unwrap_or(now);
        let age = now.duration_since(mtime).unwrap_or_default();
        if age.as_secs() > retain_days.saturating_mul(24 * 3600) {
            let _ = std::fs::remove_file(&path);
        } else {
            kept.push((mtime, md.len(), path));
        }
    }
    let cap = max_total_mb.saturating_mul(1024 * 1024);
    let total: u64 = kept.iter().map(|(_, s, _)| *s).sum();
    if total <= cap {
        return;
    }
    kept.sort_by_key(|(t, _, _)| *t);
    let mut over = total - cap;
    for (_, s, path) in kept {
        if std::fs::remove_file(&path).is_ok() {
            over = over.saturating_sub(s);
            if over == 0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 기동 → 기록 → 정지 = 오늘 파일에 줄이 남는다(시각 스탬프 동반).
    #[test]
    fn writes_lines_to_dated_file() {
        let dir = std::env::temp_dir().join(format!("nb-slog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = StatusLog::start(dir.clone(), "beep", 7, 20).expect("기동");
        log.log("hello 상태");
        log.log("second");
        log.stop(); // flush 보장
        let f = file_for(&dir, "beep", today(unix_now()));
        let text = std::fs::read_to_string(&f).expect("파일");
        assert!(text.contains("hello 상태") && text.contains("second"));
        assert!(text.starts_with('['), "시각 스탬프");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 큐 포화에도 log()는 블록하지 않는다(백프레셔 금지 — 1급 요구).
    #[test]
    fn log_never_blocks_when_queue_full() {
        let dir = std::env::temp_dir().join(format!("nb-slog2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = StatusLog::start(dir.clone(), "beep", 7, 20).expect("기동");
        let t0 = std::time::Instant::now();
        for i in 0..10_000 {
            log.log(&format!("burst {i}"));
        }
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(2),
            "1만 건 발화가 즉시 끝나야 한다(드롭 허용)"
        );
        log.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
