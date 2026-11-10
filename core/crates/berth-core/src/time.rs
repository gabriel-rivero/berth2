//! Time helpers with no dependencies: epoch millis and RFC3339 UTC.
//!
//! The core stays I/O-free, but timestamps are just arithmetic, and every
//! event needs one. The shell may replace them with a monotonic source.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `1970-01-01T00:00:00.000Z` style, UTC only.
pub fn rfc3339(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z")
}

/// Howard Hinnant's days-from-civil, inverted.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn known_instant() {
        assert_eq!(rfc3339(1_700_000_000_000), "2023-11-14T22:13:20.000Z");
    }

    #[test]
    fn leap_day() {
        assert_eq!(rfc3339(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
    }

    #[test]
    fn subsecond_and_end_of_year() {
        assert_eq!(rfc3339(1_735_689_599_999), "2024-12-31T23:59:59.999Z");
    }
}
