//! 파일 검사 어댑터(M4 §6 · FR-S-15) — [`nbeep_core::FileScanner`] 포트의 OS별 구현.
//!
//! - **Windows**: **AMSI 실물**(08-21 — `amsi.dll` 동적 로드 + `AmsiScanBuffer` 직접 ·
//!   의존 0). 시스템에 등록된 백신 제공자(기본 = Microsoft Defender)가 버퍼를
//!   판정한다. 로드·초기화·호출 실패는 전부 `Unavailable` 폴백(fail-soft — 검사
//!   불능이 실체화를 열지는 않는다: 상태기계가 마찰을 올린다 · [docs/11] §6).
//! - **macOS/Linux**: 시스템이 스크립트 가능한 온디맨드 검사 API를 주지 않는다
//!   (XProtect는 비공개 · clamav는 외부 의존 = DR-5 위반) — `Unavailable` 고정.
//!
//! `Unavailable`은 실패가 아니다 — 격리 상태기계가 마찰을 올린다(`friction_raised`).
//! 표기는 사실 3종뿐: 검사됨(탐지 없음) · 검사됨(탐지) · 검사 안 됨(NFR-S-5).

use nbeep_core::{FileScanner, NoScanner, ScanOutcome};

/// 이 OS의 기본 검사기 — Windows = AMSI(사용 불능이면 `NoScanner` 폴백) ·
/// mac/Linux = `NoScanner`. 호출부는 이 이음새만 안다(DR-21).
#[must_use]
pub fn platform_scanner() -> Box<dyn FileScanner> {
    #[cfg(windows)]
    if let Some(s) = amsi::scanner() {
        return s;
    }
    Box::new(NoScanner)
}

/// 편의 — 기본 검사기로 1회 검사.
#[must_use]
pub fn scan(name: &str, bytes: &[u8]) -> ScanOutcome {
    platform_scanner().scan(name, bytes)
}

/// Windows AMSI(Antimalware Scan Interface) 어댑터 — `amsi.dll`을 **동적 로드**한다.
/// 정적 링크(`#[link]`)를 쓰지 않는 이유: 미설치·비활성 환경에서 링크 실패가 아니라
/// **정직한 `Unavailable` 폴백**이어야 하고(문서 계약), 로드는 첫 사용 1회다.
#[cfg(windows)]
mod amsi {
    use core::ffi::c_void;
    use nbeep_core::{FileScanner, ScanOutcome};
    use std::sync::OnceLock;

    type FnAmsiInitialize = unsafe extern "system" fn(*const u16, *mut isize) -> i32;
    type FnAmsiOpenSession = unsafe extern "system" fn(isize, *mut isize) -> i32;
    type FnAmsiCloseSession = unsafe extern "system" fn(isize, isize);
    type FnAmsiScanBuffer =
        unsafe extern "system" fn(isize, *const c_void, u32, *const u16, isize, *mut i32) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> isize;
        fn GetProcAddress(module: isize, name: *const u8) -> *const c_void;
    }

    /// 초기화된 AMSI 문맥 + 함수 포인터(프로세스 수명 — Uninitialize 없음: 검사는
    /// 앱 종료까지 쓰이고, OS가 정리한다 · 폰트 mmap 누수와 같은 결).
    struct Amsi {
        ctx: isize,
        open: FnAmsiOpenSession,
        close: FnAmsiCloseSession,
        scan: FnAmsiScanBuffer,
    }
    // SAFETY: AMSI 문맥(HAMSICONTEXT)은 다중 스레드 동시 사용이 허용된다(MSDN —
    // 세션은 상관관계용 옵션). 함수 포인터는 불변.
    unsafe impl Send for Amsi {}
    unsafe impl Sync for Amsi {}

    static AMSI: OnceLock<Option<Amsi>> = OnceLock::new();

    fn wide(s: &str) -> Vec<u16> {
        s.chars()
            .filter(|c| *c != '\0')
            .collect::<String>()
            .encode_utf16()
            .chain([0])
            .collect()
    }

    fn init() -> Option<Amsi> {
        // SAFETY: 시스템 DLL 로드 + 심볼 조회 + 문서화된 서명으로의 호출.
        // 어느 단계든 실패 = None(Unavailable 폴백) — 부작용 없음.
        unsafe {
            let lib = LoadLibraryW(wide("amsi.dll").as_ptr());
            if lib == 0 {
                return None;
            }
            let sym = |n: &[u8]| GetProcAddress(lib, n.as_ptr());
            let p_init = sym(b"AmsiInitialize\0");
            let p_open = sym(b"AmsiOpenSession\0");
            let p_close = sym(b"AmsiCloseSession\0");
            let p_scan = sym(b"AmsiScanBuffer\0");
            if p_init.is_null() || p_open.is_null() || p_close.is_null() || p_scan.is_null() {
                return None;
            }
            let f_init: FnAmsiInitialize = core::mem::transmute(p_init);
            let mut ctx = 0isize;
            if f_init(wide("Nexa Beep").as_ptr(), &mut ctx) != 0 || ctx == 0 {
                return None;
            }
            Some(Amsi {
                ctx,
                open: core::mem::transmute::<*const c_void, FnAmsiOpenSession>(p_open),
                close: core::mem::transmute::<*const c_void, FnAmsiCloseSession>(p_close),
                scan: core::mem::transmute::<*const c_void, FnAmsiScanBuffer>(p_scan),
            })
        }
    }

    struct AmsiScanner(&'static Amsi);

    impl FileScanner for AmsiScanner {
        fn scan(&self, name: &str, bytes: &[u8]) -> ScanOutcome {
            if bytes.is_empty() {
                return ScanOutcome::Clean; // 빈 파일 — 검사할 내용이 없다
            }
            let Ok(len) = u32::try_from(bytes.len()) else {
                return ScanOutcome::Unavailable; // 4GiB 초과 — 절단 검사 금지(정직)
            };
            let a = self.0;
            // SAFETY: 유효한 문맥·버퍼·길이·널 종단 이름. 세션은 실패해도 0으로
            // 진행 가능(상관관계용 옵션 — MSDN).
            unsafe {
                let mut session = 0isize;
                let _ = (a.open)(a.ctx, &mut session);
                let mut result = 0i32;
                let hr = (a.scan)(
                    a.ctx,
                    bytes.as_ptr().cast(),
                    len,
                    wide(name).as_ptr(),
                    session,
                    &mut result,
                );
                if session != 0 {
                    (a.close)(a.ctx, session);
                }
                if hr != 0 {
                    return ScanOutcome::Unavailable;
                }
                // AMSI_RESULT: 0 = CLEAN · 1 = NOT_DETECTED ·
                // 0x4000~0x4FFF = 관리자 정책 차단(탐지로 취급) · ≥0x8000 = 탐지.
                if result >= 0x8000 || (0x4000..=0x4FFF).contains(&result) {
                    ScanOutcome::Detected
                } else {
                    ScanOutcome::Clean
                }
            }
        }
    }

    /// AMSI 검사기(사용 가능할 때) — 첫 호출이 로드·초기화(이후 재사용).
    pub(super) fn scanner() -> Option<Box<dyn FileScanner>> {
        AMSI.get_or_init(init)
            .as_ref()
            .map(|a| Box::new(AmsiScanner(a)) as Box<dyn FileScanner>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NoScanner` 폴백 계약 — 항상 `Unavailable`(게이트가 마찰을 올린다).
    /// Windows AMSI가 있어도 이 폴백 타입의 계약은 불변.
    #[test]
    fn no_scanner_is_unavailable() {
        assert_eq!(NoScanner.scan("a.exe", b"MZ"), ScanOutcome::Unavailable);
    }

    /// 비-Windows 기본 = `Unavailable` 고정(어댑터 없음 — 정직한 표기).
    #[cfg(not(windows))]
    #[test]
    fn default_scanner_is_unavailable() {
        assert_eq!(scan("a.exe", b"MZ"), ScanOutcome::Unavailable);
    }

    /// Windows — 무해 바이트는 `Clean`(AMSI 가용 시) 또는 `Unavailable`(제공자
    /// 없음 — CI 러너 변동 허용). `Detected`가 나오면 그건 진짜 문제다.
    #[cfg(windows)]
    #[test]
    fn windows_harmless_bytes_never_detected() {
        let got = scan("hello.txt", "안녕 nexa-beep 무해 텍스트".as_bytes());
        println!("AMSI 무해 스캔 = {got:?}");
        assert_ne!(got, ScanOutcome::Detected, "무해 텍스트가 탐지되면 안 된다");
    }

    /// Windows — **EICAR 표준 시험 문자열은 탐지돼야 한다**(가용 시 실측 —
    /// 추정 금지). 문자열은 **런타임 조립**: 소스·바이너리에 연속으로 실리면
    /// 백신이 저장소·산출물을 격리하는 사고가 난다(표준 관행).
    #[cfg(windows)]
    #[test]
    fn windows_eicar_is_detected_when_available() {
        let eicar = format!(
            "{}{}",
            r"X5O!P%@AP[4\PZX54(P^)7CC)7}$", "EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"
        );
        let got = scan("eicar.com", eicar.as_bytes());
        println!("AMSI EICAR 스캔 = {got:?}");
        if got == ScanOutcome::Unavailable {
            eprintln!("(AMSI 제공자 없음 — 환경 탓 건너뜀)");
            return;
        }
        assert_eq!(got, ScanOutcome::Detected, "EICAR는 탐지가 표준이다");
    }
}
