//! **keytap — macOS 로컬 keydown 관찰**(G1 · H-26 보충 주입의 관측 절반).
//!
//! 실측([34 §2-8](../../../docs/34-hangul-input-issues.md)): macOS에서 **IME 조합 세션이
//! 끝난 뒤 첫 1byte keydown 1개가 세션당 1회, winit(NSView `interpretKeyEvents`) 경계에서
//! 소비**돼 앱에 도달하지 않는다. 앱 코드로는 원리적으로 복구할 수 없어(이벤트 부재),
//! NSEvent **로컬 모니터**로 keydown을 winit보다 먼저 관찰해 호스트에 알린다 —
//! 주입 여부 판정(삼중 조건: 조합 직후 + winit 미도달 + 모니터 도달)은 호스트 몫이다.
//!
//! DR-21 이음새: 이 모듈은 winit·UI를 모른다 — 콜백(char)만 부른다.
//! 수명(13 §12-1): 모니터는 **앱 수명 전체 · 단일 설치**라 해제 경로가 없다(의도).
//! 콜백은 AppKit 메인 스레드에서 불린다.

/// 무수식(⌘/⌃/⌥ 없음) 인쇄 가능 **ASCII 1byte** keydown을 관찰해 `on_key`로 알린다.
///
/// 대상을 ASCII로 좁히는 이유: 소비 규칙이 "1byte 문자"에서만 실측됐고, 한글 등
/// 다바이트는 IME 경로(Preedit/Commit)가 정상 배달한다 — 넓히면 이중 주입 위험만 는다.
#[cfg(target_os = "macos")]
pub fn install_keydown_tap(on_key: Box<dyn Fn(char)>) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
    use std::ptr::NonNull;

    let block = RcBlock::new(move |ev: NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { ev.as_ref() };
        let mods = unsafe { e.modifierFlags() };
        let clean = !mods.intersects(
            NSEventModifierFlags::NSEventModifierFlagCommand
                .union(NSEventModifierFlags::NSEventModifierFlagControl)
                .union(NSEventModifierFlags::NSEventModifierFlagOption),
        );
        if clean {
            if let Some(s) = unsafe { e.characters() } {
                if let Some(c) = s.to_string().chars().next() {
                    if c.is_ascii() && !c.is_ascii_control() {
                        on_key(c);
                    }
                }
            }
        }
        ev.as_ptr()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
    };
    // 앱 수명 전체 유지(단일 설치) — 반환 토큰·블록을 의도적으로 살려 둔다.
    std::mem::forget(monitor);
    std::mem::forget(block);
}
