//! `mkavatars` — 원본 PNG를 **내장 아바타 자산**(`NBAV1`)으로 굽는다.
//!
//! ```text
//! cargo run -p mkavatars -- assets/avatars crates/nbeep-ui/assets/avatars.nbav
//! ```
//!
//! ## 왜 런타임이 아니라 여기서 하나
//!
//! 여백 잘라내기·크기 정규화는 **한 번만 하면 되는 일**이다. 런타임에 두면 매 실행
//! 같은 계산을 반복하고, 본체가 PNG 파서를 링크해야 해서 R-5가 깨진다. 그래서
//! **굽는 쪽에서 끝내고 런타임은 이미 정규화된 정사각만 본다**.
//!
//! ## 정규화 규칙 (사용자 요청 — "여백 제외하고 비슷한 크기로")
//!
//! 1. **알파 바운딩 박스**로 자른다 — 원본마다 다른 투명 여백을 없앤다.
//! 2. 잘린 그림을 **가로세로 비율을 유지**한 채 `SIDE * CONTENT` 안에 들어가게 축소한다.
//! 3. `SIDE×SIDE` 투명 캔버스 **정중앙**에 놓는다.
//!
//! ⇒ 원본이 꽉 찬 그림이든 여백투성이든 **화면에서 같은 크기로 보인다**. 이게 없으면
//! 목록에서 어떤 띠는 크고 어떤 띠는 작게 보인다(요청의 핵심).

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// 자산 한 변(px) **기본값** — 3번째 인자로 바꿀 수 있다(크기 실험용).
/// 표시 자리: 목록 40 · 프로필 카드 120(논리 px). 고배율에서 업스케일이 다소 소프트해도
/// 평면 일러스트라 수용 — **내장 용량 최소화가 우선**(사용자 확정 08-14).
const SIDE_DEFAULT: u32 = 160;
/// 내용이 차지하는 비율 — 1.0이면 테두리에 딱 붙는다. 원형 마스크에 잘리지 않게 여유를 둔다.
const CONTENT: f32 = 0.92;

/// 12간지 순서와 키(**설정에 저장되는 값이라 불변** — 순서·철자를 바꾸면 기존 선택이 깨진다).
const ZODIAC: [&str; 12] = [
    "rat", "ox", "tiger", "rabbit", "dragon", "snake", "horse", "goat", "monkey", "rooster", "dog",
    "pig",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let src = args.next().unwrap_or_else(|| "assets/avatars".into());
    let dst = args
        .next()
        .unwrap_or_else(|| "crates/nbeep-ui/assets/avatars.nbav".into());
    let side: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(SIDE_DEFAULT);

    let files = collect(Path::new(&src));
    if files.is_empty() {
        eprintln!("원본 PNG가 없다: {src}");
        std::process::exit(1);
    }

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, path) in files.iter().enumerate() {
        let key = key_for(path, i);
        let (w, h, rgba) = read_png(path);
        let mut norm = normalize(w, h, &rgba, side);
        circle_mask(side, &mut norm);
        clean(&mut norm);
        let rle = compress(&norm);
        println!(
            "{key:8} {:>4}x{:<4} → {side}x{side}  RLE {:>7}B  (원본 대비 {:.1}%)",
            w,
            h,
            rle.len(),
            rle.len() as f32 / (side * side * 4) as f32 * 100.0
        );
        entries.push((key, rle));
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"NBAV1");
    out.push(1);
    out.extend_from_slice(&u16::try_from(entries.len()).expect("항목 수").to_le_bytes());
    out.extend_from_slice(&u16::try_from(side).expect("변").to_le_bytes());
    for (key, rle) in &entries {
        out.push(u8::try_from(key.len()).expect("키 길이"));
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&u32::try_from(rle.len()).expect("RLE 길이").to_le_bytes());
        out.extend_from_slice(rle);
    }

    if let Some(dir) = Path::new(&dst).parent() {
        std::fs::create_dir_all(dir).expect("출력 폴더");
    }
    let mut f = std::fs::File::create(&dst).expect("출력 파일");
    f.write_all(&out).expect("쓰기");
    println!(
        "\n{dst} — {}종 · {} B ({:.1} KB)",
        entries.len(),
        out.len(),
        out.len() as f32 / 1024.0
    );
}

/// 원본 목록 — 이름 오름차순(파일명 앞 번호가 12간지 순서다).
fn collect(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
        .collect();
    v.sort();
    v
}

/// 파일명에서 키를 뽑는다 — `03-tiger.png` → `tiger`. 못 뽑으면 **순서로** 12간지에 대응
/// (사용자가 이름을 다르게 줘도 넣기만 하면 되게).
fn key_for(path: &Path, idx: usize) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let name = stem.rsplit('-').next().unwrap_or("").to_ascii_lowercase();
    if ZODIAC.contains(&name.as_str()) {
        return name;
    }
    ZODIAC
        .get(idx)
        .map_or_else(|| format!("a{idx}"), |z| (*z).to_string())
}

/// PNG → (w, h, RGBA8). 도구라 실패는 즉시 패닉(고칠 사람이 바로 본다).
fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let dec = png::Decoder::new(std::io::BufReader::new(f));
    let mut reader = dec.read_info().expect("PNG 헤더");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("PNG 프레임");
    let (w, h) = (info.width, info.height);
    let px = (w * h) as usize;
    let mut rgba = vec![0u8; px * 4];
    match info.color_type {
        png::ColorType::Rgba => rgba.copy_from_slice(&buf[..px * 4]),
        png::ColorType::Rgb => {
            for i in 0..px {
                rgba[i * 4..i * 4 + 3].copy_from_slice(&buf[i * 3..i * 3 + 3]);
                rgba[i * 4 + 3] = 255;
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for i in 0..px {
                let (v, a) = (buf[i * 2], buf[i * 2 + 1]);
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, a]);
            }
        }
        png::ColorType::Grayscale => {
            for i in 0..px {
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[buf[i], buf[i], buf[i], 255]);
            }
        }
        png::ColorType::Indexed => panic!("{}: 인덱스 PNG는 RGBA로 저장해 달라", path.display()),
    }
    (w, h, rgba)
}

/// 알파 바운딩 박스 — 완전 투명이 아닌 픽셀의 최소 사각형. 전부 투명이면 전체.
fn alpha_bbox(w: u32, h: u32, rgba: &[u8]) -> (u32, u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if rgba[((y * w + x) * 4 + 3) as usize] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        return (0, 0, w.saturating_sub(1), h.saturating_sub(1));
    }
    (x0, y0, x1, y1)
}

/// 여백 제거 → 비율 유지 축소 → 정사각 중앙 배치. 축소는 **박스 평균**(imgdec와 같은 방식).
fn normalize(w: u32, h: u32, rgba: &[u8], side: u32) -> Vec<u8> {
    let (x0, y0, x1, y1) = alpha_bbox(w, h, rgba);
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);

    // 내용이 들어갈 목표 크기(비율 유지 · 긴 변이 side*CONTENT).
    let box_px = (side as f32 * CONTENT).round().max(1.0);
    let scale = (box_px / cw.max(ch) as f32).min(1.0_f32.max(box_px / cw.max(ch) as f32));
    let tw = ((cw as f32 * scale).round() as u32).clamp(1, side);
    let th = ((ch as f32 * scale).round() as u32).clamp(1, side);

    let mut out = vec![0u8; (side * side * 4) as usize];
    let ox = (side - tw) / 2;
    let oy = (side - th) / 2;

    for ty in 0..th {
        for tx in 0..tw {
            // 목표 픽셀이 덮는 원본 영역(박스 평균 — 알파 가중으로 헤일로를 막는다).
            let sx0 = x0 + (tx * cw) / tw;
            let sx1 = (x0 + ((tx + 1) * cw).div_ceil(tw)).min(x1 + 1).max(sx0 + 1);
            let sy0 = y0 + (ty * ch) / th;
            let sy1 = (y0 + ((ty + 1) * ch).div_ceil(th)).min(y1 + 1).max(sy0 + 1);
            let (mut r, mut g, mut b, mut a, mut n) = (0f32, 0f32, 0f32, 0f32, 0f32);
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let i = ((sy * w + sx) * 4) as usize;
                    let av = f32::from(rgba[i + 3]) / 255.0;
                    r += f32::from(rgba[i]) * av;
                    g += f32::from(rgba[i + 1]) * av;
                    b += f32::from(rgba[i + 2]) * av;
                    a += av;
                    n += 1.0;
                }
            }
            if n == 0.0 {
                continue;
            }
            let o = (((oy + ty) * side + ox + tx) * 4) as usize;
            // 알파 가중 평균을 되돌린다(a==0이면 완전 투명이라 색은 의미 없다).
            let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
            out[o] = (r * inv).round().clamp(0.0, 255.0) as u8;
            out[o + 1] = (g * inv).round().clamp(0.0, 255.0) as u8;
            out[o + 2] = (b * inv).round().clamp(0.0, 255.0) as u8;
            out[o + 3] = (a / n * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// 원형 마스크(사용자 확정 08-14 — "동그라미 밖은 잘라낸다") — 내접원 밖 알파 0 ·
/// 가장자리 1px AA(imgdec `circle_mask`와 같은 공식). 0.92 안착이라도 정사각에
/// 가까운 그림은 **모서리가 내접원을 넘는다**(모서리 거리 0.65 > 반지름 0.5) —
/// 굽는 단계에서 잘라야 스와치·프리뷰·툴바 어디서든 원 밖이 없다. 부수 효과 =
/// 투명해진 모서리가 RLE 런으로 뭉쳐 자산이 더 준다.
fn circle_mask(side: u32, rgba: &mut [u8]) {
    let c = side as f32 / 2.0;
    for y in 0..side {
        for x in 0..side {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((px - c).powi(2) + (py - c).powi(2)).sqrt();
            let cov = (c - d + 0.5).clamp(0.0, 1.0);
            let i = ((y * side + x) * 4 + 3) as usize;
            rgba[i] = (f32::from(rgba[i]) * cov).round() as u8;
        }
    }
}

/// 용량 정리(사용자 확정 08-14 — "필요한 수준으로 축소·용량 최소화") — RLE는 **같은
/// 픽셀의 연속**만 줄이므로, 보이지 않는 차이를 지워 런을 잇는다:
/// ① 완전 투명(a≤8)은 색을 0으로 통일 — 투명 영역의 잡색이 런을 끊는 주범.
/// ② 색/알파 하위 3비트 절사(8단위 · 최대 오차 3% — 평면 일러스트에서 비인지) —
///    AA 경계의 미세 그라데이션이 1px 런 수백 개로 흩어지는 것을 뭉친다.
fn clean(rgba: &mut [u8]) {
    for p in rgba.chunks_exact_mut(4) {
        if p[3] <= 8 {
            p.copy_from_slice(&[0, 0, 0, 0]);
        } else {
            p[0] &= 0xF8;
            p[1] &= 0xF8;
            p[2] &= 0xF8;
            p[3] &= 0xF8;
        }
    }
}

/// RGBA → RLE. `nbeep-ui::avatar_assets::compress`와 **같은 규칙**(리더가 그걸 되돌린다).
fn compress(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= rgba.len() {
        let cur = &rgba[i..i + 4];
        let mut run = 1u8;
        while run < u8::MAX
            && i + (run as usize + 1) * 4 <= rgba.len()
            && &rgba[i + run as usize * 4..i + (run as usize + 1) * 4] == cur
        {
            run += 1;
        }
        out.push(run);
        out.extend_from_slice(cur);
        i += run as usize * 4;
    }
    out
}
