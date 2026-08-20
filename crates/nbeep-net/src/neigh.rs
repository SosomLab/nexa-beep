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
//! - macOS = `arp -an` 스폰(시스템 기본 도구 — 클립보드 `pbcopy` 선례 · 주기가 성겨
//!   비용 무시 가능) — sysctl `NET_RT_FLAGS` 파싱은 복잡도 대비 이득이 없어 보류
//! - Windows = iphlpapi `GetIpNetTable2` 직접 링크(WNET-1 ✅ 08-20 — 무권한 T0 ·
//!   Unreachable/Incomplete 제외 = Linux ATF_COM 필터와 같은 결)

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
    // `arp -an` 출력 예: "? (192.168.45.1) at a4:.. on en0 ifscope [ethernet]"
    // "(incomplete)" 항목도 IP는 괄호에 있으므로 함께 잡히지만, 어차피 프로브는
    // best-effort 유니캐스트라 무해하다(닿지 않으면 그만).
    let Ok(out) = std::process::Command::new("arp").arg("-an").output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|line| {
            let start = line.find('(')? + 1;
            let end = line[start..].find(')')? + start;
            line[start..end].parse().ok()
        })
        .collect()
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
