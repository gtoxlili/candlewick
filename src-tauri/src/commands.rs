//! IPC surface for the settings and chart windows.

use std::sync::atomic::Ordering;

use serde::{Serialize, Serializer};
use tauri::{AppHandle, Manager, State, WebviewWindow, ipc::Channel};
use tokio::sync::oneshot;

use crate::{
    macos,
    market::{self, Candle, ChartSpec, LiveEvent, ProviderId, Search, Trade},
    model::{self, Instrument, Settings, Shared},
    tray,
    window::{self, StatusView},
};

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    Invalid(String),
    #[error("无法保存设置：{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    LoginItem(String),
    #[error(transparent)]
    Market(#[from] market::Error),
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
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
    {
        let mut model = shared.model();
        model.quotes.retain(|id, _| settings.instrument(id).is_some());
        model.settings = settings.clone();
    }
    let symbols = settings.symbols();
    shared.control.send_if_modified(|control| {
        let changed = control.symbols != symbols;
        control.symbols = symbols;
        changed
    });
    tray::request_render(&app);
    window::emit_settings(&app, &settings);
    window::sync_chart(&app);
    Ok(settings)
}

#[tauri::command]
pub fn get_status(shared: State<'_, Shared>) -> StatusView {
    StatusView::from(&*shared.model())
}

#[tauri::command]
pub fn get_login_item() -> bool {
    macos::login_item_enabled()
}

#[tauri::command]
pub fn set_login_item(enabled: bool) -> CmdResult<bool> {
    macos::set_login_item(enabled).map_err(CommandError::LoginItem)?;
    Ok(macos::login_item_enabled())
}

/// The page has content; now its window can appear without a blank flash.
#[tauri::command]
pub fn window_ready(window: WebviewWindow) -> CmdResult<()> {
    window.show()?;
    window::bring_to_front(&window);
    Ok(())
}

/// Instruments matching the query, from every provider.
#[tauri::command]
pub async fn search_instruments(query: String) -> Search {
    let mut all = Search::default();
    for provider in ProviderId::ALL {
        let found = provider.provider().search(&query).await;
        all.candidates.extend(found.candidates);
        all.degraded |= found.degraded;
    }
    all
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
        macos::open_url(&link.url);
    }
    Ok(())
}

fn instrument(shared: &Shared, id: &str) -> CmdResult<Instrument> {
    let model = shared.model();
    let found = model.settings.instrument(id).cloned();
    found.ok_or_else(|| CommandError::Invalid(format!("未知的交易对：{id}")))
}
