//! Number formatting for the menu bar title and the dropdown rows.

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

    // A change that rounds to zero must read as flat, not "−0.00%".
    #[test]
    fn change_rounding_decides_direction() {
        assert_eq!(change_signed(1.234), "+1.23%");
        assert_eq!(change_signed(-0.41), "\u{2212}0.41%");
        assert_eq!(change_signed(-0.004), "0.00%");
        assert_eq!(change_pct(110.0, 100.0), Some(10.0));
        assert_eq!(change_pct(1.0, 0.0), None);
    }
}
