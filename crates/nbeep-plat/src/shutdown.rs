//! 정상 종료 신호 — **종료 포트**(FR-P-7 · R-16 · DR-21 이음새).
//!
//! OS 종료 요청(Unix `SIGINT`/`SIGTERM` · Windows 콘솔 `Ctrl+C`/`Ctrl+Break`/창 닫기/로그오프/종료)을 받아
//! **플래그 하나**로 노출한다. 상위(bin·헤드리스
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
#[cfg(any(unix, windows))]
use std::sync::{Mutex, OnceLock, Weak};

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

#[cfg(any(unix, windows))]
static SIGNAL_FLAG: AtomicBool = AtomicBool::new(false);
/// 자기 파이프(self-pipe) 쓰기 끝(RL-13 ⓒ · 11-04) — 핸들러가 1바이트를 쓰고 브릿지 스레드가 블로킹 read로 깬다.
/// -1 = 파이프 없음(폴 폴백).
#[cfg(unix)]
static PIPE_W: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
/// 살아 있는 구독 플래그(프로세스 전역 · `install` 호출마다 1개) — 브릿지 1개가 전부 깨운다.
#[cfg(any(unix, windows))]
static REGISTRY: Mutex<Vec<Weak<AtomicBool>>> = Mutex::new(Vec::new());
/// 핸들러·파이프·브릿지 스레드는 프로세스에 **한 번**(종전 = `install`마다 50ms 폴 스레드 1개 영구 잔존 ·
/// `--chat-live`는 재대기마다 호출해 스레드가 쌓였다).
#[cfg(unix)]
static BRIDGE: OnceLock<()> = OnceLock::new();

#[cfg(unix)]
extern "C" fn handle(_sig: libc::c_int) {
    // async-signal-safe: 원자 store + write(2)만 한다(할당·락 금지).
    SIGNAL_FLAG.store(true, Ordering::Relaxed);
    let fd = PIPE_W.load(Ordering::Relaxed);
    if fd >= 0 {
        let b = [1u8];
        // SAFETY: 유효한 fd에 1바이트 — 파이프가 찼어도(비블로킹 아님·브릿지가 비운다) 핸들러는 1바이트뿐.
        unsafe {
            libc::write(fd, b.as_ptr().cast::<libc::c_void>(), 1);
        }
    }
}

/// 등록된 플래그 전부에 "요청됨"을 쓰고 죽은 구독은 걷는다.
#[cfg(any(unix, windows))]
fn wake_all() {
    if let Ok(mut g) = REGISTRY.lock() {
        g.retain(|w| {
            w.upgrade().is_some_and(|f| {
                f.store(true, Ordering::Relaxed);
                true
            })
        });
    }
}

/// 브릿지 1회 설치(unix): 자기 파이프 + 블로킹 read 스레드(유휴 웨이크업 0 · RL-10 바닥). 파이프를 못 만들면
/// 종전 50ms 폴로 폴백. 핸들러 등록은 파이프 준비 **뒤**(핸들러가 fd를 본다).
#[cfg(unix)]
fn install_bridge_once() {
    BRIDGE.get_or_init(|| {
        let mut fds = [0i32; 2];
        // SAFETY: pipe(2)에 길이 2 배열.
        let piped = unsafe { libc::pipe(fds.as_mut_ptr()) } == 0;
        if piped {
            // SAFETY: 방금 받은 유효한 fd 둘 — 자식 프로세스(설치 도우미·열기)에 새지 않게 CLOEXEC.
            unsafe {
                libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
                libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);
            }
            PIPE_W.store(fds[1], Ordering::Relaxed);
            let r = fds[0];
            std::thread::spawn(move || loop {
                let mut b = [0u8; 1];
                // SAFETY: 읽기 끝 fd에 1바이트 버퍼 — 프로세스가 끝날 때까지 닫지 않는다.
                let n = unsafe { libc::read(r, b.as_mut_ptr().cast::<libc::c_void>(), 1) };
                if n < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                if n <= 0 {
                    return; // 파이프가 닫혔다(일어나지 않는다) — 폴백 없이 끝
                }
                wake_all();
            });
        } else {
            // 폴 폴백(종전 방식 · 50ms) — 프로세스에 하나뿐.
            std::thread::spawn(|| loop {
                if SIGNAL_FLAG.load(Ordering::Relaxed) {
                    wake_all();
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            });
        }
        // SAFETY: signal 등록은 프로세스 전역 1회. 핸들러는 원자 store·write(2)만 하는 async-signal-safe.
        let h = handle as extern "C" fn(libc::c_int) as usize as libc::sighandler_t;
        unsafe {
            libc::signal(libc::SIGINT, h);
            libc::signal(libc::SIGTERM, h);
        }
    });
}

/// Windows 콘솔 제어 핸들러(M1-8x 잔여 · R-16 · 11-06) — `SetConsoleCtrlHandler`. 콘솔이 붙은 프로세스(CLI
/// 서브커맨드 · `attach_parent_console`/`own_console_for_interactive` 뒤)에만 이벤트가 온다. `windows` 서브시스템
/// GUI(탐색기 실행)는 콘솔이 없어 받지 않는다 — 그쪽 종료는 창 닫기·트레이·`request()`가 맡는다(로그오프의
/// `WM_ENDSESSION`은 별도 슬라이스).
#[cfg(windows)]
mod win {
    pub(super) const CTRL_CLOSE_EVENT: u32 = 2;
    pub(super) const CTRL_LOGOFF_EVENT: u32 = 5;
    pub(super) const CTRL_SHUTDOWN_EVENT: u32 = 6;
    #[link(name = "kernel32")]
    extern "system" {
        pub(super) fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
}

/// 핸들러 1회 설치(windows).
#[cfg(windows)]
static HANDLER: OnceLock<bool> = OnceLock::new();

/// 콘솔 제어 이벤트 — OS가 **별도 스레드**에서 부른다(시그널 핸들러와 달리 락·sleep 가능).
#[cfg(windows)]
unsafe extern "system" fn ctrl_handler(kind: u32) -> i32 {
    SIGNAL_FLAG.store(true, Ordering::Relaxed);
    wake_all();
    if matches!(
        kind,
        win::CTRL_CLOSE_EVENT | win::CTRL_LOGOFF_EVENT | win::CTRL_SHUTDOWN_EVENT
    ) {
        // 핸들러가 돌아오면 OS가 곧(≤5s) 프로세스를 끝낸다 — 루프가 정리(기록 flush · Bye · 설정 저장)할 시간을
        // 준다. Ctrl+C/Break는 돌아와도 죽지 않으므로 즉시 반환(루프가 플래그를 보고 나간다).
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    1 // TRUE = 처리함(기본 핸들러로 넘기지 않는다 — 즉시 종료 방지)
}

#[cfg(windows)]
fn install_handler_once() {
    HANDLER.get_or_init(|| {
        // SAFETY: 프로세스 전역 1회 등록 · 핸들러는 원자 store·레지스트리 락·sleep만 한다.
        unsafe { win::SetConsoleCtrlHandler(Some(ctrl_handler), 1) != 0 }
    });
}

/// 종료 시그널 핸들러를 설치하고 [`Shutdown`]을 돌려준다.
///
/// Unix: `SIGINT`+`SIGTERM`을 잡아 플래그를 세운다. Windows: 콘솔 제어 핸들러(`SetConsoleCtrlHandler` · 11-06 —
/// Ctrl+C/Break/창 닫기/로그오프/종료 · 콘솔이 붙은 프로세스만). 그 외 타깃: no-op 플래그(`request()`로만).
///
/// ★ RL-13 ⓒ(11-04): 핸들러·브릿지는 프로세스에 **한 번**(자기 파이프 + 블로킹 read 1스레드 · 유휴 웨이크업 0).
/// `install`은 플래그를 등록만 한다 — `--chat-live`처럼 재대기마다 불러도 스레드가 쌓이지 않는다. 등록 전에 이미 온
/// 시그널은 초기값으로 잡는다.
#[must_use]
pub fn install() -> Shutdown {
    #[cfg(unix)]
    let flag = {
        install_bridge_once();
        let flag = Arc::new(AtomicBool::new(SIGNAL_FLAG.load(Ordering::Relaxed)));
        if let Ok(mut g) = REGISTRY.lock() {
            g.retain(|w| w.strong_count() > 0); // 죽은 구독(끝난 대기 루프)은 등록 때 걷는다
            g.push(Arc::downgrade(&flag));
        }
        flag
    };
    #[cfg(windows)]
    let flag = {
        install_handler_once();
        let flag = Arc::new(AtomicBool::new(SIGNAL_FLAG.load(Ordering::Relaxed)));
        if let Ok(mut g) = REGISTRY.lock() {
            g.retain(|w| w.strong_count() > 0);
            g.push(Arc::downgrade(&flag));
        }
        flag
    };
    #[cfg(not(any(unix, windows)))]
    let flag = Arc::new(AtomicBool::new(false));
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

    /// RL-13 ⓒ: 여러 번 설치해도 플래그는 각자(요청이 서로 새지 않는다) · unix면 등록부에 살아 있는 것만 남는다.
    #[test]
    fn repeated_install_keeps_flags_separate() {
        let a = install();
        let b = install();
        a.request();
        assert!(a.requested());
        assert!(!b.requested(), "다른 구독의 요청이 새지 않는다");
        #[cfg(any(unix, windows))]
        {
            drop(a);
            let _c = install(); // 등록 때 죽은 구독을 걷는다
            let live = REGISTRY
                .lock()
                .unwrap()
                .iter()
                .filter(|w| w.strong_count() > 0)
                .count();
            assert!(live >= 2, "b·c는 살아 있다(병렬 시험의 다른 구독 포함)");
        }
    }

    /// R-16(11-06): 외부 신호(콘솔 이벤트·시그널)가 오면 **등록된 구독 전부**가 깬다 — 핸들러 본체를 시험 안에서
    /// 흉내 낸다(SIGNAL_FLAG + wake_all = 핸들러가 하는 전부).
    #[cfg(any(unix, windows))]
    #[test]
    fn external_signal_wakes_every_subscriber() {
        // SIGNAL_FLAG(프로세스 전역)는 건드리지 않는다 — 병렬 시험의 install() 초기값에 샌다.
        let a = install();
        let b = install();
        wake_all();
        assert!(a.requested() && b.requested(), "두 구독 모두 깬다");
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
