//! 세션 내 형제 증명 프레임(ADR-0015 S1-e) — Control 스트림 태그 [`USER_PROOF_TAG`].
//!
//! 와이어: `USER_PROOF_TAG(1) ‖ proof(32)`. 증명값 자체는 `nbeep-crypto`가 만든다
//! (`session_proof`) — 여기는 봉투(태그·길이)만 본다. 구버전 수신측은 미지 태그로 버린다.

/// Control 스트림 태그(프로필 1~3 · ack 10 과 겹치지 않게).
pub const USER_PROOF_TAG: u8 = 11;

/// 형제 증명 한 장.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserProof {
    /// HMAC 32바이트.
    pub proof: [u8; 32],
}

impl UserProof {
    /// 인코딩.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(33);
        out.push(USER_PROOF_TAG);
        out.extend_from_slice(&self.proof);
        out
    }

    /// 디코딩 — 태그·길이가 정확히 맞을 때만.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 33 || bytes[0] != USER_PROOF_TAG {
            return None;
        }
        let mut proof = [0u8; 32];
        proof.copy_from_slice(&bytes[1..]);
        Some(Self { proof })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_reject() {
        let p = UserProof { proof: [3u8; 32] };
        let b = p.encode();
        assert_eq!(b.len(), 33);
        assert_eq!(UserProof::decode(&b), Some(p));
        assert!(UserProof::decode(&b[..32]).is_none());
        let mut bad = b.clone();
        bad[0] = crate::ACK_TAG;
        assert!(UserProof::decode(&bad).is_none());
        let mut long = b;
        long.push(0);
        assert!(UserProof::decode(&long).is_none());
    }
}
