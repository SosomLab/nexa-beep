//! App Nap 방지(11-21 · macOS) — 보이는 창이 없어도 타이머가 제때 돌게 한다.
//!
//! macOS는 **보이는 창이 없는 앱**(트레이로 숨김 · 가려짐 · 헤드리스)을 App Nap 대상으로 삼아 타이머를
//! 초 단위로 미룬다. 이 앱은 숨어 있을 때도 타이머로 도는 일이 있다(재연결 백오프 · 발견 만료 sweep · 거절
//! 실행 취소 만기 · 기록 저장 디바운스 · 상태 전환) — 메신저는 창이 없을 때가 평상시다.
//!
//! 실측(11-21 · mac 헤드리스 5-인스턴스 그룹 시나리오): 스크립트 단계가 예정보다 3~8초 늦고, 여러
//! 인스턴스가 **같은 벽시계 시각에 함께** 풀렸다. `sample` 결과 메인 스레드는 그동안 바쁘지 않고
//! `mach_msg`(런루프 대기)에 있었다 = 깨워 줄 타이머가 미뤄진 것. 네트워크 수신처럼 다른 스레드가 보내는
//! 사용자 이벤트는 런루프를 직접 깨워 영향이 없고, **타이머만** 밀린다.
//!
//! 처방 = `NSProcessInfo.beginActivityWithOptions:reason:`을 프로세스 수명 동안 쥔다. 옵션은
//! `NSActivityUserInitiatedAllowingIdleSystemSleep` — App Nap·타이머 조절만 막고 **시스템 잠자기는 막지
//! 않는다**(노트북이 잠들면 앱도 같이 잔다 · 배터리 배려). 다른 OS는 no-op.

/// App Nap을 막는다 — 프로세스당 1회(다시 불러도 같은 토큰). 반환 = 이 OS에서 실제로 걸었는가.
/// 토큰은 프로세스가 끝날 때까지 쥔다(끝내는 API를 두지 않는다 — 이 앱에 "타이머가 밀려도 되는" 구간이 없다).
pub fn hold(reason: &str) -> bool {
    imp::hold(reason)
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send_id};
    use objc2_foundation::NSString;
    use std::sync::OnceLock;

    /// `NSActivityUserInitiatedAllowingIdleSystemSleep`(NSProcessInfo.h) =
    /// `(0x00FFFFFF | IdleSystemSleepDisabled(1<<20)) & !IdleSystemSleepDisabled`.
    const USER_INITIATED_ALLOWING_IDLE_SLEEP: u64 = 0x00FF_FFFF & !(1 << 20);

    /// 활동 토큰 — 놓으면 App Nap이 돌아오므로 static에 쥔다.
    struct Token(#[allow(dead_code)] Retained<AnyObject>);
    // SAFETY: 토큰은 불투명 객체로 보관만 한다(메서드 호출 없음) · NSProcessInfo 활동 API는 스레드 안전.
    unsafe impl Send for Token {}
    // SAFETY: 위와 같다 — 공유 참조로 하는 일이 없다.
    unsafe impl Sync for Token {}

    static TOKEN: OnceLock<Token> = OnceLock::new();

    pub(super) fn hold(reason: &str) -> bool {
        TOKEN.get_or_init(|| {
            let reason = NSString::from_str(reason);
            // SAFETY: `+[NSProcessInfo processInfo]`는 항상 유효한 싱글턴 · `beginActivityWithOptions:reason:`은
            // 인자가 (uint64 옵션, NSString*)이고 반환이 retain 가능한 객체(id<NSObject>)다.
            unsafe {
                let info: Retained<AnyObject> = msg_send_id![class!(NSProcessInfo), processInfo];
                let token: Retained<AnyObject> = msg_send_id![
                    &info,
                    beginActivityWithOptions: USER_INITIATED_ALLOWING_IDLE_SLEEP,
                    reason: &*reason
                ];
                Token(token)
            }
        });
        true
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub(super) fn hold(_reason: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hold_is_idempotent() {
        // mac = 두 번 불러도 같은 토큰(패닉·이중 시작 없음) · 그 외 = false.
        let a = super::hold("test");
        let b = super::hold("test");
        assert_eq!(a, b);
        assert_eq!(a, cfg!(target_os = "macos"));
    }
}
