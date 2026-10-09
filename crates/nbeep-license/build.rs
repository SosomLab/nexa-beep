//! build.rs — 빌드 날짜 `NBEEP_BUILD_DATE`(`YYYY-MM-DD`)를 박는다(docs/23 §1-4 · docs/92 §2-3 R2): 영구 라이선스의
//! `updates_until` 판정 기준은 **시스템 시계가 아니라 빌드일**이라 시계를 되돌려도 이득이 없다.
//! 우선순위: 환경 변수 `NBEEP_BUILD_DATE`(재현 빌드·시험) → `SOURCE_DATE_EPOCH`(배포 규약) → 지금. **외부 crate 0**.
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-env-changed=NBEEP_BUILD_DATE");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let date = std::env::var("NBEEP_BUILD_DATE")
        .ok()
        .filter(|s| is_date(s))
        .or_else(|| {
            std::env::var("SOURCE_DATE_EPOCH")
                .ok()
                .and_then(|s| s.trim().parse::<i64>().ok())
                .map(|secs| civil(secs.div_euclid(86_400)))
        })
        .unwrap_or_else(|| {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            civil(secs.div_euclid(86_400))
        });
    println!("cargo:rustc-env=NBEEP_BUILD_DATE={date}");
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// 1970-01-01부터의 일 수 → `YYYY-MM-DD`(Howard Hinnant `civil_from_days`).
fn civil(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
