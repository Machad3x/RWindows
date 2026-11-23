//! Calendar grid for the notification center. Pure date math, no crate.
//! Weekdays follow Windows: Sunday = 0.

use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SABBREVDAYNAME1, LOCALE_SMONTHNAME1};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::core::PCWSTR;

/// A civil date in the Gregorian calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    /// Gregorian year.
    pub year: i32,
    /// 1..=12
    pub month: u32,
    /// 1..=31
    pub day: u32,
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(date: Date) -> i64 {
    let year = date.year - i32::from(date.month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = date.month as i32 + if date.month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + date.day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era) * 146_097 + i64::from(doe) - 719_468
}

/// Civil date for a number of days since 1970-01-01. Kept for the round-trip
/// test of `days_from_civil`.
#[cfg(test)]
fn civil_from_days(days: i64) -> Date {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (mp + if mp < 10 { 3 } else { -9 }) as u32;
    Date { year: (year + i64::from(month <= 2)) as i32, month, day }
}

/// Today in the local time zone. `UNIX_EPOCH` would give the UTC day, which is
/// a different date for several hours each day.
pub fn today() -> Date {
    let time = unsafe { GetLocalTime() };
    Date { year: i32::from(time.wYear), month: u32::from(time.wMonth), day: u32::from(time.wDay) }
}

/// Weekday of a date, Sunday = 0.
pub fn weekday(date: Date) -> u32 {
    (days_from_civil(date) + 4).rem_euclid(7) as u32
}

/// Number of days in a month, including leap years.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Six-week calendar grid for a month. `None` is a day outside the month.
pub fn grid(year: i32, month: u32) -> Vec<Option<u32>> {
    let lead = weekday(Date { year, month, day: 1 }) as usize;
    let days = days_in_month(year, month) as usize;
    let mut cells = Vec::with_capacity(42);
    cells.extend(std::iter::repeat_n::<Option<u32>>(None, lead));
    cells.extend((1..=days as u32).map(Some));
    cells.resize(42, None);
    cells
}

/// Full month name in the user's locale (1..=12).
pub fn month_name(month: u32) -> String {
    if !(1..=12).contains(&month) {
        return String::new();
    }
    let mut buffer = [0u16; 64];
    unsafe {
        GetLocaleInfoEx(PCWSTR::null(), LOCALE_SMONTHNAME1 + month - 1, Some(&mut buffer));
    }
    String::from_utf16_lossy(&buffer).trim_end_matches('\0').to_owned()
}

/// Abbreviated weekday in the user's locale, Sunday = 0.
pub fn weekday_name(weekday: u32) -> String {
    let constant = if weekday == 0 { LOCALE_SABBREVDAYNAME1 + 6 } else { LOCALE_SABBREVDAYNAME1 + weekday - 1 };
    let mut buffer = [0u16; 16];
    unsafe {
        GetLocaleInfoEx(PCWSTR::null(), constant, Some(&mut buffer));
    }
    String::from_utf16_lossy(&buffer).trim_end_matches('\0').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_should_match_known_dates() {
        assert_eq!(weekday(Date { year: 2026, month: 11, day: 22 }), 0); // Sunday
        assert_eq!(weekday(Date { year: 2026, month: 11, day: 1 }), 0); // Sunday
        assert_eq!(weekday(Date { year: 2000, month: 1, day: 1 }), 6); // Saturday
    }

    #[test]
    fn civil_round_trip_should_cover_leap_days() {
        for date in [
            Date { year: 1970, month: 1, day: 1 },
            Date { year: 2000, month: 2, day: 29 },
            Date { year: 2024, month: 12, day: 31 },
            Date { year: 1900, month: 3, day: 1 },
        ] {
            assert_eq!(civil_from_days(days_from_civil(date)), date);
        }
    }

    #[test]
    fn leap_years_should_set_february_to_29_days() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2025, 2), 28);
        assert_eq!(days_in_month(2026, 11), 30);
    }

    #[test]
    fn grid_should_pad_to_six_weeks_of_seven_days() {
        let cells = grid(2026, 11);
        assert_eq!(cells.len(), 42);
        // 2026-11-01 is a Sunday, so the first row starts with day 1.
        assert_eq!(cells[0], Some(1));
        assert_eq!(cells.iter().flatten().count(), 30);
        assert_eq!(cells[29], Some(30));
        assert_eq!(cells[30], None);
    }
}
