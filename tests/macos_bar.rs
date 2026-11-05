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
#[path = "../src/api.rs"]
mod api;
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
#[path = "../src/ui/mod.rs"]
mod ui;
#[cfg(target_os = "macos")]
#[path = "../src/update/mod.rs"]
mod update;
#[cfg(target_os = "macos")]
#[path = "../src/window.rs"]
mod window;

#[cfg(target_os = "macos")]
#[path = "../src/ui/testing.rs"]
mod ui_testing;

#[cfg(target_os = "macos")]
fn main() {
    // Nothing here may touch the real configuration directory.
    let config = std::env::temp_dir().join(format!("candlewick-test-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    // SAFETY: no other thread exists yet.
    unsafe { std::env::set_var("CANDLEWICK_CONFIG_DIR", &config) };
    ui_testing::run();
    platform::bar::install_event_handlers();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let runtime_handle = runtime.handle().clone();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let (shared, _) = model::Shared::new(
            model::Settings::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        let handle = app::App::new(cx, runtime_handle, shared);
        verify_topics(&handle);
        platform::bar::create(&handle).unwrap();
        platform::bar::tests::verify_closed_menu_refresh(&handle);
        println!("macos_bar: prices, settings and menu rows refresh without a click");
        platform::bar::shutdown();
        cx.quit();
    });
    let _ = std::fs::remove_dir_all(&config);
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
