//! `nbeep-imgdec` — 이미지 격리 디코드 전용 프로세스(M4-5 · FR-S-12 · R-5).
//!
//! 본체가 이 실행 파일을 자식으로 띄워 **신뢰 못 할 이미지**를 디코드시킨다.
//! 크래시해도 본체는 무사하고, 본체는 이미지 파서를 링크조차 하지 않는다.
//!
//! ## 프로토콜 (v1 — 파이프)
//!
//! - stdin: 이미지 바이트 전체(EOF까지 · **1MiB 상한** — 초과분은 자름 = 손상 취급).
//! - argv: `--max-side <px>`(기본 256) — 출력 긴 변 상한(박스 축소).
//! - stdout(성공): `"NIMG"` 4B ‖ w u32 LE ‖ h u32 LE ‖ RGBA(straight) `w*h*4`B.
//! - 실패: 아무 것도 쓰지 않고 비0 종료(사유는 stderr — 본체는 코드만 본다).
//!
//! ## 상한 (fail-closed)
//!
//! - 원본 ≤ 1MiB · 디코드 픽셀 ≤ 16,777,216(= 4096², RGBA 64MiB) · 변 ≤ 8192.
//! - 시간 상한은 **부모가 kill**로 강제한다(여기서 재는 시계는 신뢰 경계 밖).

use std::io::{Read as _, Write as _};

/// 원본 바이트 상한(프로필 이미지 256KiB 대비 여유 — 그 외 소비처도 1MiB면 충분).
const SRC_MAX: usize = 1024 * 1024;
/// 디코드 픽셀 상한(w*h) — RGBA 64MiB. 원본 1MiB JPEG은 12MP대가 흔해
/// 2048²(4MP)로는 폰 사진 대부분이 거부됐다(수신 미리보기 실기 08-13).
const PIXELS_MAX: u64 = 16_777_216;
/// 변 상한.
const SIDE_MAX: u32 = 8192;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let max_side: u32 = {
        let args: Vec<String> = std::env::args().collect();
        args.iter()
            .position(|a| a == "--max-side")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(256)
    };

    // 입력 — 상한 초과는 손상 취급(더 읽지 않고 실패).
    let mut src = Vec::with_capacity(64 * 1024);
    let n = std::io::stdin()
        .lock()
        .take(SRC_MAX as u64 + 1)
        .read_to_end(&mut src);
    if n.is_err() || src.is_empty() || src.len() > SRC_MAX {
        eprintln!("imgdec: 입력 없음/상한 초과");
        return 2;
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
    let mut out = std::io::stdout().lock();
    let ok = out.write_all(b"NIMG").is_ok()
        && out.write_all(&w.to_le_bytes()).is_ok()
        && out.write_all(&h.to_le_bytes()).is_ok()
        && out.write_all(&rgba).is_ok();
    i32::from(!ok)
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
        assert!(!size_ok(0, 100)); // 퇴화
    }
}
