//! `nexa-beepd` 진입점 — 릴레이 서버(별도 실행 파일 · 별도 배포 — W-4).
//!
//! 인자 규약은 클라이언트와 동일한 정신: 미지 인자 = 안내 후 종료(조용한 오동작 금지).

use std::net::IpAddr;
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn usage() {
    println!(
        "nexa-beepd v{VERSION} — Nexa Beep 릴레이 서버(MVP)

사용법: nexa-beepd [옵션]

옵션:
  --port <N>     TCP 제어/중계 + UDP 관측 포트 (기본 {port})
  --bind <IP>    바인드 주소 (기본 0.0.0.0)
  --key <경로>   서버 신원 키 파일 (기본 beepd.key — 없으면 생성)
  --rate <값>    연결당 릴레이 예산: auto|100k|1m|10m|100m|1g|바이트수 (기본 1m · 0=무제한)
  --verbose      상세 로그(봉투 수준 — RID 축약·건수·바이트만)
  --version      버전 출력
  --help         이 안내

서버는 내용을 볼 수 없다(모드 ① — 봉투만): 회전 RID·채널·바이트 수·시각이 전부이며
아무것도 저장하지 않는다. 클라이언트는 첫 접속에서 아래 '서버 신원'을 핀(TOFU)한다.",
        port = nbeep_relay::DEFAULT_RELAY_PORT
    );
}

fn main() {
    let mut cfg = nexa_beepd::Config::default();
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
                Some(v) => {
                    // 클라이언트 설정과 같은 코드 체계(auto = 기본 예산).
                    cfg.rate_bps = match nbeep_core::rate::RateLimit::from_code(&v) {
                        nbeep_core::rate::RateLimit::PerSec(n) => n,
                        nbeep_core::rate::RateLimit::Auto => nexa_beepd::DEFAULT_RATE_BPS,
                    };
                }
                None => return bad_arg("--rate <값>"),
            },
            other => {
                eprintln!("미지 인자: {other}\n");
                usage();
                std::process::exit(2);
            }
        }
    }

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
        "nexa-beepd v{VERSION} 가동\n  TCP 제어/중계 : {tcp}\n  UDP 관측      : {udp}\n  서버 신원(핀) : {key_hex}\n  릴레이 예산   : {rate} B/s/연결{unlimited}",
        tcp = handle.tcp_addr,
        udp = handle.udp_port,
        rate = cfg.rate_bps,
        unlimited = if cfg.rate_bps == 0 { " (무제한)" } else { "" },
    );
    // 상주 — 종료는 시그널(Ctrl+C·서비스 매니저)로. 우아한 정리는 커널 소켓 정리로 충분
    // (서버는 아무것도 저장하지 않는다 — 모드 ①은 파이프다).
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

fn bad_arg(expect: &str) {
    eprintln!("인자 형식 오류 — {expect}");
    std::process::exit(2);
}
