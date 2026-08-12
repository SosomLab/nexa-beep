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
//! 기본 실행은 스캐폴드 출력(헤드리스 CI 안전).

// 조립 지점 바이너리 — 창 초기화 경로의 unwrap 허용(docs/13 §9 — 복구 불가 구성 오류).
#![allow(clippy::unwrap_used)]

mod app;
mod cli;
mod gate;
mod imgdec;

use cli::chat::{chat_interactive, chat_live, ChatRole};
use cli::manual::{connect_manual, serve_manual};
use cli::probe::{discover_probe, live_echo};
use cli::quarantine::quarantine_demo;

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
        let Some(addr) = args.get(pos + 1).cloned() else {
            eprintln!("--connect <host:port> 필요");
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
        let Some(addr) = args.get(pos + 1).cloned() else {
            eprintln!("--chat-connect <host:port> 필요");
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
    // ★ GUI 셸에서 **인자 없이** 열면(macOS `.app` 더블클릭 · Windows 탐색기 더블클릭·
    //   시작 메뉴 바로가기) 그대로 두면 스캐폴드만 찍고 끝나 "눌러도 아무 일이 없다"가 된다
    //   (08-11 실기: mac brew·Windows 배포 v0.1.2 실측 = M5-4d). 그 경우 **창 모드가 의도**다.
    //   - macOS: 실행 경로가 `.app/Contents/MacOS/`인가([`launched_from_app_bundle`]).
    //   - Windows: 탐색기가 **새 콘솔을 할당**했는가([`nbeep_plat::launch::from_gui_shell`]).
    //   터미널·brew shim에서 부르면 둘 다 false라 CLI 동작이 그대로 남는다.
    let no_real_args = !open_window && !separate && !live;
    let gui_launch =
        launched_from_app_bundle() || (no_real_args && nbeep_plat::launch::from_gui_shell());
    if open_window || separate || gui_launch {
        // 탐색기가 새로 띄운 콘솔은 GUI 창 옆에 남으므로 떼어낸다(Windows · M5-4d).
        // 인자로 부른 `--window`는 기존 터미널을 쓰던 것일 수 있어 건드리지 않는다.
        if gui_launch && no_real_args {
            nbeep_plat::launch::hide_gui_console();
        }
        let mode = if separate {
            app::WindowMode::Separate
        } else {
            app::WindowMode::Single
        };
        // GUI 실행은 실물 발견이 기본이다 — 데모(에코 봇)를 보여 줄 자리가 아니다.
        app::run(
            mode,
            live || (gui_launch && !open_window && !separate),
            port,
        );
    } else {
        println!(
            "nexa-beep {} — scaffold (창 `--window [--live] [--port <N>]` · 발견 `--discover-probe [초]` · 수동 `--serve`/`--connect` · 인터랙티브 `--chat-serve [port]`/`--chat-connect <host:port>`/`--chat-live [이름] [--port <N>]`(GUI 목록에 뜸) · 무해화 실측 `--quarantine-demo <파일>` · 파일전송 `--xfer-limit-mib <N>`·`--xfer-rate-kb <N>`(chat 모드 · 대화 중 `/send`·`/accept`·`/reject`))",
            env!("CARGO_PKG_VERSION")
        );
    }
}

/// macOS 앱 번들 안에서 **인자 없이** 실행됐는가(= Finder/Dock에서 열었다).
///
/// 실행 파일 경로가 `*.app/Contents/MacOS/*` 인지로 판정한다 — 번들 밖(터미널·brew shim)
/// 에서 부른 경우는 해당하지 않아 CLI 동작이 그대로 남는다.
/// macOS는 Finder 실행 시 `-psn_...` 인자를 붙이기도 해서, 그것도 인자 없음으로 본다.
fn launched_from_app_bundle() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let has_real_args = std::env::args().skip(1).any(|a| !a.starts_with("-psn_"));
    if has_real_args {
        return false;
    }
    std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}
