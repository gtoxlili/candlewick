//! The whole order book of one pair: a REST snapshot kept current by the diff
//! stream, the way Binance describes in "How to manage a local order book
//! correctly".

use std::collections::VecDeque;

use serde::Deserialize;

use crate::market::{
    Book,
    crypto::{Entry, Ladder, Units},
};

/// Levels per side the snapshot asks for, Binance's most. Past them a level
/// is known only once it changes, so the book stops where the snapshot did.
pub(super) const SNAPSHOT_LEVELS: usize = 5000;
/// Diffs held while a snapshot loads; the stream sends one a second.
const MAX_PENDING: usize = 120;

/// `GET /api/v3/depth`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    last_update_id: u64,
    bids: Vec<Entry>,
    asks: Vec<Entry>,
}

/// The `<symbol>@depth` stream: the levels that changed, quantity 0 once empty.
#[derive(Deserialize)]
pub(super) struct Diff {
    #[serde(rename = "U")]
    first: u64,
    #[serde(rename = "u")]
    last: u64,
    #[serde(rename = "b")]
    bids: Vec<Entry>,
    #[serde(rename = "a")]
    asks: Vec<Entry>,
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
    ladder: Ladder,
    /// Diffs that came while waiting for a snapshot.
    pending: VecDeque<Diff>,
}

impl Depth {
    /// Takes a snapshot, then the diffs that came while it loaded.
    pub(super) fn snapshot(&mut self, snapshot: Snapshot) -> Sync {
        self.ladder.replace(&snapshot.bids, &snapshot.asks, SNAPSHOT_LEVELS);
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
        self.ladder.update(&diff.bids, &diff.asks);
        self.id = Some(diff.last);
        if self.ladder.thin() {
            self.id = None;
            return Sync::Lost;
        }
        Sync::Changed
    }

    pub(super) fn books(&self, steps: &[Units]) -> Vec<Book> {
        self.ladder.books(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::crypto::book_tests::{entries, prices};

    fn diff(first: u64, last: u64, bids: &[(&str, &str)], asks: &[(&str, &str)]) -> Diff {
        Diff { first, last, bids: entries(bids), asks: entries(asks) }
    }

    fn snapshot(id: u64, bids: &[(&str, &str)], asks: &[(&str, &str)]) -> Snapshot {
        Snapshot { last_update_id: id, bids: entries(bids), asks: entries(asks) }
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
}
