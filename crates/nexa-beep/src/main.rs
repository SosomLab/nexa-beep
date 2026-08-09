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
    if open_window || separate {
        let mode = if separate {
            app::WindowMode::Separate
        } else {
            app::WindowMode::Single
        };
        app::run(mode, live);
    } else {
        println!(
            "nexa-beep {} — scaffold (창 `--window [--live]` · 발견 `--discover-probe [초]` · 수동 `--serve`/`--connect` · 인터랙티브 `--chat-serve [port]`/`--chat-connect <host:port>`/`--chat-live [이름]`(GUI 목록에 뜸) · 무해화 실측 `--quarantine-demo <파일>` · 파일전송 수신상한 `--xfer-limit-mib <N>`(chat 모드 · 대화 중 `/send`·`/accept`·`/reject`))",
            env!("CARGO_PKG_VERSION")
        );
    }
}
