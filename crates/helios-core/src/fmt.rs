//! Byte and date formatting, shared by the CLI and the exporters.
//!
//! Sizes use decimal units (KB = 1000 bytes), matching what macOS Finder and
//! the Windows Explorer status bar report — Helios must never disagree with the
//! OS about how big a folder is.

/// Formats a byte count the way the Finder does.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["bytes", "KB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    // One decimal below 10, none above: "9.4 GB" but "412 GB".
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Formats a Unix timestamp as `YYYY-MM-DD HH:MM` in UTC.
///
/// Implemented directly rather than pulling in `chrono`/`time`: reports need
/// one date format, and a date library is a large dependency surface for an app
/// that markets itself on having almost none. The UI renders localized dates
/// with `Intl.DateTimeFormat`, which is where a user's timezone belongs anyway.
pub fn format_timestamp(unix_seconds: i64) -> String {
    if unix_seconds <= 0 {
        return "—".to_string();
    }
    let days = unix_seconds.div_euclid(86_400);
    let secs = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60
    )
}

pub fn format_date(unix_seconds: i64) -> String {
    if unix_seconds <= 0 {
        return "—".to_string();
    }
    let (year, month, day) = civil_from_days(unix_seconds.div_euclid(86_400));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's `civil_from_days`, shifting the epoch to 0000-03-01 so leap
/// years fall out of integer arithmetic with no lookup tables.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Formats a duration in milliseconds as a compact human string.
pub fn human_duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms} ms"),
        1_000..=59_999 => format!("{:.1} s", ms as f64 / 1000.0),
        _ => {
            let total = ms / 1000;
            format!("{} min {} s", total / 60, total % 60)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_at_every_scale() {
        assert_eq!(human_bytes(0), "0 bytes");
        assert_eq!(human_bytes(999), "999 bytes");
        assert_eq!(human_bytes(1_000), "1.0 KB");
        assert_eq!(human_bytes(9_400_000_000), "9.4 GB");
        assert_eq!(human_bytes(412_000_000_000), "412 GB");
        assert_eq!(human_bytes(2_000_000_000_000), "2.0 TB");
    }

    #[test]
    fn formats_dates_including_leap_years() {
        assert_eq!(format_timestamp(0), "—");
        assert_eq!(format_date(1_700_000_000), "2023-11-14");
        // 2024-02-29, a leap day.
        assert_eq!(format_date(1_709_164_800), "2024-02-29");
        assert_eq!(format_timestamp(1_709_164_800), "2024-02-29 00:00");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(human_duration(250), "250 ms");
        assert_eq!(human_duration(4_500), "4.5 s");
        assert_eq!(human_duration(125_000), "2 min 5 s");
    }
}
