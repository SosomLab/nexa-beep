//! **zip 포맷 파서 어댑터**(M4-4 · 08-22) — [`crate::archive`] 정책 계층의 첫 실물.
//!
//! 대전제 그대로: **해제하지 않는다.** 여기서 읽는 것은 **EOCD(끝 레코드)와 중앙
//! 디렉터리뿐**이고, 압축 데이터는 한 바이트도 해석하지 않는다(inflate 부재가
//! 구조적 보증 — 압축 폭탄이 이 경로로는 터질 수 없다). 항목 서술(`EntryDesc`)을
//! 만들어 정책([`crate::check_archive`])에 넘길 뿐이다(DR-21 — 파서를 바꿔도 규칙 불변).
//!
//! **fail-closed**: 잘림·서명 불일치·범위 밖 오프셋·ZIP64(v1 미지원) 전부
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
    /// 파싱 불가(잘림·손상·ZIP64 미지원) — 판정 불가 = 통과 아님(사유 문장).
    Malformed(&'static str),
}

const EOCD_SIG: u32 = 0x0605_4b50;
const CDIR_SIG: u32 = 0x0201_4b50;
/// EOCD 고정부 크기.
const EOCD_LEN: usize = 22;
/// 중앙 디렉터리 항목 고정부 크기.
const CDIR_LEN: usize = 46;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// 이 바이트가 zip으로 보이는가(로컬 헤더 서명) — 게이트의 진입 판정 보조.
#[must_use]
pub fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

/// **중앙 디렉터리만 읽어** 항목 서술을 만든다(압축 데이터 무접촉).
///
/// # Errors
/// 잘림·서명·범위·ZIP64 = `Err(사유)` — 호출자는 판정 불가로 취급한다(fail-closed).
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
    let total = usize::from(u16_at(bytes, e + 10).ok_or("EOCD 잘림")?);
    let cd_size = u32_at(bytes, e + 12).ok_or("EOCD 잘림")? as usize;
    let cd_off = u32_at(bytes, e + 16).ok_or("EOCD 잘림")? as usize;
    // ZIP64 표식(0xFFFF/0xFFFFFFFF) — v1 미지원 = 판정 불가(통과 아님).
    if total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
        return Err("ZIP64 — v1 판정 미지원");
    }
    let cd_end = cd_off
        .checked_add(cd_size)
        .ok_or("중앙 디렉터리 범위 오류")?;
    if cd_end > e {
        return Err("중앙 디렉터리가 EOCD를 넘는다(손상)");
    }
    let cd = bytes.get(cd_off..cd_end).ok_or("중앙 디렉터리 범위 밖")?;

    let mut out = Vec::with_capacity(total.min(1024));
    let mut p = 0usize;
    while p < cd.len() {
        if u32_at(cd, p) != Some(CDIR_SIG) {
            return Err("중앙 디렉터리 서명 불일치(손상)");
        }
        let comp = u64::from(u32_at(cd, p + 20).ok_or("항목 잘림")?);
        let uncomp = u64::from(u32_at(cd, p + 24).ok_or("항목 잘림")?);
        if comp == 0xFFFF_FFFF || uncomp == 0xFFFF_FFFF {
            return Err("ZIP64 항목 — v1 판정 미지원");
        }
        let name_len = usize::from(u16_at(cd, p + 28).ok_or("항목 잘림")?);
        let extra_len = usize::from(u16_at(cd, p + 30).ok_or("항목 잘림")?);
        let comment_len = usize::from(u16_at(cd, p + 32).ok_or("항목 잘림")?);
        let ext_attrs = u32_at(cd, p + 38).ok_or("항목 잘림")?;
        let name_b = cd
            .get(p + CDIR_LEN..p + CDIR_LEN + name_len)
            .ok_or("이름 잘림")?;
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
        // ZIP64 표식.
        let mut z = fake_zip(&[("a", 1, 1, 0)]);
        let e = z.len() - EOCD_LEN;
        z[e + 16..e + 20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // cd_off 표식
        assert!(matches!(inspect_zip(&z, &pol), ZipInspect::Malformed(_)));
        // 중앙 디렉터리 서명 파손.
        let mut z = fake_zip(&[("a", 1, 1, 0)]);
        z[4] ^= 0xFF; // cd 첫 서명 바이트
        assert!(matches!(inspect_zip(&z, &pol), ZipInspect::Malformed(_)));
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
