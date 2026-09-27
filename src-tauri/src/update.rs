//! Updates, from the latest GitHub release.
//!
//! A little after launch, and every six hours while automatic updates are
//! on, the release's manifest is fetched along the route every other request
//! takes (the system proxy, the platform's certificate check). A newer
//! version is downloaded and its signature checked right away. On macOS the
//! bundle is swapped on disk there and then, so even the next launch runs
//! it; on Windows, and wherever swapping would need an administrator, the
//! verified package waits in memory. The restart waits until nobody would
//! notice it: no window open, and the display off or the session locked.
//! Whoever wants it sooner has the dropdown's item and the settings' button.
//!
//! Only an installed copy updates itself (`platform::is_installed`): a build
//! run from elsewhere would have its own directory replaced.

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    bar, http,
    model::Shared,
    net,
    platform::{self, Pause},
    window,
};

/// The feeds and the first render come first.
const FIRST_CHECK: Duration = Duration::from_secs(90);
/// Releases are few; a machine that stays up for weeks still hears of one
/// the same day.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How often a ready update looks for its moment, besides every pause change.
const TICK: Duration = Duration::from_secs(60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// The plugin's timeout covers the check, not the download, and a stalled
/// download must not hold the updater for good. Plenty for a few megabytes.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20 * 60);

const EVENT: &str = "update";

/// What the settings show beside the version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateState {
    /// Not an installed copy.
    Disabled,
    /// Nothing newer; `checked` once a check has reached the manifest.
    Idle {
        checked: bool,
    },
    Checking,
    /// The last check didn't reach the manifest.
    Unreachable,
    Downloading {
        version: String,
    },
    /// Verified; a restart runs it.
    Ready {
        version: String,
    },
    Restarting {
        version: String,
    },
    /// The download or its installation failed; the next check tries again.
    Failed {
        version: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    /// The running version.
    pub current: String,
    pub state: UpdateState,
}

pub struct Updater {
    state: Mutex<UpdateState>,
    /// Checks and restarts never overlap.
    busy: tokio::sync::Mutex<()>,
    /// A verified package still to install: always on Windows, where the
    /// installer ends the process; on macOS when swapping the bundle needs
    /// an administrator, which only a restart the user asked for may prompt.
    pending: Mutex<Option<(Update, Vec<u8>)>>,
}

/// Starts checking for updates, if this copy is installed.
pub fn start(app: &AppHandle) {
    let enabled = !cfg!(debug_assertions) && platform::is_installed();
    let updater = Arc::new(Updater {
        state: Mutex::new(if enabled {
            UpdateState::Idle { checked: false }
        } else {
            UpdateState::Disabled
        }),
        busy: tokio::sync::Mutex::new(()),
        pending: Mutex::new(None),
    });
    app.manage(updater.clone());
    if enabled {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { updater.run(&app).await });
    }
}

pub fn view(app: &AppHandle) -> UpdateView {
    UpdateView { current: app.package_info().version.to_string(), state: state(app) }
}

/// The version a restart would run, if one is ready.
pub fn ready(app: &AppHandle) -> Option<String> {
    match state(app) {
        UpdateState::Ready { version } => Some(version),
        _ => None,
    }
}

/// Checks now, whether or not automatic updates are on.
pub fn check_now(app: &AppHandle) {
    if let Some(updater) = updater(app) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { updater.check(&app).await });
    }
}

/// Restarts into the ready update.
pub fn restart_now(app: &AppHandle) {
    if let Some(updater) = updater(app) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { updater.restart(&app).await });
    }
}

fn updater(app: &AppHandle) -> Option<Arc<Updater>> {
    app.try_state::<Arc<Updater>>().map(|updater| updater.inner().clone())
}

fn state(app: &AppHandle) -> UpdateState {
    updater(app).map_or(UpdateState::Disabled, |updater| updater.state())
}

/// Nobody would see a restart: the display is off or the session locked,
/// and no window is open.
fn unnoticed(paused: u8, windows_open: bool) -> bool {
    let unseen = Pause::DisplaySleep as u8 | Pause::SessionInactive as u8;
    paused & unseen != 0 && !windows_open
}

impl Updater {
    fn state(&self) -> UpdateState {
        lock(&self.state).clone()
    }

    fn set(&self, app: &AppHandle, state: UpdateState) {
        let was_ready = matches!(
            std::mem::replace(&mut *lock(&self.state), state.clone()),
            UpdateState::Ready { .. }
        );
        // The dropdown offers a ready update.
        if was_ready != matches!(state, UpdateState::Ready { .. }) {
            bar::request_render(app);
        }
        if app.get_webview_window(window::SETTINGS).is_some() {
            let _ = app.emit_to(window::SETTINGS, EVENT, view(app));
        }
    }

    async fn run(&self, app: &AppHandle) {
        let mut control = app.state::<Shared>().control.subscribe();
        tokio::time::sleep(FIRST_CHECK).await;
        let mut next_check = tokio::time::Instant::now();
        loop {
            let automatic = app.state::<Shared>().model().settings.auto_update;
            if automatic && tokio::time::Instant::now() >= next_check {
                self.check(app).await;
                next_check = tokio::time::Instant::now() + CHECK_EVERY;
            }
            let paused = control.borrow().paused;
            if automatic
                && matches!(self.state(), UpdateState::Ready { .. })
                && self.restarts_quietly()
                && unnoticed(paused, !app.webview_windows().is_empty())
            {
                self.restart(app).await;
            }
            tokio::select! {
                // The display going off is the moment a ready update waits for.
                changed = control.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(TICK) => {}
            }
        }
    }

    /// Whether the restart needs nobody: not an administrator's password to
    /// install a package still held on macOS.
    fn restarts_quietly(&self) -> bool {
        cfg!(windows) || lock(&self.pending).is_none()
    }

    async fn check(&self, app: &AppHandle) {
        let Ok(_busy) = self.busy.try_lock() else {
            return;
        };
        // A development/uninstalled copy must never replace itself. A
        // verified update already waiting for restart needs no new check.
        if matches!(
            self.state(),
            UpdateState::Disabled | UpdateState::Ready { .. } | UpdateState::Restarting { .. }
        ) {
            return;
        }
        self.set(app, UpdateState::Checking);
        let found = match client(app) {
            Ok(updater) => updater.check().await,
            Err(e) => Err(e),
        };
        let update = match found {
            Ok(Some(update)) => update,
            Ok(None) => {
                self.set(app, UpdateState::Idle { checked: true });
                return;
            }
            // Offline, or a release still being published: the next check
            // tries again.
            Err(e) => {
                log::info!("update check did not complete: {e}");
                self.set(app, UpdateState::Unreachable);
                return;
            }
        };
        let version = update.version.clone();
        log::info!("update {version} available");
        self.set(app, UpdateState::Downloading { version: version.clone() });
        match tokio::time::timeout(DOWNLOAD_TIMEOUT, update.download(|_, _| {}, || {})).await {
            Ok(Ok(bytes)) => self.stage(app, update, bytes).await,
            Ok(Err(e)) => {
                log::warn!("update {version} did not download: {e}");
                self.set(app, UpdateState::Failed { version });
            }
            Err(_) => {
                log::warn!("update {version} stopped downloading");
                self.set(app, UpdateState::Failed { version });
            }
        }
    }

    /// Swaps the bundle now if that needs nobody's password: the running
    /// process keeps its mapped binary, and the next launch is the new one.
    #[cfg(target_os = "macos")]
    async fn stage(&self, app: &AppHandle, update: Update, bytes: Vec<u8>) {
        let version = update.version.clone();
        if !bundle_is_writable() {
            *lock(&self.pending) = Some((update, bytes));
            self.set(app, UpdateState::Ready { version });
            return;
        }
        match tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await {
            Ok(Ok(())) => {
                log::info!("update {version} installed; a restart runs it");
                self.set(app, UpdateState::Ready { version });
            }
            Ok(Err(e)) => {
                log::warn!("update {version} did not install: {e}");
                self.set(app, UpdateState::Failed { version });
            }
            Err(e) => {
                log::warn!("update {version} did not install: {e}");
                self.set(app, UpdateState::Failed { version });
            }
        }
    }

    /// Running the installer ends the process, so it waits for the restart.
    #[cfg(windows)]
    async fn stage(&self, app: &AppHandle, update: Update, bytes: Vec<u8>) {
        let version = update.version.clone();
        *lock(&self.pending) = Some((update, bytes));
        self.set(app, UpdateState::Ready { version });
    }

    async fn restart(&self, app: &AppHandle) {
        let Ok(_busy) = self.busy.try_lock() else {
            return;
        };
        let UpdateState::Ready { version } = self.state() else {
            return;
        };
        self.set(app, UpdateState::Restarting { version: version.clone() });
        let pending = lock(&self.pending).take();
        if let Some((update, bytes)) = pending {
            // Windows: the installer takes over and this process exits inside
            // `install` (after `on_before_exit`); it only returns on failure.
            // macOS: the swap, with the administrator's prompt it needs.
            match tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    log::warn!("update {version} did not install: {e}");
                    self.set(app, UpdateState::Failed { version });
                    return;
                }
                Err(e) => {
                    log::warn!("update {version} did not install: {e}");
                    self.set(app, UpdateState::Failed { version });
                    return;
                }
            }
        }
        log::info!("restarting into {version}");
        app.request_restart();
    }
}

/// The updater for one check, on the route and TLS setup of every other
/// request (`http.rs`).
fn client(app: &AppHandle) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        .configure_client(|client| {
            client.use_preconfigured_tls(net::tls_client_config()).proxy(http::system_proxy())
        })
        // Windows: the installer is about to replace the app; the tray icon
        // and the taskbar ticker go first, as at quit.
        .on_before_exit(platform::bar::shutdown)
        .build()
}

/// Whether this user can replace the bundle: write access to it and to the
/// folder it sits in (an admin's Applications folder, or ~/Applications).
#[cfg(target_os = "macos")]
fn bundle_is_writable() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(bundle) = exe.ancestors().find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
    else {
        return false;
    };
    let can_write = |dir: &std::path::Path| {
        let probe = dir.join(format!(".candlewick-update-{}", std::process::id()));
        std::fs::create_dir(&probe).is_ok() && std::fs::remove_dir(&probe).is_ok()
    };
    bundle.parent().is_some_and(can_write) && can_write(bundle)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding the lock aborts the process (panic = "abort").
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_only_when_nobody_would_see_it() {
        let display_off = Pause::DisplaySleep as u8;
        let locked = Pause::SessionInactive as u8;
        let asleep = Pause::SystemSleep as u8;
        assert!(unnoticed(display_off, false));
        assert!(unnoticed(locked, false));
        assert!(unnoticed(display_off | locked, false));
        // A window is open, or someone is looking at the screen.
        assert!(!unnoticed(display_off, true));
        assert!(!unnoticed(0, false));
        // Asleep, nothing runs; on waking the display is back on.
        assert!(!unnoticed(asleep, false));
    }

    /// The updater compares the app's version (tauri.conf.json) with the
    /// manifest, and CI bumps it together with Cargo.toml's; a hand edit
    /// that touched only one would make every check look like an update.
    #[test]
    fn the_app_version_is_the_crate_version() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json parses");
        assert_eq!(conf["version"], env!("CARGO_PKG_VERSION"));
    }
}
