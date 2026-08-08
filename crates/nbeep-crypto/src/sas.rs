//! SAS — **육안 대조용 안전번호**([docs/08] ADR-0002 §4 · [docs/21] §3-1).
//!
//! TOFU 핀은 "처음 본 키를 기억"할 뿐, **그 키가 진짜 그 사람의 것인지**는 모른다(첫 접촉이 이미
//! 중간자였다면 잘못된 키를 핀한다). 그걸 닫는 유일한 수단이 **대역 외 대조** — 두 사람이 전화·대면으로
//! 같은 숫자를 읽고 일치를 확인한 뒤에만 `TrustStore::verify`로 승격한다.
//!
//! ## 설계
//!
//! - **양쪽 키를 정렬해 해싱** → 순서 무관. 두 사람의 화면에 **같은 값**이 뜬다(Signal 안전번호 방식).
//! - **BLAKE2s**(Noise 스위트와 동일 — [`snow`] 것을 쓰므로 **새 의존성 없음**, NFR-S-3).
//! - **60자리(5자리 12묶음, ≈199비트)** — Signal/WhatsApp과 같은 길이(M2-2b · [docs/21] Q-21-5).
//!   MITM은 양쪽에 **다른 키**를 보여주며 값이 같아지는 **쌍**을 찾으면 되므로(생일 공격) 표시
//!   비트의 절반이 실효 보안이다: 12자리(40비트)면 2²⁰(노트북 수 분)에 뚫리지만 60자리는 ≈2⁹⁹다.
//! - **세션이 아니라 키에서 파생** → 재접속해도 값이 같다. `verify()`가 영속되는 것과 짝이 맞는다.

use nbeep_core::PeerId;
use snow::params::HashChoice;
use snow::resolvers::{CryptoResolver, DefaultResolver};

/// 도메인 분리 태그 — 다른 용도의 해시와 값이 겹치지 않게 한다.
const DOMAIN: &[u8] = b"nexa-beep/sas/v1";

/// 5자리 묶음 수 — 12묶음 × 5자리 = 60자리(Signal 규격).
const GROUPS: usize = 12;

/// 묶음 하나를 만드는 데 쓰는 다이제스트 바이트 수(Signal 방식 — 5바이트 → `% 100000`).
const CHUNK: usize = 5;

/// 두 신원 사이의 **안전번호** — 5자리 12묶음, 4묶음씩 3행(예: `"12345 67890 …"`).
///
/// 양쪽에서 같은 값이 나온다(인자 순서 무관). 두 사람이 대역 외로 읽어 일치하면
/// `TrustStore::verify`로 [`nbeep_core::TrustLevel::FingerprintVerified`] 승격.
///
/// # Panics
/// BLAKE2s 리졸브에 실패하면(사실상 불가) 패닉.
#[must_use]
pub fn safety_number(a: PeerId, b: PeerId) -> String {
    // 정렬 — 누가 개시자였는지에 무관하게 같은 값.
    let (lo, hi) = if a.as_bytes() <= b.as_bytes() {
        (a, b)
    } else {
        (b, a)
    };

    // 12묶음 × 5바이트 = 60바이트가 필요한데 BLAKE2s 다이제스트는 32바이트다.
    // 카운터 도메인으로 두 번 해싱해 64바이트를 얻는다(각각 독립 도메인 — XOF 대용).
    let mut material = [0u8; 64];
    for (i, half) in material.chunks_mut(32).enumerate() {
        let mut hasher = DefaultResolver
            .resolve_hash(&HashChoice::Blake2s)
            .expect("BLAKE2s는 기본 리졸버에 항상 있다");
        hasher.reset();
        hasher.input(DOMAIN);
        hasher.input(&[u8::try_from(i).expect("카운터 2")]);
        hasher.input(lo.as_bytes());
        hasher.input(hi.as_bytes());
        let mut digest = [0u8; 64]; // snow Hash::result는 MAXHASHLEN 버퍼를 기대
        hasher.result(&mut digest);
        half.copy_from_slice(&digest[..32]);
    }

    // Signal 방식: 5바이트 빅엔디언 → % 100000 → 5자리(묶음당 상위 비트 소량 편향은 규격 수용).
    let mut out = String::with_capacity(GROUPS * 6);
    for g in 0..GROUPS {
        let c = &material[g * CHUNK..(g + 1) * CHUNK];
        let v = u64::from(c[0]) << 32
            | u64::from(c[1]) << 24
            | u64::from(c[2]) << 16
            | u64::from(c[3]) << 8
            | u64::from(c[4]);
        if g > 0 {
            out.push(' ');
        }
        use core::fmt::Write;
        write!(out, "{:05}", v % 100_000).expect("String write");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PeerId {
        let mut a = [0u8; 32];
        a[0] = b;
        PeerId::from_bytes(a)
    }

    #[test]
    fn both_sides_see_the_same_number() {
        // 순서가 달라도 같은 값 — 두 사람 화면이 일치해야 대조가 성립한다.
        assert_eq!(safety_number(pid(1), pid(2)), safety_number(pid(2), pid(1)));
    }

    #[test]
    fn different_peers_differ() {
        assert_ne!(safety_number(pid(1), pid(2)), safety_number(pid(1), pid(3)));
    }

    #[test]
    fn format_is_twelve_groups_of_five_digits() {
        let sas = safety_number(pid(1), pid(2));
        let groups: Vec<&str> = sas.split(' ').collect();
        assert_eq!(groups.len(), 12, "5자리 12묶음(60자리): {sas}");
        assert!(
            groups
                .iter()
                .all(|g| g.len() == 5 && g.bytes().all(|c| c.is_ascii_digit())),
            "각 묶음은 숫자 5자리: {sas}"
        );
    }

    #[test]
    fn is_stable_across_calls() {
        // 세션이 아니라 키에서 파생 — 재접속해도 같아야 verify()의 영속과 짝이 맞는다.
        assert_eq!(safety_number(pid(7), pid(8)), safety_number(pid(7), pid(8)));
    }

    #[test]
    fn uses_full_digest_width() {
        // 뒷 묶음(두 번째 해시 영역)도 키에 따라 달라진다 — 앞 8바이트만 접던 구멍 회귀 방지.
        let a = safety_number(pid(1), pid(2));
        let b = safety_number(pid(1), pid(4));
        let (ta, tb) = (
            a.split(' ').next_back().unwrap(),
            b.split(' ').next_back().unwrap(),
        );
        // 마지막 묶음까지 확률적으로 달라야 정상(같으면 1/100000 우연 — 키 셋 바꿔 재확인).
        let c = safety_number(pid(2), pid(3));
        let tc = c.split(' ').next_back().unwrap();
        assert!(
            ta != tb || ta != tc,
            "마지막 묶음이 전부 같으면 뒷 영역 미사용 의심"
        );
    }

    #[test]
    fn real_identities_produce_matching_numbers() {
        let (alice, bob) = (crate::Identity::generate(), crate::Identity::generate());
        let a_view = safety_number(alice.peer_id(), bob.peer_id());
        let b_view = safety_number(bob.peer_id(), alice.peer_id());
        assert_eq!(a_view, b_view, "실물 키에서도 양쪽이 일치");
    }
}
