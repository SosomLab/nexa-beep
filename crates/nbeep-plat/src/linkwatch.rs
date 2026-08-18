//! L1 링크 변화 **구독**(M1-2 · DR-14 L1 · FR-D-5) — OS별 어댑터.
//!
//! Wi-Fi 전환·케이블 분리·절전 복귀 같은 링크 변화를 OS에서 직접 구독한다 — 폴링
//! 금지. 채널로 오는 신호는 **raw**("무언가 변했다" · 폭주 가능)이고, 접기(디바운스)는
//! `nbeep_core::linkwatch::Debouncer`가 맡는다(순수 로직 — 시각 주입 테스트).
//!
//! ⚠️ 위를 **intra-doc 링크로 걸지 않는다** — `nbeep-core`는 이 크레이트에서
//! `[target.'cfg(unix)'.dependencies]`라 **Windows 빌드에는 링크되지 않고**, 그러면
//! rustdoc이 "unresolved link"로 실패한다(`-D warnings` = CI red · 08-15 실측).
//! 이 저장소가 반복해 밟는 *"조건부 컴파일은 반대편을 의심한다"* 의 문서판이다.
//!
//! 봉투 원리: 이벤트 **내용은 파싱하지 않는다** — v1이 필요한 정보는 "변했다"뿐이라
//! 어떤 어댑터도 메시지 본문을 읽지 않는다(인터페이스별 세분화는 M1-3 축과 함께).
//!
//! - macOS = `PF_ROUTE` 원시 소켓(라우팅·인터페이스·주소 변화가 전부 흐른다)
//! - Linux = `NETLINK_ROUTE`(RTMGRP_LINK + v4/v6 IFADDR 그룹)
//! - Windows = `NotifyIpInterfaceChange`(iphlpapi 콜백 → 채널)
//!
//! 실패(권한·미지원)는 `None` — 구독 없이도 앱은 동작한다(주기 광고가 폴백 ·
//! fail-soft). 구독 해지는 없다 — 앱 수명과 같다(부팅 1회 spawn).

use std::sync::mpsc::Receiver;

/// 링크 변화 구독을 시작한다 — 수신단으로 raw 신호가 온다. 실패 시 `None`.
#[must_use]
pub fn spawn() -> Option<Receiver<()>> {
    imp::spawn()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::mpsc::{channel, Receiver};

    pub(super) fn spawn() -> Option<Receiver<()>> {
        // SAFETY: PF_ROUTE 소켓 fd를 이 스레드에서만 read하고, 종료 경로에서 close.
        let fd = unsafe { libc::socket(libc::PF_ROUTE, libc::SOCK_RAW, 0) };
        if fd < 0 {
            return None;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            loop {
                // SAFETY: buf 범위 내 read. 0 이하 = 소켓 닫힘/오류 → 스레드 종료.
                let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    // SAFETY: 위에서 연 fd — 한 번만 닫는다.
                    unsafe { libc::close(fd) };
                    return;
                }
                // ★ 자기 유발 무시(08-16 실기 — 무한 루프): 재발견 kick의 멀티캐스트
                //   재조인이 RTM_NEWMADDR를 방출하고, 무파싱이던 이 감시가 그걸 다시
                //   "변화"로 읽어 **감지 → 재조인 → 감지** 1초 루프가 됐다(전송 중
                //   "네트워크 변경 감지"·프로필 재수신 반복). 헤더의 타입 1바이트만
                //   보고 링크·주소 변화만 신호한다 — 내용은 여전히 안 본다.
                //   (rt_msghdr: [len u16][version u8][type u8] — mac ABI 고정.)
                if n >= 4 {
                    let t = i32::from(buf[3]);
                    let relevant =
                        t == libc::RTM_IFINFO || t == libc::RTM_NEWADDR || t == libc::RTM_DELADDR;
                    if !relevant {
                        continue; // 멀티캐스트 조인·라우팅 캐시 등 — 전환이 아니다
                    }
                }
                if tx.send(()).is_err() {
                    // SAFETY: 위에서 연 fd — 한 번만 닫는다.
                    unsafe { libc::close(fd) };
                    return;
                }
            }
        });
        Some(rx)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::sync::mpsc::{channel, Receiver};

    pub(super) fn spawn() -> Option<Receiver<()>> {
        // SAFETY: netlink 소켓 생성·바인딩 — 실패 시 즉시 close. 이후 read 전용.
        unsafe {
            let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
            if fd < 0 {
                return None;
            }
            let mut addr: libc::sockaddr_nl = std::mem::zeroed();
            addr.nl_family = libc::sa_family_t::try_from(libc::AF_NETLINK).unwrap_or(0);
            addr.nl_groups =
                (libc::RTMGRP_LINK | libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV6_IFADDR) as u32;
            let rc = libc::bind(
                fd,
                std::ptr::addr_of!(addr).cast::<libc::sockaddr>(),
                u32::try_from(std::mem::size_of::<libc::sockaddr_nl>()).unwrap_or(0),
            );
            if rc < 0 {
                libc::close(fd);
                return None;
            }
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    let n = libc::read(fd, buf.as_mut_ptr().cast(), buf.len());
                    if n <= 0 {
                        libc::close(fd);
                        return;
                    }
                    // 타입 필터(RL-4 · 08-18) — mac PF_ROUTE 필터와 **플랫폼 대칭**.
                    // 무파싱 감시는 자기 행동(재조인 등)을 다시 듣는다(08-16 mac
                    // 1초 루프 실증). nlmsghdr.nlmsg_type(offset 4)만 보고 내용은
                    // 여전히 읽지 않는다(봉투 원리) — 링크 상태·주소 변화만 통과.
                    let relevant = n as usize >= 6 && {
                        let t = u16::from_ne_bytes([buf[4], buf[5]]);
                        t == libc::RTM_NEWLINK
                            || t == libc::RTM_DELLINK
                            || t == libc::RTM_NEWADDR
                            || t == libc::RTM_DELADDR
                    };
                    if relevant && tx.send(()).is_err() {
                        libc::close(fd);
                        return;
                    }
                }
            });
            Some(rx)
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::Mutex;

    // MIB_NOTIFICATION_TYPE 콜백 — 행 내용은 읽지 않는다(봉투 원리 · row는 불투명).
    type Callback = unsafe extern "system" fn(*mut c_void, *mut c_void, u32);

    #[link(name = "iphlpapi")]
    extern "system" {
        fn NotifyIpInterfaceChange(
            family: u16,
            callback: Callback,
            caller_context: *mut c_void,
            initial_notification: u8,
            notification_handle: *mut *mut c_void,
        ) -> u32;
    }

    /// 콜백 스레드가 임의라 `Sender`(!Sync)를 Mutex로 감싼다.
    unsafe extern "system" fn on_change(ctx: *mut c_void, _row: *mut c_void, _kind: u32) {
        let tx = &*ctx.cast::<Mutex<Sender<()>>>();
        if let Ok(g) = tx.lock() {
            let _ = g.send(());
        }
    }

    pub(super) fn spawn() -> Option<Receiver<()>> {
        let (tx, rx) = channel();
        // 구독은 앱 수명과 같다 — 컨텍스트를 의도적으로 누수(해지 경로 없음 · 부팅 1회).
        let ctx = Box::into_raw(Box::new(Mutex::new(tx)));
        let mut handle: *mut c_void = std::ptr::null_mut();
        // AF_UNSPEC(0) = v4+v6 전부 · initial_notification=0(현재 상태 콜백 생략).
        let rc = unsafe { NotifyIpInterfaceChange(0, on_change, ctx.cast(), 0, &mut handle) };
        if rc == 0 {
            Some(rx)
        } else {
            // SAFETY: 등록 실패 — 우리가 만든 박스를 되찾아 해제.
            drop(unsafe { Box::from_raw(ctx) });
            None
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
    use std::sync::mpsc::Receiver;

    pub(super) fn spawn() -> Option<Receiver<()>> {
        None // 미지원 플랫폼 — 주기 광고 폴백
    }
}

#[cfg(test)]
mod tests {
    /// 구독 스모크 — 열리기만 하면 된다(이벤트 유발은 실기 몫: 26 §7 링크 토글).
    /// 권한·환경에 따라 None일 수 있어 성공 여부는 단언하지 않는다(fail-soft 계약).
    #[test]
    fn spawn_does_not_panic() {
        let _ = super::spawn();
    }
}
