//! An order book kept locally from what an exchange sends, grouped by price
//! for the chart. Exchanges list a book two ways: Binance as a REST snapshot
//! that its diff stream keeps current ([`Ladder::update`]), Bybit and OKX as
//! the best levels only, which their streams keep that way
//! ([`Ladder::update_best`]).

use std::{borrow::Cow, collections::BTreeMap, fmt};

use serde::{
    Deserialize, Deserializer,
    de::{self, IgnoredAny, SeqAccess, Visitor},
};

use crate::market::{Book, Level};

/// A price in units of 10^-18, finer than any exchange quotes (Bybit goes to
/// 10^-13), so equal prices always meet in one level. A `u128` holds the
/// dearest pairs too, e.g. BTC/IDR at 1.5 billion.
pub type Units = u128;
const SCALE: u32 = 18;

/// Levels per side of each grouping the chart gets.
pub const LEVELS: usize = 20;

/// A level as exchanges list it, `["price", "quantity", …]`; quantity 0
/// removes it. Parsed in place: a REST snapshot has thousands.
pub struct Entry {
    price: Option<Units>,
    qty: Option<f64>,
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntryVisitor;

        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = Entry;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a [price, quantity] array")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Entry, A::Error> {
                let missing = || de::Error::custom("short level");
                let Text(price) = seq.next_element()?.ok_or_else(missing)?;
                let Text(qty) = seq.next_element()?.ok_or_else(missing)?;
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Entry { price: units(&price), qty: qty.parse().ok() })
            }
        }

        deserializer.deserialize_seq(EntryVisitor)
    }
}

/// A string field, borrowed when the input allows.
struct Text<'de>(Cow<'de, str>);

impl<'de> Deserialize<'de> for Text<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextVisitor;

        impl<'de> Visitor<'de> for TextVisitor {
            type Value = Text<'de>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string")
            }

            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Text<'de>, E> {
                Ok(Text(v.into()))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Text<'de>, E> {
                Ok(Text(v.to_owned().into()))
            }
        }

        deserializer.deserialize_str(TextVisitor)
    }
}

#[derive(Default)]
pub struct Ladder {
    bids: Side,
    asks: Side,
}

#[derive(Default)]
struct Side {
    /// Quantity by price.
    levels: BTreeMap<Units, f64>,
    /// The worst price listed, when the side may go on past it: what lies
    /// beyond is unknown.
    edge: Option<Units>,
}

impl Side {
    fn replace(&mut self, entries: &[Entry], depth: usize, bids: bool) {
        self.levels = entries
            .iter()
            .filter_map(|entry| Some((entry.price?, entry.qty?)))
            .filter(|&(_, qty)| qty > 0.0)
            .collect();
        self.edge = (entries.len() >= depth).then(|| self.worst(bids)).flatten();
    }

    /// Sets each changed level for which `known` holds, removes the others.
    fn apply(&mut self, entries: &[Entry], known: impl Fn(Units) -> bool) {
        for entry in entries {
            let (Some(price), Some(qty)) = (entry.price, entry.qty) else { continue };
            if qty > 0.0 && known(price) {
                self.levels.insert(price, qty);
            } else {
                self.levels.remove(&price);
            }
        }
    }

    fn worst(&self, bids: bool) -> Option<Units> {
        if bids { self.levels.keys().next() } else { self.levels.keys().next_back() }.copied()
    }
}

/// Whether `price` is no worse than `edge`.
fn within(edge: Option<Units>, price: Units, bids: bool) -> bool {
    edge.is_none_or(|edge| if bids { price >= edge } else { price <= edge })
}

impl Ladder {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Replaces the book with a listing of up to `depth` levels a side; a side
    /// that fills it may go on past its worst level.
    pub fn replace(&mut self, bids: &[Entry], asks: &[Entry], depth: usize) {
        self.bids.replace(bids, depth, true);
        self.asks.replace(asks, depth, false);
    }

    /// Applies changes to a snapshot. A level past the snapshot's edge stays
    /// out: the levels between it and the edge were never listed.
    pub fn update(&mut self, bids: &[Entry], asks: &[Entry]) {
        for (side, entries, is_bids) in
            [(&mut self.bids, bids, true), (&mut self.asks, asks, false)]
        {
            let edge = side.edge;
            side.apply(entries, |price| within(edge, price, is_bids));
        }
    }

    /// Applies changes from a stream that keeps the best `depth` levels a side
    /// (levels leave it as others enter), so a full side ends at its worst.
    pub fn update_best(&mut self, bids: &[Entry], asks: &[Entry], depth: usize) {
        for (side, entries, is_bids) in
            [(&mut self.bids, bids, true), (&mut self.asks, asks, false)]
        {
            side.apply(entries, |_| true);
            side.edge = (side.levels.len() >= depth).then(|| side.worst(is_bids)).flatten();
        }
    }

    /// A side that goes on past what is listed has fewer than [`LEVELS`] left:
    /// the price moved through nearly all the snapshot covered.
    pub fn thin(&self) -> bool {
        [&self.bids, &self.asks]
            .iter()
            .any(|side| side.edge.is_some() && side.levels.len() < LEVELS)
    }

    /// The book grouped by each step, or as it is when there are none. Bids
    /// round down and asks up, best first, and a side ends before the first
    /// group that reaches past its edge.
    pub fn books(&self, steps: &[Units]) -> Vec<Book> {
        let book = |step: Units| Book {
            bids: group(
                self.bids.levels.iter().rev(),
                |price| price / step * step,
                |key| within(self.bids.edge, key, true),
            ),
            asks: group(
                self.asks.levels.iter(),
                |price| price.div_ceil(step) * step,
                |key| within(self.asks.edge, key, false),
            ),
        };
        if steps.is_empty() {
            vec![book(1)]
        } else {
            steps.iter().map(|&step| book(step)).collect()
        }
    }
}

/// What the chart offers to group the book by: the tick and three coarser
/// powers of ten. None without a known tick.
pub fn steps(decimals: Option<u8>) -> Vec<Units> {
    match decimals.map(u32::from) {
        Some(decimals) if decimals <= SCALE => {
            (0..4).map(|k| 10u128.pow(SCALE - decimals + k)).collect()
        }
        _ => Vec::new(),
    }
}

/// The nearest `f64`, as parsing the decimal would give: the significant
/// digits fit 53 bits exactly, and so does their power of ten.
pub fn price(units: Units) -> f64 {
    let (mut digits, mut scale) = (units, SCALE);
    while scale > 0 && digits % 10 == 0 {
        digits /= 10;
        scale -= 1;
    }
    digits as f64 / 10f64.powi(scale as i32)
}

/// `"2.4567"` → 2_456_700_000_000_000_000, exactly.
fn units(price: &str) -> Option<Units> {
    let (whole, fraction) = price.split_once('.').unwrap_or((price, ""));
    let digits = u32::try_from(fraction.len()).ok().filter(|&digits| digits <= SCALE)?;
    let fraction = if fraction.is_empty() { 0 } else { fraction.parse::<Units>().ok()? };
    whole
        .parse::<Units>()
        .ok()?
        .checked_mul(10u128.pow(SCALE))?
        .checked_add(fraction * 10u128.pow(SCALE - digits))
}

/// Sums levels, best first, into groups while the groups are complete.
fn group<'a>(
    levels: impl Iterator<Item = (&'a Units, &'a f64)>,
    key: impl Fn(Units) -> Units,
    complete: impl Fn(Units) -> bool,
) -> Vec<Level> {
    let mut grouped: Vec<(Units, f64)> = Vec::with_capacity(LEVELS);
    for (&units, &qty) in levels {
        let key = key(units);
        if let Some((last, total)) = grouped.last_mut()
            && *last == key
        {
            *total += qty;
            continue;
        }
        if grouped.len() == LEVELS || !complete(key) {
            break;
        }
        grouped.push((key, qty));
    }
    grouped.into_iter().map(|(key, qty)| Level { price: price(key), qty }).collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Levels as an exchange would send them.
    pub fn entries(levels: &[(&str, &str)]) -> Vec<Entry> {
        let json: Vec<[&str; 2]> = levels.iter().map(|&(price, qty)| [price, qty]).collect();
        serde_json::from_str(&serde_json::to_string(&json).unwrap()).unwrap()
    }

    pub fn prices(levels: &[Level]) -> Vec<(f64, f64)> {
        levels.iter().map(|l| (l.price, l.qty)).collect()
    }

    const E: Units = 10u128.pow(SCALE);

    #[test]
    fn units_are_exact() {
        assert_eq!(units("2.4567"), Some(24_567 * E / 10_000));
        assert_eq!(units("118234.56"), Some(11_823_456 * E / 100));
        assert_eq!(units("0.0000000000001"), Some(100_000));
        assert_eq!(units("1502325313.00"), Some(1_502_325_313 * E));
        assert_eq!(units("7"), Some(7 * E));
        assert_eq!(units("0.0000000000000000001"), None);
        assert_eq!(units("-1"), None);
    }

    #[test]
    fn prices_come_back_as_written() {
        for text in ["2694.47", "2694.37", "0.0000000004145", "1502325313", "83917.3"] {
            assert_eq!(price(units(text).unwrap()), text.parse::<f64>().unwrap());
        }
    }

    #[test]
    fn entries_take_the_first_two_fields() {
        // OKX adds a deprecated field and the order count.
        let parsed: Vec<Entry> =
            serde_json::from_str(r#"[["41006.8","0.6","0","1"],["x","1"]]"#).unwrap();
        assert_eq!(parsed[0].price, Some(410_068 * E / 10));
        assert_eq!(parsed[0].qty, Some(0.6));
        assert_eq!(parsed[1].price, None);
    }

    #[test]
    fn steps_start_at_the_tick() {
        assert_eq!(steps(Some(4)), [E / 10_000, E / 1000, E / 100, E / 10]);
        assert_eq!(steps(Some(13))[0], 100_000);
        assert_eq!(steps(Some(0)), [E, 10 * E, 100 * E, 1000 * E]);
        assert!(steps(None).is_empty());
        assert!(steps(Some(19)).is_empty());
    }

    #[test]
    fn groups_round_bids_down_and_asks_up() {
        let mut ladder = Ladder::default();
        ladder.replace(
            &entries(&[("2.4566", "1"), ("2.4560", "2"), ("2.4559", "3")]),
            &entries(&[("2.4567", "4"), ("2.4570", "5"), ("2.4571", "6")]),
            5000,
        );
        let books = ladder.books(&steps(Some(4))[..2]);
        assert_eq!(prices(&books[0].bids), [(2.4566, 1.0), (2.456, 2.0), (2.4559, 3.0)]);
        assert_eq!(prices(&books[1].bids), [(2.456, 3.0), (2.455, 3.0)]);
        assert_eq!(prices(&books[1].asks), [(2.457, 9.0), (2.458, 6.0)]);
    }

    #[test]
    fn a_snapshot_keeps_out_levels_past_its_edge() {
        // A full snapshot of three levels: bids from 10 down to 8.
        let mut ladder = Ladder::default();
        ladder.replace(&entries(&[("10", "1"), ("9", "1"), ("8", "1")]), &[], 3);
        // Below it, a level that changes stays out: its neighbors were never seen.
        ladder.update(&entries(&[("7", "5"), ("9", "2")]), &[]);
        let books = ladder.books(&[E, 3 * E]);
        assert_eq!(prices(&books[0].bids), [(10.0, 1.0), (9.0, 2.0), (8.0, 1.0)]);
        // Groups of 3: the one from 6 would reach below 8.
        assert_eq!(prices(&books[1].bids), [(9.0, 3.0)]);
        assert!(ladder.thin());
    }

    #[test]
    fn the_best_levels_end_at_their_worst() {
        let mut ladder = Ladder::default();
        ladder.replace(&[], &entries(&[("1", "1"), ("2", "1"), ("3", "1")]), 3);
        // 1 leaves, 4 enters at the back: the stream keeps three levels.
        ladder.update_best(&[], &entries(&[("1", "0"), ("4", "1")]), 3);
        assert_eq!(prices(&ladder.books(&[])[0].asks), [(2.0, 1.0), (3.0, 1.0), (4.0, 1.0)]);
        assert_eq!(prices(&ladder.books(&[2 * E])[0].asks), [(2.0, 1.0), (4.0, 2.0)]);
        // Fewer than the stream keeps: the whole side.
        ladder.update_best(&[], &entries(&[("4", "0")]), 3);
        assert_eq!(prices(&ladder.books(&[2 * E])[0].asks), [(2.0, 1.0), (4.0, 1.0)]);
    }
}
