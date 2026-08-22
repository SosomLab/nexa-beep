//! **경로 등급**(DR-28 · ADR-0006 §5-1 · M5-3c 08-21) — 신뢰 2축 중 **경로 축**.
//!
//! *신원 신뢰는 키에, 경로 등급은 경로에.* 이 값은 **성립한 세션의 실소켓 주소**로만
//! 정한다(광고 금지 — 가짜 LAN 광고로 원격 제약을 풀어내는 우회 차단 · §5-1-5).
//! 정책은 두 축의 곱(§5-1-3): 원격 × 미대조 = 파일 차단·요청 대기, 원격 ×
//! 지문 대조 = 파일 허용 + "인터넷 경유" 표시 유지.
//!
//! v1 값은 둘 — `Relay`는 v2에서 뒤에 append(값 의미 불변 규약).

use std::net::IpAddr;

/// 경로 등급 — 세션이 탄 통로의 위험 분류.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathClass {
    /// 같은 링크·사설망(LAN·링크로컬·루프백) — 종전 암묵 신뢰 경계 안.
    #[default]
    Local,
    /// 인터넷 경유(공인 주소 — ADR-0006의 `Manual` 축) — 위협 모델이 다르다.
    Remote,
}

/// 실소켓 상대 주소 → 경로 등급. **판정은 주소 대역뿐**(호스트명·광고 불신).
///
/// Local = 루프백 · IPv4 사설(10/8 · 172.16/12 · 192.168/16) · 링크로컬(169.254/16 ·
/// fe80::/10) · IPv6 ULA(fc00::/7). 그 외 전부 Remote(모르면 위험한 쪽 — fail-closed).
#[must_use]
pub fn class_of_ip(ip: IpAddr) -> PathClass {
    let local = match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // 링크로컬 fe80::/10
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
                || v6.to_ipv4_mapped().is_some_and(|m| {
                    m.is_loopback() || m.is_private() || m.is_link_local()
                })
        }
    };
    if local {
        PathClass::Local
    } else {
        PathClass::Remote
    }
}

/// §5-1-3 정책 곱 — **파일 전송 허용 여부**(발신·수신 공통 · FR-S-24).
///
/// Local 경로는 종전 그대로(신뢰 게이트는 승인 정책이 따로 본다). Remote 경로는
/// **지문(SAS) 대조 완료만** 허용 — TOFU 핀은 원격에서 "그 주소의 누군가"일 뿐이라
/// 부족하다(ADR-0006: LAN 밖은 위협 모델이 다르다). 메시지는 이 게이트와 무관.
#[must_use]
/// `remote_opt_in` = 설정 `xfer.remote_files`(08-22 사용자 확정 — 기본 끄기):
/// 원격 경로 **발신** 옵트인. 켜면 지문 대조 전에도 발신이 열린다. ⚠ 이 함수는
/// 08-22 개정 후 **발신 게이트 전용** — 수신은 차단하지 않고 항상 승인 창이
/// 경로를 표시하며 사람이 결정한다(제한은 발신자 쪽에만).
pub fn file_allowed(path: PathClass, trust: crate::TrustLevel, remote_opt_in: bool) -> bool {
    path == PathClass::Local || trust == crate::TrustLevel::FingerprintVerified || remote_opt_in
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    /// §5-1-3 곱 행렬 전수 — 원격은 지문 대조만 통과(fail-closed).
    #[test]
    fn file_matrix_remote_requires_verified() {
        use crate::TrustLevel as T;
        for t in [T::Unverified, T::Pinned, T::FingerprintVerified] {
            assert!(
                file_allowed(PathClass::Local, t, false),
                "Local은 신뢰 무관 통과"
            );
        }
        assert!(!file_allowed(PathClass::Remote, T::Unverified, false));
        assert!(
            !file_allowed(PathClass::Remote, T::Pinned, false),
            "핀만으로는 부족"
        );
        assert!(file_allowed(
            PathClass::Remote,
            T::FingerprintVerified,
            false
        ));
        // 08-22 — 발신 옵트인은 원격도 연다(수신은 이 함수를 더는 안 탄다).
        assert!(file_allowed(PathClass::Remote, T::Pinned, true));
        assert!(file_allowed(PathClass::Remote, T::Unverified, true));
    }

    #[test]
    fn private_and_linklocal_are_local() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "172.31.255.1",
            "192.168.45.84",
            "169.254.12.34",
        ] {
            assert_eq!(class_of_ip(ip.parse().unwrap()), PathClass::Local, "{ip}");
        }
        assert_eq!(
            class_of_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            PathClass::Local
        );
        assert_eq!(
            class_of_ip("fe80::1".parse().unwrap()),
            PathClass::Local,
            "v6 링크로컬"
        );
        assert_eq!(
            class_of_ip("fd12:3456::1".parse().unwrap()),
            PathClass::Local,
            "ULA"
        );
    }

    #[test]
    fn public_is_remote_fail_closed() {
        for ip in ["203.0.113.10", "8.8.8.8", "2001:db8::1", "172.32.0.1"] {
            assert_eq!(class_of_ip(ip.parse().unwrap()), PathClass::Remote, "{ip}");
        }
        // v4-mapped v6도 안쪽 판정을 따른다.
        let mapped: IpAddr = Ipv4Addr::new(192, 168, 1, 1).to_ipv6_mapped().into();
        assert_eq!(class_of_ip(mapped), PathClass::Local);
    }
}
