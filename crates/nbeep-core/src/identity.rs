//! 신원 — 기기 지문(`PeerId`) · 다중 기기 사용자(`UserId`) · 신뢰 등급.
//!
//! - **`PeerId` = 기기의 정적 공개키(또는 그 지문).** 전송과 무관한 유일 기기 신원(DR-8·DR-18).
//!   공개키이므로 **비밀이 아니다** — 로그·UI 표시 가능.
//! - **`UserId` = 다중 기기를 묶는 사용자 장기 키**([docs/20] ADR-0007). **v1은 `PeerId`와 1:1**(DR-20 V1-1) —
//!   타입 자리만 세워 두고(D-21) 구현은 v2. 여기서는 대화·기록·차단이 붙을 **자리**를 확보한다.
//! - **발신 대상은 언제나 `Recipients`(PeerId 집합)** — 1:1·그룹·UserId가 같은 표현으로 수렴(D-21).

use core::fmt;

/// 32바이트 지문 값의 공통 구현(공개키 기반 — 비밀 아님).
macro_rules! fingerprint_newtype {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name([u8; 32]);

        impl $name {
            /// 지문 길이(바이트).
            pub const LEN: usize = 32;

            /// 원시 바이트로부터 구성.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// 원시 바이트 참조.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// 짧은 지문(앞 4바이트 hex) — 로그·목록 표시용.
            #[must_use]
            pub fn short(&self) -> String {
                let b = &self.0;
                format!("{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3])
            }
        }

        // 32바이트 배열을 통째로 덤프하지 않는다(로그 가독성).
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({}…)"), self.short())
            }
        }
    };
}

fingerprint_newtype! {
    /// 기기 신원 = 정적 공개키 지문(DR-8·DR-18). 계정·IP·MAC은 신원이 아니다.
    PeerId
}

fingerprint_newtype! {
    /// 다중 기기 사용자 신원([docs/20] ADR-0007). **v1은 `PeerId`와 1:1**(D-21 타입 자리).
    UserId
}

/// 기기 식별자 = 그 기기의 `PeerId`(DR-18 — PC 1대 = 노드 1개).
pub type DeviceId = PeerId;

/// 상대의 신뢰 상태([docs/08] TOFU·SAS). UI 배지·정책 마찰의 근거.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrustLevel {
    /// 발견됐으나 핸드셰이크로 확정되지 않음(발견 패킷은 "미검증 힌트").
    Unverified,
    /// TOFU로 공개키를 고정함(최초 접촉 성립).
    Pinned,
    /// 지문(SAS) 대조까지 완료.
    FingerprintVerified,
}

/// 발신 대상 — **1:1·그룹·UserId가 전부 하나의 표현으로 수렴**한다(D-21).
///
/// 1:1은 원소 1개, 그룹은 구성원 집합, UserId는 그 사용자의 기기 집합.
/// 발신 코드는 이 타입 하나만 받으면 세 경우를 모두 처리한다([docs/09] 팬아웃 재사용).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recipients {
    peers: Vec<PeerId>,
}

impl Recipients {
    /// 빈 대상.
    #[must_use]
    pub const fn empty() -> Self {
        Self { peers: Vec::new() }
    }

    /// 1:1 — 원소 하나짜리 집합.
    #[must_use]
    pub fn one(peer: PeerId) -> Self {
        Self { peers: vec![peer] }
    }

    /// 기기 집합으로부터 구성(그룹·UserId). 중복은 제거하고 정렬해 **결정적**으로 만든다.
    #[must_use]
    pub fn from_peers(mut peers: Vec<PeerId>) -> Self {
        peers.sort_unstable();
        peers.dedup();
        Self { peers }
    }

    /// 대상 기기 목록.
    #[must_use]
    pub fn peers(&self) -> &[PeerId] {
        &self.peers
    }

    /// 대상 수(팬아웃 폭).
    #[must_use]
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// 대상이 없는가.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(first: u8) -> PeerId {
        let mut b = [0u8; 32];
        b[0] = first;
        PeerId::from_bytes(b)
    }

    #[test]
    fn short_is_first_four_bytes_hex() {
        let p = PeerId::from_bytes([
            0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0,
        ]);
        assert_eq!(p.short(), "deadbeef");
    }

    #[test]
    fn debug_does_not_dump_full_array() {
        let s = format!("{:?}", pid(0xab));
        assert!(s.contains("ab000000"));
        assert!(!s.contains('['), "raw byte array must not appear: {s}");
    }

    #[test]
    fn one_is_singleton() {
        let r = Recipients::one(pid(1));
        assert_eq!(r.len(), 1);
        assert!(!r.is_empty());
    }

    #[test]
    fn from_peers_dedups_and_is_deterministic() {
        let a = Recipients::from_peers(vec![pid(3), pid(1), pid(3), pid(2)]);
        let b = Recipients::from_peers(vec![pid(2), pid(3), pid(1), pid(1)]);
        assert_eq!(a.len(), 3, "중복 제거");
        assert_eq!(a, b, "정렬로 결정적");
    }

    #[test]
    fn empty_recipients() {
        assert!(Recipients::empty().is_empty());
        assert!(Recipients::default().is_empty());
    }

    #[test]
    fn peer_and_user_are_distinct_types() {
        // 같은 바이트라도 타입이 다르면 섞이지 않는다(신원 혼동 방지).
        let same = [7u8; 32];
        let _p = PeerId::from_bytes(same);
        let _u = UserId::from_bytes(same);
        // 컴파일되는 것 자체가 확인 — PeerId와 UserId는 == 비교 불가.
    }
}
