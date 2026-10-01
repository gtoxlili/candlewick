//! Native regression test; no feeds, credentials, settings writes or windows.
#![allow(dead_code, unused_imports)]

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

    tauri::Builder::default()
        .setup(|app| {
            let (shared, _) = model::Shared::new(
                model::Settings::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            );
            app.manage(shared);
            platform::bar::create(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build native regression app")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                platform::bar::tests::verify_closed_menu_refresh(app);
                println!("macos_bar: prices, settings and menu rows refresh without a click");
                app.exit(0);
            }
        });
}

#[cfg(not(target_os = "macos"))]
fn main() {}
