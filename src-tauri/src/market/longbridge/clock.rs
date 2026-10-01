//! Local time on the exchanges Longbridge covers: US Eastern with daylight
//! saving, UTC+8 in Asia. Small enough not to need a time-zone database.

use crate::calendar::{Civil, civil, days_from_civil};

/// The markets, by symbol suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Market {
    Us,
    Hk,
    Cn,
}

impl Market {
    /// Seconds to add to UTC for local time at `epoch` (seconds).
    pub fn utc_offset(self, epoch: i64) -> i64 {
        const HOUR: i64 = 3600;
        match self {
            Self::Us if us_daylight_time(epoch) => -4 * HOUR,
            Self::Us => -5 * HOUR,
            Self::Hk | Self::Cn => 8 * HOUR,
        }
    }

    pub fn local(self, epoch: i64) -> Civil {
        civil(epoch + self.utc_offset(epoch))
    }
}

/// Since 2007: from the second Sunday of March, 02:00 local standard time,
/// to the first Sunday of November, 02:00 local daylight time.
fn us_daylight_time(epoch: i64) -> bool {
    let year = civil(epoch).year;
    let start = sunday_on_or_after(year, 3, 8) * 86_400 + 7 * 3600;
    let end = sunday_on_or_after(year, 11, 1) * 86_400 + 6 * 3600;
    (start..end).contains(&epoch)
}

/// Days since the epoch of the first Sunday on or after the given date.
fn sunday_on_or_after(year: i64, month: u32, day: u32) -> i64 {
    let days = days_from_civil(year, month, day);
    // 1970-01-01 was a Thursday: weekday 4 counting from Sunday = 0.
    let weekday = (days + 4).rem_euclid(7);
    days + (7 - weekday) % 7
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epoch(year: i64, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60
    }

    #[test]
    fn new_york_follows_daylight_saving() {
        // 2026: daylight time from March 8 to November 1.
        assert_eq!(Market::Us.utc_offset(epoch(2026, 3, 8, 6, 59)), -5 * 3600);
        assert_eq!(Market::Us.utc_offset(epoch(2026, 3, 8, 7, 0)), -4 * 3600);
        assert_eq!(Market::Us.utc_offset(epoch(2026, 11, 1, 5, 59)), -4 * 3600);
        assert_eq!(Market::Us.utc_offset(epoch(2026, 11, 1, 6, 0)), -5 * 3600);
        let open = Market::Us.local(epoch(2026, 9, 25, 13, 30));
        assert_eq!((open.day, open.hour, open.minute), (25, 9, 30));
        let shanghai = Market::Cn.local(epoch(2026, 9, 24, 7, 0));
        assert_eq!((shanghai.day, shanghai.hour), (24, 15));
    }
}
