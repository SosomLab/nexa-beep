//! 형제 기기 동기 프레임(ADR-0015 S2) — Control 스트림.
//!
//! - [`UserKeyBlob`] 태그 9: 사용자 키 **봉인본**(`user.key` 파일 내용 그대로 — 열쇠는 K_wrap_user).
//!   형제 세션(PSK 성립·증명)에서만 오간다. 여기는 봉투(태그·길이)만 본다.
//! - [`Succession`] 태그 8: **후계 증명서**(ADR-0015 §3-5 · 기기 분실 대응) — 옛 사용자 키와 새 키가
//!   **같은 문서에 둘 다 서명**한다. 상대는 ①두 서명 ②제시자 ∈ devices ∧ ∉ revoked ③버전 단조로
//!   검증하고 옛 UserId의 기록을 새 UserId로 접는다. 같은 옛 키·같은 버전·다른 새 키 = **정직한 충돌**.

use crate::identity::PeerId;

/// 후계 증명서 태그.
pub const SUCCESSION_TAG: u8 = 8;
const SUCC_DOM: &[u8] = b"nbeep-user-succ-v1";
const SUCC_MAX: usize = 16;

/// 후계 증명서.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Succession {
    /// 옛 사용자 공개키(잃어버린 기기가 아직 쥔 키).
    pub old_pub: [u8; 32],
    /// 새 사용자 공개키.
    pub new_pub: [u8; 32],
    /// 새 사용자의 기기 목록(제시자 자신 포함).
    pub devices: Vec<PeerId>,
    /// 폐기 기기(옛 키를 쥔 채 남은 기기 — 상대는 이 기기의 사용자 기록을 지운다).
    pub revoked: Vec<PeerId>,
    /// 버전(옛 사용자의 목록 버전 + 1 이상 · 단조).
    pub ver: u32,
    /// 옛 키 서명.
    pub sig_old: [u8; 64],
    /// 새 키 서명.
    pub sig_new: [u8; 64],
}

impl Succession {
    /// 서명 대상 — `DOM ‖ old ‖ new ‖ ver ‖ n ‖ devices ‖ m ‖ revoked`.
    #[must_use]
    pub fn signing_bytes(
        old_pub: &[u8; 32],
        new_pub: &[u8; 32],
        devices: &[PeerId],
        revoked: &[PeerId],
        ver: u32,
    ) -> Vec<u8> {
        let devices = &devices[..devices.len().min(SUCC_MAX)];
        let revoked = &revoked[..revoked.len().min(SUCC_MAX)];
        let mut v =
            Vec::with_capacity(SUCC_DOM.len() + 64 + 4 + 2 + 32 * (devices.len() + revoked.len()));
        v.extend_from_slice(SUCC_DOM);
        v.extend_from_slice(old_pub);
        v.extend_from_slice(new_pub);
        v.extend_from_slice(&ver.to_be_bytes());
        #[allow(clippy::cast_possible_truncation)]
        v.push(devices.len() as u8);
        for d in devices {
            v.extend_from_slice(d.as_bytes());
        }
        #[allow(clippy::cast_possible_truncation)]
        v.push(revoked.len() as u8);
        for d in revoked {
            v.extend_from_slice(d.as_bytes());
        }
        v
    }

    /// 서명 대상(검증용).
    #[must_use]
    pub fn to_sign(&self) -> Vec<u8> {
        Self::signing_bytes(
            &self.old_pub,
            &self.new_pub,
            &self.devices,
            &self.revoked,
            self.ver,
        )
    }

    /// 인코딩: `tag ‖ (서명 대상 − DOM) ‖ sig_old ‖ sig_new`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let body = self.to_sign();
        let mut out = Vec::with_capacity(1 + body.len() - SUCC_DOM.len() + 128);
        out.push(SUCCESSION_TAG);
        out.extend_from_slice(&body[SUCC_DOM.len()..]);
        out.extend_from_slice(&self.sig_old);
        out.extend_from_slice(&self.sig_new);
        out
    }

    /// 디코딩 — 길이 정확 · old ≠ new · 기기 1~16 · 폐기 0~16 · 중복 없음 · 폐기 ∩ 기기 = ∅.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.first() != Some(&SUCCESSION_TAG) {
            return None;
        }
        let mut p = 1usize;
        let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
            let s = bytes.get(*p..*p + n)?;
            *p += n;
            Some(s)
        };
        let old_pub: [u8; 32] = take(&mut p, 32)?.try_into().ok()?;
        let new_pub: [u8; 32] = take(&mut p, 32)?.try_into().ok()?;
        if old_pub == new_pub {
            return None;
        }
        let ver = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
        let read_list = |p: &mut usize, min: usize| -> Option<Vec<PeerId>> {
            let n = usize::from(take(p, 1)?[0]);
            if n < min || n > SUCC_MAX {
                return None;
            }
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                let d = PeerId::from_bytes(take(p, 32)?.try_into().ok()?);
                if v.contains(&d) {
                    return None;
                }
                v.push(d);
            }
            Some(v)
        };
        let devices = read_list(&mut p, 1)?;
        let revoked = read_list(&mut p, 0)?;
        if revoked.iter().any(|r| devices.contains(r)) {
            return None;
        }
        let sig_old: [u8; 64] = take(&mut p, 64)?.try_into().ok()?;
        let sig_new: [u8; 64] = take(&mut p, 64)?.try_into().ok()?;
        (p == bytes.len()).then_some(Self {
            old_pub,
            new_pub,
            devices,
            revoked,
            ver,
            sig_old,
            sig_new,
        })
    }
}

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

    fn pid(b: u8) -> PeerId {
        PeerId::from_bytes([b; 32])
    }

    #[test]
    fn succession_roundtrip_and_rules() {
        let s = Succession {
            old_pub: [1; 32],
            new_pub: [2; 32],
            devices: vec![pid(1)],
            revoked: vec![pid(2), pid(3)],
            ver: 5,
            sig_old: [7; 64],
            sig_new: [8; 64],
        };
        let e = s.encode();
        assert_eq!(Succession::decode(&e), Some(s.clone()));
        assert!(Succession::decode(&e[..e.len() - 1]).is_none());
        assert!(s.to_sign().starts_with(SUCC_DOM));
        assert!(
            Succession::decode(
                &Succession {
                    new_pub: [1; 32],
                    ..s.clone()
                }
                .encode()
            )
            .is_none(),
            "old = new 거부"
        );
        assert!(
            Succession::decode(
                &Succession {
                    revoked: vec![pid(1)],
                    ..s.clone()
                }
                .encode()
            )
            .is_none(),
            "폐기 ∩ 기기 = 거부"
        );
        assert!(
            Succession::decode(
                &Succession {
                    devices: vec![],
                    ..s
                }
                .encode()
            )
            .is_none(),
            "빈 기기 = 거부"
        );
    }

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
