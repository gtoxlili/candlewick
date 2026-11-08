//! Market data behind one interface, whichever service provides it.
//!
//! A [`Provider`] finds instruments, keeps the menu bar quotes of its
//! watchlist entries current, and serves the chart window: candle history,
//! recent trades and a live stream of trades, order book and day statistics.
//! The windows never reach a service themselves; they call the functions in
//! `api.rs`, which route by instrument id (`binance:BTCUSDT`). Supporting
//! another service means adding a provider here, nothing else; a crypto
//! exchange only needs what sets it apart from the others (`crypto::Exchange`).

mod binance;
mod bybit;
mod crypto;
pub mod dexscreener;
mod eastmoney;
pub mod longbridge;
mod okx;
pub mod ths;

use std::{collections::BTreeMap, future::Future, pin::Pin};

use crate::app::App;
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};

use self::crypto::account::Account;
use crate::{
    bar,
    credentials::ApiKey,
    http,
    model::{FeedControl, Instrument, Status},
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    Binance,
    Bybit,
    Okx,
    Longbridge,
    /// China A-shares, indexes and funds from Tonghuashun (同花顺).
    Ths,
    /// On-chain pools on decentralized exchanges.
    DexScreener,
    /// Public CNY fund NAVs and Shanghai Gold Exchange Au99.99 quotes.
    Eastmoney,
}

impl ProviderId {
    pub const ALL: [Self; 7] = [
        Self::Binance,
        Self::Bybit,
        Self::Okx,
        Self::Longbridge,
        Self::Ths,
        Self::DexScreener,
        Self::Eastmoney,
    ];

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
            Self::Ths => "ths",
            Self::DexScreener => "dexscreener",
            Self::Eastmoney => "eastmoney",
        }
    }

    /// As the user knows it, in the app's language.
    pub fn name(self) -> &'static str {
        match self {
            Self::Binance => t!("common.provider.binance"),
            Self::Bybit => t!("common.provider.bybit"),
            Self::Okx => t!("common.provider.okx"),
            Self::Longbridge => t!("common.provider.longbridge"),
            Self::Ths => t!("common.provider.ths"),
            Self::DexScreener => t!("common.provider.dexscreener"),
            Self::Eastmoney => t!("common.provider.eastmoney"),
        }
    }

    pub fn provider(self) -> &'static dyn Provider {
        match self {
            Self::Binance => &binance::Binance,
            Self::Bybit => &bybit::Bybit,
            Self::Okx => &okx::Okx,
            Self::Longbridge => &longbridge::Longbridge,
            Self::Ths => &ths::Ths,
            Self::DexScreener => &dexscreener::DexScreener,
            Self::Eastmoney => &eastmoney::Eastmoney,
        }
    }
}

/// Every registered source participates in the shared search. Crypto pairs
/// come from the selected exchange; the other sources are independent of it.
pub async fn search(exchange: ProviderId, query: &str) -> Search {
    let searches = ProviderId::ALL
        .into_iter()
        .filter(|provider| !provider.is_exchange() || *provider == exchange)
        .map(|provider| provider.provider().search(query));
    let mut all = Search::default();
    for found in futures_util::future::join_all(searches).await {
        all.candidates.extend(found.candidates);
        all.notes.extend(found.notes);
    }
    all
}

/// Keeps the holdings of each exchange with an API key current, for as
/// long as the app runs.
pub fn watch_accounts(app: &App, control: &watch::Receiver<FeedControl>) {
    let (app, control) = (app.clone(), control.clone());
    app.spawn(crypto::account::run::<binance::Binance>(app.clone(), control.clone()));
    app.spawn(crypto::account::run::<bybit::Bybit>(app.clone(), control.clone()));
    app.spawn(crypto::account::run::<okx::Okx>(app.clone(), control));
}

/// Asks `exchange` whether `key` works.
pub async fn check_key(exchange: ProviderId, key: &ApiKey) -> Result<(), Error> {
    match exchange {
        ProviderId::Binance => binance::Binance::check(key).await,
        ProviderId::Bybit => bybit::Bybit::check(key).await,
        ProviderId::Okx => okx::Okx::check(key).await,
        ProviderId::Longbridge
        | ProviderId::Ths
        | ProviderId::DexScreener
        | ProviderId::Eastmoney => {
            Err(Error::Message(t!("error.noApiKey", source = exchange.name())))
        }
    }
}

/// `binance:BTCUSDT`
pub fn instrument_id(provider: ProviderId, symbol: &str) -> String {
    format!("{}:{symbol}", provider.key())
}

pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Instrument semantics shared by watchlists, holdings and API consumers.
    fn instrument_meta(&self, instrument: &Instrument) -> InstrumentMeta {
        InstrumentMeta {
            asset_id: instrument.id(),
            quantity_unit: QuantityUnit::Units,
            price_kind: PriceKind::Market,
            usd_conversion: None,
        }
    }

    /// Normalizes an entry coming from the settings window and checks it.
    fn validate(&self, instrument: &mut Instrument) -> Result<(), String>;

    /// The instrument a symbol alone names, written as the provider writes
    /// it and known by nothing else: all reading market data takes
    /// (`agent::api`). `None` for a symbol that can't be one of its.
    fn bare(&self, symbol: &str) -> Option<Instrument> {
        let symbol = symbol.trim().to_uppercase();
        (!symbol.is_empty()).then(|| Instrument::bare(self.id(), &symbol))
    }

    /// Instruments matching what someone typed, best first, ready to be added
    /// to the watchlist. An empty query may start loading whatever searching
    /// needs, and finds nothing.
    fn search<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Search>;

    /// Frees whatever `search` keeps between calls: the settings window
    /// closed, so nobody is searching.
    fn drop_search_cache(&self) {}

    /// Keeps the quotes and feed status of this provider's watchlist entries
    /// current for as long as the app runs, following `control`.
    fn watch(&self, app: App, control: watch::Receiver<FeedControl>) -> BoxFuture<'static, ()>;

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
        events: tokio::sync::mpsc::UnboundedSender<LiveEvent>,
    ) -> BoxFuture<'static, ()>;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentMeta {
    pub asset_id: String,
    pub quantity_unit: QuantityUnit,
    pub price_kind: PriceKind,
    pub usd_conversion: Option<Instrument>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QuantityUnit {
    Tokens,
    Shares,
    FundUnits,
    Grams,
    Units,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PriceKind {
    Market,
    Nav { utc_offset: i64 },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Http(#[from] http::Error),
    #[error("{0}")]
    Message(String),
}

impl Error {
    /// The read may well work if asked again: the network or the service
    /// failed, rather than the service answering no (a refused key, a
    /// missing permission), which asking again won't change.
    pub fn transient(&self) -> bool {
        match self {
            Self::Message(_) => false,
            Self::Http(http::Error::Status(status)) => {
                matches!(status, 408 | 429) || *status >= 500 || *status < 400
            }
            Self::Http(_) => true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Search {
    pub candidates: Vec<Candidate>,
    /// Why results may be missing, e.g. a list that could not be loaded.
    pub notes: Vec<String>,
}

/// A search result: a watchlist entry, not yet pinned.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub instrument: Instrument,
    /// Taken from the typed text because the instrument list was unavailable.
    pub manual: bool,
    /// What a pool holds, in USD: tells the pools of one token apart.
    pub liquidity: Option<f64>,
}

/// What the chart window offers for an instrument: data, not text; the window
/// names it in its own language.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartSpec {
    pub source: ProviderId,
    pub intervals: Vec<IntervalSpec>,
    /// What the statistics cover.
    pub stats_span: StatsSpan,
    /// The base asset volume counts in; none for shares.
    pub volume_unit: Option<String>,
    pub turnover_unit: String,
    /// Seconds east of UTC at which daily candles open, for their date labels.
    pub day_offset: i64,
    /// Price steps the order book can be grouped by, finest first; book events
    /// carry it grouped by each. Empty: the book comes as it is.
    pub book_steps: Vec<f64>,
    /// The stream carries an order book; without one the window shows the
    /// trades alone.
    pub book: bool,
    /// The statistics are a pool's: volume in the quote currency only, with
    /// liquidity, market cap and swap counts beside (`Stats::onchain`).
    pub onchain: bool,
    /// The instrument's page on the provider's website.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsSpan {
    /// The last 24 hours, as crypto counts a day.
    Rolling24h,
    /// The current trading day.
    Today,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalSpec {
    pub secs: u32,
    /// How the chart first shows it.
    pub mode: ChartMode,
    /// Candles line up with the clock (`time % secs == 0`), so the chart can
    /// bucket trades itself; otherwise new candles come from the provider.
    pub aligned: bool,
    /// Trades outside the regular session don't count.
    pub regular_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartMode {
    Line,
    Candle,
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

/// Statistics over the provider's span (see [`ChartSpec::stats_span`]);
/// `None` where the provider gives none.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub last: f64,
    pub high: Option<f64>,
    pub low: Option<f64>,
    /// In the base asset (see [`ChartSpec::volume_unit`]).
    pub volume: Option<f64>,
    pub turnover: f64,
    pub change: Option<f64>,
    pub change_pct: Option<f64>,
    /// What a pool adds to the picture; none for exchanges and stocks.
    pub onchain: Option<Onchain>,
}

/// A pool's standing, over the same span as the rest of the statistics.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Onchain {
    /// USD in the pool.
    pub liquidity: Option<f64>,
    pub market_cap: Option<f64>,
    /// Fully diluted valuation, USD.
    pub fdv: Option<f64>,
    /// Swaps buying and selling the token.
    pub buys: u32,
    pub sells: u32,
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
    /// Increases with every trade of the instrument. Below 2^53, so a JSON
    /// reader keeps it exact.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedState {
    Connecting,
    Live,
    Offline,
}

#[derive(Debug, Clone)]
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
    /// A price without a trade behind it (a polled quote): moves the
    /// forming candle, not the trade list. `time` in epoch milliseconds.
    Tick {
        price: f64,
        time: f64,
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

/// Records a provider's feed status for the bar.
pub fn set_status(app: &App, provider: ProviderId, status: Status) {
    {
        let shared = app.shared();
        let mut model = shared.model();
        let feed = model.feeds.entry(provider).or_default();
        if feed.status == status {
            return;
        }
        match status {
            Status::Live => feed.stale = false,
            // Paused: nothing arrives while asleep, so on wake the prices on
            // screen are old until the stream is live again.
            Status::Retrying { .. } | Status::Paused | Status::NoCredentials => feed.stale = true,
            Status::Idle | Status::Connecting => {}
        }
        feed.status = status;
    }
    bar::request_render(app);
    app.notify(crate::app::Topic::Portfolio);
}

/// Every quote consumer reads the same price. Source time, when supplied,
/// is separate from receipt time (notably for daily fund NAVs).
pub fn record_quote(
    model: &mut crate::model::Model,
    id: String,
    mut quote: crate::model::Quote,
    source_time: Option<f64>,
) -> bool {
    if model.instrument(&id).is_none() || !quote.last.is_finite() || quote.last <= 0. {
        return false;
    }
    quote.time =
        crate::model::QuoteTime { received: crate::portfolio::now_ms(), source: source_time };
    let changed = model.quotes.get(&id).is_none_or(|old| {
        (old.last, old.open, old.session, old.time.source)
            != (quote.last, quote.open, quote.session, quote.time.source)
    });
    model.quotes.insert(id, quote);
    changed
}

/// Called at each provider's existing coalescing/poll boundary.
pub fn quotes_changed(app: &App) {
    bar::request_render(app);
    if !app.shared().model().manual.entries.is_empty() {
        app.notify(crate::app::Topic::Portfolio);
    }
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
