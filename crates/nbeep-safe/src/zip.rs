//! **zip 포맷 파서 어댑터**(M4-4 · 08-21) — [`crate::archive`] 정책 계층의 첫 실물.
//!
//! 대전제 그대로: **해제하지 않는다.** 여기서 읽는 것은 **EOCD(끝 레코드)와 중앙
//! 디렉터리뿐**이고, 압축 데이터는 한 바이트도 해석하지 않는다(inflate 부재가
//! 구조적 보증 — 압축 폭탄이 이 경로로는 터질 수 없다). 항목 서술(`EntryDesc`)을
//! 만들어 정책([`crate::check_archive`])에 넘길 뿐이다(DR-21 — 파서를 바꿔도 규칙 불변).
//!
//! **fail-closed**: 잘림·서명 불일치·범위 밖 오프셋·멀티 디스크 전부
//! [`ZipInspect::Malformed`] — "판정 불가"는 통과가 아니다([archive 모듈 규칙]
//! "애매하면 거부"). 이름 인코딩은 lossy UTF-8(정책 판정용 — 실체화 이름은 어차피
//! `sanitize_filename`을 다시 탄다).

use crate::archive::{check_archive, ArchivePolicy, ArchiveReject, EntryDesc};

/// zip 점검 결과 — 게이트가 표시·마찰의 근거로 쓴다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZipInspect {
    /// 정책 통과 — 안전한 상대 경로 목록(목록 UI 원료 · 해제 아님).
    Ok(Vec<String>),
    /// 정책 위반(Zip Slip·폭탄·링크 등) — 사유 문장.
    Reject(String),
    /// 파싱 불가(잘림·손상·멀티 디스크) — 판정 불가 = 통과 아님(사유 문장).
    Malformed(&'static str),
}

const EOCD_SIG: u32 = 0x0605_4b50;
const CDIR_SIG: u32 = 0x0201_4b50;
/// ZIP64 EOCD 로케이터 서명(EOCD 바로 앞 20B).
const Z64_LOC_SIG: u32 = 0x0706_4b50;
/// ZIP64 EOCD 레코드 서명.
const Z64_EOCD_SIG: u32 = 0x0606_4b50;
/// EOCD 고정부 크기.
const EOCD_LEN: usize = 22;
/// ZIP64 EOCD 로케이터 크기.
const Z64_LOC_LEN: usize = 20;
/// 중앙 디렉터리 항목 고정부 크기.
const CDIR_LEN: usize = 46;
/// 항목 수 상한(zip64 — u64 선언을 그대로 믿고 할당하지 않는다 · fail-closed).
const MAX_ENTRIES: u64 = 1_000_000;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// 이 바이트가 zip으로 보이는가(로컬 헤더 서명) — 게이트의 진입 판정 보조.
#[must_use]
pub fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

/// **중앙 디렉터리만 읽어** 항목 서술을 만든다(압축 데이터 무접촉).
///
/// # Errors
/// 잘림·서명·범위·멀티 디스크 = `Err(사유)` — 호출자는 판정 불가로 취급한다(fail-closed).
/// ZIP64는 지원한다(08-21 ⓑ — 로케이터→EOCD64·항목 extra 0x0001 · 역시 목록만 읽는다).
pub fn parse_zip_entries(bytes: &[u8]) -> Result<Vec<EntryDesc>, &'static str> {
    // EOCD 탐색 — 끝에서 뒤로(주석 최대 65,535B). 주석 안에 서명이 우연히 있을 수
    // 있어, "고정부+주석 길이 = 파일 끝"이 성립하는 후보만 받는다.
    let scan_from = bytes.len().saturating_sub(EOCD_LEN + 65_535);
    let mut eocd: Option<usize> = None;
    let mut i = bytes.len().saturating_sub(EOCD_LEN);
    loop {
        if u32_at(bytes, i) == Some(EOCD_SIG) {
            let comment = usize::from(u16_at(bytes, i + 20).ok_or("EOCD 잘림")?);
            if i + EOCD_LEN + comment == bytes.len() {
                eocd = Some(i);
                break;
            }
        }
        if i == scan_from || i == 0 {
            break;
        }
        i -= 1;
    }
    let e = eocd.ok_or("EOCD 없음(zip 아님·잘림)")?;
    let mut total = u64::from(u16_at(bytes, e + 10).ok_or("EOCD 잘림")?);
    let mut cd_size = u64::from(u32_at(bytes, e + 12).ok_or("EOCD 잘림")?);
    let mut cd_off = u64::from(u32_at(bytes, e + 16).ok_or("EOCD 잘림")?);
    // ZIP64 표식(0xFFFF/0xFFFFFFFF) — 로케이터→EOCD64에서 실값을 읽는다(08-21 ⓑ).
    // 여기도 원칙은 그대로: **중앙 디렉터리만** 읽고, 판정 불가는 통과가 아니다.
    if total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
        let l = e.checked_sub(Z64_LOC_LEN).ok_or("ZIP64 로케이터 없음")?;
        if u32_at(bytes, l) != Some(Z64_LOC_SIG) {
            return Err("ZIP64 로케이터 서명 불일치");
        }
        if u32_at(bytes, l + 4) != Some(0) || u32_at(bytes, l + 16) != Some(1) {
            return Err("멀티 디스크 zip — 판정 미지원"); // fail-closed
        }
        let z_off =
            usize::try_from(u64_at(bytes, l + 8).ok_or("로케이터 잘림")?).map_err(|_| "범위")?;
        if u32_at(bytes, z_off) != Some(Z64_EOCD_SIG) {
            return Err("ZIP64 EOCD 서명 불일치");
        }
        // 고정부: sig4·size8·ver2×2·disk4×2 다음이 entries(this)8·entries(total)8·
        // cd_size8·cd_off8. 디스크 필드는 로케이터에서 이미 단일임을 확인했다.
        total = u64_at(bytes, z_off + 32).ok_or("ZIP64 EOCD 잘림")?;
        cd_size = u64_at(bytes, z_off + 40).ok_or("ZIP64 EOCD 잘림")?;
        cd_off = u64_at(bytes, z_off + 48).ok_or("ZIP64 EOCD 잘림")?;
        if u64_at(bytes, z_off + 24) != Some(total) {
            return Err("ZIP64 항목 수 선언 불일치(손상)");
        }
    }
    if total > MAX_ENTRIES {
        return Err("항목 수 과다 — 판정 거부"); // 선언을 믿고 할당하지 않는다
    }
    let cd_off = usize::try_from(cd_off).map_err(|_| "중앙 디렉터리 범위 오류")?;
    let cd_end = cd_off
        .checked_add(usize::try_from(cd_size).map_err(|_| "중앙 디렉터리 범위 오류")?)
        .ok_or("중앙 디렉터리 범위 오류")?;
    if cd_end > e {
        return Err("중앙 디렉터리가 EOCD를 넘는다(손상)");
    }
    let cd = bytes.get(cd_off..cd_end).ok_or("중앙 디렉터리 범위 밖")?;
    let total = usize::try_from(total).map_err(|_| "항목 수 과다 — 판정 거부")?;

    let mut out = Vec::with_capacity(total.min(1024));
    let mut p = 0usize;
    while p < cd.len() {
        if u32_at(cd, p) != Some(CDIR_SIG) {
            return Err("중앙 디렉터리 서명 불일치(손상)");
        }
        let mut comp = u64::from(u32_at(cd, p + 20).ok_or("항목 잘림")?);
        let mut uncomp = u64::from(u32_at(cd, p + 24).ok_or("항목 잘림")?);
        let name_len = usize::from(u16_at(cd, p + 28).ok_or("항목 잘림")?);
        let extra_len = usize::from(u16_at(cd, p + 30).ok_or("항목 잘림")?);
        let comment_len = usize::from(u16_at(cd, p + 32).ok_or("항목 잘림")?);
        let ext_attrs = u32_at(cd, p + 38).ok_or("항목 잘림")?;
        let name_b = cd
            .get(p + CDIR_LEN..p + CDIR_LEN + name_len)
            .ok_or("이름 잘림")?;
        // ZIP64 항목 extra(0x0001 · 08-21 ⓑ) — 표식(0xFFFFFFFF)인 필드의 실값이
        // **표식 순서대로**(uncomp→comp→offset→disk) 8B씩 담긴다. 표식인데 extra에
        // 값이 없으면 손상 = 판정 불가(fail-closed).
        if comp == 0xFFFF_FFFF || uncomp == 0xFFFF_FFFF {
            let extra = cd
                .get(p + CDIR_LEN + name_len..p + CDIR_LEN + name_len + extra_len)
                .ok_or("extra 잘림")?;
            let mut z64: Option<&[u8]> = None;
            let mut q = 0usize;
            while q + 4 <= extra.len() {
                let id = u16_at(extra, q).ok_or("extra 잘림")?;
                let sz = usize::from(u16_at(extra, q + 2).ok_or("extra 잘림")?);
                let data = extra.get(q + 4..q + 4 + sz).ok_or("extra 잘림")?;
                if id == 0x0001 {
                    z64 = Some(data);
                    break;
                }
                q += 4 + sz;
            }
            let data = z64.ok_or("ZIP64 표식인데 extra 0x0001 없음(손상)")?;
            let mut r = 0usize;
            if uncomp == 0xFFFF_FFFF {
                uncomp = u64_at(data, r).ok_or("ZIP64 extra 잘림")?;
                r += 8;
            }
            if comp == 0xFFFF_FFFF {
                comp = u64_at(data, r).ok_or("ZIP64 extra 잘림")?;
            }
        }
        let name = String::from_utf8_lossy(name_b).into_owned();
        let is_dir = name.ends_with('/') || name.ends_with('\\');
        // 유닉스 모드(상위 16비트) S_IFLNK = 링크 — 추출 대상 밖을 가리킬 수 있다.
        let is_link = (ext_attrs >> 16) & 0xF000 == 0xA000;
        out.push(EntryDesc {
            name,
            local_name: None, // v1 = 중앙만 파싱(로컬 대조는 해제 시점 이중 방어 몫)
            compressed: comp,
            uncompressed: uncomp,
            is_dir,
            is_link,
            depth: 1,
        });
        if out.len() > total {
            return Err("항목 수가 EOCD 선언을 넘는다(손상)");
        }
        p = p
            .checked_add(CDIR_LEN + name_len + extra_len + comment_len)
            .ok_or("항목 범위 오류")?;
    }
    if out.len() != total {
        return Err("항목 수가 EOCD 선언과 다르다(손상)");
    }
    Ok(out)
}

/// zip 점검 한 번에 — 파싱 + 정책([`check_archive`]). 해제하지 않는다.
#[must_use]
pub fn inspect_zip(bytes: &[u8], policy: &ArchivePolicy) -> ZipInspect {
    match parse_zip_entries(bytes) {
        Err(why) => ZipInspect::Malformed(why),
        Ok(entries) => match check_archive(&entries, policy) {
            Ok(paths) => ZipInspect::Ok(paths),
            Err(e) => ZipInspect::Reject(reject_text(&e)),
        },
    }
}

fn reject_text(e: &ArchiveReject) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 zip 조립 — **중앙 디렉터리 + EOCD만**(파서가 읽는 전부).
    /// (comp, uncomp, ext_attrs)와 이름으로 항목을 만든다.
    fn fake_zip(entries: &[(&str, u32, u32, u32)]) -> Vec<u8> {
        let mut cd = Vec::new();
        for (name, comp, uncomp, ext) in entries {
            let nb = name.as_bytes();
            cd.extend_from_slice(&CDIR_SIG.to_le_bytes());
            cd.extend_from_slice(&[0u8; 16]); // ver×2·flag·method·time·date·crc
            cd.extend_from_slice(&comp.to_le_bytes());
            cd.extend_from_slice(&uncomp.to_le_bytes());
            cd.extend_from_slice(&(u16::try_from(nb.len()).unwrap()).to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes()); // extra
            cd.extend_from_slice(&0u16.to_le_bytes()); // comment
            cd.extend_from_slice(&[0u8; 4]); // disk·int attrs
            cd.extend_from_slice(&ext.to_le_bytes());
            cd.extend_from_slice(&0u32.to_le_bytes()); // local offset
            cd.extend_from_slice(nb);
        }
        let mut out = b"PK\x03\x04".to_vec(); // 로컬 헤더 흉내(파서는 안 읽는다)
        let cd_off = u32::try_from(out.len()).unwrap();
        out.extend_from_slice(&cd);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // disk×2
        let n = u16::try_from(entries.len()).unwrap();
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&(u32::try_from(cd.len()).unwrap()).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    #[test]
    fn parses_names_sizes_dirs_and_links() {
        let z = fake_zip(&[
            ("docs/", 0, 0, 0),
            ("docs/a.txt", 100, 1000, 0),
            ("evil-link", 10, 10, 0xA1FF_0000), // 유닉스 심링크 모드
        ]);
        let e = parse_zip_entries(&z).unwrap();
        assert_eq!(e.len(), 3);
        assert!(e[0].is_dir && !e[0].is_link);
        assert_eq!((e[1].compressed, e[1].uncompressed), (100, 1000));
        assert!(e[2].is_link, "S_IFLNK 인지");
    }

    #[test]
    fn zip_slip_and_bomb_rejected_via_policy() {
        let pol = ArchivePolicy::default();
        // Zip Slip.
        let z = fake_zip(&[("../../etc/passwd", 10, 10, 0)]);
        assert!(matches!(inspect_zip(&z, &pol), ZipInspect::Reject(_)));
        // 압축률 폭탄(1B → 1MB = 상한 200배 초과).
        let z = fake_zip(&[("a.bin", 1, 1_000_000, 0)]);
        assert!(matches!(inspect_zip(&z, &pol), ZipInspect::Reject(_)));
        // 정상.
        let z = fake_zip(&[("ok/파일.txt", 500, 1000, 0)]);
        match inspect_zip(&z, &pol) {
            ZipInspect::Ok(paths) => assert_eq!(paths, vec!["ok/파일.txt".to_string()]),
            other => panic!("정상 zip이 거부됨: {other:?}"),
        }
    }

    #[test]
    fn malformed_and_zip64_are_not_a_pass() {
        let pol = ArchivePolicy::default();
        // 잘림(EOCD 없음).
        assert!(matches!(
            inspect_zip(b"PK\x03\x04junk", &pol),
            ZipInspect::Malformed(_)
        ));
        // ZIP64 표식인데 로케이터가 없다(잘림·조작) — 판정 불가(fail-closed).
        let mut z = fake_zip(&[("a", 1, 1, 0)]);
        let e = z.len() - EOCD_LEN;
        z[e + 16..e + 20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // cd_off 표식
        assert!(matches!(inspect_zip(&z, &pol), ZipInspect::Malformed(_)));
        // 중앙 디렉터리 서명 파손.
        let mut z = fake_zip(&[("a", 1, 1, 0)]);
        z[4] ^= 0xFF; // cd 첫 서명 바이트
        assert!(matches!(inspect_zip(&z, &pol), ZipInspect::Malformed(_)));
    }

    /// zip64 조립(08-21 ⓑ) — 항목 extra 0x0001 + EOCD64 + 로케이터 + 표식 EOCD.
    /// (uncomp, comp)를 extra에 싣고 고정 필드는 표식(0xFFFFFFFF)으로 둔다.
    fn fake_zip64(entries: &[(&str, u64, u64)]) -> Vec<u8> {
        let mut cd = Vec::new();
        for (name, uncomp, comp) in entries {
            let nb = name.as_bytes();
            let mut extra = Vec::new();
            extra.extend_from_slice(&0x0001u16.to_le_bytes());
            extra.extend_from_slice(&16u16.to_le_bytes());
            extra.extend_from_slice(&uncomp.to_le_bytes());
            extra.extend_from_slice(&comp.to_le_bytes());
            cd.extend_from_slice(&CDIR_SIG.to_le_bytes());
            cd.extend_from_slice(&[0u8; 16]);
            cd.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // comp 표식
            cd.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // uncomp 표식
            cd.extend_from_slice(&(u16::try_from(nb.len()).unwrap()).to_le_bytes());
            cd.extend_from_slice(&(u16::try_from(extra.len()).unwrap()).to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes()); // comment
            cd.extend_from_slice(&[0u8; 4]); // disk·int attrs
            cd.extend_from_slice(&0u32.to_le_bytes()); // ext attrs
            cd.extend_from_slice(&0u32.to_le_bytes()); // local offset
            cd.extend_from_slice(nb);
            cd.extend_from_slice(&extra);
        }
        let mut out = b"PK\x03\x04".to_vec();
        let cd_off = out.len() as u64;
        out.extend_from_slice(&cd);
        let z_off = out.len() as u64;
        out.extend_from_slice(&Z64_EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&44u64.to_le_bytes()); // 레코드 잔여 크기
        out.extend_from_slice(&[0u8; 4]); // ver made·need
        out.extend_from_slice(&0u32.to_le_bytes()); // disk
        out.extend_from_slice(&0u32.to_le_bytes()); // cd start disk
        let n = entries.len() as u64;
        out.extend_from_slice(&n.to_le_bytes()); // entries this disk
        out.extend_from_slice(&n.to_le_bytes()); // total entries
        out.extend_from_slice(&(cd.len() as u64).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&Z64_LOC_SIG.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // z64 eocd 디스크
        out.extend_from_slice(&z_off.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes()); // 총 디스크 1
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // disk×2
        out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // 항목 수 표식
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // cd size 표식
        out.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // cd off 표식
        out.extend_from_slice(&0u16.to_le_bytes()); // comment
        out
    }

    /// ★ zip64 지원(08-21 ⓑ) — 로케이터→EOCD64 경로와 항목 extra의 8B 실값이
    /// 읽히고, 4GiB 초과 크기가 정확하다. 정책(폭탄 비율)도 그 실값으로 돈다.
    #[test]
    fn zip64_locator_eocd_and_entry_extras_parse() {
        let five_g = 5_000_000_000u64;
        let z = fake_zip64(&[("big/one.bin", five_g, four_g()), ("s.txt", 1000, 500)]);
        let e = parse_zip_entries(&z).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].uncompressed, five_g, "4GiB 초과 실값");
        assert_eq!(e[0].compressed, four_g());
        // 총 해제 상한(기본 1GiB)은 zip64 실값에도 그대로 적용 — 5GB = Reject.
        // (파서가 열어 줘도 정책이 막는다 — 계층 분리의 증거.)
        assert!(matches!(
            inspect_zip(&z, &ArchivePolicy::default()),
            ZipInspect::Reject(_)
        ));
        // 상한 안쪽 zip64(구조만 64bit) — 목록이 나온다.
        let z = fake_zip64(&[("ok/a.bin", 1000, 500), ("ok/b.txt", 2000, 900)]);
        match inspect_zip(&z, &ArchivePolicy::default()) {
            ZipInspect::Ok(paths) => assert_eq!(paths.len(), 2),
            other => panic!("정상 zip64가 거부됨: {other:?}"),
        }
    }
    fn four_g() -> u64 {
        4_000_000_000
    }

    #[test]
    fn eocd_signature_inside_comment_is_not_fooled() {
        // 주석에 EOCD 서명 바이트가 들어 있어도 "고정부+주석 = 파일 끝" 검증이 가른다.
        let mut z = fake_zip(&[("a.txt", 10, 20, 0)]);
        let mut comment = EOCD_SIG.to_le_bytes().to_vec();
        comment.extend_from_slice(b"decoy");
        let clen = u16::try_from(comment.len()).unwrap();
        let e = z.len() - 2;
        z[e..].copy_from_slice(&clen.to_le_bytes());
        z.extend_from_slice(&comment);
        let entries = parse_zip_entries(&z).unwrap();
        assert_eq!(entries.len(), 1, "미끼 서명에 속지 않는다");
    }
}
