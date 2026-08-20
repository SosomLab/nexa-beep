//! Nexa Beep 본체 — 진입·조립·생명주기.
//!
//! `--window` = InMemory 데모(에코 봇). **`--window --live` = 실물 발견(LocalDirect)** —
//! 같은 LAN·컨테이너의 실제 상대가 목록에 뜨고 진짜 Noise 세션으로 대화한다.
//! (구 문구) InMemory 종단 데모: 실물 발견(에코 봇) → 목록 → Enter →
//! Noise 핸드셰이크 → TOFU 핀 → 다중화 세션 위 대화 왕복. 전 계층이 실물이다.
//!
//! **창 모드(DR-26 · FR-U-18)**: 기본 = 단일 창(목록↔대화 전환). `--separate-windows` =
//! **상대별 별도 OS 창**(동시 대화). 대화 상태(`Conversation`)는 어느 모드든 뷰와 분리되어
//! 유지된다. ⚠️ 모드 선택의 설정 화면 연동(`chat.window_mode`)은 M3-11 — 그 전까지 실행 인자.
//!
//! 실물 네트워크 배선은 M1-4, 창 코드의 `nbeep-plat` 이관은 M3-2.
//! **기본 실행(무인자) = 창 모드 + 실물 발견**(사용자 확정 08-13 — DR-1 "실행 = 참여").
//! 사용법 안내는 `--help`(무인자 스캐폴드 출력은 08-13 폐지 — GUI가 기본이 됐다).
//! ⚠️ 헤드리스(컨테이너·CI)에서 무인자로 부르면 창을 시도한다 — 검증 도구는 항상 명시 인자.

// 조립 지점 바이너리 — 창 초기화 경로의 unwrap 허용(docs/13 §9 — 복구 불가 구성 오류).
#![allow(clippy::unwrap_used)]
// ★ Windows = **windows 서브시스템**(08-20 사용자 실기 — 실행할 때마다 터미널 창이 뜬다):
//   콘솔 서브시스템 + FreeConsole 휴리스틱(M5-4d)은 Windows 11 기본 터미널(Windows
//   Terminal)에서 무력하다 — ConPTY 세션 창이 detach가 아니라 **루트 프로세스 종료**에
//   묶여, 빈 터미널 창이 앱 옆에 상주했다. 서브시스템 전환으로 콘솔 창은 아예 안 생기고,
//   터미널 실행의 CLI 출력은 main 첫 줄 `attach_parent_console()`이 보전한다.
//   ⚠ 터미널 CLI의 셸 대기 관례가 바뀐다: cmd·pwsh는 GUI 앱을 기다리지 않아 셸과
//   입력 경합이 생기므로, 대화형 도구(--chat-live 등)는 그런 셸에서 **자기 콘솔 창을
//   새로 열어** 계속한다(launch::own_console_for_interactive — 08-20 실기 후속).
//   Git Bash(대기함)·파이프/리다이렉트(캡처·스크립트)는 그 자리 그대로 동작.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod cli;
mod gate;
mod ime_gate;
mod imgdec;
mod part;
mod statuslog;

use cli::chat::{chat_interactive, chat_live, ChatRole};
use cli::manual::{connect_manual, serve_manual};
use cli::probe::{discover_probe, live_echo};
use cli::quarantine::quarantine_demo;

fn main() {
    // 어떤 출력보다 먼저 — 터미널에서 불렸으면 부모 콘솔에 붙어 CLI 출력을 보전한다
    // (windows 서브시스템 전환의 짝 · 리다이렉트 핸들은 함수가 되살린다). GUI 실행
    // (더블클릭·자동 실행 M3-25)은 부모 콘솔이 없어 no-op = 콘솔 창 0.
    nbeep_plat::launch::attach_parent_console();
    let args: Vec<String> = std::env::args().collect();
    // 도움말·버전 — 어느 모드보다 먼저(배포 검증 절차 26 §7-2가 `--version`을 쓴다.
    // 그전엔 무인자 스캐폴드가 버전을 겸했는데, 무인자 = 창이 되면서 명시 처리로 분리).
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("nexa-beep {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--whoami") {
        app::print_whoami();
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--quarantine-demo") {
        let Some(path) = args.get(pos + 1) else {
            eprintln!("--quarantine-demo <파일> 필요");
            return;
        };
        quarantine_demo(std::path::Path::new(path));
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--discover-probe") {
        let secs = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(8);
        discover_probe(secs);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--live-echo") {
        let secs = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(15);
        live_echo(secs);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--serve") {
        let port = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(47_200);
        serve_manual(port);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--connect") {
        // 정규화 = GUI 모달과 같은 규칙(M1-14) — 포트 생략은 기본 포트, 옥텟 오타는
        // DNS로 흘리지 않고 즉시 거절(오해를 주는 Unreachable 방지).
        let Some(raw) = args.get(pos + 1) else {
            eprintln!("--connect <host[:port]> 필요 (예: 10.0.0.5 · 10.0.0.5:47200)");
            return;
        };
        let Some(addr) =
            nbeep_core::endpoint::normalize_endpoint(raw, nbeep_net::DEFAULT_SESSION_PORT)
        else {
            eprintln!("--connect 주소 형식 오류: {raw} (예: 10.0.0.5 · [fe80::1]:47200)");
            return;
        };
        connect_manual(&addr);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--chat-serve") {
        let port = args
            .get(pos + 1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(47_200);
        chat_interactive(ChatRole::Serve(port));
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--chat-connect") {
        let Some(raw) = args.get(pos + 1) else {
            eprintln!("--chat-connect <host[:port]> 필요 (예: 10.0.0.5 · 10.0.0.5:47200)");
            return;
        };
        // GUI 모달과 같은 정규화(M1-14) — `10.0.0.5`만 넣어도 붙는다.
        let Some(addr) =
            nbeep_core::endpoint::normalize_endpoint(raw, nbeep_net::DEFAULT_SESSION_PORT)
        else {
            eprintln!("--chat-connect 주소 형식 오류: {raw} (예: 10.0.0.5 · [fe80::1]:47200)");
            return;
        };
        chat_interactive(ChatRole::Connect(addr));
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--chat-live") {
        // 포트 지정(08-13) — 창 모드 `--port`와 같은 규약. 생략·0이면 임의 포트.
        let port = args
            .iter()
            .position(|a| a == "--port")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // 이름은 위치 인자다. ⚠️ `--chat-live --port 43211 테스트단말`처럼 **옵션이 먼저 와도**
        // 이름을 찾아야 한다(08-13 사용자 지적) — 옵션과 그 값을 건너뛰고 첫 일반 토큰을 쓴다.
        let name = {
            let mut it = args[pos + 1..].iter();
            let mut found = None;
            while let Some(a) = it.next() {
                if a.starts_with("--") {
                    // 값을 받는 옵션이면 그 값까지 건너뛴다.
                    if a == "--port" {
                        it.next();
                    }
                    continue;
                }
                found = Some(a.clone());
                break;
            }
            found.unwrap_or_else(|| "터미널".into())
        };
        chat_live(&name, port);
        return;
    }
    let open_window = args.iter().any(|a| a == "--window");
    let separate = args.iter().any(|a| a == "--separate-windows");
    let live = args.iter().any(|a| a == "--live");
    // 수신 포트 지정(⑥ 사용자 요청 08-13) — **이 세션만 이긴다**(--separate-windows와
    // 같은 규칙: 저장된 설정 `net.session_port`는 건드리지 않는다). 0 = 임의 포트.
    let port: Option<u16> = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok());
    // 미지 인자 = 오타 가능성 — **조용히 창을 띄우지 않는다**(안내 후 종료). 무인자가
    // 창이 된 순간, 잘못 친 인자를 무시하고 GUI가 뜨면 오타가 영영 발견되지 않는다.
    // macOS Finder의 레거시 `-psn_...`만 무시(창 실행으로 취급).
    {
        let mut it = args.iter().skip(1);
        while let Some(a) = it.next() {
            match a.as_str() {
                "--window" | "--separate-windows" | "--live" => {}
                "--port" => {
                    it.next(); // 값
                }
                _ if a.starts_with("-psn_") => {}
                _ => {
                    eprintln!("알 수 없는 인자: {a} — `nexa-beep --help` 참조");
                    std::process::exit(2);
                }
            }
        }
    }
    // ★ **무인자 기본 = 창 모드 + 실물 발견**(사용자 확정 08-13 — DR-1 "실행 = 참여").
    //   그전엔 무인자 = 스캐폴드 출력이라 GUI 셸 실행(더블클릭·바로가기)만 휴리스틱으로
    //   창을 띄웠는데(M5-4d), 이제 어디서 부르든 무인자면 창이다. 콘솔 분리 휴리스틱
    //   (from_gui_shell/FreeConsole)은 08-20 windows 서브시스템 전환으로 폐기 —
    //   콘솔은 애초에 안 생기고, 터미널 실행은 attach_parent_console이 잇는다.
    // ★ 창을 열 수 있는 자리인가 — **열기 전에 묻는다**(사용자 요청 08-13).
    //   못 여는 자리에서 열면 프로세스가 그냥 죽는다(컨테이너 실기: `Exited (134)` +
    //   "시스템 UI 폰트 없음"). SSH·원격 PowerShell·서비스 세션도 같은 자리다.
    //   무인자가 창이 된 뒤로는 **원격 셸에서 무심코 친 `nexa-beep`이 이 경로**라 더 중요하다.
    if let Err(why) = nbeep_plat::gui::probe() {
        eprintln!("{}", why.message());
        std::process::exit(3);
    }
    let mode = if separate {
        app::WindowMode::Separate
    } else {
        app::WindowMode::Single
    };
    // 데모(에코 봇)는 `--window`/`--separate-windows`를 **명시**한 경우만(개발 도구) —
    // 그 외 창 진입(무인자·--port·--live)은 전부 실물 발견이다.
    app::run(mode, live || (!open_window && !separate), port);
}

/// `--help`/`-h` — 모드·옵션 안내(사용자 요청 08-13).
fn print_help() {
    println!(
        "\
Nexa Beep {v} — 제로 컨피그 로컬 네트워크 메신저 (\"실행 = 참여\")

사용법:
  nexa-beep                        창 모드 + 실물 발견(기본) — 같은 LAN의 상대가
                                   목록에 뜨고, 고르면 즉시 암호화 대화
  nexa-beep [창 옵션]              창 모드 세부 지정
  nexa-beep <검증 도구> [옵션]     터미널 검증 도구(개발·실측용 · 창 없음)

창 옵션:
  --port <N>            세션 수신 포트(이 실행만 · 기본 47200, 점유 시 임의 폴백 ·
                        저장 설정은 net.session_port)
  --separate-windows    상대별 별도 창으로 시작(기본 = 단일 창 · 설정 chat.window_mode)
  --window              InMemory 데모(에코 봇 · 네트워크 없음 — 개발용).
                        --live를 붙이면 무인자와 같은 실물 발견

터미널 검증 도구(인터랙티브):
  --chat-live [이름] [--port <N>]   CLI 단말 — 발견 광고 + 조회·능동 연결(08-20 일습).
                                    수신은 전 채널(인바운드 = 수신 전용 표시) ·
                                    상호 채팅은 /connect 상대 1명(1:1)
      대기 중 명령: /peers = 발견 번호 목록 · /connect <번호|host[:port]> = 대화 시작 · /quit
  --chat-serve [port]               고정 포트로 기다리는 대화 단말(기본 47200)
  --chat-connect <host[:port]>      주소로 직접 붙는 대화 단말(발견 없이 · 포트 생략 = 47200)
      대화 중 명령: 한 줄 = 전송 · /send <파일…>(공백 구분 다중 · 요청당 최대 5) ·
                    /accept·/reject = 요청 전체 승인/거절 · /help · /quit = 목록 복귀(Ctrl+D)
      수신/읽음 확인: 수신 즉시 전달·출력 즉시 읽음 되쏨 · 내 발신분은 [확인]으로 표시
      파일 수신 옵션: --xfer-limit-mib <N>(수신 상한 · 기본 256) · --xfer-rate-kb <N>(속도 상한)

터미널 검증 도구(비대화):
  --serve [port] · --connect <host[:port]>   수동 엔드포인트 왕복 실증(DR-19 · 포트 생략 = 47200)
  --discover-probe [초]                    발견 브로드캐스트 관찰(기본 8초)
  --live-echo [초]                         발견 + 에코 응답 노드(기본 15초)
  --quarantine-demo <파일>                 수신 무해화 게이트 종단 실측(.beepq)

기타:
  -h, --help            이 도움말
  -V, --version         버전 출력",
        v = env!("CARGO_PKG_VERSION")
    );
}
