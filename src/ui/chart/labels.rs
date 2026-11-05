//! Time labels on the chart: the axis, the start of a candle, and the stretch
//! a hover card names.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::format::{LocalTime, local_time_parts};

const MINUTE: f64 = 60.0;
const DAY: f64 = 86_400.0;
/// A day as an interval, which only daily candles reach.
const DAY_SECS: u32 = 86_400;

/// A time-axis label. Called for every label on every frame, so it formats
/// numbers by hand rather than through a locale.
///
/// Midnight, and every label of a multi-day view, reads as a date. Labels
/// land on whole minutes beyond about five minutes of view.
pub fn axis_label(t: f64, span: f64) -> String {
    axis_label_at(t, span, local_time_parts)
}

fn axis_label_at(t: f64, span: f64, local: fn(i64) -> LocalTime) -> String {
    let time = local(whole_secs(t));
    if span > 2.0 * DAY || (time.hour == 0 && time.minute == 0 && time.second == 0) {
        format!("{}/{}", time.month, time.day)
    } else if span <= 5.0 * MINUTE {
        format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second)
    } else {
        format!("{:02}:{:02}", time.hour, time.minute)
    }
}

/// The start of a candle. Daily candles carry the date of the day they open,
/// where they open; a shorter candle carries its local time, with the date
/// when that is not today.
pub fn candle_label(t: f64, secs: u32, day_offset: i64) -> String {
    candle_label_at(t, secs, day_offset, now(), local_time_parts)
}

fn candle_label_at(
    t: f64,
    secs: u32,
    day_offset: i64,
    now: f64,
    local: fn(i64) -> LocalTime,
) -> String {
    if secs >= DAY_SECS {
        let (year, month, day) = utc_date((t + day_offset as f64).floor() as i64);
        return format!("{year}/{month}/{day}");
    }
    let time = local(whole_secs(t));
    let clock = if secs < 60 {
        format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second)
    } else {
        format!("{:02}:{:02}", time.hour, time.minute)
    };
    let today = local(whole_secs(now));
    if (time.year, time.month, time.day) == (today.year, today.month, today.day) {
        clock
    } else {
        format!("{}/{} {clock}", time.month, time.day)
    }
}

/// Whole seconds, as the chart's timestamps reach the formatters.
fn whole_secs(t: f64) -> i64 {
    t.floor() as i64
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The proleptic Gregorian date of `secs` since the epoch, in UTC, as
/// `(year, month, day)`.
fn utc_date(secs: i64) -> (i32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The local clock the expected strings below were written against:
    /// UTC, with the timestamps they use.
    fn utc(t: i64) -> LocalTime {
        match t {
            1_704_067_200 => {
                LocalTime { year: 2024, month: 1, day: 1, hour: 0, minute: 0, second: 0 }
            }
            1_704_101_415 => {
                LocalTime { year: 2024, month: 1, day: 1, hour: 9, minute: 30, second: 15 }
            }
            1_704_110_400 => {
                LocalTime { year: 2024, month: 1, day: 1, hour: 12, minute: 0, second: 0 }
            }
            1_704_114_000 => {
                LocalTime { year: 2024, month: 1, day: 1, hour: 13, minute: 0, second: 0 }
            }
            1_704_027_600 => {
                LocalTime { year: 2023, month: 12, day: 31, hour: 13, minute: 0, second: 0 }
            }
            _ => panic!("no expected local time for {t}"),
        }
    }

    #[test]
    fn axis_labels_follow_the_span() {
        assert_eq!(axis_label_at(1_704_067_200.0, 3600.0, utc), "1/1");
        assert_eq!(axis_label_at(1_704_101_415.0, 300.0, utc), "09:30:15");
        assert_eq!(axis_label_at(1_704_101_415.0, 301.0, utc), "09:30");
        assert_eq!(axis_label_at(1_704_110_400.0, 3.0 * DAY, utc), "1/1");
    }

    // Daily candles open where the provider's day starts, so their date is
    // read in UTC after shifting by the offset.
    #[test]
    fn daily_candles_use_the_provider_day() {
        assert_eq!(candle_label_at(1_704_067_200.0, 86_400, 0, 0.0, utc), "2024/1/1");
        assert_eq!(candle_label_at(1_704_067_200.0, 86_400, -18_000, 0.0, utc), "2023/12/31");
        assert_eq!(candle_label_at(1_704_067_200.0, 86_400, 28_800, 0.0, utc), "2024/1/1");
        assert_eq!(candle_label_at(1_704_110_400.0, 86_400, 0, 0.0, utc), "2024/1/1");
        assert_eq!(candle_label_at(1_709_164_800.0, 86_400, 0, 0.0, utc), "2024/2/29");
    }

    #[test]
    fn intraday_candles_show_the_date_only_when_not_today() {
        let now = 1_704_114_000.0;
        assert_eq!(candle_label_at(1_704_101_415.0, 60, 0, now, utc), "09:30");
        assert_eq!(candle_label_at(1_704_101_415.0, 30, 0, now, utc), "09:30:15");
        assert_eq!(candle_label_at(1_704_027_600.0, 60, 0, now, utc), "12/31 13:00");
    }

    #[test]
    fn utc_dates_cover_month_and_leap_boundaries() {
        assert_eq!(utc_date(0), (1970, 1, 1));
        assert_eq!(utc_date(-1), (1969, 12, 31));
        assert_eq!(utc_date(1_704_067_200), (2024, 1, 1));
        assert_eq!(utc_date(1_709_164_800), (2024, 2, 29));
        assert_eq!(utc_date(1_709_251_200), (2024, 3, 1));
    }

    // The production wrappers read the machine's clock and time zone; they
    // only have to produce the same shape as the pinned cases above.
    #[test]
    fn wrappers_use_the_system_clock() {
        let label = axis_label(0.0, 60.0);
        assert!(label.contains(':') || label.contains('/'));
        assert!(!candle_label(0.0, 60, 0).is_empty());
    }
}
