//! 기본 아바타 자산 — **12간지 그림 12종을 바이너리에 내장**한다(사용자 확정 08-13).
//!
//! ## 왜 파일이 아니라 내장인가
//!
//! - **파일 의존 0**(DR-4/DR-5) — 포터블은 압축을 풀면 곧바로 돌아야 한다. 자산 폴더가
//!   따라다녀야 하면 "exe 하나면 된다"가 깨지고, 빠뜨렸을 때 **조용히 아바타만 죽는다**
//!   (imgdec 동봉에서 이미 겪은 실패 양식 — [docs/26 §1-3]).
//! - **imgdec 스폰 0** — 기본 아바타는 12개가 동시에 뜬다. 매번 격리 프로세스를 띄우면
//!   목록 표시가 그만큼 늦는다(08-13 "대화창 열림 1~2초 얼음"과 같은 비용).
//! - **R-5와 무관하다** — R-5가 막는 것은 **상대가 보낸 바이트**를 본체가 파싱하는 일이다.
//!   여기 있는 바이트는 우리가 빌드 때 만들어 넣은 것이고, 리더는 아래 30줄이 전부다.
//!   ⚠️ 그래도 **길이·범위는 전부 검사**한다(자산이 깨져도 패닉 없이 없는 셈 친다).
//!
//! ## 포맷 `NBAV1` — 무손실 RLE
//!
//! ```text
//! [magic 5B "NBAV1"][ver u8=1][count u16 LE][side u16 LE]
//! count × { [key_len u8][key UTF-8][rle_len u32 LE][rle] }
//! rle = (run u8≥1, r u8, g u8, b u8, a u8) 반복 — run 합 = side*side
//! ```
//!
//! 평면 색 일러스트라 같은 픽셀이 길게 이어져 **무손실인데도 작다**(투명 배경이 통째로
//! 몇 개의 런). 팔레트 양자화를 쓰지 않은 이유 = AA 경계에서 색이 뭉개지는데, 얻는
//! 크기 이득이 그 대가만큼 크지 않았다.
//!
//! 자산은 `tools/mkavatars`가 원본(`assets/avatars/*.png`)에서 만든다 — **여백 잘라내기와
//! 크기 정규화도 거기서** 한다(런타임은 이미 정규화된 정사각만 본다).

use crate::theme::IconImage;

/// 자산 매직(포맷 v1).
const MAGIC: &[u8; 5] = b"NBAV1";
/// 한 항목의 최대 변 — 자산이 깨졌을 때 거대한 할당을 막는 상한(방어).
const MAX_SIDE: u32 = 512;
/// 최대 항목 수(방어).
const MAX_COUNT: usize = 256;

/// 내장 자산 바이트. 파일이 없으면 **빈 자산**으로 빌드된다(그림 없이도 앱은 돈다 —
/// 이니셜 아바타가 그대로 선다). 자산을 넣는 순간 12종이 살아난다.
static BUNDLE: &[u8] = include_bytes!("../assets/avatars.nbav");

/// 해석된 기본 아바타 한 종.
#[derive(Debug, Clone)]
pub struct BuiltinAvatar {
    /// 안정 키(`rat`·`ox`… — **설정에 저장되는 값**이라 바뀌면 안 된다).
    pub key: String,
    /// 정사각 RGBA.
    pub image: IconImage,
}

/// 내장 자산을 해석한다. **깨진 자산은 거기까지만 쓰고 멈춘다**(패닉 없음 · fail-soft —
/// 아바타가 몇 개 없는 것보다 앱이 죽는 게 나쁘다).
#[must_use]
pub fn builtins() -> Vec<BuiltinAvatar> {
    parse(BUNDLE)
}

/// 바이트 → 항목들(테스트가 합성 입력으로 부른다).
#[must_use]
fn parse(b: &[u8]) -> Vec<BuiltinAvatar> {
    let mut out = Vec::new();
    if b.len() < 10 || &b[..5] != MAGIC || b[5] != 1 {
        return out; // 매직/버전 불일치 = 자산 없음 취급
    }
    let count = u16::from_le_bytes([b[6], b[7]]) as usize;
    let side = u32::from(u16::from_le_bytes([b[8], b[9]]));
    if side == 0 || side > MAX_SIDE || count > MAX_COUNT {
        return out;
    }
    let px = (side * side) as usize;
    let mut p = 10usize;
    for _ in 0..count {
        let Some(&key_len) = b.get(p) else { break };
        p += 1;
        let Some(key_bytes) = b.get(p..p + key_len as usize) else {
            break;
        };
        let Ok(key) = core::str::from_utf8(key_bytes) else {
            break;
        };
        p += key_len as usize;
        let Some(len_bytes) = b.get(p..p + 4) else { break };
        let rle_len = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]])
            as usize;
        p += 4;
        let Some(rle) = b.get(p..p + rle_len) else {
            break;
        };
        p += rle_len;
        let Some(rgba) = expand(rle, px) else { break };
        out.push(BuiltinAvatar {
            key: key.to_string(),
            image: IconImage::from_rgba(side, side, rgba),
        });
    }
    out
}

/// RLE → RGBA. 픽셀 수가 정확히 `px`가 아니면 `None`(부분 해석물을 쓰지 않는다).
fn expand(rle: &[u8], px: usize) -> Option<Vec<u8>> {
    if rle.len() % 5 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(px * 4);
    for c in rle.chunks_exact(5) {
        let run = usize::from(c[0]);
        if run == 0 || out.len() + run * 4 > px * 4 {
            return None; // run 0 = 무한 루프 유발 · 초과 = 깨진 자산
        }
        for _ in 0..run {
            out.extend_from_slice(&c[1..5]);
        }
    }
    (out.len() == px * 4).then_some(out)
}

/// RGBA → RLE(생성 도구와 테스트가 쓴다 — 왕복 검증이 포맷의 계약이다).
#[must_use]
pub fn compress(rgba: &[u8]) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// 합성 이미지를 만든다 — 실자산 없이도 포맷 계약을 고정한다.
    fn synth(side: u32, seed: u8) -> Vec<u8> {
        (0..side * side)
            .flat_map(|i| {
                let v = ((i as u8) ^ seed).wrapping_mul(7);
                // 절반은 같은 색(런이 생기는 평면 영역) · 절반은 변화(최악 경우)
                if i % side < side / 2 {
                    [seed, seed, seed, 255]
                } else {
                    [v, v, v, 255]
                }
            })
            .collect()
    }

    fn bundle(items: &[(&str, u32, Vec<u8>)], side: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.push(1);
        b.extend_from_slice(&(items.len() as u16).to_le_bytes());
        b.extend_from_slice(&(side as u16).to_le_bytes());
        for (k, _, rgba) in items {
            let rle = compress(rgba);
            b.push(k.len() as u8);
            b.extend_from_slice(k.as_bytes());
            b.extend_from_slice(&(rle.len() as u32).to_le_bytes());
            b.extend_from_slice(&rle);
        }
        b
    }

    /// 왕복이 **무손실**이다 — 포맷의 핵심 계약.
    #[test]
    fn roundtrip_is_lossless() {
        let side = 16;
        let a = synth(side, 3);
        let items = vec![("rat", side, a.clone())];
        let parsed = parse(&bundle(&items, side));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].key, "rat");
        assert_eq!(parsed[0].image.rgba, a, "무손실이어야 한다");
        assert_eq!((parsed[0].image.w, parsed[0].image.h), (side, side));
    }

    /// 여러 항목이 순서대로 해석된다(키 = 설정 저장값이라 순서·이름이 계약).
    #[test]
    fn multiple_entries_keep_order() {
        let side = 8;
        let items = vec![
            ("rat", side, synth(side, 1)),
            ("ox", side, synth(side, 2)),
            ("tiger", side, synth(side, 3)),
        ];
        let parsed = parse(&bundle(&items, side));
        let keys: Vec<_> = parsed.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["rat", "ox", "tiger"]);
    }

    /// 평면 영역이 실제로 압축된다(내장이 정당한 이유 = 작다).
    #[test]
    fn flat_regions_compress() {
        let side = 64;
        let flat: Vec<u8> = (0..side * side).flat_map(|_| [9, 9, 9, 255]).collect();
        let rle = compress(&flat);
        // 4096px / 255 = 17런 미만 — 원본 16KB가 100B 아래로.
        assert!(rle.len() < 100, "평면은 극단적으로 줄어야 한다: {}", rle.len());
        assert_eq!(expand(&rle, (side * side) as usize).unwrap(), flat);
    }

    /// ⚠️ 깨진 자산은 **패닉 없이** 없는 셈 친다(fail-soft).
    #[test]
    fn corrupt_asset_is_not_a_panic() {
        assert!(parse(b"").is_empty(), "빈 입력");
        assert!(parse(b"NOTIT\x01\x00\x00\x00\x00").is_empty(), "매직 불일치");
        // 매직은 맞고 본문이 잘린 경우 — 거기까지만.
        let side = 8;
        let full = bundle(&[("rat", side, synth(side, 1))], side);
        for cut in [10, 12, full.len() - 1] {
            let _ = parse(&full[..cut]); // 패닉하지 않으면 통과
        }
    }

    /// ★ **실자산이 실제로 실려 있다** — 자산이 빠지거나 깨지면 앱은 조용히 이니셜로
    /// 폴백하므로(fail-soft) 사람이 눈치채기 어렵다. 그 침묵을 여기서 깬다.
    #[test]
    fn bundled_zodiac_is_present_and_sane() {
        let all = builtins();
        assert_eq!(all.len(), 12, "12간지 12종이 실려 있어야 한다");
        let keys: Vec<_> = all.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "rat", "ox", "tiger", "rabbit", "dragon", "snake", "horse", "goat", "monkey",
                "rooster", "dog", "pig"
            ],
            "키와 순서는 설정 저장값이라 불변이다"
        );
        for a in &all {
            assert_eq!(a.image.w, a.image.h, "{}: 정사각", a.key);
            assert_eq!(
                a.image.rgba.len(),
                (a.image.w * a.image.h * 4) as usize,
                "{}: RGBA 길이",
                a.key
            );
            // 완전 투명만 있는 자산 = 굽기 실패인데 조용하다 — 내용이 있는지 본다.
            let opaque = a.image.rgba.chunks_exact(4).filter(|p| p[3] > 8).count();
            assert!(
                opaque > (a.image.w * a.image.h / 20) as usize,
                "{}: 내용이 거의 없다({opaque}px) — 굽기 실패 의심",
                a.key
            );
        }
    }

    /// run=0은 무한 루프 유발 — 거부한다.
    #[test]
    fn zero_run_is_rejected() {
        assert!(expand(&[0, 1, 2, 3, 4], 4).is_none());
    }

    /// 픽셀 수가 어긋나면 부분 해석물을 쓰지 않는다.
    #[test]
    fn pixel_count_mismatch_is_rejected() {
        assert!(expand(&[2, 1, 2, 3, 4], 4).is_none(), "모자람");
        assert!(expand(&[8, 1, 2, 3, 4], 4).is_none(), "넘침");
    }
}
