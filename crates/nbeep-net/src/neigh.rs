//! OS 이웃 테이블 읽기(S4 · M1-8 · DR-14 L2) — 멀티캐스트·브로드캐스트가 **차단된 망**
//! (기업 무선 클라이언트 격리·VLAN — [06 §4])에서 쓸 마지막 로컬 폴백의 재료.
//!
//! 원리: 같은 링크에서 통신한 적 있는 상대는 OS의 ARP/NDP 이웃 테이블에 남는다.
//! 그 주소들로 **1:1 유니캐스트 HELLO**를 보내면 멀티캐스트가 막혀 있어도 발견이
//! 성립한다(수신측은 유니캐스트 Announce로 응답 — `udp.rs`).
//!
//! 봉투 원리: 여기서 읽는 건 **주소뿐**이다(MAC·호스트명 등은 버린다).
//!
//! - Linux = `/proc/net/arp`(파일 읽기 · 의존 0)
//! - macOS = `sysctl(NET_RT_FLAGS, RTF_LLINFO)` 직접 읽기(09-10 — 종전 `arp -an` 스폰
//!   폐지: EDR은 exec 이벤트를 전부 기록하고 `arp -a`는 T1016/T1018 발견 룰의 문자
//!   그대로라, 13초마다 스폰하면 "관제 소음을 만드는 앱"이 된다 · 프로세스 0 =
//!   Linux·Windows와 같은 결) — 파서는 순수 함수라 실기 없이 배치 회귀를 박제한다
//! - Windows = iphlpapi `GetIpNetTable2` 직접 링크(WNET-1 ✅ 08-20 — 무권한 T0 ·
//!   Unreachable/Incomplete 제외 = Linux ATF_COM 필터와 같은 결)
//!
//! 세 OS 공통 필터 = **완성 항목만**(Linux ATF_COM · mac `sdl_alen>0` · Win state≥Probe)
//! 그리고 **MAC ff:…:ff 제외**(서브넷 지향 브로드캐스트 잡음 · mac/Win). MAC은
//! 판정에만 쓰고 저장하지 않는다.

use std::net::Ipv4Addr;

/// 이웃 테이블의 IPv4 주소들(중복 제거 · 순서 불보증). 실패는 빈 목록(best-effort).
#[must_use]
pub fn neighbors_v4() -> Vec<Ipv4Addr> {
    let mut v = neighbors_v4_impl();
    v.sort_unstable();
    v.dedup();
    // 멀티캐스트·브로드캐스트·미지정은 이웃이 아니다(테이블에 낀 잡음 방어).
    v.retain(|a| !a.is_multicast() && !a.is_broadcast() && !a.is_unspecified());
    v
}

#[cfg(target_os = "linux")]
fn neighbors_v4_impl() -> Vec<Ipv4Addr> {
    // /proc/net/arp: "IP address  HW type  Flags  HW address  Mask  Device"
    // Flags 0x2 = ATF_COM(완성 항목) — 미해결(0x0) 항목은 제외.
    let Ok(s) = std::fs::read_to_string("/proc/net/arp") else {
        return Vec::new();
    };
    s.lines()
        .skip(1)
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let ip = it.next()?.parse().ok()?;
            let _hw = it.next()?;
            let flags = it.next()?;
            u32::from_str_radix(flags.trim_start_matches("0x"), 16)
                .ok()
                .filter(|f| f & 0x2 != 0)
                .map(|_| ip)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn neighbors_v4_impl() -> Vec<Ipv4Addr> {
    rtflags_dump()
        .map(|b| parse_rt_llinfo(&b))
        .unwrap_or_default()
}

/// PF_ROUTE `NET_RT_FLAGS`/`RTF_LLINFO` 덤프(= `arp -an`이 읽는 바로 그 표) —
/// `sysctl` 2단(크기 조회 → 읽기 · 사이 성장은 ENOMEM 재시도 3회). 실패 = None.
#[cfg(target_os = "macos")]
fn rtflags_dump() -> Option<Vec<u8>> {
    // 배치 박제 — libc 정의가 파서 상수와 어긋나면 컴파일이 선다(추정 금지).
    const _: () = assert!(std::mem::size_of::<libc::rt_msghdr>() == rtdump::HDR_LEN);
    const _: () = assert!(std::mem::offset_of!(libc::rt_msghdr, rtm_flags) == rtdump::OFF_FLAGS);
    const _: () = assert!(std::mem::offset_of!(libc::rt_msghdr, rtm_addrs) == rtdump::OFF_ADDRS);
    const _: () = assert!(libc::RTM_VERSION == rtdump::RTM_VERSION as libc::c_int);
    const _: () = assert!(libc::RTF_LLINFO == rtdump::RTF_LLINFO as libc::c_int);

    let mut mib = [
        libc::CTL_NET,
        libc::PF_ROUTE,
        0,
        libc::AF_INET,
        libc::NET_RT_FLAGS,
        libc::RTF_LLINFO,
    ];
    let n = mib.len() as libc::c_uint;
    for _ in 0..4 {
        let mut len: libc::size_t = 0;
        // SAFETY: oldp=NULL은 크기 조회. mib는 유효한 배열, newp 없음.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                n,
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return None;
        }
        if len == 0 {
            return Some(Vec::new());
        }
        // 조회와 읽기 사이에 표가 자랄 수 있다(arp.c 선례) — 여유를 두고 읽는다.
        let mut buf = vec![0u8; len + len / 2 + 256];
        let mut cap: libc::size_t = buf.len();
        // SAFETY: buf는 cap 바이트의 쓰기 가능한 메모리. 커널이 cap을 실제 길이로 갱신.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                n,
                buf.as_mut_ptr().cast(),
                &mut cap,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 {
            buf.truncate(cap);
            return Some(buf);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOMEM) {
            return None;
        }
    }
    None
}

/// `rt_msghdr` + sockaddr 사슬 배치 상수(macOS · x86_64/arm64 동일). 파서가 OS 무관하게
/// 컴파일되도록 libc 타입 대신 수치로 둔다 — 실제 값은 macOS 빌드에서 libc와 대조한다.
#[cfg(any(target_os = "macos", test))]
mod rtdump {
    /// `sizeof(struct rt_msghdr)` = 36 + `rt_metrics` 56.
    pub(super) const HDR_LEN: usize = 92;
    pub(super) const OFF_MSGLEN: usize = 0;
    pub(super) const OFF_VERSION: usize = 2;
    pub(super) const OFF_FLAGS: usize = 8;
    pub(super) const OFF_ADDRS: usize = 12;
    pub(super) const RTM_VERSION: u8 = 5;
    pub(super) const RTF_LLINFO: u32 = 0x400;
    /// `rtm_addrs` 비트 — DST가 있어야 첫 sockaddr가 목적지다.
    pub(super) const RTA_DST: u32 = 0x1;
    pub(super) const RTA_GATEWAY: u32 = 0x2;
    pub(super) const AF_INET: u8 = 2;
    pub(super) const AF_LINK: u8 = 18;
    /// sockaddr_dl: `sdl_nlen`/`sdl_alen` 오프셋 · 링크 주소 시작 = 8 + `sdl_nlen`.
    pub(super) const OFF_SDL_NLEN: usize = 5;
    pub(super) const OFF_SDL_ALEN: usize = 6;
    pub(super) const SDL_DATA: usize = 8;

    /// BSD `ROUNDUP(sa_len)` — 4바이트 정렬 · 0은 4(빈 sockaddr 자리).
    pub(super) const fn roundup(len: usize) -> usize {
        if len == 0 {
            4
        } else {
            (len + 3) & !3
        }
    }
}

/// `NET_RT_FLAGS` 덤프 → 완성된 IPv4 이웃(순수 함수 · fail-closed: 길이가 안 맞는 메시지는
/// 그 자리에서 멈춘다). 조건 = 버전 5 · `RTF_LLINFO` · DST=AF_INET · GATEWAY=AF_LINK이고
/// `sdl_alen>0`(미완성 제외) · MAC 전부 0xff 제외(지향 브로드캐스트 잡음 — Windows와 동형).
#[cfg(any(target_os = "macos", test))]
fn parse_rt_llinfo(buf: &[u8]) -> Vec<Ipv4Addr> {
    use rtdump::{
        roundup, AF_INET, AF_LINK, HDR_LEN, OFF_ADDRS, OFF_FLAGS, OFF_MSGLEN, OFF_SDL_ALEN,
        OFF_SDL_NLEN, OFF_VERSION, RTA_DST, RTA_GATEWAY, RTF_LLINFO, RTM_VERSION, SDL_DATA,
    };
    let u32_at = |m: &[u8], o: usize| u32::from_ne_bytes([m[o], m[o + 1], m[o + 2], m[o + 3]]);
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + HDR_LEN <= buf.len() {
        let m = &buf[off..];
        let msglen = usize::from(u16::from_ne_bytes([m[OFF_MSGLEN], m[OFF_MSGLEN + 1]]));
        if msglen < HDR_LEN || off + msglen > buf.len() {
            break; // 손상·잘림 — 이후는 믿지 않는다
        }
        let msg = &m[..msglen];
        let flags = u32_at(m, OFF_FLAGS);
        let addrs = u32_at(m, OFF_ADDRS);
        off += msglen;
        if m[OFF_VERSION] != RTM_VERSION || flags & RTF_LLINFO == 0 || addrs & RTA_DST == 0 {
            continue;
        }
        // 첫 sockaddr = DST(sockaddr_inarp — 앞 8바이트는 sockaddr_in과 같다).
        let dst = &msg[HDR_LEN..];
        if dst.len() < 8 || dst[1] != AF_INET {
            continue;
        }
        let ip = Ipv4Addr::new(dst[4], dst[5], dst[6], dst[7]);
        // 둘째 sockaddr = GATEWAY(sockaddr_dl) — 있어야 완성 여부를 판정할 수 있다.
        if addrs & RTA_GATEWAY == 0 {
            continue;
        }
        let gw_off = roundup(usize::from(dst[0]));
        let Some(gw) = dst.get(gw_off..) else {
            continue;
        };
        if gw.len() <= OFF_SDL_ALEN || gw[1] != AF_LINK {
            continue;
        }
        let alen = usize::from(gw[OFF_SDL_ALEN]);
        if alen == 0 {
            continue; // (incomplete)
        }
        let mac_off = SDL_DATA + usize::from(gw[OFF_SDL_NLEN]);
        if let Some(mac) = gw.get(mac_off..mac_off + alen) {
            if alen == 6 && mac.iter().all(|b| *b == 0xff) {
                continue; // x.x.x.255 지향 브로드캐스트 항목
            }
        }
        out.push(ip);
    }
    out
}

#[cfg(windows)]
fn neighbors_v4_impl() -> Vec<Ipv4Addr> {
    // WNET-1(08-20) — iphlpapi `GetIpNetTable2`(문서 06 §7-2 "이웃 테이블 조회는 T0").
    // 직접 링크 = 의존 0 규약(launch.rs kernel32·autostart.rs advapi32와 동형).
    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetIpNetTable2(family: u16, table: *mut *mut MibIpnetTable2) -> u32;
        fn FreeMibTable(memory: *mut core::ffi::c_void);
    }
    /// `SOCKADDR_INET` — AF_INET이면 sockaddr_in 배치(family·port·addr4). 총 28B.
    #[repr(C)]
    struct SockaddrInet {
        family: u16,
        port: u16,
        addr: [u8; 4],
        rest: [u8; 20],
    }
    /// `MIB_IPNET_ROW2` — 주소·상태만 쓴다(봉투 원리 — MAC은 읽지 않는다).
    /// 배치는 SDK 정의 그대로(x64 88B — 아래 컴파일 타임 단언으로 박제).
    #[repr(C)]
    struct MibIpnetRow2 {
        address: SockaddrInet,        // 0..28
        if_index: u32,                // 28..32
        interface_luid: u64,          // 32..40 (8 정렬)
        physical_address: [u8; 32],   // 40..72
        physical_address_length: u32, // 72..76
        state: u32,                   // 76..80 (NL_NEIGHBOR_STATE)
        flags: u8,                    // 80
        _pad: [u8; 3],
        reachability: u32, // 84..88
    }
    /// `MIB_IPNET_TABLE2` 머리 — 행 배열은 8바이트 정렬로 offset 8부터.
    #[repr(C)]
    struct MibIpnetTable2 {
        num_entries: u32,
        _pad: u32,
        first_row: MibIpnetRow2, // ANY_SIZE 배열의 첫 칸(엔트리 0개면 접근 금지)
    }
    // 배치가 어긋나면 여기서 컴파일이 선다(추정 금지 — SDK 크기 88B 박제).
    const _: () = assert!(std::mem::size_of::<MibIpnetRow2>() == 88);
    const _: () = assert!(std::mem::offset_of!(MibIpnetTable2, first_row) == 8);

    const AF_INET: u16 = 2;
    // NL_NEIGHBOR_STATE — 0 Unreachable · 1 Incomplete는 이웃이 아니다(Linux의
    // ATF_COM 필터와 같은 결). 2 Probe~6 Permanent = 통신 흔적이 있는 항목.
    const STATE_PROBE: u32 = 2;

    let mut table: *mut MibIpnetTable2 = std::ptr::null_mut();
    // SAFETY: 출력 포인터를 넘기고, 성공 시 반드시 FreeMibTable로 되돌린다.
    let rc = unsafe { GetIpNetTable2(AF_INET, &mut table) };
    if rc != 0 || table.is_null() {
        return Vec::new(); // ERROR_NOT_FOUND(빈 테이블) 포함 — best-effort
    }
    let mut out = Vec::new();
    // SAFETY: 성공 반환이므로 table은 유효한 MIB_IPNET_TABLE2. 행은 offset 8부터
    // num_entries개가 88B 간격으로 이어진다(위 단언이 배치를 지킨다).
    unsafe {
        let n = (*table).num_entries as usize;
        let rows = std::ptr::addr_of!((*table).first_row);
        for i in 0..n {
            let row = &*rows.add(i);
            if row.address.family != AF_INET || row.state < STATE_PROBE {
                continue;
            }
            // 서브넷 지향 브로드캐스트(x.x.x.255류) — Windows 테이블에 Permanent로
            // 상주하는 잡음. `is_broadcast()`는 255.255.255.255만 잡고 마지막 옥텟
            // 검사도 마스크 의존이라, **MAC ff:…:ff**로 거른다(마스크 무관 · 정확 ·
            // 실측 08-20: 이 필터 없이는 .255 3건이 섞였다). MAC은 판정에만 쓰고
            // 저장하지 않는다(봉투 원리 유지).
            if row.physical_address_length == 6 && row.physical_address[..6] == [0xff; 6] {
                continue;
            }
            out.push(Ipv4Addr::from(row.address.addr));
        }
        FreeMibTable(table.cast());
    }
    out
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn neighbors_v4_impl() -> Vec<Ipv4Addr> {
    Vec::new() // 미지 타깃 — 조용히 빈 목록(best-effort)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtdump::{
        roundup, AF_INET, AF_LINK, HDR_LEN, OFF_ADDRS, OFF_FLAGS, OFF_MSGLEN, OFF_SDL_ALEN,
        OFF_SDL_NLEN, OFF_VERSION, RTA_DST, RTA_GATEWAY, RTF_LLINFO, RTM_VERSION, SDL_DATA,
    };

    /// 합성 `rt_msghdr`+sockaddr_inarp+sockaddr_dl 한 건(macOS 배치 · 네이티브 엔디안).
    #[allow(clippy::cast_possible_truncation)]
    fn rt_entry(ip: [u8; 4], mac: Option<&[u8]>, version: u8, flags: u32, family: u8) -> Vec<u8> {
        let mut sin = vec![0u8; 16];
        sin[0] = 16;
        sin[1] = family;
        sin[4..8].copy_from_slice(&ip);
        // sockaddr_dl: len · AF_LINK · index(2) · type · nlen · alen · slen · data
        let nlen = 3usize; // "en0"
        let alen = mac.map_or(0, <[u8]>::len);
        let mut sdl = vec![0u8; SDL_DATA + nlen + alen];
        sdl[0] = sdl.len() as u8;
        sdl[1] = AF_LINK;
        sdl[OFF_SDL_NLEN] = nlen as u8;
        sdl[OFF_SDL_ALEN] = alen as u8;
        sdl[SDL_DATA..SDL_DATA + nlen].copy_from_slice(b"en0");
        if let Some(m) = mac {
            sdl[SDL_DATA + nlen..].copy_from_slice(m);
        }
        let msglen = HDR_LEN + roundup(sin.len()) + roundup(sdl.len());
        let mut v = vec![0u8; msglen];
        v[OFF_MSGLEN..OFF_MSGLEN + 2].copy_from_slice(&(msglen as u16).to_ne_bytes());
        v[OFF_VERSION] = version;
        v[OFF_FLAGS..OFF_FLAGS + 4].copy_from_slice(&flags.to_ne_bytes());
        v[OFF_ADDRS..OFF_ADDRS + 4].copy_from_slice(&(RTA_DST | RTA_GATEWAY).to_ne_bytes());
        v[HDR_LEN..HDR_LEN + 16].copy_from_slice(&sin);
        let so = HDR_LEN + roundup(sin.len());
        v[so..so + sdl.len()].copy_from_slice(&sdl);
        v
    }

    const MAC_A: [u8; 6] = [0xa4, 0x83, 0xe7, 0x11, 0x22, 0x33];

    /// 완성 2건 + 미완성 1건 + 지향 브로드캐스트 1건이 섞인 덤프 → 완성 단말 2건만.
    #[test]
    fn rt_llinfo_parses_complete_entries_only() {
        let mut buf = Vec::new();
        buf.extend(rt_entry(
            [192, 168, 45, 1],
            Some(&MAC_A),
            RTM_VERSION,
            RTF_LLINFO,
            AF_INET,
        ));
        buf.extend(rt_entry(
            [192, 168, 45, 7],
            None,
            RTM_VERSION,
            RTF_LLINFO,
            AF_INET,
        ));
        buf.extend(rt_entry(
            [192, 168, 45, 255],
            Some(&[0xff; 6]),
            RTM_VERSION,
            RTF_LLINFO,
            AF_INET,
        ));
        buf.extend(rt_entry(
            [192, 168, 45, 9],
            Some(&MAC_A),
            RTM_VERSION,
            RTF_LLINFO,
            AF_INET,
        ));
        let got = parse_rt_llinfo(&buf);
        assert_eq!(
            got,
            vec![
                Ipv4Addr::new(192, 168, 45, 1),
                Ipv4Addr::new(192, 168, 45, 9)
            ]
        );
    }

    /// 버전·플래그·주소족이 어긋난 메시지는 건너뛴다(다른 메시지는 계속 읽는다).
    #[test]
    fn rt_llinfo_skips_foreign_messages() {
        let mut buf = Vec::new();
        buf.extend(rt_entry(
            [10, 0, 0, 1],
            Some(&MAC_A),
            4,
            RTF_LLINFO,
            AF_INET,
        )); // 옛 버전
        buf.extend(rt_entry(
            [10, 0, 0, 2],
            Some(&MAC_A),
            RTM_VERSION,
            0x1,
            AF_INET,
        )); // LLINFO 아님
        buf.extend(rt_entry(
            [10, 0, 0, 3],
            Some(&MAC_A),
            RTM_VERSION,
            RTF_LLINFO,
            30,
        )); // AF_INET6
        buf.extend(rt_entry(
            [10, 0, 0, 4],
            Some(&MAC_A),
            RTM_VERSION,
            RTF_LLINFO,
            AF_INET,
        ));
        assert_eq!(parse_rt_llinfo(&buf), vec![Ipv4Addr::new(10, 0, 0, 4)]);
    }

    /// 잘린 덤프·거짓 길이는 그 자리에서 멈춘다(패닉 없음 · 앞 항목은 보존).
    #[test]
    fn rt_llinfo_truncation_is_fail_closed() {
        let good = rt_entry(
            [10, 0, 0, 4],
            Some(&MAC_A),
            RTM_VERSION,
            RTF_LLINFO,
            AF_INET,
        );
        let mut buf = good.clone();
        let mut cut = rt_entry(
            [10, 0, 0, 5],
            Some(&MAC_A),
            RTM_VERSION,
            RTF_LLINFO,
            AF_INET,
        );
        cut.truncate(cut.len() - 5);
        buf.extend(cut);
        assert_eq!(parse_rt_llinfo(&buf), vec![Ipv4Addr::new(10, 0, 0, 4)]);
        // msglen이 헤더보다 작은 거짓 메시지 · 빈 입력
        let mut bogus = good;
        bogus[OFF_MSGLEN..OFF_MSGLEN + 2].copy_from_slice(&8u16.to_ne_bytes());
        assert!(parse_rt_llinfo(&bogus).is_empty());
        assert!(parse_rt_llinfo(&[]).is_empty());
    }

    /// 실호스트 스모크 — 파싱이 형식을 깨지 않고, 잡음(멀티캐스트 등)이 걸러진다.
    /// 이웃이 하나도 없는 호스트도 있으므로 개수는 단언하지 않는다(형식만).
    #[test]
    fn neighbors_are_clean() {
        for a in neighbors_v4() {
            assert!(!a.is_multicast() && !a.is_broadcast() && !a.is_unspecified());
        }
    }

    /// 실측 보조(--nocapture로 관찰) — LAN 호스트라면 보통 1개 이상이지만
    /// 이웃 0인 호스트(격리·콜드 테이블)도 정상이라 개수는 단언하지 않는다.
    #[test]
    fn neighbors_count_observable() {
        let v = neighbors_v4();
        println!("neighbors_v4 = {}개: {:?}", v.len(), v);
    }
}
