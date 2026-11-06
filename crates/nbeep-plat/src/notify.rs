//! OS 알림 어댑터(M3-8 최소 슬라이스 · FR-U-15 — 08-15 사용자 요청 "알림 3-OS").
//!
//! **범위**: 배너/토스트 표시 + 소리 유무(DR-25 신뢰 게이트 — 미검증은 소리 없음).
//! 등급 3종·강등표·수신자 릴레이는 🔴 D-23 확정 후(M2-4b와 함께) — 여기는 그 밑의
//! OS 표시 계층만이다. 내용 정책(미리보기 무해화·파일명 금지 — FR-S-41)은 호출자 몫.
//!
//! - **macOS** = `osascript display notification` 스폰(시스템 기본 도구 — `pbcopy`·
//!   `arp` 선례). 번들 밖 바이너리도 동작한다(UserNotifications는 번들·서명 요구라
//!   포터블(DR-4)에서 불가 — M5-4a 공증 후 재검토).
//! - **Linux**(L-3 · 11-06) = **`org.freedesktop.Notifications` D-Bus 직접**(zbus blocking · 상주 연결 1개 ·
//!   `notify-send` 스폰 폐지). `Notify` 힌트 = urgency(u8) · suppress-sound · sound-name · desktop-entry.
//!   클릭 = `ActionInvoked("default")` → `init` 콜백으로 대상 토큰 왕복(Win/mac 동등) · 전면화 토큰은
//!   `ActivationToken` 시그널(spec 1.2 · GNOME/KDE)을 트레이의 토큰 저장소에 넣어 기존 `raise_main` 경로가 쓴다.
//!   세션 버스·데몬 부재 = `false`(fail-soft — 호스트가 "소리 불가" 1회 고지 · ADR-0010 §4-3).
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
    /// 소리 억제(DR-25 — 미검증 발신자 · 설정 off).
    pub silent: bool,
    /// 긴급(Alarm · ADR-0010 §3-2) — Linux는 `--urgency=critical` + 데몬 `sound-name` 힌트(소리는 데몬이
    /// 낸다 · 4-3 권고). mac/Win은 호스트가 [`crate::sound`]로 따로 울린다.
    pub urgent: bool,
    /// 클릭 시 되돌아올 **불투명 대상 토큰**(호스트만 해석 · 빈 = 대상 없음).
    /// mac 정식(UN) 경로 = 요청 identifier에 실어 delegate 응답에서 파싱 · Linux = 알림 id → 토큰 맵
    /// (`ActionInvoked`) — 둘 다 `init`의 콜백 인자로 돌려준다.
    pub target: &'a str,
}

/// 알림을 띄운다 — 성공 여부(false = 이 OS 경로 없음/실패 · fail-soft).
#[must_use]
pub fn notify(n: &Note<'_>) -> bool {
    imp::notify(n)
}

/// 정식 알림 초기화(M3-8b · macOS + Linux(L-3) — Windows는 no-op false · 트레이 풍선이 클릭을 맡는다).
///
/// **번들(.app) 실행이면** `UNUserNotificationCenter`를 켠다: 권한 요청 + 클릭
/// delegate(`on_open` = 알림 클릭 → 앱 열기). **메인 스레드에서, 부팅 때 1회** 호출.
/// 비번들(포터블·개발 실행)은 false — [`notify`]가 osascript 폴백을 그대로 쓴다.
/// 콜백 인자 = 클릭된 알림의 대상 토큰(`Note::target` 왕복 · None = 대상 없음).
pub fn init<F: Fn(Option<String>) + Send + Sync + 'static>(on_open: F) -> bool {
    imp::init(Box::new(on_open))
}

/// UN 요청 identifier에서 대상 토큰을 파싱(형식 = `"{일련}|{토큰}"` · 토큰 없으면 None).
/// 일련 접두는 identifier 고유성 몫이고, 토큰은 그대로 돌려준다(불투명).
#[cfg(any(target_os = "macos", test))]
fn parse_target(identifier: &str) -> Option<String> {
    identifier
        .split_once('|')
        .map(|(_, t)| t)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
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

    /// 클릭 콜백(M3-8b — 알림 클릭 = 앱 열기 + 대상 대화). 응답 스레드 임의 = Send+Sync.
    static ON_OPEN: OnceLock<Box<dyn Fn(Option<String>) + Send + Sync>> = OnceLock::new();
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
                response: &UNNotificationResponse,
                completion: &block2::Block<dyn Fn()>,
            ) {
                // 어느 알림인지 = 요청 identifier(우리가 대상 토큰을 실어 뒀다).
                let target = {
                    let id = response.notification().request().identifier();
                    super::parse_target(&id.to_string())
                };
                if let Some(f) = ON_OPEN.get() {
                    f(target);
                }
                completion.call(());
            }
        }
    );

    pub(super) fn init(on_open: Box<dyn Fn(Option<String>) + Send + Sync>) -> bool {
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
                // identifier = 일련 + 대상 토큰(클릭 왕복 — parse_target과 한 쌍).
                let id = format!("{}|{}", SEQ.fetch_add(1, Ordering::Relaxed), n.target);
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
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::Value;

    const DEST: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";
    const IFACE: &str = "org.freedesktop.Notifications";
    const APP_NAME: &str = "Nexa Beep";
    /// 아이콘·desktop-entry = 설치본 `.desktop` 이름(packaging/linux · `with_name("nexa-beep", …)`와 같은 값).
    const DESKTOP_ID: &str = "nexa-beep";

    /// 세션 버스 연결 — 프로세스에 하나(알림마다 새로 열지 않는다). `None` = 버스 없음(CI Xvfb · 헤드리스).
    static CONN: OnceLock<Option<Connection>> = OnceLock::new();
    /// 클릭 콜백(`init`).
    static ON_OPEN: OnceLock<Box<dyn Fn(Option<String>) + Send + Sync>> = OnceLock::new();
    /// 알림 id → 대상 토큰(클릭 왕복 · 닫히면 지운다).
    static TARGETS: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();

    fn conn() -> Option<&'static Connection> {
        CONN.get_or_init(|| Connection::session().ok()).as_ref()
    }

    fn targets() -> &'static Mutex<HashMap<u32, String>> {
        TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// `Notify` 힌트 — urgency(u8: 1 보통 · 2 critical) · suppress-sound · sound-name(데몬이 소리를 낸다 ·
    /// libcanberra LGPL 회피) · desktop-entry(아이콘·앱 식별).
    pub(super) fn hints(silent: bool, urgent: bool) -> HashMap<&'static str, Value<'static>> {
        let mut h = HashMap::new();
        h.insert("urgency", Value::U8(if urgent { 2 } else { 1 }));
        h.insert("desktop-entry", Value::from(DESKTOP_ID));
        if silent {
            h.insert("suppress-sound", Value::Bool(true));
        }
        if urgent && !silent {
            h.insert("sound-name", Value::from("message-new-instant"));
        }
        h
    }

    pub(super) fn init(on_open: Box<dyn Fn(Option<String>) + Send + Sync>) -> bool {
        let Some(c) = conn() else {
            return false; // 세션 버스 없음 — notify도 false로 떨어진다(fail-soft)
        };
        if ON_OPEN.set(on_open).is_err() {
            return true; // 이미 초기화(재호출 무해)
        }
        let c = c.clone();
        // 시그널 수신 스레드(theme.rs 관용구) — ActivationToken(토큰 → 트레이 저장소) · ActionInvoked(클릭 → 콜백) ·
        // NotificationClosed(맵 정리). 데몬이 없으면 시그널이 안 오고 스레드는 조용히 대기한다.
        std::thread::Builder::new()
            .name("nb-notify-watch".into())
            .spawn(move || {
                let Ok(proxy) = Proxy::new(&c, DEST, PATH, IFACE) else {
                    return;
                };
                let Ok(sigs) = proxy.receive_all_signals() else {
                    return;
                };
                for msg in sigs {
                    let member = msg.header().member().map(|m| m.as_str().to_owned());
                    match member.as_deref() {
                        Some("ActivationToken") => {
                            if let Ok((_, tok)) = msg.body().deserialize::<(u32, String)>() {
                                crate::tray::put_activation_token(tok);
                            }
                        }
                        Some("ActionInvoked") => {
                            if let Ok((id, _key)) = msg.body().deserialize::<(u32, String)>() {
                                let target = targets().lock().ok().and_then(|mut g| g.remove(&id));
                                if let Some(cb) = ON_OPEN.get() {
                                    cb(target);
                                }
                            }
                        }
                        Some("NotificationClosed") => {
                            if let Ok((id, _reason)) = msg.body().deserialize::<(u32, u32)>() {
                                if let Ok(mut g) = targets().lock() {
                                    g.remove(&id);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            })
            .is_ok()
    }

    pub(super) fn notify(n: &Note<'_>) -> bool {
        let Some(c) = conn() else {
            return false;
        };
        let Ok(proxy) = Proxy::new(c, DEST, PATH, IFACE) else {
            return false;
        };
        // "default" 액션 = 본문 클릭(데몬이 액션을 지원하지 않으면 무시 — 표시는 된다).
        let actions: Vec<&str> = if n.target.is_empty() {
            Vec::new()
        } else {
            vec!["default", "Open"]
        };
        let id: u32 = match proxy.call(
            "Notify",
            &(
                APP_NAME,
                0u32,
                DESKTOP_ID,
                n.title,
                n.body,
                actions,
                hints(n.silent, n.urgent),
                -1i32,
            ),
        ) {
            Ok(id) => id,
            Err(_) => return false, // 데몬 없음(ServiceUnknown) 등 — 호스트가 폴백·고지
        };
        if !n.target.is_empty() {
            if let Ok(mut g) = targets().lock() {
                g.insert(id, n.target.to_string());
                // 상한 — 클릭되지 않고 사라진 알림(데몬이 Closed를 안 주는 경우) 누적 방지.
                if g.len() > 256 {
                    let oldest = g.keys().min().copied();
                    if let Some(k) = oldest {
                        g.remove(&k);
                    }
                }
            }
        }
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hints_follow_alert_policy() {
            let h = hints(false, false);
            assert_eq!(h.get("urgency"), Some(&Value::U8(1)));
            assert!(h.contains_key("desktop-entry") && !h.contains_key("sound-name"));
            let u = hints(false, true);
            assert_eq!(u.get("urgency"), Some(&Value::U8(2)));
            assert_eq!(
                u.get("sound-name"),
                Some(&Value::from("message-new-instant"))
            );
            // 무음(미검증·설정 off)이면 critical이라도 소리 힌트 없음 + suppress-sound.
            let s = hints(true, true);
            assert!(!s.contains_key("sound-name"));
            assert_eq!(s.get("suppress-sound"), Some(&Value::Bool(true)));
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::Note;

    pub(super) fn init(_on_open: Box<dyn Fn(Option<String>) + Send + Sync>) -> bool {
        false // Windows = 트레이 풍선(클릭 포함) · 기타 OS = 없음
    }

    pub(super) fn notify(_n: &Note<'_>) -> bool {
        false // Windows = 트레이 풍선 경로(tray::TrayHandle::notify) · 기타 OS = 없음
    }
}

#[cfg(test)]
mod tests {
    /// 대상 토큰 왕복(M3-8 클릭→대화) — identifier 형식과 파서가 한 쌍이다.
    #[test]
    fn target_roundtrips_through_identifier() {
        assert_eq!(
            super::parse_target("7|p:abcd1234"),
            Some("p:abcd1234".into())
        );
        assert_eq!(super::parse_target("7|"), None, "빈 토큰 = 대상 없음");
        assert_eq!(super::parse_target("legacy-id"), None, "구형식 관용");
        // 토큰 안의 '|'도 살아남는다(첫 구분자만 뗀다 — 불투명 토큰 계약).
        assert_eq!(super::parse_target("1|g:a|b"), Some("g:a|b".into()));
    }

    #[test]
    fn applescript_escaping_neutralizes_quotes_and_backslashes() {
        // 수신 본문이 스크립트를 탈출하면 임의 osascript 실행이 된다 — 필수 회귀.
        assert_eq!(super::escape_applescript(r#"a"b\c"#), r#"a\"b\\c"#,);
        assert_eq!(super::escape_applescript("줄\n바꿈"), "줄\n바꿈");
    }
}
