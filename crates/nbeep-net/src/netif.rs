//! 네트워크 인터페이스 열거(M1-9 · DR-14 L3) — 발견을 **인터페이스별로** 내보내기 위한
//! 최소 정보(IPv4 주소·넷마스크·플래그)를 OS에서 직접 읽는다.
//!
//! 왜 필요한가: 기본 소켓은 **기본 경로 인터페이스 하나**로만 멀티캐스트·브로드캐스트를
//! 내보낸다. DHCP 없는 직결(자기 배정 169.254.x.x — FR-D-7)이나 다중 NIC에서는 그
//! 인터페이스가 상대와 닿는 링크가 아닐 수 있다 — 그래서 후보 인터페이스 전부로
//! 명시 발신한다(발견 다단 폴백 [06 §4]의 같은 결).
//!
//! **가상·터널 인터페이스는 제외**한다(FR-D-7) — VPN·컨테이너 브리지로 발견 패킷이
//! 새면 위협 모델이 다른 망에 존재가 방송된다(R-18 결). 판별은 이름 접두사 휴리스틱
//! (완전할 수 없다 — 새 접두사는 실측으로 추가).
//!
//! Unix = `getifaddrs`(libc — 이미 링크되는 시스템 라이브러리 바인딩 · 런타임 의존 0).
//! Windows = **`GetAdaptersAddresses` 직접**(08-22 M1-9 — iphlpapi 직접 링크 ·
//! WNET-1의 이웃 테이블과 같은 문법: repr(C) 부분 선언 + `offset_of` 컴파일 타임
//! 단언 · 실패 = 빈 목록 폴백 = 종전 기본 경로 발신 그대로).

use std::net::Ipv4Addr;

/// 인터페이스의 IPv4 항목 하나(같은 인터페이스에 주소가 여럿이면 여러 항목).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetIf {
    /// OS 인터페이스 이름(`en0`·`eth0` 등).
    pub name: String,
    /// 배정된 IPv4 주소(자기 배정 169.254.x.x 포함 — 링크로컬 직결이 바로 이 경우다).
    pub v4: Ipv4Addr,
    /// 넷마스크(서브넷 브로드캐스트 계산용 · 없으면 브로드캐스트 발신 생략).
    pub mask: Option<Ipv4Addr>,
    /// UP + RUNNING(케이블·링크 살아 있음).
    pub up: bool,
    /// 루프백 여부.
    pub loopback: bool,
}

impl NetIf {
    /// 이 서브넷의 지향 브로드캐스트 주소(`addr | !mask`) — S3를 인터페이스별로.
    #[must_use]
    pub fn subnet_broadcast(&self) -> Option<Ipv4Addr> {
        let mask = u32::from(self.mask?);
        if mask == 0 {
            return None; // /0은 브로드캐스트가 무의미(오폭 방지)
        }
        Some(Ipv4Addr::from(u32::from(self.v4) | !mask))
    }
}

/// 가상·터널 인터페이스 판별(이름 접두사 휴리스틱) — 발견을 태우면 안 되는 곳.
/// VPN(utun·tun·tap·wg·ppp·zt·ts) · 컨테이너/VM(docker·veth·br-·virbr·vmnet) ·
/// macOS 보조 링크(awdl·llw·bridge) 등. 대소문자 무시.
#[must_use]
pub fn is_virtual_name(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "utun",
        "tun",
        "tap",
        "wg",
        "ppp",
        "zt",
        "ts",
        "tailscale",
        "docker",
        "veth",
        "br-",
        "virbr",
        "vmnet",
        "vnic",
        "awdl",
        "llw",
        "bridge",
        "gif",
        "stf",
        "anpi",
        // Windows 친화 이름(08-22 M1-9 — GetAdaptersAddresses FriendlyName 기준).
        "vethernet", // Hyper-V/WSL 가상 스위치
        "openvpn",
        "wireguard",
        "wintun",
        "nordlynx",
        "zerotier",
        "hamachi",
    ];
    let lower = name.to_ascii_lowercase();
    PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// 발견을 태울 자격이 있는 IPv4 인터페이스 — UP·비루프백·비가상.
#[must_use]
pub fn eligible_v4() -> Vec<NetIf> {
    list_v4()
        .into_iter()
        .filter(|i| i.up && !i.loopback && !is_virtual_name(&i.name))
        .collect()
}

/// 모든 IPv4 인터페이스 항목(플래그 포함 · 필터 없음).
#[cfg(unix)]
#[must_use]
pub fn list_v4() -> Vec<NetIf> {
    let mut out = Vec::new();
    // SAFETY: getifaddrs가 채운 연결 리스트를 freeifaddrs까지만 읽는다.
    // ifa_addr/ifa_netmask는 널 검사 + sa_family 확인 후에만 sockaddr_in으로 본다.
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() {
                continue;
            }
            if i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                continue;
            }
            let sin = &*(ifa.ifa_addr.cast::<libc::sockaddr_in>());
            let v4 = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            let mask = (!ifa.ifa_netmask.is_null()
                && i32::from((*ifa.ifa_netmask).sa_family) == libc::AF_INET)
                .then(|| {
                    let m = &*(ifa.ifa_netmask.cast::<libc::sockaddr_in>());
                    Ipv4Addr::from(u32::from_be(m.sin_addr.s_addr))
                });
            let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                .to_string_lossy()
                .into_owned();
            let flags = ifa.ifa_flags;
            out.push(NetIf {
                name,
                v4,
                mask,
                up: flags & (libc::IFF_UP as libc::c_uint) != 0
                    && flags & (libc::IFF_RUNNING as libc::c_uint) != 0,
                loopback: flags & (libc::IFF_LOOPBACK as libc::c_uint) != 0,
            });
        }
        libc::freeifaddrs(ifap);
    }
    out
}

/// Windows — `GetAdaptersAddresses`(iphlpapi 직접 · 08-22 M1-9). 시스템이 채운
/// 연결 리스트를 읽기만 하며, 어느 단계든 실패 = 빈 목록(종전 폴백과 동일).
#[cfg(windows)]
#[must_use]
pub fn list_v4() -> Vec<NetIf> {
    use core::ffi::c_void;

    // IP_ADAPTER_ADDRESSES_LH — **읽는 필드까지만** 부분 선언(x64 배치).
    // 시스템이 할당한 버퍼를 포인터로 읽을 뿐 우리가 만들지 않으므로, 선언한
    // 오프셋이 실제와 일치하면 된다(offset_of 단언 — 추정 금지를 빌드가 지킨다).
    #[repr(C)]
    struct Adapter {
        length: u32,
        if_index: u32,
        next: *mut Adapter,
        adapter_name: *mut i8,
        first_unicast: *mut Unicast,
        first_anycast: *mut c_void,
        first_multicast: *mut c_void,
        first_dns: *mut c_void,
        dns_suffix: *mut u16,
        description: *mut u16,
        friendly_name: *mut u16,
        physical_address: [u8; 8],
        physical_address_length: u32,
        flags: u32,
        mtu: u32,
        if_type: u32,
        oper_status: u32,
    }
    const _: () = {
        assert!(core::mem::offset_of!(Adapter, next) == 8);
        assert!(core::mem::offset_of!(Adapter, first_unicast) == 24);
        assert!(core::mem::offset_of!(Adapter, friendly_name) == 72);
        assert!(core::mem::offset_of!(Adapter, if_type) == 100);
        assert!(core::mem::offset_of!(Adapter, oper_status) == 104);
    };

    // IP_ADAPTER_UNICAST_ADDRESS_LH — 역시 부분 선언.
    #[repr(C)]
    struct Unicast {
        length: u32,
        flags: u32,
        next: *mut Unicast,
        lp_sockaddr: *mut SockaddrIn,
        sockaddr_len: i32,
        _pad: i32,
        prefix_origin: u32,
        suffix_origin: u32,
        dad_state: u32,
        valid_lifetime: u32,
        preferred_lifetime: u32,
        lease_lifetime: u32,
        on_link_prefix_length: u8,
    }
    const _: () = {
        assert!(core::mem::offset_of!(Unicast, next) == 8);
        assert!(core::mem::offset_of!(Unicast, lp_sockaddr) == 16);
        assert!(core::mem::offset_of!(Unicast, on_link_prefix_length) == 56);
    };

    #[repr(C)]
    struct SockaddrIn {
        family: u16,
        port: u16,
        addr: [u8; 4],
        zero: [u8; 8],
    }

    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetAdaptersAddresses(
            family: u32,
            flags: u32,
            reserved: *mut c_void,
            adapter_addresses: *mut Adapter,
            size_pointer: *mut u32,
        ) -> u32;
    }

    const AF_INET: u32 = 2;
    // ANYCAST·MULTICAST·DNS 서버 목록 생략(우리는 유니캐스트 주소만 본다).
    const GAA_FLAGS: u32 = 0x0002 | 0x0004 | 0x0008;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
    const IF_OPER_STATUS_UP: u32 = 1;

    let mut out = Vec::new();
    // 크기 질의 → 할당 → 재호출(성장 재시도 2회 — 그 사이 어댑터가 늘 수 있다).
    let mut size: u32 = 16 * 1024;
    for _ in 0..3 {
        let mut buf = vec![0u8; size as usize];
        // SAFETY: 문서화된 호출 규약 — buf를 시스템이 채우고, 성공 시에만 연결
        // 리스트를 읽는다(널 검사 · AF_INET만 · 버퍼 수명 안에서만 접근).
        let rc = unsafe {
            GetAdaptersAddresses(
                AF_INET,
                GAA_FLAGS,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast::<Adapter>(),
                &mut size,
            )
        };
        if rc == ERROR_BUFFER_OVERFLOW {
            continue; // size가 필요치로 갱신됨 — 재할당
        }
        if rc != 0 {
            return out; // 실패 = 빈 목록 폴백(종전 동작)
        }
        unsafe {
            let mut a = buf.as_ptr().cast::<Adapter>();
            while !a.is_null() {
                let ad = &*a;
                let name = if ad.friendly_name.is_null() {
                    String::new()
                } else {
                    let mut len = 0usize;
                    while *ad.friendly_name.add(len) != 0 {
                        len += 1;
                    }
                    String::from_utf16_lossy(core::slice::from_raw_parts(ad.friendly_name, len))
                };
                let up = ad.oper_status == IF_OPER_STATUS_UP;
                let loopback = ad.if_type == IF_TYPE_SOFTWARE_LOOPBACK;
                let mut u = ad.first_unicast;
                while !u.is_null() {
                    let un = &*u;
                    if !un.lp_sockaddr.is_null() && (*un.lp_sockaddr).family == AF_INET as u16 {
                        let v4 = Ipv4Addr::from((*un.lp_sockaddr).addr);
                        let plen = u32::from(un.on_link_prefix_length);
                        let mask = (plen > 0 && plen <= 32)
                            .then(|| Ipv4Addr::from(u32::MAX << (32 - plen)));
                        out.push(NetIf {
                            name: name.clone(),
                            v4,
                            mask,
                            up,
                            loopback,
                        });
                    }
                    u = un.next;
                }
                a = ad.next;
            }
        }
        return out;
    }
    out
}

/// 그 외 OS — 빈 목록 폴백(호출측은 기본 경로 발신 유지).
#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn list_v4() -> Vec<NetIf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_prefixes_are_flagged() {
        for n in ["utun3", "docker0", "veth1a2b", "awdl0", "vmnet8", "TUN0"] {
            assert!(is_virtual_name(n), "{n}");
        }
        for n in ["en0", "eth0", "wlan0", "enp3s0", "Ethernet"] {
            assert!(!is_virtual_name(n), "{n}");
        }
    }

    #[test]
    fn subnet_broadcast_math() {
        let i = NetIf {
            name: "en0".into(),
            v4: Ipv4Addr::new(192, 168, 45, 84),
            mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            up: true,
            loopback: false,
        };
        assert_eq!(i.subnet_broadcast(), Some(Ipv4Addr::new(192, 168, 45, 255)));
        // 링크로컬 직결(FR-D-7) — 자기 배정 /16.
        let ll = NetIf {
            mask: Some(Ipv4Addr::new(255, 255, 0, 0)),
            v4: Ipv4Addr::new(169, 254, 12, 34),
            ..i.clone()
        };
        assert_eq!(
            ll.subnet_broadcast(),
            Some(Ipv4Addr::new(169, 254, 255, 255))
        );
        assert_eq!(NetIf { mask: None, ..i }.subnet_broadcast(), None);
    }

    /// Windows 실 호스트 열거 스모크(08-22 M1-9) — 루프백 어댑터가 보이고,
    /// 자격 목록엔 루프백이 없다. 자격 인터페이스는 실측 출력(--nocapture).
    #[cfg(windows)]
    #[test]
    fn windows_enumeration_sees_loopback_and_eligible() {
        let all = list_v4();
        assert!(
            all.iter().any(|i| i.loopback && i.v4.is_loopback()),
            "루프백 127.x가 보여야 한다: {all:?}"
        );
        let el = eligible_v4();
        assert!(el.iter().all(|i| !i.loopback && i.up));
        for i in &el {
            println!(
                "자격 IF: {} {}/{:?} bcast={:?}",
                i.name,
                i.v4,
                i.mask,
                i.subnet_broadcast()
            );
        }
    }

    /// 실 호스트 열거 스모크 — unix에선 루프백이 반드시 있다(형식·플래그 파싱 검증).
    #[cfg(unix)]
    #[test]
    fn enumeration_sees_loopback() {
        let all = list_v4();
        assert!(
            all.iter().any(|i| i.loopback && i.v4.is_loopback()),
            "루프백 127.x가 보여야 한다: {all:?}"
        );
        // 자격 목록엔 루프백·가상이 없어야 한다.
        assert!(eligible_v4().iter().all(|i| !i.loopback));
    }
}
