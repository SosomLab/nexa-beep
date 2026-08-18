//! **범용 로컬 봉투**(ADR-0005 §4~ · 08-17 사용자 승인 "평문 3면 조치") — 로컬
//! 디스크에 남는 민감 파일(격리물 `.beepq` · 프로필 이미지 캐시 · PII 사이드카)을
//! 한 문법으로 봉인한다. trust.seg(ADR-0005 §4-2)와 같은 계열(ChaCha20-Poly1305 ·
//! Noise와 동일 암호군)이되, **도메인 분리 파생 키**로 파일 종류끼리 섞이지
//! 않게 한다 — 한 파일의 봉인을 다른 종류 파일로 바꿔치기해도 열리지 않는다
//! (도메인이 AAD와 키 유도 양쪽에 들어간다).
//!
//! 형식: `"NBSE"` ‖ ver(1) ‖ salt(16) ‖ nonce(12) ‖ ct(+tag16)
//! 키: SHA-256("nexa-beep/adr-0005/sealed-v1" ‖ domain ‖ salt ‖ wrap_secret)
//! (wrap_secret = 신원 키 파생 `Identity::wrap_secret` — D-18 §3 확정 계층.)
//!
//! 실패는 전부 **None/Err = fail-closed** — 손상·바꿔치기·다른 신원의 파일은
//! 평문인 척 통과하지 않는다. 구본(봉인 전 평문) 판정은 [`is_sealed`]가 맡고,
//! 관용(읽기)과 재봉인(이관)은 호출측 정책이다.

use std::io;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};

const MAGIC: [u8; 4] = *b"NBSE";
const VER: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const HEAD: usize = 4 + 1 + SALT_LEN;

fn derive_key(domain: &[u8], salt: &[u8; SALT_LEN], secret: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"nexa-beep/adr-0005/sealed-v1");
    h.update(domain);
    h.update(salt);
    h.update(secret);
    h.finalize().into()
}

/// 봉인 여부 — 구본(평문) 이관 판정용. 매직 우연 충돌(원본이 "NBSE"로 시작)은
/// open 실패로 드러나므로(fail-closed) 호출측 관용 정책이 감당한다.
#[must_use]
pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.len() > HEAD + NONCE_LEN && bytes[..4] == MAGIC
}

/// 봉인 — salt·nonce는 매 호출 새로 뽑는다(같은 평문도 매번 다른 봉투).
///
/// # Errors
/// OS 난수 실패·AEAD 실패(둘 다 진행 불가 — 평문을 쓰는 폴백은 없다).
pub fn seal(domain: &[u8], secret: &[u8; 32], plain: &[u8]) -> io::Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut salt)
        .and_then(|()| getrandom::getrandom(&mut nonce))
        .map_err(|_| io::Error::other("OS 난수 실패"))?;
    let mut out = Vec::with_capacity(HEAD + NONCE_LEN + plain.len() + 16);
    out.extend_from_slice(&MAGIC);
    out.push(VER);
    out.extend_from_slice(&salt);
    let mut aad = out.clone();
    aad.extend_from_slice(domain);
    let ct = ChaCha20Poly1305::new(&derive_key(domain, &salt, secret).into())
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .map_err(|_| io::Error::other("AEAD 봉인 실패"))?;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// 개봉 — 형식·버전·태그 어느 하나라도 어긋나면 None(fail-closed).
#[must_use]
pub fn open(domain: &[u8], secret: &[u8; 32], bytes: &[u8]) -> Option<Vec<u8>> {
    if !is_sealed(bytes) || bytes[4] != VER {
        return None;
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&bytes[5..HEAD]);
    let mut aad = bytes[..HEAD].to_vec();
    aad.extend_from_slice(domain);
    ChaCha20Poly1305::new(&derive_key(domain, &salt, secret).into())
        .decrypt(
            Nonce::from_slice(&bytes[HEAD..HEAD + NONCE_LEN]),
            Payload {
                msg: &bytes[HEAD + NONCE_LEN..],
                aad: &aad,
            },
        )
        .ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const S: [u8; 32] = [7u8; 32];

    #[test]
    fn roundtrip_and_fresh_envelope_every_time() {
        let a = seal(b"quarantine-v1", &S, b"hello").unwrap();
        let b = seal(b"quarantine-v1", &S, b"hello").unwrap();
        assert_ne!(a, b, "salt·nonce 신선 — 같은 평문도 봉투가 다르다");
        assert!(is_sealed(&a));
        assert_eq!(open(b"quarantine-v1", &S, &a).unwrap(), b"hello");
    }

    /// 도메인 분리 — 격리물 봉투를 캐시 자리로 옮겨도 열리지 않는다(바꿔치기 차단).
    #[test]
    fn wrong_domain_or_secret_fails_closed() {
        let a = seal(b"quarantine-v1", &S, b"x").unwrap();
        assert!(open(b"profile-cache-v1", &S, &a).is_none(), "도메인 분리");
        assert!(
            open(b"quarantine-v1", &[8u8; 32], &a).is_none(),
            "다른 신원"
        );
    }

    #[test]
    fn tamper_fails_closed() {
        let mut a = seal(b"pii-v1", &S, b"secret@x").unwrap();
        let last = a.len() - 1;
        a[last] ^= 1;
        assert!(open(b"pii-v1", &S, &a).is_none(), "태그 검증");
        assert!(!is_sealed(b"plain old bytes"), "구본 판정");
    }
}
