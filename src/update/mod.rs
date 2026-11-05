//! Updates, from the latest GitHub release.
//!
//! A little after launch, and every six hours while automatic updates are
//! on, the release's manifest is fetched along the route every other request
//! takes (the system proxy, the platform's certificate check). A newer
//! version is downloaded and its signature checked right away, and `install`
//! puts it in place. On macOS the bundle is swapped on disk there and then,
//! so even the next launch runs it; on Windows the verified installer waits
//! for the restart that runs it. The restart waits until nobody would notice
//! it: no window open, and the display off or the session locked. Whoever
//! wants it sooner has the dropdown's item and the settings' button.
//!
//! Only an installed copy updates itself (`platform::is_installed`): a build
//! run from elsewhere would have its own directory replaced.

mod install;

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use crate::app::{App, Topic};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use cargo_packager_updater::{RemoteRelease, UpdateFormat, semver::Version, url::Url};

use crate::{
    bar, http, net,
    platform::{self, Pause},
};

const ENDPOINT: &str =
    "https://github.com/gtoxlili/candlewick/releases/latest/download/latest.json";
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMwQzE1RjI4RTU5ODI3NjcKUldSbko1amxLRi9CTUZZbjhRaGcxYnZvUzFoemIwRFVIYTlkUDlkVkdzUEVQclhUUFRaM2R2bncK";

/// The feeds and the first render come first.
const FIRST_CHECK: Duration = Duration::from_secs(90);
/// Releases are few; a machine that stays up for weeks still hears of one
/// the same day.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How often a ready update looks for its moment, besides every pause change.
const TICK: Duration = Duration::from_secs(60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// A stalled download must not hold the updater indefinitely.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// What the settings show beside the version.
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
pub struct UpdateView {
    /// The running version.
    pub current: String,
    pub state: UpdateState,
}

/// A release the manifest says is newer, still to be downloaded.
struct Release {
    version: Version,
    download_url: Url,
    signature: String,
}

pub struct Updater {
    state: Mutex<UpdateState>,
    /// Checks and restarts never overlap.
    busy: tokio::sync::Mutex<()>,
    /// Windows: the verified installer. Running it ends the process, so it
    /// waits for a restart.
    #[cfg(windows)]
    pending: Mutex<Option<Vec<u8>>>,
}

/// Starts checking for updates, if this copy is installed.
pub fn start(app: &App) {
    let enabled = !cfg!(debug_assertions) && platform::is_installed();
    let updater = Arc::new(Updater {
        state: Mutex::new(if enabled {
            UpdateState::Idle { checked: false }
        } else {
            UpdateState::Disabled
        }),
        busy: tokio::sync::Mutex::new(()),
        #[cfg(windows)]
        pending: Mutex::new(None),
    });
    app.set_updater(updater.clone());
    if enabled {
        let app = app.clone();
        app.clone().spawn(async move { updater.run(&app).await });
    }
}

pub fn view(app: &App) -> UpdateView {
    UpdateView { current: env!("CARGO_PKG_VERSION").to_owned(), state: state(app) }
}

/// The version a restart would run, if one is ready.
pub fn ready(app: &App) -> Option<String> {
    match state(app) {
        UpdateState::Ready { version } => Some(version),
        _ => None,
    }
}

/// Checks now, whether or not automatic updates are on.
pub fn check_now(app: &App) {
    if let Some(updater) = updater(app) {
        let app = app.clone();
        app.clone().spawn(async move { updater.check(&app).await });
    }
}

/// Restarts into the ready update.
pub fn restart_now(app: &App) {
    if let Some(updater) = updater(app) {
        let app = app.clone();
        app.clone().spawn(async move { updater.restart(&app).await });
    }
}

fn updater(app: &App) -> Option<Arc<Updater>> {
    app.updater()
}

fn state(app: &App) -> UpdateState {
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

    fn set(&self, app: &App, state: UpdateState) {
        let was_ready = matches!(
            std::mem::replace(&mut *lock(&self.state), state.clone()),
            UpdateState::Ready { .. }
        );
        // The dropdown offers a ready update.
        if was_ready != matches!(state, UpdateState::Ready { .. }) {
            bar::request_render(app);
        }
        app.notify(Topic::Update);
    }

    async fn run(&self, app: &App) {
        let mut control = app.shared().control.subscribe();
        tokio::time::sleep(FIRST_CHECK).await;
        let mut next_check = tokio::time::Instant::now();
        loop {
            let automatic = app.shared().model().settings.auto_update;
            if automatic && tokio::time::Instant::now() >= next_check {
                self.check(app).await;
                next_check = tokio::time::Instant::now() + CHECK_EVERY;
            }
            let paused = control.borrow().paused;
            if automatic
                && matches!(self.state(), UpdateState::Ready { .. })
                && unnoticed(paused, app.windows_open())
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

    async fn check(&self, app: &App) {
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
        let found = check().await;
        let release = match found {
            Ok(Some(release)) => release,
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
        let version = release.version.to_string();
        log::info!("update {version} available");
        self.set(app, UpdateState::Downloading { version: version.clone() });
        match download(&release).await {
            Ok(bytes) => self.stage(app, release, bytes).await,
            Err(e) => {
                log::warn!("update {version} did not download: {e}");
                self.set(app, UpdateState::Failed { version });
            }
        }
    }

    /// Replaces the bundle now if its folder can be written to: the running
    /// process keeps its mapped binary, and the next launch is the new one.
    #[cfg(target_os = "macos")]
    async fn stage(&self, app: &App, release: Release, bytes: Vec<u8>) {
        let version = release.version.to_string();
        let Some(bundle) = install::installed_bundle() else {
            log::warn!("update {version} did not install: it is not inside an app bundle");
            self.set(app, UpdateState::Failed { version });
            return;
        };
        match app.spawn_blocking(move || install::install(&bundle, &bytes)).await {
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

    /// The installer ends the process, so it waits for the restart.
    #[cfg(windows)]
    async fn stage(&self, app: &App, release: Release, bytes: Vec<u8>) {
        let version = release.version.to_string();
        *lock(&self.pending) = Some(bytes);
        self.set(app, UpdateState::Ready { version });
    }

    async fn restart(&self, app: &App) {
        let Ok(_busy) = self.busy.try_lock() else {
            return;
        };
        let UpdateState::Ready { version } = self.state() else {
            return;
        };
        self.set(app, UpdateState::Restarting { version: version.clone() });

        #[cfg(windows)]
        {
            let Some(bytes) = lock(&self.pending).take() else {
                log::warn!("update {version} is no longer held in memory");
                self.set(app, UpdateState::Failed { version });
                return;
            };
            // The installer replaces the files of the running app, so this
            // process leaves first; the installer starts the new version.
            crate::agent::shutdown(app);
            platform::bar::shutdown();
            match app.spawn_blocking(move || install::install(&bytes)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    log::warn!("update {version} did not start: {e}");
                    self.set(app, UpdateState::Failed { version });
                    return;
                }
                Err(e) => {
                    log::warn!("update {version} did not start: {e}");
                    self.set(app, UpdateState::Failed { version });
                    return;
                }
            }
            log::info!("update {version} is installing; the installer starts the new version");
            std::process::exit(0);
        }

        #[cfg(target_os = "macos")]
        {
            log::info!("restarting into {version}");
            app.request_restart();
        }
    }
}

type UpdateError = Box<dyn std::error::Error + Send + Sync>;

fn client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .use_preconfigured_tls(net::tls_client_config())
        .proxy(http::system_proxy())
        .user_agent(concat!("Candlewick/", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
        .build()
}

fn newer(release: &RemoteRelease) -> bool {
    release.version > Version::parse(env!("CARGO_PKG_VERSION")).expect("valid package version")
}

async fn check() -> Result<Option<Release>, UpdateError> {
    let response = client(CHECK_TIMEOUT)?.get(ENDPOINT).send().await?.error_for_status()?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    let release: RemoteRelease = response.json().await?;
    if !newer(&release) {
        return Ok(None);
    }
    let target = cargo_packager_updater::target().ok_or("unsupported update target")?;
    let format = release.format(&target)?;
    if !matches!(
        (std::env::consts::OS, format),
        ("macos", UpdateFormat::App) | ("windows", UpdateFormat::Nsis)
    ) {
        return Err("unsupported update format".into());
    }
    Ok(Some(Release {
        version: release.version.clone(),
        download_url: release.download_url(&target)?.clone(),
        signature: release.signature(&target)?.clone(),
    }))
}

async fn download(release: &Release) -> Result<Vec<u8>, UpdateError> {
    let bytes = client(DOWNLOAD_TIMEOUT)?
        .get(release.download_url.clone())
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();
    verify(&bytes, &release.signature, PUBLIC_KEY)?;
    Ok(bytes)
}

fn verify(bytes: &[u8], signature: &str, public_key: &str) -> Result<(), UpdateError> {
    let key = String::from_utf8(STANDARD.decode(public_key)?)?;
    let signature = String::from_utf8(STANDARD.decode(signature)?)?;
    let key = minisign_verify::PublicKey::decode(&key)?;
    let signature = minisign_verify::Signature::decode(&signature)?;
    key.verify(bytes, &signature, true)?;
    Ok(())
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

    #[test]
    fn compares_release_version_to_the_running_package() {
        let current = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        let manifest = |version: &str| -> RemoteRelease {
            serde_json::from_value(serde_json::json!({
                "version": version,
                "platforms": {}
            }))
            .unwrap()
        };
        assert!(!newer(&manifest(&current.to_string())));
        assert!(newer(&manifest(&format!("{}.0.0", current.major + 1))));
        assert!(!newer(&manifest("0.0.0")));
    }

    #[test]
    fn verifies_signed_artifacts_and_rejects_changes() {
        // Upstream minisign-verify's signed "test" vector, in the release feed's encoding.
        let key = "untrusted comment: minisign public key\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n";
        let signature = concat!(
            "untrusted comment: signature from minisign secret key\n",
            "RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n",
            "trusted comment: timestamp:1633700835\tfile:test\tprehashed\n",
            "wLMDjy9FLAuxZ3q4NlEvkgtyhrr0gtTu6KC4KBJdITbbOeAi1zBIYo0v4iTgt8jJpIidRJnp94ABQkJAgAooBQ==\n"
        );
        let key = STANDARD.encode(key);
        let signature = STANDARD.encode(signature);
        assert!(verify(b"test", &signature, &key).is_ok());
        assert!(verify(b"tampered", &signature, &key).is_err());
    }
}
