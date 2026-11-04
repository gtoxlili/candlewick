//! IPC surface for the settings and chart windows.

use std::sync::atomic::Ordering;

use serde::{Serialize, Serializer};
use tauri::{AppHandle, Manager, State, WebviewWindow, ipc::Channel};
use tokio::sync::oneshot;

use crate::{
    agent::{self, Agent, AgentStatus},
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
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
}

fn save_failed(error: &std::io::Error) -> String {
    t!("error.saveFailed", error = error)
}

// The webview only needs a readable message.
impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

type CmdResult<T> = Result<T, CommandError>;

#[tauri::command]
pub fn get_settings(shared: State<'_, Shared>) -> Settings {
    shared.model().settings.clone()
}

/// Validates, persists and applies settings; returns them normalized.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    shared: State<'_, Shared>,
    settings: Settings,
) -> CmdResult<Settings> {
    let settings = settings.validated().map_err(CommandError::Invalid)?;
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
        agent::sync(&app);
    }
    if settings.language != before.language {
        platform::remember_language(settings.language);
        i18n::set(settings.language.locale());
        window::relabel(&app);
    }
    let symbols = settings.symbols();
    shared.control.send_if_modified(|control| {
        let changed = control.symbols != symbols;
        control.symbols = symbols;
        changed
    });
    bar::request_render(&app);
    window::emit_settings(&app, &settings);
    window::sync_chart(&app);
    Ok(settings)
}

/// The language the app speaks now, for a page as it loads; `locale` events
/// tell it of changes (`window::relabel`).
#[tauri::command]
pub fn get_locale() -> Locale {
    i18n::current()
}

/// Whether agents have the skill, as the settings window shows it.
#[tauri::command]
pub fn get_agent(agent: State<'_, Agent>) -> AgentStatus {
    agent.status()
}

#[tauri::command]
pub fn get_login_item() -> bool {
    platform::login_item_enabled()
}

#[tauri::command]
pub fn set_login_item(enabled: bool) -> CmdResult<bool> {
    platform::set_login_item(enabled).map_err(CommandError::LoginItem)?;
    Ok(platform::login_item_enabled())
}

#[tauri::command]
pub fn get_update(app: AppHandle) -> UpdateView {
    update::view(&app)
}

/// Starts a check; the result arrives as `update` events.
#[tauri::command]
pub fn check_update(app: AppHandle) {
    update::check_now(&app);
}

#[tauri::command]
pub fn restart_to_update(app: AppHandle) {
    update::restart_now(&app);
}

/// The page has content; now its window can appear without a blank flash.
#[tauri::command]
pub fn window_ready(window: WebviewWindow) -> CmdResult<()> {
    window.show()?;
    platform::window::bring_to_front(&window);
    Ok(())
}

/// Instruments matching the query: pairs on the exchange picked, stocks,
/// and pools.
#[tauri::command]
pub async fn search_instruments(app: AppHandle, query: String) -> Search {
    let exchange = app.state::<Shared>().model().settings.exchange;
    market::search(exchange, &query).await
}

/// The Longbridge credentials as the settings window shows them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LongbridgeView {
    /// The app key's ends, enough to recognize it; the secrets never leave.
    app_key: Option<String>,
    account: Option<longbridge::Account>,
}

#[tauri::command]
pub fn get_longbridge(shared: State<'_, Shared>) -> LongbridgeView {
    let keys = shared.credentials().longbridge.clone();
    let account = keys.as_ref().and_then(|_| longbridge::last_account());
    LongbridgeView { app_key: keys.map(|keys| masked(&keys.app_key)), account }
}

/// Saves (or with `None`, removes) the Longbridge credentials.
#[tauri::command]
pub fn set_longbridge(
    app: AppHandle,
    shared: State<'_, Shared>,
    keys: Option<LongbridgeKeys>,
) -> CmdResult<LongbridgeView> {
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
    bar::request_render(&app);
    Ok(get_longbridge(shared))
}

/// Logs in with the saved credentials (unless already logged in) and
/// returns what the account may see.
#[tauri::command]
pub async fn check_longbridge() -> CmdResult<longbridge::Account> {
    Ok(longbridge::account().await?)
}

/// Tonghuashun as the settings window shows it: the key, the account whose
/// watchlist the dropdown shows, and a QR sign-in under way.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThsView {
    /// The key's ends, enough to recognize it; the key never leaves.
    key: Option<String>,
    account: Option<ths::AccountView>,
    sign_in: Option<ths::SignIn>,
}

#[tauri::command]
pub fn get_ths(shared: State<'_, Shared>) -> ThsView {
    let credentials = shared.credentials().clone();
    let model = shared.model();
    ThsView {
        key: credentials.ths.as_ref().map(|k| masked(&k.key)),
        account: credentials.ths_signed_in.then(|| ths::account_view(model.synced.as_ref())),
        sign_in: ths::sign_in_view(),
    }
}

/// Starts a QR sign-in; its code and how it ends arrive as `ths` events.
#[tauri::command]
pub fn ths_sign_in(app: AppHandle) {
    ths::sign_in(&app);
}

#[tauri::command]
pub fn ths_sign_in_cancel(app: AppHandle) {
    ths::cancel_sign_in();
    window::emit_ths(&app);
}

#[tauri::command]
pub fn ths_sign_out(app: AppHandle) -> ThsView {
    ths::sign_out(&app);
    get_ths(app.state::<Shared>())
}

/// Saves (or with `None`, removes) the Tonghuashun key, once Tonghuashun
/// takes it.
#[tauri::command]
pub async fn set_ths(app: AppHandle, key: Option<String>) -> CmdResult<ThsView> {
    let key = key
        .map(|key| key.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|key| !key.is_empty());
    if let Some(key) = &key {
        ths::check(key).await?;
    }
    let shared = app.state::<Shared>();
    {
        let mut stored = shared.credentials();
        stored.ths = key.clone().map(|key| ThsKey { key });
        credentials::save(&shared.credentials_path, &stored)?;
        let credentials = stored.clone();
        shared.control.send_modify(|control| control.credentials = credentials);
    }
    ths::configure(key.clone());
    bar::request_render(&app);
    Ok(get_ths(shared))
}

/// The CoinGecko key as the settings window shows it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoinGeckoView {
    /// The key's ends, enough to recognize it; the key never leaves.
    key: Option<String>,
    /// What the key is, once probed.
    access: Option<dexscreener::Access>,
}

#[tauri::command]
pub fn get_coingecko(shared: State<'_, Shared>) -> CoinGeckoView {
    let key = shared.credentials().coingecko.clone();
    CoinGeckoView {
        access: key.as_ref().and_then(|_| dexscreener::coingecko_access()),
        key: key.map(|k| masked(&k.key)),
    }
}

/// Saves (or with `None`, removes) the CoinGecko key, once CoinGecko says
/// what it is.
#[tauri::command]
pub async fn set_coingecko(app: AppHandle, key: Option<String>) -> CmdResult<CoinGeckoView> {
    let key = key
        .map(|key| key.chars().filter(|c| !c.is_whitespace()).collect::<String>())
        .filter(|key| !key.is_empty());
    let access = match &key {
        Some(key) => Some(dexscreener::coingecko_check(key).await?),
        None => None,
    };
    let shared = app.state::<Shared>();
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
#[tauri::command]
pub async fn check_coingecko(shared: State<'_, Shared>) -> CmdResult<dexscreener::Access> {
    let key = shared.credentials().coingecko.clone();
    let key =
        key.ok_or_else(|| CommandError::Invalid(t!("error.coingeckoKeyMissing").to_owned()))?;
    Ok(dexscreener::coingecko_check(&key.key).await?)
}

/// An exchange's API key as the settings window shows it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeKey {
    exchange: ProviderId,
    /// The key's ends, enough to recognize it, when one is saved; the secrets
    /// never leave.
    key: Option<String>,
}

#[tauri::command]
pub fn get_exchange_keys(shared: State<'_, Shared>) -> Vec<ExchangeKey> {
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
#[tauri::command]
pub async fn set_exchange_key(
    app: AppHandle,
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
    let shared = app.state::<Shared>();
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
    Ok(get_exchange_keys(shared))
}

#[tauri::command]
pub fn get_portfolio(shared: State<'_, Shared>) -> Portfolio {
    Portfolio::of(&shared.model().accounts)
}

#[tauri::command]
pub fn open_holdings(app: AppHandle) -> CmdResult<()> {
    window::open_holdings(&app)?;
    Ok(())
}

#[tauri::command]
pub fn open_settings(app: AppHandle) -> CmdResult<()> {
    window::open_settings(&app)?;
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
#[tauri::command]
pub fn get_chart_instrument(shared: State<'_, Shared>) -> Option<String> {
    shared.model().chart.clone()
}

/// Shows the instrument `id` in the chart window (switching it if already open).
#[tauri::command]
pub fn open_chart(app: AppHandle, id: String) -> CmdResult<()> {
    window::open_chart(&app, &id)?;
    Ok(())
}

/// The instrument `id` names, for a chart of one outside the watchlist (the
/// synced Tonghuashun watchlist's).
#[tauri::command]
pub fn chart_instrument(shared: State<'_, Shared>, id: String) -> CmdResult<Instrument> {
    instrument(&shared, &id)
}

#[tauri::command]
pub fn chart_spec(shared: State<'_, Shared>, id: String) -> CmdResult<ChartSpec> {
    let instrument = instrument(&shared, &id)?;
    Ok(instrument.provider.provider().chart_spec(&instrument))
}

#[tauri::command]
pub async fn chart_history(
    shared: State<'_, Shared>,
    id: String,
    interval: u32,
    end: Option<f64>,
    limit: usize,
) -> CmdResult<Vec<Candle>> {
    let instrument = instrument(&shared, &id)?;
    Ok(instrument.provider.provider().history(&instrument, interval, end, limit).await?)
}

#[tauri::command]
pub async fn chart_trades(
    shared: State<'_, Shared>,
    id: String,
    limit: usize,
) -> CmdResult<Vec<Trade>> {
    let instrument = instrument(&shared, &id)?;
    Ok(instrument.provider.provider().recent_trades(&instrument, limit).await?)
}

/// Starts streaming `id` into `events`; returns a handle for `chart_stream_stop`.
#[tauri::command]
pub fn chart_stream(
    app: AppHandle,
    shared: State<'_, Shared>,
    id: String,
    events: Channel<LiveEvent>,
) -> CmdResult<u32> {
    let instrument = instrument(&shared, &id)?;
    let (stop, stopped) = oneshot::channel();
    let handle = shared.next_stream.fetch_add(1, Ordering::Relaxed);
    shared.streams.lock().unwrap_or_else(|e| e.into_inner()).insert(handle, stop);
    let stream = instrument.provider.provider().stream(instrument, stopped, events);
    tauri::async_runtime::spawn(async move {
        stream.await;
        // A stream also ends on its own once its page is gone.
        app.state::<Shared>().streams.lock().unwrap_or_else(|e| e.into_inner()).remove(&handle);
    });
    Ok(handle)
}

#[tauri::command]
pub fn chart_stream_stop(shared: State<'_, Shared>, handle: u32) {
    shared.streams.lock().unwrap_or_else(|e| e.into_inner()).remove(&handle);
}

/// Opens the instrument's page on its provider's website in the browser.
#[tauri::command]
pub fn open_link(shared: State<'_, Shared>, id: String) -> CmdResult<()> {
    let instrument = instrument(&shared, &id)?;
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
