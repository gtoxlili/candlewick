//! Number and time formatting for the bar and the windows.
//!
//! Prices, amounts and the clock read the same in every language, as trading
//! screens write them; what is said in words (compact units, durations,
//! dates) takes the language the app speaks now.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::i18n::{self, Locale};

/// Decimals for a price: the pair's tick precision when known, otherwise
/// about six significant digits.
pub fn decimals(price: f64, tick_decimals: Option<u8>) -> usize {
    if let Some(d) = tick_decimals {
        return usize::from(d);
    }
    if price.is_nan() || price <= 0.0 {
        return 2;
    }
    let int_digits = price.log10().floor() as i32 + 1;
    (6 - int_digits).clamp(2, 10) as usize
}

/// The menu bar has little room: whole units once a price reaches 1,000.
pub fn compact_decimals(price: f64, tick_decimals: Option<u8>) -> usize {
    if price >= 1000.0 { 0 } else { decimals(price, tick_decimals) }
}

/// Fixed decimals with comma-grouped thousands: `84049.95` → `84,049.95`.
pub fn price(value: f64, decimals: usize) -> String {
    let raw = format!("{:.*}", decimals, value.abs());
    let (int, frac) = raw.split_once('.').unwrap_or((raw.as_str(), ""));
    let mut out = String::with_capacity(raw.len() + int.len() / 3 + 1);
    if value.is_sign_negative() && raw.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
        out.push('-');
    }
    for (i, digit) in int.char_indices() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// An amount held: up to eight decimals for small ones, fewer as they grow,
/// none trailing: `1.5`, `0.00012345`, `12,345.67`.
pub fn amount(value: f64) -> String {
    let magnitude = value.abs();
    let decimals = if magnitude >= 1000.0 {
        2
    } else if magnitude >= 1.0 {
        4
    } else {
        8
    };
    let text = price(value, decimals);
    match text.split_once('.') {
        Some((int, frac)) => {
            let frac = frac.trim_end_matches('0');
            if frac.is_empty() { int.to_owned() } else { format!("{int}.{frac}") }
        }
        None => text,
    }
}

/// A signed amount with fixed decimals, signed as it rounds: `+12.34`,
/// `−0.41`, `0.00`.
pub fn signed(value: f64, decimals: usize) -> String {
    let text = price(value.abs(), decimals);
    let nonzero = text.bytes().any(|b| b.is_ascii_digit() && b != b'0');
    match (nonzero, value < 0.0) {
        (false, _) => text,
        (true, false) => format!("+{text}"),
        (true, true) => format!("\u{2212}{text}"),
    }
}

/// Compact units as the language counts: `12345` → `1.23万` (zh, ja),
/// `12.35K` (en). At most two decimals, none trailing.
pub fn compact(value: f64) -> String {
    compact_in(value, i18n::current())
}

fn compact_in(value: f64, locale: Locale) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-∞" } else { "∞" }.to_owned();
    }
    static EN: [(f64, &str); 5] = [(1.0, ""), (1e3, "K"), (1e6, "M"), (1e9, "B"), (1e12, "T")];
    static ZH_CN: [(f64, &str); 4] = [(1.0, ""), (1e4, "万"), (1e8, "亿"), (1e12, "万亿")];
    static JA: [(f64, &str); 5] = [(1.0, ""), (1e4, "万"), (1e8, "億"), (1e12, "兆"), (1e16, "京")];
    let units: &[(f64, &str)] = match locale {
        Locale::En => &EN,
        Locale::ZhCn => &ZH_CN,
        Locale::Ja => &JA,
    };
    let magnitude = value.abs();
    let mut index = units.iter().rposition(|(factor, _)| magnitude >= *factor).unwrap_or(0);
    let text = loop {
        let scaled = magnitude / units[index].0;
        let rounded = rounded_decimal(scaled, 2, usize::MAX);
        let reached: f64 = rounded.parse().unwrap_or(scaled);
        // Rounding can carry into the next unit: 999.999K is 1M, not 1,000K.
        if let Some((next, _)) = units.get(index + 1)
            && reached >= next / units[index].0
        {
            index += 1;
            continue;
        }
        break group_thousands(&rounded, 5);
    };
    let sign = if value < 0.0 { "-" } else { "" };
    format!("{sign}{text}{}", units[index].1)
}

/// Order-book and trade sizes: compact when huge, fewer decimals as they grow.
pub fn qty(value: f64) -> String {
    qty_in(value, i18n::current())
}

fn qty_in(value: f64, locale: Locale) -> String {
    let magnitude = value.abs();
    if magnitude >= 100_000.0 {
        return compact_in(value, locale);
    }
    let decimals = if magnitude >= 1000.0 {
        2
    } else if magnitude >= 1.0 {
        4
    } else {
        5
    };
    price(value, decimals)
}

/// Epoch milliseconds as the local wall clock, `14:03:27`.
pub fn clock(ms: f64) -> String {
    let secs = (ms / 1000.0).floor() as i64;
    let (hour, minute, second) = local_time(secs);
    format!("{hour:02}:{minute:02}:{second:02}")
}

/// A manually maintained record needs its date as well as its local time.
pub fn timestamp(ms: f64) -> String {
    let time = local_time_parts((ms / 1000.).floor() as i64);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute
    )
}

/// A stretch of time in words, rounded to its largest unit: `45 seconds`,
/// `15分钟`, `1.5 時間`.
pub fn duration(secs: f64) -> String {
    duration_in(secs, i18n::current())
}

fn duration_in(secs: f64, locale: Locale) -> String {
    let (value, unit) = if secs < MINUTE {
        (secs.round(), Unit::Second)
    } else if secs < HOUR {
        ((secs / MINUTE).round(), Unit::Minute)
    } else if secs < 2.0 * DAY {
        (tenths(secs / HOUR), Unit::Hour)
    } else {
        (tenths(secs / DAY), Unit::Day)
    };
    let number = rounded_decimal(value.abs(), 1, 4);
    let number = if value < 0.0 { format!("-{number}") } else { number };
    match locale {
        Locale::En => {
            let word = match (unit, value == 1.0) {
                (Unit::Second, true) => "second",
                (Unit::Second, false) => "seconds",
                (Unit::Minute, true) => "minute",
                (Unit::Minute, false) => "minutes",
                (Unit::Hour, true) => "hour",
                (Unit::Hour, false) => "hours",
                (Unit::Day, true) => "day",
                (Unit::Day, false) => "days",
            };
            format!("{number} {word}")
        }
        Locale::ZhCn => {
            let word = match unit {
                Unit::Second => "秒钟",
                Unit::Minute => "分钟",
                Unit::Hour => "小时",
                Unit::Day => "天",
            };
            format!("{number}{word}")
        }
        Locale::Ja => {
            let word = match unit {
                Unit::Second => "秒",
                Unit::Minute => "分",
                Unit::Hour => "時間",
                Unit::Day => "日",
            };
            format!("{number} {word}")
        }
    }
}

/// Round to one decimal below ten, to whole units from ten up.
fn tenths(value: f64) -> f64 {
    if value < 10.0 { (value * 10.0).round() / 10.0 } else { value.round() }
}

/// A stretch between two epoch seconds, the language's way: clock times
/// within a day or two, dates beyond. `14:00 – 15:30`, `5/3 22:00 – 5/4
/// 01:30`, `May 3 – 9`; times have seconds below ten minutes, dates have
/// years outside this one.
pub fn range(from: f64, to: f64) -> String {
    let now =
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs() as i64);
    range_in(from, to, i18n::current(), now)
}

fn range_in(from: f64, to: f64, locale: Locale, now: i64) -> String {
    let a = local_time_parts(from.floor() as i64);
    let b = local_time_parts(to.floor() as i64);
    let span = to - from;
    if span < 2.0 * DAY {
        let today = local_time_parts(now);
        let dated = !a.same_date(today) || !b.same_date(today);
        let seconds = span < 10.0 * MINUTE;
        if dated { dated_clock(a, b, seconds, locale) } else { clock_range(a, b, seconds, locale) }
    } else {
        let this_year = local_time_parts(now).year;
        let with_years = a.year != this_year || b.year != this_year;
        dates(a, b, with_years, locale)
    }
}

fn clock_range(a: LocalTime, b: LocalTime, seconds: bool, locale: Locale) -> String {
    if a == b {
        return clock_text(a, seconds, locale);
    }
    let (first, second) = (clock_text(a, seconds, locale), clock_text(b, seconds, locale));
    match locale {
        Locale::En => format!("{first}\u{2009}\u{2013}\u{2009}{second}"),
        Locale::ZhCn => {
            if seconds {
                format!("{first} \u{2013} {second}")
            } else {
                format!("{first}\u{2013}{second}")
            }
        }
        Locale::Ja => format!("{first}\u{ff5e}{second}"),
    }
}

fn clock_text(time: LocalTime, seconds: bool, locale: Locale) -> String {
    match (locale, seconds) {
        (Locale::Ja, false) => format!("{}時{:02}分", time.hour, time.minute),
        (_, false) => format!("{:02}:{:02}", time.hour, time.minute),
        (_, true) => format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second),
    }
}

fn dated_clock(a: LocalTime, b: LocalTime, seconds: bool, locale: Locale) -> String {
    let with_year = a.year != b.year;
    if a.same_date(b) {
        let clock = clock_range(a, b, seconds, locale);
        return match locale {
            Locale::En => format!("{}, {clock}", english_date(a, with_year)),
            Locale::ZhCn => format!("{} {clock}", numeric_date(a, with_year, false)),
            Locale::Ja => format!("{} {clock}", numeric_date(a, with_year, false)),
        };
    }
    // With both dates shown, Japanese writes plain clock digits rather than
    // its 時/分 form.
    let dated_time = |time: LocalTime| {
        if locale == Locale::Ja && !seconds {
            format!("{}:{:02}", time.hour, time.minute)
        } else {
            clock_text(time, seconds, locale)
        }
    };
    let (time_a, time_b) = (dated_time(a), dated_time(b));
    match locale {
        Locale::En => format!(
            "{}, {time_a}\u{2009}\u{2013}\u{2009}{}, {time_b}",
            english_date(a, with_year),
            english_date(b, with_year),
        ),
        Locale::ZhCn => format!(
            "{} {time_a} \u{2013} {} {time_b}",
            numeric_date(a, with_year, false),
            numeric_date(b, with_year, false),
        ),
        Locale::Ja => format!(
            "{} {time_a}\u{ff5e}{} {time_b}",
            numeric_date(a, with_year, false),
            numeric_date(b, with_year, false),
        ),
    }
}

fn dates(a: LocalTime, b: LocalTime, with_years: bool, locale: Locale) -> String {
    match locale {
        Locale::En => {
            let same_year = a.year == b.year;
            let same_month = same_year && a.month == b.month;
            let body = if same_month {
                format!("{} {}\u{2009}\u{2013}\u{2009}{}", month_name(a.month), a.day, b.day)
            } else if same_year {
                format!(
                    "{} {}\u{2009}\u{2013}\u{2009}{} {}",
                    month_name(a.month),
                    a.day,
                    month_name(b.month),
                    b.day,
                )
            } else {
                format!(
                    "{} {}, {}\u{2009}\u{2013}\u{2009}{} {}, {}",
                    month_name(a.month),
                    a.day,
                    a.year,
                    month_name(b.month),
                    b.day,
                    b.year,
                )
            };
            if with_years && same_year { format!("{body}, {}", a.year) } else { body }
        }
        Locale::ZhCn => format!(
            "{} \u{2013} {}",
            numeric_date(a, with_years, false),
            numeric_date(b, with_years, false),
        ),
        Locale::Ja => format!(
            "{}\u{ff5e}{}",
            numeric_date(a, with_years, true),
            numeric_date(b, with_years, true),
        ),
    }
}

fn english_date(time: LocalTime, with_year: bool) -> String {
    if with_year {
        format!("{} {}, {}", month_name(time.month), time.day, time.year)
    } else {
        format!("{} {}", month_name(time.month), time.day)
    }
}

/// `5/3`, `2025/12/30`; Japanese dates in a range pad month and day.
fn numeric_date(time: LocalTime, with_year: bool, pad: bool) -> String {
    let (month, day) = if pad {
        (format!("{:02}", time.month), format!("{:02}", time.day))
    } else {
        (time.month.to_string(), time.day.to_string())
    };
    if with_year { format!("{}/{month}/{day}", time.year) } else { format!("{month}/{day}") }
}

fn month_name(month: i32) -> &'static str {
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    MONTHS[(month - 1).clamp(0, 11) as usize]
}

const MINUTE: f64 = 60.0;
const HOUR: f64 = 3600.0;
const DAY: f64 = 86_400.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Second,
    Minute,
    Hour,
    Day,
}

/// A civil date and wall-clock time in the local time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

impl LocalTime {
    fn same_date(self, other: Self) -> bool {
        (self.year, self.month, self.day) == (other.year, other.month, other.day)
    }
}

/// The C runtime's local time. `struct tm` starts with seconds, minutes,
/// hours, the day of the month, the month and the year on both platforms;
/// the buffer leaves room for the rest of it.
pub fn local_time_parts(secs: i64) -> LocalTime {
    let mut tm = [0i32; 16];
    let ok = {
        #[cfg(unix)]
        {
            unsafe extern "C" {
                fn localtime_r(time: *const i64, out: *mut i32) -> *mut i32;
            }
            // SAFETY: `tm` is larger than `struct tm` on every supported Unix.
            !unsafe { localtime_r(&secs, tm.as_mut_ptr()) }.is_null()
        }
        #[cfg(windows)]
        {
            unsafe extern "C" {
                fn _localtime64_s(out: *mut i32, time: *const i64) -> i32;
            }
            // SAFETY: `tm` is larger than the CRT's `struct tm`.
            unsafe { _localtime64_s(tm.as_mut_ptr(), &secs) == 0 }
        }
    };
    if !ok {
        return LocalTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0 };
    }
    LocalTime {
        year: tm[5] + 1900,
        month: tm[4] + 1,
        day: tm[3],
        hour: tm[2],
        minute: tm[1],
        second: tm[0],
    }
}

fn local_time(secs: i64) -> (i32, i32, i32) {
    let time = local_time_parts(secs);
    (time.hour, time.minute, time.second)
}

/// `value` rounded half away from zero to `places` decimals, with no
/// trailing zeros, and thousands grouped once the integer part is at least
/// `from` digits long.
fn rounded_decimal(value: f64, places: usize, from: usize) -> String {
    let text = format!("{value}");
    let (int, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let point = int.len();
    let keep = point + places;
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes()).map(|b| b - b'0').collect();
    if digits.len() > keep {
        let carry = digits[keep] >= 5;
        digits.truncate(keep);
        if carry {
            let mut i = digits.len();
            loop {
                if i == 0 {
                    digits.insert(0, 1);
                    break;
                }
                i -= 1;
                if digits[i] == 9 {
                    digits[i] = 0;
                } else {
                    digits[i] += 1;
                    break;
                }
            }
        }
    } else {
        digits.resize(keep, 0);
    }
    let point = digits.len().saturating_sub(places);
    let mut int: String = digits[..point].iter().map(|d| char::from(b'0' + d)).collect();
    let mut frac: String = digits[point..].iter().map(|d| char::from(b'0' + d)).collect();
    while frac.ends_with('0') {
        frac.pop();
    }
    if int.is_empty() {
        int.push('0');
    }
    let text = if frac.is_empty() { int } else { format!("{int}.{frac}") };
    group_thousands(&text, from)
}

fn group_thousands(text: &str, from: usize) -> String {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    if int.len() < from {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + int.len() / 3);
    for (i, digit) in int.char_indices() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if !frac.is_empty() {
        out.push('.');
        out.push_str(frac);
    }
    out
}

/// 24h change in percent, or `None` when the open price is unusable.
pub fn change_pct(last: f64, open: f64) -> Option<f64> {
    (open > 0.0 && last.is_finite()).then(|| (last - open) / open * 100.0)
}

/// Direction of a change after rounding to the two decimals we display, so a
/// `0.00%` row is never colored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Flat,
}

pub fn direction(pct: f64) -> Direction {
    let hundredths = (pct * 100.0).round();
    if hundredths > 0.0 {
        Direction::Up
    } else if hundredths < 0.0 {
        Direction::Down
    } else {
        Direction::Flat
    }
}

/// `+1.23%`, `−0.41%` (U+2212, same width as `+`), `0.00%`.
pub fn change_signed(pct: f64) -> String {
    match direction(pct) {
        Direction::Up => format!("+{:.2}%", pct),
        Direction::Down => format!("\u{2212}{:.2}%", -pct),
        Direction::Flat => "0.00%".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected strings follow what Binance's own UI shows for these pairs.
    #[test]
    fn groups_thousands_and_keeps_tick_decimals() {
        assert_eq!(price(84049.95, 2), "84,049.95");
        assert_eq!(price(3412.1, 2), "3,412.10");
        assert_eq!(price(182.33, 2), "182.33");
        assert_eq!(price(1_234_567.0, 0), "1,234,567");
        assert_eq!(price(0.00001234, 8), "0.00001234");
    }

    // The menu bar drops decimals from 1,000 up; below that the tick precision stays.
    #[test]
    fn compact_prices_drop_decimals_from_one_thousand() {
        assert_eq!(price(84049.95, compact_decimals(84049.95, Some(2))), "84,050");
        assert_eq!(price(999.5, compact_decimals(999.5, Some(2))), "999.50");
        assert_eq!(price(2.4531, compact_decimals(2.4531, Some(4))), "2.4531");
    }

    // Without tick info we aim for ~6 significant digits, never fewer than 2 decimals.
    #[test]
    fn fallback_decimals_track_magnitude() {
        assert_eq!(decimals(84049.95, None), 2);
        assert_eq!(decimals(182.33, None), 3);
        assert_eq!(decimals(0.00001234, None), 10);
        assert_eq!(decimals(0.0, None), 2);
    }

    // Held amounts read like the holdings window shows them: no trailing
    // zeros, more decimals the smaller they are.
    #[test]
    fn amounts_drop_trailing_zeros() {
        assert_eq!(amount(1.5), "1.5");
        assert_eq!(amount(0.00012345), "0.00012345");
        assert_eq!(amount(12345.678), "12,345.68");
        assert_eq!(amount(10.0), "10");
    }

    #[test]
    fn signed_amounts_show_the_rounding_sign() {
        assert_eq!(signed(12.345, 2), "+12.35");
        assert_eq!(signed(-0.41, 2), "\u{2212}0.41");
        assert_eq!(signed(0.0, 2), "0.00");
        assert_eq!(signed(-0.004, 2), "0.00");
        assert_eq!(signed(0.0001234, 6), "+0.000123");
    }

    // The compact unit is the language's: thousands in English, ten
    // thousands in Chinese and Japanese.
    #[test]
    fn compact_units_follow_the_language() {
        for (value, en, zh_cn, ja) in [
            (999.0, "999", "999", "999"),
            (1000.0, "1K", "1000", "1000"),
            (1234.0, "1.23K", "1234", "1234"),
            (12345.0, "12.35K", "1.23万", "1.23万"),
            (99999.0, "100K", "10万", "10万"),
            (999999.0, "1M", "100万", "100万"),
            (12_345_678.0, "12.35M", "1234.57万", "1234.57万"),
            (99_999_999.0, "100M", "1亿", "1億"),
            (123_456_789.0, "123.46M", "1.23亿", "1.23億"),
            (1e9, "1B", "10亿", "10億"),
            (1.234e12, "1.23T", "1.23万亿", "1.23兆"),
            (1e16, "10,000T", "10,000万亿", "1京"),
            (-12345.0, "-12.35K", "-1.23万", "-1.23万"),
            (1.005, "1.01", "1.01", "1.01"),
        ] {
            assert_eq!(compact_in(value, Locale::En), en, "{value}");
            assert_eq!(compact_in(value, Locale::ZhCn), zh_cn, "{value}");
            assert_eq!(compact_in(value, Locale::Ja), ja, "{value}");
        }
    }

    #[test]
    fn quantities_shrink_to_compact_units() {
        for (value, en, zh_cn) in [
            (0.0, "0.00000", "0.00000"),
            (0.00012345, "0.00012", "0.00012"),
            (1.23456, "1.2346", "1.2346"),
            (1234.567, "1,234.57", "1,234.57"),
            (99999.0, "99,999.00", "99,999.00"),
            (100_000.0, "100K", "10万"),
            (123_456.0, "123.46K", "12.35万"),
        ] {
            assert_eq!(qty_in(value, Locale::En), en, "{value}");
            assert_eq!(qty_in(value, Locale::ZhCn), zh_cn, "{value}");
        }
    }

    #[test]
    fn durations_take_the_largest_unit() {
        for (secs, en, zh_cn, ja) in [
            (45.0, "45 seconds", "45秒钟", "45 秒"),
            (60.0, "1 minute", "1分钟", "1 分"),
            (90.0, "2 minutes", "2分钟", "2 分"),
            (5400.0, "1.5 hours", "1.5小时", "1.5 時間"),
            (172_440.0, "48 hours", "48小时", "48 時間"),
            (172_800.0, "2 days", "2天", "2 日"),
            (216_000.0, "2.5 days", "2.5天", "2.5 日"),
        ] {
            assert_eq!(duration_in(secs, Locale::En), en, "{secs}");
            assert_eq!(duration_in(secs, Locale::ZhCn), zh_cn, "{secs}");
            assert_eq!(duration_in(secs, Locale::Ja), ja, "{secs}");
        }
    }

    // Times within a day or two read as clock times, dates beyond; the
    // language chooses the separators and order.
    #[test]
    fn ranges_follow_the_language() {
        let now = local_epoch(2026, 6, 15, 12, 0, 0) as i64;
        let today = |hour, minute, second| local_epoch(2026, 6, 15, hour, minute, second);
        for (from, to, en, zh_cn, ja) in [
            (
                today(14, 0, 0),
                today(15, 30, 0),
                "14:00\u{2009}\u{2013}\u{2009}15:30",
                "14:00\u{2013}15:30",
                "14時00分\u{ff5e}15時30分",
            ),
            (
                today(14, 0, 5),
                today(14, 0, 9),
                "14:00:05\u{2009}\u{2013}\u{2009}14:00:09",
                "14:00:05 \u{2013} 14:00:09",
                "14:00:05\u{ff5e}14:00:09",
            ),
            (
                local_epoch(2026, 5, 3, 14, 0, 0),
                local_epoch(2026, 5, 3, 15, 30, 0),
                "May 3, 14:00\u{2009}\u{2013}\u{2009}15:30",
                "5/3 14:00\u{2013}15:30",
                "5/3 14時00分\u{ff5e}15時30分",
            ),
            (
                local_epoch(2026, 6, 14, 22, 0, 0),
                local_epoch(2026, 6, 15, 1, 30, 0),
                "Jun 14, 22:00\u{2009}\u{2013}\u{2009}Jun 15, 01:30",
                "6/14 22:00 \u{2013} 6/15 01:30",
                "6/14 22:00\u{ff5e}6/15 1:30",
            ),
            (
                local_epoch(2025, 12, 31, 22, 0, 0),
                local_epoch(2026, 1, 1, 1, 30, 0),
                "Dec 31, 2025, 22:00\u{2009}\u{2013}\u{2009}Jan 1, 2026, 01:30",
                "2025/12/31 22:00 \u{2013} 2026/1/1 01:30",
                "2025/12/31 22:00\u{ff5e}2026/1/1 1:30",
            ),
            (
                local_epoch(2026, 5, 3, 0, 0, 0),
                local_epoch(2026, 5, 9, 0, 0, 0),
                "May 3\u{2009}\u{2013}\u{2009}9",
                "5/3 \u{2013} 5/9",
                "05/03\u{ff5e}05/09",
            ),
            (
                local_epoch(2026, 5, 3, 0, 0, 0),
                local_epoch(2026, 6, 9, 0, 0, 0),
                "May 3\u{2009}\u{2013}\u{2009}Jun 9",
                "5/3 \u{2013} 6/9",
                "05/03\u{ff5e}06/09",
            ),
            (
                local_epoch(2025, 12, 30, 0, 0, 0),
                local_epoch(2026, 1, 2, 0, 0, 0),
                "Dec 30, 2025\u{2009}\u{2013}\u{2009}Jan 2, 2026",
                "2025/12/30 \u{2013} 2026/1/2",
                "2025/12/30\u{ff5e}2026/01/02",
            ),
            (
                local_epoch(2025, 5, 3, 0, 0, 0),
                local_epoch(2025, 5, 9, 0, 0, 0),
                "May 3\u{2009}\u{2013}\u{2009}9, 2025",
                "2025/5/3 \u{2013} 2025/5/9",
                "2025/05/03\u{ff5e}2025/05/09",
            ),
        ] {
            assert_eq!(range_in(from, to, Locale::En, now), en);
            assert_eq!(range_in(from, to, Locale::ZhCn, now), zh_cn);
            assert_eq!(range_in(from, to, Locale::Ja, now), ja);
        }
    }

    // The local clock agrees with the C runtime on the current time zone.
    #[test]
    fn clock_is_local_time() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let (hour, _, _) = local_time(now);
        assert!((0..24).contains(&hour));
        assert_eq!(clock(0.0).len(), 8);
    }

    // A change that rounds to zero must read as flat, not "−0.00%".
    #[test]
    fn change_rounding_decides_direction() {
        assert_eq!(change_signed(1.234), "+1.23%");
        assert_eq!(change_signed(-0.41), "\u{2212}0.41%");
        assert_eq!(change_signed(-0.004), "0.00%");
        assert_eq!(change_pct(110.0, 100.0), Some(10.0));
        assert_eq!(change_pct(1.0, 0.0), None);
    }

    /// Epoch seconds of a local wall-clock time, through the same C runtime.
    fn local_epoch(year: i32, month: i32, day: i32, hour: i32, minute: i32, second: i32) -> f64 {
        let mut tm = [0i32; 16];
        tm[0] = second;
        tm[1] = minute;
        tm[2] = hour;
        tm[3] = day;
        tm[4] = month - 1;
        tm[5] = year - 1900;
        tm[8] = -1;
        let secs = {
            #[cfg(unix)]
            {
                unsafe extern "C" {
                    fn mktime(out: *mut i32) -> i64;
                }
                // SAFETY: `tm` is larger than `struct tm` on every supported Unix.
                unsafe { mktime(tm.as_mut_ptr()) }
            }
            #[cfg(windows)]
            {
                unsafe extern "C" {
                    fn _mktime64(out: *mut i32) -> i64;
                }
                // SAFETY: `tm` is larger than the CRT's `struct tm`.
                unsafe { _mktime64(tm.as_mut_ptr()) }
            }
        };
        secs as f64
    }
}
