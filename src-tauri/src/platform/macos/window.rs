//! Window chrome and activation on macOS: a transparent window over an
//! NSVisualEffectView (vibrancy), with the title bar overlaid on the page,
//! which draws its own title bar under the traffic lights. The app turns
//! regular (Dock icon, Cmd-Tab, app menu) while a window is open.

use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindow};
use tauri::{
    ActivationPolicy, AppHandle, Manager, TitleBarStyle, WebviewWindow, WebviewWindowBuilder, Wry,
    menu::{AboutMetadata, Menu, PredefinedMenuItem, Submenu},
    window::{Effect, EffectState, EffectsBuilder},
};

pub type Builder<'a> = WebviewWindowBuilder<'a, Wry, AppHandle>;

/// Activating the app (a Dock click, a second launch) brings its open windows
/// forward by itself.
pub const ACTIVATION_RAISES_WINDOWS: bool = true;

/// Builds a window with the look both share.
pub fn build(builder: Builder<'_>) -> tauri::Result<()> {
    decorate(builder).build()?;
    Ok(())
}

fn decorate(builder: Builder<'_>) -> Builder<'_> {
    let effects = EffectsBuilder::new()
        .effect(Effect::Sidebar)
        .state(EffectState::FollowsWindowActiveState)
        .build();
    builder
        // The page draws its own title bar under the native traffic lights.
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        // Vibrancy shows through the transparent window and webview.
        .transparent(true)
        .effects(effects)
}

/// Builds a window right away: AppKit has no reentrancy trap here.
pub fn create(
    app: &AppHandle,
    build: impl FnOnce(&AppHandle) -> tauri::Result<()> + Send + 'static,
) -> tauri::Result<()> {
    build(app)
}

/// A window with `label` is about to show: become a regular app, and ask to
/// be the active one.
pub fn will_show(app: &AppHandle, label: &'static str) -> tauri::Result<()> {
    app.set_activation_policy(ActivationPolicy::Regular)?;
    // macOS ignores activation requested in the same turn as the policy
    // switch, and one requested much later (after the page loads) no longer
    // counts as a response to the click. ~100 ms after the click works.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            activate_app();
            if let Some(window) = main.get_webview_window(label)
                && window.is_visible().unwrap_or(false)
            {
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
        log::error!("cannot focus {}: {e}", window.label());
    }
    if let Ok(ns_window) = window.ns_window() {
        order_front_regardless(ns_window);
    }
}

/// WKWebView hides a minimized page by itself.
pub fn resized(_app: &AppHandle, _label: &str) {}

/// Back to a menu-bar-only app once the last window is gone.
pub fn did_close_all(app: &AppHandle) {
    if let Err(e) = app.set_activation_policy(ActivationPolicy::Accessory) {
        log::error!("cannot restore accessory policy: {e}");
    }
    // WebKit tears down asynchronously and malloc keeps the freed pages
    // dirty; hand them back so the idle footprint returns to its baseline.
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        release_free_memory();
    });
}

/// Asks to make this the active app. Call it shortly after the user's click
/// (not in the same turn as switching to the regular activation policy,
/// which macOS silently ignores) — see [`will_show`].
fn activate_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)] // still the more forceful request on macOS 14+
        app.activateIgnoringOtherApps(true);
        app.activate();
    }
}

/// Orders the window above other apps' windows even when this app is not
/// (yet) active; `makeKeyAndOrderFront` alone leaves it behind them.
fn order_front_regardless(ns_window: *mut std::ffi::c_void) {
    if ns_window.is_null() || MainThreadMarker::new().is_none() {
        return;
    }
    // SAFETY: a live NSWindow pointer from Tauri, used on the main thread.
    let window: &NSWindow = unsafe { &*ns_window.cast() };
    window.orderFrontRegardless();
}

fn release_free_memory() {
    unsafe extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
    }
    // SAFETY: a null zone means "all zones"; goal 0 releases everything possible.
    let released = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    log::debug!("returned {} KiB of free heap to the system", released / 1024);
}

/// The app speaks another language: so does its menu.
pub fn relabel(app: &AppHandle) {
    if let Err(e) = app_menu(app).and_then(|menu| app.set_menu(menu).map(|_| ())) {
        log::error!("cannot relabel the app menu: {e}");
    }
}

/// The app menu shown while a window makes this a regular app, in the
/// app's language. Edit items matter: without them Cmd-C/V/A do nothing in
/// the text field.
pub fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let about = AboutMetadata {
        name: Some("Candlewick".to_owned()),
        version: Some(app.package_info().version.to_string()),
        comments: Some(t!("appMenu.tagline").to_owned()),
        copyright: app.config().bundle.copyright.clone(),
        ..Default::default()
    };
    let app_submenu = Submenu::with_items(
        app,
        "Candlewick",
        true,
        &[
            &PredefinedMenuItem::about(app, Some(t!("appMenu.about")), Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some(t!("appMenu.hide")))?,
            &PredefinedMenuItem::hide_others(app, Some(t!("appMenu.hideOthers")))?,
            &PredefinedMenuItem::show_all(app, Some(t!("appMenu.showAll")))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some(t!("tray.quit")))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        t!("appMenu.edit"),
        true,
        &[
            &PredefinedMenuItem::undo(app, Some(t!("appMenu.undo")))?,
            &PredefinedMenuItem::redo(app, Some(t!("appMenu.redo")))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some(t!("appMenu.cut")))?,
            &PredefinedMenuItem::copy(app, Some(t!("appMenu.copy")))?,
            &PredefinedMenuItem::paste(app, Some(t!("appMenu.paste")))?,
            &PredefinedMenuItem::select_all(app, Some(t!("appMenu.selectAll")))?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        t!("appMenu.window"),
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(t!("appMenu.minimize")))?,
            &PredefinedMenuItem::close_window(app, Some(t!("appMenu.closeWindow")))?,
        ],
    )?;
    Menu::with_items(app, &[&app_submenu, &edit, &window])
}
