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
        let name = args
            .get(pos + 1)
            .cloned()
            .unwrap_or_else(|| "터미널".into());
        chat_live(&name);
        return;
    }
    let open_window = args.iter().any(|a| a == "--window");
    let separate = args.iter().any(|a| a == "--separate-windows");
    let live = args.iter().any(|a| a == "--live");
    // ★ macOS `.app`을 더블클릭하면 **인자가 하나도 오지 않는다** — 그대로 두면
    //   스캐폴드 안내만 찍고 즉시 끝나, 사용자 눈에는 "눌러도 아무 일이 없다"가 된다
    //   (08-11 실기: brew로 설치한 앱을 Finder에서 열었을 때). 번들 안에서 인자 없이
    //   실행됐다면 **창 모드가 의도**다. 터미널에서 부른 경우는 경로가 다르니 영향 없다.
    let bundled = launched_from_app_bundle();
    if open_window || separate || bundled {
        let mode = if separate {
            app::WindowMode::Separate
        } else {
            app::WindowMode::Single
        };
        // 번들 실행은 실물 발견이 기본이다 — 데모(에코 봇)를 보여 줄 자리가 아니다.
        app::run(mode, live || (bundled && !open_window && !separate));
    } else {
        println!(
            "nexa-beep {} — scaffold (창 `--window [--live]` · 발견 `--discover-probe [초]` · 수동 `--serve`/`--connect` · 인터랙티브 `--chat-serve [port]`/`--chat-connect <host:port>`/`--chat-live [이름]`(GUI 목록에 뜸) · 무해화 실측 `--quarantine-demo <파일>` · 파일전송 `--xfer-limit-mib <N>`·`--xfer-rate-kb <N>`(chat 모드 · 대화 중 `/send`·`/accept`·`/reject`))",
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
