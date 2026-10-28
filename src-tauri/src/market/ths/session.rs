//! When Shanghai and Shenzhen trade, so polling can rest while they don't.
//! China Standard Time is UTC+8 all year.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex;

use super::api;

/// Seconds east of UTC.
pub const OFFSET: i64 = 8 * 3600;
const DAY: i64 = 86_400;
/// When quotes move, in seconds into the day: from the opening call auction
/// at 9:15 to the morning close at 11:30, and from 13:00 to the close at
/// 15:00, each five minutes longer for the final prints.
const SESSIONS: [(i64, i64); 2] =
    [(9 * 3600 + 15 * 60, 11 * 3600 + 35 * 60), (13 * 3600, 15 * 3600 + 5 * 60)];
/// The longest rest between looks at the clock, so a machine that slept
/// through an opening catches up soon after it wakes.
const MAX_REST: i64 = 600;
/// A weekday the calendar calls a holiday is asked about again after this,
/// in case the calendar hadn't caught up with the day.
const RECHECK: Duration = Duration::from_secs(3600);
/// After the calendar couldn't be read.
const RETRY: Duration = Duration::from_secs(600);

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Days since 1970-01-01 in Shanghai, and seconds into that day.
pub fn local(epoch: i64) -> (i64, i64) {
    let t = epoch + OFFSET;
    (t.div_euclid(DAY), t.rem_euclid(DAY))
}

/// Days since Monday; 1970-01-01 was a Thursday.
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

fn is_weekday(day: i64) -> bool {
    weekday(day) < 5
}

/// The epoch second the week of `epoch` began: Monday, 00:00 in Shanghai.
pub fn week_start(epoch: i64) -> i64 {
    let (day, _) = local(epoch);
    (day - weekday(day)) * DAY - OFFSET
}

/// Seconds from `epoch` until quotes move again; 0 while they do. Later
/// days are taken to trade if they are weekdays.
pub fn until_open(epoch: i64, trading_today: bool) -> i64 {
    let (day, secs) = local(epoch);
    if trading_today {
        for (open, close) in SESSIONS {
            if secs < open {
                return open - secs;
            }
            if secs < close {
                return 0;
            }
        }
    }
    let mut next = day + 1;
    while !is_weekday(next) {
        next += 1;
    }
    (next - day) * DAY + SESSIONS[0].0 - secs
}

/// How long to rest with `until_open` seconds to go.
pub fn rest(until_open: i64) -> Duration {
    Duration::from_secs(until_open.clamp(1, MAX_REST) as u64)
}

struct Today {
    day: i64,
    trading: bool,
    /// When to ask the calendar again; never for a trading day.
    recheck: Option<Instant>,
}

static TODAY: Mutex<Option<Today>> = Mutex::const_new(None);

/// Whether Shanghai trades today. Weekends never; a weekday as the exchange
/// calendar says, asked once a day, and taken to trade while the calendar
/// can't be read.
pub async fn trading_today() -> bool {
    let (day, _) = local(now());
    if !is_weekday(day) {
        return false;
    }
    let mut today = TODAY.lock().await;
    if let Some(known) = &*today
        && known.day == day
        && known.recheck.is_none_or(|at| Instant::now() < at)
    {
        return known.trading;
    }
    let (trading, recheck) = match api::trading_days().await {
        Ok(days) if !days.is_empty() => {
            let trading = days.contains(&((day * DAY - OFFSET) * 1000));
            (trading, (!trading).then(|| Instant::now() + RECHECK))
        }
        Ok(_) => (true, Some(Instant::now() + RETRY)),
        Err(e) => {
            log::debug!("tonghuashun calendar: {e}");
            (true, Some(Instant::now() + RETRY))
        }
    };
    *today = Some(Today { day, trading, recheck });
    trading
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::days_from_civil;

    /// Epoch seconds of a wall-clock time in Shanghai.
    fn shanghai(year: i64, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        days_from_civil(year, month, day) * DAY + hour * 3600 + minute * 60 - OFFSET
    }

    // contract: SSE and SZSE trading hours: call auction from 9:15, continuous
    // trading 9:30–11:30 and 13:00–15:00, Monday to Friday.
    #[test]
    fn quotes_move_during_the_sessions_of_a_trading_day() {
        // Wednesday 2026-10-28.
        assert_eq!(until_open(shanghai(2026, 10, 28, 9, 0), true), 15 * 60);
        assert_eq!(until_open(shanghai(2026, 10, 28, 9, 15), true), 0);
        assert_eq!(until_open(shanghai(2026, 10, 28, 11, 34), true), 0);
        assert_eq!(until_open(shanghai(2026, 10, 28, 12, 0), true), 3600);
        assert_eq!(until_open(shanghai(2026, 10, 28, 15, 4), true), 0);
        // After the close, and all day on a holiday: tomorrow morning.
        let thursday = shanghai(2026, 10, 29, 9, 15);
        assert_eq!(
            until_open(shanghai(2026, 10, 28, 15, 5), true),
            thursday - shanghai(2026, 10, 28, 15, 5)
        );
        assert_eq!(
            until_open(shanghai(2026, 10, 28, 10, 0), false),
            thursday - shanghai(2026, 10, 28, 10, 0)
        );
        // Friday evening waits for Monday.
        let monday = shanghai(2026, 11, 2, 9, 15);
        assert_eq!(
            until_open(shanghai(2026, 10, 30, 20, 0), true),
            monday - shanghai(2026, 10, 30, 20, 0)
        );
    }

    #[test]
    fn weeks_open_on_monday_in_shanghai() {
        let monday = shanghai(2026, 10, 26, 0, 0);
        assert_eq!(week_start(shanghai(2026, 10, 28, 14, 0)), monday);
        assert_eq!(week_start(monday), monday);
        // A minute before Monday in Shanghai (16:59 UTC) is the week before.
        assert_eq!(week_start(monday - 60), shanghai(2026, 10, 19, 0, 0));
    }
}
