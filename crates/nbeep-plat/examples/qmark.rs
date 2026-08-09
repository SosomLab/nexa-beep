//! 수동 실측용 — 파일에 격리 표식을 붙인다: `cargo run -p nbeep-plat --example qmark <path>`.
#![allow(clippy::unwrap_used)] // 실측 도구 — 실패 = 즉시 관측이 목적
fn main() {
    let p = std::env::args().nth(1).expect("경로 인자");
    let ok = nbeep_plat::quarantine::apply_quarantine_mark(std::path::Path::new(&p)).unwrap();
    println!("applied={ok}");
}
