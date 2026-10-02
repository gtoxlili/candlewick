//! Spot pairs on crypto exchanges: Binance, Bybit and OKX. Their public market
//! data has the same shape, so each exchange only says what sets it apart
//! ([`Exchange`]); search, the menu bar quotes, the chart's live feed and the
//! local order book are common to them.

pub mod account;
mod book;
mod live;
mod pairs;
mod quotes;

use std::{borrow::Cow, future::Future, time::Duration};

use serde::de::DeserializeOwned;
use tauri::{AppHandle, ipc::Channel};
use tokio::sync::{oneshot, watch};

pub use self::{
    book::{Entry, Ladder, Units},
    live::{Out, Session, backoff},
    pairs::Pair,
};
use super::{
    BoxFuture, Candle, ChartMode, ChartSpec, Error, IntervalSpec, LiveEvent, Provider, ProviderId,
    Search, Stats, StatsSpan, Trade,
};
use crate::{
    http,
    model::{FeedControl, Instrument},
};

/// How often a socket gets the exchange's keepalive frame, where it wants
/// one: Bybit asks for 20 seconds, OKX hangs up after 30 without traffic.
const KEEPALIVE: Duration = Duration::from_secs(20);

/// What sets one exchange apart; [`Provider`] comes with it.
pub trait Exchange: Send + Sync + 'static {
    const ID: ProviderId;
    /// REST hosts, tried in turn: the later ones serve the same data and
    /// answer on some networks where the first does not.
    const REST_HOSTS: &'static [&'static str];
    /// Websocket servers as host and port, tried the same way.
    const SOCKETS: &'static [(&'static str, u16)];
    /// The frame the exchange wants every [`KEEPALIVE`] to keep a socket
    /// open; `None` when it pings by itself.
    const PING: Option<&'static str>;
    /// What the chart offers; candles line up with the clock, days start at
    /// 00:00 UTC.
    const INTERVALS: &'static [Interval];

    /// The chart's live feed for one pair.
    type Live: Session;

    /// The pair's symbol as the exchange writes it: `BTCUSDT`, `BTC-USDT`.
    fn symbol(base: &str, quote: &str) -> String;

    /// The pair's page on the exchange's website, in the app's language
    /// where the site has it.
    fn link(instrument: &Instrument) -> String;

    /// Every spot pair being traded.
    fn pairs() -> impl Future<Output = Result<Vec<Pair>, Error>> + Send;

    /// See [`Provider::history`].
    fn history(
        instrument: &Instrument,
        interval: &Interval,
        end: Option<f64>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Candle>, Error>> + Send;

    /// See [`Provider::recent_trades`].
    fn recent_trades(
        instrument: &Instrument,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<Trade>, Error>> + Send;

    /// Where the menu bar's socket for `symbols` connects.
    fn quotes_path(symbols: &[String]) -> String;

    /// The frames that subscribe it to `symbols` once connected.
    fn quotes_subscribe(symbols: &[String]) -> Vec<String>;

    /// A pair's price from one of its frames.
    fn quote(text: &str) -> Option<Tick<'_>>;

    fn live(instrument: &Instrument) -> Self::Live;
}

/// A candle interval the chart offers.
pub struct Interval {
    pub secs: u32,
    /// The exchange's name for it.
    pub name: &'static str,
    /// How the chart first shows it.
    pub mode: ChartMode,
}

impl Interval {
    pub const fn new(secs: u32, name: &'static str) -> Self {
        // One-second candles are mostly noise; a line reads better.
        let mode = if secs < 60 { ChartMode::Line } else { ChartMode::Candle };
        Self { secs, name, mode }
    }
}

/// A pair's latest price, as the menu bar's socket brings it.
pub struct Tick<'a> {
    pub symbol: Cow<'a, str>,
    pub last: f64,
    /// The price 24 hours ago.
    pub open: f64,
}

impl<E: Exchange> Provider for E {
    fn validate(&self, instrument: &mut Instrument) -> Result<(), String> {
        for field in [&mut instrument.symbol, &mut instrument.base, &mut instrument.quote] {
            *field = field.trim().to_uppercase();
        }
        if !valid_asset(&instrument.base) || !valid_asset(&instrument.quote) {
            return Err(t!("error.invalidPair", symbol = instrument.symbol));
        }
        if instrument.symbol != E::symbol(&instrument.base, &instrument.quote) {
            return Err(t!("error.pairMismatch", symbol = instrument.symbol));
        }
        Ok(())
    }

    fn search<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Search> {
        Box::pin(pairs::search::<E>(query))
    }

    fn drop_search_cache(&self) {
        pairs::drop_cache(E::ID);
    }

    fn watch(
        &self,
        app: AppHandle,
        control: watch::Receiver<FeedControl>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(quotes::run::<E>(app, control))
    }

    fn chart_spec(&self, instrument: &Instrument) -> ChartSpec {
        ChartSpec {
            source: E::ID,
            intervals: E::INTERVALS
                .iter()
                .map(|interval| IntervalSpec {
                    secs: interval.secs,
                    mode: interval.mode,
                    aligned: true,
                    regular_only: false,
                })
                .collect(),
            stats_span: StatsSpan::Rolling24h,
            volume_unit: Some(instrument.base.clone()),
            turnover_unit: instrument.quote.clone(),
            day_offset: 0,
            book_steps: book::steps(instrument.decimals).into_iter().map(book::price).collect(),
            link: Some(E::link(instrument)),
        }
    }

    fn history<'a>(
        &'a self,
        instrument: &'a Instrument,
        interval: u32,
        end: Option<f64>,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Candle>, Error>> {
        Box::pin(async move {
            let interval = E::INTERVALS.iter().find(|i| i.secs == interval).ok_or_else(|| {
                Error::Message(t!("error.unsupportedInterval", seconds = interval))
            })?;
            E::history(instrument, interval, end, limit).await
        })
    }

    fn recent_trades<'a>(
        &'a self,
        instrument: &'a Instrument,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Trade>, Error>> {
        Box::pin(E::recent_trades(instrument, limit))
    }

    fn stream(
        &self,
        instrument: Instrument,
        stop: oneshot::Receiver<()>,
        events: Channel<LiveEvent>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(live::run::<E>(instrument, stop, events))
    }
}

/// A base or quote asset: letters and digits, in any script (Binance lists
/// 币安人生).
fn valid_asset(asset: &str) -> bool {
    !asset.is_empty() && asset.chars().count() <= 16 && asset.chars().all(char::is_alphanumeric)
}

/// GETs a public endpoint, trying each of the exchange's hosts in turn.
pub async fn get<E: Exchange, T: DeserializeOwned>(path: &str) -> Result<T, Error> {
    let mut failure = None;
    for host in E::REST_HOSTS {
        match http::get_json(&format!("https://{host}{path}")).await {
            Ok(value) => return Ok(value),
            Err(e) => failure = Some(e),
        }
    }
    Err(failure.expect("at least one host").into())
}

/// A number the exchange sent as text; NaN if it isn't one.
pub fn num(text: &str) -> f64 {
    text.parse().unwrap_or(f64::NAN)
}

/// `["openTime ms", "open", "high", "low", "close", …]`, as Bybit and OKX list
/// candles.
pub fn candle(row: &[String]) -> Option<Candle> {
    let [time, open, high, low, close] = [0, 1, 2, 3, 4].map(|i| row.get(i).map(|f| num(f)));
    Some(Candle { time: time? / 1000.0, open: open?, high: high?, low: low?, close: close? })
}

/// Day statistics from the last price and the one 24 hours before.
pub fn stats(last: f64, open: f64, high: f64, low: f64, volume: f64, turnover: f64) -> Stats {
    let change = last - open;
    let change_pct = if open > 0.0 { change / open * 100.0 } else { 0.0 };
    Stats { last, high, low, volume, turnover, change, change_pct }
}

/// The book grouped by `steps`, for the chart.
pub fn book_event(ladder: &Ladder, steps: &[Units]) -> LiveEvent {
    LiveEvent::Book { books: ladder.books(steps) }
}

/// The steps the chart groups `instrument`'s book by.
pub fn book_steps(instrument: &Instrument) -> Vec<Units> {
    book::steps(instrument.decimals)
}

#[cfg(test)]
pub use self::book::tests as book_tests;
