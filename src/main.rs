//! Candlewick: live market prices in the macOS menu bar and the Windows
//! taskbar.
//!
//! Idle footprint is one Rust process: the bar item and its dropdown are
//! native (AppKit on macOS; on Windows a tray icon and a ticker drawn into
//! the taskbar), prices arrive over one websocket per market-data provider,
//! and native windows exist only while open.

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
pub mod format;
mod http;
mod market;
mod model;
mod net;
mod platform;
mod portfolio;
mod sign;
pub mod ui;
mod update;
mod window;

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
    let runtime_handle = runtime.handle().clone();
    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    application.on_reopen(|cx| {
        let handle = cx.global::<AppGlobal>().0.clone();
        if let Err(error) = window::reopen(&handle) {
            log::error!("cannot reopen a window: {error}");
        }
    });
    application.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_quit_mode(gpui_kit::QuitMode::Explicit);
        cx.set_activation_policy(gpui_kit::ActivationPolicy::Accessory);
        let config_dir = app::config_dir();
        let settings_path = config_dir.join("settings.json");
        let credentials_path = config_dir.join("credentials.json");
        let settings = model::load(&settings_path);
        i18n::set(settings.language.locale());
        let (shared, control) = Shared::new(
            settings,
            settings_path,
            credentials::load(&credentials_path),
            credentials_path,
        );
        let handle = app::App::new(cx, runtime_handle, shared);
        cx.set_global(AppGlobal(handle.clone()));
        window::init(&handle, cx);
        platform::window::init(&handle, cx);
        agent::sync(&handle);
        let cleanup = handle.clone();
        cx.on_app_quit(move |_| {
            agent::shutdown(&cleanup);
            platform::bar::shutdown();
            async {}
        })
        .detach();

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

        #[cfg(target_os = "macos")]
        platform::bar::install_event_handlers();
        platform::bar::create(&handle).expect("failed to create the bar");
        for provider in ProviderId::ALL {
            let quotes = provider.provider().watch(handle.clone(), control.clone());
            handle.spawn(quotes);
        }
        market::watch_accounts(&handle, &control);
        handle.spawn(market::ths::sync_watchlist(handle.clone(), control.clone()));
        update::start(&handle);

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
    });
}

struct AppGlobal(app::App);
impl gpui_kit::Global for AppGlobal {}

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
