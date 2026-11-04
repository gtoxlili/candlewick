use std::time::Duration;

use gpui_kit::{ActivationPolicy, KeyBinding, Menu, MenuItem, Window, actions};
use objc2::{AnyThread, MainThreadMarker, runtime::AnyObject};
use objc2_app_kit::{NSAboutPanelOptionCredits, NSApplication, NSView};
use objc2_foundation::{NSAttributedString, NSDictionary, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{app::App, window};

actions!(candlewick, [About, Settings, Hide, HideOthers, ShowAll, Minimize, Close, Quit]);

pub const ACTIVATION_RAISES_WINDOWS: bool = true;

pub fn init(app: &App, cx: &mut gpui_kit::App) {
    let handle = app.clone();
    cx.on_action(move |_: &Settings, _| {
        let _ = window::open_settings(&handle);
    });
    cx.on_action(|_: &About, _| {
        let mtm = MainThreadMarker::new().expect("menu actions run on the main thread");
        let text = NSString::from_str(&format!(
            "{}\n© 2026 Hangzhou Healing Era Technology Co., Ltd",
            t!("appMenu.tagline")
        ));
        let credits = NSAttributedString::initWithString(NSAttributedString::alloc(), &text);
        let credits: &NSAttributedString = &credits;
        let object: &AnyObject = credits.as_ref();
        // SAFETY: the credits option accepts an attributed string.
        unsafe {
            let options = NSDictionary::<NSString, AnyObject>::from_slices(
                &[NSAboutPanelOptionCredits],
                &[object],
            );
            NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options);
        }
    });
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.on_action(|_: &Close, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([
        KeyBinding::new("cmd-,", Settings, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-w", Close, None),
    ]);
    relabel(cx);
}

pub fn relabel(cx: &mut gpui_kit::App) {
    use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    cx.set_menus([
        Menu::new("Candlewick").items([
            MenuItem::action(t!("appMenu.about"), About),
            MenuItem::action(t!("tray.settings"), Settings),
            MenuItem::separator(),
            MenuItem::action(t!("appMenu.hide"), Hide),
            MenuItem::action(t!("appMenu.hideOthers"), HideOthers),
            MenuItem::action(t!("appMenu.showAll"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(t!("tray.quit"), Quit),
        ]),
        Menu::new(t!("appMenu.edit")).items([
            MenuItem::action(t!("appMenu.undo"), Undo),
            MenuItem::action(t!("appMenu.redo"), Redo),
            MenuItem::separator(),
            MenuItem::action(t!("appMenu.cut"), Cut),
            MenuItem::action(t!("appMenu.copy"), Copy),
            MenuItem::action(t!("appMenu.paste"), Paste),
            MenuItem::action(t!("appMenu.selectAll"), SelectAll),
        ]),
        Menu::new(t!("appMenu.window")).items([
            MenuItem::action(t!("appMenu.minimize"), Minimize),
            MenuItem::action(t!("appMenu.closeWindow"), Close),
        ]),
    ]);
}

pub fn will_show(app: &App, cx: &mut gpui_kit::App, label: &'static str) {
    cx.set_activation_policy(ActivationPolicy::Regular);
    let handle = app.clone();
    app.spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.on_gpui(move |cx| {
            cx.activate(true);
            window::raise_label(cx, label);
        });
    });
}

fn with_native_window<R>(
    window: &Window,
    f: impl FnOnce(&objc2_app_kit::NSWindow) -> R,
) -> Option<R> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return None };
    // SAFETY: GPUI owns this NSView and calls this function on its main thread.
    let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    view.window().map(|window| f(&window))
}

pub fn is_unminimized(window: &Window) -> bool {
    with_native_window(window, |window| !window.isMiniaturized()).unwrap_or(true)
}

pub fn bring_to_front(window: &Window) {
    with_native_window(window, |native| {
        if native.isMiniaturized() {
            native.deminiaturize(None);
        }
        native.orderFrontRegardless();
    });
    window.activate_window();
}

pub fn did_close_all(cx: &mut gpui_kit::App) {
    cx.set_activation_policy(ActivationPolicy::Accessory);
}
