//! 정상 종료 신호 — **종료 포트**(FR-P-7 · R-16 · DR-21 이음새).
//!
//! OS 종료 요청(Unix `SIGINT`/`SIGTERM`)을 받아 **플래그 하나**로 노출한다. 상위(bin·헤드리스
//! 루프)는 이 플래그를 폴해 **GOODBYE 발신·세션/소켓 정리·zeroize**를 마치고 나간다(FR-D-8의
//! 절반 · NFR-B-6 · FR-S-22).
//!
//! ⚠️ **핸들러만으로는 부족하다** — 블로킹 `accept`/`recv`에 갇히면 시그널이 와도 루프가 안 깬다.
//! 그래서 이 포트는 "요청됨" 플래그만 주고, **깨우기 수단**(recv 타임아웃·논블로킹 accept 폴)은
//! 각 루프가 책임진다([docs/01 §6-1]).
//!
//! ★ **컨테이너 주의**(R-16 실측): PID 1은 커널이 기본 시그널 동작을 적용하지 않아, **핸들러를
//! 설치하지 않으면 SIGTERM이 무시**된다(`docker stop` = 10초 후 SIGKILL). 이 핸들러가 그걸 푼다.
//! 그래도 개발 중 컨테이너는 `docker run --init` 규약을 함께 쓴다([docs/18 §2-1]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 종료 요청 플래그(구독자에 공유).
#[derive(Clone, Debug)]
pub struct Shutdown {
    flag: Arc<AtomicBool>,
}

impl Shutdown {
    /// 종료가 요청됐는가(폴).
    #[must_use]
    pub fn requested(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// 공유 플래그 핸들(다른 스레드·루프로 전달).
    #[must_use]
    pub fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }

    /// 종료를 프로그램적으로 요청한다(GUI 메뉴·주 창 닫기 등에서).
    pub fn request(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
}

#[cfg(unix)]
static SIGNAL_FLAG: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn handle(_sig: libc::c_int) {
    // async-signal-safe: 원자 store만 한다(할당·락 금지).
    SIGNAL_FLAG.store(true, Ordering::Relaxed);
}

/// 종료 시그널 핸들러를 설치하고 [`Shutdown`]을 돌려준다.
///
/// Unix: `SIGINT`+`SIGTERM`을 잡아 플래그를 세운다. 그 외 타깃: 지금은 no-op 플래그
/// (Windows 콘솔/세션 핸들러 `SetConsoleCtrlHandler`는 후속 — 그때까지 `request()`로만).
#[must_use]
pub fn install() -> Shutdown {
    let flag = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    {
        // SAFETY: signal 등록은 프로세스 전역 1회. 핸들러는 원자 store만 하는 async-signal-safe.
        let h = handle as extern "C" fn(libc::c_int) as usize as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGINT, h);
            libc::signal(libc::SIGTERM, h);
        }
        // 시그널 static → 공유 플래그로 옮기는 브릿지 스레드(폴 — 시그널 문맥 밖에서 안전).
        let bridge = Arc::clone(&flag);
        std::thread::spawn(move || loop {
            if SIGNAL_FLAG.load(Ordering::Relaxed) {
                bridge.store(true, Ordering::Relaxed);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        });
    }
    Shutdown { flag }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_sets_flag() {
        let s = install();
        assert!(!s.requested());
        s.request();
        assert!(s.requested());
    }

    #[test]
    fn flag_handle_is_shared() {
        let s = install();
        let f = s.flag();
        assert!(!f.load(Ordering::Relaxed));
        s.request();
        assert!(f.load(Ordering::Relaxed), "공유 플래그에 반영");
    }
}
