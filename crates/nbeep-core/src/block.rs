//! 수신 제한(차단) 목록 — **4축 설계**([docs/38] · M5-3 스팸 방어 1차 · BL-1 · 11-05).
//!
//! 네 축은 등가가 아니다([docs/38 §1]): **PeerId**(기기 키 · 암호학적 · v1 주축) · **UserId**(사람 · v2 자리 —
//! v1은 PeerId=UserId 1:1(DR-20 V1-1)이라 축을 두지 않는다) · **IP/CIDR**(경로 · 약함 — 새 키 재생성 우회의
//! 마찰) · **표시 이름**(가장 약함 · 정규화 후 **정확 일치만** — B-4). 정책 결합 = **OR**.
//!
//! 이 모듈은 **순수 자료 구조**다 — 판정 지점 배선(발견 recv · accept · 핸드셰이크 후 · 아웃바운드)은 호스트,
//! 봉인 영속은 `nbeep-store`(`blocks.seg`). 만료는 **조회 시점 게으른 판정**(`until_ms < now` = 통과) ·
//! 목록 정리는 [`BlockList::purge_expired`](부팅·틱) — [docs/38 §4].
//!
//! 봉투 원리: 이 목록은 "누구를 안 받는가"만 안다. 차단 이유·대화 내용은 없다(`note`는 사람이 적은 메모만).

use crate::identity::PeerId;
use std::net::IpAddr;

/// 1시간(ms).
pub const DUR_1H_MS: u64 = 60 * 60 * 1000;
/// 24시간(ms) — IP 축 기본(B-4).
pub const DUR_24H_MS: u64 = 24 * DUR_1H_MS;
/// 7일(ms).
pub const DUR_7D_MS: u64 = 7 * DUR_24H_MS;
/// 자동 임시 차단 기본 10분(B-3 · [docs/38 §5]).
pub const AUTO_TEMP_MS: u64 = 10 * 60 * 1000;

/// 차단 축 — 어느 층이 무엇으로 판정하는가([docs/38 §3]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BlockAxis {
    /// 기기 키(신원 차단 · 핸드셰이크 확정 후 판정).
    Peer(PeerId),
    /// 경로 차단 — 단일 주소는 `prefix` = 32/128. 소켓 직후(파싱 전) 판정.
    Ip { addr: IpAddr, prefix: u8 },
    /// 표시 이름(**정규화된 값** — [`normalize_name`]) · 발견 패킷 파싱 후 판정.
    Name(String),
}

/// 한 차단 항목([docs/38 §4]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEntry {
    pub axis: BlockAxis,
    /// 등재 시각(unix ms).
    pub added_ms: u64,
    /// 만료(unix ms) · `None` = 영구.
    pub until_ms: Option<u64>,
    /// 자동 임시 차단(B-3)으로 등재됐는가 — 영구 승격·해제는 사람이.
    pub auto: bool,
    /// 사람이 적은 메모(표시 전용 · 빈 문자열 = 없음).
    pub note: String,
}

impl BlockEntry {
    /// 수동 항목 — `dur_ms` = `None`이면 영구.
    #[must_use]
    pub fn manual(axis: BlockAxis, now_ms: u64, dur_ms: Option<u64>) -> Self {
        Self {
            axis,
            added_ms: now_ms,
            until_ms: dur_ms.map(|d| now_ms.saturating_add(d)),
            auto: false,
            note: String::new(),
        }
    }

    /// 자동 임시 항목(기본 [`AUTO_TEMP_MS`]).
    #[must_use]
    pub fn auto_temp(axis: BlockAxis, now_ms: u64, note: &str) -> Self {
        Self::auto_temp_for(axis, now_ms, note, AUTO_TEMP_MS)
    }

    /// 자동 임시 항목 — 기간 지정(설정 `block.auto_minutes` · M5-3 2차).
    #[must_use]
    pub fn auto_temp_for(axis: BlockAxis, now_ms: u64, note: &str, dur_ms: u64) -> Self {
        Self {
            axis,
            added_ms: now_ms,
            until_ms: Some(now_ms.saturating_add(dur_ms.max(1))),
            auto: true,
            note: note.to_owned(),
        }
    }

    /// 지금 유효한가(만료 전 또는 영구).
    #[must_use]
    pub fn active(&self, now_ms: u64) -> bool {
        self.until_ms.is_none_or(|u| u > now_ms)
    }

    /// 남은 시간(ms) · 영구 = `None` · 만료 = `Some(0)`.
    #[must_use]
    pub fn remaining_ms(&self, now_ms: u64) -> Option<u64> {
        self.until_ms.map(|u| u.saturating_sub(now_ms))
    }
}

/// 표시 이름 정규화(B-4 "정규화 후 정확 일치") — 앞뒤 공백 제거 · 연속 공백 1개로 · 제어/폭 0 문자 제거 ·
/// 유니코드 소문자. 부분 일치는 **하지 않는다**(오차단 배제).
#[must_use]
pub fn normalize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_control() || matches!(ch, '\u{200B}'..='\u{200F}' | '\u{FEFF}' | '\u{2060}') {
            continue;
        }
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        for l in ch.to_lowercase() {
            out.push(l);
        }
    }
    out
}

impl BlockAxis {
    /// `"192.168.0.7"` · `"10.0.0.0/8"` · `"fe80::1"` · `"fd00::/8"` 형태를 읽는다. 잘못된 표기 = `None`.
    #[must_use]
    pub fn parse_ip(s: &str) -> Option<Self> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (
                a.trim().parse::<IpAddr>().ok()?,
                p.trim().parse::<u8>().ok()?,
            ),
            None => {
                let a = s.parse::<IpAddr>().ok()?;
                (a, if a.is_ipv4() { 32 } else { 128 })
            }
        };
        let max = if addr.is_ipv4() { 32 } else { 128 };
        if prefix > max {
            return None;
        }
        Some(Self::Ip { addr, prefix })
    }

    /// 이름 축 — 정규화해서 담는다 · 정규화 결과가 비면 `None`.
    #[must_use]
    pub fn name(s: &str) -> Option<Self> {
        let n = normalize_name(s);
        (!n.is_empty()).then_some(Self::Name(n))
    }

    /// 이 축이 IP 축이고 `ip`를 덮는가(프리픽스 매칭 · v4↔v6 불일치 = false).
    #[must_use]
    pub fn covers_ip(&self, ip: IpAddr) -> bool {
        match (self, ip) {
            (
                Self::Ip {
                    addr: IpAddr::V4(a),
                    prefix,
                },
                IpAddr::V4(b),
            ) => {
                let mask = if *prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - u32::from(*prefix))
                };
                (u32::from(*a) & mask) == (u32::from(b) & mask)
            }
            (
                Self::Ip {
                    addr: IpAddr::V6(a),
                    prefix,
                },
                IpAddr::V6(b),
            ) => {
                let mask = if *prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - u32::from(*prefix))
                };
                (u128::from(*a) & mask) == (u128::from(b) & mask)
            }
            _ => false,
        }
    }

    /// 사람이 읽는 값(목록 표시) — Peer = 짧은 지문 · Ip = `addr[/prefix]` · Name = 그대로.
    #[must_use]
    pub fn value_text(&self) -> String {
        match self {
            Self::Peer(p) => p.short(),
            Self::Ip { addr, prefix } => {
                let full = if addr.is_ipv4() { 32 } else { 128 };
                if *prefix == full {
                    addr.to_string()
                } else {
                    format!("{addr}/{prefix}")
                }
            }
            Self::Name(n) => n.clone(),
        }
    }
}

/// 차단 목록(순수 · 영속은 `nbeep-store`). 항목 수는 수십 규모 — 선형 대조로 충분([docs/38 §4]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockList {
    entries: Vec<BlockEntry>,
}

const MAGIC: [u8; 4] = *b"NBBL";
const VER: u8 = 1;

impl BlockList {
    /// 빈 목록.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 전체 항목(등재 순).
    #[must_use]
    pub fn entries(&self) -> &[BlockEntry] {
        &self.entries
    }

    /// 같은 축 항목.
    #[must_use]
    pub fn get(&self, axis: &BlockAxis) -> Option<&BlockEntry> {
        self.entries.iter().find(|e| &e.axis == axis)
    }

    /// 등재 — 같은 축이 이미 있으면 **교체**(수동이 자동을 덮고, 기간을 다시 센다). 반환 = 바뀌었는가.
    pub fn add(&mut self, entry: BlockEntry) -> bool {
        if let Some(slot) = self.entries.iter_mut().find(|e| e.axis == entry.axis) {
            if *slot == entry {
                return false;
            }
            *slot = entry;
            return true;
        }
        self.entries.push(entry);
        true
    }

    /// 해제. 반환 = 있었는가.
    pub fn remove(&mut self, axis: &BlockAxis) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| &e.axis != axis);
        self.entries.len() != before
    }

    /// 만료 항목 정리 — 걷어낸 항목을 돌려준다(호스트가 Peer 축이면 신뢰 저장소 플래그도 푼다).
    pub fn purge_expired(&mut self, now_ms: u64) -> Vec<BlockEntry> {
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|e| !e.active(now_ms));
        self.entries = keep;
        gone
    }

    /// Peer 축 판정.
    #[must_use]
    pub fn is_peer_blocked(&self, peer: PeerId, now_ms: u64) -> bool {
        self.entries
            .iter()
            .any(|e| e.active(now_ms) && e.axis == BlockAxis::Peer(peer))
    }

    /// IP 축 판정(프리픽스 포함).
    #[must_use]
    pub fn is_ip_blocked(&self, ip: IpAddr, now_ms: u64) -> bool {
        self.entries
            .iter()
            .any(|e| e.active(now_ms) && e.axis.covers_ip(ip))
    }

    /// 이름 축 판정 — 입력을 정규화해 **정확 일치**만.
    #[must_use]
    pub fn is_name_blocked(&self, name: &str, now_ms: u64) -> bool {
        let n = normalize_name(name);
        if n.is_empty() {
            return false;
        }
        self.entries
            .iter()
            .any(|e| e.active(now_ms) && matches!(&e.axis, BlockAxis::Name(x) if *x == n))
    }

    /// 유효한 IP 규칙 스냅샷 `(addr, prefix, until)` — 전송 계층 핫 필터 동기용(소켓 스레드는 이 목록 대신
    /// 스냅샷을 본다).
    #[must_use]
    pub fn ip_rules(&self, now_ms: u64) -> Vec<(IpAddr, u8, Option<u64>)> {
        self.entries
            .iter()
            .filter(|e| e.active(now_ms))
            .filter_map(|e| match e.axis {
                BlockAxis::Ip { addr, prefix } => Some((addr, prefix, e.until_ms)),
                _ => None,
            })
            .collect()
    }

    /// 직렬화(v1) — `"NBBL"` ver u8 count u32 · 항목 = 축 태그 u8(1 peer 32B · 2 ip: fam u8 + addr + prefix u8 ·
    /// 3 name: len u16 + UTF-8) · added u64 · until u64(0 = 영구) · auto u8 · note len u16 + UTF-8. 전부 BE.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.entries.len() * 64);
        out.extend_from_slice(&MAGIC);
        out.push(VER);
        out.extend_from_slice(
            &u32::try_from(self.entries.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        for e in &self.entries {
            match &e.axis {
                BlockAxis::Peer(p) => {
                    out.push(1);
                    out.extend_from_slice(p.as_bytes());
                }
                BlockAxis::Ip { addr, prefix } => {
                    out.push(2);
                    match addr {
                        IpAddr::V4(a) => {
                            out.push(4);
                            out.extend_from_slice(&a.octets());
                        }
                        IpAddr::V6(a) => {
                            out.push(6);
                            out.extend_from_slice(&a.octets());
                        }
                    }
                    out.push(*prefix);
                }
                BlockAxis::Name(n) => {
                    out.push(3);
                    put_str(&mut out, n);
                }
            }
            out.extend_from_slice(&e.added_ms.to_be_bytes());
            out.extend_from_slice(&e.until_ms.unwrap_or(0).to_be_bytes());
            out.push(u8::from(e.auto));
            put_str(&mut out, &e.note);
        }
        out
    }

    /// 역직렬화 — 형식 오류는 `None`(fail-closed · 호출자가 잠김 처리).
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC || r.u8()? != VER {
            return None;
        }
        let count = r.u32()? as usize;
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let axis = match r.u8()? {
                1 => BlockAxis::Peer(PeerId::from_bytes(r.take(32)?.try_into().ok()?)),
                2 => {
                    let addr = match r.u8()? {
                        4 => IpAddr::V4(<[u8; 4]>::try_from(r.take(4)?).ok()?.into()),
                        6 => IpAddr::V6(<[u8; 16]>::try_from(r.take(16)?).ok()?.into()),
                        _ => return None,
                    };
                    let prefix = r.u8()?;
                    if prefix > if addr.is_ipv4() { 32 } else { 128 } {
                        return None;
                    }
                    BlockAxis::Ip { addr, prefix }
                }
                3 => BlockAxis::Name(r.str()?),
                _ => return None,
            };
            let added_ms = r.u64()?;
            let until = r.u64()?;
            let auto = r.u8()? != 0;
            let note = r.str()?;
            entries.push(BlockEntry {
                axis,
                added_ms,
                until_ms: (until != 0).then_some(until),
                auto,
                note,
            });
        }
        Some(Self { entries })
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(usize::from(u16::MAX));
    out.extend_from_slice(&u16::try_from(n).unwrap_or(u16::MAX).to_be_bytes());
    out.extend_from_slice(&b[..n]);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (h, t) = self.0.split_at(n);
        self.0 = t;
        Some(h)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_be_bytes(b.try_into().expect("4B")))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .map(|b| u64::from_be_bytes(b.try_into().expect("8B")))
    }
    fn str(&mut self) -> Option<String> {
        let n = usize::from(u16::from_be_bytes(self.take(2)?.try_into().expect("2B")));
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    #[test]
    fn name_normalization_is_exact_match_only() {
        assert_eq!(normalize_name("  Kiros\u{200B}33   Bae \n"), "kiros33 bae");
        let mut bl = BlockList::new();
        bl.add(BlockEntry::manual(
            BlockAxis::name("Spam Bot").unwrap(),
            0,
            None,
        ));
        assert!(bl.is_name_blocked("spam  BOT", 1));
        assert!(!bl.is_name_blocked("spam bot 2", 1), "부분 일치 금지");
        assert!(
            BlockAxis::name(" \u{200B} ").is_none(),
            "빈 이름은 축이 아니다"
        );
    }

    #[test]
    fn ip_prefix_matching_v4_v6() {
        let net = BlockAxis::parse_ip("10.1.0.0/16").unwrap();
        assert!(net.covers_ip("10.1.200.3".parse().unwrap()));
        assert!(!net.covers_ip("10.2.0.1".parse().unwrap()));
        assert!(
            !net.covers_ip("fe80::1".parse().unwrap()),
            "v4 규칙은 v6에 안 걸린다"
        );
        let one = BlockAxis::parse_ip("fd00::1").unwrap();
        assert!(one.covers_ip("fd00::1".parse().unwrap()));
        assert!(!one.covers_ip("fd00::2".parse().unwrap()));
        assert!(
            BlockAxis::parse_ip("10.0.0.0/33").is_none(),
            "프리픽스 범위"
        );
        assert!(BlockAxis::parse_ip("not an ip").is_none());
        assert_eq!(
            BlockAxis::parse_ip("1.2.3.4").unwrap().value_text(),
            "1.2.3.4"
        );
        assert_eq!(net.value_text(), "10.1.0.0/16");
    }

    #[test]
    fn expiry_is_lazy_and_purge_returns_gone() {
        let mut bl = BlockList::new();
        bl.add(BlockEntry::manual(
            BlockAxis::Peer(pid(1)),
            1_000,
            Some(DUR_1H_MS),
        ));
        bl.add(BlockEntry::manual(BlockAxis::Peer(pid(2)), 1_000, None));
        bl.add(BlockEntry::auto_temp(
            BlockAxis::parse_ip("192.168.0.9").unwrap(),
            1_000,
            "30/min",
        ));
        assert!(bl.is_peer_blocked(pid(1), 1_000 + DUR_1H_MS - 1));
        assert!(
            !bl.is_peer_blocked(pid(1), 1_000 + DUR_1H_MS),
            "만료 = 통과(게으른 판정)"
        );
        assert!(bl.is_peer_blocked(pid(2), u64::MAX / 2), "영구");
        assert!(bl.is_ip_blocked("192.168.0.9".parse().unwrap(), 2_000));
        let gone = bl.purge_expired(1_000 + DUR_24H_MS);
        assert_eq!(gone.len(), 2);
        assert_eq!(bl.entries().len(), 1);
        assert_eq!(bl.ip_rules(0).len(), 0);
    }

    #[test]
    fn add_replaces_same_axis_and_manual_overrides_auto() {
        let mut bl = BlockList::new();
        let ax = BlockAxis::Peer(pid(7));
        assert!(bl.add(BlockEntry::auto_temp(ax.clone(), 10, "rate")));
        assert!(bl.get(&ax).unwrap().auto);
        assert!(bl.add(BlockEntry::manual(ax.clone(), 20, None)));
        assert_eq!(bl.entries().len(), 1);
        assert!(!bl.get(&ax).unwrap().auto);
        assert!(
            !bl.add(BlockEntry::manual(ax.clone(), 20, None)),
            "동일 = 무변화"
        );
        assert!(bl.remove(&ax));
        assert!(!bl.remove(&ax), "멱등");
    }

    #[test]
    fn codec_roundtrip_and_fail_closed() {
        let mut bl = BlockList::new();
        bl.add(BlockEntry::manual(
            BlockAxis::Peer(pid(3)),
            123,
            Some(DUR_7D_MS),
        ));
        bl.add(BlockEntry::manual(
            BlockAxis::parse_ip("fd00::/8").unwrap(),
            124,
            None,
        ));
        let mut e = BlockEntry::auto_temp(BlockAxis::name("Ad Bot").unwrap(), 125, "5/s×10s");
        e.note = "메모 — 한글".into();
        bl.add(e);
        let bytes = bl.encode();
        assert_eq!(BlockList::decode(&bytes).unwrap(), bl);
        assert!(
            BlockList::decode(&bytes[..bytes.len() - 1]).is_none(),
            "잘림 = None"
        );
        let mut bad = bytes;
        bad[4] = 9;
        assert!(BlockList::decode(&bad).is_none(), "미지 버전 = None");
        assert_eq!(
            BlockList::decode(&BlockList::new().encode()).unwrap(),
            BlockList::new()
        );
    }
}
