//! Repro from the 0.7.0 macOS tray freeze, 2026-09-27.

use super::*;

pub(crate) fn verify_closed_menu_refresh(app: &AppHandle) {
    use crate::model::{Quote, Shared};
    use tauri::Manager;

    // Repro: 0.7.0 froze prices and display settings until the tray was
    // clicked. All assertions inspect AppKit while its menu is detached.
    let mtm = MainThreadMarker::new().unwrap();
    let shared = app.state::<Shared>();
    let button = UI.with_borrow(|ui| {
        // UX contract: the update entry exists before any update is found.
        let menu = ui.as_ref().unwrap().menu.as_ref().unwrap();
        assert!(
            (0..menu.numberOfItems())
                .any(|i| { menu.itemAtIndex(i).unwrap().title().to_string() == "检查更新…" })
        );
        let item = ui.as_ref().unwrap().status_item.as_ref().unwrap();
        assert!(item.menu(mtm).is_none(), "exercise the closed dropdown");
        item.button(mtm).unwrap()
    });
    for (price, expected) in [(84_001.0, "BTC 84,001"), (84_002.0, "BTC 84,002")] {
        shared
            .model()
            .quotes
            .insert("binance:BTCUSDT".into(), Quote { last: price, open: 84_000.0, session: None });
        bar::request_render(app);
        assert_eq!(button.title().to_string(), expected);
    }
    UI.with_borrow(|ui| {
        let menu = ui.as_ref().unwrap().menu.as_ref().unwrap();
        let row = menu.itemAtIndex(0).unwrap().attributedTitle().unwrap().string().to_string();
        assert!(row.contains("84,002.00"), "closed menu must already hold the new quote: {row}");
    });

    shared.model().settings.show_symbol = false;
    bar::request_render(app);
    assert_eq!(button.title().to_string(), "84,002");

    shared.model().settings.show_change = true;
    bar::request_render(app);
    assert!(button.title().is_empty());
    let two_rows = button.image().expect("two-row ticker image");
    shared.model().settings.color_scheme = ColorScheme::RedUp;
    bar::request_render(app);
    let recolored = button.image().unwrap();
    assert_ne!(Retained::as_ptr(&two_rows), Retained::as_ptr(&recolored));

    {
        let mut model = shared.model();
        model.settings.show_change = false;
        model.settings.show_symbol = true;
        model.settings.watchlist[0].pinned = false;
        model.settings.watchlist[1].pinned = true;
        model.quotes.insert(
            "binance:ETHUSDT".into(),
            Quote { last: 3_001.0, open: 3_000.0, session: None },
        );
    }
    bar::request_render(app);
    assert_eq!(button.title().to_string(), "ETH 3,001");
    assert!(button.image().is_none());

    shared.model().settings.watchlist.clear();
    bar::request_render(app);
    assert!(button.title().is_empty());
    assert!(button.image().is_some(), "no pinned instrument shows the template icon");
    UI.with_borrow(|ui| {
        let ui = ui.as_ref().unwrap();
        assert!(ui.status_item.as_ref().unwrap().menu(mtm).is_none());
        assert_eq!(
            ui.menu.as_ref().unwrap().itemAtIndex(0).unwrap().title().to_string(),
            "在设置中添加自选"
        );
    });
}
