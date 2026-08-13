//! 아바타 선택 — **무엇을 내 얼굴로 쓸 것인가**(사용자 확정 08-13).
//!
//! 그전에는 `profile.image_path` 문자열 하나뿐이라 **"없음"과 "이니셜"을 구분할 수 없었다**
//! (빈 값이 둘 다였다). 선택지가 넷으로 늘면서 표현을 하나로 모은다:
//!
//! | 저장값 | 뜻 |
//! | --- | --- |
//! | `initials` | 이니셜 아바타(이름 앞 2글자 · 기존 스타일) |
//! | `none` | 없음 — 빈 원(아무것도 안 보여주고 싶을 때) |
//! | `b:<키>` | 내장 12간지(`b:tiger` — 키는 [`ZODIAC`]) |
//! | `c:<파일명>` | 사용자가 등록한 이미지(`data/profiles/custom/<파일명>`) |
//!
//! ## 왜 문자열 하나인가
//!
//! 설정 영속(ADR-0011)은 **키=값 문자열**이고 미지 키를 보존한다. 선택을 여러 키로 쪼개면
//! (`avatar.kind` + `avatar.value`) 둘이 어긋난 상태가 저장될 수 있다 — 한 값이면 그
//! 상태 자체가 없다. 파싱은 여기 한 곳뿐이라 형식이 갈릴 자리도 없다.
//!
//! ## 미지 값은 조용히 이니셜
//!
//! 구버전이 모르는 키(`b:phoenix`)나 지워진 사용자 이미지를 만나면 **이니셜로 폴백**한다
//! (발견 패킷·mux의 "미지 값 무시"와 같은 전방 호환 규약). 얼굴이 안 보이는 것보다
//! 조용히 대체되는 쪽이 낫고, 설정값은 건드리지 않아 자산이 돌아오면 되살아난다.

/// 12간지 키 — **순서와 철자는 불변**(설정에 저장되는 값이라 바뀌면 기존 선택이 깨진다).
pub const ZODIAC: [&str; 12] = [
    "rat", "ox", "tiger", "rabbit", "dragon", "snake", "horse", "goat", "monkey", "rooster", "dog",
    "pig",
];

/// 내 아바타로 무엇을 쓸지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvatarChoice {
    /// 이니셜 아바타(기본 폴백이기도 하다).
    Initials,
    /// 없음 — 빈 원.
    None,
    /// 내장 12간지 중 하나(키는 [`ZODIAC`]).
    Builtin(String),
    /// 사용자가 등록한 이미지(`data/profiles/custom/` 안의 파일명).
    Custom(String),
}

impl AvatarChoice {
    /// 저장 문자열 → 선택. **미지 형식은 [`Initials`](Self::Initials)**(전방 호환).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        match s {
            "" | "initials" => Self::Initials,
            "none" => Self::None,
            _ => {
                if let Some(k) = s.strip_prefix("b:") {
                    if ZODIAC.contains(&k) {
                        return Self::Builtin(k.to_string());
                    }
                    return Self::Initials; // 모르는 내장 키 = 신버전 값 → 폴백
                }
                if let Some(f) = s.strip_prefix("c:") {
                    // 경로 탈출 금지 — 파일명만 받는다(fail-closed).
                    if is_safe_name(f) {
                        return Self::Custom(f.to_string());
                    }
                }
                Self::Initials
            }
        }
    }

    /// 선택 → 저장 문자열([`parse`](Self::parse)의 역).
    #[must_use]
    pub fn to_setting(&self) -> String {
        match self {
            Self::Initials => "initials".to_string(),
            Self::None => "none".to_string(),
            Self::Builtin(k) => format!("b:{k}"),
            Self::Custom(f) => format!("c:{f}"),
        }
    }

    /// 상대에게 알릴 **내장 키**(있을 때만). 사용자 이미지는 바이트로 따로 보내고,
    /// 내장은 **키만 보낸다** — 상대도 같은 자산을 갖고 있으니 그림을 실어 나를 이유가 없다.
    #[must_use]
    pub fn builtin_key(&self) -> Option<&str> {
        match self {
            Self::Builtin(k) => Some(k.as_str()),
            _ => None,
        }
    }
}

/// 사용자 이미지 파일명 안전성 — **파일명만**(경로 구분자·상위 참조·숨김 금지).
/// 설정 파일은 사람이 편집할 수 있으니 여기로 경로 탈출이 들어올 수 있다(fail-closed).
#[must_use]
pub fn is_safe_name(f: &str) -> bool {
    !f.is_empty()
        && f.len() <= 128
        && !f.starts_with('.')
        && f.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !f.contains("..")
}

/// **기본값 — 시드에서 12간지 하나를 안정적으로 고른다.**
///
/// 사용자 요청("랜덤하게 적용")을 실행마다 다른 난수로 하면 **상대 화면에서 내 얼굴이
/// 계속 바뀐다**(신원은 그대로인데 얼굴만 요동친다 — 사칭처럼 보인다). 그래서 난수가
/// 아니라 **내 키 지문을 시드로 한 안정 선택**이다: 사람마다 다르고, 나는 늘 같다.
#[must_use]
pub fn default_for_seed(seed: &[u8]) -> AvatarChoice {
    if seed.is_empty() {
        return AvatarChoice::Builtin(ZODIAC[0].to_string());
    }
    let h = seed.iter().fold(0u64, |a, b| {
        a.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from(*b))
    });
    let idx = (h % ZODIAC.len() as u64) as usize;
    AvatarChoice::Builtin(ZODIAC[idx].to_string())
}

/// 아바타 보더 색(08-14 사용자 요청) — `"#RRGGBB"`만 유효. 무효·미지는 `None`
/// (호출자가 시드 기본값으로 폴백). 설정 파일은 사람이 고칠 수 있다 — 관용 파싱.
#[must_use]
pub fn parse_border(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    #[allow(clippy::cast_possible_truncation)]
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// (r, g, b) → 저장 문자열([`parse_border`]의 역).
#[must_use]
pub fn border_to_setting(rgb: (u8, u8, u8)) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2)
}

/// **기본 보더 색 — r/g/b 각각을 시드에서 유도**(사용자 확정 08-14 "각각 랜덤").
/// [`default_for_seed`]와 같은 이유로 실행 난수가 아니라 **키 지문 시드 안정**이다
/// (실행마다 바뀌면 상대 화면에서 테두리가 요동친다). 채널별 다른 해시 회전을 써서
/// 서로 독립적으로 퍼진다.
#[must_use]
pub fn default_border_for_seed(seed: &[u8]) -> (u8, u8, u8) {
    let h = |salt: u64| -> u8 {
        #[allow(clippy::cast_possible_truncation)]
        let v = seed.iter().fold(salt, |a, b| {
            a.wrapping_mul(0x0100_0000_01b3).wrapping_add(u64::from(*b))
        }) as u8;
        v
    };
    (
        h(0xcbf2_9ce4_8422_2325),
        h(0x9e37_79b9_7f4a_7c15),
        h(0x2545_f491_4f6c_dd1d),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 저장 왕복이 손실 없다 — 설정에 쓰고 읽는 계약.
    #[test]
    fn setting_roundtrip() {
        for c in [
            AvatarChoice::Initials,
            AvatarChoice::None,
            AvatarChoice::Builtin("tiger".into()),
            AvatarChoice::Custom("me_01.img".into()),
        ] {
            assert_eq!(AvatarChoice::parse(&c.to_setting()), c);
        }
    }

    /// ★ 빈 값과 "없음"이 구분된다 — 이 구분이 없어서 이 모델을 만들었다.
    #[test]
    fn empty_is_initials_not_none() {
        assert_eq!(AvatarChoice::parse(""), AvatarChoice::Initials);
        assert_eq!(AvatarChoice::parse("none"), AvatarChoice::None);
        assert_ne!(AvatarChoice::parse(""), AvatarChoice::parse("none"));
    }

    /// 미지 값은 조용히 이니셜(전방 호환 — 신버전이 쓴 키를 구버전이 만나도 안 깨진다).
    #[test]
    fn unknown_values_fall_back_to_initials() {
        for s in ["b:phoenix", "b:", "zzz", "c:", "x:tiger"] {
            assert_eq!(
                AvatarChoice::parse(s),
                AvatarChoice::Initials,
                "미지 값 {s}"
            );
        }
    }

    /// ⚠️ 사용자 이미지 이름으로 **경로 탈출이 들어오지 않는다**(설정은 사람이 고칠 수 있다).
    #[test]
    fn custom_name_rejects_path_escape() {
        for bad in [
            "c:../../etc/passwd",
            "c:/abs/path",
            "c:sub/dir.img",
            "c:..",
            "c:.hidden",
            "c:a\\b",
        ] {
            assert_eq!(
                AvatarChoice::parse(bad),
                AvatarChoice::Initials,
                "탈출 시도 {bad}"
            );
        }
        assert!(matches!(
            AvatarChoice::parse("c:ok_name-1.img"),
            AvatarChoice::Custom(_)
        ));
    }

    /// 기본값은 **사람마다 다르고 나는 늘 같다**(요청의 "랜덤"을 안정 선택으로 해석).
    #[test]
    fn default_is_stable_per_seed_and_spread() {
        assert_eq!(default_for_seed(b"abc"), default_for_seed(b"abc"));
        let picks: std::collections::HashSet<_> = (0u8..60)
            .map(|i| default_for_seed(&[i, i ^ 0x5a, i.wrapping_mul(7)]).to_setting())
            .collect();
        assert!(
            picks.len() >= 6,
            "12간지에 고루 퍼져야 한다: {}",
            picks.len()
        );
        // 빈 시드도 패닉 없이 유효한 선택.
        assert!(default_for_seed(&[]).builtin_key().is_some());
    }

    /// 보더 색 — 왕복·관용 파싱·시드 안정(08-14).
    #[test]
    fn border_roundtrip_and_lenient_parse() {
        for rgb in [(0u8, 0u8, 0u8), (255, 128, 7), (0x3D, 0x8B, 0xFF)] {
            assert_eq!(parse_border(&border_to_setting(rgb)), Some(rgb));
        }
        for bad in ["", "#12345", "#GGGGGG", "3D8BFF", "#1234567", "red"] {
            assert_eq!(parse_border(bad), None, "무효 값 {bad}");
        }
        // 시드 안정 + 시드별 분산(완전 일치 아님만 확인 — 채널 독립 유도).
        assert_eq!(
            default_border_for_seed(b"abc"),
            default_border_for_seed(b"abc")
        );
        assert_ne!(
            default_border_for_seed(b"abc"),
            default_border_for_seed(b"xyz")
        );
    }

    /// 내장만 키를 내놓는다(사용자 이미지는 바이트로 가야 한다).
    #[test]
    fn only_builtin_exposes_key() {
        assert_eq!(AvatarChoice::Builtin("ox".into()).builtin_key(), Some("ox"));
        assert!(AvatarChoice::Custom("a.img".into()).builtin_key().is_none());
        assert!(AvatarChoice::Initials.builtin_key().is_none());
        assert!(AvatarChoice::None.builtin_key().is_none());
    }
}
