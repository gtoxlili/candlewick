//! What the native regression test (`tests/macos_bar.rs`) runs on the main
//! thread: AppKit must be driven from it, not from a libtest worker.

/// Renders the windows headlessly, then checks that the topics notify and that
/// the menu-bar item refreshes without a click. Nothing here touches the real
/// configuration directory, feeds or credentials.
#[cfg(target_os = "macos")]
pub fn run_native_checks() {
    use crate::{app, model, platform, ui};

    let config = std::env::temp_dir().join(format!("candlewick-test-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    // SAFETY: no other thread exists yet.
    unsafe { std::env::set_var("CANDLEWICK_CONFIG_DIR", &config) };
    ui::testing::run();
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

#[cfg(target_os = "macos")]
fn verify_topics(app: &crate::app::App) {
    use crate::app::Topic;

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

/// Nothing to check off macOS.
#[cfg(not(target_os = "macos"))]
pub fn run_native_checks() {}
