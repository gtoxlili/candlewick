//! Settings, live quotes and feed status, shared by the feed tasks, the bar
//! renderer and the windows' API (`api.rs`).

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU32},
    },
};

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};

use crate::{
    credentials::Credentials,
    i18n::Language,
    market::{self, Feeds, ProviderId},
    portfolio,
};

pub const MAX_INSTRUMENTS: usize = 30;

/// A watchlist entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instrument {
    pub provider: ProviderId,
    /// The provider's symbol, e.g. `BTCUSDT`.
    pub symbol: String,
    /// Crypto: the base asset (`BTC`). Stocks: the code (`AAPL`, `700`).
    pub base: String,
    /// Crypto: the quote asset (`USDT`). Stocks: the currency.
    pub quote: String,
    /// A stock's name, e.g. 腾讯控股.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Decimals of the tick size. `None` falls back to a magnitude rule.
    #[serde(default)]
    pub decimals: Option<u8>,
    /// Shown in the bar (the menu bar, the taskbar), not only in the
    /// dropdown. At most one entry is.
    #[serde(default)]
    pub pinned: bool,
    /// Where a pool trades: its chain and DEX, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<Venue>,
}

/// Where an on-chain pool trades.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Venue {
    /// `Solana`, `BNB Chain`
    pub chain: String,
    /// `Raydium`, `Uniswap`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dex: Option<String>,
}

impl Instrument {
    pub fn metadata(&self) -> market::InstrumentMeta {
        self.provider.provider().instrument_meta(self)
    }

    fn preset(base: &str, pinned: bool) -> Self {
        Self {
            provider: ProviderId::Binance,
            symbol: format!("{base}USDT"),
            base: base.to_owned(),
            quote: "USDT".to_owned(),
            name: None,
            decimals: Some(2),
            pinned,
            venue: None,
        }
    }

    /// An instrument known by its symbol alone (see `Provider::bare`).
    pub fn bare(provider: ProviderId, symbol: &str) -> Self {
        Self {
            provider,
            symbol: symbol.to_owned(),
            base: String::new(),
            quote: String::new(),
            name: None,
            decimals: None,
            pinned: false,
            venue: None,
        }
    }

    /// `binance:BTCUSDT`: how windows and menus refer to it.
    pub fn id(&self) -> String {
        market::instrument_id(self.provider, &self.symbol)
    }

    /// The bar's name for it: `BTC` for USD-like quotes, `ETH/BTC`
    /// otherwise; a pool's token; US tickers as they are, other stocks by
    /// name.
    pub fn short_label(&self) -> String {
        if self.provider.is_exchange() {
            return if is_usd_like(&self.quote) { self.base.clone() } else { self.pair_label() };
        }
        if self.provider == ProviderId::DexScreener {
            return self.base.clone();
        }
        match &self.name {
            Some(name) if !self.symbol.ends_with(".US") => name.clone(),
            _ => self.base.clone(),
        }
    }

    /// `BTC/USDT`; a pool's token and chain: `$WIF · Solana`; a stock's
    /// name and code: `苹果 AAPL`, `腾讯控股 700`.
    pub fn pair_label(&self) -> String {
        if self.provider.is_exchange() {
            return format!("{}/{}", self.base, self.quote);
        }
        if self.provider == ProviderId::DexScreener {
            return match &self.venue {
                Some(venue) => format!("{} · {}", self.base, venue.chain),
                None => self.base.clone(),
            };
        }
        match &self.name {
            Some(name) => format!("{name} {}", self.base),
            None => self.base.clone(),
        }
    }

    /// The dropdown row's name and the dimmed part after it: `BTC` `/USDT`,
    /// `$WIF` ` Solana`, `AAPL` ` 苹果`, `腾讯控股` ` 700`.
    pub fn row_label(&self) -> (String, String) {
        if self.provider.is_exchange() {
            return (self.base.clone(), format!("/{}", self.quote));
        }
        if self.provider == ProviderId::DexScreener {
            let chain = self.venue.as_ref().map(|v| format!(" {}", v.chain)).unwrap_or_default();
            return (self.base.clone(), chain);
        }
        let short = self.short_label();
        let other = if short == self.base { self.name.clone() } else { Some(self.base.clone()) };
        (short, other.map(|other| format!(" {other}")).unwrap_or_default())
    }
}

fn is_usd_like(quote: &str) -> bool {
    matches!(quote, "USDT" | "USDC" | "FDUSD" | "USD1" | "TUSD" | "BUSD" | "USD" | "U")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorScheme {
    #[default]
    GreenUp,
    RedUp,
}

/// Decimals of a tick size past this are taken as unknown; exchanges quote no
/// finer than 10^-13.
const MAX_DECIMALS: u8 = 18;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Display unit for portfolio values, independent of market quote units.
    pub currency: crate::currency::Currency,
    /// The crypto exchange every pair comes from (one of
    /// [`ProviderId::EXCHANGES`]); the watchlist holds no other's.
    pub exchange: ProviderId,
    pub watchlist: Vec<Instrument>,
    pub show_symbol: bool,
    pub show_change: bool,
    /// The bar shows the total holdings instead of a pinned entry (which is
    /// then unpinned: the bar shows one thing).
    pub holdings_in_bar: bool,
    pub color_scheme: ColorScheme,
    /// Check for, download and apply updates on its own (`update.rs`).
    pub auto_update: bool,
    /// AI agents may read data and maintain manual holdings (`agent`).
    pub agent_access: bool,
    /// What the app speaks (`i18n`).
    pub language: Language,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            currency: crate::currency::Currency::default(),
            exchange: ProviderId::Binance,
            watchlist: vec![
                Instrument::preset("BTC", true),
                Instrument::preset("ETH", false),
                Instrument::preset("SOL", false),
            ],
            show_symbol: true,
            show_change: false,
            holdings_in_bar: false,
            color_scheme: ColorScheme::GreenUp,
            auto_update: true,
            agent_access: false,
            language: Language::System,
        }
    }
}

impl Settings {
    /// Normalizes and checks settings coming from the settings window. Pairs
    /// of another exchange than the one picked are dropped: picking another
    /// one takes them off the watchlist.
    pub fn validated(mut self) -> Result<Self, String> {
        if !self.exchange.is_exchange() {
            return Err(t!("error.unsupportedExchange", name = self.exchange.name()));
        }
        let exchange = self.exchange;
        self.watchlist.retain(|instrument| {
            !instrument.provider.is_exchange() || instrument.provider == exchange
        });
        if self.watchlist.len() > MAX_INSTRUMENTS {
            return Err(t!("error.watchlistFull", max = MAX_INSTRUMENTS));
        }
        let mut seen = HashSet::new();
        for instrument in &mut self.watchlist {
            instrument.provider.provider().validate(instrument)?;
            if instrument.decimals.is_some_and(|d| d > MAX_DECIMALS) {
                instrument.decimals = None;
            }
            if !seen.insert(instrument.id()) {
                return Err(t!("error.duplicate", symbol = instrument.symbol));
            }
        }
        if self.watchlist.iter().filter(|instrument| instrument.pinned).count() > 1 {
            return Err(t!("error.onePinned").to_owned());
        }
        if self.holdings_in_bar {
            for instrument in &mut self.watchlist {
                instrument.pinned = false;
            }
        }
        Ok(self)
    }

    /// The entry shown in the bar, if any.
    pub fn pinned(&self) -> Option<&Instrument> {
        self.watchlist.iter().find(|instrument| instrument.pinned)
    }

    pub fn instrument(&self, id: &str) -> Option<&Instrument> {
        self.watchlist.iter().find(|instrument| instrument.id() == id)
    }

    /// What each provider streams: the symbol sets only, order and flags don't matter.
    pub fn symbols(&self) -> BTreeMap<ProviderId, Vec<String>> {
        let mut symbols: BTreeMap<ProviderId, Vec<String>> = BTreeMap::new();
        for instrument in &self.watchlist {
            symbols.entry(instrument.provider).or_default().push(instrument.symbol.clone());
        }
        for list in symbols.values_mut() {
            list.sort();
        }
        symbols
    }
}

/// Reads settings; a missing or corrupt file yields defaults.
pub fn load(path: &Path) -> Settings {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
            Ok(settings) => settings.validated().unwrap_or_default(),
            Err(e) => {
                log::warn!("settings file unreadable, using defaults: {e}");
                Settings::default()
            }
        },
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("cannot read settings: {e}");
            }
            Settings::default()
        }
    }
}

/// Writes through a temp file so a crash never leaves half a JSON document.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(&tmp, path)
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Quote {
    pub last: f64,
    /// Reference price (24h ago, previous close or previous NAV); zero when unknown.
    pub open: f64,
    /// Set outside the regular session (US stocks).
    pub session: Option<Session>,
    pub time: QuoteTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Session {
    Pre,
    Post,
    Overnight,
}

impl Session {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pre => t!("session.pre"),
            Self::Post => t!("session.post"),
            Self::Overnight => t!("session.overnight"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Status {
    /// Nothing to stream.
    Idle,
    Paused,
    #[default]
    Connecting,
    Live,
    /// The connection failed; the next attempt is this far off.
    Retrying {
        retry_in_secs: u64,
    },
    /// Waits for the user to enter credentials; not retried.
    NoCredentials,
}

impl Status {
    /// Short line under the watchlist in the dropdown; empty (hidden) while
    /// live. `source` names the provider.
    pub fn caption(self, source: &str) -> String {
        match self {
            Self::Idle | Self::Live => String::new(),
            Self::Paused => t!("status.paused").to_owned(),
            Self::Connecting => t!("status.connecting", source = source),
            Self::Retrying { retry_in_secs } => {
                t!("status.retrying", source = source, seconds = retry_in_secs)
            }
            Self::NoCredentials => t!("status.noCredentials", source = source),
        }
    }
}

/// What the feed tasks need to know: which symbols, whether to run at all,
/// and the credentials to log in with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedControl {
    pub symbols: BTreeMap<ProviderId, Vec<String>>,
    /// Bit set of reasons the machine is not being looked at (see `platform::Pause`).
    pub paused: u8,
    pub credentials: Credentials,
    /// The holdings window is open: holdings refresh often.
    pub holdings_open: bool,
    /// Manual records also need portfolio display exchange rates.
    pub manual_holdings: bool,
    /// Goes up when someone is about to look at the holdings (the dropdown
    /// opened), so old ones refresh.
    pub holdings_wanted: u32,
    /// The codes of the user's Tonghuashun watchlist the data API quotes,
    /// sorted: their quotes are wanted too, less often than the watchlist's.
    pub ths_synced: Vec<String>,
    /// Goes up when the dropdown opens with that watchlist in it, so its
    /// quotes refresh.
    pub menu_opened: u32,
}

/// The user's Tonghuashun watchlist, as last read from their account.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Synced {
    /// What the app can quote, in the account's order.
    pub instruments: Vec<Instrument>,
    /// The user's own groups, in their order; those with nothing quoted left
    /// out.
    pub groups: Vec<SyncedGroup>,
    /// Entries nothing here quotes: Hong Kong and US stocks, futures, bonds.
    pub skipped: usize,
    /// Epoch milliseconds of the last good read; none before the first.
    pub updated: Option<f64>,
    pub status: SyncStatus,
}

impl Synced {
    /// Every instrument, the watchlist's and the groups' (some in both).
    pub fn all(&self) -> impl Iterator<Item = &Instrument> {
        self.instruments.iter().chain(self.groups.iter().flat_map(|group| &group.instruments))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncedGroup {
    pub name: String,
    pub instruments: Vec<Instrument>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SyncStatus {
    #[default]
    Reading,
    Read,
    /// The login ran out: only signing in again helps.
    SignedOut,
    /// The last read failed, and why; the instruments are the read before.
    Failed(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct QuoteTime {
    pub received: f64,
    pub source: Option<f64>,
}

fn feed_symbols(
    settings: &Settings,
    manual: &crate::manual::Store,
) -> BTreeMap<ProviderId, Vec<String>> {
    let mut symbols = settings.symbols();
    for instrument in manual.instruments.values() {
        symbols.entry(instrument.provider).or_default().push(instrument.symbol.clone());
    }
    for list in symbols.values_mut() {
        list.sort();
        list.dedup();
    }
    symbols
}

pub struct Model {
    pub settings: Settings,
    pub exchange_rates: crate::currency::ExchangeRates,
    /// By instrument id.
    pub quotes: HashMap<String, Quote>,
    pub feeds: Feeds,
    /// The instrument id the chart window shows (or last showed). The window
    /// reads it on load, since a switch requested while it was still opening
    /// can't reach it as a topic change.
    pub chart: Option<String>,
    /// Holdings of each exchange with an API key.
    pub accounts: BTreeMap<ProviderId, portfolio::Account>,
    pub manual: crate::manual::Store,
    /// The user's Tonghuashun watchlist, while signed in there.
    pub synced: Option<Synced>,
}

impl Model {
    pub fn portfolio(&self) -> portfolio::Portfolio {
        let prices = crate::manual::Prices {
            quotes: &self.quotes,
            feeds: &self.feeds,
            rates: &self.exchange_rates,
        };
        portfolio::Portfolio::of(
            &self.accounts,
            self.manual.entries.iter().map(|entry| entry.valued(&prices)),
            self.manual.error.as_deref(),
        )
        .converted(self.exchange_rates.valuation(self.settings.currency))
    }

    pub fn symbols(&self) -> BTreeMap<ProviderId, Vec<String>> {
        feed_symbols(&self.settings, &self.manual)
    }

    pub fn has_holdings(&self) -> bool {
        !self.accounts.is_empty() || !self.manual.entries.is_empty()
    }

    /// An instrument used by the watchlist, a registered holding (including
    /// its quote-currency conversion), or the Tonghuashun watchlist.
    pub fn instrument(&self, id: &str) -> Option<&Instrument> {
        self.settings
            .instrument(id)
            .or_else(|| self.manual.instruments.get(id))
            .or_else(|| self.synced.as_ref()?.all().find(|instrument| instrument.id() == id))
    }

    /// Whether prices from `provider` may be old.
    pub fn stale(&self, provider: ProviderId) -> bool {
        self.feeds.get(&provider).is_some_and(|feed| feed.stale)
    }
}

pub struct Shared {
    model: Mutex<Model>,
    pub control: watch::Sender<FeedControl>,
    pub render_pending: AtomicBool,
    pub settings_path: PathBuf,
    credentials: Mutex<Credentials>,
    pub credentials_path: PathBuf,
    /// The chart window's live streams; dropping a sender stops its stream.
    pub streams: Mutex<HashMap<u32, oneshot::Sender<()>>>,
    pub next_stream: AtomicU32,
}

impl Shared {
    /// Serialize changes to instruments and publication of their subscriptions.
    /// The model lock remains held until the matching control state is visible.
    pub fn update_instruments<T, E>(
        &self,
        update: impl FnOnce(&mut Model) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut model = self.model();
        let result = update(&mut model)?;
        let kept: HashSet<_> =
            model.quotes.keys().filter(|id| model.instrument(id).is_some()).cloned().collect();
        model.quotes.retain(|id, _| kept.contains(id));
        let symbols = model.symbols();
        let manual_holdings = !model.manual.entries.is_empty();
        let mut ths_synced: Vec<_> = model
            .synced
            .iter()
            .flat_map(Synced::all)
            .map(|instrument| instrument.symbol.clone())
            .collect();
        ths_synced.sort();
        ths_synced.dedup();
        self.control.send_if_modified(|control| {
            let changed = control.symbols != symbols
                || control.manual_holdings != manual_holdings
                || control.ths_synced != ths_synced;
            control.symbols = symbols;
            control.manual_holdings = manual_holdings;
            control.ths_synced = ths_synced;
            changed
        });
        Ok(result)
    }

    pub fn new(
        settings: Settings,
        settings_path: PathBuf,
        credentials: Credentials,
        credentials_path: PathBuf,
    ) -> (Self, watch::Receiver<FeedControl>) {
        let exchange_rates = crate::currency::ExchangeRates::load(
            &settings_path.with_file_name("exchange-rates.json"),
        );
        let manual =
            crate::manual::Store::load(&settings_path.with_file_name("manual-holdings.json"));
        let (control, rx) = watch::channel(FeedControl {
            symbols: feed_symbols(&settings, &manual),
            credentials: credentials.clone(),
            manual_holdings: !manual.entries.is_empty(),
            ..FeedControl::default()
        });
        let shared = Self {
            model: Mutex::new(Model {
                settings,
                exchange_rates,
                quotes: HashMap::new(),
                feeds: Feeds::new(),
                chart: None,
                accounts: BTreeMap::new(),
                manual,
                synced: None,
            }),
            control,
            render_pending: AtomicBool::new(false),
            settings_path,
            credentials: Mutex::new(credentials),
            credentials_path,
            streams: Mutex::new(HashMap::new()),
            next_stream: AtomicU32::new(1),
        };
        (shared, rx)
    }

    pub fn credentials(&self) -> MutexGuard<'_, Credentials> {
        self.credentials.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn model(&self) -> MutexGuard<'_, Model> {
        self.model.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // contract: UI and HTTP writers publish the subscriptions belonging to
    // their state before the next instrument mutation can proceed.
    #[test]
    fn instrument_updates_publish_before_the_next_writer() {
        use std::{
            sync::{Arc, mpsc},
            time::Duration,
        };
        let dir =
            tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
        let (shared, _receiver) = Shared::new(
            Settings::default(),
            dir.path().join("settings.json"),
            Default::default(),
            dir.path().join("credentials.json"),
        );
        let shared = Arc::new(shared);
        // A slow subscription consumer holds publication while two real
        // writers update different entries through the production boundary.
        let pending_publication = shared.control.borrow();
        let (first_mutated, mutated) = mpsc::channel();
        let first = shared.clone();
        let first = std::thread::spawn(move || {
            first
                .update_instruments(|model| -> Result<(), ()> {
                    model
                        .settings
                        .watchlist
                        .push(ProviderId::Eastmoney.provider().bare("118.AU9999").unwrap());
                    first_mutated.send(()).unwrap();
                    Ok(())
                })
                .unwrap()
        });
        mutated.recv_timeout(Duration::from_secs(2)).unwrap();
        let (started, running) = mpsc::channel();
        let (second_mutated, entered) = mpsc::channel();
        let second = shared.clone();
        let second = std::thread::spawn(move || {
            started.send(()).unwrap();
            second
                .update_instruments(|model| -> Result<(), ()> {
                    second_mutated.send(()).unwrap();
                    model
                        .settings
                        .watchlist
                        .push(ProviderId::Eastmoney.provider().bare("150.000216").unwrap());
                    Ok(())
                })
                .unwrap();
        });
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        let premature = entered.recv_timeout(Duration::from_millis(100));
        drop(pending_publication);
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(
            premature,
            Err(mpsc::RecvTimeoutError::Timeout),
            "a newer state must not overtake an unpublished mutation"
        );
        assert_eq!(
            shared.control.borrow().symbols[&ProviderId::Eastmoney],
            ["118.AU9999", "150.000216"]
        );
    }

    #[test]
    fn another_exchange_takes_the_pairs_off_the_watchlist() {
        let mut settings = Settings::default();
        settings.watchlist.push(Instrument {
            provider: ProviderId::Longbridge,
            symbol: "AAPL.US".into(),
            base: "AAPL".into(),
            quote: "USD".into(),
            name: Some("苹果".into()),
            decimals: None,
            pinned: false,
            venue: None,
        });
        settings.watchlist.push(Instrument {
            provider: ProviderId::DexScreener,
            symbol: "solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx".into(),
            base: "$WIF".into(),
            quote: "USD".into(),
            name: Some("dogwifhat".into()),
            decimals: None,
            pinned: false,
            venue: Some(Venue { chain: "Solana".into(), dex: Some("Raydium".into()) }),
        });
        settings.exchange = ProviderId::Okx;
        let settings = settings.validated().unwrap();
        let symbols: Vec<&str> = settings.watchlist.iter().map(|i| i.symbol.as_str()).collect();
        assert_eq!(symbols, ["AAPL.US", "solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx"]);
        assert!(settings.pinned().is_none());
        let pool = &settings.watchlist[1];
        assert_eq!(pool.short_label(), "$WIF");
        assert_eq!(pool.pair_label(), "$WIF · Solana");
        assert_eq!(pool.row_label(), ("$WIF".to_owned(), " Solana".to_owned()));

        let mut stocks = settings.clone();
        stocks.exchange = ProviderId::Longbridge;
        assert!(stocks.validated().is_err());
    }

    // The bar shows one thing: the total holdings take the pinned entry's place.
    #[test]
    fn holdings_in_the_bar_unpin_the_entry() {
        let settings =
            Settings { holdings_in_bar: true, ..Default::default() }.validated().unwrap();
        assert!(settings.pinned().is_none());
    }

    #[test]
    fn at_most_one_entry_is_pinned() {
        let mut settings = Settings::default();
        settings.watchlist[1].pinned = true;
        assert!(settings.clone().validated().is_err());
        settings.watchlist[0].pinned = false;
        let settings = settings.validated().unwrap();
        assert_eq!(settings.pinned().map(|i| i.base.as_str()), Some("ETH"));
    }
}
