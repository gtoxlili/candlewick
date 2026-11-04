//! The chart's live feed for a code: its snapshot every [`POLL`] while
//! Shanghai trades, as the menu bar reads it. The day's statistics come
//! straight from it. What the day's volume grew by since the snapshot before
//! is a trade at the new price, on the side the price moved to (the tick
//! rule), the way a level-1 quote screen fills its tape; a price that moved
//! without volume is a tick. There is no order book.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::ipc::Channel;
use tokio::{sync::oneshot, time::sleep};

use super::{POLL, api, session};
use crate::{
    market::{FeedState, LiveEvent, Stats, Trade},
    model::Instrument,
};

/// Failures in a row before the feed reads as offline.
const OFFLINE_AFTER: u32 = 2;
const MAX_BACKOFF_SECS: u64 = 60;
/// A snapshot whose volume is below this share of the last one's starts a
/// new day's count; above it, it is an older copy of the day from a server
/// that lags a few seconds behind the last one.
const NEW_DAY: f64 = 0.5;

pub(super) async fn run(
    instrument: Instrument,
    mut stop: oneshot::Receiver<()>,
    events: Channel<LiveEvent>,
) {
    let symbol = instrument.symbol;
    if !send(&events, LiveEvent::State { state: FeedState::Connecting }) {
        return;
    }
    let mut feed = Feed::default();
    let mut failures: u32 = 0;
    // The prices have been read since the market last closed.
    let mut settled = false;
    loop {
        let closed_for = session::until_open(session::now(), session::trading_today().await);
        let delay = if closed_for > 0 && settled {
            session::rest(closed_for)
        } else {
            match api::snapshot(&symbol).await {
                Ok(snapshot) => {
                    failures = 0;
                    settled = closed_for > 0;
                    let mut ok =
                        feed.live || send(&events, LiveEvent::State { state: FeedState::Live });
                    feed.live = true;
                    for event in feed.take(&snapshot, now_secs()) {
                        ok = ok && send(&events, event);
                    }
                    if !ok {
                        return;
                    }
                    if settled { session::rest(closed_for) } else { POLL }
                }
                Err(e) => {
                    log::debug!("tonghuashun snapshot of {symbol}: {e}");
                    failures += 1;
                    if failures >= OFFLINE_AFTER && feed.live {
                        feed.live = false;
                        if !send(&events, LiveEvent::State { state: FeedState::Offline }) {
                            return;
                        }
                    }
                    Duration::from_secs((1u64 << (failures - 1).min(6)).min(MAX_BACKOFF_SECS))
                }
            }
        };
        tokio::select! {
            _ = &mut stop => return,
            () = sleep(delay) => {}
        }
    }
}

/// What the feed knows between snapshots.
#[derive(Default)]
struct Feed {
    live: bool,
    last: Option<api::Snapshot>,
    /// The side of the last trade, kept while the price holds.
    sell: bool,
}

impl Feed {
    /// What a snapshot taken in at `now` (epoch seconds) says that the one
    /// before didn't, in order.
    fn take(&mut self, snapshot: &api::Snapshot, now: f64) -> Vec<LiveEvent> {
        let mut events = Vec::new();
        match self.last {
            Some(last)
                if snapshot.volume < last.volume && snapshot.volume >= last.volume * NEW_DAY =>
            {
                return events;
            }
            Some(last) if snapshot.volume > last.volume => {
                if snapshot.last != last.last {
                    self.sell = snapshot.last < last.last;
                }
                events.push(LiveEvent::Trades {
                    trades: vec![Trade {
                        // Seconds, then the day's volume: increasing, and
                        // below 2^53 for the webview.
                        id: ((now as u64) << 20) | (snapshot.volume as u64 & 0xF_FFFF),
                        price: snapshot.last,
                        qty: snapshot.volume - last.volume,
                        time: now * 1000.0,
                        sell: self.sell,
                        extended: false,
                    }],
                });
            }
            Some(last) if snapshot.last != last.last => {
                events.push(LiveEvent::Tick { price: snapshot.last, time: now * 1000.0 });
            }
            _ => {}
        }
        if self.last != Some(*snapshot) {
            events.push(LiveEvent::Stats { stats: stats(snapshot) });
        }
        self.last = Some(*snapshot);
        events
    }
}

fn stats(snapshot: &api::Snapshot) -> Stats {
    let change = snapshot.last - snapshot.prev_close;
    Stats {
        last: snapshot.last,
        high: snapshot.high,
        low: snapshot.low,
        volume: Some(snapshot.volume),
        turnover: snapshot.turnover,
        change,
        change_pct: if snapshot.prev_close > 0.0 {
            change / snapshot.prev_close * 100.0
        } else {
            0.0
        },
        onchain: None,
    }
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// False once the window is gone.
fn send(events: &Channel<LiveEvent>, event: LiveEvent) -> bool {
    events.send(event).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(last: f64, volume: f64) -> api::Snapshot {
        api::Snapshot { last, prev_close: 10.0, high: None, low: None, volume, turnover: 0.0 }
    }

    fn trades(events: &[LiveEvent]) -> Vec<(f64, f64, bool)> {
        events
            .iter()
            .filter_map(|event| match event {
                LiveEvent::Trades { trades } => {
                    Some(trades.iter().map(|t| (t.price, t.qty, t.sell)))
                }
                _ => None,
            })
            .flatten()
            .collect()
    }

    // Each snapshot's added volume is one print, signed by the tick rule; the
    // first only sets where counting starts.
    #[test]
    fn added_volume_prints_on_the_side_the_price_moved() {
        let mut feed = Feed::default();
        assert!(trades(&feed.take(&snapshot(10.0, 1000.0), 0.0)).is_empty());
        assert_eq!(trades(&feed.take(&snapshot(10.1, 1300.0), 3.0)), [(10.1, 300.0, false)]);
        assert_eq!(trades(&feed.take(&snapshot(10.0, 1400.0), 6.0)), [(10.0, 100.0, true)]);
        // An unchanged price keeps the side.
        assert_eq!(trades(&feed.take(&snapshot(10.0, 1450.0), 9.0)), [(10.0, 50.0, true)]);
        // A price that moved without volume (the opening auction) only ticks.
        let moved = feed.take(&snapshot(10.2, 1450.0), 12.0);
        assert!(trades(&moved).is_empty());
        assert!(moved.iter().any(|e| matches!(e, LiveEvent::Tick { price, .. } if *price == 10.2)));
    }

    // upstream: consecutive snapshots come from servers a few seconds apart
    // (their timestamps go back and forth, seen 2026-10-28), and the volume
    // starts from zero each morning.
    #[test]
    fn a_lagging_copy_is_skipped_and_a_new_day_starts_over() {
        let mut feed = Feed::default();
        feed.take(&snapshot(10.0, 1_000_000.0), 0.0);
        assert!(feed.take(&snapshot(9.9, 990_000.0), 3.0).is_empty());
        assert_eq!(trades(&feed.take(&snapshot(10.1, 1_000_500.0), 6.0)), [(10.1, 500.0, false)]);
        // The next morning: the new price moves the new day's candle.
        let morning = feed.take(&snapshot(10.3, 2_000.0), 86_400.0);
        assert!(trades(&morning).is_empty());
        assert!(
            morning.iter().any(|e| matches!(e, LiveEvent::Tick { price, .. } if *price == 10.3))
        );
        assert_eq!(trades(&feed.take(&snapshot(10.3, 2_600.0), 86_403.0)), [(10.3, 600.0, false)]);
    }
}
