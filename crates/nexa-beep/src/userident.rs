//! 사용자 신원(ADR-0015 · DR-29) — 설정값 검증 · 인증 상태기계 · `user.key` 봉인 파일.
//!
//! **순수 부분**(검증·상태 전이)은 여기서 테스트하고, 앱은 전이 결과만 소비한다.
//!
//! 사용자 규칙(09-06 확정):
//! - **필수 값(핸들·페어링 암호)이 비면 기능·하위 설정이 전부 잠긴다** — 형식이 틀려도 같다.
//! - **테스트 버튼**은 두 값이 유효할 때만 활성. 값이 바뀌면(키인·붙여넣기 불문) 검증 마커가
//!   내려가고 다시 테스트할 때까지 사용자 기능이 멈춘다.
//! - **한 번 성공 = 즉시 인증**(마커 `user.verified` 영속 — 재기동에도 유지) · **실패 = 인증 의존
//!   기능 전부 중지**.
//! - 성공 시 재료는 **노출되지 않게 보관** — 암호는 `profile.sec` 봉인 사이드카(PII_KEYS),
//!   UserKey는 `user.key`(K_wrap_user 봉인 · 0600) · 로그·상태바에는 핸들과 UserId 지문만.

use nbeep_core::UserId;
use nbeep_crypto::userkey::{KeyMaterial, UserKey, USERKEY_LEN};
use std::path::Path;

/// 핸들 길이(문자 수) 범위.
pub(crate) const HANDLE_MIN: usize = 3;
pub(crate) const HANDLE_MAX: usize = 32;
/// 페어링 암호 최소 길이(문자 수) — 테스트 활성 조건(D-32-7 강제 없음의 예외 = "빈 값에 가까운
/// 암호"만 막는다 · 강도 경고는 별도).
pub(crate) const PASS_MIN: usize = 8;
/// 자동 추천 암호 길이(≈59비트 · clip과 동일 규격 `xxxx-xxxx-xxxx`).
pub(crate) const PASS_SUGGEST_LEN: usize = 12;

/// 입력값 판정 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Validity {
    /// 둘 다 유효 — 테스트 가능.
    Ok,
    /// 둘 중 하나라도 빔 — 미설정(기능 잠금).
    Empty,
    /// 핸들 형식 위반(길이·허용 문자).
    BadHandle,
    /// 암호가 너무 짧다.
    ShortPass,
}

/// 핸들 정규화 — 앞뒤 공백 제거. 그 외는 손대지 않는다(사용자가 친 그대로가 라벨).
#[must_use]
pub(crate) fn normalize_handle(s: &str) -> &str {
    s.trim()
}

/// 핸들 형식: 3~32자 · `A-Z a-z 0-9 . _ - @` 만(방송·목록 라벨이라 제어문자·공백 금지 ·
/// `@`는 기본 핸들 = 정제된 표시 이름(`kiros33@mac`)을 허용하기 위해).
#[must_use]
pub(crate) fn handle_ok(h: &str) -> bool {
    let n = h.chars().count();
    (HANDLE_MIN..=HANDLE_MAX).contains(&n)
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
}

/// 두 값의 판정 — 빈 값 우선(잠금 사유가 "형식"보다 "미설정"이 먼저다).
#[must_use]
pub(crate) fn validate(handle: &str, pass: &str) -> Validity {
    let h = normalize_handle(handle);
    if h.is_empty() || pass.is_empty() {
        return Validity::Empty;
    }
    if !handle_ok(h) {
        return Validity::BadHandle;
    }
    if pass.chars().count() < PASS_MIN {
        return Validity::ShortPass;
    }
    Validity::Ok
}

/// 암호 강도(표시용 · 대략적 비트) — 문자 집합 크기 × 길이. 강제하지 않는다(D-32-7).
#[must_use]
pub(crate) fn pass_strength_bits(pass: &str) -> u32 {
    let (mut lo, mut up, mut di, mut sy) = (false, false, false, false);
    // 구분자 `-`는 구조(추천 형식 `xxxx-xxxx-xxxx`)라 엔트로피에 넣지 않는다.
    let body: Vec<char> = pass.chars().filter(|c| *c != '-').collect();
    for &c in &body {
        if c.is_ascii_lowercase() {
            lo = true;
        } else if c.is_ascii_uppercase() {
            up = true;
        } else if c.is_ascii_digit() {
            di = true;
        } else {
            sy = true;
        }
    }
    let pool: f64 = [(lo, 26.0), (up, 26.0), (di, 10.0), (sy, 33.0)]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| n)
        .sum::<f64>()
        .max(1.0);
    let n = body.len() as f64;
    (n * pool.log2()).round() as u32
}

/// 강도 등급(표시 문구 선택용): 0 = 약함(<40비트) · 1 = 보통(<60) · 2 = 강함.
#[must_use]
pub(crate) fn pass_grade(pass: &str) -> u8 {
    match pass_strength_bits(pass) {
        b if b < 40 => 0,
        b if b < 60 => 1,
        _ => 2,
    }
}

/// 자동 추천 암호 — `xxxx-xxxx-xxxx`(base32풍 · 혼동 글자 `0 1 o i l` 제외 · OS CSPRNG).
#[must_use]
pub(crate) fn suggest_passphrase() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut buf = [0u8; PASS_SUGGEST_LEN];
    if getrandom::getrandom(&mut buf).is_err() {
        // 난수원 실패 = 추천하지 않는다(약한 값을 조용히 주지 않는다).
        return String::new();
    }
    let mut out = String::with_capacity(PASS_SUGGEST_LEN + 2);
    for (i, b) in buf.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(ALPHABET[usize::from(*b) % ALPHABET.len()] as char);
    }
    out
}

/// 기본 핸들(09-06 사용자 확정 "기본 UserId·암호 자동 지정") — 정제된 표시 이름이 형식에 맞으면
/// 그것, 아니면 `beep-{지문8}`(실명 비노출 — DR-22와 같은 결).
#[must_use]
pub(crate) fn default_handle(display_name: &str, peer_short: &str) -> String {
    let d = normalize_handle(display_name);
    if handle_ok(d) {
        d.to_string()
    } else {
        format!("beep-{peer_short}")
    }
}

/// 인증 상태 — 설정 창 노트·기능 게이트의 단일 원천.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UserState {
    /// 필수 값 없음(또는 형식 위반) — 기능·하위 설정 잠금.
    Unconfigured(Validity),
    /// 값은 유효하나 검증 전(값 변경 포함) — 테스트 가능 · 기능 중지.
    Untested,
    /// 테스트 진행 중(워커) — 버튼 잠금.
    Testing,
    /// 인증됨 — 기능 활성. 지문은 표시용.
    Verified { user_id: UserId },
    /// 실패 — 기능 전부 중지 · 사유 표시 · 값을 고쳐 다시 테스트.
    Failed(String),
}

impl UserState {
    /// 인증 의존 기능을 돌려도 되는가.
    #[must_use]
    #[allow(dead_code)] // S1(형제 세션·RID 등록)이 소비한다
    pub(crate) fn active(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }

    /// 테스트 버튼 활성 조건 — 값이 유효하고 진행 중이 아닐 때(이미 인증됐어도 재테스트 허용).
    #[must_use]
    pub(crate) fn can_test(&self) -> bool {
        matches!(
            self,
            Self::Untested | Self::Verified { .. } | Self::Failed(_)
        )
    }
}

/// 값(핸들·암호)이 바뀌었을 때의 다음 상태 — 검증 마커는 값에 붙으므로 **항상** 내려간다.
#[must_use]
pub(crate) fn on_values_changed(handle: &str, pass: &str) -> UserState {
    match validate(handle, pass) {
        Validity::Ok => UserState::Untested,
        v => UserState::Unconfigured(v),
    }
}

/// 부팅 시 상태 — 값이 유효하고 마커가 켜져 있으면 재검증(워커)으로 들어간다.
#[must_use]
pub(crate) fn on_boot(handle: &str, pass: &str, verified_marker: bool) -> UserState {
    match validate(handle, pass) {
        Validity::Ok if verified_marker => UserState::Testing,
        Validity::Ok => UserState::Untested,
        v => UserState::Unconfigured(v),
    }
}

/// 테스트 결과 반영.
#[must_use]
pub(crate) fn on_test_result(result: &Result<UserId, String>) -> UserState {
    match result {
        Ok(user_id) => UserState::Verified { user_id: *user_id },
        Err(e) => UserState::Failed(e.clone()),
    }
}

/// 런타임 신원 재료 — 인증 성공 시에만 채워지고, 값이 바뀌면 통째로 비운다.
#[derive(Default)]
pub(crate) struct UserRuntime {
    pub(crate) material: Option<KeyMaterial>,
    pub(crate) key: Option<UserKey>,
    /// 쥔 키의 생성 시각(unix ms) — 형제 병합의 승자 기준(D-32-8 · 오래된 쪽).
    pub(crate) key_created: u64,
}

impl UserRuntime {
    /// 값 변경·실패 = 인증 의존 재료(KP)만 비운다. **UserKey는 쥐고 있는다** — 다음 인증이 새 암호로
    /// 재래핑할 근거(암호 변경 ≠ 신원 변경). 키를 버리는 것은 "신원에서 분리"뿐.
    pub(crate) fn clear(&mut self) {
        self.material = None;
    }
}

/// 테스트 성공 산출물 — 워커 → 앱(이벤트 페이로드).
#[derive(Debug)]
pub(crate) struct TestOk {
    pub(crate) material: KeyMaterial,
    pub(crate) key: UserKey,
    /// 이번에 `user.key`를 새로 만들었는가(첫 기기 부트스트랩).
    pub(crate) created: bool,
    /// 키 생성 시각(unix ms · 봉인 페이로드 꼬리 8B — 구본 64B는 지금 시각으로 승격).
    pub(crate) created_at: u64,
}

/// 봉인 페이로드 = `[UserKey 64][created_at u64 BE]`(구본 = 64B만 → `None` 시각).
pub(crate) fn key_payload(key: &UserKey, created_at: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(USERKEY_LEN + 8);
    v.extend_from_slice(&key.to_bytes());
    v.extend_from_slice(&created_at.to_be_bytes());
    v
}

/// 페이로드 해석 — 64B(구본 · 시각 없음) 또는 72B. 그 외 = 손상.
pub(crate) fn parse_key_payload(plain: &[u8]) -> Option<(UserKey, Option<u64>)> {
    match plain.len() {
        USERKEY_LEN => UserKey::from_bytes(plain).map(|k| (k, None)),
        n if n == USERKEY_LEN + 8 => {
            let key = UserKey::from_bytes(&plain[..USERKEY_LEN])?;
            let mut t = [0u8; 8];
            t.copy_from_slice(&plain[USERKEY_LEN..]);
            Some((key, Some(u64::from_be_bytes(t))))
        }
        _ => None,
    }
}

/// 봉인본 생성(파일·와이어 공용 — 같은 도메인·같은 열쇠라 형제가 파일로 그대로 쓸 수 있다).
pub(crate) fn seal_key(
    seal_domain: &[u8],
    wrap: &[u8; 32],
    key: &UserKey,
    created_at: u64,
) -> Result<Vec<u8>, String> {
    nbeep_store::sealed::seal(seal_domain, wrap, &key_payload(key, created_at))
        .map_err(|e| format!("user.key 봉인 실패: {e}"))
}

/// 봉인본 열기(파일·와이어 공용).
pub(crate) fn open_key(
    seal_domain: &[u8],
    wrap: &[u8; 32],
    sealed: &[u8],
) -> Option<(UserKey, Option<u64>)> {
    let plain = nbeep_store::sealed::open(seal_domain, wrap, sealed)?;
    parse_key_payload(&plain)
}

/// 형제 병합 규칙(D-32-8 · 순수): **오래된 키가 이긴다** · 같은 시각이면 공개키 사전순 작은 쪽.
/// 참 = 상대 키를 채택한다. 같은 키면 거짓(할 일 없음).
#[must_use]
pub(crate) fn adopt_theirs(mine: (u64, [u8; 32]), theirs: (u64, [u8; 32])) -> bool {
    if mine.1 == theirs.1 {
        return false;
    }
    theirs.0 < mine.0 || (theirs.0 == mine.0 && theirs.1 < mine.1)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// 워커 본체 — KP 파생(60k · 수십 ms) → `user.key` 봉인 파일 열기/생성/재래핑.
/// - 열림 = 같은 두 값(재기동 · 재인증).
/// - 안 열리는데 **쥔 키(`held`)가 있다** = 암호·핸들 변경 → 새 열쇠로 **재래핑**(재암호화 없음 ·
///   UserId 불변 — ADR-0015 3층 · 47 §3-1).
/// - 안 열리고 쥔 키도 없다 = 재기동 뒤 다른 암호 → **fail-closed**(이 기기 키를 열 수 없다).
/// - 파일 없음 = 쥔 키를 봉인하거나(있으면) 새로 만든다(첫 기기 부트스트랩).
///
/// # Errors
/// 사용자에게 보일 한 줄 사유(비밀 없음).
pub(crate) fn derive_and_load(
    handle: &str,
    pass: &str,
    key_path: &Path,
    seal_domain: &[u8],
    held: Option<(&UserKey, u64)>,
) -> Result<TestOk, String> {
    let material = KeyMaterial::derive(normalize_handle(handle), pass)
        .ok_or_else(|| "핸들과 페어링 암호가 모두 필요합니다".to_string())?;
    let wrap = material.wrap_key();
    let seal_to_file = |key: &UserKey, created_at: u64| -> Result<(), String> {
        let env = seal_key(seal_domain, &wrap, key, created_at)?;
        debug_assert!(env.len() > USERKEY_LEN);
        nbeep_store::privfile::write_atomic(key_path, &env)
            .map_err(|e| format!("user.key 저장 실패: {e}"))
    };
    let (held_key, held_at) = match held {
        Some((k, at)) => (Some(k), at),
        None => (None, 0),
    };
    match std::fs::read(key_path) {
        Ok(bytes) => {
            if nbeep_store::sealed::open(seal_domain, &wrap, &bytes).is_some() {
                let (key, at) = open_key(seal_domain, &wrap, &bytes)
                    .ok_or_else(|| "user.key: 손상된 사용자 키(덮어쓰지 않음)".to_string())?;
                let created_at = match at {
                    Some(t) => t,
                    None => {
                        // 구본(시각 없음) — 지금 시각으로 승격해 다시 봉인(자기 치유 1회).
                        let t = now_ms();
                        seal_to_file(&key, t)?;
                        t
                    }
                };
                return Ok(TestOk {
                    material,
                    key,
                    created: false,
                    created_at,
                });
            }
            if let Some(k) = held_key {
                seal_to_file(k, held_at)?;
                return Ok(TestOk {
                    material,
                    key: k.clone(),
                    created: false,
                    created_at: held_at,
                });
            }
            Err("user.key: 이 기기의 사용자 키가 다른 암호로 봉인되어 있습니다".to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let (key, created, created_at) = match held_key {
                Some(k) => (k.clone(), false, held_at),
                None => (
                    UserKey::generate().map_err(|e| format!("난수원 실패: {e}"))?,
                    true,
                    now_ms(),
                ),
            };
            seal_to_file(&key, created_at)?;
            Ok(TestOk {
                material,
                key,
                created,
                created_at,
            })
        }
        Err(e) => Err(format!("user.key 읽기 실패: {e}")),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn handle_rules() {
        assert!(handle_ok("kiros33"));
        assert!(handle_ok("a.b_c-d"));
        assert!(handle_ok("kiros33@mac"), "기본 핸들 형식");
        assert!(!handle_ok("ab"), "3자 미만");
        assert!(!handle_ok(&"x".repeat(33)), "32자 초과");
        assert!(!handle_ok("kir os"), "공백 금지");
        assert!(!handle_ok("한글"), "ASCII만(방송 라벨)");
        assert_eq!(normalize_handle("  kiros33 "), "kiros33");
    }

    #[test]
    fn validate_order_empty_first() {
        assert_eq!(validate("", ""), Validity::Empty);
        assert_eq!(validate("kiros33", ""), Validity::Empty);
        assert_eq!(validate("", "longenough1"), Validity::Empty);
        assert_eq!(validate("k s", "longenough1"), Validity::BadHandle);
        assert_eq!(validate("kiros33", "short"), Validity::ShortPass);
        assert_eq!(validate(" kiros33 ", "longenough1"), Validity::Ok);
    }

    #[test]
    fn state_machine_gates() {
        let s = on_values_changed("kiros33", "abcd-efgh-jkmn");
        assert_eq!(s, UserState::Untested);
        assert!(s.can_test() && !s.active());
        let s = on_values_changed("", "abcd-efgh-jkmn");
        assert!(matches!(s, UserState::Unconfigured(Validity::Empty)));
        assert!(!s.can_test() && !s.active());
        assert!(!UserState::Testing.can_test());
        let ok = on_test_result(&Ok(UserId::from_bytes([7u8; 32])));
        assert!(ok.active() && ok.can_test(), "인증 뒤에도 재테스트는 허용");
        let bad = on_test_result(&Err("x".into()));
        assert!(!bad.active() && bad.can_test());
        assert_eq!(
            on_boot("kiros33", "abcd-efgh-jkmn", true),
            UserState::Testing
        );
        assert_eq!(
            on_boot("kiros33", "abcd-efgh-jkmn", false),
            UserState::Untested
        );
        assert!(matches!(
            on_boot("kiros33", "", true),
            UserState::Unconfigured(Validity::Empty)
        ));
    }

    #[test]
    fn strength_and_suggestion() {
        assert_eq!(pass_grade("aaaaaaaa"), 0);
        assert_eq!(pass_grade("abcd-efgh-jkmn"), 1, "추천 형식 ≈ 59비트 = 보통");
        assert_eq!(pass_grade("Correct-Horse-Battery-9"), 2);
        assert_eq!(default_handle("kiros33@mac", "be140131"), "kiros33@mac");
        assert_eq!(default_handle("홍 길동", "be140131"), "beep-be140131");
        let s = suggest_passphrase();
        assert_eq!(s.chars().count(), PASS_SUGGEST_LEN + 2);
        assert!(s
            .chars()
            .all(|c| c == '-' || "abcdefghjkmnpqrstuvwxyz23456789".contains(c)));
        assert_ne!(s, suggest_passphrase(), "난수");
        assert_eq!(validate("kiros33", &s), Validity::Ok);
    }

    #[test]
    fn key_payload_legacy_and_merge_rule() {
        let k = UserKey::generate().unwrap();
        let (k2, at) = parse_key_payload(&key_payload(&k, 1234)).unwrap();
        assert_eq!(k2.user_id(), k.user_id());
        assert_eq!(at, Some(1234));
        let (k3, at3) = parse_key_payload(&k.to_bytes()).unwrap();
        assert_eq!(k3.user_id(), k.user_id());
        assert_eq!(at3, None, "구본 64B = 시각 없음");
        assert!(parse_key_payload(&[0u8; 70]).is_none());
        // 봉인 왕복(파일·와이어 공용).
        let wrap = [5u8; 32];
        let sealed = seal_key(b"t", &wrap, &k, 77).unwrap();
        let (k4, at4) = open_key(b"t", &wrap, &sealed).unwrap();
        assert_eq!(k4.user_id(), k.user_id());
        assert_eq!(at4, Some(77));
        assert!(
            open_key(b"t", &[6u8; 32], &sealed).is_none(),
            "다른 열쇠 = 안 열림"
        );
        // 병합 규칙 — 오래된 키 승 · 동시각은 공개키 사전순 · 같은 키는 no-op.
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert!(adopt_theirs((200, a), (100, b)));
        assert!(!adopt_theirs((100, a), (200, b)));
        assert!(adopt_theirs((100, b), (100, a)));
        assert!(!adopt_theirs((100, a), (100, b)));
        assert!(!adopt_theirs((100, a), (50, a)), "같은 키 = 할 일 없음");
    }

    #[test]
    fn derive_and_load_creates_then_reopens_and_rejects_other_pass() {
        let d = std::env::temp_dir().join(format!("nb-userident-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("user.key");
        let dom = b"user-key-test";
        let t1 = derive_and_load("kiros33", "abcd-efgh-jkmn", &p, dom, None).unwrap();
        assert!(t1.created);
        let t2 = derive_and_load("kiros33", "abcd-efgh-jkmn", &p, dom, None).unwrap();
        assert!(!t2.created);
        assert_eq!(
            t1.key.user_id(),
            t2.key.user_id(),
            "같은 두 값 = 같은 UserKey 재개봉"
        );
        assert_eq!(t1.material.psk(), t2.material.psk());
        assert!(
            derive_and_load("kiros33", "", &p, dom, None).is_err(),
            "빈 암호 = 거부"
        );
        let err = derive_and_load("kiros33", "other-pass-9999", &p, dom, None).unwrap_err();
        assert!(err.contains("다른 암호"), "{err}");
        // ★ 쥔 키가 있으면 새 암호로 재래핑 — UserId 불변(암호 변경 ≠ 신원 변경).
        let t3 = derive_and_load(
            "kiros33",
            "other-pass-9999",
            &p,
            dom,
            Some((&t1.key, t1.created_at)),
        )
        .unwrap();
        assert_eq!(t3.key.user_id(), t1.key.user_id());
        assert!(!t3.created);
        assert_eq!(t3.created_at, t1.created_at, "재래핑은 생성 시각을 보존");
        assert!(t1.created_at > 0, "첫 생성 = 지금 시각");
        let t4 = derive_and_load("kiros33", "other-pass-9999", &p, dom, None).unwrap();
        assert_eq!(
            t4.key.user_id(),
            t1.key.user_id(),
            "재래핑 뒤엔 새 암호로 열린다"
        );
        assert!(
            derive_and_load("kiros33", "abcd-efgh-jkmn", &p, dom, None).is_err(),
            "옛 암호는 더는 못 연다"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(d);
    }
}
