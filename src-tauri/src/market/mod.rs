//! Market data behind one interface, whichever service provides it.
//!
//! A [`Provider`] finds instruments, keeps the menu bar quotes of its
//! watchlist entries current, and serves the chart window: candle history,
//! recent trades and a live stream of trades, order book and day statistics.
//! The webviews never reach a service themselves; they call the commands in
//! `commands.rs`, which route by instrument id (`binance:BTCUSDT`). Supporting
//! another service means adding a provider here, nothing else; a crypto
//! exchange only needs what sets it apart from the others (`crypto::Exchange`).

mod binance;
mod bybit;
mod crypto;
pub mod longbridge;
mod okx;

use std::{collections::BTreeMap, future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, ipc::Channel};
use tokio::sync::{oneshot, watch};

use self::crypto::account::Account;
use crate::{
    bar,
    credentials::ApiKey,
    http,
    model::{FeedControl, Instrument, Shared, Status},
    window::{self, StatusView},
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    Binance,
    Bybit,
    Okx,
    Longbridge,
}

impl ProviderId {
    pub const ALL: [Self; 4] = [Self::Binance, Self::Bybit, Self::Okx, Self::Longbridge];

    /// The crypto exchanges, one of which the settings pick for every pair
    /// (`Settings::exchange`).
    pub const EXCHANGES: [Self; 3] = [Self::Binance, Self::Bybit, Self::Okx];

    pub fn is_exchange(self) -> bool {
        Self::EXCHANGES.contains(&self)
    }

    /// The prefix of instrument ids.
    pub fn key(self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Bybit => "bybit",
            Self::Okx => "okx",
            Self::Longbridge => "longbridge",
        }
    }

    /// For status lines.
    pub fn name(self) -> &'static str {
        match self {
            Self::Binance => "币安",
            Self::Bybit => "Bybit",
            Self::Okx => "OKX",
            Self::Longbridge => "长桥",
        }
    }

    pub fn provider(self) -> &'static dyn Provider {
        match self {
            Self::Binance => &binance::Binance,
            Self::Bybit => &bybit::Bybit,
            Self::Okx => &okx::Okx,
            Self::Longbridge => &longbridge::Longbridge,
        }
    }
}

/// Instruments matching `query`: pairs on `exchange`, and stocks.
pub async fn search(exchange: ProviderId, query: &str) -> Search {
    let searches =
        [exchange, ProviderId::Longbridge].map(|provider| provider.provider().search(query));
    let mut all = Search::default();
    for found in futures_util::future::join_all(searches).await {
        all.candidates.extend(found.candidates);
        all.notes.extend(found.notes);
    }
    all
}

/// Keeps the holdings of each exchange with an API key current, for as
/// long as the app runs.
pub fn watch_accounts(app: &AppHandle, control: &watch::Receiver<FeedControl>) {
    let (app, control) = (app.clone(), control.clone());
    tauri::async_runtime::spawn(crypto::account::run::<binance::Binance>(
        app.clone(),
        control.clone(),
    ));
    tauri::async_runtime::spawn(crypto::account::run::<bybit::Bybit>(app.clone(), control.clone()));
    tauri::async_runtime::spawn(crypto::account::run::<okx::Okx>(app, control));
}

/// Asks `exchange` whether `key` works.
pub async fn check_key(exchange: ProviderId, key: &ApiKey) -> Result<(), Error> {
    match exchange {
        ProviderId::Binance => binance::Binance::check(key).await,
        ProviderId::Bybit => bybit::Bybit::check(key).await,
        ProviderId::Okx => okx::Okx::check(key).await,
        ProviderId::Longbridge => Err(Error::Message("长桥不使用 API Key".to_owned())),
    }
}

/// `binance:BTCUSDT`
pub fn instrument_id(provider: ProviderId, symbol: &str) -> String {
    format!("{}:{symbol}", provider.key())
}

pub trait Provider: Send + Sync {
    /// Normalizes an entry coming from the settings window and checks it.
    fn validate(&self, instrument: &mut Instrument) -> Result<(), String>;

    /// Instruments matching what someone typed, best first, ready to be added
    /// to the watchlist. An empty query may start loading whatever searching
    /// needs, and finds nothing.
    fn search<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Search>;

    /// Frees whatever `search` keeps between calls: the settings window
    /// closed, so nobody is searching.
    fn drop_search_cache(&self) {}

    /// Keeps the quotes and feed status of this provider's watchlist entries
    /// current for as long as the app runs, following `control`.
    fn watch(
        &self,
        app: AppHandle,
        control: watch::Receiver<FeedControl>,
    ) -> BoxFuture<'static, ()>;

    /// What the chart window offers for `instrument`.
    fn chart_spec(&self, instrument: &Instrument) -> ChartSpec;

    /// Up to `limit` candles of `interval` seconds, oldest first: the latest
    /// ones, whose last is still forming, or those that opened before `end`.
    /// A service that pages smaller returns fewer; none means nothing older.
    fn history<'a>(
        &'a self,
        instrument: &'a Instrument,
        interval: u32,
        end: Option<f64>,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Candle>, Error>>;

    /// The latest trades, oldest first.
    fn recent_trades<'a>(
        &'a self,
        instrument: &'a Instrument,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Trade>, Error>>;

    /// Streams live data for `instrument` into `events` until `stop` fires
    /// (or its sender is dropped), reconnecting on its own meanwhile.
    fn stream(
        &self,
        instrument: Instrument,
        stop: oneshot::Receiver<()>,
        events: Channel<LiveEvent>,
    ) -> BoxFuture<'static, ()>;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Http(#[from] http::Error),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Search {
    pub candidates: Vec<Candidate>,
    /// Why results may be missing, e.g. a list that could not be loaded.
    pub notes: Vec<String>,
}

/// A search result: a watchlist entry, not yet pinned.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    #[serde(flatten)]
    pub instrument: Instrument,
    /// Taken from the typed text because the instrument list was unavailable.
    pub manual: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSpec {
    /// The service's name, for messages like "cannot reach …".
    pub source: &'static str,
    pub intervals: Vec<IntervalSpec>,
    /// What the statistics cover, e.g. `24h`.
    pub stats_span: &'static str,
    pub volume_unit: String,
    pub turnover_unit: String,
    /// Seconds east of UTC at which daily candles open, for their date labels.
    pub day_offset: i64,
    /// Price steps the order book can be grouped by, finest first; book events
    /// carry it grouped by each. Empty: the book comes as it is.
    pub book_steps: Vec<f64>,
    pub link: Option<Link>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntervalSpec {
    pub secs: u32,
    pub label: &'static str,
    /// How the chart first shows it.
    pub mode: ChartMode,
    /// Candles line up with the clock (`time % secs == 0`), so the chart can
    /// bucket trades itself; otherwise new candles come from the provider.
    pub aligned: bool,
    /// Trades outside the regular session don't count.
    pub regular_only: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartMode {
    Line,
    Candle,
}

#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub label: &'static str,
    pub url: String,
}

/// Times are epoch seconds, as the chart library takes them.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Candle {
    pub time: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// Statistics over the provider's span (see [`ChartSpec::stats_span`]).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub last: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub turnover: f64,
    pub change: f64,
    pub change_pct: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Level {
    pub price: f64,
    pub qty: f64,
}

/// Best level first on both sides.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Book {
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Trade {
    /// Increases with every trade of the instrument. Below 2^53, so the
    /// webview reads it exactly.
    pub id: u64,
    pub price: f64,
    pub qty: f64,
    /// Epoch milliseconds.
    pub time: f64,
    /// The taker sold.
    pub sell: bool,
    /// Outside the regular session (US pre-market, post-market, overnight).
    pub extended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FeedState {
    Connecting,
    Live,
    Offline,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LiveEvent {
    State {
        state: FeedState,
    },
    Stats {
        stats: Stats,
    },
    /// The book grouped by each of the chart spec's `book_steps` in turn, or
    /// just the book when there are none.
    Book {
        books: Vec<Book>,
    },
    /// New trades, oldest first.
    Trades {
        trades: Vec<Trade>,
    },
}

/// This provider's part of the feed control: its symbols, and whether the
/// machine is being looked at.
pub fn wanted(control: &FeedControl, provider: ProviderId) -> (Vec<String>, u8) {
    (control.symbols.get(&provider).cloned().unwrap_or_default(), control.paused)
}

/// Waits until this provider's part of `control` differs from `current`;
/// fails once the app is shutting down.
pub async fn changed(
    control: &mut watch::Receiver<FeedControl>,
    provider: ProviderId,
    current: &(Vec<String>, u8),
) -> Result<(), watch::error::RecvError> {
    loop {
        control.changed().await?;
        if &wanted(&control.borrow_and_update(), provider) != current {
            return Ok(());
        }
    }
}

/// Records a provider's feed status for the menu bar and settings window.
pub fn set_status(app: &AppHandle, provider: ProviderId, status: Status) {
    let shared = app.state::<Shared>();
    let view = {
        let mut model = shared.model();
        let feed = model.feeds.entry(provider).or_default();
        if feed.status == status {
            return;
        }
        match status {
            Status::Live(_) => feed.stale = false,
            // Paused: nothing arrives while asleep, so on wake the prices on
            // screen are old until the stream is live again.
            Status::Retrying { .. } | Status::Paused | Status::Unavailable(_) => feed.stale = true,
            Status::Idle | Status::Connecting => {}
        }
        feed.status = status;
        StatusView::from(&*model)
    };
    bar::request_render(app);
    window::emit_status(app, &view);
}

/// Feed status of every provider, by provider.
pub type Feeds = BTreeMap<ProviderId, Feed>;

#[derive(Debug, Clone, Default)]
pub struct Feed {
    pub status: Status,
    /// Prices on screen may be old: set when a connection fails, cleared on
    /// the next successful connection.
    pub stale: bool,
}
