//! The whole order book of one pair, grouped by price for the chart: a REST
//! snapshot kept current by the diff stream, the way Binance describes in
//! "How to manage a local order book correctly".

use std::collections::{BTreeMap, VecDeque};

use serde::Deserialize;

use crate::market::{Book, Level};

/// Levels per side the snapshot asks for, Binance's most. Past them a level
/// is known only once it changes, so the book stops where the snapshot did.
pub(super) const SNAPSHOT_LEVELS: usize = 5000;
/// Levels per side of each grouping the chart gets.
const LEVELS: usize = 20;
/// Diffs held while a snapshot loads; the stream sends one a second.
const MAX_PENDING: usize = 120;
/// Prices are kept in hundred-millionths, the finest Binance quotes, so
/// equal prices always meet in one level.
const UNITS_PER_PRICE: f64 = 1e8;

/// `GET /api/v3/depth`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Snapshot {
    last_update_id: u64,
    bids: Vec<(String, String)>,
    asks: Vec<(String, String)>,
}

/// The `<symbol>@depth` stream: the levels that changed, quantity 0 once empty.
#[derive(Deserialize)]
pub(super) struct Diff {
    #[serde(rename = "U")]
    first: u64,
    #[serde(rename = "u")]
    last: u64,
    #[serde(rename = "b")]
    bids: Vec<(String, String)>,
    #[serde(rename = "a")]
    asks: Vec<(String, String)>,
}

pub(super) enum Sync {
    Changed,
    Unchanged,
    /// Diffs were missed, or the price has moved through nearly all the
    /// snapshot covered: the book waits for a new snapshot.
    Lost,
}

#[derive(Default)]
pub(super) struct Depth {
    /// The last update applied; none while waiting for a snapshot.
    id: Option<u64>,
    /// Quantity by price in hundred-millionths.
    bids: BTreeMap<u64, f64>,
    asks: BTreeMap<u64, f64>,
    /// The lowest bid and highest ask the snapshot listed, when it didn't
    /// list the whole side.
    bid_floor: Option<u64>,
    ask_ceiling: Option<u64>,
    /// Diffs that came while waiting for a snapshot.
    pending: VecDeque<Diff>,
}

impl Depth {
    /// Takes a snapshot, then the diffs that came while it loaded.
    pub(super) fn snapshot(&mut self, snapshot: Snapshot) -> Sync {
        self.bids = levels(&snapshot.bids);
        self.asks = levels(&snapshot.asks);
        let partial = |side: &[(String, String)]| side.len() >= SNAPSHOT_LEVELS;
        self.bid_floor =
            partial(&snapshot.bids).then(|| self.bids.keys().next().copied()).flatten();
        self.ask_ceiling =
            partial(&snapshot.asks).then(|| self.asks.keys().next_back().copied()).flatten();
        self.id = Some(snapshot.last_update_id);
        let mut synced = Sync::Changed;
        for diff in std::mem::take(&mut self.pending) {
            if let Sync::Lost = self.diff(diff) {
                synced = Sync::Lost;
            }
        }
        synced
    }

    /// One diff from the stream.
    pub(super) fn diff(&mut self, diff: Diff) -> Sync {
        let Some(id) = self.id else {
            if self.pending.len() == MAX_PENDING {
                self.pending.pop_front();
            }
            self.pending.push_back(diff);
            return Sync::Unchanged;
        };
        if diff.last <= id {
            return Sync::Unchanged;
        }
        if diff.first > id + 1 {
            self.id = None;
            self.pending.push_back(diff);
            return Sync::Lost;
        }
        let (floor, ceiling) = (self.bid_floor, self.ask_ceiling);
        apply(&mut self.bids, &diff.bids, |price| floor.is_none_or(|floor| price >= floor));
        apply(&mut self.asks, &diff.asks, |price| ceiling.is_none_or(|ceiling| price <= ceiling));
        self.id = Some(diff.last);
        let thin =
            |side: &BTreeMap<u64, f64>, edge: Option<u64>| edge.is_some() && side.len() < LEVELS;
        if thin(&self.bids, floor) || thin(&self.asks, ceiling) {
            self.id = None;
            return Sync::Lost;
        }
        Sync::Changed
    }

    /// The book grouped by each step (in hundred-millionths), or as it is when
    /// there are none. Bids round down and asks up, best first, and a side
    /// ends before the first group the snapshot didn't fully cover.
    pub(super) fn books(&self, steps: &[u64]) -> Vec<Book> {
        let book = |step: u64| Book {
            bids: group(
                self.bids.iter().rev(),
                |price| price / step * step,
                |key| self.bid_floor.is_none_or(|floor| key >= floor),
            ),
            asks: group(
                self.asks.iter(),
                |price| price.div_ceil(step) * step,
                |key| self.ask_ceiling.is_none_or(|ceiling| key <= ceiling),
            ),
        };
        if steps.is_empty() {
            vec![book(1)]
        } else {
            steps.iter().map(|&step| book(step)).collect()
        }
    }
}

/// What the chart offers to group the book by, in hundred-millionths: the
/// tick and three coarser powers of ten. None without a known tick.
pub(super) fn steps(decimals: Option<u8>) -> Vec<u64> {
    match decimals {
        Some(decimals) if decimals <= 8 => {
            (0..4).map(|k| 10u64.pow(u32::from(8 - decimals) + k)).collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn price(units: u64) -> f64 {
    units as f64 / UNITS_PER_PRICE
}

/// `"2.45670000"` → 245_670_000, exactly.
fn units(price: &str) -> Option<u64> {
    let (whole, fraction) = price.split_once('.').unwrap_or((price, ""));
    let digits = u32::try_from(fraction.len()).ok().filter(|&digits| digits <= 8)?;
    let fraction = if fraction.is_empty() { 0 } else { fraction.parse::<u64>().ok()? };
    whole
        .parse::<u64>()
        .ok()?
        .checked_mul(100_000_000)?
        .checked_add(fraction * 10u64.pow(8 - digits))
}

fn levels(side: &[(String, String)]) -> BTreeMap<u64, f64> {
    side.iter()
        .filter_map(|(price, qty)| Some((units(price)?, qty.parse::<f64>().ok()?)))
        .filter(|&(_, qty)| qty > 0.0)
        .collect()
}

/// Sets each changed level, leaving out those past what the snapshot covered.
fn apply(
    side: &mut BTreeMap<u64, f64>,
    changes: &[(String, String)],
    covered: impl Fn(u64) -> bool,
) {
    for (price, qty) in changes {
        let (Some(price), Ok(qty)) = (units(price), qty.parse::<f64>()) else { continue };
        if qty > 0.0 && covered(price) {
            side.insert(price, qty);
        } else {
            side.remove(&price);
        }
    }
}

/// Sums levels, best first, into groups while the groups are complete.
fn group<'a>(
    levels: impl Iterator<Item = (&'a u64, &'a f64)>,
    key: impl Fn(u64) -> u64,
    complete: impl Fn(u64) -> bool,
) -> Vec<Level> {
    let mut grouped: Vec<(u64, f64)> = Vec::with_capacity(LEVELS);
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
mod tests {
    use super::*;

    fn side(levels: &[(&str, &str)]) -> Vec<(String, String)> {
        levels.iter().map(|&(price, qty)| (price.to_owned(), qty.to_owned())).collect()
    }

    fn diff(first: u64, last: u64, bids: &[(&str, &str)], asks: &[(&str, &str)]) -> Diff {
        Diff { first, last, bids: side(bids), asks: side(asks) }
    }

    fn snapshot(id: u64, bids: &[(&str, &str)], asks: &[(&str, &str)]) -> Snapshot {
        Snapshot { last_update_id: id, bids: side(bids), asks: side(asks) }
    }

    fn prices(levels: &[Level]) -> Vec<(f64, f64)> {
        levels.iter().map(|l| (l.price, l.qty)).collect()
    }

    #[test]
    fn units_are_exact() {
        assert_eq!(units("2.45670000"), Some(245_670_000));
        assert_eq!(units("118234.56"), Some(11_823_456_000_000));
        assert_eq!(units("0.00000001"), Some(1));
        assert_eq!(units("7"), Some(700_000_000));
        assert_eq!(units("0.000000001"), None);
        assert_eq!(units("-1"), None);
    }

    #[test]
    fn steps_start_at_the_tick() {
        assert_eq!(steps(Some(4)), [10_000, 100_000, 1_000_000, 10_000_000]);
        assert_eq!(steps(Some(0)), [100_000_000, 1_000_000_000, 10_000_000_000, 100_000_000_000]);
        assert!(steps(None).is_empty());
    }

    #[test]
    fn diffs_wait_for_the_snapshot_and_apply_in_order() {
        let mut depth = Depth::default();
        assert!(matches!(depth.diff(diff(95, 100, &[("1.0", "9")], &[])), Sync::Unchanged));
        assert!(matches!(
            depth.diff(diff(101, 103, &[("1.1", "2")], &[("1.3", "0")])),
            Sync::Unchanged
        ));
        // The first diff is already in the snapshot; the second applies.
        let synced = depth.snapshot(snapshot(100, &[("1.0", "5")], &[("1.2", "4"), ("1.3", "6")]));
        assert!(matches!(synced, Sync::Changed));
        let book = &depth.books(&[])[0];
        assert_eq!(prices(&book.bids), [(1.1, 2.0), (1.0, 5.0)]);
        assert_eq!(prices(&book.asks), [(1.2, 4.0)]);

        assert!(matches!(depth.diff(diff(102, 103, &[("1.1", "7")], &[])), Sync::Unchanged));
        assert!(matches!(depth.diff(diff(104, 104, &[("1.1", "0")], &[])), Sync::Changed));
        assert_eq!(prices(&depth.books(&[])[0].bids), [(1.0, 5.0)]);
        // A skipped update loses sync until the next snapshot.
        assert!(matches!(depth.diff(diff(106, 107, &[], &[])), Sync::Lost));
        assert!(matches!(depth.diff(diff(108, 108, &[], &[])), Sync::Unchanged));
        let synced = depth.snapshot(snapshot(107, &[("0.9", "1")], &[("1.4", "1")]));
        assert!(matches!(synced, Sync::Changed));
        assert_eq!(prices(&depth.books(&[])[0].bids), [(0.9, 1.0)]);
    }

    #[test]
    fn a_snapshot_older_than_the_diffs_is_lost() {
        let mut depth = Depth::default();
        depth.diff(diff(110, 112, &[], &[]));
        assert!(matches!(depth.snapshot(snapshot(100, &[], &[])), Sync::Lost));
    }

    #[test]
    fn groups_round_bids_down_and_asks_up() {
        let mut depth = Depth::default();
        depth.snapshot(snapshot(
            1,
            &[("2.4566", "1"), ("2.4560", "2"), ("2.4559", "3")],
            &[("2.4567", "4"), ("2.4570", "5"), ("2.4571", "6")],
        ));
        let books = depth.books(&steps(Some(4))[..2]);
        assert_eq!(prices(&books[0].bids), [(2.4566, 1.0), (2.456, 2.0), (2.4559, 3.0)]);
        assert_eq!(prices(&books[1].bids), [(2.456, 3.0), (2.455, 3.0)]);
        assert_eq!(prices(&books[1].asks), [(2.457, 9.0), (2.458, 6.0)]);
    }

    #[test]
    fn groups_stop_where_the_snapshot_did() {
        // A full snapshot: bids from 10 000 down to 5 001.
        let bids =
            (0..SNAPSHOT_LEVELS).map(|i| ((10_000 - i).to_string(), "1".to_owned())).collect();
        let mut depth = Depth::default();
        depth.snapshot(Snapshot { last_update_id: 1, bids, asks: Vec::new() });
        // Below it, a level that changes is left out: its neighbors were never seen.
        depth.diff(diff(2, 2, &[("5000", "8")], &[]));
        assert!(!depth.bids.contains_key(&units("5000").unwrap()));
        let books = depth.books(&[100_000_000, 1_000_000_000_000]);
        assert_eq!(books[0].bids.len(), LEVELS);
        // Groups of 10 000: the one from 0 would reach below 5 001.
        assert_eq!(prices(&books[1].bids), [(10_000.0, 1.0)]);
    }
}
