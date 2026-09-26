//! The settings window. It exists only while open: closing it destroys the
//! webview, so the always-on app carries no WebKit process in between.

use serde::Serialize;
use tauri::{
    ActivationPolicy, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    Wry,
    menu::{AboutMetadata, Menu, PredefinedMenuItem, Submenu},
};

use crate::{macos, model::Status};

pub const LABEL: &str = "settings";
pub const STATUS_EVENT: &str = "status";

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

pub fn open(app: &AppHandle) -> tauri::Result<()> {
    // A regular app while the window is open: Dock icon, Cmd-Tab, app menu.
    app.set_activation_policy(ActivationPolicy::Regular)?;
    // macOS ignores activation requested in the same turn as the policy
    // switch, and one requested much later (after the page loads) no longer
    // counts as a response to the click. ~100 ms after the click works.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            macos::activate_app();
            if let Some(window) = main.get_webview_window(LABEL)
                && window.is_visible().unwrap_or(false)
            {
                bring_to_front(&window);
            }
        });
    });
    if let Some(window) = app.get_webview_window(LABEL) {
        window.unminimize()?;
        window.show()?;
        bring_to_front(&window);
        return Ok(());
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Coin Tray 设置")
        .inner_size(460.0, 640.0)
        .min_inner_size(420.0, 480.0)
        .maximizable(false)
        .center()
        // Shown by `settings_ready` once the page has its data, to avoid a
        // blank flash; the fallback below covers a page that never reports.
        .visible(false)
        .build()?;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(window) = main.get_webview_window(LABEL)
                && !window.is_visible().unwrap_or(true)
            {
                log::warn!("settings page did not report ready; showing anyway");
                let _ = window.show();
                bring_to_front(&window);
            }
        });
    });
    Ok(())
}

/// Key window and above other apps' windows, whether or not activation has
/// gone through yet.
pub fn bring_to_front(window: &WebviewWindow) {
    if let Err(e) = window.set_focus() {
        log::error!("cannot focus settings: {e}");
    }
    if let Ok(ns_window) = window.ns_window() {
        macos::order_front_regardless(ns_window);
    }
}

/// Back to a menu-bar-only app once the window is gone.
pub fn on_destroyed(app: &AppHandle) {
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
    if app.get_webview_window(LABEL).is_some() {
        let _ = app.emit_to(LABEL, STATUS_EVENT, StatusView::from(status));
    }
}

/// The app menu shown while the settings window makes this a regular app.
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
