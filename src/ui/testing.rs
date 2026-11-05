//! Offscreen checks for window and widget rendering.

use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};

use crate::{
    api,
    app::App,
    market::longbridge::{Account, MarketAccess},
    model::{ColorScheme, Settings, Shared},
    ui::{self, theme::Palette, widgets},
};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Context, Entity, HeadlessAppContext, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, WindowBounds, WindowOptions,
    assets::AllAssets, div, px, size, test::TestWindowExt,
};

struct Backdrop<V> {
    app: App,
    view: Entity<V>,
}
impl<V: Render> Render for Backdrop<V> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(Palette::current(&self.app, cx).background).child(self.view.clone())
    }
}

fn backdrop<V: Render>(view: Entity<V>, app: &App, cx: &mut gpui_kit::App) -> Entity<Backdrop<V>> {
    cx.new(|_| Backdrop { app: app.clone(), view })
}

struct SectionStage {
    section: Entity<ui::settings::LongbridgeSection>,
}
impl Render for SectionStage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().px_5().pt_3().flex().flex_col().child(self.section.clone())
    }
}

struct Gallery {
    app: App,
}
impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(&self.app, cx);
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .text_color(p.foreground)
            .child(widgets::pill_tabs(
                "tabs",
                [SharedString::from("Day"), SharedString::from("Week")],
                0,
                |_, _, _| {},
            ))
            .child(widgets::panel(
                div()
                    .p_4()
                    .flex()
                    .gap_3()
                    .child(widgets::change_badge("+1.23%", 1.23, p))
                    .child(widgets::change_badge("-1.23%", -1.23, p))
                    .child(widgets::change_badge("0.00%", 0., p)),
                p,
            ))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(widgets::status_dot(widgets::Status::Live, p))
                    .child(widgets::status_dot(widgets::Status::Busy, p))
                    .child(widgets::status_dot(widgets::Status::Error, p))
                    .child(widgets::status_dot(widgets::Status::Stale, p)),
            )
            .child(widgets::stat_tile("Balance", "84,049.95", p))
    }
}

fn render_png(cx: &mut HeadlessAppContext, window: AnyWindowHandle, name: &str) {
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    let image = cx.capture_screenshot(window).expect("offscreen renderer");
    assert!(image.width() > 0 && image.height() > 0);
    if let Some(dir) = std::env::var_os("CANDLEWICK_RENDER_DIR") {
        image.save(Path::new(&dir).join(format!("{name}.png"))).unwrap();
    }
}

fn overlay_sample(cx: &mut HeadlessAppContext, app: &App) {
    use ui::chart::{
        geometry::Frame,
        market::Market,
        model::{Anchor, Drawing},
        model_state::ChartModel,
        overlay::Overlay,
    };

    let id = crate::api::get_chart_instrument(app).unwrap();
    let instrument = crate::api::chart_instrument(app, id.clone()).unwrap();
    let spec = crate::api::chart_spec(app, id).unwrap();
    let interval = spec.intervals[0];
    let step = f64::from(interval.secs);
    let start = 1_700_000_000.;
    let candles: Vec<_> = (0..100)
        .map(|i| {
            let close = 100. + (f64::from(i) / 7.).sin() * 8. + f64::from(i) * 0.1;
            crate::market::Candle {
                time: start + f64::from(i) * step,
                open: close - 1.,
                high: close + 2.,
                low: close - 2.,
                close,
            }
        })
        .collect();
    let mut market = Market::new(interval);
    let load = market.begin_history();
    market.finish_history(load, candles);
    let frame = Frame {
        left: start + step * 25.,
        right: start + step * 100.,
        min: 85.,
        max: 125.,
        plot_x: 20.,
        plot_y: 20.,
        plot_w: 680.,
        plot_h: 360.,
        width: 800.,
        height: 400.,
        now: start + step * 100.,
        candle_width: step,
        hover_x: None,
        scrub: 0.,
        line_mode: 0.,
        hover: None,
    };
    let handle = app.clone();
    let options = cx.update(|cx| {
        let mut options = crate::window::window_options(cx, "chart", "overlay".to_owned());
        options.window_bounds = Some(WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(800.), px(430.)),
        }));
        options.focus = false;
        options.show = false;
        options
    });
    let window = cx.update(|cx| {
        gpui_kit::open_window(options, cx, |_, cx| {
            let mut model = ChartModel::with_market(handle.clone(), Some(instrument), spec, market);
            model.frame.set(Some(frame));
            let a = Anchor { time: start + step * 38., price: 95. };
            let b = Anchor { time: start + step * 56., price: 115. };
            let analysis = model.analysis_mut();
            analysis.drawings = vec![
                Drawing::Level { id: "level".into(), price: 104. },
                Drawing::Trend { id: "trend".into(), a, b },
                Drawing::Fib {
                    id: "fib".into(),
                    a: Anchor { time: start + step * 60., price: 98. },
                    b: Anchor { time: start + step * 80., price: 120. },
                },
                Drawing::Measure {
                    id: "measure".into(),
                    a: Anchor { time: start + step * 30., price: 90. },
                    b: Anchor { time: start + step * 45., price: 100. },
                },
            ];
            analysis.selected = Some("trend".into());
            analysis.set_indicator("ma1", Some(true), None);
            analysis.set_indicator("boll", Some(true), None);
            analysis.pointer_move(
                ui::chart::geometry::Point { x: 500., y: 150. },
                Some(&frame),
                step,
                false,
            );
            let model = cx.new(|_| model);
            let overlay = cx.new(|cx| Overlay::new(model, cx));
            backdrop(overlay, &handle, cx)
        })
        .unwrap()
        .0
    });
    for (name, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render_png(cx, window, &format!("overlay-{name}"));
    }
}

/// The holdings window over a realistic portfolio: both window sizes, both
/// themes, and every language at the sizes its strings must fit.
fn holdings_samples(cx: &mut HeadlessAppContext, app: &App) {
    use crate::i18n::Locale;

    {
        let mut model = app.shared().model();
        model.accounts = crate::ui::holdings::fixtures::accounts();
    }
    app.notify(crate::app::Topic::Portfolio);
    cx.run_until_parked();

    let mut sample_windows = Vec::new();
    for (width, height) in [(980., 640.), (800., 500.)] {
        let mut options = cx.update(|cx| {
            let mut options = crate::window::window_options(cx, "holdings", "holdings".to_owned());
            options.window_bounds = Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: size(px(width), px(height)),
            }));
            options.focus = false;
            options.show = false;
            options
        });
        options.focus = false;
        options.show = false;
        let handle = app.clone();
        let window = cx.update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| ui::holdings::HoldingsView::new(handle.clone(), window, cx));
                backdrop(view, &handle, cx)
            })
            .unwrap()
            .0
        });
        sample_windows.push((width, window));
    }

    for (tag, locale) in [("en", Locale::En), ("zh", Locale::ZhCn), ("ja", Locale::Ja)] {
        cx.update(|_| crate::i18n::set(locale));
        app.notify(crate::app::Topic::Locale);
        cx.run_until_parked();
        for (name, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(app, dark, cx));
            for (width, window) in &sample_windows {
                render_png(cx, *window, &format!("holdings-{width:.0}-{name}-{tag}"));
            }
        }
    }
    cx.update(|_| crate::i18n::set(Locale::En));
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
}

/// What a signed-in Longbridge account may quote.
fn longbridge_account() -> Account {
    Account {
        markets: vec![
            MarketAccess {
                market: "US",
                packages: vec!["LV1 Real-time Quotes".into(), "LV2 Real-time Quotes".into()],
                note: None,
            },
            MarketAccess {
                market: "HK",
                packages: vec!["LV1 Real-time Quotes".into()],
                note: None,
            },
            MarketAccess { market: "CN", packages: Vec::new(), note: None },
        ],
    }
}

fn longbridge_state(state: &str) -> (api::LongbridgeView, bool, Option<SharedString>) {
    let key = Some("LB-8f2c…6d1a".to_owned());
    match state {
        "saved" => {
            (api::LongbridgeView { app_key: key, account: Some(longbridge_account()) }, false, None)
        }
        "checking" => (api::LongbridgeView { app_key: key, account: None }, true, None),
        "error" => (
            api::LongbridgeView { app_key: key, account: None },
            false,
            Some(t!("error.sourceSays", source = "Longbridge", message = "invalid app key").into()),
        ),
        _ => (api::LongbridgeView { app_key: None, account: None }, false, None),
    }
}

/// The Longbridge section alone and inside the settings window: an empty
/// form, saved keys with what the account may quote, a login under way, and
/// a failed login; both themes and every language whose text must fit.
fn longbridge_samples(cx: &mut HeadlessAppContext, app: &App) {
    use crate::i18n::Locale;

    let (section_window, section) = cx.update(|cx| {
        let handle = app.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(460.), px(440.)),
            ))),
            focus: false,
            show: false,
            ..Default::default()
        };
        let mut created = None;
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| {
                ui::settings::LongbridgeSection::with_view(
                    handle.clone(),
                    longbridge_state("empty").0,
                    window,
                    cx,
                )
            });
            created = Some(view.clone());
            backdrop(cx.new(|_| SectionStage { section: view }), &handle, cx)
        })
        .unwrap();
        (window, created.unwrap())
    });

    let (settings_window, settings) = cx.update(|cx| {
        let handle = app.clone();
        let mut options = crate::window::window_options(cx, "settings", "settings".to_owned());
        options.focus = false;
        options.show = false;
        let mut created = None;
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| ui::settings::SettingsView::new(handle.clone(), window, cx));
            created = Some(view.clone());
            backdrop(view, &handle, cx)
        })
        .unwrap();
        (window, created.unwrap())
    });
    let settings_section = cx.update(|cx| settings.read(cx).longbridge());

    for (tag, locale) in [("en", Locale::En), ("ja", Locale::Ja)] {
        cx.update(|_| crate::i18n::set(locale));
        app.notify(crate::app::Topic::Locale);
        cx.run_until_parked();
        for state in ["empty", "saved", "checking", "error"] {
            let (view, checking, error) = longbridge_state(state);
            let (settings_view, settings_checking, settings_error) = longbridge_state(state);
            cx.update(|cx| {
                section.update(cx, |this, cx| this.show(view, checking, error, cx));
                settings_section.update(cx, |this, cx| {
                    this.show(settings_view, settings_checking, settings_error, cx)
                });
                settings.update(cx, |_, cx| cx.notify());
            });
            for (name, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                render_png(cx, section_window, &format!("longbridge-{state}-{name}-{tag}"));
                render_png(
                    cx,
                    settings_window,
                    &format!("settings-longbridge-{state}-{name}-{tag}"),
                );
            }
        }
    }
    cx.update(|_| crate::i18n::set(Locale::En));
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
}

/// The analysis toolbar over the chart: nothing drawn, drawings with an undo
/// step, a tool pressed, and the indicators menu open. Both themes, the menu
/// also in Japanese.
fn chart_toolbar_samples(cx: &mut HeadlessAppContext, app: &App) {
    use crate::ui::chart::{
        ChartView,
        analysis::Tool,
        market::Market,
        model::{Anchor, Drawing, DrawingKind},
        model_state::ChartModel,
    };

    let mut options =
        cx.update(|cx| crate::window::window_options(cx, "chart", "chart".to_owned()));
    options.focus = false;
    options.show = false;
    let handle = app.clone();
    let created = Rc::new(RefCell::new(None));
    let filled = created.clone();
    let window = cx.update(|cx| {
        gpui_kit::open_window(options, cx, |window, cx| {
            let id = crate::api::get_chart_instrument(&handle).unwrap();
            let instrument = crate::api::chart_instrument(&handle, id.clone()).unwrap();
            let spec = crate::api::chart_spec(&handle, id).unwrap();
            let market = Market::new(spec.intervals[0]);
            let chart =
                cx.new(|_| ChartModel::with_market(handle.clone(), Some(instrument), spec, market));
            *filled.borrow_mut() = Some(chart.clone());
            cx.new(|cx| ChartView::with_model(handle.clone(), chart, window, cx))
        })
        .unwrap()
        .0
    });
    let model = created.borrow().clone().expect("the chart model");
    cx.update_window(window, |_, window, _| {
        assert_eq!(window.viewport_size(), size(px(980.), px(640.)));
    })
    .unwrap();

    for (theme, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render_png(cx, window, &format!("chart-toolbar-plain-{theme}"));
    }

    // Two drawings, one of them removed again: an eraser and an undo step.
    let drawings = vec![
        Drawing::Level { id: "level".to_owned(), price: 100.0 },
        Drawing::Trend {
            id: "trend".to_owned(),
            a: Anchor { time: 1_000.0, price: 98.0 },
            b: Anchor { time: 1_300.0, price: 104.0 },
        },
    ];
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            let analysis = model.analysis_mut();
            analysis.set_instrument(Some("sample".to_owned()), drawings);
            analysis.selected = Some("level".to_owned());
            analysis.remove_selected();
            cx.notify();
        });
    });
    for (theme, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render_png(cx, window, &format!("chart-toolbar-drawings-{theme}"));
    }

    // A drawing tool pressed, and an indicator on so the menu trigger lifts.
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            let analysis = model.analysis_mut();
            analysis.set_tool(Tool::Drawing(DrawingKind::Trend));
            analysis.set_indicator("ma1", Some(true), Some(7));
            analysis.take_changed();
            analysis.take_persist();
            cx.notify();
        });
    });
    for (theme, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render_png(cx, window, &format!("chart-toolbar-tool-{theme}"));
    }

    // The trigger opens the menu.
    cx.update_window(window, |_, window, cx| {
        assert!(window.try_find("chart-indicator-menu").is_none());
        window.click("chart-indicators-button", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for (theme, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render_png(cx, window, &format!("chart-toolbar-menu-{theme}"));
    }
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("chart-indicator-menu").is_some(), "the menu opens");
    })
    .unwrap();
    cx.update(|_| crate::i18n::set(crate::i18n::Locale::Ja));
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
    for (theme, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render_png(cx, window, &format!("chart-toolbar-menu-ja-{theme}"));
    }
    cx.update(|_| crate::i18n::set(crate::i18n::Locale::En));
    app.notify(crate::app::Topic::Locale);

    // A tool in the pill reaches the model, and the click away closes the menu.
    cx.update_window(window, |_, window, cx| window.click(("chart-tool", 4usize), cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| model.read(cx).analysis().tool), Tool::Drawing(DrawingKind::Measure));
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("chart-indicator-menu").is_none(), "a click away closes the menu");
    })
    .unwrap();
}

/// A fixed, plausible market: candles, statistics, a grouped book and enough
/// trades to fill the list. No network, no stream.
fn panels_candles(secs: u32) -> Vec<crate::market::Candle> {
    use crate::market::Candle;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs() as f64);
    let end = (now / f64::from(secs)).floor() * f64::from(secs);
    let base = 67_400.0;
    (0..140)
        .map(|i| {
            let open = base + (i as f64 * 0.7).sin() * 140.0;
            let close = base + ((i + 1) as f64 * 0.7).sin() * 140.0;
            Candle {
                time: end - f64::from(secs) * (139 - i) as f64,
                open,
                high: open.max(close) + 12.0 + (i % 7) as f64 * 4.0,
                low: open.min(close) - 10.0 - (i % 5) as f64 * 3.0,
                close,
            }
        })
        .collect()
}

fn panels_book(steps: &[f64], last: f64) -> Vec<crate::market::Book> {
    use crate::market::{Book, Level};
    let steps: Vec<f64> = if steps.is_empty() { vec![1.0] } else { steps.to_vec() };
    steps
        .iter()
        .map(|step| {
            let level = |side: f64, i: usize| Level {
                price: last + side * step * (i as f64 + 0.5),
                qty: 0.004 + ((i * 7 + side as usize) % 17) as f64 * 0.0023,
            };
            Book {
                bids: (0..26).map(|i| level(-1.0, i)).collect(),
                asks: (0..26).map(|i| level(1.0, i)).collect(),
            }
        })
        .collect()
}

fn panels_trades(last: f64, end: f64) -> Vec<crate::market::Trade> {
    use crate::market::Trade;
    let newest = end * 1000.0;
    (0..60)
        .map(|i| Trade {
            id: 9_000 + i,
            price: last + (i as f64 * 1.7).sin() * 36.0,
            qty: 0.002 + (i % 9) as f64 * 0.007,
            time: newest - (59 - i) as f64 * 2_100.0,
            sell: i % 2 == 0,
            extended: false,
        })
        .collect()
}

fn panels_market(spec: &crate::market::ChartSpec) -> ui::chart::market::Market {
    use crate::market::{FeedState, LiveEvent, Onchain, Stats};
    use ui::chart::market::Market;
    let interval = spec
        .intervals
        .iter()
        .copied()
        .find(|interval| interval.secs == 60)
        .unwrap_or(spec.intervals[0]);
    let candles = panels_candles(interval.secs);
    let end = candles.last().map_or(0.0, |candle| candle.time);
    let last = candles.last().map_or(0.0, |candle| candle.close);
    let first = candles.first().map_or(last, |candle| candle.open);
    let high = candles.iter().map(|candle| candle.high).fold(f64::MIN, f64::max);
    let low = candles.iter().map(|candle| candle.low).fold(f64::MAX, f64::min);
    let mut market = Market::new(interval);
    let token = market.begin_history();
    market.finish_history(token, candles);
    market.apply(LiveEvent::State { state: FeedState::Live });
    market.apply(LiveEvent::Stats {
        stats: Stats {
            last,
            high: Some(high),
            low: Some(low),
            volume: Some(18_420.0),
            turnover: 1_246_000_000.0,
            change: last - first,
            change_pct: (last - first) / first * 100.0,
            onchain: spec.onchain.then_some(Onchain {
                liquidity: Some(42_800_000.0),
                market_cap: None,
                fdv: Some(918_000_000.0),
                buys: 7_431,
                sells: 5_208,
            }),
        },
    });
    if spec.book {
        market.apply(LiveEvent::Book { books: panels_book(&spec.book_steps, last) });
    }
    market.apply(LiveEvent::Trades { trades: panels_trades(last, end) });
    market
}

fn panels_model(
    app: &App,
    cx: &mut HeadlessAppContext,
    pool: bool,
    width: f64,
) -> Entity<ui::chart::model_state::ChartModel> {
    use ui::chart::{geometry::Frame, model::Anchor};
    let id = crate::api::get_chart_instrument(app).expect("a chart instrument");
    let instrument = crate::api::chart_instrument(app, id.clone()).expect("the instrument");
    let mut spec = crate::api::chart_spec(app, id).expect("a chart spec");
    if pool {
        spec.book = false;
        spec.onchain = true;
        spec.book_steps.clear();
        spec.volume_unit = None;
        spec.turnover_unit = "USD".to_owned();
    }
    let secs = spec
        .intervals
        .iter()
        .find(|interval| interval.secs == 60)
        .map_or(spec.intervals[0].secs, |interval| interval.secs);
    let market = panels_market(&spec);
    let app = app.clone();
    cx.update(move |cx| {
        cx.new(move |_| {
            let mut model = ui::chart::model_state::ChartModel::with_market(
                app,
                Some(instrument),
                spec,
                market,
            );
            let candles = model
                .market_mut()
                .map(|market| market.chart_data().candles.to_vec())
                .unwrap_or_default();
            if let Some(last) = candles.last().copied() {
                model.analysis_mut().set_candles(candles.clone());
                for id in ["ma1", "ma2", "boll"] {
                    model.analysis_mut().set_indicator(id, Some(true), None);
                }
                // The plot is narrower by the price axis and the window's side
                // column; the card is placed against the plot rectangle the canvas
                // would paint.
                let plot_w = width - 340.0;
                let plot_h = if width >= 900.0 { 360.0 } else { 205.0 };
                let pointer = if width >= 900.0 {
                    (plot_w * 0.45, 120.0)
                } else {
                    (plot_w - 20.0, plot_h - 25.0)
                };
                model.frame.set(Some(Frame {
                    left: candles[0].time,
                    right: last.time + f64::from(secs),
                    min: candles.iter().map(|candle| candle.low).fold(f64::MAX, f64::min),
                    max: candles.iter().map(|candle| candle.high).fold(f64::MIN, f64::max),
                    plot_x: 0.0,
                    plot_y: 0.0,
                    plot_w,
                    plot_h,
                    width: plot_w,
                    height: plot_h,
                    now: last.time + f64::from(secs),
                    candle_width: f64::from(secs),
                    hover_x: Some(pointer.0),
                    scrub: 1.0,
                    line_mode: 0.0,
                    hover: Some(last),
                }));
                model.analysis_mut().pointer =
                    Some(ui::chart::geometry::Point { x: pointer.0, y: pointer.1 });
                model.analysis_mut().snapped =
                    Some(Anchor { time: last.time + f64::from(secs) / 2.0, price: last.close });
            }
            model
        })
    })
}

fn panels_window(
    cx: &mut HeadlessAppContext,
    app: &App,
    model: Entity<ui::chart::model_state::ChartModel>,
    width: f64,
    height: f64,
    title: &str,
) -> AnyWindowHandle {
    let mut options = cx.update(|cx| {
        let mut options = crate::window::window_options(cx, "chart", title.to_owned());
        options.window_bounds = Some(WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(width as f32), px(height as f32)),
        }));
        options.focus = false;
        options.show = false;
        options
    });
    options.focus = false;
    options.show = false;
    let handle = app.clone();
    cx.update(|cx| {
        gpui_kit::open_window(options, cx, |window, cx| {
            let view =
                cx.new(|cx| ui::chart::ChartView::with_model(handle.clone(), model, window, cx));
            backdrop(view, &handle, cx)
        })
        .unwrap()
        .0
    })
}

/// The chart's side panels with a book, the trades tab and a pool's flow
/// header: both window widths, both themes, the two longest languages.
fn panels_samples(cx: &mut HeadlessAppContext, app: &App) {
    use crate::i18n::Locale;
    use ui::prefs::SideTab;

    let original = crate::i18n::current();
    for (width, height) in [(980.0, 640.0), (760.0, 480.0)] {
        let book = panels_model(app, cx, false, width);
        let trades = panels_model(app, cx, false, width);
        let flow = panels_model(app, cx, true, width);
        book.update(cx, |model, cx| model.set_side_tab(SideTab::Book, cx));
        trades.update(cx, |model, cx| model.set_side_tab(SideTab::Trades, cx));
        let book_window = panels_window(cx, app, book, width, height, "panels-book");
        let trades_window = panels_window(cx, app, trades, width, height, "panels-trades");
        let flow_window = panels_window(cx, app, flow, width, height, "panels-flow");
        for (tag, locale) in [("en", Locale::En), ("ja", Locale::Ja)] {
            crate::i18n::set(locale);
            app.notify(crate::app::Topic::Locale);
            cx.run_until_parked();
            for (name, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                render_png(cx, book_window, &format!("panels-{width:.0}-book-{name}-{tag}"));
                render_png(cx, trades_window, &format!("panels-{width:.0}-trades-{name}-{tag}"));
                render_png(cx, flow_window, &format!("panels-{width:.0}-flow-{name}-{tag}"));
            }
        }
    }
    crate::i18n::set(original);
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
}

pub fn run() {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(AllAssets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (shared, _) = Shared::new(
        Settings::default(),
        Default::default(),
        Default::default(),
        Default::default(),
    );
    let app = cx.update(|cx| App::new(cx, runtime.handle().clone(), shared));
    {
        let mut model = app.shared().model();
        model.chart = model.settings.watchlist.first().map(|instrument| instrument.id());
    }
    overlay_sample(&mut cx, &app);
    let mut windows = Vec::new();
    for (kind, width, height) in [
        ("settings", 460., 640.),
        ("chart", 980., 640.),
        ("chart-narrow", 760., 480.),
        ("holdings", 980., 640.),
        ("widgets", 460., 640.),
    ] {
        let handle = app.clone();
        let mut options = cx.update(|cx| {
            if kind == "widgets" {
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(width), px(height)),
                    })),
                    ..Default::default()
                }
            } else if kind == "chart-narrow" {
                let mut options = crate::window::window_options(cx, "chart", kind.to_owned());
                options.window_bounds = Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(width), px(height)),
                }));
                options
            } else {
                crate::window::window_options(cx, kind, kind.to_owned())
            }
        });
        options.focus = false;
        options.show = false;
        let window = cx.update(|cx| match kind {
            "settings" => {
                gpui_kit::open_window(options, cx, |window, cx| {
                    let view =
                        cx.new(|cx| ui::settings::SettingsView::new(handle.clone(), window, cx));
                    backdrop(view, &handle, cx)
                })
                .unwrap()
                .0
            }
            "chart" | "chart-narrow" => {
                gpui_kit::open_window(options, cx, |window, cx| {
                    let id = crate::api::get_chart_instrument(&handle).unwrap();
                    let instrument = crate::api::chart_instrument(&handle, id.clone()).unwrap();
                    let spec = crate::api::chart_spec(&handle, id).unwrap();
                    let mut market = ui::chart::market::Market::new(spec.intervals[0]);
                    market.apply(crate::market::LiveEvent::Stats {
                        stats: crate::market::Stats {
                            last: 84_049.95,
                            high: Some(85_000.),
                            low: Some(82_000.),
                            volume: Some(12_345.6),
                            turnover: 987_654_321.,
                            change: 1_024.5,
                            change_pct: 1.23,
                            onchain: None,
                        },
                    });
                    market.apply(crate::market::LiveEvent::Book {
                        books: vec![crate::market::Book {
                            bids: (1..=4)
                                .map(|i| crate::market::Level {
                                    price: 84_049. - f64::from(i),
                                    qty: 12. * f64::from(i),
                                })
                                .collect(),
                            asks: (1..=4)
                                .map(|i| crate::market::Level {
                                    price: 84_050. + f64::from(i),
                                    qty: 9. * f64::from(i),
                                })
                                .collect(),
                        }],
                    });
                    let model = cx.new(|_| {
                        ui::chart::model_state::ChartModel::with_market(
                            handle.clone(),
                            Some(instrument),
                            spec,
                            market,
                        )
                    });
                    model.update(cx, |model, cx| {
                        model.set_visible_stats(
                            Some(ui::chart::market::VisibleStats {
                                live: true,
                                span: 3600.,
                                from: 0.,
                                to: 0.,
                                change_pct: 1.23,
                                amplitude_pct: 3.68,
                            }),
                            cx,
                        );
                    });
                    let view = cx.new(|cx| {
                        ui::chart::ChartView::with_model(handle.clone(), model, window, cx)
                    });
                    backdrop(view, &handle, cx)
                })
                .unwrap()
                .0
            }
            "holdings" => {
                gpui_kit::open_window(options, cx, |window, cx| {
                    let view =
                        cx.new(|cx| ui::holdings::HoldingsView::new(handle.clone(), window, cx));
                    backdrop(view, &handle, cx)
                })
                .unwrap()
                .0
            }
            _ => {
                gpui_kit::open_window(options, cx, |_, cx| {
                    let view = cx.new(|_| Gallery { app: handle.clone() });
                    backdrop(view, &handle, cx)
                })
                .unwrap()
                .0
            }
        });
        cx.update_window(window, |_, window, _| {
            assert_eq!(window.viewport_size(), size(px(width), px(height)));
        })
        .unwrap();
        windows.push((kind, window));
    }
    chart_toolbar_samples(&mut cx, &app);

    let original = crate::api::get_settings(&app);
    let error = ui::settings::parts::update_settings(&app, |settings| {
        settings.watchlist.push(settings.watchlist[0].clone());
    });
    assert!(error.is_err(), "duplicate watchlist entries must be rejected");
    assert_eq!(crate::api::get_settings(&app), original);

    for (locale_name, locale) in [("en", crate::i18n::Locale::En), ("ja", crate::i18n::Locale::Ja)]
    {
        crate::i18n::set(locale);
        app.notify(crate::app::Topic::Locale);
        cx.run_until_parked();
        for (name, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(&app, dark, cx));
            let settings = windows.iter().find(|(kind, _)| *kind == "settings").unwrap().1;
            render_png(&mut cx, settings, &format!("settings-{locale_name}-{name}"));
        }
    }
    crate::i18n::set(crate::i18n::Locale::En);
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
    for (name, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(&app, dark, cx));
        for (kind, window) in &windows {
            render_png(&mut cx, *window, &format!("{kind}-{name}"));
        }
    }
    let locale = crate::i18n::current();
    let chart_windows: Vec<_> = windows
        .iter()
        .filter(|(kind, _)| kind.starts_with("chart"))
        .map(|(kind, window)| (*kind, *window))
        .collect();
    for (tag, locale) in [("ja", crate::i18n::Locale::Ja), ("zh", crate::i18n::Locale::ZhCn)] {
        crate::i18n::set(locale);
        app.notify(crate::app::Topic::Locale);
        cx.run_until_parked();
        for (name, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(&app, dark, cx));
            for (kind, window) in &chart_windows {
                render_png(&mut cx, *window, &format!("{kind}-{tag}-{name}"));
            }
        }
    }
    crate::i18n::set(locale);
    holdings_samples(&mut cx, &app);
    longbridge_samples(&mut cx, &app);
    keys_samples(&mut cx, &app);
    crate::ui::settings::settings_samples(&mut cx, &app, &render_png);
    panels_samples(&mut cx, &app);
    display_samples(&mut cx, &app);
    app.shared().model().settings.color_scheme = ColorScheme::RedUp;
    app.notify(crate::app::Topic::Settings);
    cx.run_until_parked();
    let active = Palette::new(true, ColorScheme::RedUp);
    cx.update(|cx| {
        assert_eq!(gpui_kit::component::Theme::global(cx).colors.chart_bullish, active.up)
    });
    let normal = Palette::new(false, ColorScheme::GreenUp);
    let reversed = Palette::new(false, ColorScheme::RedUp);
    assert_eq!(normal.up, reversed.down);
    assert_eq!(normal.down, reversed.up);
}

/// The exchange API keys section, alone at the settings width and inside the
/// settings window: with no key, a saved key, a save being confirmed and a
/// refused key, in both themes and both checked languages.
fn keys_samples(cx: &mut HeadlessAppContext, app: &App) {
    use crate::{
        credentials::ApiKey,
        i18n::Locale,
        market::ProviderId,
        ui::settings::{SettingsView, keys::ApiKeysSection},
    };

    type Sample = (AnyWindowHandle, Entity<ApiKeysSection>);

    fn open_section(cx: &mut HeadlessAppContext, app: &App) -> Sample {
        let handle = app.clone();
        let mut section = None;
        let window = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(460.), px(640.)),
                })),
                focus: false,
                show: false,
                ..Default::default()
            };
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| ApiKeysSection::new(handle.clone(), window, cx));
                section = Some(view.clone());
                backdrop(view, &handle, cx)
            })
            .unwrap();
            window
        });
        (window, section.unwrap())
    }

    fn open_settings(cx: &mut HeadlessAppContext, app: &App) -> Sample {
        let handle = app.clone();
        let mut view = None;
        let window = cx.update(|cx| {
            let mut options = crate::window::window_options(cx, "settings", "settings".to_owned());
            options.focus = false;
            options.show = false;
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                let entity = cx.new(|cx| SettingsView::new(handle.clone(), window, cx));
                view = Some(entity.clone());
                backdrop(entity, &handle, cx)
            })
            .unwrap();
            window
        });
        let section = cx.update(|cx| view.unwrap().read(cx).keys());
        (window, section)
    }

    fn state(
        cx: &mut HeadlessAppContext,
        (window, section): &Sample,
        editing: Option<ProviderId>,
        saving: bool,
        problem: Option<&str>,
    ) {
        cx.update_window(*window, |_, window, cx| {
            section.update(cx, |section, cx| {
                section.testing_state(editing, saving, problem.map(SharedString::from), window, cx);
            });
        })
        .unwrap();
    }

    // With no key saved: three "Not set" rows and no holdings rows.
    let mut samples = vec![("keys-none", open_section(cx, app), open_settings(cx, app))];
    // With a key saved: its masked form and the holdings rows appear.
    app.shared().credentials().exchanges.insert(
        ProviderId::Binance,
        ApiKey {
            key: "aV3ryLongSavedKey123456".into(),
            secret: "the-secret".into(),
            passphrase: None,
        },
    );
    samples.push(("keys-saved", open_section(cx, app), open_settings(cx, app)));
    // Editing OKX (three inputs) while the exchange confirms the key.
    let (section, window) = (open_section(cx, app), open_settings(cx, app));
    state(cx, &section, Some(ProviderId::Okx), true, None);
    state(cx, &window, Some(ProviderId::Okx), true, None);
    samples.push(("keys-saving", section, window));
    // Editing Bybit after the exchange refused the key.
    let (section, window) = (open_section(cx, app), open_settings(cx, app));
    let refused = Some("The exchange refused this key.");
    state(cx, &section, Some(ProviderId::Bybit), false, refused);
    state(cx, &window, Some(ProviderId::Bybit), false, refused);
    samples.push(("keys-error", section, window));

    for (tag, locale) in [("en", Locale::En), ("ja", Locale::Ja)] {
        cx.update(|_| crate::i18n::set(locale));
        app.notify(crate::app::Topic::Locale);
        cx.run_until_parked();
        for (name, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(app, dark, cx));
            for (kind, (window, _), (settings, _)) in &samples {
                render_png(cx, *window, &format!("{kind}-{name}-{tag}"));
                render_png(cx, *settings, &format!("settings-{kind}-{name}-{tag}"));
            }
        }
    }
    cx.update(|_| crate::i18n::set(Locale::En));
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
    app.shared().credentials().exchanges.clear();
}

/// The Display section alone and inside the settings window, in both themes
/// and both checked languages.
fn display_samples(cx: &mut HeadlessAppContext, app: &App) {
    crate::ui::settings::display_samples(cx, app, &render_png);
}
