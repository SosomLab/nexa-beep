//! Dock/작업표시줄 노출 제어(08-30 · `ui.tray_hide_taskbar`).
//!
//! Windows·Linux는 창 자체(숨김/파괴)로 작업표시줄에서 빠지지만 **macOS Dock 아이콘은
//! 앱 단위**라 창을 숨겨도 남는다 → `NSApplication` activation policy를 `Accessory`로
//! 내리면 Dock·⌘Tab에서 사라지고(메뉴바 트레이만 남음) `Regular`로 올리면 복귀한다.
//! 다른 OS는 no-op(호출부 cfg 분기 없음).

/// Dock 아이콘 표시 여부(mac) — **메인 스레드에서만**(AppKit 규약 · winit 이벤트 루프).
pub fn set_dock_visible(visible: bool) {
    imp::set_dock_visible(visible);
}

/// ★ M3-22(11-06) 앱 아이콘 — **번들 밖 실행**(포터블 zip · `cargo run`)이면 Dock에 기본 exec 아이콘이 뜬다.
/// `NSApplication.applicationIconImage`에 내장 RGBA(정사각 `side`)를 꽂는다(번들은 Info.plist 아이콘이 이미 맞으니
/// 덮어도 같은 그림). 다른 OS는 no-op(Windows = exe 리소스 아이콘 · Linux = `.desktop` Icon · 포터블 Linux는 한계).
/// **메인 스레드에서만**.
pub fn set_app_icon(rgba: &[u8], side: u32) {
    imp::set_app_icon(rgba, side);
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::msg_send_id;
    use objc2::rc::Retained;
    use objc2::ClassType;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBitmapImageRep, NSImage};
    use objc2_foundation::{MainThreadMarker, NSSize, NSString};

    pub(super) fn set_app_icon(rgba: &[u8], side: u32) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(img) = image_from_rgba(rgba, side) else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: AppKit 메인 스레드 · 유효한 NSImage.
        unsafe { app.setApplicationIconImage(Some(&img)) };
    }

    /// RGBA → NSImage(tray.rs `image_from_rgba`와 같은 처방 · 크기만 Dock용 포인트).
    fn image_from_rgba(rgba: &[u8], side: u32) -> Option<Retained<NSImage>> {
        if side == 0 || rgba.len() != (side as usize) * (side as usize) * 4 {
            return None;
        }
        unsafe {
            let rep: Option<Retained<NSBitmapImageRep>> = msg_send_id![
                NSBitmapImageRep::alloc(),
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
                pixelsWide: side as isize,
                pixelsHigh: side as isize,
                bitsPerSample: 8_isize,
                samplesPerPixel: 4_isize,
                hasAlpha: true,
                isPlanar: false,
                colorSpaceName: &*NSString::from_str("NSDeviceRGBColorSpace"),
                bytesPerRow: (side * 4) as isize,
                bitsPerPixel: 32_isize,
            ];
            let rep = rep?;
            let data = rep.bitmapData();
            if data.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), data, rgba.len());
            let pt = f64::from(side);
            let img = NSImage::initWithSize(NSImage::alloc(), NSSize::new(pt, pt));
            img.addRepresentation(&rep);
            Some(img)
        }
    }

    pub(super) fn set_dock_visible(visible: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            return; // 메인 스레드가 아니면 하지 않는다(fail-soft)
        };
        let app = NSApplication::sharedApplication(mtm);
        let policy = if visible {
            NSApplicationActivationPolicy::Regular
        } else {
            NSApplicationActivationPolicy::Accessory
        };
        app.setActivationPolicy(policy);
        if visible {
            // Accessory→Regular 직후엔 앱이 비활성일 수 있다 — 앞으로 가져온다.
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub(super) fn set_dock_visible(_visible: bool) {}
    pub(super) fn set_app_icon(_rgba: &[u8], _side: u32) {}
}
