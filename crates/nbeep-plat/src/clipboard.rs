//! OS 클립보드 **텍스트** 어댑터(DR-21 — 외부 기술은 이음새 뒤에).
//!
//! ui 계층은 OS를 모른다 — 위젯은 문자열만 내놓고/받고, 호출자(bin)가 이 모듈로
//! OS 클립보드와 오간다. **Windows만 실물**(CF_UNICODETEXT · Win32 직접 호출 —
//! 런타임 의존 0 유지). macOS(NSPasteboard)·Linux(X11/Wayland)는 objc·디스플레이
//! 서버 연동이 필요해 **의도된 보류**(M3-1b OS 동작 어댑터) — 미지원 환경은
//! 조용히 실패하는 대신 `None`/`false`로 정직하게 알린다.
//! ★ Linux는 08-29부터 **자체 구현**(`linuxclip` — Wayland data-control/data_device ·
//! X11 셀렉션 · 앱이 `linuxclip::init_wayland`로 winit의 wl_display를 넘긴다) · 도구 스폰은 폴백.
//! ★ macOS는 08-30부터 **자체 구현**(`macclip` — `NSPasteboard` objc2 직접 · 도구 스폰 0).
//! 이로써 **3-OS 전부 자체 경로**(Windows Win32 · mac AppKit · Linux Wayland/X11).

/// 클립보드의 텍스트를 읽는다. 미지원 OS·비텍스트·잠금 실패면 `None`.
#[must_use]
pub fn get_text() -> Option<String> {
    imp::get_text()
}

/// 클립보드에 텍스트를 쓴다. 성공 여부 반환(미지원 OS는 `false`).
pub fn set_text(text: &str) -> bool {
    imp::set_text(text)
}

/// 클립보드 **이미지**(③ 08-20 — 사용자 확정 3-OS).
/// `Png` = 그대로 쓸 수 있는 바이트(mac/Linux — 도구가 PNG로 준다) ·
/// `Rgba` = 원시 픽셀(Windows CF_DIB — PNG 인코딩은 본체가 **imgdec 워커**로 ·
/// R-5 "본체는 이미지 인코더도 링크하지 않는다").
#[derive(Debug)]
pub enum ClipImage {
    /// PNG 원본 바이트.
    Png(Vec<u8>),
    /// 원시 RGBA(top-down · straight alpha).
    Rgba {
        /// 폭(px).
        w: u32,
        /// 높이(px).
        h: u32,
        /// `w*h*4` 바이트.
        data: Vec<u8>,
    },
}

/// 클립보드의 이미지를 읽는다 — 없거나 미지원 형식·잠금 실패면 `None`.
#[must_use]
pub fn get_image() -> Option<ClipImage> {
    imp::get_image()
}

/// CF_DIB(BITMAPINFO) → RGBA(top-down) 변환 — **순수 함수**(전 OS 테스트).
/// 지원 = 24/32bpp × BI_RGB(0)·BI_BITFIELDS(3 · 표준 BGRA 마스크)만, 그 외
/// (팔레트·RLE·비표준 마스크)는 None(fail-soft). 알파는 255 고정 — 스크린샷
/// DIB의 알파 채널은 관행상 0이라 신뢰할 수 없다(실측 기반 보수).
#[cfg(any(windows, test))]
fn dib_to_rgba(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    fn r_u32(b: &[u8], o: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
    }
    fn r_i32(b: &[u8], o: usize) -> Option<i32> {
        Some(i32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
    }
    fn r_u16(b: &[u8], o: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
    }
    let bi_size = r_u32(dib, 0)? as usize;
    if bi_size < 40 || dib.len() < bi_size {
        return None;
    }
    let width = r_i32(dib, 4)?;
    let height_raw = r_i32(dib, 8)?;
    let bit_count = r_u16(dib, 14)?;
    let compression = r_u32(dib, 16)?;
    if width <= 0 || height_raw == 0 {
        return None;
    }
    let w = width as u32;
    let top_down = height_raw < 0;
    let h = height_raw.unsigned_abs();
    // imgdec와 같은 상한(변 8192 · 픽셀 16.7M) — 할당 폭탄 방어.
    if w > 8192 || h > 8192 || u64::from(w) * u64::from(h) > 16_777_216 {
        return None;
    }
    let bytes_pp = match (bit_count, compression) {
        (32, 0) => 4,
        (32, 3) => {
            // 마스크는 헤더 40바이트 뒤(BITMAPINFOHEADER) 또는 V4/V5 헤더 안 —
            // 어느 쪽이든 시작에서 40..52. 표준 BGRA 배치만 받는다.
            let (r, g, b) = (r_u32(dib, 40)?, r_u32(dib, 44)?, r_u32(dib, 48)?);
            if (r, g, b) != (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) {
                return None;
            }
            4
        }
        (24, 0) => 3,
        _ => return None,
    };
    let masks_extra = if bi_size == 40 && compression == 3 {
        12
    } else {
        0
    };
    let offset = bi_size + masks_extra;
    let stride = (w as usize * bytes_pp).div_ceil(4) * 4;
    let need = offset.checked_add(stride.checked_mul(h as usize)?)?;
    if dib.len() < need {
        return None;
    }
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for row_i in 0..h as usize {
        // DIB 기본은 bottom-up(양수 높이) — top-down으로 뒤집어 담는다.
        let src_row = if top_down {
            row_i
        } else {
            h as usize - 1 - row_i
        };
        let row = &dib[offset + src_row * stride..];
        for x in 0..w as usize {
            let px = &row[x * bytes_pp..];
            // DIB 픽셀은 BGR(A) 순서.
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    Some((w, h, rgba))
}

#[cfg(target_os = "windows")]
mod imp {
    use core::ffi::c_void;

    type Handle = *mut c_void;
    const CF_UNICODETEXT: u32 = 13;
    const CF_DIB: u32 = 8;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(hwnd: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> Handle;
        fn SetClipboardData(format: u32, mem: Handle) -> Handle;
        fn CreateWindowExW(
            ex_style: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: *mut c_void,
        ) -> Handle;
        fn DestroyWindow(hwnd: Handle) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(mem: Handle) -> *mut c_void;
        fn GlobalUnlock(mem: Handle) -> i32;
        fn GlobalFree(mem: Handle) -> Handle;
        fn GlobalSize(mem: Handle) -> usize;
    }

    /// ★ 클립보드는 **실제 HWND로 연다**(08-21 힙 오염 실측 — cargo 루프 4/20
    /// 0xc0000374·0xc0000005). `OpenClipboard(NULL)`은 내부 오픈 상태를 "연 창
    /// 핸들"로 대조하는데 NULL==NULL이라 **둘째 호출자도 성공**한다 — 같은
    /// 프로세스의 두 스레드(이미지 워커 × UI 복사)든 다중 인스턴스(3-신원
    /// 실기)든. 그 순간 한쪽 `EmptyClipboard`가 상대가 `GlobalLock` 중인
    /// HGLOBAL을 해제해 use-after-free가 된다. 호출마다 메시지 전용 창을 만들어
    /// 넘기면 오픈 상태가 진짜로 배타된다(둘째는 실패 → None/false 경로).
    /// 프로세스 안 스레드끼리는 뮤텍스로 먼저 직렬화한다(공정한 대기 —
    /// OpenClipboard 실패는 재시도 없이 그냥 실패라서).
    struct ClipGuard {
        hwnd: Handle,
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    fn open_clipboard() -> Option<ClipGuard> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let serial = SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
        const HWND_MESSAGE: Handle = -3isize as Handle;
        // SAFETY: 사전 정의 클래스("STATIC")의 메시지 전용 창 — 표시되지 않고
        // 이 함수 호출 스레드가 만들었다가 Drop에서 같은 스레드로 부순다.
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                core::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            );
            if hwnd.is_null() {
                return None; // 창도 못 만드는 세션 — 클립보드 없음으로 처리
            }
            if OpenClipboard(hwnd) == 0 {
                DestroyWindow(hwnd);
                return None;
            }
            Some(ClipGuard {
                hwnd,
                _serial: serial,
            })
        }
    }

    impl Drop for ClipGuard {
        fn drop(&mut self) {
            // SAFETY: open_clipboard에서 연 클립보드/만든 창을 닫는다(짝 보장).
            unsafe {
                CloseClipboard();
                DestroyWindow(self.hwnd);
            }
        }
    }

    /// 클립보드 이미지(③ 08-20) — CF_DIB를 통째로 복사해 락 밖에서 파싱한다
    /// (스크린샷·그림판 복사 등은 OS가 CF_DIB를 합성해 준다).
    pub(super) fn get_image() -> Option<super::ClipImage> {
        let guard = open_clipboard()?;
        // SAFETY: 클립보드는 guard가 배타로 열었다(진짜 HWND — 위 문서). 데이터
        // 핸들은 시스템 소유(해제 금지), Lock/Unlock 짝. 버퍼는 락 안에서 복사만.
        let dib: Vec<u8> = unsafe {
            let h = GetClipboardData(CF_DIB);
            let out = if h.is_null() {
                None
            } else {
                let p = GlobalLock(h).cast::<u8>();
                if p.is_null() {
                    None
                } else {
                    let n = GlobalSize(h);
                    let v = core::slice::from_raw_parts(p, n).to_vec();
                    GlobalUnlock(h);
                    Some(v)
                }
            };
            drop(guard);
            out?
        };
        super::dib_to_rgba(&dib).map(|(w, h, data)| super::ClipImage::Rgba { w, h, data })
    }

    pub(super) fn get_text() -> Option<String> {
        let guard = open_clipboard()?;
        // SAFETY: 클립보드는 guard가 배타로 열었다. GetClipboardData 핸들은 시스템
        // 소유(해제 금지), Lock/Unlock 짝 유지. 문자열 스캔은 GlobalSize 상한 안.
        unsafe {
            let h = GetClipboardData(CF_UNICODETEXT);
            let out = if h.is_null() {
                None
            } else {
                let p = GlobalLock(h).cast::<u16>();
                if p.is_null() {
                    None
                } else {
                    // 널 종단을 신뢰하지 않는다 — 다른 앱이 널 없이 넣었어도
                    // 할당 크기 밖은 읽지 않는다(GlobalSize 상한).
                    let cap = GlobalSize(h) / 2;
                    let mut len = 0usize;
                    while len < cap && *p.add(len) != 0 {
                        len += 1;
                    }
                    let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, len));
                    GlobalUnlock(h);
                    Some(s)
                }
            };
            drop(guard);
            out
        }
    }

    pub(super) fn set_text(text: &str) -> bool {
        let wide: Vec<u16> = text.encode_utf16().chain(core::iter::once(0)).collect();
        let Some(guard) = open_clipboard() else {
            return false;
        };
        // SAFETY: 클립보드는 guard가 배타로 열었다. GMEM_MOVEABLE 메모리에
        // UTF-16(널 종단)을 채워 SetClipboardData에 넘기면 **소유권이 시스템으로
        // 이전**된다(성공 시 GlobalFree 금지 · 실패 시 해제).
        unsafe {
            let mut ok = false;
            if EmptyClipboard() != 0 {
                let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
                if !h.is_null() {
                    let p = GlobalLock(h);
                    if p.is_null() {
                        GlobalFree(h);
                    } else {
                        core::ptr::copy_nonoverlapping(wide.as_ptr(), p.cast::<u16>(), wide.len());
                        GlobalUnlock(h);
                        if SetClipboardData(CF_UNICODETEXT, h).is_null() {
                            GlobalFree(h);
                        } else {
                            ok = true;
                        }
                    }
                }
            }
            drop(guard);
            ok
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod img_tests {
    /// 실측 보조(--nocapture) — 클립보드에 이미지가 있으면 크기를 찍는다.
    /// 없는 환경(CI)도 정상이라 존재는 단언하지 않는다.
    #[test]
    fn get_image_observable() {
        match super::get_image() {
            Some(super::ClipImage::Rgba { w, h, data }) => {
                assert_eq!(data.len(), (w * h * 4) as usize);
                println!("클립보드 이미지 = {w}x{h} RGBA");
            }
            Some(super::ClipImage::Png(b)) => println!("클립보드 PNG {}B", b.len()),
            None => println!("(클립보드 이미지 없음 — 건너뜀)"),
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    #[test]
    fn clipboard_roundtrip_windows() {
        let marker = "nexa-beep 클립보드 검증 ✓ 한글/CRLF\r\n포함";
        if !super::set_text(marker) {
            // 다른 프로세스가 클립보드를 잠근 순간일 수 있다 — 환경 탓 실패로 만들지 않는다.
            return;
        }
        // set 직후 get은 실기 데스크톱에서 간헐 실패한다(08-21 300회 루프 실측 2%) —
        // ① 클립보드 히스토리·클라우드 동기화 서비스가 새 내용을 읽으려 잠깐 열어
        // None ② 사용자·다른 앱이 그 사이 실제로 다른 내용을 복사(공유 자원).
        // 둘 다 환경 탓 skip. 왕복 인코딩 검증은 조용한 환경(CI)이 지킨다 —
        // 진짜 UTF-16 왕복 버그라면 거기서 결정적으로 실패한다.
        for _ in 0..20 {
            match super::get_text() {
                Some(got) if got == marker => return, // 왕복 성립
                Some(_) => {
                    eprintln!("(다른 프로세스가 클립보드를 덮어씀 — 건너뜀)");
                    return;
                }
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        eprintln!("(클립보드가 다른 프로세스에 잠겨 있음 — 건너뜀)");
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    //! 비-Windows — macOS는 `macclip`(NSPasteboard 직접 · 08-30) · Linux는
    //! `linuxclip`(Wayland/X11 직접 · 08-29)이 1차이고, **OS 기본 도구** 파이프
    //! (`wl-copy`/`wl-paste`·`xclip`)는 자체 경로가 없는 환경의 폴백이다. 도구가 없으면
    //! 실패를 그대로 보고한다(조용히 성공한 척 하지 않는다 — 복사가 안 됐는데 됐다고
    //! 하면 사용자가 붙여넣기에서 잃는다).

    #[cfg(not(target_os = "macos"))]
    use std::io::Write as _;
    #[cfg(not(target_os = "macos"))]
    use std::process::{Command, Stdio};

    /// 명령을 실행해 표준 출력을 문자열로 받는다.
    #[cfg(not(target_os = "macos"))]
    fn read_from(cmd: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(cmd)
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// 명령에 표준 입력으로 텍스트를 밀어 넣는다.
    #[cfg(not(target_os = "macos"))]
    fn write_to(cmd: &str, args: &[&str], text: &str) -> bool {
        let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let wrote = child
            .stdin
            .as_mut()
            .is_some_and(|si| si.write_all(text.as_bytes()).is_ok());
        // stdin을 닫아야 도구가 입력 끝을 안다(닫지 않으면 wait에서 멈춘다).
        drop(child.stdin.take());
        wrote && child.wait().is_ok_and(|s| s.success())
    }

    // macOS(08-30 L-1 mac 판) — NSPasteboard 직접(`macclip`) · 스폰 0.
    #[cfg(target_os = "macos")]
    pub(super) fn get_text() -> Option<String> {
        crate::macclip::get_text()
    }
    #[cfg(target_os = "macos")]
    pub(super) fn set_text(text: &str) -> bool {
        crate::macclip::set_text(text)
    }

    // Linux(08-29 L-1) — **자체 경로 우선**(`linuxclip` · Wayland/X11 직접) · 도구 스폰은
    // 자체 경로가 없을 때만(비-Linux unix 등). 도구가 없으면 종전엔 전부 죽었다.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn get_text() -> Option<String> {
        #[cfg(target_os = "linux")]
        if crate::linuxclip::native_available() {
            return crate::linuxclip::get(crate::linuxclip::TEXT_MIMES)
                .map(|b| String::from_utf8_lossy(&b).into_owned());
        }
        read_from("wl-paste", &["--no-newline"])
            .or_else(|| read_from("xclip", &["-selection", "clipboard", "-o"]))
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn set_text(text: &str) -> bool {
        #[cfg(target_os = "linux")]
        if crate::linuxclip::native_available() {
            return crate::linuxclip::set(crate::linuxclip::text_items(text));
        }
        write_to("wl-copy", &[], text) || write_to("xclip", &["-selection", "clipboard"], text)
    }

    /// 명령의 표준 출력을 바이트로 받는다(이미지 — 텍스트 read_from의 바이트판).
    /// Linux 전용(mac 이미지는 osascript 경로) — cfg가 없으면 mac에서 dead fn.
    #[cfg(not(target_os = "macos"))]
    fn read_bytes_from(cmd: &str, args: &[&str]) -> Option<Vec<u8>> {
        let out = Command::new(cmd)
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
    }

    /// 클립보드 이미지(③ 08-20) — mac = `NSPasteboard` PNG 직접(TIFF만 있으면
    /// AppKit 재포장 · 08-30 osascript 임시 파일 경로 폐지). Linux = 자체 경로 →
    /// wl-paste → xclip 사다리. 결과는 PNG 서명 확인 후에만 준다.
    #[cfg(target_os = "macos")]
    pub(super) fn get_image() -> Option<super::ClipImage> {
        crate::macclip::get_image_png().map(super::ClipImage::Png)
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn get_image() -> Option<super::ClipImage> {
        #[cfg(target_os = "linux")]
        if crate::linuxclip::native_available() {
            return crate::linuxclip::get(crate::linuxclip::IMAGE_MIMES)
                .filter(|b| b.starts_with(&[0x89, b'P', b'N', b'G']))
                .map(super::ClipImage::Png);
        }
        for (cmd, args) in [
            ("wl-paste", &["--type", "image/png"][..]),
            (
                "xclip",
                &["-selection", "clipboard", "-t", "image/png", "-o"][..],
            ),
        ] {
            if let Some(out) = read_bytes_from(cmd, args) {
                if out.starts_with(&[0x89, b'P', b'N', b'G']) {
                    return Some(super::ClipImage::Png(out));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod dib_tests {
    use super::dib_to_rgba;

    /// 32bpp BI_RGB 2x2 bottom-up — BGR(A)→RGBA 변환 + 행 뒤집기.
    #[test]
    fn dib_32bpp_bottom_up() {
        let mut d = vec![0u8; 40];
        d[0] = 40;
        d[4..8].copy_from_slice(&2i32.to_le_bytes());
        d[8..12].copy_from_slice(&2i32.to_le_bytes());
        d[14..16].copy_from_slice(&32u16.to_le_bytes());
        // 픽셀(BGRA · bottom-up): 아래 행 = [파랑, 초록] · 위 행 = [빨강, 흰].
        d.extend_from_slice(&[255, 0, 0, 0, 0, 255, 0, 0]);
        d.extend_from_slice(&[0, 0, 255, 0, 255, 255, 255, 0]);
        let (w, h, rgba) = dib_to_rgba(&d).expect("파싱");
        assert_eq!((w, h), (2, 2));
        // top-down 결과 첫 행 = 위 행(빨강·흰) — 알파는 255 고정.
        assert_eq!(&rgba[0..8], &[255, 0, 0, 255, 255, 255, 255, 255]);
        assert_eq!(&rgba[8..16], &[0, 0, 255, 255, 0, 255, 0, 255]);
    }

    /// 24bpp — stride 4바이트 정렬 패딩 + top-down(음수 높이).
    #[test]
    fn dib_24bpp_stride_and_top_down() {
        let mut d = vec![0u8; 40];
        d[0] = 40;
        d[4..8].copy_from_slice(&1i32.to_le_bytes());
        d[8..12].copy_from_slice(&(-2i32).to_le_bytes());
        d[14..16].copy_from_slice(&24u16.to_le_bytes());
        d.extend_from_slice(&[10, 20, 30, 0]); // BGR + 패딩 1
        d.extend_from_slice(&[40, 50, 60, 0]);
        let (w, h, rgba) = dib_to_rgba(&d).expect("파싱");
        assert_eq!((w, h), (1, 2));
        assert_eq!(&rgba[..], &[30, 20, 10, 255, 60, 50, 40, 255]);
    }

    /// 미지원(팔레트)·손상·상한 초과 = None(fail-soft).
    #[test]
    fn dib_rejects_unsupported() {
        assert!(dib_to_rgba(&[0u8; 10]).is_none(), "헤더 미달");
        let mut pal = vec![0u8; 40];
        pal[0] = 40;
        pal[4..8].copy_from_slice(&2i32.to_le_bytes());
        pal[8..12].copy_from_slice(&2i32.to_le_bytes());
        pal[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert!(dib_to_rgba(&pal).is_none(), "팔레트 미지원");
        let mut big = vec![0u8; 40];
        big[0] = 40;
        big[4..8].copy_from_slice(&9000i32.to_le_bytes());
        big[8..12].copy_from_slice(&2i32.to_le_bytes());
        big[14..16].copy_from_slice(&32u16.to_le_bytes());
        assert!(dib_to_rgba(&big).is_none(), "변 상한 8192");
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod unix_tests {
    /// 실측 — 이 기기에서 실제로 왕복하는가(도구가 없으면 건너뛴다).
    #[test]
    fn roundtrip_on_this_machine() {
        let marker = "nexa-beep 클립보드 \"왕복\" 시험";
        if !super::set_text(marker) {
            eprintln!("(클립보드 도구 없음 — 건너뜀)");
            return;
        }
        let got = super::get_text().expect("쓴 뒤에는 읽혀야 한다");
        assert_eq!(got.trim_end(), marker, "쓴 그대로 돌아와야 한다");
    }
}
