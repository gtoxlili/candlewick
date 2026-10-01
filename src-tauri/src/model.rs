//! Settings, live quotes and feed status, shared by the feed tasks, the bar
//! renderer and the IPC commands.

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
    market::{self, Feeds, ProviderId},
    net::Route,
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
}

impl Instrument {
    fn preset(base: &str, pinned: bool) -> Self {
        Self {
            provider: ProviderId::Binance,
            symbol: format!("{base}USDT"),
            base: base.to_owned(),
            quote: "USDT".to_owned(),
            name: None,
            decimals: Some(2),
            pinned,
        }
    }

    /// `binance:BTCUSDT`: how windows and menus refer to it.
    pub fn id(&self) -> String {
        market::instrument_id(self.provider, &self.symbol)
    }

    /// The bar's name for it: `BTC` for USD-like quotes, `ETH/BTC`
    /// otherwise; US tickers as they are, other stocks by name.
    pub fn short_label(&self) -> String {
        if self.provider.is_exchange() {
            return if is_usd_like(&self.quote) { self.base.clone() } else { self.pair_label() };
        }
        match &self.name {
            Some(name) if !self.symbol.ends_with(".US") => name.clone(),
            _ => self.base.clone(),
        }
    }

    /// `BTC/USDT`, or a stock's name and code: `苹果 AAPL`, `腾讯控股 700`.
    pub fn pair_label(&self) -> String {
        if self.provider.is_exchange() {
            return format!("{}/{}", self.base, self.quote);
        }
        match &self.name {
            Some(name) => format!("{name} {}", self.base),
            None => self.base.clone(),
        }
    }

    /// The dropdown row's name and the dimmed part after it: `BTC` `/USDT`,
    /// `AAPL` ` 苹果`, `腾讯控股` ` 700`.
    pub fn row_label(&self) -> (String, String) {
        if self.provider.is_exchange() {
            return (self.base.clone(), format!("/{}", self.quote));
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
    /// AI agents may read the app's data (`agent`).
    pub agent_access: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
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
        }
    }
}

impl Settings {
    /// Normalizes and checks settings coming from the webview. Pairs of
    /// another exchange than the one picked are dropped: picking another one
    /// takes them off the watchlist.
    pub fn validated(mut self) -> Result<Self, String> {
        if !self.exchange.is_exchange() {
            return Err(format!("不支持的交易所：{}", self.exchange.name()));
        }
        let exchange = self.exchange;
        self.watchlist.retain(|instrument| {
            !instrument.provider.is_exchange() || instrument.provider == exchange
        });
        if self.watchlist.len() > MAX_INSTRUMENTS {
            return Err(format!("最多添加 {MAX_INSTRUMENTS} 个"));
        }
        let mut seen = HashSet::new();
        for instrument in &mut self.watchlist {
            instrument.provider.provider().validate(instrument)?;
            if instrument.decimals.is_some_and(|d| d > MAX_DECIMALS) {
                instrument.decimals = None;
            }
            if !seen.insert(instrument.id()) {
                return Err(format!("重复添加：{}", instrument.symbol));
            }
        }
        if self.watchlist.iter().filter(|instrument| instrument.pinned).count() > 1 {
            return Err("菜单栏只能显示一个".to_owned());
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quote {
    pub last: f64,
    /// What the change counts from: the price 24 hours ago, or the last close.
    pub open: f64,
    /// Set outside the regular session (US stocks).
    pub session: Option<Session>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Pre,
    Post,
    Overnight,
}

impl Session {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pre => "盘前",
            Self::Post => "盘后",
            Self::Overnight => "夜盘",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Status {
    /// Nothing to stream.
    Idle,
    Paused,
    #[default]
    Connecting,
    Live(Route),
    Retrying {
        reason: String,
        retry_in_secs: u64,
    },
    /// Waits for the user, e.g. to enter credentials; not retried.
    Unavailable(String),
}

impl Status {
    /// Short line under the watchlist in the dropdown; empty (hidden) while
    /// live. `source` names the provider.
    pub fn caption(&self, source: &str) -> String {
        match self {
            Self::Idle | Self::Live(_) => String::new(),
            Self::Paused => "已暂停".to_owned(),
            Self::Connecting => format!("正在连接{source}…"),
            Self::Retrying { retry_in_secs, .. } => {
                format!("{source}连接失败 · {retry_in_secs} 秒后重试")
            }
            Self::Unavailable(reason) => reason.clone(),
        }
    }

    /// Full description for the settings window, including the route.
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "还没有自选".to_owned(),
            Self::Paused => "已暂停（屏幕休眠）".to_owned(),
            Self::Connecting => "连接中…".to_owned(),
            Self::Live(route) => format!("实时 · {route}"),
            Self::Retrying { reason, retry_in_secs } => {
                let reason: String = reason.chars().take(48).collect();
                format!("{retry_in_secs} 秒后重连 · {reason}")
            }
            Self::Unavailable(reason) => reason.clone(),
        }
    }

    pub fn tone(&self) -> &'static str {
        match self {
            Self::Live(_) => "live",
            Self::Retrying { .. } | Self::Unavailable(_) => "error",
            Self::Connecting => "busy",
            Self::Idle | Self::Paused => "idle",
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
    /// Goes up when someone is about to look at the holdings (the dropdown
    /// opened), so old ones refresh.
    pub holdings_wanted: u32,
}

pub struct Model {
    pub settings: Settings,
    /// By instrument id.
    pub quotes: HashMap<String, Quote>,
    pub feeds: Feeds,
    /// The instrument id the chart window shows (or last showed). The page
    /// reads it on load, since a switch requested while it was still loading
    /// can't reach it as an event.
    pub chart: Option<String>,
    /// Holdings of each exchange with an API key.
    pub accounts: BTreeMap<ProviderId, portfolio::Account>,
}

impl Model {
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
    pub fn new(
        settings: Settings,
        settings_path: PathBuf,
        credentials: Credentials,
        credentials_path: PathBuf,
    ) -> (Self, watch::Receiver<FeedControl>) {
        let (control, rx) = watch::channel(FeedControl {
            symbols: settings.symbols(),
            credentials: credentials.clone(),
            ..FeedControl::default()
        });
        let shared = Self {
            model: Mutex::new(Model {
                settings,
                quotes: HashMap::new(),
                feeds: Feeds::new(),
                chart: None,
                accounts: BTreeMap::new(),
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
        // A panic while holding the lock aborts the process (panic = "abort"),
        // so poisoning cannot be observed in release builds.
        self.model.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        });
        settings.exchange = ProviderId::Okx;
        let settings = settings.validated().unwrap();
        let symbols: Vec<&str> = settings.watchlist.iter().map(|i| i.symbol.as_str()).collect();
        assert_eq!(symbols, ["AAPL.US"]);
        assert!(settings.pinned().is_none());

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
