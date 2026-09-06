//! 서명 기기 목록 `UserHello`(ADR-0015 §5-3 · S2-e) — Control 스트림 태그 [`USER_HELLO_TAG`].
//!
//! 사용자 키(Ed25519)가 **"이 기기들은 내 것"** 을 서명한 공개 문서다. 세션 성립 직후 양방향(형제·타인
//! 모두). 수신측 검증(A-1 방어) = ① 서명 ② **제시한 기기 자신이 목록에 있다** ③ version 단조.
//! 남이 준 목록만으로 다른 기기를 편입하지 않는다 — 접는 것은 그 기기와의 세션이 있을 때뿐.
//!
//! 와이어: `tag ‖ user_pub(32) ‖ list_ver(u32) ‖ name_len(u8) ‖ name ‖ n(u8) ‖ PeerId×n ‖ sig(64)`.
//! 서명 대상 = [`UserHello::signing_bytes`](도메인 분리 `nbeep-user-hello-v1`).

use crate::identity::PeerId;

/// Control 스트림 태그.
pub const USER_HELLO_TAG: u8 = 4;
/// 기기 목록 상한(한 사용자의 PC 수 — 프레임 1KB 아래).
pub const USER_HELLO_MAX_DEVICES: usize = 16;
/// 핸들 상한(설정과 동일).
pub const USER_HELLO_MAX_NAME: usize = 32;
const DOM: &[u8] = b"nbeep-user-hello-v1";

/// 서명 기기 목록.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserHello {
    /// 사용자 공개키(Ed25519 32B) — `UserId = H(pub)`.
    pub user_pub: [u8; 32],
    /// 핸들(공개 라벨 · ASCII ≤32).
    pub name: String,
    /// 이 사용자의 기기 목록(제시자 자신 포함).
    pub devices: Vec<PeerId>,
    /// 목록 버전(단조 증가 — 롤백 방지).
    pub list_ver: u32,
    /// Ed25519 서명(64B) — [`Self::signing_bytes`] 위.
    pub sig: [u8; 64],
}

impl UserHello {
    /// 서명 대상 바이트 — `DOM ‖ pub ‖ ver ‖ name_len ‖ name ‖ n ‖ devices`.
    #[must_use]
    pub fn signing_bytes(
        user_pub: &[u8; 32],
        name: &str,
        devices: &[PeerId],
        list_ver: u32,
    ) -> Vec<u8> {
        let name = &name.as_bytes()[..name.len().min(USER_HELLO_MAX_NAME)];
        let devices = &devices[..devices.len().min(USER_HELLO_MAX_DEVICES)];
        let mut v =
            Vec::with_capacity(DOM.len() + 32 + 4 + 1 + name.len() + 1 + 32 * devices.len());
        v.extend_from_slice(DOM);
        v.extend_from_slice(user_pub);
        v.extend_from_slice(&list_ver.to_be_bytes());
        #[allow(clippy::cast_possible_truncation)]
        v.push(name.len() as u8);
        v.extend_from_slice(name);
        #[allow(clippy::cast_possible_truncation)]
        v.push(devices.len() as u8);
        for d in devices {
            v.extend_from_slice(d.as_bytes());
        }
        v
    }

    /// 인코딩(태그 포함) — 도메인 없이 서명 대상과 같은 배열 + 서명.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let body = Self::signing_bytes(&self.user_pub, &self.name, &self.devices, self.list_ver);
        let mut out = Vec::with_capacity(1 + body.len() - DOM.len() + 64);
        out.push(USER_HELLO_TAG);
        out.extend_from_slice(&body[DOM.len()..]);
        out.extend_from_slice(&self.sig);
        out
    }

    /// 디코딩 — 태그·길이 정확 일치 · 핸들 ASCII · 기기 1~16 · 중복 없음.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.first() != Some(&USER_HELLO_TAG) {
            return None;
        }
        let mut p = 1usize;
        let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
            let s = bytes.get(*p..*p + n)?;
            *p += n;
            Some(s)
        };
        let user_pub: [u8; 32] = take(&mut p, 32)?.try_into().ok()?;
        let list_ver = u32::from_be_bytes(take(&mut p, 4)?.try_into().ok()?);
        let nl = usize::from(take(&mut p, 1)?[0]);
        if nl > USER_HELLO_MAX_NAME {
            return None;
        }
        let name = std::str::from_utf8(take(&mut p, nl)?).ok()?;
        if !name.is_ascii() || name.chars().any(char::is_control) {
            return None;
        }
        let n = usize::from(take(&mut p, 1)?[0]);
        if n == 0 || n > USER_HELLO_MAX_DEVICES {
            return None;
        }
        let mut devices = Vec::with_capacity(n);
        for _ in 0..n {
            let d = PeerId::from_bytes(take(&mut p, 32)?.try_into().ok()?);
            if devices.contains(&d) {
                return None;
            }
            devices.push(d);
        }
        let sig: [u8; 64] = take(&mut p, 64)?.try_into().ok()?;
        (p == bytes.len()).then_some(Self {
            user_pub,
            name: name.to_string(),
            devices,
            list_ver,
            sig,
        })
    }

    /// 서명 대상(수신측 검증용).
    #[must_use]
    pub fn to_sign(&self) -> Vec<u8> {
        Self::signing_bytes(&self.user_pub, &self.name, &self.devices, self.list_ver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PeerId {
        PeerId::from_bytes([b; 32])
    }

    #[test]
    fn roundtrip_and_validation() {
        let h = UserHello {
            user_pub: [7u8; 32],
            name: "kiros33".into(),
            devices: vec![pid(1), pid(2)],
            list_ver: 3,
            sig: [9u8; 64],
        };
        let e = h.encode();
        assert_eq!(UserHello::decode(&e), Some(h.clone()));
        assert!(UserHello::decode(&e[..e.len() - 1]).is_none(), "길이 부족");
        let mut long = e.clone();
        long.push(0);
        assert!(UserHello::decode(&long).is_none(), "길이 초과");
        // 서명 대상은 도메인이 앞에 붙고, 버전이 다르면 다르다(롤백 방지 재료).
        let a = h.to_sign();
        assert!(a.starts_with(DOM));
        let b = UserHello::signing_bytes(&h.user_pub, &h.name, &h.devices, 4);
        assert_ne!(a, b);
        // 중복 기기 = 거부.
        let dup = UserHello {
            devices: vec![pid(1), pid(1)],
            ..h
        };
        assert!(UserHello::decode(&dup.encode()).is_none());
        // 빈 목록 = 거부(제시자 자신은 반드시 있어야 한다).
        let mut empty = e;
        empty[1 + 32 + 4 + 7] = 0; // n
        assert!(UserHello::decode(&empty).is_none());
    }
}
