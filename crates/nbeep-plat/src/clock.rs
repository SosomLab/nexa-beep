//! **지역 시각 표기** — Unix 초 → 사람이 읽는 `HH:MM` / `HH:MM:SS`.
//!
//! 시간대 변환은 OS가 안다(`localtime_r`) — 외부 크레이트 없이 그 지식을 빌린다(DR-5).
//! 지원하지 않는 플랫폼에서는 **UTC로 계산하고 그 사실을 숨기지 않는다**
//! ([`local_hms`]가 `is_local = false`를 함께 준다).

/// 시·분·초 + **지역 시각인지 여부**.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hms {
    /// 0~23.
    pub h: u32,
    /// 0~59.
    pub m: u32,
    /// 0~59(윤초 60 가능).
    pub s: u32,
    /// `false`면 지역 시간대를 적용하지 못하고 UTC로 계산했다(표시할 때 밝혀야 한다).
    pub is_local: bool,
}

impl Hms {
    /// `HH:MM` 표기.
    #[must_use]
    pub fn hm(self) -> String {
        format!("{:02}:{:02}", self.h, self.m)
    }
    /// `HH:MM:SS` 표기.
    #[must_use]
    pub fn hms(self) -> String {
        format!("{:02}:{:02}:{:02}", self.h, self.m, self.s)
    }
}

/// Unix 초 → 지역 시·분·초.
#[must_use]
pub fn local_hms(unix_secs: u64) -> Hms {
    imp(unix_secs)
}

#[cfg(unix)]
fn imp(unix_secs: u64) -> Hms {
    // localtime_r는 스레드 안전 변형이다(localtime은 정적 버퍼를 공유해 쓰면 안 된다).
    let t = unix_secs as libc::time_t;
    let mut tm: libc::tm = unsafe { core::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    if ok {
        Hms {
            h: tm.tm_hour.clamp(0, 23) as u32,
            m: tm.tm_min.clamp(0, 59) as u32,
            s: tm.tm_sec.clamp(0, 60) as u32,
            is_local: true,
        }
    } else {
        utc(unix_secs)
    }
}

#[cfg(not(unix))]
fn imp(unix_secs: u64) -> Hms {
    // Windows 지역 시각(GetLocalTime)은 별도 슬라이스 — 그때까지 UTC로 계산하고 밝힌다.
    utc(unix_secs)
}

/// UTC 기준 시·분·초(폴백).
fn utc(unix_secs: u64) -> Hms {
    let day = unix_secs % 86_400;
    Hms {
        h: (day / 3600) as u32,
        m: ((day % 3600) / 60) as u32,
        s: (day % 60) as u32,
        is_local: false,
    }
}

/// 초 → `1시간 5분` 같은 짧은 한국어 기간 표기(0초 = `0초`).
#[must_use]
pub fn duration_ko(mut secs: u64) -> String {
    if secs == 0 {
        return "0초".into();
    }
    let h = secs / 3600;
    secs %= 3600;
    let m = secs / 60;
    let s = secs % 60;
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}시간 "));
    }
    if m > 0 {
        out.push_str(&format!("{m}분 "));
    }
    // 시·분이 있으면 초는 생략(길어지기만 한다).
    if h == 0 && m == 0 {
        out.push_str(&format!("{s}초"));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_fallback_splits_day_correctly() {
        // 1970-01-01 01:02:03 UTC.
        let t = utc(3723);
        assert_eq!((t.h, t.m, t.s), (1, 2, 3));
        assert!(!t.is_local, "폴백은 UTC임을 밝힌다");
        assert_eq!(t.hm(), "01:02");
        assert_eq!(t.hms(), "01:02:03");
    }

    #[test]
    fn local_is_within_a_day_and_reports_locality() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let t = local_hms(now);
        assert!(t.h < 24 && t.m < 60 && t.s <= 60);
        #[cfg(unix)]
        assert!(t.is_local, "unix는 localtime_r로 지역 시각을 얻는다");
    }

    #[test]
    fn duration_reads_naturally() {
        assert_eq!(duration_ko(0), "0초");
        assert_eq!(duration_ko(45), "45초");
        assert_eq!(duration_ko(60), "1분");
        assert_eq!(duration_ko(3600), "1시간");
        assert_eq!(duration_ko(3900), "1시간 5분");
        assert_eq!(duration_ko(21_600), "6시간");
    }
}
