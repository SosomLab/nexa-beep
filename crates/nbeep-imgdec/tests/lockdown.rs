//! M4-5ⓒ 통합 실측 — **강등이 적용된 실제 바이너리**로 디코드가 되는지(3-OS CI).
//!
//! 단위 테스트가 아니라 빌드된 `nbeep-imgdec`를 실제로 스폰한다(`CARGO_BIN_EXE_`).
//! 이 테스트가 green이면 "강등 후에도 정상 경로가 산다"가 그 OS에서 실측된 것이고,
//! red면 허용 목록이 부족한 것이다(추정 금지 — CI가 3-OS에서 돌려 준다).
#![allow(clippy::unwrap_used)] // 테스트 코드 — docs/13 §9

use std::io::Write as _;
use std::process::{Command, Stdio};

/// 4×4 불투명 PNG 표본(dev-dependency `png`로 즉석 인코딩).
fn sample_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 4, 4);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        let px: Vec<u8> = (0..4u8 * 4)
            .flat_map(|i| [i * 10, 255 - i * 10, 128, 255])
            .collect();
        w.write_image_data(&px).unwrap();
    }
    out
}

fn run_imgdec(input: &[u8]) -> (std::process::ExitStatus, Vec<u8>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nbeep-imgdec"))
        .arg("--max-side")
        .arg("64")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("imgdec 스폰");
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().expect("imgdec 종료 대기");
    (
        out.status,
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// ★ 강등 상태에서 정상 PNG 디코드가 산다 — lockdown이 적용됐음을 stderr로 확인.
#[test]
fn locked_down_decode_still_works() {
    let (status, stdout, stderr) = run_imgdec(&sample_png());
    assert!(
        stderr.contains("lockdown = "),
        "강등이 적용 보고돼야 한다: {stderr}"
    );
    assert!(status.success(), "정상 디코드 exit 0: {stderr}");
    assert_eq!(&stdout[..4], b"NIMG", "출력 머리");
    let w = u32::from_le_bytes(stdout[4..8].try_into().unwrap());
    let h = u32::from_le_bytes(stdout[8..12].try_into().unwrap());
    assert_eq!((w, h), (4, 4));
    assert_eq!(stdout.len(), 12 + (w * h * 4) as usize, "RGBA 길이 정합");
}

/// 손상 입력 = 비0 종료 + stdout 무출력(fail-closed 그대로 — 강등이 실패 경로를
/// 바꾸지 않는다).
#[test]
fn garbage_input_fails_closed_under_lockdown() {
    let (status, stdout, _stderr) = run_imgdec(b"not an image at all");
    assert!(!status.success());
    assert!(stdout.is_empty(), "실패 시 stdout 무출력");
}
