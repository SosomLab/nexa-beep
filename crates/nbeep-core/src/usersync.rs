//! 형제 기기 동기 프레임(ADR-0015 S2) — Control 스트림.
//!
//! - [`UserKeyBlob`] 태그 9: 사용자 키 **봉인본**(`user.key` 파일 내용 그대로 — 열쇠는 K_wrap_user).
//!   형제 세션(PSK 성립·증명)에서만 오간다. 여기는 봉투(태그·길이)만 본다.

/// 사용자 키 봉인본 태그.
pub const USER_KEY_BLOB_TAG: u8 = 9;
/// 봉인본 상한(파일 = 64+8 평문 + 봉투 — 넉넉히).
pub const USER_KEY_BLOB_MAX: usize = 512;

/// 사용자 키 봉인본.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserKeyBlob {
    /// `nbeep_store::sealed` 봉투 바이트(수신측이 자기 K_wrap_user로 연다).
    pub sealed: Vec<u8>,
}

impl UserKeyBlob {
    /// 인코딩: `tag ‖ len(u16 BE) ‖ sealed`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let n = self.sealed.len().min(USER_KEY_BLOB_MAX);
        let mut out = Vec::with_capacity(3 + n);
        out.push(USER_KEY_BLOB_TAG);
        #[allow(clippy::cast_possible_truncation)]
        out.extend_from_slice(&(n as u16).to_be_bytes());
        out.extend_from_slice(&self.sealed[..n]);
        out
    }

    /// 디코딩 — 태그·길이 정확 일치·상한 이내만.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 3 || bytes[0] != USER_KEY_BLOB_TAG {
            return None;
        }
        let n = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
        if n == 0 || n > USER_KEY_BLOB_MAX || bytes.len() != 3 + n {
            return None;
        }
        Some(Self {
            sealed: bytes[3..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_roundtrip_and_reject() {
        let b = UserKeyBlob {
            sealed: vec![1, 2, 3, 4],
        };
        let e = b.encode();
        assert_eq!(UserKeyBlob::decode(&e), Some(b));
        assert!(UserKeyBlob::decode(&e[..5]).is_none(), "길이 부족");
        let mut long = e.clone();
        long.push(9);
        assert!(UserKeyBlob::decode(&long).is_none(), "길이 초과");
        assert!(
            UserKeyBlob::decode(&[USER_KEY_BLOB_TAG, 0, 0]).is_none(),
            "빈 봉인본"
        );
        let mut bad = e;
        bad[0] = crate::USER_PROOF_TAG;
        assert!(UserKeyBlob::decode(&bad).is_none());
    }
}
