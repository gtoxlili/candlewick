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
mod bar;
mod calendar;
mod commands;
mod credentials;
mod format;
mod http;
mod market;
mod model;
mod net;
mod platform;
mod portfolio;
mod sign;
mod update;
mod window;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

use crate::{market::ProviderId, model::Shared};

fn main() {
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

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_locale,
            commands::get_login_item,
            commands::set_login_item,
            commands::get_update,
            commands::check_update,
            commands::restart_to_update,
            commands::window_ready,
            commands::search_instruments,
            commands::get_chart_instrument,
            commands::open_chart,
            commands::chart_spec,
            commands::chart_history,
            commands::chart_trades,
            commands::chart_stream,
            commands::chart_stream_stop,
            commands::open_link,
            commands::get_longbridge,
            commands::set_longbridge,
            commands::check_longbridge,
            commands::get_exchange_keys,
            commands::set_exchange_key,
            commands::get_portfolio,
            commands::open_holdings,
            commands::open_settings,
            commands::get_agent,
        ])
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
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
            app.manage(shared);
            app.manage(agent::Agent::default());
            agent::sync(app.handle());

            // Before the bar: Windows reports the display's state as soon as
            // the bar's thread asks for it.
            let handle = app.handle().clone();
            platform::observe_pauses(move |reason, paused| {
                let bit = reason as u8;
                handle.state::<Shared>().control.send_if_modified(|control| {
                    let before = control.paused;
                    if paused {
                        control.paused |= bit;
                    } else {
                        control.paused &= !bit;
                    }
                    control.paused != before
                });
            });

            platform::bar::create(app.handle())?;

            for provider in ProviderId::ALL {
                let quotes = provider.provider().watch(app.handle().clone(), control.clone());
                tauri::async_runtime::spawn(quotes);
            }
            market::watch_accounts(app.handle(), &control);

            update::start(app.handle());

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
                    && let Err(e) = window::open_settings(app.handle())
                {
                    log::error!("cannot open settings: {e}");
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Resized(_) => {
                platform::window::resized(window.app_handle(), window.label())
            }
            WindowEvent::Destroyed => window::on_destroyed(window.app_handle(), window.label()),
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
            agent::shutdown(app);
            platform::bar::shutdown();
        }
        // Only macOS reports reopening through the event loop.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Err(e) = window::reopen(app) {
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
