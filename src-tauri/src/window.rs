//! The settings and chart windows. Each exists only while open: closing one
//! destroys its webview, so the always-on app carries no browser engine in
//! between.
//!
//! Both share one look, which `platform::window` gives them: the system's
//! translucent material behind a transparent page, which draws its own title
//! bar and marks it as the drag region.

use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    webview::NewWindowResponse,
};

use crate::{
    market::ProviderId,
    model::{Model, Settings, Shared, Status},
    net, platform,
    portfolio::Portfolio,
};

pub const SETTINGS: &str = "settings";
pub const CHART: &str = "chart";
pub const HOLDINGS: &str = "holdings";
pub const STATUS_EVENT: &str = "status";
pub const SETTINGS_EVENT: &str = "settings";
pub const CHART_INSTRUMENT_EVENT: &str = "chart-instrument";
pub const PORTFOLIO_EVENT: &str = "portfolio";

/// The feed status line at the bottom of the settings window.
#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub label: String,
    pub tone: &'static str,
}

impl From<&Model> for StatusView {
    /// The status of each provider the watchlist uses, named when there are
    /// several; the most urgent tone wins.
    fn from(model: &Model) -> Self {
        let providers: Vec<_> = model.settings.symbols().into_keys().collect();
        let status = |provider| {
            model.feeds.get(provider).map(|feed| feed.status.clone()).unwrap_or_default()
        };
        match providers.as_slice() {
            [] => Self { label: Status::Idle.label(), tone: Status::Idle.tone() },
            [only] => {
                let status = status(only);
                Self { label: status.label(), tone: status.tone() }
            }
            several => {
                let statuses: Vec<Status> = several.iter().map(status).collect();
                let label = several
                    .iter()
                    .zip(&statuses)
                    .map(|(provider, status)| format!("{} {}", provider.name(), status.label()))
                    .collect::<Vec<_>>()
                    .join("；");
                let urgency =
                    |tone: &str| ["live", "idle", "busy", "error"].iter().position(|t| *t == tone);
                let tone = statuses
                    .iter()
                    .map(Status::tone)
                    .max_by_key(|tone| urgency(tone))
                    .unwrap_or("idle");
                Self { label, tone }
            }
        }
    }
}

struct Spec {
    label: &'static str,
    url: String,
    title: String,
    size: (f64, f64),
    min_size: (f64, f64),
    resizable: bool,
}

pub fn open_settings(app: &AppHandle) -> tauri::Result<()> {
    open(
        app,
        Spec {
            label: SETTINGS,
            url: "index.html".to_owned(),
            title: "Candlewick 设置".to_owned(),
            size: (460.0, 640.0),
            min_size: (460.0, 640.0),
            resizable: false,
        },
    )
}

/// Opens the holdings window; while it is open, holdings refresh often.
pub fn open_holdings(app: &AppHandle) -> tauri::Result<()> {
    set_holdings_open(app, true);
    open(
        app,
        Spec {
            label: HOLDINGS,
            url: "holdings.html".to_owned(),
            title: "Candlewick 持仓".to_owned(),
            size: (760.0, 620.0),
            min_size: (560.0, 420.0),
            resizable: true,
        },
    )
}

fn set_holdings_open(app: &AppHandle, open: bool) {
    app.state::<Shared>().control.send_if_modified(|control| {
        let changed = control.holdings_open != open;
        control.holdings_open = open;
        changed
    });
}

/// Opens the chart for the instrument `id`, or switches the open chart window to it.
pub fn open_chart(app: &AppHandle, id: &str) -> tauri::Result<()> {
    let Some(title) = retarget_chart(app, id)? else {
        return Ok(());
    };
    open(
        app,
        Spec {
            label: CHART,
            url: format!("chart.html?id={}", net::percent_encode(id)),
            title,
            size: (980.0, 640.0),
            min_size: (760.0, 480.0),
            resizable: true,
        },
    )
}

/// Points the chart at the instrument `id` without bringing its window
/// forward: records it (a page still loading reads it from there), retitles
/// an open window and tells its page. Returns the title, or `None` for an
/// instrument not in the watchlist.
fn retarget_chart(app: &AppHandle, id: &str) -> tauri::Result<Option<String>> {
    let title = {
        let shared = app.state::<Shared>();
        let mut model = shared.model();
        let Some(instrument) = model.settings.instrument(id) else {
            return Ok(None);
        };
        let title = format!("{} 行情", instrument.pair_label());
        model.chart = Some(id.to_owned());
        title
    };
    if let Some(window) = app.get_webview_window(CHART) {
        window.set_title(&title)?;
        app.emit_to(CHART, CHART_INSTRUMENT_EVENT, id)?;
    }
    Ok(Some(title))
}

/// The Dock icon was clicked, or the app was launched again while running.
/// On macOS activation already brings open windows forward (and unhides them
/// after Cmd-H); one still loading shows itself once its page is ready. So
/// only when every window is minimized is one restored, the chart first; on
/// Windows, where a second launch activates nothing, it always is. Only with
/// no window open does settings open: the status item can hide behind the
/// notch or a crowded menu bar, the tray icon in the taskbar's overflow, so
/// this is the way back in.
pub fn reopen(app: &AppHandle) -> tauri::Result<()> {
    let open: Vec<WebviewWindow> = [CHART, HOLDINGS, SETTINGS]
        .into_iter()
        .filter_map(|label| app.get_webview_window(label))
        .collect();
    let Some(first) = open.first() else {
        return open_settings(app);
    };
    if platform::window::ACTIVATION_RAISES_WINDOWS
        && open.iter().any(|w| !w.is_minimized().unwrap_or(false))
    {
        return Ok(());
    }
    first.unminimize()?;
    first.show()?;
    platform::window::bring_to_front(first);
    Ok(())
}

/// After settings change: an open chart whose instrument was removed moves to
/// the first remaining one, or closes when none are left.
pub fn sync_chart(app: &AppHandle) {
    let Some(window) = app.get_webview_window(CHART) else {
        return;
    };
    let replacement = {
        let shared = app.state::<Shared>();
        let model = shared.model();
        if model.chart.as_deref().is_some_and(|id| model.settings.instrument(id).is_some()) {
            return;
        }
        model.settings.watchlist.first().map(|instrument| instrument.id())
    };
    let result = match replacement {
        Some(id) => retarget_chart(app, &id).map(|_| ()),
        None => window.close(),
    };
    if let Err(e) = result {
        log::error!("cannot update the chart after a settings change: {e}");
    }
}

fn open(app: &AppHandle, spec: Spec) -> tauri::Result<()> {
    platform::window::will_show(app, spec.label)?;
    if let Some(window) = app.get_webview_window(spec.label) {
        return raise(&window);
    }
    platform::window::create(app, move |app| match app.get_webview_window(spec.label) {
        // Opened again while its creation was queued.
        Some(window) => raise(&window),
        None => build(app, spec),
    })
}

fn raise(window: &WebviewWindow) -> tauri::Result<()> {
    window.unminimize()?;
    window.show()?;
    platform::window::bring_to_front(window);
    Ok(())
}

fn build(app: &AppHandle, spec: Spec) -> tauri::Result<()> {
    let label = spec.label;
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(spec.url.into()))
        .title(spec.title)
        .inner_size(spec.size.0, spec.size.1)
        .min_inner_size(spec.min_size.0, spec.min_size.1)
        .resizable(spec.resizable)
        .maximizable(spec.resizable)
        .center()
        // The webview only ever shows the app's own pages; links out open
        // in the browser.
        .on_navigation(|url| {
            let internal = is_app_url(url);
            if !internal {
                open_externally(url);
            }
            internal
        })
        .on_new_window(|url, _| {
            open_externally(&url);
            NewWindowResponse::Deny
        })
        // Shown by `window_ready` once the page has content, to avoid a
        // blank flash; the fallback below covers a page that never reports.
        .visible(false);
    platform::window::build(builder)?;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        let main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(window) = main.get_webview_window(label)
                && !window.is_visible().unwrap_or(true)
            {
                log::warn!("{label} page did not report ready; showing anyway");
                let _ = window.show();
                platform::window::bring_to_front(&window);
            }
        });
    });
    Ok(())
}

/// The bundled frontend (`tauri://localhost` in WKWebView, a `tauri.localhost`
/// host in WebView2), or the Vite dev server under `tauri dev`.
fn is_app_url(url: &Url) -> bool {
    let host = url.host_str();
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => {
            host == Some("tauri.localhost") || (cfg!(dev) && host == Some("localhost"))
        }
        _ => false,
    }
}

fn open_externally(url: &Url) {
    if url.scheme() == "https" {
        platform::open_url(url.as_str());
    }
}

/// Stops what fed a closed window; back to a bar-only app once the last one
/// is gone.
pub fn on_destroyed(app: &AppHandle, label: &str) {
    if label == CHART {
        // Dropping the senders stops the streams feeding the page.
        app.state::<Shared>().streams.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    if label == SETTINGS {
        for provider in ProviderId::ALL {
            provider.provider().drop_search_cache();
        }
    }
    if label == HOLDINGS {
        set_holdings_open(app, false);
    }
    if app.webview_windows().keys().any(|other| other != label) {
        return;
    }
    platform::window::did_close_all(app);
}

pub fn emit_status(app: &AppHandle, status: &StatusView) {
    if app.get_webview_window(SETTINGS).is_some() {
        let _ = app.emit_to(SETTINGS, STATUS_EVENT, status);
    }
}

pub fn emit_portfolio(app: &AppHandle, portfolio: &Portfolio) {
    if app.get_webview_window(HOLDINGS).is_some() {
        let _ = app.emit_to(HOLDINGS, PORTFOLIO_EVENT, portfolio);
    }
}

/// Keeps open windows in step with saved settings (watchlist, colors).
pub fn emit_settings(app: &AppHandle, settings: &Settings) {
    for label in [SETTINGS, CHART, HOLDINGS] {
        if app.get_webview_window(label).is_some() {
            let _ = app.emit_to(label, SETTINGS_EVENT, settings);
        }
    }
}
