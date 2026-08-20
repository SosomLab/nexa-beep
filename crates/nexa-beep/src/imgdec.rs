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

/// 워커 스폰 공통 준비 — Windows는 **콘솔 창 없이**(CREATE_NO_WINDOW). 본체가
/// windows 서브시스템(08-20)이라 콘솔이 없는데, 그 상태에서 콘솔 서브시스템 자식을
/// 그냥 스폰하면 **디코드마다 콘솔 창이 번쩍인다**(파이프 stdio와는 별개 축).
fn worker_command(path: &std::path::Path) -> Command {
    let mut c = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

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
    let mut child = worker_command(&child_path)
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

/// 원본 사진 → **와이어용 축소 PNG**(08-16 · 프로필 사진 상한 대응).
///
/// 원본이 `PROFILE_IMAGE_MAX`(256KiB)를 넘으면 이 축소본(`me.wire.png`)이 대신
/// 실려 나간다 — 종전에는 초과 사진이 **조용히 생략**되고 내장 아바타 키가 광고돼
/// "본인은 사진, 상대는 옛 내장 그림"이 됐다. 인코딩도 imgdec 몫(본체는 이미지
/// 인코더도 링크하지 않는다 — R-5 결). 실패는 None — 호출측이 **소리 내어** 알린다.
pub(crate) fn wire_png_from_bytes(bytes: &[u8], max_side: u32) -> Option<Vec<u8>> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let child_path = dir.join(if cfg!(windows) {
        "nbeep-imgdec.exe"
    } else {
        "nbeep-imgdec"
    });
    if !child_path.exists() {
        return None;
    }
    let mut child = worker_command(&child_path)
        .args(["--max-side", &max_side.to_string(), "--encode-png"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(bytes).ok()?;
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
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    let status = child.wait().ok()?;
    // PNG 서명 + 와이어 상한 재확인 — 축소했는데도 초과면 만들 이유가 없던 것.
    (status.success()
        && out.starts_with(&[0x89, b'P', b'N', b'G'])
        && out.len() <= nbeep_core::PROFILE_IMAGE_MAX)
        .then_some(out)
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

/// 바이트 → 원형 아바타 **원시 픽셀**(w, h, 마스크 적용 RGBA).
///
/// ★ 워커 스레드용(08-13 실기 — 자식 프로세스 왕복(스폰+Defender 검사)이 메인을
/// 1~2초 멈췄다). 그리기 타입(`IconImage`·`Rc`)은 메인 소유라 **스레드 경계는
/// 원시 바이트로** 건너고, 감싸는 건 메인(`AppEvent::Decoded` 처리부)이 한다.
pub(crate) fn avatar_raw_from_bytes(bytes: &[u8], max_side: u32) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h, mut rgba) = decode_isolated(bytes, max_side)?;
    circle_mask(w, h, &mut rgba);
    Some((w, h, rgba))
}

/// `.beepq` 격리물 → 사각 썸네일 **원시 픽셀**(M4-5ⓑ — 격리함·대화 스레드 미리보기).
///
/// 원본을 **본체에서 해석하지 않는다** — `.beepq` 컨테이너에서 바이트를 꺼내
/// (선두 봉인 프리픽스 ‖ 본문 재조립) imgdec에 그대로 넘길 뿐, 픽셀은 격리
/// 프로세스가 만든다. 이미지가 아니거나 16MiB 초과·손상이면 None(미리보기 없음).
/// [`avatar_raw_from_bytes`]와 같은 이유로 워커에서 돌고 원시 픽셀로 돌아온다.
pub(crate) fn thumb_raw_from_beepq(
    path: &std::path::Path,
    max_side: u32,
    seal_secret: &[u8; 32],
) -> Option<(u32, u32, Vec<u8>)> {
    // 봉인 관문 경유(08-17 — 격리물 디스크 봉인 · 구본 관용은 관문 정책).
    let bytes = crate::gate::read_beepq_bytes(path, seal_secret)?;
    let q = nbeep_safe::Beepq::open(&bytes).ok()?;
    let total = q.sealed_prefix.len() + q.body.len();
    if total as u64 != q.original_size || total > 16 * 1024 * 1024 {
        return None; // 크기 불일치(손상) 또는 미리보기 상한 초과 — 조용히 없음
    }
    let mut original = Vec::with_capacity(total);
    original.extend_from_slice(&q.sealed_prefix);
    original.extend_from_slice(&q.body);
    decode_isolated(&original, max_side)
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
