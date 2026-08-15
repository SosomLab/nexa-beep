//! OS 알림 어댑터(M3-8 최소 슬라이스 · FR-U-15 — 08-15 사용자 요청 "알림 3-OS").
//!
//! **범위**: 배너/토스트 표시 + 소리 유무(DR-25 신뢰 게이트 — 미검증은 소리 없음).
//! 등급 3종·강등표·수신자 릴레이는 🔴 D-23 확정 후(M2-4b와 함께) — 여기는 그 밑의
//! OS 표시 계층만이다. 내용 정책(미리보기 무해화·파일명 금지 — FR-S-41)은 호출자 몫.
//!
//! - **macOS** = `osascript display notification` 스폰(시스템 기본 도구 — `pbcopy`·
//!   `arp` 선례). 번들 밖 바이너리도 동작한다(UserNotifications는 번들·서명 요구라
//!   포터블(DR-4)에서 불가 — M5-4a 공증 후 재검토).
//! - **Linux** = `notify-send` 스폰(libnotify 도구 · 대부분의 데스크톱에 존재).
//!   없으면 조용히 실패(fail-soft — 알림은 보조 채널이다).
//! - **Windows** = 이 모듈이 아니라 **트레이 풍선**(`tray::TrayHandle::notify` —
//!   Shell_NotifyIcon NIF_INFO · 이미 떠 있는 아이콘 재사용 · AppUserModelID가
//!   필요한 WinRT 토스트는 설치본 전용이라 T0/포터블 원칙과 안 맞는다).

/// 알림 한 건 — 제목·본문·무음 여부(신뢰 게이트: 미검증 = `silent`).
#[derive(Debug)]
pub struct Note<'a> {
    /// 제목(발신자 표시 이름 등).
    pub title: &'a str,
    /// 본문(호출자가 무해화·절단을 끝낸 문자열).
    pub body: &'a str,
    /// 소리 억제(DR-25 — 미검증 발신자).
    pub silent: bool,
}

/// 알림을 띄운다 — 성공 여부(false = 이 OS 경로 없음/실패 · fail-soft).
#[must_use]
pub fn notify(n: &Note<'_>) -> bool {
    imp::notify(n)
}

/// 정식 알림 초기화(M3-8b · macOS 전용 의미 — 다른 OS는 no-op false).
///
/// **번들(.app) 실행이면** `UNUserNotificationCenter`를 켠다: 권한 요청 + 클릭
/// delegate(`on_open` = 알림 클릭 → 앱 열기). **메인 스레드에서, 부팅 때 1회** 호출.
/// 비번들(포터블·개발 실행)은 false — [`notify`]가 osascript 폴백을 그대로 쓴다.
pub fn init<F: Fn() + Send + Sync + 'static>(on_open: F) -> bool {
    imp::init(Box::new(on_open))
}

/// AppleScript 문자열 이스케이프(mac) — 역슬래시·따옴표만이 특수문자다.
/// 스폰 인자로 스크립트를 넘기므로 셸 이스케이프는 불필요(Command = no shell).
#[cfg(any(target_os = "macos", test))]
fn escape_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            // 개행은 본문 줄바꿈으로 유효 — 그대로 둔다.
            _ => out.push(c),
        }
    }
    out
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Note;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{declare_class, msg_send_id, mutability, ClassType, DeclaredClass};
    use objc2_foundation::{NSBundle, NSObjectProtocol, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::OnceLock;

    /// 클릭 콜백(M3-8b — 알림 클릭 = 앱 열기). delegate 응답 스레드는 임의라 Send+Sync.
    static ON_OPEN: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
    /// UN 경로 활성(번들 실행 + 초기화 완료) — false면 osascript 폴백.
    static UN_READY: AtomicBool = AtomicBool::new(false);
    /// 요청 식별자 일련(같은 id는 이전 알림을 교체한다 — 고유하게).
    static SEQ: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        /// delegate 보유(center의 delegate는 비소유 참조 — 우리가 살려 둔다).
        /// 초기화가 메인 스레드 1회라 thread_local이 곧 수명이다(트레이와 같은 문법).
        static DELEGATE: std::cell::RefCell<Option<Retained<NotifyDelegate>>> =
            const { std::cell::RefCell::new(None) };
    }

    declare_class!(
        /// UN 클릭 delegate — 응답(배너 클릭) = `ON_OPEN`(앱 열기).
        struct NotifyDelegate;

        unsafe impl ClassType for NotifyDelegate {
            type Super = NSObject;
            type Mutability = mutability::InteriorMutable;
            const NAME: &'static str = "NbeepNotifyDelegate";
        }

        impl DeclaredClass for NotifyDelegate {
            type Ivars = ();
        }

        unsafe impl NSObjectProtocol for NotifyDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for NotifyDelegate {
            #[method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:)]
            unsafe fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                _response: &UNNotificationResponse,
                completion: &block2::Block<dyn Fn()>,
            ) {
                if let Some(f) = ON_OPEN.get() {
                    f();
                }
                completion.call(());
            }
        }
    );

    pub(super) fn init(on_open: Box<dyn Fn() + Send + Sync>) -> bool {
        // 번들 판정 — UN은 번들 신원(Info.plist bundle id)이 없으면 못 산다.
        // SAFETY: mainBundle 조회는 읽기 전용.
        let bundled = unsafe { NSBundle::mainBundle().bundleIdentifier().is_some() };
        if !bundled {
            return false; // 포터블·개발 실행 — osascript 폴백 유지
        }
        let _ = ON_OPEN.set(on_open);
        // SAFETY: 부팅(메인 스레드) 1회 — delegate 등록 + 권한 요청(비동기 · 결과는
        // fail-soft: 거부돼도 notify는 조용히 무시될 뿐 앱 동작 불변).
        unsafe {
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let del: Retained<NotifyDelegate> = msg_send_id![NotifyDelegate::alloc(), init];
            center.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*del)));
            DELEGATE.with(|d| *d.borrow_mut() = Some(del));
            let opts = UNAuthorizationOptions::UNAuthorizationOptionAlert
                | UNAuthorizationOptions::UNAuthorizationOptionSound;
            let done = block2::StackBlock::new(
                |_granted: objc2::runtime::Bool, _err: *mut objc2_foundation::NSError| {},
            );
            center.requestAuthorizationWithOptions_completionHandler(opts, &done);
        }
        UN_READY.store(true, Ordering::Release);
        true
    }

    pub(super) fn notify(n: &Note<'_>) -> bool {
        if UN_READY.load(Ordering::Acquire) {
            // 정식 경로(번들) — 알림 소유가 우리 앱 · 클릭 = delegate → 앱 열기.
            // SAFETY: UN 센터는 스레드 안전 · 우리는 메인에서만 부른다.
            unsafe {
                let content: Retained<UNMutableNotificationContent> =
                    msg_send_id![UNMutableNotificationContent::alloc(), init];
                content.setTitle(&NSString::from_str(n.title));
                content.setBody(&NSString::from_str(n.body));
                if n.silent {
                    content.setSound(None); // DR-25 — 미검증은 소리 없음
                } else {
                    content.setSound(Some(&UNNotificationSound::defaultSound()));
                }
                let id = format!("nbeep-{}", SEQ.fetch_add(1, Ordering::Relaxed));
                let req = UNNotificationRequest::requestWithIdentifier_content_trigger(
                    &NSString::from_str(&id),
                    &content,
                    None, // 트리거 없음 = 즉시
                );
                UNUserNotificationCenter::currentNotificationCenter()
                    .addNotificationRequest_withCompletionHandler(&req, None);
            }
            return true;
        }
        osascript(n)
    }

    /// 비번들 폴백 — 표시만 가능(클릭 콜백 없음 · 소유자 = Script Editor 한계 명문).
    fn osascript(n: &Note<'_>) -> bool {
        let mut script = format!(
            "display notification \"{}\" with title \"{}\"",
            super::escape_applescript(n.body),
            super::escape_applescript(n.title),
        );
        if !n.silent {
            script.push_str(" sound name \"default\"");
        }
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::Note;

    pub(super) fn init(_on_open: Box<dyn Fn() + Send + Sync>) -> bool {
        false // 정식 초기화는 macOS 의미 — Linux는 notify-send 그대로
    }

    pub(super) fn notify(n: &Note<'_>) -> bool {
        let mut cmd = std::process::Command::new("notify-send");
        cmd.arg("--app-name=Nexa Beep");
        if n.silent {
            // 데몬 재량 힌트(관용) — 미지원이면 무시된다(무해).
            cmd.arg("--hint=boolean:suppress-sound:true");
        }
        cmd.arg(n.title).arg(n.body);
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::Note;

    pub(super) fn init(_on_open: Box<dyn Fn() + Send + Sync>) -> bool {
        false // Windows = 트레이 풍선(클릭 포함) · 기타 OS = 없음
    }

    pub(super) fn notify(_n: &Note<'_>) -> bool {
        false // Windows = 트레이 풍선 경로(tray::TrayHandle::notify) · 기타 OS = 없음
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn applescript_escaping_neutralizes_quotes_and_backslashes() {
        // 수신 본문이 스크립트를 탈출하면 임의 osascript 실행이 된다 — 필수 회귀.
        assert_eq!(super::escape_applescript(r#"a"b\c"#), r#"a\"b\\c"#,);
        assert_eq!(super::escape_applescript("줄\n바꿈"), "줄\n바꿈");
    }
}
