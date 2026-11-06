//! 빌드 스크립트 — **Windows exe 리소스 아이콘**(M3-22 · 11-06). 종전엔 아이콘이 NSIS 설치기·바로가기에만 있어 포터블 zip의
//! exe·탐색기·실행 중 작업표시줄(창 아이콘이 덮기 전)에 기본 아이콘이 보였다. 호스트가 Windows일 때만(cross check는 건너뜀 ·
//! 빌드 의존 `winresource`는 `[target.'cfg(windows)'.build-dependencies]` — 런타임 의존 0 유지).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../packaging/branding/nexa-beep.ico");
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("../../packaging/branding/nexa-beep.ico");
            // rc.exe/windres 부재 등은 경고만(fail-soft — 아이콘 없는 exe가 빌드 실패보다 낫다 · CI windows-latest엔 있다).
            if let Err(e) = res.compile() {
                println!("cargo:warning=exe icon resource skipped: {e}");
            }
        }
    }
}
