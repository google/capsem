//! UTC time parsing and formatting shared by host crates.

use std::time::SystemTime;

use jiff::Timestamp;

/// Parse an RFC 3339 timestamp into a system clock instant.
pub fn parse_rfc3339(value: &str) -> Result<SystemTime, jiff::Error> {
    value.parse::<Timestamp>().map(Into::into)
}

/// Format a system clock instant with smart nanosecond precision.
///
/// Whole seconds omit the fractional component. Any non-zero fraction is
/// rendered with nine digits to preserve the historical Capsem wire format.
pub fn format_rfc3339(time: SystemTime) -> String {
    let timestamp = timestamp(time);
    if timestamp.subsec_nanosecond() == 0 {
        timestamp.to_string()
    } else {
        format!("{timestamp:.9}")
    }
}

/// Format a system clock instant at whole-second precision.
pub fn format_rfc3339_seconds(time: SystemTime) -> String {
    format!("{:.0}", timestamp(time))
}

/// Format a system clock instant at millisecond precision.
pub fn format_rfc3339_millis(time: SystemTime) -> String {
    format!("{:.3}", timestamp(time))
}

/// Format a system clock instant at microsecond precision.
pub fn format_rfc3339_micros(time: SystemTime) -> String {
    format!("{:.6}", timestamp(time))
}

/// Format a system clock instant at nanosecond precision.
pub fn format_rfc3339_nanos(time: SystemTime) -> String {
    format!("{:.9}", timestamp(time))
}

fn timestamp(time: SystemTime) -> Timestamp {
    Timestamp::try_from(time).expect("system time is outside the supported RFC 3339 range")
}

/// Break epoch seconds into UTC calendar and clock components.
pub fn epoch_to_parts(secs: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let hours = time_of_day / 3_600;
    let minutes = (time_of_day % 3_600) / 60;
    let seconds = time_of_day % 60;

    let mut year = 1970_i64;
    let mut remaining_days = days as i64;
    loop {
        let year_days = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        year += 1;
    }

    let month_days = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0_u32;
    for days_in_month in month_days {
        if remaining_days < days_in_month {
            break;
        }
        remaining_days -= days_in_month;
        month += 1;
    }

    (year, month + 1, remaining_days as u32 + 1, hours, minutes, seconds)
}

/// Convert epoch seconds to an ISO 8601 UTC timestamp.
pub fn epoch_to_iso(secs: u64) -> String {
    let (year, month, day, hours, minutes, seconds) = epoch_to_parts(secs);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

const fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests;
