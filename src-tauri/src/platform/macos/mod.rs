//! AppKit, ServiceManagement and SystemConfiguration glue.

pub mod bar;
pub mod proxy;
mod ticker;
pub mod window;

use std::{path::Path, ptr::NonNull, sync::Arc};

use block2::RcBlock;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{NSNotification, NSString, NSURL};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use tauri::{Builder, Wry};

use super::Pause;

/// Whether the first launch from an install location opens the settings
/// window. The status item is in plain sight, so macOS leaves the app to it.
pub const SETTINGS_ON_FIRST_RUN: bool = false;

/// LaunchServices keeps one instance and reports a second launch as a
/// reopen, so there is nothing to claim.
pub fn claim_single_instance() {}

/// The app menu (see [`window::app_menu`]).
pub fn configure(builder: Builder<Wry>) -> Builder<Wry> {
    builder.menu(window::app_menu)
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
