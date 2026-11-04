//! Native regression test; no feeds, credentials, settings writes or windows.
#![allow(dead_code, unused_imports)]

// First, so `t!` reaches every module after it.
#[cfg(target_os = "macos")]
#[macro_use]
#[path = "../src/i18n.rs"]
mod i18n;
#[cfg(target_os = "macos")]
#[path = "../src/agent/mod.rs"]
mod agent;
#[cfg(target_os = "macos")]
#[path = "../src/app.rs"]
mod app;
#[cfg(target_os = "macos")]
#[path = "../src/bar.rs"]
mod bar;
#[cfg(target_os = "macos")]
#[path = "../src/calendar.rs"]
mod calendar;
#[cfg(target_os = "macos")]
#[path = "../src/credentials.rs"]
mod credentials;
#[cfg(target_os = "macos")]
#[path = "../src/format.rs"]
mod format;
#[cfg(target_os = "macos")]
#[path = "../src/http.rs"]
mod http;
#[cfg(target_os = "macos")]
#[path = "../src/market/mod.rs"]
mod market;
#[cfg(target_os = "macos")]
#[path = "../src/model.rs"]
mod model;
#[cfg(target_os = "macos")]
#[path = "../src/net.rs"]
mod net;
#[cfg(target_os = "macos")]
#[path = "../src/platform/mod.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "../src/portfolio.rs"]
mod portfolio;
#[cfg(target_os = "macos")]
#[path = "../src/sign.rs"]
mod sign;
#[cfg(target_os = "macos")]
#[path = "../src/update.rs"]
mod update;
#[cfg(target_os = "macos")]
#[path = "../src/window.rs"]
mod window;

#[cfg(target_os = "macos")]
fn main() {
    use tauri::Manager;

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let runtime_handle = runtime.handle().clone();
    tauri::Builder::default()
        .setup(move |app| {
            let (shared, _) = model::Shared::new(
                model::Settings::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            );
            let handle = app::App::new(app.handle().clone(), runtime_handle.clone(), shared);
            app.manage(handle.clone());
            verify_topics(&handle);
            platform::bar::create(&handle)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build native regression app")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                platform::bar::tests::verify_closed_menu_refresh(&app.state::<app::App>());
                println!("macos_bar: prices, settings and menu rows refresh without a click");
                app.exit(0);
            }
        });
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn verify_topics(app: &app::App) {
    use app::Topic;

    let topics = [
        Topic::Settings,
        Topic::Locale,
        Topic::Portfolio,
        Topic::Ths,
        Topic::Update,
        Topic::ChartInstrument,
    ];
    for topic in topics {
        app.notify(topic);
        let mut receiver = app.subscribe(topic);
        assert_eq!(*receiver.borrow_and_update(), 1);
        app.clone().notify(topic);
        assert!(receiver.has_changed().unwrap());
        assert_eq!(*receiver.borrow_and_update(), 2);
    }
}
