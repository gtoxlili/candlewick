//! Window chrome on Windows: no system title bar (the page draws its own,
//! with Windows 11's caption buttons, and marks it with CSS `app-region:
//! drag`, which WebView2 turns into a real caption: dragging, snapping,
//! double-click to maximize, the system menu on right-click), over Mica on
//! Windows 11 or the solid Fluent background on Windows 10. The window keeps
//! its resizable frame and shadow, so it has rounded corners and resizes from
//! its edges.

use std::time::Duration;

use tauri::{
    AppHandle, Manager, WebviewWindow, WebviewWindowBuilder, Wry,
    webview::ScrollBarStyle,
    window::{Color, Effect, EffectsBuilder},
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    ICoreWebView2_19, ICoreWebView2Settings3,
};
use windows::core::{BOOL, Interface};

use super::{portable::color::Tone, theme::Look};

pub type Builder<'a> = WebviewWindowBuilder<'a, Wry, AppHandle>;

/// A second launch activates nothing by itself; `reopen` raises windows.
pub const ACTIVATION_RAISES_WINDOWS: bool = false;

/// Mica needs Windows 11 (build 22000); before that the window would be
/// see-through, as transparency doesn't depend on the effect taking hold.
fn mica() -> bool {
    windows_version::OsVersion::current().build >= 22000
}

/// No app menu to relabel: the tray's menu is built each time it opens.
pub fn relabel(_app: &AppHandle) {}

pub fn build(builder: Builder<'_>) -> tauri::Result<()> {
    let mica = mica();
    let builder = builder
        .decorations(false)
        .shadow(true)
        .scroll_bar_style(ScrollBarStyle::FluentOverlay)
        .general_autofill_enabled(false)
        .initialization_script(chrome_script(mica));
    let builder = if mica {
        builder.transparent(true).effects(EffectsBuilder::new().effect(Effect::Mica).build())
    } else {
        // Fluent's SolidBackgroundFillColorBase; the page paints the same.
        builder.background_color(match Look::current().apps {
            Tone::Dark => Color(32, 32, 32, 255),
            Tone::Light => Color(243, 243, 243, 255),
        })
    };
    let window = builder.build()?;
    lock_browser_keys(&window);
    Ok(())
}

/// What the page needs to know before it draws: whether Mica is behind it.
fn chrome_script(mica: bool) -> String {
    let chrome = serde_json::json!({
        "backdrop": if mica { "mica" } else { "solid" },
    });
    format!(
        "Object.defineProperty(window, '__CANDLEWICK__', {{ value: Object.freeze({chrome}) }});"
    )
}

/// Browser shortcuts have no place in an app window: F5 and Ctrl+R would
/// reload the page, Ctrl+P print it, Ctrl+F search it. Kept in development,
/// where reloading and the developer tools help.
fn lock_browser_keys(window: &WebviewWindow) {
    if cfg!(debug_assertions) {
        return;
    }
    let result = window.with_webview(|webview| {
        // SAFETY: WebView2 calls on the webview's own thread (this closure
        // runs on the main thread).
        unsafe {
            let settings = webview.controller().CoreWebView2().and_then(|core| core.Settings());
            if let Ok(settings) =
                settings.and_then(|settings| settings.cast::<ICoreWebView2Settings3>())
            {
                let _ = settings.SetAreBrowserAcceleratorKeysEnabled(false);
            }
        }
    });
    if let Err(e) = result {
        log::warn!("cannot turn off browser shortcuts: {e}");
    }
}

/// Builds a window in a fresh main-thread turn: WebView2 can't create a
/// webview inside one of its own callbacks, which is where IPC commands run.
pub fn create(
    app: &AppHandle,
    build: impl FnOnce(&AppHandle) -> tauri::Result<()> + Send + 'static,
) -> tauri::Result<()> {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let handle = app.clone();
        let queued = app.run_on_main_thread(move || {
            if let Err(e) = build(&handle) {
                log::error!("cannot open a window: {e}");
            }
        });
        if let Err(e) = queued {
            log::error!("cannot reach the main thread: {e}");
        }
    });
    Ok(())
}

/// A window is about to show: full speed while the user is looking.
pub fn will_show(_app: &AppHandle, _label: &'static str) -> tauri::Result<()> {
    super::set_efficiency_mode(false);
    Ok(())
}

/// Focused and in front. This follows the user's click in the dropdown or a
/// second launch, both of which let this process take the foreground.
pub fn bring_to_front(window: &WebviewWindow) {
    if let Err(e) = window.set_focus() {
        log::error!("cannot focus {}: {e}", window.label());
    }
}

/// WebView2 goes on painting a minimized window unless its host says
/// otherwise. Hidden while minimized, it stops, and the page reads as hidden
/// (the chart pauses its stream after a minute), as WKWebView does by itself.
/// Its memory target drops meanwhile, so WebView2 trims what it can.
pub fn resized(app: &AppHandle, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    let visible = !window.is_minimized().unwrap_or(false);
    let result = window.with_webview(move |webview| {
        // SAFETY: WebView2 calls on the webview's own (main) thread.
        unsafe {
            let controller = webview.controller();
            let mut shown = BOOL(1);
            if controller.IsVisible(&mut shown).is_err() || shown.as_bool() == visible {
                return;
            }
            let _ = controller.SetIsVisible(visible);
            let level = if visible {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            };
            if let Ok(core) =
                controller.CoreWebView2().and_then(|core| core.cast::<ICoreWebView2_19>())
            {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        }
    });
    if let Err(e) = result {
        log::debug!("cannot update {label}'s visibility: {e}");
    }
}

/// Back to the taskbar alone once the last window is gone.
pub fn did_close_all(_app: &AppHandle) {
    super::set_efficiency_mode(true);
    // WebView2 winds down over a few seconds.
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        super::release_free_memory();
    });
}
