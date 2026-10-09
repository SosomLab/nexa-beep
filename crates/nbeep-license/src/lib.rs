//! `nbeep-license` — Nexa Beep 라이선스 **앱 층**(docs/50 P4 · D-33-6·7).
//! 출처: nexa-dir3 `crates/ndir-license/src/lib.rs`(← nexa-sql `crates/nsql-license/src/lib.rs` 1:1 복제) — 제품·폴더 주입만 다르다.
//!
//! 정책(D-33-6): **무료(비상업) = 파일 없이 Free · 상업 사용은 라이선스 필요(PolyForm NC가 정한다 · 앱은 전화홈 0) ·
//! 기능 게이트 0** → [`Feature`]는 변형이 없는 열거형이다(`check`·`entitlement` API는 nexa-sql과 같은 모양으로 남겨 두어,
//! 상업 전용 기능이 정해지면(후속 결정 — 후보: 릴레이 Managed · 그룹 N명 초과 · 컨텐츠 모드) 변형만 더한다).
//! 게이트 스위치 `license.gates`(기본 off · HIDDEN)는 앱 설정 몫이다.
//!
//! 폴더(D-33-7): 사용자 = **앱이 주는 데이터 폴더**`/license`(beep `data_dir()` — 포터블 = 실행 파일 옆 · DR-4) →
//! 기기 공용(Win `%ProgramData%\nexa\nexa-beep` · mac `/Library/Application Support/nexa/nexa-beep` · Linux `/etc/nexa/nexa-beep`).
//! 이 크레이트는 데이터 폴더를 스스로 정하지 않는다(경로 규칙의 단일 원천 = 앱).
//!
//! 라이브러리 `nexa-license`가 형식·서명·기기 ID·파일 자리를 주고, 이 크레이트는 **앱이 아는 것**만 얹는다:
//! 제품 기술자([`PRODUCT`] · 빌드일 = R2) · [`Feature`] 열거(문자열은 파일 매핑 한 곳) · [`LicenseState`] ·
//! **유일한 판정 함수 [`Licensing::check`]**(순수 · I/O 없음) · 폴더 순서(사용자 → 기기 공용 · R4) · 변경 서명 재판정(R1) ·
//! 요청 코드. 게이트 위치 규칙(23 §4-2 · 기능당 정확히 한 곳 · UI·CLI 입구)은 호출측 몫이다.
//!
//! 판정 상태 넷(Free · Invalid · Outdated · Expired)은 **전부 features = ∅** — 차이는 안내 문구뿐(23 §4-1).
//! 루트 공개키가 아직 비어 있으면(`ROOT_KEYS` · 발급 PC keygen 전) 모든 파일은 `Invalid(NoRootKey)` = Free로 취급된다.
//!
//! 문서 번호(23·25·92 §…)는 nexa-sql docs다(출처 크레이트의 근거 · beep 근거 = docs/50).
//!
//! 로그 규칙(nexa-sql docs/92 §4): 이 크레이트는 `licensee`·이메일·기기 ID 원문을 **어디에도 쓰지 않는다** — 표시는 호출측이, 기기 코드는 base32만.

// 테스트 코드는 unwrap 허용(docs/13 §9 — 금지는 프로덕션 경로 한정 · nbeep-ui와 같은 규칙).
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::{
    collections::BTreeSet,
    fmt, io,
    path::{Path, PathBuf},
};

/// `nexa-beep --license …` CLI 본체(앱은 인자 분기만 · docs/50 D-33-7).
pub mod cli;

pub use nexa_license::request;
use nexa_license::{
    date,
    ed25519::Ed25519Verifier,
    format::Doc,
    fs::{Stamp, Store},
    machine,
    verify::{verify_license, Verdict},
};
pub use nexa_license::{
    keys::{RootKey, ROOT_KEYS},
    types::{Kind, Product, SeatMode},
    verify::{Invalid, License},
};

/// 이 앱의 제품 기술자 — `product=` 값 · 빌드일(`build.rs` `NBEEP_BUILD_DATE`).
pub const PRODUCT: Product = Product {
    id: "nexa-beep",
    build_date: env!("NBEEP_BUILD_DATE"),
    // `max_major`(Major 바뀌면 무효) · `max_version`(그 버전부터 무효) 판정 기준(09-27).
    version: env!("CARGO_PKG_VERSION"),
};

/// 라이선스 폴더 이름(`<데이터 폴더>/license/`).
pub const LICENSE_SUBDIR: &str = "license";

/// 요청 코드를 보내는 곳(사용자 09-27 "어느 이메일로 보내는지 정보가 없다") — 창·CLI·위키가 이 한 곳을 쓴다.
pub const LICENSE_CONTACT: &str = "kiros33@sosomlab.com";

// ─────────────────────────────────────────────────────────────── Feature

/// 게이트 대상 기능 — **코드에 열거**. D-33-6(기능 게이트 0)에 따라 지금은 **변형이 없다**. 유료 기능이 정해지면 여기
/// 변형 + [`Feature::as_str`] 이름 + 게이트 호출 한 곳을 더한다. 파일에 모르는 이름이 있어도 무시(전방 호환).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {}

impl Feature {
    /// 전 목록(표시·시험용).
    pub const ALL: &'static [Feature] = &[];

    /// 파일의 `features=` 이름.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {}
    }

    /// 이름 → 기능(모르는 이름 = None · 무시).
    #[must_use]
    pub fn parse(s: &str) -> Option<Feature> {
        Feature::ALL.iter().copied().find(|f| f.as_str() == s)
    }

    /// Org 전용.
    #[must_use]
    pub const fn is_org_only(self) -> bool {
        match self {}
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 표시용 등급(`tier=` · 판정에는 쓰지 않는다 — 판정은 `features` 낱개).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tier {
    #[default]
    Free,
    Trial,
    Pro,
    Org,
}

impl Tier {
    #[must_use]
    pub fn parse(s: &str) -> Tier {
        match s.trim().to_ascii_lowercase().as_str() {
            "trial" => Tier::Trial,
            "pro" | "team" => Tier::Pro,
            "org" | "organization" | "enterprise" => Tier::Org,
            _ => Tier::Free,
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Tier::Free => "free",
            Tier::Trial => "trial",
            Tier::Pro => "pro",
            Tier::Org => "org",
        }
    }
}

// ─────────────────────────────────────────────────────────────── 상태 · 판정

/// 라이선스 상태(23 §1-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseState {
    /// 파일 없음(오류 아님).
    Free,
    Licensed(License),
    /// 파싱·서명·제품·기기 실패 — Free로 취급 + 안내.
    Invalid(Invalid),
    /// 영구 모델: 이 빌드가 `updates_until` 뒤 — 그 전 판만 정식.
    Outdated(License),
    /// 만료형: 오늘이 `expires` 뒤.
    Expired(License),
}

impl LicenseState {
    /// 정식(features 유효) 상태인가.
    #[must_use]
    pub fn is_licensed(&self) -> bool {
        matches!(self, LicenseState::Licensed(_))
    }
    /// 붙어 있는 라이선스 내용(정식이 아니어도 표시용으로).
    #[must_use]
    pub fn license(&self) -> Option<&License> {
        match self {
            LicenseState::Licensed(l) | LicenseState::Outdated(l) | LicenseState::Expired(l) => {
                Some(l)
            }
            LicenseState::Free | LicenseState::Invalid(_) => None,
        }
    }
    /// 표시용 등급.
    #[must_use]
    pub fn tier(&self) -> Tier {
        match self {
            LicenseState::Licensed(l) => Tier::parse(&l.tier),
            _ => Tier::Free,
        }
    }
    /// 짧은 상태 이름(로그·상태줄 키 · 사용자 문자열은 호출측 i18n).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            LicenseState::Free => "free",
            LicenseState::Licensed(_) => "licensed",
            LicenseState::Invalid(_) => "invalid",
            LicenseState::Outdated(_) => "outdated",
            LicenseState::Expired(_) => "expired",
        }
    }
}

/// 거부 사유 — 안내 문구를 고르는 열쇠(문구는 호출측 i18n).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// 라이선스 없음.
    Free,
    Invalid(Invalid),
    Outdated,
    Expired,
    /// 정식이지만 이 기능은 `features`에 없다.
    NotIncluded,
}

/// 거부 상세.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Denial {
    pub feature: Feature,
    pub reason: DenyReason,
}

/// 판정 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entitlement {
    Allowed,
    Denied(Denial),
}

impl Entitlement {
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Entitlement::Allowed)
    }
}

/// 순수 판정 — 상태 × 기능 → 결과(I/O 없음 · 시험 가능).
#[must_use]
pub fn entitlement(state: &LicenseState, feature: Feature) -> Entitlement {
    let reason = match state {
        LicenseState::Licensed(l) => {
            if l.allows(feature.as_str()) {
                return Entitlement::Allowed;
            }
            DenyReason::NotIncluded
        }
        LicenseState::Free => DenyReason::Free,
        LicenseState::Invalid(i) => DenyReason::Invalid(*i),
        LicenseState::Outdated(_) => DenyReason::Outdated,
        LicenseState::Expired(_) => DenyReason::Expired,
    };
    Entitlement::Denied(Denial { feature, reason })
}

/// 본문 → 상태(순수 · `machine`/`today` 주입 · 시험용 입구). 루트 키 목록도 주입(앱은 [`ROOT_KEYS`]).
#[must_use]
pub fn judge(roots: &[RootKey], text: &str, machine: Option<&[u8]>, today: i64) -> LicenseState {
    match verify_license(&PRODUCT, roots, &Ed25519Verifier, text, machine, today) {
        Verdict::Licensed(l) => LicenseState::Licensed(l),
        Verdict::Outdated(l) => LicenseState::Outdated(l),
        Verdict::Expired(l) => LicenseState::Expired(l),
        Verdict::Invalid(i) => LicenseState::Invalid(i),
    }
}

// ─────────────────────────────────────────────────────────────── 폴더

/// 사용자 라이선스 폴더 = `<데이터 폴더>/license` — 데이터 폴더는 **앱이 준다**(beep `data_dir()` · 포터블 = 실행 파일 옆).
#[must_use]
pub fn user_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(LICENSE_SUBDIR)
}

/// 기기 공용 폴더(Device 라이선스 자리 · docs/25 §11-2) — 읽기만. 설치는 관리자가 손으로(권한).
#[must_use]
pub fn machine_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("nexa").join(PRODUCT.id))
    }
    #[cfg(target_os = "macos")]
    {
        Some(
            PathBuf::from("/Library/Application Support")
                .join("nexa")
                .join(PRODUCT.id),
        )
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Some(PathBuf::from("/etc/nexa").join(PRODUCT.id))
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// 판정 순서의 폴더 목록(사용자 → 기기 공용).
#[must_use]
pub fn default_dirs(data_dir: &Path) -> Vec<PathBuf> {
    std::iter::once(user_dir(data_dir))
        .chain(machine_dir())
        .collect()
}

// ─────────────────────────────────────────────────────────────── Licensing

/// 설치 실패.
#[derive(Debug)]
pub enum InstallError {
    Io(io::Error),
    /// 검증을 통과하지 못해 **쓰지 않았다**(무효·기한 지남·다른 기기). Box = clippy `large_enum_variant`(License 232B).
    Rejected(Box<LicenseState>),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::Io(e) => write!(f, "io: {e}"),
            InstallError::Rejected(s) => write!(f, "rejected: {}", s.name()),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self {
        InstallError::Io(e)
    }
}

/// 요청 코드 메타(표시·대장용 · 서명 대상 아님).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestMeta {
    pub name: String,
    pub email: String,
}

/// 앱의 라이선스 문맥 — 프로세스에 하나. 상태는 캐시하고 [`Licensing::refresh`]가 변경 서명(mtime·길이)으로 재판정한다(23 §1-4 · R1).
#[derive(Debug)]
pub struct Licensing {
    store: Store,
    roots: &'static [RootKey],
    state: LicenseState,
    path: Option<PathBuf>,
    stamp: Option<Stamp>,
}

impl Licensing {
    /// 기본 자리(사용자 → 기기 공용) + 내장 루트 키. 파일 없음 = Free(오류 아님).
    #[must_use]
    pub fn open_default(data_dir: &Path) -> Licensing {
        Licensing::with_roots(default_dirs(data_dir), ROOT_KEYS)
    }

    /// 폴더를 지정(격리·시험).
    #[must_use]
    pub fn open(dirs: Vec<PathBuf>) -> Licensing {
        Licensing::with_roots(dirs, ROOT_KEYS)
    }

    /// 폴더 + 루트 키 주입(시험 픽스처).
    #[must_use]
    pub fn with_roots(dirs: Vec<PathBuf>, roots: &'static [RootKey]) -> Licensing {
        let mut l = Licensing {
            store: Store::new(PRODUCT, dirs),
            roots,
            state: LicenseState::Free,
            path: None,
            stamp: None,
        };
        l.reload();
        l
    }

    /// 지금 상태(캐시).
    #[must_use]
    pub fn state(&self) -> &LicenseState {
        &self.state
    }

    /// 판정에 쓴 파일(없으면 None).
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 설치 자리(첫 폴더의 파일 경로).
    #[must_use]
    pub fn primary_path(&self) -> Option<PathBuf> {
        self.store.primary_path()
    }

    /// 판정 폴더 순서.
    #[must_use]
    pub fn dirs(&self) -> &[PathBuf] {
        self.store.dirs()
    }

    /// ★ 유일한 판정 함수 — 순수(I/O 없음). 게이트는 이것만 부른다.
    #[must_use]
    pub fn check(&self, feature: Feature) -> Entitlement {
        entitlement(&self.state, feature)
    }

    /// 파일이 바뀌었으면(경로·mtime·길이) 다시 판정. 바뀌었으면 `true`.
    pub fn refresh(&mut self) -> bool {
        let now = self.store.stamp();
        let same = match (&now, &self.path, &self.stamp) {
            (None, None, _) => true,
            (Some((p, s)), Some(q), Some(t)) => p == q && s == t,
            _ => false,
        };
        if same {
            return false;
        }
        self.reload();
        true
    }

    /// 본문을 이 기기·오늘 기준으로 판정(파일은 건드리지 않음).
    #[must_use]
    pub fn judge_text(&self, text: &str) -> LicenseState {
        let m = machine::machine_id();
        judge(self.roots, text, m.as_deref(), date::today_days())
    }

    /// 파일에서 설치 — **검증 통과(Licensed)만** 쓴다. 원본은 그대로.
    pub fn install(&mut self, file: &Path) -> Result<License, InstallError> {
        let text = std::fs::read_to_string(file)?;
        self.install_text(&text)
    }

    /// 본문에서 설치(붙여넣기·드롭).
    pub fn install_text(&mut self, text: &str) -> Result<License, InstallError> {
        match self.judge_text(text) {
            LicenseState::Licensed(l) => {
                self.store.install_text(text)?;
                self.reload();
                Ok(l)
            }
            other => Err(InstallError::Rejected(Box::new(other))),
        }
    }

    /// 사용자 폴더의 파일 삭제(기기 공용 파일은 남는다 → 그것으로 재판정). 없었으면 `Ok(false)`.
    pub fn remove(&mut self) -> io::Result<bool> {
        let r = self.store.remove()?;
        self.reload();
        Ok(r)
    }

    /// 이 PC의 기기 코드(base32 · 20B) — 원문은 나가지 않는다.
    #[must_use]
    pub fn machine_code() -> Option<String> {
        machine::machine_id().map(|m| nexa_license::base32::encode(&m))
    }

    /// 요청 코드 한 줄(`NEXAREQ1.<기기>.<메타>` · 23 §1-1 · 25 §12-2). 메타 = os · app · d(오늘) · n · e.
    #[must_use]
    pub fn request_code(meta: &RequestMeta) -> Option<String> {
        let m = machine::machine_id()?;
        let mut d = Doc::default();
        d.set("os", std::env::consts::OS);
        d.set("app", concat!("nexa-beep/", env!("CARGO_PKG_VERSION")));
        d.set("d", &date::format_days(date::today_days()));
        if !meta.name.trim().is_empty() {
            d.set("n", meta.name.trim());
        }
        if !meta.email.trim().is_empty() {
            d.set("e", meta.email.trim());
        }
        Some(request::encode(&m, &d))
    }

    fn reload(&mut self) {
        match self.store.read() {
            Ok(Some(found)) => {
                self.state = self.judge_text(&found.text);
                self.path = Some(found.path);
                self.stamp = Some(found.stamp);
            }
            Ok(None) => {
                self.state = LicenseState::Free;
                self.path = None;
                self.stamp = None;
            }
            Err(_) => {
                // 읽기 오류(권한 등) = 파일이 있는데 못 읽음 → 무효로 안내(Free보다 정직).
                self.state = LicenseState::Invalid(Invalid::Malformed);
                self.path = self.store.locate();
                self.stamp = None;
            }
        }
    }
}

/// 정식 라이선스의 기능 집합을 [`Feature`]로(표시용 · 모르는 이름은 버린다 · `*`는 전부).
#[must_use]
pub fn features_of(l: &License) -> BTreeSet<Feature> {
    if l.features.contains("*") {
        return Feature::ALL.iter().copied().collect();
    }
    l.features
        .iter()
        .filter_map(|s| Feature::parse(s))
        .collect()
}

// ─────────────────────────────────────────────────────────────── 시험(docs/23 §5)

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use nexa_license::{base32, types::Alg, LICENSE_DOMAIN};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let d = std::env::temp_dir().join(format!(
            "nbeep-license-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&d).expect("temp dir");
        d
    }
    fn keypair() -> (SigningKey, &'static [RootKey]) {
        let sk = SigningKey::generate(&mut rand_core::OsRng);
        let pk: &'static [u8] =
            Box::leak(sk.verifying_key().to_bytes().to_vec().into_boxed_slice());
        let roots: &'static [RootKey] = Box::leak(
            vec![RootKey {
                id: "test-v1",
                alg: Alg::Ed25519,
                public: pk,
            }]
            .into_boxed_slice(),
        );
        (sk, roots)
    }
    /// 이 기기용 문서(기기 ID를 못 얻는 환경이면 기기 없는 Org형).
    fn signed(sk: &SigningKey, extra: &[(&str, &str)]) -> String {
        let mut d = Doc::default();
        for (k, v) in [
            ("format", "nxl1"),
            ("product", "nexa-beep"),
            ("id", "NBL-2026-000001"),
            ("licensee", "ACME"),
            ("kind", "user"),
            ("tier", "pro"),
            ("features", "unknown-future"),
            ("issued", "2026-09-27"),
            ("updates_until", "2999-12-31"),
            ("max_major", "0"),
            ("note", "unknown key is ignored"),
        ] {
            d.set(k, v);
        }
        if let Some(m) = Licensing::machine_code() {
            d.set("machine", &m);
        }
        for (k, v) in extra {
            d.set(k, v);
        }
        let sig = sk.sign(&d.canonical(LICENSE_DOMAIN));
        d.set("sig", &base32::encode(&sig.to_bytes()));
        d.serialize()
    }

    #[test]
    fn feature_names_round_trip_and_are_unique() {
        let mut seen = BTreeSet::new();
        for f in Feature::ALL {
            assert!(seen.insert(f.as_str()), "중복 이름 {f}");
            assert_eq!(Feature::parse(f.as_str()), Some(*f));
        }
        assert_eq!(Feature::parse("no-such"), None);
        assert!(Feature::ALL.is_empty(), "D-33-6: 기능 게이트 0");
        assert_eq!(Tier::parse("PRO"), Tier::Pro);
        assert_eq!(Tier::parse("whatever"), Tier::Free);
        assert!(
            PRODUCT.build_date.len() == 10,
            "build.rs 날짜 {}",
            PRODUCT.build_date
        );
    }

    #[test]
    fn no_file_or_dir_is_free_not_error() {
        let base = tmp("free");
        let l = Licensing::open(vec![base.join("missing"), base.join("also")]);
        assert_eq!(*l.state(), LicenseState::Free);
        assert_eq!(l.path(), None);
        assert_eq!(l.state().name(), "free");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn install_then_check_allowed_and_denied() {
        let base = tmp("install");
        let (sk, roots) = keypair();
        let src = base.join("mail.license");
        std::fs::write(&src, signed(&sk, &[])).expect("src");
        let mut l = Licensing::with_roots(vec![base.join("u")], roots);
        let lic = l.install(&src).expect("install");
        assert_eq!(lic.id, "NBL-2026-000001");
        assert!(l.state().is_licensed());
        assert_eq!(l.state().tier(), Tier::Pro);
        assert_eq!(l.state().name(), "licensed");
        let fs = features_of(&lic);
        assert!(fs.is_empty(), "모르는 feature 이름은 버린다: {fs:?}");
        // 새 프로세스가 같은 폴더를 열면 같은 판정.
        let again = Licensing::with_roots(vec![base.join("u")], roots);
        assert!(again.state().is_licensed());
        // 제거 → Free.
        assert!(l.remove().expect("remove"));
        assert_eq!(*l.state(), LicenseState::Free);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn star_means_everything_including_org() {
        let base = tmp("star");
        let (sk, roots) = keypair();
        let mut l = Licensing::with_roots(vec![base.join("u")], roots);
        l.install_text(&signed(&sk, &[("features", "*")]))
            .expect("install");
        assert!(l.state().is_licensed());
        assert_eq!(
            features_of(l.state().license().expect("lic")).len(),
            Feature::ALL.len()
        );
        assert!(
            l.state().license().expect("lic").allows("anything"),
            "`*` = 전부"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn tamper_is_rejected_but_reorder_and_crlf_pass() {
        let base = tmp("tamper");
        let (sk, roots) = keypair();
        let text = signed(&sk, &[]);
        let mut l = Licensing::with_roots(vec![base.join("u")], roots);
        // 값 변조.
        let bad = text.replace("licensee=ACME", "licensee=EVIL");
        match l.install_text(&bad) {
            Err(InstallError::Rejected(s)) if *s == LicenseState::Invalid(Invalid::Signature) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(*l.state(), LicenseState::Free, "거부 = 쓰지 않았다");
        // 순서 교란 + CRLF + 주석.
        let mut lines: Vec<&str> = text.lines().collect();
        lines.reverse();
        let shuffled = format!("# comment\r\n{}\r\n", lines.join("\r\n"));
        assert!(l.install_text(&shuffled).is_ok());
        assert!(l.state().is_licensed());
        // 다른 루트 키로 서명한 파일은 거부.
        let (other_sk, _) = keypair();
        match l.judge_text(&signed(&other_sk, &[])) {
            LicenseState::Invalid(Invalid::Signature) => {}
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn other_machine_outdated_expired_and_wrong_product() {
        let base = tmp("states");
        let (sk, roots) = keypair();
        let l = Licensing::with_roots(vec![base.join("u")], roots);
        let other = base32::encode(&[7u8; 20]);
        match l.judge_text(&signed(&sk, &[("machine", &other)])) {
            LicenseState::Invalid(Invalid::Machine) => {}
            other => panic!("{other:?}"),
        }
        let s = l.judge_text(&signed(&sk, &[("updates_until", "2000-01-01")]));
        assert!(matches!(s, LicenseState::Outdated(_)), "{s:?}");
        assert_eq!(s.name(), "outdated");
        assert_eq!(s.tier(), Tier::Free, "features ∅ = 표시도 Free");
        let s = l.judge_text(&signed(&sk, &[("expires", "2000-01-01")]));
        assert!(matches!(s, LicenseState::Expired(_)), "{s:?}");
        // 버전 조항: 앱 0.0.1 · max_version 0.0.1 = 그 버전부터 무효 · max_major 없음(none) = 정식.
        let s = l.judge_text(&signed(&sk, &[("max_version", "0.0.1")]));
        assert!(matches!(s, LicenseState::Outdated(_)), "{s:?}");
        let s = l.judge_text(&signed(&sk, &[("max_version", "9.0.0")]));
        assert!(matches!(s, LicenseState::Licensed(_)), "{s:?}");
        match l.judge_text(&signed(&sk, &[("product", "nexa-sql")])) {
            LicenseState::Invalid(Invalid::Product) => {}
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            l.judge_text("hello"),
            LicenseState::Invalid(Invalid::Format)
        ));
        // 루트 키가 비어 있으면(발급 전) 어떤 파일도 NoRootKey.
        let empty = Licensing::with_roots(vec![base.join("u")], &[]);
        assert!(matches!(
            empty.judge_text(&signed(&sk, &[])),
            LicenseState::Invalid(Invalid::NoRootKey)
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn refresh_notices_file_changes_and_user_dir_wins_over_machine_dir() {
        let base = tmp("refresh");
        let (sk, roots) = keypair();
        let user = base.join("u");
        let machine = base.join("m");
        let mut l = Licensing::with_roots(vec![user.clone(), machine.clone()], roots);
        assert!(!l.refresh(), "변화 없음");
        // 기기 공용 폴더에 다른 프로세스가 파일을 둔다.
        std::fs::create_dir_all(&machine).expect("mkdir");
        std::fs::write(
            machine.join(PRODUCT.license_file_name()),
            signed(&sk, &[("id", "NBL-M"), ("kind", "device")]),
        )
        .expect("write");
        assert!(l.refresh());
        assert_eq!(l.state().license().map(|x| x.id.as_str()), Some("NBL-M"));
        assert!(!l.refresh());
        // 사용자 폴더 설치가 앞선다.
        l.install_text(&signed(&sk, &[("id", "NBL-U")]))
            .expect("install");
        assert_eq!(l.state().license().map(|x| x.id.as_str()), Some("NBL-U"));
        assert_eq!(
            l.path(),
            Some(user.join(PRODUCT.license_file_name()).as_path())
        );
        // 제거 → 기기 파일로 복귀.
        assert!(l.remove().expect("remove"));
        assert_eq!(l.state().license().map(|x| x.id.as_str()), Some("NBL-M"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn request_code_and_machine_code() {
        let Some(code) = Licensing::request_code(&RequestMeta {
            name: "홍길동".into(),
            email: " a@b.c ".into(),
        }) else {
            eprintln!("이 환경은 기기 ID가 없다(컨테이너?) — 건너뜀");
            return;
        };
        assert!(code.starts_with("NEXAREQ1."));
        let r = request::decode(&code).expect("decode");
        assert_eq!(r.meta.get("n"), Some("홍길동"));
        assert_eq!(r.meta.get("e"), Some("a@b.c"));
        assert_eq!(r.meta.get("os"), Some(std::env::consts::OS));
        assert!(r
            .meta
            .get("app")
            .is_some_and(|a| a.starts_with("nexa-beep/")));
        assert_eq!(
            base32::encode(&r.machine),
            Licensing::machine_code().expect("mc")
        );
        // 이름 없는 요청 = 메타에 n 없음.
        let bare = Licensing::request_code(&RequestMeta::default()).expect("bare");
        assert_eq!(request::decode(&bare).expect("d").meta.get("n"), None);
    }

    /// CLI(`nexa-beep --license …`) — status → install(거부·통과) → status → remove 왕복 · 사용법 종료 2.
    #[test]
    fn cli_status_install_remove_roundtrip() {
        let base = tmp("cli");
        let (sk, roots) = keypair();
        let mut l = Licensing::with_roots(vec![base.join("u")], roots);
        let run = |l: &mut Licensing, args: &[&str]| {
            let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = cli::run_with(l, &args, &mut out, &mut err);
            (
                code,
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            )
        };
        let (c, out, _) = run(&mut l, &["status"]);
        assert_eq!(c, 0);
        assert!(out.contains("state          free"), "{out}");
        assert!(out.contains(&format!("build_date     {}", PRODUCT.build_date)));
        // 다른 제품 파일 = 거부(종료 1 · 쓰지 않음).
        let bad = base.join("sql.license");
        std::fs::write(&bad, signed(&sk, &[("product", "nexa-sql")])).unwrap();
        let (c, _, err) = run(&mut l, &["install", bad.to_str().unwrap()]);
        assert_eq!(c, 1);
        assert!(err.contains("invalid(product)"), "{err}");
        assert_eq!(*l.state(), LicenseState::Free);
        // 정상 파일 = 설치.
        let good = base.join("mail.license");
        std::fs::write(&good, signed(&sk, &[])).unwrap();
        let (c, out, _) = run(&mut l, &["install", good.to_str().unwrap()]);
        assert_eq!(c, 0, "{out}");
        assert!(out.contains("id             NBL-2026-000001"), "{out}");
        let (_, out, _) = run(&mut l, &["status"]);
        assert!(out.contains("state          licensed"), "{out}");
        let (c, out, _) = run(&mut l, &["remove"]);
        assert_eq!(c, 0);
        assert!(out.starts_with("removed · state free"), "{out}");
        let (_, out, _) = run(&mut l, &["remove"]);
        assert!(out.starts_with("nothing to remove"), "{out}");
        // 사용법.
        assert_eq!(run(&mut l, &[]).0, 2);
        assert_eq!(run(&mut l, &["install"]).0, 2);
        assert_eq!(run(&mut l, &["bogus"]).0, 2);
        let (c, out, _) = run(&mut l, &["path"]);
        assert_eq!(c, 0);
        assert!(
            out.trim_end()
                .ends_with(PRODUCT.license_file_name().as_str()),
            "{out}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 데이터 폴더 주입(D-33-7) — 사용자 폴더 = `<data>/license` · 기기 공용은 그 뒤.
    #[test]
    fn default_dirs_put_injected_data_dir_first() {
        let data = std::path::Path::new("/x/data");
        let dirs = default_dirs(data);
        assert_eq!(dirs[0], data.join("license"));
        assert_eq!(dirs.len(), 1 + usize::from(machine_dir().is_some()));
        assert_eq!(PRODUCT.id, "nexa-beep");
    }
}
