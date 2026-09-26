//! Settings, live quotes and feed status, shared by the feed task, the tray
//! renderer and the IPC commands.

use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::net::Route;

pub const MAX_COINS: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coin {
    /// Binance spot symbol, always `base + quote`, e.g. `BTCUSDT`.
    pub symbol: String,
    pub base: String,
    pub quote: String,
    /// Decimals of the pair's tick size. `None` falls back to a magnitude rule.
    #[serde(default)]
    pub decimals: Option<u8>,
    /// Shown in the menu bar, not only in the dropdown. At most one pair is.
    #[serde(default)]
    pub pinned: bool,
}

impl Coin {
    fn preset(base: &str, pinned: bool) -> Self {
        Self {
            symbol: format!("{base}USDT"),
            base: base.to_owned(),
            quote: "USDT".to_owned(),
            decimals: Some(2),
            pinned,
        }
    }

    /// `BTC` for USD-like quotes, `ETH/BTC` otherwise.
    pub fn short_label(&self) -> String {
        if is_usd_like(&self.quote) { self.base.clone() } else { self.pair_label() }
    }

    pub fn pair_label(&self) -> String {
        format!("{}/{}", self.base, self.quote)
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub coins: Vec<Coin>,
    pub show_symbol: bool,
    pub show_change: bool,
    pub color_scheme: ColorScheme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            coins: vec![
                Coin::preset("BTC", true),
                Coin::preset("ETH", false),
                Coin::preset("SOL", false),
            ],
            show_symbol: true,
            show_change: false,
            color_scheme: ColorScheme::GreenUp,
        }
    }
}

impl Settings {
    /// Normalizes and checks settings coming from the webview.
    pub fn validated(mut self) -> Result<Self, String> {
        if self.coins.len() > MAX_COINS {
            return Err(format!("最多添加 {MAX_COINS} 个币种"));
        }
        let mut seen = HashSet::new();
        for coin in &mut self.coins {
            coin.symbol = coin.symbol.trim().to_uppercase();
            coin.base = coin.base.trim().to_uppercase();
            coin.quote = coin.quote.trim().to_uppercase();
            let valid = |s: &str, max: usize| {
                !s.is_empty() && s.chars().count() <= max && s.chars().all(char::is_alphanumeric)
            };
            if !valid(&coin.symbol, 24) || !valid(&coin.base, 16) || !valid(&coin.quote, 12) {
                return Err(format!("无效的交易对：{}", coin.symbol));
            }
            if coin.symbol != format!("{}{}", coin.base, coin.quote) {
                return Err(format!("交易对与币种不匹配：{}", coin.symbol));
            }
            if coin.decimals.is_some_and(|d| d > 12) {
                coin.decimals = None;
            }
            if !seen.insert(coin.symbol.clone()) {
                return Err(format!("重复的交易对：{}", coin.symbol));
            }
        }
        // The menu bar shows one pair; files from when it showed several keep the first.
        let mut found = false;
        for coin in &mut self.coins {
            if coin.pinned {
                coin.pinned = !found;
                found = true;
            }
        }
        Ok(self)
    }

    /// The pair shown in the menu bar, if any.
    pub fn pinned(&self) -> Option<&Coin> {
        self.coins.iter().find(|coin| coin.pinned)
    }

    /// Stream subscriptions: the symbol set only, order and flags don't matter.
    pub fn symbols(&self) -> Vec<String> {
        let mut symbols: Vec<String> = self.coins.iter().map(|c| c.symbol.clone()).collect();
        symbols.sort();
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

#[derive(Debug, Clone, Copy)]
pub struct Quote {
    pub last: f64,
    pub open: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NoCoins,
    Paused,
    Connecting,
    Live(Route),
    Retrying { reason: String, retry_in_secs: u64 },
}

impl Status {
    /// Short line under the coins in the dropdown; empty (hidden) while live.
    pub fn caption(&self) -> String {
        match self {
            Self::NoCoins | Self::Live(_) => String::new(),
            Self::Paused => "已暂停".to_owned(),
            Self::Connecting => "正在连接币安…".to_owned(),
            Self::Retrying { retry_in_secs, .. } => {
                format!("连接失败 · {retry_in_secs} 秒后重试")
            }
        }
    }

    /// Full description for the settings window, including the route.
    pub fn label(&self) -> String {
        match self {
            Self::NoCoins => "未添加币种".to_owned(),
            Self::Paused => "已暂停（屏幕休眠）".to_owned(),
            Self::Connecting => "连接中…".to_owned(),
            Self::Live(route) => format!("实时 · {route}"),
            Self::Retrying { reason, retry_in_secs } => {
                let reason: String = reason.chars().take(48).collect();
                format!("{retry_in_secs} 秒后重连 · {reason}")
            }
        }
    }

    pub fn tone(&self) -> &'static str {
        match self {
            Self::Live(_) => "live",
            Self::Retrying { .. } => "error",
            Self::Connecting => "busy",
            Self::NoCoins | Self::Paused => "idle",
        }
    }
}

/// What the stream task needs to know: which symbols, and whether to run at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedControl {
    pub symbols: Vec<String>,
    /// Bit set of reasons the machine is not being looked at (see `macos::Pause`).
    pub paused: u8,
}

pub struct Model {
    pub settings: Settings,
    pub quotes: HashMap<String, Quote>,
    pub status: Status,
    /// Prices on screen may be old: set when a connection fails, cleared on
    /// the next successful connection.
    pub stale: bool,
    /// The pair the chart window shows (or last showed). The page reads it on
    /// load, since a switch requested while it was still loading can't reach
    /// it as an event.
    pub chart_symbol: Option<String>,
}

pub struct Shared {
    model: Mutex<Model>,
    pub control: watch::Sender<FeedControl>,
    pub render_pending: AtomicBool,
    pub settings_path: PathBuf,
}

impl Shared {
    pub fn new(settings: Settings, settings_path: PathBuf) -> (Self, watch::Receiver<FeedControl>) {
        let (control, rx) = watch::channel(FeedControl { symbols: settings.symbols(), paused: 0 });
        let shared = Self {
            model: Mutex::new(Model {
                settings,
                quotes: HashMap::new(),
                status: Status::Connecting,
                stale: false,
                chart_symbol: None,
            }),
            control,
            render_pending: AtomicBool::new(false),
            settings_path,
        };
        (shared, rx)
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
    fn keeps_only_the_first_pinned_pair() {
        let mut settings = Settings::default();
        for coin in &mut settings.coins {
            coin.pinned = coin.base != "BTC";
        }
        let settings = settings.validated().unwrap();
        let pinned: Vec<&str> =
            settings.coins.iter().filter(|c| c.pinned).map(|c| c.base.as_str()).collect();
        assert_eq!(pinned, ["ETH"]);
        assert_eq!(settings.pinned().map(|c| c.base.as_str()), Some("ETH"));
    }
}
