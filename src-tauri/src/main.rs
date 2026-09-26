//! Candlewick: live market prices in the macOS menu bar.
//!
//! Idle footprint is one Rust process: the status item and its dropdown are
//! native AppKit, prices arrive over one websocket per market-data provider,
//! and the webviews only exist while their windows are open.

mod commands;
mod credentials;
mod format;
mod http;
mod macos;
mod market;
mod model;
mod net;
mod ticker;
mod tray;
mod window;

use tauri::{Manager, RunEvent, WindowEvent};

use crate::{market::ProviderId, model::Shared};

fn main() {
    #[cfg(debug_assertions)]
    init_debug_logger();

    // A few websockets need one worker thread, not one per core.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("candlewick-io")
        .enable_all()
        .build()
        .expect("failed to start the tokio runtime");
    tauri::async_runtime::set(runtime.handle().clone());

    tauri::Builder::default()
        .menu(window::app_menu)
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_status,
            commands::get_login_item,
            commands::set_login_item,
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
        ])
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let settings_path = config_dir.join("settings.json");
            let credentials_path = config_dir.join("credentials.json");
            let (shared, control) = Shared::new(
                model::load(&settings_path),
                settings_path,
                credentials::load(&credentials_path),
                credentials_path,
            );
            app.manage(shared);

            tray::create(app.handle())?;

            let handle = app.handle().clone();
            macos::observe_pauses(move |reason, paused| {
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

            for provider in ProviderId::ALL {
                let quotes = provider.provider().watch(app.handle().clone(), control.clone());
                tauri::async_runtime::spawn(quotes);
            }

            // Meant to run all the time: register as a login item the first
            // time it runs from an install location, once. The marker keeps a
            // later "off" (in settings or System Settings) from being undone.
            let marker = config_dir.join("login-item-registered");
            if macos::is_installed() && !marker.exists() {
                if let Err(e) = macos::set_login_item(true) {
                    log::warn!("cannot register login item: {e}");
                }
                if let Err(e) =
                    std::fs::create_dir_all(&config_dir).and_then(|()| std::fs::write(&marker, b""))
                {
                    log::warn!("cannot write {}: {e}", marker.display());
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Destroyed) {
                window::on_destroyed(window.app_handle(), window.label());
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the app")
        .run(|app, event| match event {
            // Closing the settings window must not quit the menu bar app.
            // Explicit quits (tray menu, Cmd-Q) carry an exit code or bypass this.
            RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            RunEvent::Reopen { .. } => {
                if let Err(e) = window::reopen(app) {
                    log::error!("cannot reopen a window: {e}");
                }
            }
            _ => {}
        });
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
