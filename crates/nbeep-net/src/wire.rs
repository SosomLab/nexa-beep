//! 발견 와이어 포맷 — **M1-3**([docs/08 §2] ADR-0002 · D-22 확정 08-08).
//!
//! ```text
//! [magic 4B "NXBP"][ver 1B][type 1B][flags 2B][static_pubkey 32B]
//! [tcp_port 2B][epoch 8B][seq 4B][instance 16B][name_len 1B][name UTF-8 ≤255B]
//! ```
//!
//! - **총 512B 이하 강제**([docs/08 §2] — IPv6 최소 MTU 여유·단편화 회피). 고정부 71B.
//! - **미지 버전은 무시**(오류 아님 — 미래 호환). 미지 `type`도 무시.
//! - **`instance` 16B(D-22 U-P1 · 사용자 확정)** — 프로세스 기동 시 난수. 같은 키·다른
//!   `instance`가 동시 관측되면 **키 파일 복제 경고**([`CloneWatch`]). `epoch`만으로는 재시작과
//!   구별되지 않고 클론 VM은 시각까지 같을 수 있다. **탐지이지 방지가 아니다**([docs/21 §5]).
//! - 발견 패킷은 **서명하지 않는 힌트**다([docs/08 §2]) — 신원 확정은 세션 핸드셰이크.
//! - `name`은 decode에서 무해화([`DisplayName::parse`]) — 실패하면 키 지문 라벨로 폴백
//!   (이름이 깨졌다고 존재를 숨기지 않는다).

use nbeep_core::ports::MonoInstant;
use nbeep_core::{DisplayName, PeerId};

/// 프로토콜 매직.
pub const MAGIC: [u8; 4] = *b"NXBP";
/// 현재 와이어 버전.
pub const WIRE_VER: u8 = 1;
/// 패킷 상한(바이트) — 초과 인코딩은 불가능하게 만든다.
pub const MAX_PACKET: usize = 512;
/// 고정부 길이(name_len 포함).
const FIXED: usize = 4 + 1 + 1 + 2 + 32 + 2 + 8 + 4 + 16 + 1;

/// 패킷 종류. 값은 불변(추가는 뒤에 append).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketKind {
    /// 기동 직후 존재 알림(응답 유도).
    Hello = 1,
    /// 주기 광고.
    Announce = 2,
    /// 특정 대상 확인(S4 유니캐스트 프로브).
    Probe = 3,
    /// 명시적 이탈.
    Goodbye = 4,
}

impl PacketKind {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(PacketKind::Hello),
            2 => Some(PacketKind::Announce),
            3 => Some(PacketKind::Probe),
            4 => Some(PacketKind::Goodbye),
            _ => None,
        }
    }
}

/// 발견 패킷(파싱 결과).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    /// 종류.
    pub kind: PacketKind,
    /// 능력 비트(릴레이·그룹·파일 등 — 배정은 해당 기능 구현 시).
    pub flags: u16,
    /// 발신 신원(= X25519 정적 공개키 — **미검증 힌트**).
    pub peer: PeerId,
    /// 세션 수신 TCP 포트.
    pub tcp_port: u16,
    /// 인스턴스 기동 시각(재시작 감지).
    pub epoch: u64,
    /// 재전송·중복 억제 시퀀스.
    pub seq: u32,
    /// 기동 난수(D-22 U-P1 — 복제 탐지).
    pub instance: [u8; 16],
    /// 무해화된 표시 이름.
    pub name: DisplayName,
}

/// 해석 결과 — 미지 버전/종류는 오류가 아니라 **무시**다(전방 호환).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded {
    /// 유효 패킷.
    Packet(Packet),
    /// 미래 버전·미지 종류 — 조용히 버린다.
    Ignore,
    /// 우리 프로토콜이 아니거나 손상 — 버린다(로그 대상).
    Invalid,
}

impl Packet {
    /// 인코딩. `name`은 UTF-8 255B까지 잘라 담고, 전체가 [`MAX_PACKET`]을 넘지 않는다.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FIXED + 64);
        out.extend_from_slice(&MAGIC);
        out.push(WIRE_VER);
        out.push(self.kind as u8);
        out.extend_from_slice(&self.flags.to_be_bytes());
        out.extend_from_slice(self.peer.as_bytes());
        out.extend_from_slice(&self.tcp_port.to_be_bytes());
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.instance);
        // 이름: UTF-8 경계 보존 255B 상한.
        let name = self.name.as_str().as_bytes();
        let mut cut = name.len().min(255).min(MAX_PACKET - FIXED);
        while cut > 0 && !self.name.as_str().is_char_boundary(cut) {
            cut -= 1;
        }
        out.push(u8::try_from(cut).unwrap_or(255));
        out.extend_from_slice(&name[..cut]);
        debug_assert!(out.len() <= MAX_PACKET);
        out
    }

    /// 해석. 손상 = [`Decoded::Invalid`], 미래 버전·미지 종류 = [`Decoded::Ignore`].
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Decoded {
        if bytes.len() < FIXED || bytes.len() > MAX_PACKET || bytes[..4] != MAGIC {
            return Decoded::Invalid;
        }
        if bytes[4] != WIRE_VER {
            return Decoded::Ignore; // 미래 버전 — 무시(docs/08 §2)
        }
        let Some(kind) = PacketKind::from_byte(bytes[5]) else {
            return Decoded::Ignore; // 미지 종류 — 무시(같은 전방 호환 규약)
        };
        let flags = u16::from_be_bytes([bytes[6], bytes[7]]);
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&bytes[8..40]);
        let peer = PeerId::from_bytes(pk);
        let tcp_port = u16::from_be_bytes([bytes[40], bytes[41]]);
        let mut e8 = [0u8; 8];
        e8.copy_from_slice(&bytes[42..50]);
        let epoch = u64::from_be_bytes(e8);
        let mut s4 = [0u8; 4];
        s4.copy_from_slice(&bytes[50..54]);
        let seq = u32::from_be_bytes(s4);
        let mut instance = [0u8; 16];
        instance.copy_from_slice(&bytes[54..70]);
        let name_len = bytes[70] as usize;
        if bytes.len() != FIXED + name_len {
            return Decoded::Invalid;
        }
        let name = std::str::from_utf8(&bytes[FIXED..])
            .ok()
            .and_then(|s| DisplayName::parse(s).ok())
            // 이름이 깨져도 존재는 숨기지 않는다 — 키 지문 라벨 폴백.
            .unwrap_or_else(|| {
                DisplayName::parse(&peer.short())
                    .unwrap_or_else(|_| DisplayName::parse("peer").expect("고정 문자열"))
            });
        Decoded::Packet(Packet {
            kind,
            flags,
            peer,
            tcp_port,
            epoch,
            seq,
            instance,
            name,
        })
    }
}

/// 키 파일 복제 탐지(D-22 U-P1 · [docs/21 §5] R-12) — **탐지이지 방지가 아니다**.
///
/// 같은 [`PeerId`]인데 **다른 `instance`** 가 `window_ms` 안에 함께 관측되면 복제 의심.
/// 재시작(instance 교체)은 이전 관측이 창을 벗어나므로 오탐하지 않는다.
#[derive(Debug)]
pub struct CloneWatch {
    window_ms: u32,
    seen: std::collections::HashMap<PeerId, ([u8; 16], MonoInstant)>,
}

impl CloneWatch {
    /// `window_ms` 동시 관측 창의 탐지기(수치는 D-8b 실측 후 확정 — 주입).
    #[must_use]
    pub fn new(window_ms: u32) -> Self {
        Self {
            window_ms,
            seen: std::collections::HashMap::new(),
        }
    }

    /// 관측 하나를 접는다 — 복제 의심이면 `true`(UI 경고 대상 · [docs/21 §5] "막을 수 없는 것은 알려 준다").
    pub fn observe(&mut self, peer: PeerId, instance: [u8; 16], now: MonoInstant) -> bool {
        match self.seen.get(&peer) {
            Some(&(prev, at))
                if prev != instance && now.saturating_ms_since(at) < self.window_ms =>
            {
                // 다른 instance가 창 안에 공존 — 복제 의심. 최신 관측으로 갱신.
                self.seen.insert(peer, (instance, now));
                true
            }
            _ => {
                self.seen.insert(peer, (instance, now));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }
    fn name(s: &str) -> DisplayName {
        DisplayName::parse(s).unwrap()
    }
    fn packet() -> Packet {
        Packet {
            kind: PacketKind::Announce,
            flags: 0x0003,
            peer: pid(0xAB),
            tcp_port: 47_100,
            epoch: 0x0102_0304_0506_0708,
            seq: 42,
            instance: [7u8; 16],
            name: name("김철수의 MacBook"),
        }
    }

    #[test]
    fn roundtrip() {
        let p = packet();
        assert_eq!(Packet::decode(&p.encode()), Decoded::Packet(p));
    }

    #[test]
    fn golden_wire_layout_is_stable() {
        // 와이어 회귀 고정 — 이 바이트가 바뀌면 프로토콜 호환이 깨진 것이다.
        let b = packet().encode();
        assert_eq!(&b[..4], b"NXBP", "magic");
        assert_eq!(b[4], 1, "ver");
        assert_eq!(b[5], 2, "type=Announce");
        assert_eq!(&b[6..8], &[0, 3], "flags BE");
        assert_eq!(b[8], 0xAB, "pubkey 시작");
        assert_eq!(&b[40..42], &47_100u16.to_be_bytes(), "tcp_port");
        assert_eq!(&b[42..50], &[1, 2, 3, 4, 5, 6, 7, 8], "epoch BE");
        assert_eq!(&b[50..54], &[0, 0, 0, 42], "seq BE");
        assert_eq!(&b[54..70], &[7u8; 16], "instance(D-22 U-P1)");
        assert_eq!(b[70] as usize, "김철수의 MacBook".len(), "name_len");
        assert!(b.len() <= MAX_PACKET);
    }

    /// ★ M1-11① 허용 목록의 구조적 강제 — 인코딩 전체가 **허용 필드의 연접과
    /// 바이트 단위로 동일**해야 한다(W-1: "허용 목록 밖의 바이트가 한 개도 없다").
    /// 누군가 Packet에 필드를 추가해 몰래 실으면 이 테스트가 바로 깨진다.
    #[test]
    fn every_byte_comes_from_the_allow_list() {
        for kind in [
            PacketKind::Hello,
            PacketKind::Announce,
            PacketKind::Probe,
            PacketKind::Goodbye,
        ] {
            let mut p = packet();
            p.kind = kind;
            let b = p.encode();
            let mut want = Vec::new();
            want.extend_from_slice(b"NXBP");
            want.push(1);
            want.push(kind as u8);
            want.extend_from_slice(&p.flags.to_be_bytes());
            want.extend_from_slice(p.peer.as_bytes());
            want.extend_from_slice(&p.tcp_port.to_be_bytes());
            want.extend_from_slice(&p.epoch.to_be_bytes());
            want.extend_from_slice(&p.seq.to_be_bytes());
            want.extend_from_slice(&p.instance);
            let nb = p.name.as_str().as_bytes();
            want.push(u8::try_from(nb.len()).unwrap());
            want.extend_from_slice(nb);
            assert_eq!(b, want, "{kind:?} — 허용 목록 연접과 정확히 일치해야 한다");
        }
    }

    /// ★ M1-11② 금칙어 스캔 — 프로필 계열 문자열(이메일·전화·설정 키)이 발견
    /// 패킷 어디에도 실릴 수 없다(구조적으로 입력이 없지만, 필드 추가 회귀 감시).
    #[test]
    fn discovery_never_carries_profile_content() {
        const FORBIDDEN: &[&[u8]] = &[b"@", b"profile.", b"share", b"010-", b"kiros", b".com"];
        for kind in [
            PacketKind::Hello,
            PacketKind::Announce,
            PacketKind::Probe,
            PacketKind::Goodbye,
        ] {
            let mut p = packet();
            p.kind = kind;
            p.name = name("beep-1a2b3c4d"); // 기본 이름 규약(M1-10 — 실명 미노출)
            let b = p.encode();
            for f in FORBIDDEN {
                assert!(
                    !b.windows(f.len()).any(|w| w == *f),
                    "{kind:?} 패킷에 금칙어 {:?}",
                    String::from_utf8_lossy(f)
                );
            }
        }
    }

    #[test]
    fn unknown_version_and_kind_are_ignored_not_invalid() {
        let mut future_ver = packet().encode();
        future_ver[4] = 9;
        assert_eq!(
            Packet::decode(&future_ver),
            Decoded::Ignore,
            "미래 버전 무시"
        );
        let mut future_kind = packet().encode();
        future_kind[5] = 200;
        assert_eq!(
            Packet::decode(&future_kind),
            Decoded::Ignore,
            "미지 종류 무시"
        );
    }

    #[test]
    fn corrupt_packets_are_invalid() {
        assert_eq!(
            Packet::decode(b"XXXX"),
            Decoded::Invalid,
            "매직 불일치·짧음"
        );
        let mut truncated = packet().encode();
        truncated.pop();
        assert_eq!(Packet::decode(&truncated), Decoded::Invalid, "길이 불일치");
        let mut oversize = packet().encode();
        oversize.resize(MAX_PACKET + 1, 0);
        assert_eq!(Packet::decode(&oversize), Decoded::Invalid, "상한 초과");
    }

    #[test]
    fn garbled_name_falls_back_to_fingerprint_label() {
        // 이름 바이트가 깨져도(비 UTF-8) 존재는 숨기지 않는다.
        let mut b = packet().encode();
        let fixed = 71;
        b[fixed] = 0xFF; // invalid UTF-8 시작 바이트
        let Decoded::Packet(p) = Packet::decode(&b) else {
            panic!("패킷이어야 함");
        };
        assert_eq!(p.name.as_str(), pid(0xAB).short(), "지문 라벨 폴백");
    }

    #[test]
    fn encode_never_exceeds_cap_with_long_name() {
        let mut p = packet();
        p.name = name(&"가".repeat(64)); // 최장 이름(64자 × 3B = 192B)
        let b = p.encode();
        assert!(b.len() <= MAX_PACKET, "{}", b.len());
        assert_eq!(
            Packet::decode(&b),
            Decoded::Packet(p),
            "경계 보존 라운드트립"
        );
    }

    #[test]
    fn clone_watch_flags_concurrent_instances_only() {
        use nbeep_core::ports::MonoInstant;
        let at = |ms: u64| MonoInstant(ms * 1_000_000);
        let mut w = CloneWatch::new(10_000);
        let (a, b) = ([1u8; 16], [2u8; 16]);
        assert!(!w.observe(pid(1), a, at(0)), "첫 관측");
        assert!(!w.observe(pid(1), a, at(1_000)), "같은 instance = 정상");
        assert!(
            w.observe(pid(1), b, at(2_000)),
            "다른 instance 동시 공존 = 복제 의심"
        );
        // 재시작 시나리오: 창을 벗어난 뒤의 새 instance는 오탐하지 않는다.
        let mut w2 = CloneWatch::new(10_000);
        w2.observe(pid(2), a, at(0));
        assert!(
            !w2.observe(pid(2), b, at(20_000)),
            "창 밖 = 재시작으로 간주"
        );
    }
}
