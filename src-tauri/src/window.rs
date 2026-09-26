//! The settings and chart windows. Each exists only while open: closing one
//! destroys its webview, so the always-on app carries no WebKit process in
//! between.
//!
//! Both share one look: a transparent window over an NSVisualEffectView
//! (vibrancy), with the title bar overlaid on the page, which draws its own
//! title bar and marks it as the drag region.

use serde::Serialize;
use tauri::{
    ActivationPolicy, AppHandle, Emitter, Manager, TitleBarStyle, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, Wry,
    menu::{AboutMetadata, Menu, PredefinedMenuItem, Submenu},
    webview::NewWindowResponse,
    window::{Effect, EffectState, EffectsBuilder},
};

use crate::{
    macos,
    model::{Settings, Shared, Status},
    net,
};

pub const SETTINGS: &str = "settings";
pub const CHART: &str = "chart";
pub const STATUS_EVENT: &str = "status";
pub const SETTINGS_EVENT: &str = "settings";
pub const CHART_SYMBOL_EVENT: &str = "chart-symbol";

#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub label: String,
    pub tone: &'static str,
}

impl From<&Status> for StatusView {
    fn from(status: &Status) -> Self {
        Self { label: status.label(), tone: status.tone() }
    }
}

struct Spec {
    label: &'static str,
    url: String,
    title: String,
    size: (f64, f64),
    min_size: (f64, f64),
    resizable: bool,
    material: Effect,
}

pub fn open_settings(app: &AppHandle) -> tauri::Result<()> {
    open(
        app,
        Spec {
            label: SETTINGS,
            url: "index.html".to_owned(),
            title: "Coin Tray 设置".to_owned(),
            size: (460.0, 640.0),
            min_size: (460.0, 640.0),
            resizable: false,
            material: Effect::Sidebar,
        },
    )
}

/// Opens the chart for `symbol`, or switches the open chart window to it.
pub fn open_chart(app: &AppHandle, symbol: &str) -> tauri::Result<()> {
    let Some(title) = retarget_chart(app, symbol)? else {
        return Ok(());
    };
    open(
        app,
        Spec {
            label: CHART,
            url: format!("chart.html?symbol={}", net::percent_encode(symbol)),
            title,
            size: (980.0, 640.0),
            min_size: (760.0, 480.0),
            resizable: true,
            material: Effect::Sidebar,
        },
    )
}

/// Points the chart at `symbol` without bringing its window forward: records
/// it (a page still loading reads it from there), retitles an open window and
/// tells its page. Returns the title, or `None` for a pair not in settings.
fn retarget_chart(app: &AppHandle, symbol: &str) -> tauri::Result<Option<String>> {
    let title = {
        let shared = app.state::<Shared>();
        let mut model = shared.model();
        let Some(coin) = model.settings.coins.iter().find(|c| c.symbol == symbol) else {
            return Ok(None);
        };
        let title = format!("{} 行情", coin.pair_label());
        model.chart_symbol = Some(symbol.to_owned());
        title
    };
    if let Some(window) = app.get_webview_window(CHART) {
        window.set_title(&title)?;
        app.emit_to(CHART, CHART_SYMBOL_EVENT, symbol)?;
    }
    Ok(Some(title))
}

/// The Dock icon was clicked, or the app was launched again while running.
/// Activation already brings open windows forward (and unhides them after
/// Cmd-H); one still loading shows itself once its page is ready. So only when
/// every window is minimized is one restored, the chart first. Only with no
/// window open does settings open: the status item can hide behind the notch
/// or a crowded menu bar, so this is the way back in.
pub fn reopen(app: &AppHandle) -> tauri::Result<()> {
    let open: Vec<WebviewWindow> =
        [CHART, SETTINGS].into_iter().filter_map(|label| app.get_webview_window(label)).collect();
    let Some(first) = open.first() else {
        return open_settings(app);
    };
    if open.iter().any(|w| !w.is_minimized().unwrap_or(false)) {
        return Ok(());
    }
    first.unminimize()?;
    first.show()?;
    bring_to_front(first);
    Ok(())
}

/// After settings change: an open chart whose pair was removed moves to the
/// first remaining pair, or closes when none are left.
pub fn sync_chart(app: &AppHandle) {
    let Some(window) = app.get_webview_window(CHART) else {
        return;
    };
    let replacement = {
        let shared = app.state::<Shared>();
        let model = shared.model();
        let current = model.chart_symbol.as_deref();
        if model.settings.coins.iter().any(|c| Some(c.symbol.as_str()) == current) {
            return;
        }
        model.settings.coins.first().map(|c| c.symbol.clone())
    };
    let result = match replacement {
        Some(symbol) => retarget_chart(app, &symbol).map(|_| ()),
        None => window.close(),
    };
    if let Err(e) = result {
        log::error!("cannot update the chart after a settings change: {e}");
    }
}

fn open(app: &AppHandle, spec: Spec) -> tauri::Result<()> {
    // A regular app while a window is open: Dock icon, Cmd-Tab, app menu.
    app.set_activation_policy(ActivationPolicy::Regular)?;
    // macOS ignores activation requested in the same turn as the policy
    // switch, and one requested much later (after the page loads) no longer
    // counts as a response to the click. ~100 ms after the click works.
    let label = spec.label;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            macos::activate_app();
            if let Some(window) = main.get_webview_window(label)
                && window.is_visible().unwrap_or(false)
            {
                bring_to_front(&window);
            }
        });
    });
    if let Some(window) = app.get_webview_window(label) {
        window.unminimize()?;
        window.show()?;
        bring_to_front(&window);
        return Ok(());
    }

    let effects = EffectsBuilder::new()
        .effect(spec.material)
        .state(EffectState::FollowsWindowActiveState)
        .build();
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(spec.url.into()))
        .title(spec.title)
        .inner_size(spec.size.0, spec.size.1)
        .min_inner_size(spec.min_size.0, spec.min_size.1)
        .resizable(spec.resizable)
        .maximizable(spec.resizable)
        .center()
        // The page draws its own title bar under the native traffic lights.
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        // Vibrancy shows through the transparent window and webview.
        .transparent(true)
        .effects(effects)
        // The webview only ever shows the app's own pages; links out (such
        // as the chart's TradingView attribution) open in the browser.
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
        .visible(false)
        .build()?;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(window) = main.get_webview_window(label)
                && !window.is_visible().unwrap_or(true)
            {
                log::warn!("{label} page did not report ready; showing anyway");
                let _ = window.show();
                bring_to_front(&window);
            }
        });
    });
    Ok(())
}

/// The bundled frontend, or the Vite dev server under `tauri dev`.
fn is_app_url(url: &Url) -> bool {
    url.scheme() == "tauri" || (cfg!(dev) && url.host_str() == Some("localhost"))
}

fn open_externally(url: &Url) {
    if url.scheme() == "https" {
        macos::open_url(url.as_str());
    }
}

/// Key window and above other apps' windows, whether or not activation has
/// gone through yet.
pub fn bring_to_front(window: &WebviewWindow) {
    if let Err(e) = window.set_focus() {
        log::error!("cannot focus {}: {e}", window.label());
    }
    if let Ok(ns_window) = window.ns_window() {
        macos::order_front_regardless(ns_window);
    }
}

/// Back to a menu-bar-only app once the last window is gone.
pub fn on_destroyed(app: &AppHandle, label: &str) {
    if app.webview_windows().keys().any(|other| other != label) {
        return;
    }
    if let Err(e) = app.set_activation_policy(ActivationPolicy::Accessory) {
        log::error!("cannot restore accessory policy: {e}");
    }
    // WebKit tears down asynchronously and malloc keeps the freed pages
    // dirty; hand them back so the idle footprint returns to its baseline.
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        release_free_memory();
    });
}

fn release_free_memory() {
    unsafe extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
    }
    // SAFETY: a null zone means "all zones"; goal 0 releases everything possible.
    let released = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    log::debug!("returned {} KiB of free heap to the system", released / 1024);
}

pub fn emit_status(app: &AppHandle, status: &Status) {
    if app.get_webview_window(SETTINGS).is_some() {
        let _ = app.emit_to(SETTINGS, STATUS_EVENT, StatusView::from(status));
    }
}

/// Keeps open windows in step with saved settings (coin list, colors).
pub fn emit_settings(app: &AppHandle, settings: &Settings) {
    for label in [SETTINGS, CHART] {
        if app.get_webview_window(label).is_some() {
            let _ = app.emit_to(label, SETTINGS_EVENT, settings);
        }
    }
}

/// The app menu shown while a window makes this a regular app.
/// Edit items matter: without them Cmd-C/V/A do nothing in the text field.
pub fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let about = AboutMetadata {
        name: Some("Coin Tray".to_owned()),
        version: Some(app.package_info().version.to_string()),
        comments: Some("在菜单栏显示币安现货实时价格".to_owned()),
        copyright: app.config().bundle.copyright.clone(),
        ..Default::default()
    };
    let app_submenu = Submenu::with_items(
        app,
        "Coin Tray",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("关于 Coin Tray"), Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("隐藏 Coin Tray"))?,
            &PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?,
            &PredefinedMenuItem::show_all(app, Some("全部显示"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("退出 Coin Tray"))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("拷贝"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("最小化"))?,
            &PredefinedMenuItem::close_window(app, Some("关闭窗口"))?,
        ],
    )?;
    Menu::with_items(app, &[&app_submenu, &edit, &window])
}
