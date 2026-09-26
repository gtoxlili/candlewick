//! Coin Tray: live Binance spot prices in the macOS menu bar.
//!
//! Idle footprint is one Rust process: the status item and its dropdown are
//! native AppKit, prices arrive over a single websocket, and the settings
//! webview only exists while its window is open.

mod commands;
mod feed;
mod format;
mod macos;
mod model;
mod net;
mod ticker;
mod tray;
mod window;

use tauri::{Manager, RunEvent, WindowEvent};

use crate::model::Shared;

fn main() {
    #[cfg(debug_assertions)]
    init_debug_logger();

    // One websocket needs one worker thread, not one per core.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("coin-tray-io")
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
            commands::get_chart_symbol,
            commands::open_chart,
            commands::open_in_binance,
        ])
        .setup(|app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let settings_path = config_dir.join("settings.json");
            let (shared, control) = Shared::new(model::load(&settings_path), settings_path);
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

            feed::spawn(app.handle().clone(), control);

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
            metadata.target().starts_with("coin_tray")
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
    let level = std::env::var("COIN_TRAY_LOG").ok().and_then(|l| l.parse().ok());
    log::set_max_level(level.unwrap_or(log::LevelFilter::Debug));
}
