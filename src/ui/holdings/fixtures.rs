//! A realistic portfolio for rendering checks: two exchanges, priced and
//! unpriced assets, costs, dust under 1 USD, and positions near and far
//! from liquidation.

use std::collections::BTreeMap;

use crate::{
    market::ProviderId,
    portfolio::{self, Account, Balance, Position, Price, Wallet},
};

fn prices() -> std::collections::HashMap<String, Price> {
    [
        ("BTC", 84049.95, 82100.0),
        ("ETH", 3412.1, 3450.0),
        ("SOL", 182.33, 178.2),
        ("DOGE", 0.26421, 0.27100),
        ("LINK", 12.534, 12.401),
        ("PEPE", 0.0000121, 0.0000118),
    ]
    .map(|(asset, last, open)| (asset.to_owned(), Price { last, open }))
    .into()
}

/// Every exchange with a key, as the app would keep them after a good read.
pub fn accounts() -> BTreeMap<ProviderId, Account> {
    let prices = prices();
    let binance_balances = [
        Balance::new(Wallet::Spot, "BTC", 0.841),
        Balance::new(Wallet::Spot, "ETH", 4.2),
        Balance::new(Wallet::Spot, "SOL", 18.4),
        Balance::new(Wallet::Funding, "USDT", 12500.0),
        Balance::new(Wallet::Earn, "USDT", 8000.0),
        Balance::new(Wallet::Spot, "DOGE", 42000.0),
        Balance::new(Wallet::Spot, "PEPE", 21000000.0),
        // No USDT pair and the exchange values it itself.
        Balance { usd: Some(7.0), ..Balance::new(Wallet::Funding, "EUR", 6.0) },
        Balance::new(Wallet::Spot, "NOPAIR", 3.0),
    ];
    let binance_positions = [
        Position {
            symbol: "BTCUSDT".to_owned(),
            kind: portfolio::PositionKind::UsdtPerpetual,
            long: true,
            size: 0.25,
            size_unit: Some("BTC".to_owned()),
            entry: 79800.0,
            mark: 84049.95,
            liquidation: Some(71500.0),
            leverage: Some(10.0),
            isolated: false,
            pnl: 1062.49,
            pnl_asset: "USDT".to_owned(),
            exposure: 21012.49,
            day: Some(Price { last: 84049.95, open: 82100.0 }),
        },
        Position {
            symbol: "ETHUSDT".to_owned(),
            kind: portfolio::PositionKind::UsdtPerpetual,
            long: false,
            size: 3.0,
            size_unit: Some("ETH".to_owned()),
            entry: 3300.0,
            mark: 3412.1,
            liquidation: Some(3695.0),
            leverage: Some(5.0),
            isolated: true,
            pnl: -336.3,
            pnl_asset: "USDT".to_owned(),
            exposure: -10236.3,
            day: Some(Price { last: 3412.1, open: 3450.0 }),
        },
    ];
    let okx_balances = [
        Balance {
            cost: Some(79000.0),
            pnl: Some(4200.0),
            ..Balance::new(Wallet::Trading, "BTC", 0.062)
        },
        Balance::new(Wallet::Trading, "LINK", 150.0),
        Balance::new(Wallet::Funding, "USDC", 4300.0),
    ];
    let okx_positions = [Position {
        symbol: "BTC-USD-SWAP".to_owned(),
        kind: portfolio::PositionKind::CoinPerpetual,
        long: true,
        size: 100.0,
        size_unit: Some("USD".to_owned()),
        entry: 81000.0,
        mark: 84049.95,
        liquidation: Some(83600.0),
        leverage: Some(20.0),
        isolated: true,
        pnl: 0.0037,
        pnl_asset: "BTC".to_owned(),
        exposure: 8405.0,
        day: Some(Price { last: 84049.95, open: 82100.0 }),
    }];

    let mut accounts = BTreeMap::new();
    for (exchange, balances, positions) in [
        (ProviderId::Binance, &binance_balances[..], &binance_positions[..]),
        (ProviderId::Okx, &okx_balances[..], &okx_positions[..]),
    ] {
        let holdings = portfolio::value(exchange, balances, positions, &prices);
        accounts.insert(
            exchange,
            Account {
                exchange,
                holdings: Some(holdings),
                updated: Some(portfolio::now_ms() - 4000.0),
                error: None,
                checked: Some(portfolio::now_ms() - 3000.0),
            },
        );
    }
    accounts
}

/// Every UI touched by registered holdings, using isolated local fixture data.
pub fn registered_samples(
    cx: &mut gpui_kit::HeadlessAppContext,
    app: &crate::app::App,
    render: &impl Fn(&mut gpui_kit::HeadlessAppContext, gpui_kit::AnyWindowHandle, &str),
) {
    use crate::{
        api,
        i18n::{self, Locale},
        manual,
        market::{Candidate, Search},
        model::Quote,
        ui,
    };
    use gpui_kit::{AppContext as _, Bounds, WindowBounds, px, size, test::TestWindowExt as _};
    let instrument = |source: ProviderId, symbol: &str, name: &str| {
        let mut instrument = source.provider().bare(symbol).unwrap();
        instrument.name = Some(name.to_owned());
        instrument
    };
    let fund = instrument(ProviderId::Eastmoney, "150.110022", "易方达消费行业股票");
    let gold = instrument(ProviderId::Eastmoney, "118.AU9999", "黄金9999");
    let etf = instrument(ProviderId::Ths, "510300.SH", "沪深300ETF");
    let pending = instrument(ProviderId::Eastmoney, "150.000216", "华安黄金ETF联接A");
    let results = || Search {
        candidates: [
            instrument(ProviderId::Ths, "000506.SZ", "招金黄金"),
            instrument(ProviderId::Ths, "002155.SZ", "湖南黄金"),
            instrument(ProviderId::Ths, "159315.SZ", "黄金股ETF工银"),
            pending.clone(),
            gold.clone(),
        ]
        .into_iter()
        .map(|instrument| Candidate { instrument, manual: false, liquidity: None })
        .collect(),
        notes: Vec::new(),
    };
    let open = |cx: &mut gpui_kit::HeadlessAppContext, width: f32, height: f32| {
        let mut root = None;
        let window = cx.update(|cx| {
            let mut options = crate::window::window_options(cx, "holdings", "Holdings".into());
            options.window_bounds = Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(width), px(height)),
            )));
            options.show = false;
            options.focus = false;
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| super::HoldingsView::new(app.clone(), window, cx));
                root = Some(view.clone());
                ui::testing::backdrop(view, app, cx)
            })
            .unwrap()
            .0
        });
        (window, root.unwrap())
    };
    i18n::set(Locale::ZhCn);
    let (window, _root) = open(cx, 980., 640.);
    for (theme, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render(cx, window, &format!("holdings-empty-{theme}"));
    }
    cx.update_window(window, |_, window, _| window.remove_window()).unwrap();
    let open_manual = |cx: &mut gpui_kit::HeadlessAppContext| {
        let mut manual = None;
        let window = cx.update(|cx| {
            let options = crate::window::window_options(
                cx,
                "manual-holdings",
                t!("holdings.manual.title").to_owned(),
            );
            gpui_kit::open_window(
                gpui_kit::WindowOptions { show: false, focus: false, ..options },
                cx,
                |window, cx| {
                    let view = cx.new(|cx| super::ManualView::new(app.clone(), window, cx));
                    manual = Some(view.clone());
                    ui::testing::backdrop(view, app, cx)
                },
            )
            .unwrap()
            .0
        });
        (window, manual.unwrap())
    };
    let (window, manual_view) = open_manual(cx);
    for (theme, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render(cx, window, &format!("registration-empty-{theme}"));
    }
    let picker = cx.update(|cx| manual_view.read(cx).picker());
    ui::instrument_picker::sample_results(cx, window, &picker, "黄金", results());
    render(cx, window, "registration-search-dark");
    cx.update_window(window, |_, window, _| window.remove_window()).unwrap();

    let mut entries = Vec::new();
    for (instrument, platform, amount, cost) in [
        (fund.clone(), "支付宝", 12000.50, 2.85),
        (gold.clone(), "招商银行", 15.60, 789.20),
        (etf.clone(), "招商证券", 1800., 4.001),
        (pending.clone(), "支付宝", 3000., 2.30),
    ] {
        entries.push(
            api::create_manual_holding(
                app,
                manual::Input { instrument, platform: platform.into(), amount, cost: Some(cost) },
            )
            .unwrap(),
        );
    }
    {
        let mut model = app.shared().model();
        model.accounts = accounts();
        model.settings.currency = crate::currency::Currency::Cny;
        model.exchange_rates.snapshot = Some(crate::currency::Snapshot {
            date: "2026-11-06".into(),
            usd: crate::currency::Rates { cny: 7.12, jpy: 153., hkd: 7.78 },
        });
        for (instrument, last, open) in [
            (fund.clone(), 3.005, 3.003),
            (gold.clone(), 874., 868.57),
            (etf.clone(), 4.231, 4.20),
            (pending.clone(), 2.38, 2.35),
        ] {
            crate::market::record_quote(
                &mut model,
                instrument.id(),
                Quote { last, open, ..Default::default() },
                Some(1_793_894_400_000.),
            );
            let feed = model.feeds.entry(instrument.provider).or_default();
            feed.status = crate::model::Status::Live;
        }
        model.settings.watchlist = vec![fund.clone(), gold.clone(), etf.clone()];
    }
    for (tag, locale) in [("zh", Locale::ZhCn), ("en", Locale::En), ("ja", Locale::Ja)] {
        i18n::set(locale);
        {
            let mut model = app.shared().model();
            crate::market::record_quote(
                &mut model,
                pending.id(),
                Quote { last: 2.38, open: 2.35, ..Default::default() },
                Some(1_793_894_400_000.),
            );
        }
        app.notify(crate::app::Topic::Portfolio);
        for (width, height) in [(980., 640.), (800., 500.)] {
            let (window, _root) = open(cx, width, height);
            for (theme, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                render(cx, window, &format!("holdings-registered-{tag}-{width}-{theme}"));
            }
            cx.update_window(window, |_, window, _| window.remove_window()).unwrap();
        }
        {
            app.shared().model().quotes.remove(&pending.id());
        }
        app.notify(crate::app::Topic::Portfolio);
        let (manager, manual_view) = open_manual(cx);
        for (theme, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(app, dark, cx));
            render(cx, manager, &format!("registration-records-{tag}-460-{theme}"));
        }
        if tag == "zh" {
            cx.update_window(manager, |_, window, cx| {
                window.click(
                    gpui_kit::SharedString::from(format!("manual-delete-{}", entries[0].id)),
                    cx,
                )
            })
            .unwrap();
            render(cx, manager, "registration-delete-dark");
            cx.update_window(manager, |_, window, cx| {
                window.click(
                    gpui_kit::SharedString::from(format!("manual-keep-{}", entries[0].id)),
                    cx,
                )
            })
            .unwrap();
        }
        for (entry, kind) in [(&entries[0], "fund"), (&entries[1], "gold")] {
            cx.update_window(manager, |_, window, cx| {
                manual_view.update(cx, |this, cx| this.begin_edit(entry, window, cx));
            })
            .unwrap();
            for (theme, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                render(cx, manager, &format!("registration-edit-{kind}-{tag}-{theme}"));
            }
        }
        cx.update_window(manager, |_, window, _| window.remove_window()).unwrap();
        // The real settings window includes the watchlist and holdings access rows.
        let mut settings = None;
        let window = cx.update(|cx| {
            let mut options = crate::window::window_options(cx, "settings", "Settings".into());
            options.show = false;
            options.focus = false;
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| ui::settings::SettingsView::new(app.clone(), window, cx));
                settings = Some(view.clone());
                ui::testing::backdrop(view, app, cx)
            })
            .unwrap()
            .0
        });
        let picker = cx.update(|cx| settings.unwrap().read(cx).instrument_picker(cx));
        for (theme, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(app, dark, cx));
            render(cx, window, &format!("settings-registered-{tag}-{theme}"));
        }
        ui::instrument_picker::sample_results(cx, window, &picker, "黄金", results());
        for (theme, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(app, dark, cx));
            render(cx, window, &format!("settings-search-{tag}-{theme}"));
        }
        cx.update_window(window, |_, window, cx| {
            picker.update(cx, |picker, cx| picker.set_selection(None, window, cx));
            window.remove_window();
        })
        .unwrap();
    }
}
