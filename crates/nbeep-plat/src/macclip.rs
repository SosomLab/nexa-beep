//! macOS 클립보드 **자체 구현**(08-30 · L-1의 mac 판 — 도구 스폰 폐지).
//!
//! 종전 경로는 `pbcopy`/`pbpaste`(텍스트)·`osascript`(이미지 → 임시 파일)였다.
//! Linux 08-29 실기에서 "이미지 붙여넣기 전송 = Windows만"이 보고됐고 mac의
//! osascript 경로도 미동작(샌드박스·권한 프롬프트·임시 파일 왕복)이라 **3-OS
//! 자체 구현 통일**로 확정됐다. 여기서는 `NSPasteboard`를 objc2로 직접 부른다 —
//! keytap·트레이·알림과 **같은 판**(objc2 0.5 / app-kit 0.2 · 의존 신규 0).
//!
//! 봉투 원리: 클립보드에서 **텍스트 문자열·PNG 바이트**만 꺼낸다. 다른 타입(RTF·
//! 파일 URL·TIFF 원본)은 보지 않는다. TIFF만 있는 경우(Preview·스크린샷 일부)는
//! AppKit `NSBitmapImageRep`으로 PNG로 재포장한다 — R-5(본체는 이미지 코덱을
//! 링크하지 않는다)는 **우리 코드가 디코더를 갖지 않는다**는 뜻이고, 시스템
//! 프레임워크(AppKit)의 변환은 이미 프로세스에 링크된 OS 코드라 새 공격면이 아니다.
//!
//! 상한: 이미지 64MiB(Linux `linuxclip`과 동일 · 할당 폭탄 방어) · 초과 = None.
//!
//! ⚠ **프로세스 안 직렬화**(08-30 실측): 스레드 둘이 동시에 `clearContents`/`setString`
//! 을 부르면 AppKit이 abort(SIGABRT — 테스트 병렬 실행에서 즉시 재현). Windows
//! 경로의 뮤텍스와 같은 이유(이미지 워커 × UI 복사)로 뮤텍스 하나로 직렬화하고,
//! 호출마다 `autoreleasepool`을 두어 메인 루프 밖 스레드에서도 누수가 없게 한다.

use objc2::rc::autoreleasepool;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG,
    NSPasteboardTypeString, NSPasteboardTypeTIFF,
};
use objc2_foundation::{NSDictionary, NSString};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// 프로세스 안 클립보드 접근 직렬화.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 이미지 상한(바이트) — Linux 자체 구현과 동일.
const IMAGE_MAX: usize = 64 * 1024 * 1024;

/// 클립보드 텍스트(`NSPasteboardTypeString` = `public.utf8-plain-text`).
pub(crate) fn get_text() -> Option<String> {
    let _g = serial();
    // SAFETY: generalPasteboard는 프로세스 전역 싱글턴. 접근은 위 뮤텍스로 직렬화.
    // stringForType은 없으면 None. 반환 문자열은 풀 안에서 즉시 Rust String으로 복사.
    autoreleasepool(|_| unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.stringForType(NSPasteboardTypeString)
            .map(|s| s.to_string())
    })
}

/// 클립보드에 텍스트를 쓴다 — `clearContents` 뒤 `setString:forType:`(짝 필수 —
/// clear 없이 set하면 소유권 미확보로 실패한다).
pub(crate) fn set_text(text: &str) -> bool {
    let _g = serial();
    // SAFETY: 위와 동일. NSString은 Rust 쪽 소유(Retained) — AppKit이 복사한다.
    autoreleasepool(|_| unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString)
    })
}

/// 클립보드 이미지 → **PNG 바이트**. PNG 타입이 있으면 그대로 · TIFF만 있으면
/// AppKit으로 PNG 재포장 · 둘 다 없으면 None. 결과는 PNG 서명 확인 후에만 준다.
pub(crate) fn get_image_png() -> Option<Vec<u8>> {
    let _g = serial();
    // SAFETY: NSData.bytes()는 Retained가 살아 있는 동안 유효 — 즉시 Vec으로 복사.
    let raw = autoreleasepool(|_| unsafe {
        let pb = NSPasteboard::generalPasteboard();
        if let Some(d) = pb.dataForType(NSPasteboardTypePNG) {
            if d.len() > IMAGE_MAX {
                return None;
            }
            Some(d.bytes().to_vec())
        } else {
            let tiff = pb.dataForType(NSPasteboardTypeTIFF)?;
            if tiff.len() > IMAGE_MAX {
                return None;
            }
            let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
            let png = rep.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )?;
            if png.len() > IMAGE_MAX {
                return None;
            }
            Some(png.bytes().to_vec())
        }
    })?;
    raw.starts_with(&[0x89, b'P', b'N', b'G']).then_some(raw)
}

#[cfg(test)]
mod tests {
    /// 실측 — 텍스트 왕복(NSPasteboard 직접 · 도구 불요). 헤드리스 세션(SSH)도
    /// generalPasteboard는 동작하므로 건너뛰지 않고 단언한다.
    #[test]
    fn text_roundtrip_nspasteboard() {
        let marker = "nexa-beep NSPasteboard 왕복 ✓ 한글·이모지 🐝";
        assert!(super::set_text(marker), "쓰기");
        assert_eq!(
            super::get_text().as_deref(),
            Some(marker),
            "쓴 그대로 돌아와야 한다"
        );
    }

    /// 실측 보조 — 이미지가 있으면 PNG 서명·크기를 찍는다(없어도 정상 · CI).
    #[test]
    fn image_observable() {
        match super::get_image_png() {
            Some(b) => {
                assert!(b.starts_with(&[0x89, b'P', b'N', b'G']));
                println!("클립보드 PNG {}B", b.len());
            }
            None => println!("(클립보드 이미지 없음 — 건너뜀)"),
        }
    }
}
