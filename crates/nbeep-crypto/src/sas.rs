//! SAS — **육안 대조용 안전번호**([docs/08] ADR-0002 §4 · [docs/13] §7).
//!
//! TOFU 핀은 "처음 본 키를 기억"할 뿐, **그 키가 진짜 그 사람의 것인지**는 모른다(첫 접촉이 이미
//! 중간자였다면 잘못된 키를 핀한다). 그걸 닫는 유일한 수단이 **대역 외 대조** — 두 사람이 전화·대면으로
//! 같은 숫자를 읽고 일치를 확인한 뒤에만 `TrustStore::verify`로 승격한다.
//!
//! ## 설계
//!
//! - **양쪽 키를 정렬해 해싱** → 순서 무관. 두 사람의 화면에 **같은 값**이 뜬다(Signal 안전번호 방식).
//! - **BLAKE2s**(Noise 스위트와 동일 — [`snow`] 것을 쓰므로 **새 의존성 없음**, NFR-S-3).
//! - **60비트 / 12자리**를 4자리 3묶음으로 — 사람이 읽을 수 있으면서 무차별 대입이 무의미한 길이.
//! - **세션이 아니라 키에서 파생** → 재접속해도 값이 같다. `verify()`가 영속되는 것과 짝이 맞는다.

use nbeep_core::PeerId;
use snow::params::HashChoice;
use snow::resolvers::{CryptoResolver, DefaultResolver};

/// 도메인 분리 태그 — 다른 용도의 해시와 값이 겹치지 않게 한다.
const DOMAIN: &[u8] = b"nexa-beep/sas/v1";

/// 안전번호 자릿수(10진) — 12자리 ≈ 40비트. 사람이 읽을 수 있으면서 우연 일치가 무의미한 길이.
const DIGITS: usize = 12;

/// 두 신원 사이의 **안전번호** — 4자리 3묶음(예: `"4829 1377 0254"`).
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

    let mut hasher = DefaultResolver
        .resolve_hash(&HashChoice::Blake2s)
        .expect("BLAKE2s는 기본 리졸버에 항상 있다");
    hasher.reset();
    hasher.input(DOMAIN);
    hasher.input(lo.as_bytes());
    hasher.input(hi.as_bytes());
    let mut digest = [0u8; 64]; // MAXHASHLEN 여유
    debug_assert!(hasher.hash_len() >= 8, "BLAKE2s는 32바이트");
    hasher.result(&mut digest);

    // 다이제스트 앞 8바이트를 10진 12자리로 접는다.
    let mut v = u64::from_be_bytes(digest[..8].try_into().expect("8바이트"));

    let mut digits = [0u8; DIGITS];
    for slot in digits.iter_mut().rev() {
        *slot = u8::try_from(v % 10).expect("한 자리");
        v /= 10;
    }

    // 4자리씩 끊어 읽기 쉽게.
    let mut out = String::with_capacity(DIGITS + 2);
    for (i, d) in digits.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push(' ');
        }
        out.push(char::from(b'0' + d));
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
    fn format_is_three_groups_of_four_digits() {
        let sas = safety_number(pid(1), pid(2));
        let groups: Vec<&str> = sas.split(' ').collect();
        assert_eq!(groups.len(), 3, "4자리 3묶음: {sas}");
        assert!(
            groups
                .iter()
                .all(|g| g.len() == 4 && g.bytes().all(|c| c.is_ascii_digit())),
            "각 묶음은 숫자 4자리: {sas}"
        );
    }

    #[test]
    fn is_stable_across_calls() {
        // 세션이 아니라 키에서 파생 — 재접속해도 같아야 verify()의 영속과 짝이 맞는다.
        assert_eq!(safety_number(pid(7), pid(8)), safety_number(pid(7), pid(8)));
    }

    #[test]
    fn real_identities_produce_matching_numbers() {
        let (alice, bob) = (crate::Identity::generate(), crate::Identity::generate());
        let a_view = safety_number(alice.peer_id(), bob.peer_id());
        let b_view = safety_number(bob.peer_id(), alice.peer_id());
        assert_eq!(a_view, b_view, "실물 키에서도 양쪽이 일치");
    }
}
