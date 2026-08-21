//! `nbeep-imgdec` — 이미지 격리 디코드 전용 프로세스(M4-5 · FR-S-12 · R-5).
//!
//! 본체가 이 실행 파일을 자식으로 띄워 **신뢰 못 할 이미지**를 디코드시킨다.
//! 크래시해도 본체는 무사하고, 본체는 이미지 파서를 링크조차 하지 않는다.
//!
//! ## 프로토콜 (v1 — 파이프)
//!
//! - stdin: 이미지 바이트 전체(EOF까지 · **16MiB 상한** — 초과분은 자름 = 손상 취급).
//! - argv: `--max-side <px>`(기본 256) — 출력 긴 변 상한(박스 축소).
//! - stdout(성공): `"NIMG"` 4B ‖ w u32 LE ‖ h u32 LE ‖ RGBA(straight) `w*h*4`B.
//! - 실패: 아무 것도 쓰지 않고 비0 종료(사유는 stderr — 본체는 코드만 본다).
//!
//! ## 권한 강등 (M4-5ⓒ · R-5 종결)
//!
//! 입력(stdin)을 **전부 읽은 직후 · 파싱 전에** 자신을 잠근다 — 이후 허용되는
//! 일은 메모리 할당·stdout/stderr 쓰기·종료뿐이다. 파서가 뚫려도(RCE) 열 수
//! 있는 것이 없다: macOS = Seatbelt `pure-computation` · Linux = seccomp-bpf
//! 허용 목록(+no_new_privs) · Windows = 완화 정책(동적 코드·원격 이미지 금지 —
//! win32k 락아웃은 실기 검증 후 강화). 강등 실패 = **fail-closed**(디코드 포기 ·
//! exit 4). 3-OS 실측은 통합 테스트(`tests/lockdown.rs`)가 CI에서 실행한다.
//!
//! ## 상한 (fail-closed)
//!
//! - 원본 ≤ 16MiB · 디코드 픽셀 ≤ 16,777,216(= 4096², RGBA 64MiB) · 변 ≤ 8192.
//! - 시간 상한은 **부모가 kill**로 강제한다(여기서 재는 시계는 신뢰 경계 밖).

use std::io::{Read as _, Write as _};

/// 원본 바이트 상한(08-16 16MiB 상향 — 폰 사진·고해상도가 1MiB 초과가 보통이라
/// 수신 미리보기가 자주 침묵했다. 할당 폭탄은 픽셀 상한이, CPU 폭탄은 부모 3초
/// kill이 막으므로 바이트 상한 상향은 방어선을 옮기지 않는다).
const SRC_MAX: usize = 16 * 1024 * 1024;
/// 디코드 픽셀 상한(w*h) — RGBA 64MiB. 원본 1MiB JPEG은 12MP대가 흔해
/// 2048²(4MP)로는 폰 사진 대부분이 거부됐다(수신 미리보기 실기 08-13).
const PIXELS_MAX: u64 = 16_777_216;
/// 변 상한.
const SIDE_MAX: u32 = 8192;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let max_side: u32 = args
        .iter()
        .position(|a| a == "--max-side")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    // 출력 = PNG 재인코딩(08-16 · 프로필 와이어 축소본): 원본이 아무리 커도 축소
    // 결과를 PNG로 되뱉는다 — 본체는 인코더도 링크하지 않는다(png는 여기만).
    let encode_png = args.iter().any(|a| a == "--encode-png");
    // 원시 RGBA → PNG(③ 08-20 클립보드 이미지): 입력 = `w u32 LE ‖ h u32 LE ‖ RGBA`.
    // 파서가 아예 안 돌므로(신뢰 데이터 = 본체가 방금 만든 픽셀) 디코드 상한과
    // 무관하지만, 할당 상한(픽셀 16.7M = RGBA 64MiB)은 동일하게 지킨다.
    let encode_raw = args.iter().any(|a| a == "--encode-raw");

    // 입력 — 상한 초과는 손상 취급(더 읽지 않고 실패). 16MiB는 디코드·인코드
    // 공통(08-16 상향 — 종전 디코드 1MiB는 폰 사진에서 미리보기를 침묵시켰다).
    // 원시 모드는 픽셀 상한만큼 크다(4K 스크린샷 RGBA ≈ 33MiB).
    let src_cap = if encode_raw {
        PIXELS_MAX as usize * 4 + 8
    } else {
        SRC_MAX
    };
    let mut src = Vec::with_capacity(64 * 1024);
    let n = std::io::stdin()
        .lock()
        .take(src_cap as u64 + 1)
        .read_to_end(&mut src);
    if n.is_err() || src.is_empty() || src.len() > src_cap {
        eprintln!("imgdec: 입력 없음/상한 초과");
        return 2;
    }

    // ★ 권한 강등(M4-5ⓒ · R-5) — 입력을 전부 읽었으니 **파싱 전에** 잠근다.
    //   실패 = fail-closed: 파서를 무방비로 돌리지 않는다(이미지만 죽는다 —
    //   imgdec 부재와 같은 강도의 실패 · 본체는 이니셜 폴백).
    match lockdown::engage() {
        Ok(mode) => eprintln!("imgdec: lockdown = {mode}"),
        Err(why) => {
            eprintln!("imgdec: lockdown 실패({why}) — fail-closed");
            return 4;
        }
    }

    if encode_raw {
        // 헤더 + 정확한 길이 검증(fail-closed) 후 전체 크기 그대로 PNG.
        if src.len() < 8 {
            return 2;
        }
        let w = u32::from_le_bytes(src[0..4].try_into().unwrap_or([0; 4]));
        let h = u32::from_le_bytes(src[4..8].try_into().unwrap_or([0; 4]));
        if !size_ok(w, h) || src.len() != 8 + (w as usize) * (h as usize) * 4 {
            eprintln!("imgdec: raw 크기 불일치");
            return 2;
        }
        let Some(buf) = encode_png_rgba(w, h, &src[8..]) else {
            return 3;
        };
        let mut out = std::io::stdout().lock();
        return if out.write_all(&buf).is_ok() && out.flush().is_ok() {
            0
        } else {
            3
        };
    }

    let decoded = if src.starts_with(&[0x89, b'P', b'N', b'G']) {
        decode_png(&src)
    } else if src.starts_with(&[0xFF, 0xD8]) {
        decode_jpeg(&src)
    } else {
        eprintln!("imgdec: 미지 형식(PNG/JPEG만)");
        return 2;
    };
    let Some((w, h, rgba)) = decoded else {
        return 3; // 손상·상한 초과 — 파서가 뭐라 했든 결과는 "없음"뿐
    };

    let (w, h, rgba) = downscale_box(w, h, &rgba, max_side);
    if encode_png {
        let Some(buf) = encode_png_rgba(w, h, &rgba) else {
            return 3;
        };
        let mut out = std::io::stdout().lock();
        return if out.write_all(&buf).is_ok() && out.flush().is_ok() {
            0
        } else {
            3
        };
    }
    let mut out = std::io::stdout().lock();
    let ok = out.write_all(b"NIMG").is_ok()
        && out.write_all(&w.to_le_bytes()).is_ok()
        && out.write_all(&h.to_le_bytes()).is_ok()
        && out.write_all(&rgba).is_ok();
    i32::from(!ok)
}

/// 축소 픽셀 → PNG 인코딩(`--encode-png` — 프로필 와이어 축소본 · 08-16).
/// 인코딩 입력은 방금 우리가 만든 픽셀(신뢰 데이터)이라 잠금 안에서 안전하다.
fn encode_png_rgba(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(rgba).ok()?;
    }
    Some(buf)
}

/// 픽셀 상한 검사 — 파서가 크기를 주장하는 시점(할당 전)에 자른다.
fn size_ok(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= SIDE_MAX && h <= SIDE_MAX && u64::from(w) * u64::from(h) <= PIXELS_MAX
}

fn decode_png(src: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(src);
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if !size_ok(info.width, info.height) {
        return None;
    }
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let out = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (out.width, out.height);
    let line = out.line_size;
    let data = &buf[..out.buffer_size()];
    // RGBA로 정규화(그레이/팔레트는 png가 확장하도록 요구할 수도 있지만, 여기선
    // 색 타입별 최소 변환 — 미지 조합은 실패로).
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    match out.color_type {
        png::ColorType::Rgba => {
            for row in data.chunks(line) {
                rgba.extend_from_slice(&row[..(w * 4) as usize]);
            }
        }
        png::ColorType::Rgb => {
            for row in data.chunks(line) {
                for px in row[..(w * 3) as usize].chunks(3) {
                    rgba.extend_from_slice(px);
                    rgba.push(255);
                }
            }
        }
        png::ColorType::Grayscale => {
            for row in data.chunks(line) {
                for &g in &row[..w as usize] {
                    rgba.extend_from_slice(&[g, g, g, 255]);
                }
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for row in data.chunks(line) {
                for px in row[..(w * 2) as usize].chunks(2) {
                    rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
                }
            }
        }
        png::ColorType::Indexed => return None, // read_info 기본은 팔레트 확장 아님 — 거부
    }
    (rgba.len() == (w * h * 4) as usize).then_some((w, h, rgba))
}

fn decode_jpeg(src: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = jpeg_decoder::Decoder::new(src);
    dec.read_info().ok()?;
    let info = dec.info()?;
    if !size_ok(u32::from(info.width), u32::from(info.height)) {
        return None;
    }
    let pixels = dec.decode().ok()?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            for px in pixels.chunks(3) {
                rgba.extend_from_slice(px);
                rgba.push(255);
            }
        }
        jpeg_decoder::PixelFormat::L8 => {
            for &g in &pixels {
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        _ => return None, // CMYK·L16 등 — 프로필 사진 범위 밖(fail-closed)
    }
    (rgba.len() == (w * h * 4) as usize).then_some((w, h, rgba))
}

/// 박스 평균 축소 — 긴 변을 `max_side` 이하로. 확대는 하지 않는다(원본 유지).
fn downscale_box(w: u32, h: u32, rgba: &[u8], max_side: u32) -> (u32, u32, Vec<u8>) {
    let side = w.max(h);
    if side <= max_side || max_side == 0 {
        return (w, h, rgba.to_vec());
    }
    let scale = f64::from(max_side) / f64::from(side);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (nw, nh) = (
        ((f64::from(w) * scale).round() as u32).max(1),
        ((f64::from(h) * scale).round() as u32).max(1),
    );
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for ny in 0..nh {
        // 원본에서 이 출력 픽셀이 덮는 y 구간.
        let y0 = (u64::from(ny) * u64::from(h) / u64::from(nh)) as u32;
        let y1 = (((u64::from(ny) + 1) * u64::from(h)).div_ceil(u64::from(nh)) as u32).min(h);
        for nx in 0..nw {
            let x0 = (u64::from(nx) * u64::from(w) / u64::from(nw)) as u32;
            let x1 = (((u64::from(nx) + 1) * u64::from(w)).div_ceil(u64::from(nw)) as u32).min(w);
            let (mut r, mut g, mut b, mut a, mut cnt) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    r += u64::from(rgba[i]);
                    g += u64::from(rgba[i + 1]);
                    b += u64::from(rgba[i + 2]);
                    a += u64::from(rgba[i + 3]);
                    cnt += 1;
                }
            }
            let cnt = cnt.max(1);
            #[allow(clippy::cast_possible_truncation)]
            out.extend_from_slice(&[
                (r / cnt) as u8,
                (g / cnt) as u8,
                (b / cnt) as u8,
                (a / cnt) as u8,
            ]);
        }
    }
    (nw, nh, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실기(08-13)에서 거부됐던 폰 사진 해상도들이 상한 안에 들어야 한다.
    #[test]
    fn phone_photos_within_pixel_cap() {
        assert!(size_ok(2198, 2198)); // 483만 px — 구 상한(2048²)에서 거부됐던 실물
        assert!(size_ok(4000, 3000)); // 12MP — 1MiB JPEG로 흔한 크기
        assert!(size_ok(4096, 4096)); // 정확히 상한
    }

    #[test]
    fn oversized_still_rejected() {
        assert!(!size_ok(4097, 4096)); // 픽셀 상한 초과
        assert!(!size_ok(8193, 1)); // 변 상한 초과
    }

    /// 인코드 왕복(08-16 · 와이어 축소본) — 구운 PNG를 우리 디코더가 도로 읽어
    /// 같은 크기·같은 픽셀이 나와야 한다. 축소본이 256KiB 상한 안인 것도 함께
    /// (256px 사진 축소본이 상한을 넘으면 파이프라인 전체가 무의미하다).
    #[test]
    fn encode_png_roundtrips_through_our_decoder() {
        let (w, h) = (256u32, 256u32);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let png = encode_png_rgba(w, h, &rgba).expect("인코드");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(
            png.len() <= 256 * 1024,
            "축소본이 와이어 상한 초과: {}",
            png.len()
        );
        let (dw, dh, back) = decode_png(&png).expect("왕복 디코드");
        assert_eq!((dw, dh), (w, h));
        assert_eq!(back, rgba, "픽셀 비트 동일");
        assert!(!size_ok(0, 100)); // 퇴화
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// M4-5ⓒ 권한 강등(R-5 종결) — **입력을 전부 읽은 뒤 · 파싱 전에** 잠근다.
// 이후 이 프로세스에 허용되는 일은 "메모리 할당 · stdout/stderr 쓰기 · 종료"뿐.
// 강등 실패 = fail-closed(디코드 포기 · 비0 종료) — 파서를 무방비로 돌리지 않는다.
// 실측 경로: 통합 테스트(tests/lockdown.rs)가 3-OS CI에서 실제 바이너리를 돌린다.
mod lockdown {
    // 바이너리 내부 모듈 — pub이 밖에 닿지 않는다(unreachable_pub 의도 문서용).
    #![allow(unreachable_pub)]

    /// 강등 적용 — `Ok(적용 방식)` / `Err(사유)`.
    #[cfg(target_os = "macos")]
    pub fn engage() -> Result<&'static str, String> {
        // Seatbelt "pure-computation" 프로파일 — 파일/네트워크/exec 전부 차단,
        // **이미 열린 fd의 read/write는 허용**(stdin은 이미 다 읽었고 stdout만 쓴다).
        // sandbox_init는 deprecated 표기지만 시스템 헬퍼들이 여전히 쓰는 경로다.
        unsafe extern "C" {
            fn sandbox_init(profile: *const u8, flags: u64, errorbuf: *mut *mut u8) -> i32;
            fn sandbox_free_error(errorbuf: *mut u8);
        }
        const SANDBOX_NAMED: u64 = 0x0001;
        let mut err: *mut u8 = std::ptr::null_mut();
        let rc =
            unsafe { sandbox_init(c"pure-computation".as_ptr().cast(), SANDBOX_NAMED, &mut err) };
        if rc == 0 {
            return Ok("macos-seatbelt(pure-computation)");
        }
        let why = if err.is_null() {
            "sandbox_init 실패".to_string()
        } else {
            let s = unsafe { std::ffi::CStr::from_ptr(err.cast()) }
                .to_string_lossy()
                .into_owned();
            unsafe { sandbox_free_error(err) };
            s
        };
        Err(why)
    }

    /// Linux — seccomp-bpf 허용 목록(할당·쓰기·종료 계열만) + no_new_privs.
    /// 번호는 libc의 아키텍처별 상수를 쓴다(손 번호 금지 — x86_64/aarch64 분기 오류원).
    #[cfg(target_os = "linux")]
    pub fn engage() -> Result<&'static str, String> {
        #[repr(C)]
        struct SockFilter {
            code: u16,
            jt: u8,
            jf: u8,
            k: u32,
        }
        #[repr(C)]
        struct SockFprog {
            len: u16,
            filter: *const SockFilter,
        }
        const BPF_LD_W_ABS: u16 = 0x20;
        const BPF_JMP_JEQ_K: u16 = 0x15;
        const BPF_RET_K: u16 = 0x06;
        const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
        const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
        // seccomp_data 오프셋: nr=0 · arch=4.
        const OFF_NR: u32 = 0;
        const OFF_ARCH: u32 = 4;
        #[cfg(target_arch = "x86_64")]
        const AUDIT_ARCH: u32 = 0xC000_003E; // AUDIT_ARCH_X86_64
        #[cfg(target_arch = "aarch64")]
        const AUDIT_ARCH: u32 = 0xC000_00B7; // AUDIT_ARCH_AARCH64

        // 허용 syscall — 할당(brk/mmap 계열)·쓰기·동기화·종료·시그널 복귀.
        // open/exec/socket 계열은 목록에 없다 = 즉사(KILL_PROCESS).
        #[allow(clippy::cast_possible_truncation)]
        let allow: Vec<u32> = vec![
            libc::SYS_read as u32,
            libc::SYS_write as u32,
            libc::SYS_writev as u32,
            libc::SYS_brk as u32,
            libc::SYS_mmap as u32,
            libc::SYS_munmap as u32,
            libc::SYS_mremap as u32,
            libc::SYS_mprotect as u32,
            libc::SYS_madvise as u32,
            libc::SYS_futex as u32,
            libc::SYS_exit as u32,
            libc::SYS_exit_group as u32,
            libc::SYS_rt_sigreturn as u32,
            libc::SYS_rt_sigprocmask as u32,
            libc::SYS_sigaltstack as u32,
            libc::SYS_getrandom as u32,
            libc::SYS_clock_gettime as u32,
            libc::SYS_tgkill as u32, // abort 경로(panic=abort·assert)
            libc::SYS_sched_yield as u32,
        ];
        let mut prog: Vec<SockFilter> = Vec::with_capacity(allow.len() + 4);
        let ld = |k: u32| SockFilter {
            code: BPF_LD_W_ABS,
            jt: 0,
            jf: 0,
            k,
        };
        // arch 검증(다른 ABI로의 우회 차단) → nr 비교 사다리 → 기본 즉사.
        prog.push(ld(OFF_ARCH));
        prog.push(SockFilter {
            code: BPF_JMP_JEQ_K,
            jt: 1,
            jf: 0,
            k: AUDIT_ARCH,
        });
        prog.push(SockFilter {
            code: BPF_RET_K,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_KILL_PROCESS,
        });
        prog.push(ld(OFF_NR));
        for nr in &allow {
            prog.push(SockFilter {
                code: BPF_JMP_JEQ_K,
                jt: 0,
                jf: 1,
                k: *nr,
            });
            prog.push(SockFilter {
                code: BPF_RET_K,
                jt: 0,
                jf: 0,
                k: SECCOMP_RET_ALLOW,
            });
        }
        prog.push(SockFilter {
            code: BPF_RET_K,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_KILL_PROCESS,
        });
        let fprog = SockFprog {
            len: u16::try_from(prog.len()).map_err(|_| "필터 과대".to_string())?,
            filter: prog.as_ptr(),
        };
        // no_new_privs — seccomp 무권한 설치의 전제.
        let rc = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if rc != 0 {
            return Err("PR_SET_NO_NEW_PRIVS 실패".into());
        }
        let rc = unsafe {
            libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                std::ptr::addr_of!(fprog),
                0,
                0,
            )
        };
        if rc != 0 {
            return Err("seccomp 필터 설치 실패".into());
        }
        Ok("linux-seccomp-bpf(allowlist)")
    }

    /// Windows — 프로세스 자체 완화 정책(동적 코드 금지 + 원격 이미지 로드 금지).
    /// win32k 락아웃은 실기 검증 후 강화([TODO WGUI]) — 콘솔 경로 호환을 먼저 실측.
    #[cfg(target_os = "windows")]
    pub fn engage() -> Result<&'static str, String> {
        #[repr(C)]
        #[derive(Default)]
        struct DynamicCodePolicy {
            flags: u32, // bit0 = ProhibitDynamicCode
        }
        #[repr(C)]
        #[derive(Default)]
        struct ImageLoadPolicy {
            flags: u32, // bit0 = NoRemoteImages · bit1 = NoLowMandatoryLabelImages
        }
        #[repr(C)]
        #[derive(Default)]
        struct SystemCallDisablePolicy {
            flags: u32, // bit0 = DisallowWin32kSystemCalls(win32k 락아웃)
        }
        // ProcessDynamicCodePolicy = 2 · ProcessSystemCallDisablePolicy = 4 ·
        // ProcessImageLoadPolicy = 10.
        unsafe extern "system" {
            fn SetProcessMitigationPolicy(
                policy: i32,
                buf: *const core::ffi::c_void,
                len: usize,
            ) -> i32;
        }
        let dc = DynamicCodePolicy { flags: 1 };
        let il = ImageLoadPolicy { flags: 0b11 };
        let ok1 = unsafe {
            SetProcessMitigationPolicy(
                2,
                std::ptr::addr_of!(dc).cast(),
                core::mem::size_of::<DynamicCodePolicy>(),
            )
        };
        let ok2 = unsafe {
            SetProcessMitigationPolicy(
                10,
                std::ptr::addr_of!(il).cast(),
                core::mem::size_of::<ImageLoadPolicy>(),
            )
        };
        if ok1 == 0 && ok2 == 0 {
            return Err("SetProcessMitigationPolicy 실패".into());
        }
        // ★ win32k 락아웃(08-22 강화 — "실기 검증 후 강화" 예고분): 이 프로세스는
        //   콘솔 I/O(condrv)와 순수 연산뿐이라 win32k 시스템 콜이 필요 없다.
        //   파서가 뚫려도 GUI 계열 커널 표면(폰트·GDI 취약점 역사)이 통째로 닫힌다.
        //   콘솔 창 없는 부모(CREATE_NO_WINDOW 스폰)에서도 성립. 베스트 에포트 —
        //   실패해도 기본 2종은 이미 섰다(모드 문자열로 실측 가시화).
        let sc = SystemCallDisablePolicy { flags: 1 };
        let ok3 = unsafe {
            SetProcessMitigationPolicy(
                4,
                std::ptr::addr_of!(sc).cast(),
                core::mem::size_of::<SystemCallDisablePolicy>(),
            )
        };
        Ok(if ok3 != 0 {
            "windows-mitigation(dynamic-code·image-load·win32k-lockout)"
        } else {
            "windows-mitigation(dynamic-code·image-load)"
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    pub fn engage() -> Result<&'static str, String> {
        Err("미지원 OS — 강등 수단 없음".into())
    }
}
