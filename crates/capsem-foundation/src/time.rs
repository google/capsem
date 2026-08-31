//! Dependency-free UTC time formatting shared by host crates.

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

    (
        year,
        month + 1,
        remaining_days as u32 + 1,
        hours,
        minutes,
        seconds,
    )
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
