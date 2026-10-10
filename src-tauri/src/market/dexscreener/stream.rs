//! The chart's live feed for a pool, by polling. DEX Screener's statistics
//! every [`STATS`] carry the price, the 24-hour change and volume,
//! liquidity, market cap and the buy and sell counts; a changed price is
//! a tick that moves the forming candle. GeckoTerminal's trades come every
//! [`TRADES`] (less often while the pool is quiet), and its hourly candles
//! every [`RANGE`] give the 24-hour high and low, which neither service
//! states. Pools have no order book.

use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tauri::ipc::Channel;
use tokio::{
    sync::oneshot,
    time::{Instant, MissedTickBehavior, interval, sleep},
};

use super::{chains, gecko, parse, screener};
use crate::{
    market::{Candle, FeedState, LiveEvent, Stats, Trade},
    model::Instrument,
};

const STATS: Duration = Duration::from_secs(10);
const TRADES: Duration = Duration::from_secs(20);
/// A quiet pool is asked less and less often, up to this.
const TRADES_MAX: Duration = Duration::from_secs(60);
const RANGE: Duration = Duration::from_secs(600);
/// DEX Screener failures in a row before the feed reads as offline.
const OFFLINE_AFTER: u32 = 2;
/// Trade ids remembered, to send each trade once; a poll brings 300.
const REMEMBERED: usize = 1200;

pub(super) async fn run(
    instrument: Instrument,
    mut stop: oneshot::Receiver<()>,
    events: Channel<LiveEvent>,
) {
    let Some((chain, address)) = parse(&instrument.symbol) else {
        send(&events, LiveEvent::State { state: FeedState::Offline });
        return;
    };
    let network = chains::gecko_network(&chain).to_owned();
    if !send(&events, LiveEvent::State { state: FeedState::Connecting }) {
        return;
    }
    let mut feed = Feed::default();
    let mut failures: u32 = 0;
    let mut stats = interval(STATS);
    stats.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut range = interval(RANGE);
    range.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // After the statistics and the range, so their requests go first.
    let trades = sleep(Duration::from_secs(1));
    tokio::pin!(trades);

    loop {
        tokio::select! {
            _ = &mut stop => return,
            _ = stats.tick() => {
                let found = screener::pairs(&chain, std::slice::from_ref(&address)).await;
                let sent = match found.map(|mut pairs| pairs.pop()) {
                    Ok(Some(pair)) => {
                        failures = 0;
                        let now = now_secs();
                        let mut ok = feed.live || send(&events, LiveEvent::State { state: FeedState::Live });
                        feed.live = true;
                        if let Some(price) = pair.price()
                            && feed.last != Some(price)
                        {
                            feed.last = Some(price);
                            feed.range.add(price, now);
                            ok = ok && send(&events, LiveEvent::Tick { price, time: now * 1000.0 });
                        }
                        feed.pair = Some(pair);
                        ok && feed.send_stats(&events)
                    }
                    Ok(None) | Err(_) => {
                        failures += 1;
                        if failures >= OFFLINE_AFTER && feed.live {
                            feed.live = false;
                            send(&events, LiveEvent::State { state: FeedState::Offline })
                        } else {
                            true
                        }
                    }
                };
                if !sent {
                    return;
                }
            }
            () = &mut trades => {
                match gecko::trades(&network, &address).await {
                    Ok(list) => {
                        let fresh = feed.fresh(list);
                        let now = now_secs();
                        for trade in &fresh {
                            feed.range.add(trade.price, trade.time / 1000.0);
                        }
                        feed.trade_wait = if fresh.is_empty() { (feed.trade_wait * 2).min(TRADES_MAX) } else { TRADES };
                        if !fresh.is_empty() && !send(&events, LiveEvent::Trades { trades: fresh }) {
                            return;
                        }
                        feed.range.expire(now);
                    }
                    Err(e) => log::debug!("pool trades for {}: {e}", instrument.symbol),
                }
                // A closed gate is waited out rather than knocked on.
                let wait = gecko::blocked().map_or(feed.trade_wait, |left| left.max(feed.trade_wait));
                trades.as_mut().reset(Instant::now() + wait);
            }
            _ = range.tick() => {
                match gecko::candles(&network, &address, "hour", 1, None, 25).await {
                    Ok(candles) => {
                        feed.range.replace(&candles, now_secs());
                        if feed.pair.is_some() && !feed.send_stats(&events) {
                            return;
                        }
                    }
                    Err(e) => log::debug!("pool range for {}: {e}", instrument.symbol),
                }
            }
        }
    }
}

/// What the feed knows between polls.
struct Feed {
    live: bool,
    pair: Option<screener::Pair>,
    last: Option<f64>,
    range: Range,
    seen: HashSet<u64>,
    order: VecDeque<u64>,
    trade_wait: Duration,
}

impl Default for Feed {
    fn default() -> Self {
        Self {
            live: false,
            pair: None,
            last: None,
            range: Range::default(),
            seen: HashSet::new(),
            order: VecDeque::new(),
            trade_wait: TRADES,
        }
    }
}

impl Feed {
    fn stats(&self) -> Option<Stats> {
        let mut stats = self.pair.as_ref()?.stats(self.range.high(), self.range.low())?;
        if let Some(last) = self.last {
            stats.last = last;
        }
        Some(stats)
    }

    fn send_stats(&self, events: &Channel<LiveEvent>) -> bool {
        self.stats().is_none_or(|stats| send(events, LiveEvent::Stats { stats }))
    }

    /// The trades not sent before, oldest first.
    fn fresh(&mut self, trades: Vec<Trade>) -> Vec<Trade> {
        let mut fresh = Vec::new();
        for trade in trades {
            if self.seen.insert(trade.id) {
                self.order.push_back(trade.id);
                fresh.push(trade);
            }
        }
        while self.order.len() > REMEMBERED {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        fresh
    }
}

/// The last 24 hours' high and low: hourly candles for the bulk of it,
/// then every price seen since.
#[derive(Default)]
struct Range {
    /// `(time, high, low)` per hour, oldest first.
    hours: Vec<(f64, f64, f64)>,
    /// Prices seen since the hours were fetched, as `(time, price)`.
    ticks: Vec<(f64, f64)>,
}

const DAY: f64 = 86_400.0;

impl Range {
    fn replace(&mut self, candles: &[Candle], now: f64) {
        self.hours = candles.iter().map(|c| (c.time, c.high, c.low)).collect();
        self.ticks.clear();
        self.expire(now);
    }

    fn add(&mut self, price: f64, time: f64) {
        self.ticks.push((time, price));
    }

    /// Drops what left the window; an hour counts while any of it is in.
    fn expire(&mut self, now: f64) {
        let since = now - DAY;
        self.hours.retain(|(time, _, _)| time + 3600.0 > since);
        self.ticks.retain(|(time, _)| *time > since);
    }

    fn high(&self) -> Option<f64> {
        self.hours
            .iter()
            .map(|(_, high, _)| *high)
            .chain(self.ticks.iter().map(|(_, price)| *price))
            .reduce(f64::max)
    }

    fn low(&self) -> Option<f64> {
        self.hours
            .iter()
            .map(|(_, _, low)| *low)
            .chain(self.ticks.iter().map(|(_, price)| *price))
            .reduce(f64::min)
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

    #[test]
    fn the_range_spans_a_day_of_hours_and_the_ticks_since() {
        let now = 1_791_654_000.0;
        let hour = |ago: f64, high, low| Candle {
            time: now - ago * 3600.0,
            open: 0.0,
            high,
            low,
            close: 0.0,
        };
        let mut range = Range::default();
        // The oldest hour began 25 hours ago but ran into the window; the one before didn't.
        range.replace(&[hour(26.0, 9.0, 0.1), hour(24.5, 3.0, 1.0), hour(1.0, 2.0, 1.5)], now);
        assert_eq!((range.high(), range.low()), (Some(3.0), Some(1.0)));
        range.add(3.5, now);
        range.add(0.9, now - 2.0 * DAY);
        assert_eq!((range.high(), range.low()), (Some(3.5), Some(0.9)));
        range.expire(now);
        assert_eq!(range.low(), Some(1.0));
        assert_eq!(Range::default().high(), None);
    }

    #[test]
    fn each_trade_is_sent_once() {
        let trade =
            |id| Trade { id, price: 1.0, qty: 1.0, time: 0.0, sell: false, extended: false };
        let mut feed = Feed::default();
        assert_eq!(feed.fresh(vec![trade(1), trade(2)]).len(), 2);
        assert_eq!(feed.fresh(vec![trade(2), trade(3)]).len(), 1);
        let many: Vec<Trade> = (10..10 + REMEMBERED as u64 + 5).map(trade).collect();
        feed.fresh(many);
        assert!(feed.seen.len() <= REMEMBERED);
    }
}
