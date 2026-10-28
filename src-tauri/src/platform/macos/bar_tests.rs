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
                .any(|i| menu.itemAtIndex(i).unwrap().title().to_string() == t!("tray.checkUpdate"))
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
            t!("tray.emptyWatchlist")
        );
    });

    verify_holdings_menu(app);
}

/// An exchange account leads the menu with the total and a submenu of what
/// it holds; both update in place, and the total can take the bar.
fn verify_holdings_menu(app: &AppHandle) {
    use crate::{
        format,
        i18n::{self, Locale},
        market::ProviderId,
        model::Shared,
        portfolio::{self, Balance, Price, Wallet},
    };
    use std::collections::HashMap;
    use tauri::Manager;

    let mtm = MainThreadMarker::new().unwrap();
    let shared = app.state::<Shared>();
    let button = UI
        .with_borrow(|ui| ui.as_ref().unwrap().status_item.as_ref().unwrap().button(mtm).unwrap());
    let holdings = |btc: f64| {
        let prices: HashMap<String, Price> =
            [("BTC".to_owned(), Price { last: btc, open: 80_000.0 })].into();
        portfolio::value(
            ProviderId::Binance,
            &[Balance::new(Wallet::Spot, "BTC", 1.0), Balance::new(Wallet::Spot, "USDT", 500.0)],
            &[],
            &prices,
        )
    };
    shared.model().accounts.insert(
        ProviderId::Binance,
        portfolio::Account {
            holdings: Some(holdings(84_000.0)),
            updated: Some(1_700_000_000_000.0),
            ..portfolio::Account::new(ProviderId::Binance)
        },
    );
    bar::request_render(app);
    let titles = |menu: &NSMenu| -> Vec<String> {
        (0..menu.numberOfItems())
            .map(|i| {
                let item = menu.itemAtIndex(i).unwrap();
                item.attributedTitle()
                    .map_or_else(|| item.title().to_string(), |t| t.string().to_string())
            })
            .collect()
    };
    UI.with_borrow(|ui| {
        let ui = ui.as_ref().unwrap();
        let menu = ui.menu.as_ref().unwrap();
        let total = titles(menu)[0].clone();
        let label = t!("common.holdings.total");
        assert!(total.starts_with(&format!("{label} USDT\t84,500.00\t")), "{total}");
        let submenu = menu.itemAtIndex(0).unwrap().submenu().expect("a holdings submenu");
        let rows = titles(&submenu);
        assert_eq!(rows[0], t!("tray.viewHoldings"));
        let day = format!("{}\t+4,000.00", t!("common.holdings.dayPnl"));
        assert!(rows.iter().any(|r| r.starts_with(&day)), "{rows:?}");
        assert!(rows.iter().any(|r| *r == t!("common.holdings.assets")), "header: {rows:?}");
        assert!(rows.iter().any(|r| r.starts_with("BTC  1\t84,000.00\t+5.00%")), "{rows:?}");
        assert!(rows.iter().any(|r| r.starts_with("USDT  500\t500.00")), "{rows:?}");
        let updated = t!("common.holdings.updated", time = format::clock(1_700_000_000_000.0));
        assert_eq!(rows.last().unwrap(), &updated, "{rows:?}");
    });

    // A price tick updates the open menu's rows without rebuilding it.
    let (menu_before, submenu_before) = UI.with_borrow(|ui| {
        let ui = ui.as_ref().unwrap();
        (
            Retained::as_ptr(ui.menu.as_ref().unwrap()),
            Retained::as_ptr(ui.submenu.as_ref().unwrap()),
        )
    });
    shared.model().accounts.get_mut(&ProviderId::Binance).unwrap().holdings =
        Some(holdings(85_000.0));
    bar::request_render(app);
    UI.with_borrow(|ui| {
        let ui = ui.as_ref().unwrap();
        assert_eq!(Retained::as_ptr(ui.menu.as_ref().unwrap()), menu_before);
        assert_eq!(Retained::as_ptr(ui.submenu.as_ref().unwrap()), submenu_before);
        let rows = titles(ui.submenu.as_ref().unwrap());
        assert!(rows.iter().any(|r| r.starts_with("BTC  1\t85,000.00\t+6.25%")), "{rows:?}");
    });

    // The total takes the bar when asked, with the change in two rows.
    {
        let mut model = shared.model();
        model.settings.holdings_in_bar = true;
        model.settings.show_change = false;
    }
    bar::request_render(app);
    assert_eq!(button.title().to_string(), format!("{} 85,500", t!("common.holdings.total")));
    shared.model().settings.show_change = true;
    bar::request_render(app);
    assert!(button.title().is_empty());
    assert!(button.image().is_some(), "two-row total");

    // Another language rebuilds the menu in it, submenu headers included.
    i18n::set(Locale::Ja);
    bar::request_render(app);
    UI.with_borrow(|ui| {
        let ui = ui.as_ref().unwrap();
        let menu = ui.menu.as_ref().unwrap();
        assert!(titles(menu)[0].starts_with("総資産 USDT\t"), "{:?}", titles(menu));
        assert_eq!(titles(menu).last().unwrap(), "Candlewick を終了");
        let rows = titles(ui.submenu.as_ref().unwrap());
        assert_eq!(rows[0], "保有資産を表示…");
        assert!(rows.iter().any(|r| r == "資産"), "header: {rows:?}");
    });
    i18n::set(Locale::En);
    bar::request_render(app);

    verify_synced_menu(app);
}

/// Signed in to a Tonghuashun account: a row after the watchlist whose
/// submenu is the watchlist there, then a submenu per group of the user's,
/// all with quotes that update in place.
fn verify_synced_menu(app: &AppHandle) {
    use crate::{
        format,
        market::ProviderId,
        model::{Quote, Shared, SyncStatus, Synced, SyncedGroup},
    };
    use tauri::Manager;

    let shared = app.state::<Shared>();
    let moutai = Instrument {
        provider: ProviderId::Ths,
        symbol: "600519.SH".into(),
        base: "600519".into(),
        quote: "CNY".into(),
        name: Some("贵州茅台".into()),
        decimals: Some(2),
        pinned: false,
        venue: None,
    };
    let etf = Instrument {
        symbol: "510300.SH".into(),
        base: "510300".into(),
        name: Some("沪深300ETF".into()),
        decimals: Some(3),
        ..moutai.clone()
    };
    let updated = 1_700_000_000_000.0;
    {
        let mut model = shared.model();
        model.synced = Some(Synced {
            instruments: vec![moutai],
            groups: vec![SyncedGroup { name: "宽基".into(), instruments: vec![etf] }],
            skipped: 2,
            updated: Some(updated),
            status: SyncStatus::Read,
        });
        let quote = |last, open| Quote { last, open, session: None };
        model.quotes.insert("ths:600519.SH".into(), quote(1_331.45, 1_355.9));
        model.quotes.insert("ths:510300.SH".into(), quote(4.156, 4.123));
    }
    bar::request_render(app);
    let titles = |menu: &NSMenu| -> Vec<String> {
        (0..menu.numberOfItems())
            .map(|i| {
                let item = menu.itemAtIndex(i).unwrap();
                item.attributedTitle()
                    .map_or_else(|| item.title().to_string(), |t| t.string().to_string())
            })
            .collect()
    };
    let menus = UI.with_borrow(|ui| {
        let ui = ui.as_ref().unwrap();
        let menu = ui.menu.as_ref().unwrap();
        let index = titles(menu).iter().position(|t| *t == t!("tray.thsWatchlist"));
        let item = menu.itemAtIndex(index.expect("a synced watchlist row") as isize).unwrap();
        let submenu = item.submenu().expect("its submenu");
        assert_eq!(ui.synced_menu.as_ref().map(Retained::as_ptr), Some(Retained::as_ptr(&submenu)));
        let rows = titles(&submenu);
        assert!(rows[0].starts_with("贵州茅台 600519\t1,331.45\t"), "{rows:?}");
        let group = rows.iter().position(|r| r == "宽基").expect("the group's row");
        let group = submenu.itemAtIndex(group as isize).unwrap().submenu().expect("its submenu");
        let members = titles(&group);
        assert!(members[0].starts_with("沪深300ETF 510300\t4.156\t"), "{members:?}");
        let caption = format!(
            "{} · {}",
            t!("common.holdings.updated", time = format::clock(updated)),
            t!("tray.thsSkipped", count = 2)
        );
        assert_eq!(rows.last().unwrap(), &caption, "{rows:?}");
        (Retained::as_ptr(&submenu), Retained::as_ptr(&group))
    });

    // Quotes move the rows, a group's too, without rebuilding the menu.
    {
        let mut model = shared.model();
        let quote = |last, open| Quote { last, open, session: None };
        model.quotes.insert("ths:600519.SH".into(), quote(1_340.0, 1_355.9));
        model.quotes.insert("ths:510300.SH".into(), quote(4.2, 4.123));
    }
    bar::request_render(app);
    UI.with_borrow(|ui| {
        let ui = ui.as_ref().unwrap();
        let synced = ui.synced_menu.as_ref().unwrap();
        assert_eq!((Retained::as_ptr(synced), Retained::as_ptr(&ui.synced_groups[0])), menus);
        let rows = titles(synced);
        assert!(rows[0].starts_with("贵州茅台 600519\t1,340.00\t"), "{rows:?}");
        let members = titles(&ui.synced_groups[0]);
        assert!(members[0].starts_with("沪深300ETF 510300\t4.200\t"), "{members:?}");
    });

    shared.model().synced = None;
    bar::request_render(app);
    UI.with_borrow(|ui| assert!(ui.as_ref().unwrap().synced_menu.is_none()));
}
