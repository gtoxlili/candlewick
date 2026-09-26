//! The chart's live feed for one stock: the day's statistics from quote
//! pushes, the book (as many levels as the account's quote package gives)
//! merged from depth pushes, and every trade. A snapshot starts it off and
//! follows every reconnection.

use std::{collections::BTreeMap, future::Future, pin::Pin};

use tauri::ipc::Channel;
use tokio::sync::oneshot;

use super::{
    decimal, latest,
    proto::{self, cmd, trade_session},
    service::{self, Push},
};
use crate::{
    market::{Book, Error, FeedState, Level, LiveEvent, Stats, Trade},
    model::Instrument,
};

type Snapshot = Pin<Box<dyn Future<Output = Result<(Day, Depth), Error>> + Send>>;

pub(super) async fn run(
    instrument: Instrument,
    mut stop: oneshot::Receiver<()>,
    events: Channel<LiveEvent>,
) {
    let symbol = instrument.symbol;
    if !send(&events, LiveEvent::State { state: FeedState::Connecting }) {
        return;
    }
    let Ok(mut watch) = service::watch(&symbol) else {
        send(&events, LiveEvent::State { state: FeedState::Offline });
        return;
    };
    let mut pending: Option<Snapshot> = Some(Box::pin(snapshot(symbol.clone())));
    // Pushes are changes to a snapshot, so they wait for one.
    let mut day: Option<Day> = None;
    let mut depth: Option<Depth> = None;

    loop {
        tokio::select! {
            _ = &mut stop => return,
            result = next(&mut pending) => {
                pending = None;
                let events_ok = match result {
                    Ok((fresh_day, fresh_depth)) => {
                        let (stats, book) = (fresh_day.stats(), fresh_depth.book());
                        day = Some(fresh_day);
                        depth = Some(fresh_depth);
                        send(&events, LiveEvent::Stats { stats })
                            && send(&events, LiveEvent::Book { book })
                            && send(&events, LiveEvent::State { state: FeedState::Live })
                    }
                    Err(e) => {
                        log::info!("chart snapshot for {symbol} failed: {e}");
                        send(&events, LiveEvent::State { state: FeedState::Offline })
                    }
                };
                if !events_ok {
                    return;
                }
            }
            push = watch.pushes.recv() => {
                let sent = match push {
                    None => {
                        send(&events, LiveEvent::State { state: FeedState::Offline });
                        return;
                    }
                    Some(Push::Offline) => send(&events, LiveEvent::State { state: FeedState::Offline }),
                    Some(Push::Resumed) => {
                        day = None;
                        depth = None;
                        pending = Some(Box::pin(snapshot(symbol.clone())));
                        true
                    }
                    Some(Push::Quote(push)) => match &mut day {
                        Some(day) => !day.apply(&push) || send(&events, LiveEvent::Stats { stats: day.stats() }),
                        None => true,
                    },
                    Some(Push::Depth(push)) => match &mut depth {
                        Some(depth) => {
                            depth.apply(&push.ask, &push.bid);
                            send(&events, LiveEvent::Book { book: depth.book() })
                        }
                        None => true,
                    },
                    Some(Push::Trades(push)) => {
                        let mut trades: Vec<Trade> = push.trade.iter().map(trade).collect();
                        trades.sort_by(|a, b| a.time.total_cmp(&b.time));
                        trades.is_empty() || send(&events, LiveEvent::Trades { trades })
                    }
                };
                if !sent {
                    return;
                }
            }
        }
    }
}

/// Resolves with the pending snapshot, or never while there is none.
async fn next(pending: &mut Option<Snapshot>) -> Result<(Day, Depth), Error> {
    match pending {
        Some(snapshot) => snapshot.await,
        None => std::future::pending().await,
    }
}

async fn snapshot(symbol: String) -> Result<(Day, Depth), Error> {
    let quote_request = proto::MultiSecurityRequest { symbol: vec![symbol.clone()] };
    let depth_request = proto::SecurityRequest { symbol };
    let (quotes, book) = tokio::join!(
        service::call::<_, proto::QuoteResponse>(cmd::QUOTE, &quote_request),
        service::call::<_, proto::DepthResponse>(cmd::DEPTH, &depth_request),
    );
    let quote = quotes?
        .secu_quote
        .into_iter()
        .next()
        .ok_or_else(|| Error::Message("长桥没有这只股票的报价".to_owned()))?;
    let mut depth = Depth::default();
    if let Ok(book) = book {
        depth.replace(&book.ask, &book.bid);
    }
    Ok((Day::from(&quote), depth))
}

/// The trading day so far, as the chart's statistics show it.
struct Day {
    last: f64,
    /// The close the change counts from (see `latest`).
    reference: f64,
    prev_close: f64,
    /// The regular session's last price: the close the extended sessions
    /// count from.
    regular_last: f64,
    high: f64,
    low: f64,
    volume: f64,
    turnover: f64,
}

impl Day {
    fn from(quote: &proto::SecurityQuote) -> Self {
        let (last, reference, _) = latest(quote);
        Self {
            last,
            reference,
            prev_close: decimal(&quote.prev_close),
            regular_last: decimal(&quote.last_done),
            high: decimal(&quote.high),
            low: decimal(&quote.low),
            volume: quote.volume as f64,
            turnover: decimal(&quote.turnover),
        }
    }

    /// Returns whether anything shown changed. Zero fields are unchanged;
    /// the day's range and volume are the regular session's.
    fn apply(&mut self, push: &proto::PushQuote) -> bool {
        const END_OF_DAY: i32 = 1;
        let last = decimal(&push.last_done);
        if push.tag == END_OF_DAY || last <= 0.0 {
            return false;
        }
        self.last = last;
        if push.trade_session == trade_session::INTRADAY {
            self.regular_last = last;
            self.reference = self.prev_close;
            let set = |field: &mut f64, value: f64| {
                if value > 0.0 {
                    *field = value;
                }
            };
            set(&mut self.high, decimal(&push.high));
            set(&mut self.low, decimal(&push.low));
            set(&mut self.volume, push.volume as f64);
            set(&mut self.turnover, decimal(&push.turnover));
        } else {
            self.reference = self.regular_last;
        }
        true
    }

    fn stats(&self) -> Stats {
        let change = self.last - self.reference;
        let change_pct = if self.reference > 0.0 { change / self.reference * 100.0 } else { 0.0 };
        Stats {
            last: self.last,
            high: self.high,
            low: self.low,
            volume: self.volume,
            turnover: self.turnover,
            change,
            change_pct,
        }
    }
}

/// The book by position (1 = best), as depth pushes update it.
#[derive(Default)]
struct Depth {
    asks: BTreeMap<i32, Level>,
    bids: BTreeMap<i32, Level>,
}

impl Depth {
    /// A queried book lists its levels best first, without positions.
    fn replace(&mut self, asks: &[proto::Depth], bids: &[proto::Depth]) {
        let numbered = |levels: &[proto::Depth]| -> Vec<proto::Depth> {
            (1..)
                .zip(levels)
                .map(|(position, level)| proto::Depth { position, ..level.clone() })
                .collect()
        };
        *self = Self::default();
        self.apply(&numbered(asks), &numbered(bids));
    }

    /// Pushed changes, by position. A level without a price has emptied.
    fn apply(&mut self, asks: &[proto::Depth], bids: &[proto::Depth]) {
        for (side, levels) in [(&mut self.asks, asks), (&mut self.bids, bids)] {
            for level in levels {
                let price = decimal(&level.price);
                if price > 0.0 {
                    side.insert(level.position, Level { price, qty: level.volume as f64 });
                } else {
                    side.remove(&level.position);
                }
            }
        }
    }

    fn book(&self) -> Book {
        Book {
            asks: self.asks.values().copied().collect(),
            bids: self.bids.values().copied().collect(),
        }
    }
}

/// Longbridge trades carry no id, so one is made from the second and the
/// trade's content: the same trade gets the same id whether it arrives in a
/// push or a query, and ids still order by time.
pub(super) fn trade(trade: &proto::Trade) -> Trade {
    // FNV-1a over what distinguishes trades within a second.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let content = [
        trade.price.as_bytes(),
        &trade.volume.to_le_bytes(),
        trade.trade_type.as_bytes(),
        &[trade.direction as u8],
    ];
    for byte in content.into_iter().flatten() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    const DOWN: i32 = 1;
    Trade {
        id: (trade.timestamp as u64) << 20 | (hash & 0xf_ffff),
        price: decimal(&trade.price),
        qty: trade.volume as f64,
        time: trade.timestamp as f64 * 1000.0,
        sell: trade.direction == DOWN,
        extended: trade.trade_session != trade_session::INTRADAY,
    }
}

/// False once the window is gone.
fn send(events: &Channel<LiveEvent>, event: LiveEvent) -> bool {
    events.send(event).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(price: &str, volume: i64, timestamp: i64) -> proto::Trade {
        proto::Trade {
            price: price.into(),
            volume,
            timestamp,
            trade_type: String::new(),
            direction: 2,
            trade_session: 0,
        }
    }

    #[test]
    fn trade_ids_are_stable_and_ordered_by_time() {
        let a = trade(&raw("10.5", 100, 1_790_000_000));
        let again = trade(&raw("10.5", 100, 1_790_000_000));
        let other = trade(&raw("10.6", 100, 1_790_000_000));
        let later = trade(&raw("10.4", 100, 1_790_000_001));
        assert_eq!(a.id, again.id);
        assert_ne!(a.id, other.id);
        assert!(later.id > a.id && later.id > other.id);
        assert!(a.id < 1 << 53, "exact as a JavaScript number");
    }

    #[test]
    fn depth_pushes_update_levels_by_position() {
        let level = |position, price: &str, volume| proto::Depth {
            position,
            price: price.into(),
            volume,
            order_num: 1,
        };
        let mut depth = Depth::default();
        depth.replace(&[level(0, "10.1", 5), level(0, "10.2", 7)], &[level(0, "10.0", 3)]);
        depth.apply(&[level(2, "", 0)], &[level(1, "9.9", 4)]);
        let book = depth.book();
        assert_eq!(book.asks.iter().map(|l| l.price).collect::<Vec<_>>(), [10.1]);
        assert_eq!(book.bids.iter().map(|l| (l.price, l.qty)).collect::<Vec<_>>(), [(9.9, 4.0)]);
    }
}
