//! `nexa-beepd` 진입점 — 릴레이 서버(별도 실행 파일 · 별도 배포 — W-4).
//!
//! 설정 = **환경변수 `BEEPD_*` → CLI 인자**(뒤가 이긴다). 미지 인자 = 안내 후 종료(조용한 오동작 금지).
//! 종료 = SIGTERM/SIGINT(unix) · Ctrl+C/Close(Windows) → 연결마다 `Goaway{Shutdown, retry_after}` → 소켓 정리.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!(
        "nexa-beepd v{VERSION} — Nexa Beep 릴레이 서버(와이어 v2 · ADR-0016)

사용법: nexa-beepd [옵션]      (환경변수 BEEPD_PORT·BEEPD_BIND·BEEPD_KEY·BEEPD_RATE·BEEPD_VERBOSE가 기본값 · CLI가 이긴다)

옵션:
  --port <N>     TCP 제어/중계 + UDP 관측 포트 (기본 {port})
  --bind <IP>    바인드 주소 (기본 0.0.0.0)
  --key <경로>   서버 신원 키 파일 (기본 beepd.key — 없으면 생성)
  --rate <값>    연결당 릴레이 예산: auto|100k|1m|10m|100m|1g|바이트수 (기본 1m · 0=무제한)
  --verbose      상세 로그(봉투 수준 — RID 축약·건수·바이트만) + 60초마다 계측 한 줄
  --version      버전 출력
  --help         이 안내

서버는 내용을 볼 수 없다(모드 ① — 봉투만): kind·회전 RID·채널·바이트 수·시각이 전부이며
아무것도 저장하지 않는다. 클라이언트는 첫 접속에서 아래 '서버 신원'을 핀(TOFU)한다.
헬스 확인 = 같은 포트에 Hello{{app=\"probe\"}} → Welcome(서버 버전·능력·상한).",
        port = nexa_beepd::DEFAULT_PORT
    );
}

fn parse_rate(v: &str) -> u64 {
    // 클라이언트 설정과 같은 코드 체계(auto = 기본 예산).
    match nbeep_core::rate::RateLimit::from_code(v) {
        nbeep_core::rate::RateLimit::PerSec(n) => n,
        nbeep_core::rate::RateLimit::Auto => nexa_beepd::DEFAULT_RATE_BPS,
    }
}

fn env_defaults(cfg: &mut nexa_beepd::Config) {
    if let Some(p) = std::env::var("BEEPD_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
    {
        cfg.port = p;
    }
    if let Some(ip) = std::env::var("BEEPD_BIND")
        .ok()
        .and_then(|v| v.parse::<IpAddr>().ok())
    {
        cfg.bind_ip = ip;
    }
    if let Ok(k) = std::env::var("BEEPD_KEY") {
        if !k.is_empty() {
            cfg.key_path = PathBuf::from(k);
        }
    }
    if let Ok(r) = std::env::var("BEEPD_RATE") {
        if !r.is_empty() {
            cfg.rate_bps = parse_rate(&r);
        }
    }
    if let Ok(v) = std::env::var("BEEPD_VERBOSE") {
        cfg.verbose = matches!(v.as_str(), "1" | "true" | "yes" | "on");
    }
}

fn main() {
    let mut cfg = nexa_beepd::Config::default();
    env_defaults(&mut cfg);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--help" | "-h" => {
                usage();
                return;
            }
            "--version" | "-V" => {
                println!("nexa-beepd {VERSION}");
                return;
            }
            "--verbose" => cfg.verbose = true,
            "--port" => match args.next().and_then(|v| v.parse::<u16>().ok()) {
                Some(p) => cfg.port = p,
                None => return bad_arg("--port <숫자>"),
            },
            "--bind" => match args.next().and_then(|v| v.parse::<IpAddr>().ok()) {
                Some(ip) => cfg.bind_ip = ip,
                None => return bad_arg("--bind <IP>"),
            },
            "--key" => match args.next() {
                Some(p) => cfg.key_path = PathBuf::from(p),
                None => return bad_arg("--key <경로>"),
            },
            "--rate" => match args.next() {
                Some(v) => cfg.rate_bps = parse_rate(&v),
                None => return bad_arg("--rate <값>"),
            },
            other => {
                eprintln!("미지 인자: {other}\n");
                usage();
                std::process::exit(2);
            }
        }
    }

    signal::install();
    let handle = match nexa_beepd::spawn(&cfg) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[beepd] 시작 실패: {e}");
            std::process::exit(1);
        }
    };
    let key_hex: String = handle
        .server_peer
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!(
        "nexa-beepd v{VERSION} 가동(와이어 v{major}.{minor})\n  TCP 제어/중계 : {tcp}\n  UDP 관측      : {udp}\n  서버 신원(핀) : {key_hex}\n  릴레이 예산   : {rate} B/s/연결{unlimited}",
        major = nbeep_relay_proto::PROTO_MAJOR,
        minor = nbeep_relay_proto::PROTO_MINOR,
        tcp = handle.tcp_addr,
        udp = handle.udp_port,
        rate = cfg.rate_bps,
        unlimited = if cfg.rate_bps == 0 { " (무제한)" } else { "" },
    );
    // 상주 — 시그널이 오면 Goaway 방송 뒤 내려간다(서버는 아무것도 저장하지 않는다 — 모드 ①은 파이프다).
    let mut last_report = std::time::Instant::now();
    while !signal::stopped() {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if cfg.verbose && last_report.elapsed() >= std::time::Duration::from_secs(60) {
            last_report = std::time::Instant::now();
            eprintln!("[beepd] 계측 {}", handle.metrics());
        }
    }
    eprintln!("[beepd] 종료 신호 — Goaway 방송 뒤 내려간다");
    handle.shutdown();
}

fn bad_arg(expect: &str) {
    eprintln!("인자 형식 오류 — {expect}");
    std::process::exit(2);
}

/// 종료 시그널 — 외부 크레이트 없이(서버 전용 의존 0 · W-3). 핸들러는 플래그만 세운다.
mod signal {
    use super::{AtomicBool, Ordering};

    static STOP: AtomicBool = AtomicBool::new(false);

    pub(super) fn stopped() -> bool {
        STOP.load(Ordering::SeqCst)
    }

    #[cfg(unix)]
    pub(super) fn install() {
        extern "C" {
            fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
        }
        extern "C" fn on_signal(_sig: i32) {
            STOP.store(true, Ordering::SeqCst);
        }
        const SIGINT: i32 = 2;
        const SIGTERM: i32 = 15;
        // SAFETY: libc `signal`에 async-signal-safe 핸들러(원자 저장만)를 등록한다. 반환값(이전 핸들러)은 쓰지 않는다.
        unsafe {
            signal(SIGINT, on_signal);
            signal(SIGTERM, on_signal);
        }
    }

    #[cfg(windows)]
    pub(super) fn install() {
        type HandlerRoutine = unsafe extern "system" fn(u32) -> i32;
        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleCtrlHandler(handler: Option<HandlerRoutine>, add: i32) -> i32;
        }
        unsafe extern "system" fn on_ctrl(_kind: u32) -> i32 {
            STOP.store(true, Ordering::SeqCst);
            // 처리했다 — 기본 핸들러(즉시 종료)를 막고 main 루프가 Goaway 뒤 내려가게 한다.
            // Windows는 CTRL_CLOSE/SHUTDOWN 뒤 약 5초 여유를 준다 — Handle::shutdown의 500ms 대기 안이다.
            1
        }
        // SAFETY: 콘솔 제어 핸들러 등록(kernel32) — 핸들러는 원자 저장만 한다.
        unsafe {
            SetConsoleCtrlHandler(Some(on_ctrl), 1);
        }
    }

    #[cfg(not(any(unix, windows)))]
    pub(super) fn install() {}
}
