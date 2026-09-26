//! AppKit and ServiceManagement glue.

use std::{path::Path, ptr::NonNull, sync::Arc};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSFont, NSFontWeightRegular, NSStatusItem, NSWindow, NSWorkspace,
    NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSString, NSURL};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Why streaming is paused: in each case nobody can see the menu bar.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum Pause {
    SystemSleep = 1 << 0,
    DisplaySleep = 1 << 1,
    SessionInactive = 1 << 2,
}

/// Calls `on_change(reason, paused)` on sleep/wake, display sleep/wake and
/// fast-user-switching. Observers live for the rest of the process.
pub fn observe_pauses(on_change: impl Fn(Pause, bool) + Send + Sync + 'static) {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let on_change = Arc::new(on_change);
    // SAFETY: reading immutable AppKit notification-name constants.
    let events = unsafe {
        [
            (NSWorkspaceWillSleepNotification, Pause::SystemSleep, true),
            (NSWorkspaceDidWakeNotification, Pause::SystemSleep, false),
            (NSWorkspaceScreensDidSleepNotification, Pause::DisplaySleep, true),
            (NSWorkspaceScreensDidWakeNotification, Pause::DisplaySleep, false),
            (NSWorkspaceSessionDidResignActiveNotification, Pause::SessionInactive, true),
            (NSWorkspaceSessionDidBecomeActiveNotification, Pause::SessionInactive, false),
        ]
    };
    for (name, reason, paused) in events {
        let on_change = Arc::clone(&on_change);
        let block = RcBlock::new(move |_: NonNull<NSNotification>| on_change(reason, paused));
        // SAFETY: the block is Send + Sync; a nil queue runs it on the posting
        // (main) thread.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(token);
    }
}

/// Asks to make this the active app. Call it shortly after the user's click
/// (not in the same turn as switching to the regular activation policy,
/// which macOS silently ignores) — see `window::open`.
pub fn activate_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)] // still the more forceful request on macOS 14+
        app.activateIgnoringOtherApps(true);
        app.activate();
    }
}

/// Orders the window above other apps' windows even when this app is not
/// (yet) active; `makeKeyAndOrderFront` alone leaves it behind them.
pub fn order_front_regardless(ns_window: *mut std::ffi::c_void) {
    if ns_window.is_null() || MainThreadMarker::new().is_none() {
        return;
    }
    // SAFETY: a live NSWindow pointer from Tauri, used on the main thread.
    let window: &NSWindow = unsafe { &*ns_window.cast() };
    window.orderFrontRegardless();
}

/// Tabular digits keep the status item from changing width every tick.
pub fn use_tabular_digits(item: &NSStatusItem) {
    let Some(button) = MainThreadMarker::new().and_then(|mtm| item.button(mtm)) else {
        return;
    };
    let size = NSFont::menuBarFontOfSize(0.0).pointSize();
    // SAFETY: reading an immutable AppKit constant.
    let font = NSFont::monospacedDigitSystemFontOfSize_weight(size, unsafe { NSFontWeightRegular });
    button.setFont(Some(&font));
}

pub fn open_url(url: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

pub fn login_item_enabled() -> bool {
    // SAFETY: plain ServiceManagement queries on the app's own service.
    unsafe { SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled }
}

/// Registers or removes the app as a login item (System Settings → General →
/// Login Items). Only works for a signed app bundle.
pub fn set_login_item(enabled: bool) -> Result<(), String> {
    // SAFETY: plain ServiceManagement calls on the app's own service.
    unsafe {
        let service = SMAppService::mainAppService();
        let status = service.status();
        let result = match (enabled, status) {
            (true, SMAppServiceStatus::Enabled) | (false, SMAppServiceStatus::NotRegistered) => {
                Ok(())
            }
            (true, _) => service.registerAndReturnError(),
            (false, _) => service.unregisterAndReturnError(),
        };
        result.map_err(|e| e.localizedDescription().to_string())?;
        if enabled && service.status() == SMAppServiceStatus::RequiresApproval {
            SMAppService::openSystemSettingsLoginItems();
        }
    }
    Ok(())
}

/// Registering a login item from a DMG or Downloads would point at a path
/// that goes away, so first-launch registration only happens once installed.
pub fn is_installed() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let user_apps = std::env::var_os("HOME").map(|home| Path::new(&home).join("Applications"));
    exe.starts_with("/Applications") || user_apps.is_some_and(|dir| exe.starts_with(dir))
}
