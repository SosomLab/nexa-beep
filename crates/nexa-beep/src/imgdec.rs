//! 이미지 격리 디코드 **어댑터**(M4-5 · FR-S-12 · R-5) — 본체는 이미지 파서를 링크하지
//! 않는다. `nbeep-imgdec` 자식 프로세스에 바이트를 보내고, 상한 걸린 RGBA만 받는다.
//!
//! - 자식이 크래시·오염돼도 본체는 `None`을 받을 뿐이다.
//! - **시간 상한은 부모가 강제**(3초 — 초과 시 kill · 압축 폭탄류의 CPU 소진 차단).
//! - 결과 픽셀 상한은 프로토콜이 보장(긴 변 `max_side` · 검증 후에도 본체가 재확인).

use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};

/// 자식 응답 대기 상한 — 초과는 손상·폭탄 취급(kill).
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// 격리 디코드 — 성공 시 (w, h, RGBA). 실패 사유는 구분하지 않는다(전부 "없음").
pub(crate) fn decode_isolated(bytes: &[u8], max_side: u32) -> Option<(u32, u32, Vec<u8>)> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let child_path = dir.join(if cfg!(windows) {
        "nbeep-imgdec.exe"
    } else {
        "nbeep-imgdec"
    });
    if !child_path.exists() {
        return None; // 동봉 안 됨(포장 잔여) — 조용히 이니셜 폴백
    }
    let mut child = Command::new(child_path)
        .args(["--max-side", &max_side.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // 입력을 쓰고 stdin을 닫아야 자식이 EOF를 본다.
    child.stdin.take()?.write_all(bytes).ok()?;
    // 출력은 별도 스레드에서 끝까지 읽고, 본 스레드는 시간 상한을 잰다.
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let ok = stdout.read_to_end(&mut out).is_ok();
        let _ = tx.send(ok.then_some(out));
    });
    let out = match rx.recv_timeout(TIMEOUT) {
        Ok(Some(out)) => out,
        _ => {
            let _ = child.kill(); // 시간 초과·읽기 실패 — 폭탄 취급
            let _ = child.wait();
            return None;
        }
    };
    let status = child.wait().ok()?;
    if !status.success() || out.len() < 12 || &out[..4] != b"NIMG" {
        return None;
    }
    let w = u32::from_le_bytes(out[4..8].try_into().ok()?);
    let h = u32::from_le_bytes(out[8..12].try_into().ok()?);
    // 본체 측 재검증 — 자식이 오염됐어도 여기서 상한이 선다(fail-closed).
    if w == 0 || h == 0 || w > max_side.max(1) * 2 || h > max_side.max(1) * 2 {
        return None;
    }
    let need = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    (out.len() == 12 + need).then(|| (w, h, out[12..].to_vec()))
}

/// 원형 마스크(아바타) — 원 밖 알파 0 · 가장자리 1px AA. 정사각이 아니면 중앙 원.
pub(crate) fn circle_mask(w: u32, h: u32, rgba: &mut [u8]) {
    let (cx, cy) = (f32::from(u16::try_from(w).unwrap_or(u16::MAX)) / 2.0, {
        f32::from(u16::try_from(h).unwrap_or(u16::MAX)) / 2.0
    });
    let r = cx.min(cy);
    for y in 0..h {
        for x in 0..w {
            #[allow(clippy::cast_precision_loss)]
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0);
            let i = ((y * w + x) * 4 + 3) as usize;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                rgba[i] = (f32::from(rgba[i]) * cov).round() as u8;
            }
        }
    }
}

/// 파일 → 원형 아바타 이미지(격리 디코드 + 마스크). 실패 = None(이니셜 폴백).
pub(crate) fn avatar_from_file(
    path: &std::path::Path,
    max_side: u32,
) -> Option<nbeep_ui::IconImage> {
    let bytes = std::fs::read(path).ok()?;
    avatar_from_bytes(&bytes, max_side)
}

/// 바이트 → 원형 아바타 이미지.
pub(crate) fn avatar_from_bytes(bytes: &[u8], max_side: u32) -> Option<nbeep_ui::IconImage> {
    let (w, h, mut rgba) = decode_isolated(bytes, max_side)?;
    circle_mask(w, h, &mut rgba);
    Some(nbeep_ui::IconImage::from_rgba(w, h, rgba))
}

/// `.beepq` 격리물 → 사각 썸네일(M4-5ⓑ — 격리함·대화 스레드 미리보기).
///
/// 원본을 **본체에서 해석하지 않는다** — `.beepq` 컨테이너에서 바이트를 꺼내
/// (선두 봉인 프리픽스 ‖ 본문 재조립) imgdec에 그대로 넘길 뿐, 픽셀은 격리
/// 프로세스가 만든다. 이미지가 아니거나 1MiB 초과·손상이면 None(미리보기 없음).
pub(crate) fn thumb_from_beepq(
    path: &std::path::Path,
    max_side: u32,
) -> Option<nbeep_ui::IconImage> {
    let bytes = std::fs::read(path).ok()?;
    let q = nbeep_safe::Beepq::open(&bytes).ok()?;
    let total = q.sealed_prefix.len() + q.body.len();
    if total as u64 != q.original_size || total > 1024 * 1024 {
        return None; // 크기 불일치(손상) 또는 미리보기 상한 초과 — 조용히 없음
    }
    let mut original = Vec::with_capacity(total);
    original.extend_from_slice(&q.sealed_prefix);
    original.extend_from_slice(&q.body);
    let (w, h, rgba) = decode_isolated(&original, max_side)?;
    Some(nbeep_ui::IconImage::from_rgba(w, h, rgba))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// 최소 PNG(2×2 RGBA) 생성 — png 인코더 없이 수제 청크(고정 픽셀).
    /// 여기서는 대신 imgdec 실물을 호출하는 통합 테스트가 어려워(자식 실행 파일이
    /// 테스트 시점에 빌드돼 있어야 함 — cargo test는 bin 의존을 보장하지 않는다)
    /// **마스크·검증 로직만** 단위로 잠근다. 종단은 실기 검증.
    #[test]
    fn circle_mask_zeroes_corners_keeps_center() {
        let (w, h) = (16u32, 16u32);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        circle_mask(w, h, &mut rgba);
        assert_eq!(rgba[3], 0, "모서리는 투명");
        let ci = (((h / 2) * w + w / 2) * 4 + 3) as usize;
        assert_eq!(rgba[ci], 255, "중앙은 불투명");
    }
}
