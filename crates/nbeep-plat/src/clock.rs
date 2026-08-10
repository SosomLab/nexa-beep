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

/// 지역 연·월·일·시·분·초(+ 지역 시각 여부) — 메시지 타임스탬프 표시용(08-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// 연(예: 2026).
    pub y: u32,
    /// 월 1~12.
    pub mo: u32,
    /// 일 1~31.
    pub d: u32,
    /// 0~23.
    pub h: u32,
    /// 0~59.
    pub m: u32,
    /// 0~59(윤초 60 가능).
    pub s: u32,
    /// `false`면 지역 시간대를 적용하지 못하고 UTC로 계산했다(표시할 때 밝혀야 한다).
    pub is_local: bool,
}

impl LocalTime {
    /// 같은 달력 날짜인가(연·월·일).
    #[must_use]
    pub fn same_day(self, other: Self) -> bool {
        (self.y, self.mo, self.d) == (other.y, other.mo, other.d)
    }
    /// 같은 분인가(연월일시분).
    #[must_use]
    pub fn same_minute(self, other: Self) -> bool {
        self.same_day(other) && (self.h, self.m) == (other.h, other.m)
    }
}

/// Unix 초 → 지역 시·분·초.
#[must_use]
pub fn local_hms(unix_secs: u64) -> Hms {
    let t = local_time(unix_secs);
    Hms {
        h: t.h,
        m: t.m,
        s: t.s,
        is_local: t.is_local,
    }
}

/// Unix 초 → 지역 연월일시분초.
#[must_use]
pub fn local_time(unix_secs: u64) -> LocalTime {
    imp(unix_secs)
}

#[cfg(unix)]
fn imp(unix_secs: u64) -> LocalTime {
    // localtime_r는 스레드 안전 변형이다(localtime은 정적 버퍼를 공유해 쓰면 안 된다).
    let t = unix_secs as libc::time_t;
    let mut tm: libc::tm = unsafe { core::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    if ok {
        LocalTime {
            y: (tm.tm_year + 1900).max(0) as u32,
            mo: (tm.tm_mon + 1).clamp(1, 12) as u32,
            d: tm.tm_mday.clamp(1, 31) as u32,
            h: tm.tm_hour.clamp(0, 23) as u32,
            m: tm.tm_min.clamp(0, 59) as u32,
            s: tm.tm_sec.clamp(0, 60) as u32,
            is_local: true,
        }
    } else {
        utc(unix_secs)
    }
}

#[cfg(windows)]
fn imp(unix_secs: u64) -> LocalTime {
    // Unix 초 → FILETIME(1601 기준 100ns) → UTC SYSTEMTIME → 지역 SYSTEMTIME.
    // 시간대·DST 지식은 OS의 것(외부 크레이트 없이 — DR-5). 실패 시 UTC 폴백을 밝힌다.
    #[repr(C)]
    struct FileTime {
        lo: u32,
        hi: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime16 {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn FileTimeToSystemTime(ft: *const FileTime, st: *mut SystemTime16) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            tz: *const core::ffi::c_void,
            utc: *const SystemTime16,
            local: *mut SystemTime16,
        ) -> i32;
    }
    const UNIX_EPOCH_FT: u64 = 116_444_736_000_000_000; // 1601→1970(100ns 단위)
    let Some(ticks) = unix_secs
        .checked_mul(10_000_000)
        .and_then(|t| t.checked_add(UNIX_EPOCH_FT))
    else {
        return utc(unix_secs);
    };
    let ft = FileTime {
        lo: (ticks & 0xFFFF_FFFF) as u32,
        hi: (ticks >> 32) as u32,
    };
    let mut st = SystemTime16::default();
    let mut lt = SystemTime16::default();
    // SAFETY: 두 호출 모두 값 구조체 입출력뿐(핸들·소유권 없음). 실패는 0 반환으로 판정.
    let ok = unsafe {
        FileTimeToSystemTime(&ft, &mut st) != 0
            && SystemTimeToTzSpecificLocalTime(core::ptr::null(), &st, &mut lt) != 0
    };
    if ok {
        LocalTime {
            y: u32::from(lt.year),
            mo: u32::from(lt.month).clamp(1, 12),
            d: u32::from(lt.day).clamp(1, 31),
            h: u32::from(lt.hour).min(23),
            m: u32::from(lt.minute).min(59),
            s: u32::from(lt.second).min(60),
            is_local: true,
        }
    } else {
        utc(unix_secs)
    }
}

#[cfg(not(any(unix, windows)))]
fn imp(unix_secs: u64) -> LocalTime {
    utc(unix_secs)
}

/// UTC 기준 연월일시분초(폴백) — 날짜는 civil-from-days(그레고리력) 계산.
fn utc(unix_secs: u64) -> LocalTime {
    let days = (unix_secs / 86_400) as i64;
    let day = unix_secs % 86_400;
    // Howard Hinnant civil_from_days — epoch(1970-01-01) 기준.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    LocalTime {
        y: y.max(0) as u32,
        mo: mo as u32,
        d: d as u32,
        h: (day / 3600) as u32,
        m: ((day % 3600) / 60) as u32,
        s: (day % 60) as u32,
        is_local: false,
    }
}

/// 초 → `00:00:00` 고정 폭 표기(하루를 넘겨도 시를 그대로 늘린다 — 사용자 확정 08-09).
#[must_use]
pub fn clock_hms(secs: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
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
        assert_eq!((t.y, t.mo, t.d), (1970, 1, 1));
        assert_eq!((t.h, t.m, t.s), (1, 2, 3));
        assert!(!t.is_local, "폴백은 UTC임을 밝힌다");
    }

    #[test]
    fn utc_civil_date_handles_leap_and_month_edges() {
        // 2026-08-10 00:00:00 UTC = 1786320000.
        let t = utc(1_786_320_000);
        assert_eq!((t.y, t.mo, t.d), (2026, 8, 10));
        // 2024-02-29(윤년) 12:00 UTC = 1709208000.
        let t = utc(1_709_208_000);
        assert_eq!((t.y, t.mo, t.d), (2024, 2, 29));
        // 2025-12-31 23:59:59 UTC = 1767225599.
        let t = utc(1_767_225_599);
        assert_eq!((t.y, t.mo, t.d, t.h, t.m, t.s), (2025, 12, 31, 23, 59, 59));
    }

    #[test]
    fn local_is_within_a_day_and_reports_locality() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let t = local_hms(now);
        assert!(t.h < 24 && t.m < 60 && t.s <= 60);
        #[cfg(any(unix, windows))]
        assert!(t.is_local, "unix·windows는 OS 지역 시각을 얻는다");
        let full = local_time(now);
        assert!((1970..3000).contains(&full.y) && (1..=12).contains(&full.mo));
    }

    #[test]
    fn same_minute_and_day_comparisons() {
        let a = utc(1_786_320_000); // 2026-08-10 00:00:00
        let b = utc(1_786_320_059); // 같은 분(00:00:59)
        let c = utc(1_786_320_060); // 다음 분(00:01:00)
        let d = utc(1_786_320_000 + 86_400); // 다음 날
        assert!(a.same_minute(b));
        assert!(!a.same_minute(c));
        assert!(a.same_day(c));
        assert!(!a.same_day(d));
    }

    #[test]
    fn clock_format_is_fixed_width() {
        assert_eq!(clock_hms(0), "00:00:00");
        assert_eq!(clock_hms(59), "00:00:59");
        assert_eq!(clock_hms(3600), "01:00:00");
        assert_eq!(clock_hms(3661), "01:01:01");
        assert_eq!(clock_hms(86_400), "24:00:00", "하루를 넘겨도 시를 늘린다");
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
