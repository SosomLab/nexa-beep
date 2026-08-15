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
//! - Windows = 빈 목록 폴백(`GetIpNetTable` 어댑터 = Windows 실기 몫 · WNET-1)

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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn neighbors_v4_impl() -> Vec<Ipv4Addr> {
    Vec::new() // Windows = WNET-1(GetIpNetTable) — 폴백은 조용히 빈 목록
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
}
