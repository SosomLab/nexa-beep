//! ★ 헤드리스 캡처용 PNG 인코더(11-02 · nexa-sql `png.rs` D-436 이식) — 외부 crate 0 · zlib **저장 블록**(압축 없음 ·
//! 시험 산출물이라 크기보다 단순함) · 입력 = 프레임버퍼(`0x00RRGGBB` u32 · 행 우선) → RGB 8비트 PNG.
//! `NEXA_SCRIPT capture=<파일>[@<창 제목 일부>]`가 present 직전 프레임을 떨구는 데 쓴다(`app.rs` `redraw`).

/// CRC-32(IEEE · PNG 청크).
fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_be_bytes());
    let mut c = Vec::with_capacity(4 + body.len());
    c.extend_from_slice(kind);
    c.extend_from_slice(body);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

/// zlib 스트림(저장 블록 · 최대 65,535바이트씩) + adler32.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut z = Vec::with_capacity(raw.len() + raw.len() / 65_535 * 5 + 16);
    z.extend_from_slice(&[0x78, 0x01]);
    let mut chunks = raw.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        let last = chunks.peek().is_none();
        z.push(u8::from(last));
        #[allow(clippy::cast_possible_truncation)] // ≤ 65_535
        let n = c.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&nexa_gfx::inflate::adler32(raw).to_be_bytes());
    z
}

/// `0x00RRGGBB` 프레임 → PNG 바이트(행마다 필터 0).
#[must_use]
pub(crate) fn encode(width: u32, height: u32, px: &[u32]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut raw = Vec::with_capacity(h * (1 + w * 3));
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let p = px.get(y * w + x).copied().unwrap_or(0);
            #[allow(clippy::cast_possible_truncation)]
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    let mut out = Vec::with_capacity(raw.len() + 64);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8비트 · RGB · deflate · 필터 0 · 비인터레이스
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be32(b: &[u8]) -> u32 {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }

    /// 2×2 프레임 → PNG 머리·IHDR·IEND · zlib 저장 블록을 nexa-gfx 인플레이터로 되풀면 원시 행 그대로 · CRC 표준값.
    #[test]
    fn encodes_png_round_trip_with_inflate() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926, "CRC-32 표준 검증값");
        let px = [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0x00FF_FFFF];
        let png = encode(2, 2, &px);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(be32(&png[16..20]), 2);
        assert_eq!(be32(&png[20..24]), 2);
        assert!(png.ends_with(&[b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
        let idat_len = be32(&png[33..37]) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let raw = nexa_gfx::inflate::inflate_zlib(&png[41..41 + idat_len]).unwrap();
        assert_eq!(
            raw,
            vec![0, 255, 0, 0, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255]
        );
        let big = vec![0x0012_3456u32; 300 * 100];
        let png = encode(300, 100, &big);
        let idat_len = be32(&png[33..37]) as usize;
        let raw = nexa_gfx::inflate::inflate_zlib(&png[41..41 + idat_len]).unwrap();
        assert_eq!(raw.len(), 100 * (1 + 300 * 3));
        assert_eq!(&raw[1..4], &[0x12, 0x34, 0x56]);
    }
}
