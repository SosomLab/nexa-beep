//! 사용자 신원 재료 — ADR-0015([docs/46] · DR-29) **3층 구조**의 ②·③층.
//!
//! ```text
//! ③ 문 열쇠  handle + passphrase ──PBKDF2──► KP ──► PSK · RID_pair · LAN_tag · K_wrap_user
//!                                                    (바꿔도 신원은 불변 — 만남·형제 인증·봉인 재료일 뿐)
//! ② 신원     UserKey(Ed25519 · 무작위 · 첫 기기가 1회 생성 · 형제에 봉인 복제)
//!            UserId = SHA-256("nbeep-user-id-v2" ‖ pub)                ← 남이 나를 가리키는 값
//! ① 앵커     PeerId(기기 정적 키 · 현행)                                ← 핀·세션·후계 판정 근거
//! ```
//!
//! - **KDF는 sha2로 직접**(HMAC RFC 2104 · PBKDF2 RFC 8018) — nexa-clip `nclip-sync/src/rid.rs`와
//!   같은 구현(자매 프로젝트 · 같은 저자 · 09-06 이식). `hmac`/`pbkdf2` 크레이트를 끌지 않는다(DR-5).
//! - **도메인 문자열 `nbeep-user-*`** 는 앱 식별자다([docs/44 §1]) — clip(`nclip-*`)과 다르므로
//!   같은 핸들·암호를 두 앱에 넣어도 서로 만나지 않는다. 바꾸면 기존 사용자 기기끼리 못 만난다.
//! - 서명은 `ed25519-dalek 2`(BSD-3 · `curve25519-dalek 4`를 snow와 공유 — 트리 증가 크레이트 1개 ·
//!   [docs/10 §3]). 암호 원시는 직접 구현하지 않는다(NFR-S-3).
//! - `KP`·개인키는 **디스크에 평문으로 두지 않는다** — 봉인·저장은 앱 계층([docs/46 §3-6]).

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use nbeep_core::UserId;
use sha2::{Digest as _, Sha256};

/// PBKDF2 반복 수 — clip과 같은 값(추측 1회 비용 · 기기 1회 계산 수십 ms).
pub const KDF_ITERS: u32 = 60_000;
/// KDF 솔트 도메인(핸들이 뒤에 붙는다).
const DOM_KDF: &[u8] = b"nbeep-user-kdf-v1";
const DOM_PSK: &[u8] = b"nbeep-user-psk-v1";
const DOM_RID: &[u8] = b"nbeep-user-rid-v1";
const DOM_LAN: &[u8] = b"nbeep-user-lan-v1";
const DOM_WRAP: &[u8] = b"nbeep-user-wrap-v1";
const DOM_ID: &[u8] = b"nbeep-user-id-v2";

/// 16바이트 태그(랑데부 RID · LAN 힌트) — `nbeep_relay::Rid`와 같은 모양.
pub type Tag16 = [u8; 16];

// ---------------------------------------------------------------- KDF (sha2 직접)

/// HMAC-SHA256(RFC 2104).
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = {
        let mut h = Sha256::new();
        h.update(ipad);
        h.update(msg);
        h.finalize()
    };
    let mut h = Sha256::new();
    h.update(opad);
    h.update(inner);
    h.finalize().into()
}

/// PBKDF2-HMAC-SHA256 첫 블록(32B) — 필요한 재료가 전부 32B 이하라 1블록이면 충분.
fn pbkdf2_block1(pass: &[u8], salt: &[u8], iters: u32) -> [u8; 32] {
    let mut msg = salt.to_vec();
    msg.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(pass, &msg);
    let mut out = u;
    for _ in 1..iters {
        u = hmac_sha256(pass, &u);
        for (o, b) in out.iter_mut().zip(u.iter()) {
            *o ^= b;
        }
    }
    out
}

fn h32(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(domain);
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// 지금 시각의 에폭 일 번호(UTC) — 릴레이 RID와 같은 단위.
#[must_use]
pub fn current_epoch_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400)
}

// ---------------------------------------------------------------- ③ 문 열쇠

/// 핸들+패스프레이즈에서 파생한 **문 열쇠 재료** `KP`(32B) — 메모리에만 둔다.
#[derive(Clone)]
pub struct KeyMaterial([u8; 32]);

impl std::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyMaterial(…)") // 비밀 — 로그에 남기지 않는다
    }
}

impl Drop for KeyMaterial {
    fn drop(&mut self) {
        // 최선의 지움(컴파일러가 최적화로 없앨 수 있으나 비용이 0이라 둔다).
        for b in &mut self.0 {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

impl KeyMaterial {
    /// `KP = PBKDF2-HMAC-SHA256(passphrase, "nbeep-user-kdf-v1" ‖ handle, 60k)`.
    /// 핸들은 **솔트에만** 들어간다 — 같은 핸들·다른 암호 = 다른 재료(충돌 소멸).
    #[must_use]
    pub fn derive(handle: &str, passphrase: &str) -> Self {
        Self::derive_iters(handle, passphrase, KDF_ITERS)
    }

    /// 반복 수 지정(테스트 · 벤치 전용 — 제품은 [`KDF_ITERS`]).
    #[must_use]
    pub fn derive_iters(handle: &str, passphrase: &str, iters: u32) -> Self {
        let mut salt = Vec::with_capacity(DOM_KDF.len() + handle.len());
        salt.extend_from_slice(DOM_KDF);
        salt.extend_from_slice(handle.as_bytes());
        Self(pbkdf2_block1(passphrase.as_bytes(), &salt, iters.max(1)))
    }

    /// Noise `XXpsk3` 재료 — 형제 기기 세션의 소속 증명(ADR-0015 §3-3).
    #[must_use]
    pub fn psk(&self) -> [u8; 32] {
        h32(DOM_PSK, &[&self.0])
    }

    /// 사용자 마스터 키·UserKey 봉인 열쇠(§3-6 · §6-2).
    #[must_use]
    pub fn wrap_key(&self) -> [u8; 32] {
        h32(DOM_WRAP, &[&self.0])
    }

    /// 릴레이 만남 지점(에폭 일) — 기기별 RID와 나란히 등록한다(§3-4 · 서버 무변경).
    #[must_use]
    pub fn rid_pair(&self, epoch_day: u64) -> Tag16 {
        let out = h32(DOM_RID, &[&self.0, &epoch_day.to_be_bytes()]);
        let mut t = [0u8; 16];
        t.copy_from_slice(&out[..16]);
        t
    }

    /// 어제·오늘·내일 셋 — 시계 오차 흡수(`nbeep_relay::rids_around`와 같은 규칙).
    #[must_use]
    pub fn rids_around(&self) -> [Tag16; 3] {
        let d = current_epoch_day();
        [
            self.rid_pair(d.saturating_sub(1)),
            self.rid_pair(d),
            self.rid_pair(d + 1),
        ]
    }

    /// LAN 발견 힌트 태그 — RID를 한 번 더 감싼다(도청자가 태그로 만남 지점을 얻지 못하게).
    #[must_use]
    pub fn lan_tag(&self, epoch_day: u64) -> Tag16 {
        let out = h32(DOM_LAN, &[&self.rid_pair(epoch_day)]);
        let mut t = [0u8; 16];
        t.copy_from_slice(&out[..16]);
        t
    }

    /// 어제·오늘·내일 LAN 태그.
    #[must_use]
    pub fn lan_tags_around(&self) -> [Tag16; 3] {
        let d = current_epoch_day();
        [
            self.lan_tag(d.saturating_sub(1)),
            self.lan_tag(d),
            self.lan_tag(d + 1),
        ]
    }
}

// ---------------------------------------------------------------- ② 신원

/// 사용자 장기 서명 키(Ed25519) — 첫 기기가 1회 생성해 형제 기기에 복제한다.
#[derive(Clone)]
pub struct UserKey {
    signing: SigningKey,
}

impl std::fmt::Debug for UserKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UserKey({})", self.user_id().short())
    }
}

/// 직렬화 길이(시드 32 + 공개키 32).
pub const USERKEY_LEN: usize = 64;
/// 서명 길이.
pub const SIG_LEN: usize = 64;

impl UserKey {
    /// 새 키(OS CSPRNG).
    ///
    /// # Errors
    /// 난수원 실패.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed)?;
        Ok(Self {
            signing: SigningKey::from_bytes(&seed),
        })
    }

    /// `[seed 32][pub 32]` 직렬화 — 봉인 저장·형제 복제 단위.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; USERKEY_LEN] {
        let mut out = [0u8; USERKEY_LEN];
        out[..32].copy_from_slice(&self.signing.to_bytes());
        out[32..].copy_from_slice(self.signing.verifying_key().as_bytes());
        out
    }

    /// 역직렬화 — 공개키가 시드와 맞지 않으면 `None`(손상·위조).
    #[must_use]
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() != USERKEY_LEN {
            return None;
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&b[..32]);
        let signing = SigningKey::from_bytes(&seed);
        (signing.verifying_key().as_bytes() == &b[32..]).then_some(Self { signing })
    }

    /// 공개키(32B).
    #[must_use]
    pub fn public(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// `UserId = SHA-256("nbeep-user-id-v2" ‖ pub)`.
    #[must_use]
    pub fn user_id(&self) -> UserId {
        user_id_of(&self.public())
    }

    /// 서명(도메인 분리는 호출자가 메시지 앞에 붙인다 — `UserHello`·`Succession` 등).
    #[must_use]
    pub fn sign(&self, msg: &[u8]) -> [u8; SIG_LEN] {
        self.signing.sign(msg).to_bytes()
    }
}

/// 공개키에서 `UserId`.
#[must_use]
pub fn user_id_of(public: &[u8; 32]) -> UserId {
    UserId::from_bytes(h32(DOM_ID, &[public]))
}

/// 서명 검증 — 공개키 손상·서명 불일치 전부 `false`(fail-closed).
#[must_use]
pub fn verify(public: &[u8; 32], msg: &[u8], sig: &[u8]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(public) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(sig) else {
        return false;
    };
    vk.verify(msg, &sig).is_ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// RFC 4231 test case 2 — HMAC-SHA256("Jefe", "what do ya want for nothing?").
    #[test]
    fn hmac_rfc4231_case2() {
        let out = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&out),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    /// RFC 6070류(PBKDF2-HMAC-SHA256) 공개 벡터 — "password"/"salt"/1회.
    #[test]
    fn pbkdf2_sha256_vectors() {
        let one = pbkdf2_block1(b"password", b"salt", 1);
        assert_eq!(
            hex(&one),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        let two = pbkdf2_block1(b"password", b"salt", 2);
        assert_eq!(
            hex(&two),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
    }

    #[test]
    fn material_is_deterministic_and_handle_salted() {
        let a = KeyMaterial::derive_iters("kiros33", "correct horse", 50);
        let b = KeyMaterial::derive_iters("kiros33", "correct horse", 50);
        let c = KeyMaterial::derive_iters("someone", "correct horse", 50);
        let d = KeyMaterial::derive_iters("kiros33", "other", 50);
        assert_eq!(a.psk(), b.psk(), "같은 두 값 = 같은 재료");
        assert_ne!(
            a.psk(),
            c.psk(),
            "핸들이 솔트 — 같은 암호·다른 핸들은 다르다"
        );
        assert_ne!(a.psk(), d.psk());
        // 재료 5종은 서로 다른 도메인에서 나온다.
        assert_ne!(a.psk(), a.wrap_key());
        assert_ne!(&a.psk()[..16], &a.rid_pair(1)[..]);
        assert_ne!(a.rid_pair(1), a.lan_tag(1));
        assert_ne!(a.rid_pair(1), a.rid_pair(2), "에폭 일마다 회전");
    }

    #[test]
    fn rids_around_are_three_distinct_days() {
        let m = KeyMaterial::derive_iters("h", "p", 10);
        let r = m.rids_around();
        assert_ne!(r[0], r[1]);
        assert_ne!(r[1], r[2]);
        let t = m.lan_tags_around();
        assert_ne!(t[0], t[1]);
    }

    #[test]
    fn userkey_roundtrip_sign_verify() {
        let k = UserKey::generate().unwrap();
        let bytes = k.to_bytes();
        let k2 = UserKey::from_bytes(&bytes).unwrap();
        assert_eq!(k.user_id(), k2.user_id());
        let msg = b"UserHello v1";
        let sig = k.sign(msg);
        assert!(verify(&k.public(), msg, &sig));
        assert!(
            !verify(&k.public(), b"tampered", &sig),
            "메시지 변조 = 실패"
        );
        let mut bad = sig;
        bad[0] ^= 1;
        assert!(!verify(&k.public(), msg, &bad), "서명 변조 = 실패");
        let other = UserKey::generate().unwrap();
        assert!(!verify(&other.public(), msg, &sig), "다른 키 = 실패");
        assert!(
            !verify(&[0u8; 32], msg, &sig),
            "무효 공개키 = 실패(패닉 없음)"
        );
    }

    #[test]
    fn userkey_from_bytes_rejects_mismatch_and_length() {
        let k = UserKey::generate().unwrap();
        let mut b = k.to_bytes();
        b[40] ^= 1; // 공개키 훼손
        assert!(UserKey::from_bytes(&b).is_none());
        assert!(UserKey::from_bytes(&b[..63]).is_none());
    }

    #[test]
    fn user_id_is_stable_and_independent_of_handle() {
        let k = UserKey::generate().unwrap();
        assert_eq!(k.user_id(), user_id_of(&k.public()));
        // 핸들·암호는 UserId에 관여하지 않는다(ADR-0015 3층 — 바꿔도 신원 불변).
        let _m1 = KeyMaterial::derive_iters("a", "x", 5);
        let _m2 = KeyMaterial::derive_iters("b", "y", 5);
        assert_eq!(k.user_id(), user_id_of(&k.public()));
    }
}
