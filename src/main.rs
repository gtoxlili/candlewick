//! Candlewick: live market prices in the macOS menu bar and the Windows
//! taskbar.
//!
//! Idle footprint is one Rust process: the bar item and its dropdown are
//! native (AppKit on macOS; on Windows a tray icon and a ticker drawn into
//! the taskbar), prices arrive over one websocket per market-data provider,
//! and the webviews only exist while their windows are open.

// A release build is a GUI app on Windows: no console window beside it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// First, so `t!` reaches every module after it.
#[macro_use]
mod i18n;

mod agent;
pub mod api;
pub mod app;
mod bar;
mod calendar;
mod credentials;
mod format;
mod http;
mod market;
mod model;
mod net;
mod platform;
mod portfolio;
mod sign;
mod ui;
mod update;
mod window;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

use crate::{market::ProviderId, model::Shared};

fn main() {
    // Started as thsdk's helper: serve it, and nothing else of the app.
    if let Some(code) = market::ths::serve_if_asked() {
        std::process::exit(code);
    }

    #[cfg(debug_assertions)]
    init_debug_logger();

    platform::claim_single_instance();

    // A few websockets need one worker thread, not one per core.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("candlewick-io")
        .enable_all()
        .build()
        .expect("failed to start the tokio runtime");
    tauri::async_runtime::set(runtime.handle().clone());
    let runtime_handle = runtime.handle().clone();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            let config_dir = app::config_dir();
            let settings_path = config_dir.join("settings.json");
            let credentials_path = config_dir.join("credentials.json");
            let settings = model::load(&settings_path);
            // Before anything says a word: the bar, the menus, the first window.
            i18n::set(settings.language.locale());
            let (shared, control) = Shared::new(
                settings,
                settings_path,
                credentials::load(&credentials_path),
                credentials_path,
            );
            let handle = app::App::new(app.handle().clone(), runtime_handle.clone(), shared);
            app.manage(handle.clone());
            agent::sync(&handle);

            // Before the bar: Windows reports the display's state as soon as
            // the bar's thread asks for it.
            let pause_handle = handle.clone();
            platform::observe_pauses(move |reason, paused| {
                let bit = reason as u8;
                pause_handle.shared().control.send_if_modified(|control| {
                    let before = control.paused;
                    if paused {
                        control.paused |= bit;
                    } else {
                        control.paused &= !bit;
                    }
                    control.paused != before
                });
            });

            platform::bar::create(&handle)?;

            for provider in ProviderId::ALL {
                let quotes = provider.provider().watch(handle.clone(), control.clone());
                handle.spawn(quotes);
            }
            market::watch_accounts(&handle, &control);
            handle.spawn(market::ths::sync_watchlist(handle.clone(), control.clone()));

            update::start(&handle);

            // Meant to run all the time: register as a login item the first
            // time it runs from an install location, once. The marker keeps a
            // later "off" (in settings or the system's) from being undone.
            let marker = config_dir.join("login-item-registered");
            if platform::is_installed() && !marker.exists() {
                if let Err(e) = platform::set_login_item(true) {
                    log::warn!("cannot register login item: {e}");
                }
                if let Err(e) =
                    std::fs::create_dir_all(&config_dir).and_then(|()| std::fs::write(&marker, b""))
                {
                    log::warn!("cannot write {}: {e}", marker.display());
                }
                if platform::SETTINGS_ON_FIRST_RUN
                    && let Err(e) = window::open_settings(&handle)
                {
                    log::error!("cannot open settings: {e}");
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Resized(_) => {
                platform::window::resized(&window.app_handle().state::<app::App>(), window.label())
            }
            WindowEvent::Destroyed => {
                window::on_destroyed(&window.app_handle().state::<app::App>(), window.label())
            }
            _ => {}
        });

    platform::configure(builder)
        .build(tauri::generate_context!())
        .expect("failed to build the app")
        .run(on_event);
}

fn on_event(app: &AppHandle, event: RunEvent) {
    match event {
        // Closing the last window must not quit the bar app. Explicit quits
        // (the dropdown, Cmd-Q) carry an exit code or bypass this.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::Exit => {
            agent::shutdown(&app.state::<app::App>());
            platform::bar::shutdown();
        }
        // Only macOS reports reopening through the event loop.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Err(e) = window::reopen(&app.state::<app::App>()) {
                log::error!("cannot reopen a window: {e}");
            }
        }
        _ => {}
    }
}

#[cfg(debug_assertions)]
fn init_debug_logger() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.target().starts_with("candlewick")
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                eprintln!("[{}] {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    let _ = log::set_logger(&LOGGER);
    let level = std::env::var("CANDLEWICK_LOG").ok().and_then(|l| l.parse().ok());
    log::set_max_level(level.unwrap_or(log::LevelFilter::Debug));
}
