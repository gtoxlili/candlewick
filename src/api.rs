//! Operations for the settings and chart windows.

use std::sync::atomic::Ordering;

use crate::app::{App, Topic};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::{
    agent::{self, AgentStatus},
    bar,
    credentials::{self, ApiKey, CoinGeckoKey, LongbridgeKeys, ThsKey},
    i18n::{self, Locale},
    market::{
        self, Candle, ChartSpec, LiveEvent, ProviderId, Search, Trade, dexscreener, longbridge, ths,
    },
    model::{self, Instrument, Settings, Shared},
    platform,
    portfolio::Portfolio,
    update::{self, UpdateView},
    window,
};

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    Invalid(String),
    #[error("{}", save_failed(.0))]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    LoginItem(String),
    #[error(transparent)]
    Market(#[from] market::Error),
    #[error("{0}")]
    Window(String),
}

fn save_failed(error: &std::io::Error) -> String {
    t!("error.saveFailed", error = error)
}

type CmdResult<T> = Result<T, CommandError>;

pub fn get_settings(app: &App) -> Settings {
    let shared = app.shared();
    shared.model().settings.clone()
}

/// Validates, persists and applies settings; returns them normalized.
pub fn save_settings(app: &App, settings: Settings) -> CmdResult<Settings> {
    let shared = app.shared();
    let settings = settings.validated().map_err(CommandError::Invalid)?;
    let (currency_changed, currency_available) = {
        let model = shared.model();
        (
            model.settings.currency != settings.currency,
            model.exchange_rates.valuation(settings.currency).currency == settings.currency,
        )
    };
    if currency_changed && !currency_available && !shared.credentials().exchanges.is_empty() {
        return Err(CommandError::Invalid(t!("settings.display.currencyUnavailable").to_owned()));
    }
    model::save(&shared.settings_path, &settings)?;
    let before = {
        let mut model = shared.model();
        let before = std::mem::replace(&mut model.settings, settings.clone());
        let kept: Vec<String> =
            model.quotes.keys().filter(|id| model.instrument(id).is_some()).cloned().collect();
        model.quotes.retain(|id, _| kept.contains(id));
        before
    };
    if settings.agent_access != before.agent_access {
        agent::sync(app);
    }
    if settings.language != before.language {
        platform::remember_language(settings.language);
        i18n::set(settings.language.locale());
        window::relabel(app);
    }
    let symbols = settings.symbols();
    shared.control.send_if_modified(|control| {
        let changed = control.symbols != symbols;
        control.symbols = symbols;
        changed
    });
    bar::request_render(app);
    app.notify(Topic::Settings);
    if settings.currency != before.currency {
        app.notify(Topic::Portfolio);
    }
    window::sync_chart(app);
    Ok(settings)
}

/// The language the app speaks now, for a window as it opens; the `Locale`
/// topic tells it of changes (`window::relabel`).
pub fn get_locale() -> Locale {
    i18n::current()
}

/// Whether agents have the skill, as the settings window shows it.
pub fn get_agent(app: &App) -> AgentStatus {
    app.agent().status()
}

pub fn get_login_item() -> bool {
    platform::login_item_enabled()
}

pub fn set_login_item(enabled: bool) -> CmdResult<bool> {
    platform::set_login_item(enabled).map_err(CommandError::LoginItem)?;
    Ok(platform::login_item_enabled())
}

pub fn get_update(app: &App) -> UpdateView {
    update::view(app)
}

/// Starts a check; the result arrives as `update` events.
pub fn check_update(app: &App) {
    update::check_now(app);
}

pub fn restart_to_update(app: &App) {
    update::restart_now(app);
}

/// Instruments matching the query: pairs on the exchange picked, stocks,
/// and pools.
pub async fn search_instruments(app: &App, query: String) -> Search {
    let exchange = app.shared().model().settings.exchange;
    market::search(exchange, &query).await
}

/// The Longbridge credentials as the settings window shows them.
pub struct LongbridgeView {
    /// The app key's ends, enough to recognize it; the secrets never leave.
    pub app_key: Option<String>,
    pub account: Option<longbridge::Account>,
}

pub fn get_longbridge(app: &App) -> LongbridgeView {
    let shared = app.shared();
    let keys = shared.credentials().longbridge.clone();
    let account = keys.as_ref().and_then(|_| longbridge::last_account());
    LongbridgeView { app_key: keys.map(|keys| masked(&keys.app_key)), account }
}

/// Saves (or with `None`, removes) the Longbridge credentials.
pub fn set_longbridge(app: &App, keys: Option<LongbridgeKeys>) -> CmdResult<LongbridgeView> {
    let shared = app.shared();
    let keys = keys
        .map(|keys| {
            let clean = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let keys = LongbridgeKeys {
                app_key: clean(&keys.app_key),
                app_secret: clean(&keys.app_secret),
                access_token: clean(&keys.access_token),
            };
            let complete = !keys.app_key.is_empty()
                && !keys.app_secret.is_empty()
                && !keys.access_token.is_empty();
            if complete {
                Ok(keys)
            } else {
                Err(CommandError::Invalid(t!("error.longbridgeIncomplete").to_owned()))
            }
        })
        .transpose()?;
    let credentials = {
        let mut stored = shared.credentials();
        stored.longbridge = keys;
        credentials::save(&shared.credentials_path, &stored)?;
        stored.clone()
    };
    longbridge::forget_account();
    shared.control.send_modify(|control| control.credentials = credentials);
    bar::request_render(app);
    Ok(get_longbridge(app))
}

/// Logs in with the saved credentials (unless already logged in) and
/// returns what the account may see.
pub async fn check_longbridge() -> CmdResult<longbridge::Account> {
    Ok(longbridge::account().await?)
}

/// Tonghuashun as the settings window shows it: the key, the account whose
/// watchlist the dropdown shows, and a QR sign-in under way.
pub struct ThsView {
    /// The key's ends, enough to recognize it; the key never leaves.
    pub key: Option<String>,
    pub account: Option<ths::AccountView>,
    pub sign_in: Option<ths::SignIn>,
}

pub fn get_ths(app: &App) -> ThsView {
    let shared = app.shared();
    let credentials = shared.credentials().clone();
    let model = shared.model();
    ThsView {
        key: credentials.ths.as_ref().map(|k| masked(&k.key)),
        account: credentials.ths_signed_in.then(|| ths::account_view(model.synced.as_ref())),
        sign_in: ths::sign_in_view(),
    }
}

/// Starts a QR sign-in; its code and how it ends arrive as `ths` events.
pub fn ths_sign_in(app: &App) {
    ths::sign_in(app);
}

pub fn ths_sign_in_cancel(app: &App) {
    ths::cancel_sign_in();
    app.notify(Topic::Ths);
}

pub fn ths_sign_out(app: &App) -> ThsView {
    ths::sign_out(app);
    get_ths(app)
}

/// Saves (or with `None`, removes) the Tonghuashun key, once Tonghuashun
/// takes it.
pub async fn set_ths(app: &App, key: Option<String>) -> CmdResult<ThsView> {
    let key = key
        .map(|key| key.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|key| !key.is_empty());
    if let Some(key) = &key {
        ths::check(key).await?;
    }
    let shared = app.shared();
    {
        let mut stored = shared.credentials();
        stored.ths = key.clone().map(|key| ThsKey { key });
        credentials::save(&shared.credentials_path, &stored)?;
        let credentials = stored.clone();
        shared.control.send_modify(|control| control.credentials = credentials);
    }
    ths::configure(key.clone());
    bar::request_render(app);
    Ok(get_ths(app))
}

/// The CoinGecko key as the settings window shows it.
pub struct CoinGeckoView {
    /// The key's ends, enough to recognize it; the key never leaves.
    pub key: Option<String>,
    /// What the key is, once probed.
    pub access: Option<dexscreener::Access>,
}

pub fn get_coingecko(app: &App) -> CoinGeckoView {
    let shared = app.shared();
    let key = shared.credentials().coingecko.clone();
    CoinGeckoView {
        access: key.as_ref().and_then(|_| dexscreener::coingecko_access()),
        key: key.map(|k| masked(&k.key)),
    }
}

/// Saves (or with `None`, removes) the CoinGecko key, once CoinGecko says
/// what it is.
pub async fn set_coingecko(app: &App, key: Option<String>) -> CmdResult<CoinGeckoView> {
    let key = key
        .map(|key| key.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|key| !key.is_empty());
    let access = match &key {
        Some(key) => Some(dexscreener::coingecko_check(key).await?),
        None => None,
    };
    let shared = app.shared();
    {
        let mut stored = shared.credentials();
        stored.coingecko = key.clone().map(|key| CoinGeckoKey { key });
        credentials::save(&shared.credentials_path, &stored)?;
        let credentials = stored.clone();
        shared.control.send_modify(|control| control.credentials = credentials);
    }
    dexscreener::configure_coingecko(key.clone());
    Ok(CoinGeckoView { key: key.map(|k| masked(&k)), access })
}

/// Asks CoinGecko again what the saved key is (its plan's usage moves).
pub async fn check_coingecko(app: &App) -> CmdResult<dexscreener::Access> {
    let shared = app.shared();
    let key = shared.credentials().coingecko.clone();
    let key =
        key.ok_or_else(|| CommandError::Invalid(t!("error.coingeckoKeyMissing").to_owned()))?;
    Ok(dexscreener::coingecko_check(&key.key).await?)
}

/// An exchange's API key as the settings window shows it.
pub struct ExchangeKey {
    pub exchange: ProviderId,
    /// The key's ends, enough to recognize it, when one is saved; the secrets
    /// never leave.
    pub key: Option<String>,
}

pub fn get_exchange_keys(app: &App) -> Vec<ExchangeKey> {
    let shared = app.shared();
    let credentials = shared.credentials();
    ProviderId::EXCHANGES
        .map(|exchange| ExchangeKey {
            exchange,
            key: credentials.exchanges.get(&exchange).map(|key| masked(&key.key)),
        })
        .into()
}

/// Saves (or with `None`, removes) an exchange's API key, once the exchange
/// confirms it works.
pub async fn set_exchange_key(
    app: &App,
    exchange: ProviderId,
    key: Option<ApiKey>,
) -> CmdResult<Vec<ExchangeKey>> {
    if !exchange.is_exchange() {
        return Err(CommandError::Invalid(t!("error.noApiKey", source = exchange.name())));
    }
    let key = key
        .map(|key| {
            let clean = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let passphrase = key.passphrase.as_deref().map(str::trim).unwrap_or_default();
            let needs_passphrase = exchange == ProviderId::Okx;
            let key = ApiKey {
                key: clean(&key.key),
                secret: clean(&key.secret),
                passphrase: needs_passphrase.then(|| passphrase.to_owned()),
            };
            if key.key.is_empty()
                || key.secret.is_empty()
                || (needs_passphrase && passphrase.is_empty())
            {
                let incomplete = if needs_passphrase {
                    t!("error.keyIncompletePassphrase")
                } else {
                    t!("error.keyIncomplete")
                };
                return Err(CommandError::Invalid(incomplete.to_owned()));
            }
            Ok(key)
        })
        .transpose()?;
    if let Some(key) = &key {
        market::check_key(exchange, key).await?;
    }
    let shared = app.shared();
    let credentials = {
        let mut stored = shared.credentials();
        match key {
            Some(key) => stored.exchanges.insert(exchange, key),
            None => stored.exchanges.remove(&exchange),
        };
        credentials::save(&shared.credentials_path, &stored)?;
        stored.clone()
    };
    shared.control.send_modify(|control| control.credentials = credentials);
    Ok(get_exchange_keys(app))
}

pub fn get_portfolio(app: &App) -> Portfolio {
    let shared = app.shared();
    shared.model().portfolio()
}

pub fn open_holdings(app: &App) -> CmdResult<()> {
    window::open_holdings(app).map_err(|e| CommandError::Window(e.to_string()))?;
    Ok(())
}

/// `hk_abc1…wxyz`
fn masked(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 10 {
        return "…".to_owned();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// The instrument id the chart window should show.
pub fn get_chart_instrument(app: &App) -> Option<String> {
    let shared = app.shared();
    shared.model().chart.clone()
}

/// Shows the instrument `id` in the chart window (switching it if already open).
pub fn open_chart(app: &App, id: String) -> CmdResult<()> {
    window::open_chart(app, &id).map_err(|e| CommandError::Window(e.to_string()))?;
    Ok(())
}

/// The instrument `id` names, for a chart of one outside the watchlist (the
/// synced Tonghuashun watchlist's).
pub fn chart_instrument(app: &App, id: String) -> CmdResult<Instrument> {
    let shared = app.shared();
    instrument(shared, &id)
}

pub fn chart_spec(app: &App, id: String) -> CmdResult<ChartSpec> {
    let shared = app.shared();
    let instrument = instrument(shared, &id)?;
    Ok(instrument.provider.provider().chart_spec(&instrument))
}

pub async fn chart_history(
    app: &App,
    id: String,
    interval: u32,
    end: Option<f64>,
    limit: usize,
) -> CmdResult<Vec<Candle>> {
    let shared = app.shared();
    let instrument = instrument(shared, &id)?;
    Ok(instrument.provider.provider().history(&instrument, interval, end, limit).await?)
}

pub async fn chart_trades(app: &App, id: String, limit: usize) -> CmdResult<Vec<Trade>> {
    let shared = app.shared();
    let instrument = instrument(shared, &id)?;
    Ok(instrument.provider.provider().recent_trades(&instrument, limit).await?)
}

/// Starts streaming `id` into `events`; returns a handle for `chart_stream_stop`.
pub fn chart_stream(app: &App, id: String, events: UnboundedSender<LiveEvent>) -> CmdResult<u32> {
    let shared = app.shared();
    let instrument = instrument(shared, &id)?;
    let (stop, stopped) = oneshot::channel();
    let handle = shared.next_stream.fetch_add(1, Ordering::Relaxed);
    shared.streams.lock().unwrap_or_else(|e| e.into_inner()).insert(handle, stop);
    let stream = instrument.provider.provider().stream(instrument, stopped, events);
    let app = app.clone();
    let task_app = app.clone();
    app.spawn(async move {
        stream.await;
        // A stream also ends on its own once its window is gone.
        task_app.shared().streams.lock().unwrap_or_else(|e| e.into_inner()).remove(&handle);
    });
    Ok(handle)
}

pub fn chart_stream_stop(app: &App, handle: u32) {
    let shared = app.shared();
    shared.streams.lock().unwrap_or_else(|e| e.into_inner()).remove(&handle);
}

/// Opens the instrument's page on its provider's website in the browser.
pub fn open_link(app: &App, id: String) -> CmdResult<()> {
    let shared = app.shared();
    let instrument = instrument(shared, &id)?;
    if let Some(link) = instrument.provider.provider().chart_spec(&instrument).link {
        platform::open_url(&link);
    }
    Ok(())
}

fn instrument(shared: &Shared, id: &str) -> CmdResult<Instrument> {
    let model = shared.model();
    let found = model.instrument(id).cloned();
    found.ok_or_else(|| CommandError::Invalid(t!("error.notInWatchlist", id = id)))
}
