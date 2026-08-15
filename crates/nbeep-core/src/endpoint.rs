//! 수동 엔드포인트 주소 문자열 정규화(M1-14 · DR-19).
//!
//! GUI 모달(`nbeep-ui::addr_prompt`)과 CLI(`--connect`/`--chat-connect`)가 **같은
//! 규칙**을 쓴다 — 같은 입력이 경로에 따라 갈리는 건 규약이 아니라 누락이다
//! (2026-08-13 실기: GUI는 `10.0.0.5`가 되는데 CLI는 `BadAddress`).
//!
//! 형식 검증을 해석(`to_socket_addrs`)보다 **먼저** 태우는 이유: 숫자·점뿐인 오타
//! (`10.60.218.517`)가 DNS 조회로 흘러 **오해를 주는 `Unreachable`**이 됐다(실기).
//! 반면 호스트명·DDNS는 수동 엔드포인트의 정당한 입력이라 문자 집합은 안 따진다.

/// 주소를 정규화한다 — **포트를 생략하면 `default_port`를 붙인다.**
///
/// 받는 형식 = `host` · `host:port` · `[v6]` · `[v6]:port`. 형식이 틀리면 `None`.
/// 포트를 **적었다면** 1~65535 숫자여야 한다(오타 즉시 검출).
///
/// ```
/// # use nbeep_core::endpoint::normalize_endpoint;
/// assert_eq!(normalize_endpoint("10.0.0.1", 47200).as_deref(), Some("10.0.0.1:47200"));
/// assert_eq!(normalize_endpoint("10.0.0.1:9000", 47200).as_deref(), Some("10.0.0.1:9000"));
/// assert_eq!(normalize_endpoint("[fe80::1]", 48000).as_deref(), Some("[fe80::1]:48000"));
/// assert_eq!(normalize_endpoint("beep.example.com", 47200).as_deref(), Some("beep.example.com:47200"));
/// assert_eq!(normalize_endpoint("10.60.218.517", 47200), None); // 옥텟 오타 — DNS로 흘리지 않는다
/// assert_eq!(normalize_endpoint("", 47200), None);
/// ```
#[must_use]
pub fn normalize_endpoint(s: &str, default_port: u16) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // [v6] 또는 [v6]:port — 스코프 지정(`%en0`)이 섞일 수 있어 v6 파싱은 강제하지 않는다.
    if let Some(rest) = s.strip_prefix('[') {
        if let Some((host, port)) = rest.split_once("]:") {
            return (!host.is_empty() && valid_port(port)).then(|| s.to_string());
        }
        // 포트 없는 [v6]
        let host = rest.strip_suffix(']')?;
        return (!host.is_empty()).then(|| format!("[{host}]:{default_port}"));
    }
    // 대괄호 없는 v6(':'가 둘 이상) — 포트를 붙이려면 대괄호가 필요하다.
    if s.matches(':').count() >= 2 {
        return None;
    }
    match s.rsplit_once(':') {
        // host:port
        Some((host, port)) => (plausible_host(host) && valid_port(port)).then(|| s.to_string()),
        // 포트 생략 — 기본 포트를 붙인다.
        None => plausible_host(s).then(|| format!("{s}:{default_port}")),
    }
}

/// 주소 형식 검증 — [`normalize_endpoint`]가 받아주는가(붙는 포트 값과는 무관).
#[must_use]
pub fn valid_endpoint(s: &str) -> bool {
    normalize_endpoint(s, 1).is_some()
}

/// 호스트부의 최소 검증 — **숫자·점뿐이면 IPv4 리터럴이어야 한다**(오타를 DNS로
/// 흘리지 않는다). 그 외(호스트명·DDNS)는 문자 집합을 따지지 않는다(해석기 몫).
fn plausible_host(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return host.parse::<std::net::Ipv4Addr>().is_ok();
    }
    true
}

fn valid_port(p: &str) -> bool {
    p.parse::<u32>().is_ok_and(|n| (1..=65_535).contains(&n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_literal_typos_are_rejected_not_resolved() {
        // 08-13 실기에서 실제로 밟은 오타 — DNS로 흘러 Unreachable로 오해됐다.
        assert_eq!(normalize_endpoint("10.60.218.517", 47_200), None);
        assert_eq!(normalize_endpoint("10.60.218.517:47200", 47_200), None);
        // 정상 리터럴·호스트명은 통과.
        assert!(normalize_endpoint("10.60.218.157", 47_200).is_some());
        assert!(normalize_endpoint("my-pc.local", 47_200).is_some());
    }

    #[test]
    fn port_rules_hold() {
        assert_eq!(normalize_endpoint("h:0", 1), None);
        assert_eq!(normalize_endpoint("h:65536", 1), None);
        assert_eq!(normalize_endpoint("h:80", 1).as_deref(), Some("h:80"));
        assert!(!valid_endpoint("[]"));
        assert!(valid_endpoint("[fe80::1%en0]")); // 스코프 지정 v6 관용
    }
}
