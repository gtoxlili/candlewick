//! Offscreen checks for window and widget rendering.

use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};

use crate::{
    api,
    app::App,
    market::longbridge::{Account, MarketAccess},
    model::{ColorScheme, Settings, Shared},
    ui::{self, assets::Assets, theme::Palette, widgets},
};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Context, Entity, HeadlessAppContext, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, WindowBounds, WindowOptions, div, px,
    size, test::TestWindowExt,
};

pub(crate) struct Backdrop<V> {
    app: App,
    view: Entity<V>,
}
impl<V: Render> Render for Backdrop<V> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(Palette::current(&self.app, cx).background).child(self.view.clone())
    }
}

pub(crate) fn backdrop<V: Render>(
    view: Entity<V>,
    app: &App,
    cx: &mut gpui_kit::App,
) -> Entity<Backdrop<V>> {
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
            turnover: Some(1_246_000_000.0),
            source_time: None,
            change: Some(last - first),
            change_pct: Some((last - first) / first * 100.0),
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

/// A market with a long enough history for a chart, a forming candle, and
/// nothing that reaches the network.
fn live_market(
    spec: &crate::market::ChartSpec,
) -> (ui::chart::market::Market, Vec<crate::market::Candle>) {
    use crate::market::{Candle, FeedState, LiveEvent, Stats};
    use ui::chart::market::Market;

    let interval = spec
        .intervals
        .iter()
        .copied()
        .find(|interval| interval.secs == 60)
        .unwrap_or(spec.intervals[0]);
    let secs = f64::from(interval.secs);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    let end = (now / secs).floor() * secs;
    let candles: Vec<Candle> = (0..300)
        .map(|i| {
            let i = i as f64;
            let open = 84_000.0 + (i * 0.11).sin() * 140.0 + (i * 0.031).cos() * 30.0;
            let close =
                84_000.0 + ((i + 1.0) * 0.11).sin() * 140.0 + ((i + 1.0) * 0.031).cos() * 30.0;
            Candle {
                time: end - secs * (300.0 - i),
                open,
                high: open.max(close) + 25.0 + (i % 5.0) * 6.0,
                low: open.min(close) - 20.0 - (i % 3.0) * 5.0,
                close,
            }
        })
        .collect();
    let mut market = Market::new(interval);
    let token = market.begin_history();
    market.finish_history(token, candles.clone());
    market.apply(LiveEvent::State { state: FeedState::Live });
    let last = candles.last().map_or(0.0, |candle| candle.close);
    let first = candles.first().map_or(last, |candle| candle.open);
    market.apply(LiveEvent::Stats {
        stats: Stats {
            last,
            high: Some(last + 180.0),
            low: Some(last - 160.0),
            volume: Some(12.0),
            turnover: Some(1.0e9),
            source_time: None,
            change: Some(last - first),
            change_pct: Some((last - first) / first * 100.0),
            onchain: None,
        },
    });
    if spec.book {
        market.apply(LiveEvent::Book { books: panels_book(&spec.book_steps, last) });
    }
    market.apply(LiveEvent::Trades { trades: panels_trades(last, end) });
    // The forming candle, opened by the first tick of its bucket.
    market.apply(LiveEvent::Tick { price: last + 12.0, time: (end + 1.0) * 1000.0 });
    (market, candles)
}

fn chart_live_model(
    cx: &mut HeadlessAppContext,
    app: &App,
) -> (
    Entity<ui::chart::model_state::ChartModel>,
    Vec<crate::market::Candle>,
    crate::market::ChartSpec,
) {
    let id = crate::api::get_chart_instrument(app).expect("a chart instrument");
    let instrument = crate::api::chart_instrument(app, id.clone()).expect("the instrument");
    let spec = crate::api::chart_spec(app, id).expect("a chart spec");
    let (market, candles) = live_market(&spec);
    let kept = spec.clone();
    let handle = app.clone();
    let model = cx.update(move |cx| {
        cx.new(move |_| {
            ui::chart::model_state::ChartModel::with_market(handle, Some(instrument), spec, market)
        })
    });
    (model, candles, kept)
}

fn chart_live_window(
    cx: &mut HeadlessAppContext,
    app: &App,
    width: f64,
    height: f64,
    model: Entity<ui::chart::model_state::ChartModel>,
) -> AnyWindowHandle {
    let handle = app.clone();
    let mut options = cx.update(|cx| {
        let mut options = crate::window::window_options(cx, "chart", "chart-live".to_owned());
        options.window_bounds = Some(WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(width as f32), px(height as f32)),
        }));
        options
    });
    options.focus = false;
    options.show = false;
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

/// Draw one frame and deliver the frame requests it made. Returns how many
/// animation frames the chart asked for.
fn chart_frame(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> usize {
    // One frame of animation time: the chart measures how long it has been
    // since it last advanced, and the offscreen renderer draws instantly.
    cx.advance_clock(std::time::Duration::from_millis(16));
    cx.update_window(window, |_, window, cx| {
        window.draw(cx).clear(cx);
        window.simulate_next_frame(cx)
    })
    .unwrap()
}

/// Draw frames until the chart stops asking for more. Returns whether it
/// settled within `limit` frames.
fn chart_settle(cx: &mut HeadlessAppContext, window: AnyWindowHandle, limit: usize) -> bool {
    for _ in 0..limit {
        let pending = chart_frame(cx, window);
        cx.run_until_parked();
        if pending == 0 {
            return true;
        }
    }
    false
}

fn chart_png(cx: &mut HeadlessAppContext, window: AnyWindowHandle, name: &str) {
    // The offscreen renderer refreshes the window, which lays every view out
    // again; settle first so the captured frame is the one the chart paints.
    chart_settle(cx, window, 400);
    render_png(cx, window, name);
}

/// Where the overlay painted the price tag of the level drawing, in window
/// pixels. Tags are the only 18-pixel-tall quads drawn in the drawing colour.
fn level_tag_y(window: &Window, scale: f32, color: gpui_kit::Hsla) -> Option<f64> {
    let background = gpui_kit::solid_background(color);
    window
        .painted_quads()
        .iter()
        .filter(|quad| quad.background == background)
        .filter(|quad| (quad.bounds.size.height.0 - 18.0 * scale).abs() < 0.5)
        .map(|quad| f64::from(quad.bounds.origin.y.0))
        .next()
}

/// The largest quad painted in `color`, in CSS pixels from the window's
/// origin: the hover card's background.
fn card_quad(window: &Window, color: gpui_kit::Hsla) -> Option<(f64, f64, f64, f64)> {
    let background = gpui_kit::solid_background(color);
    window
        .painted_quads()
        .iter()
        .filter(|quad| quad.background == background)
        .max_by_key(|quad| quad.bounds.size.width.0 as i64 * quad.bounds.size.height.0 as i64)
        .map(|quad| {
            let scale = f64::from(window.scale_factor());
            let bounds = quad.bounds;
            (
                f64::from(bounds.origin.x.0) / scale,
                f64::from(bounds.origin.y.0) / scale,
                f64::from(bounds.size.width.0) / scale,
                f64::from(bounds.size.height.0) / scale,
            )
        })
}

/// The hover card reads the frame the canvas writes: a frame whose plot is
/// narrower flips the card to the other side of the pointer, and the card is
/// repainted when the canvas notifies it of that change.
fn hover_frame_sample(cx: &mut HeadlessAppContext, app: &App) {
    use ui::chart::{geometry::Frame, model::Anchor, panels::HoverCard};

    let (model, candles, spec) = chart_live_model(cx, app);
    let secs = f64::from(spec.intervals[0].secs);
    let last = candles.last().copied().expect("candles");
    let pointer = ui::chart::geometry::Point { x: 200.0, y: 40.0 };
    let frame = |plot_w: f64| Frame {
        left: last.time - secs * 60.0,
        right: last.time + secs,
        min: 83_000.0,
        max: 85_000.0,
        plot_x: 16.0,
        plot_y: 16.0,
        plot_w,
        plot_h: 300.0,
        width: plot_w + 80.0,
        height: 348.0,
        now: last.time + secs,
        candle_width: secs,
        hover_x: Some(pointer.x),
        scrub: 1.0,
        line_mode: 0.0,
        hover: Some(last),
    };
    model.update(cx, |model, cx| {
        model.frame.set(Some(frame(600.0)));
        model.analysis_mut().pointer = Some(pointer);
        model.analysis_mut().snapped =
            Some(Anchor { time: last.time + secs / 2.0, price: last.close });
        cx.notify();
    });

    let handle = app.clone();
    let shaped = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Default::default(),
                size: size(px(800.), px(400.)),
            })),
            focus: false,
            show: false,
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            let card = cx.new(|_| HoverCard::new(model.clone()));
            backdrop(card, &handle, cx)
        })
        .unwrap()
        .0
    });
    let palette = cx.update(|cx| ui::theme::Palette::current(app, cx));
    render_png(cx, shaped, "chart-hover-frame-wide");
    let wide = cx
        .update_window(shaped, |_, window, _| card_quad(window, palette.popover))
        .unwrap()
        .expect("the card is painted");
    assert!(wide.2 >= 176.0, "the card is at least its minimum width");
    // The card is placed against the plot the frame describes and the pointer
    // in the same coordinates the overlay reports it in.
    let pointer_x = pointer.x;
    assert!(
        (wide.0 - (pointer_x + 14.0)).abs() < 1.0,
        "the card sits beside the pointer: {wide:?}"
    );

    // A narrower plot no longer leaves room to the pointer's right.
    model.update(cx, |model, cx| {
        model.frame.set(Some(frame(300.0)));
        cx.notify();
    });
    cx.update_window(shaped, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
    let narrow = cx
        .update_window(shaped, |_, window, _| card_quad(window, palette.popover))
        .unwrap()
        .expect("the card is painted");
    assert!(
        (narrow.0 - (pointer_x - 14.0 - narrow.2).max(6.0)).abs() < 1.0,
        "the card flips to the pointer's other side: {narrow:?}"
    );
}

/// The chart window as a running window: a live feed of ticks, a candle
/// rollover, an older history page, a pan, a zoom, a trend line drawn with the
/// tool hotkey, an indicator switched on, the frame requests a settled chart
/// makes, and the overlay and hover card following the frame the canvas wrote.
fn chart_live_samples(cx: &mut HeadlessAppContext, app: &App) {
    use crate::market::LiveEvent;
    use ui::chart::{
        analysis::Tool,
        geometry::mapping,
        model::{Anchor, Drawing},
        model_state::ChartModel,
    };

    let original = crate::i18n::current();
    let (model, candles, spec) = chart_live_model(cx, app);
    let window = chart_live_window(cx, app, 980.0, 640.0, model.clone());
    assert!(chart_settle(cx, window, 400), "the chart settles after loading");

    // A settled chart asks for no frames at all.
    assert_eq!(chart_frame(cx, window), 0, "a chart at rest requests no frames");

    // A window that changes size lays the chart out again at the new size.
    let wide = cx.update(|cx| model.read(cx).frame.get()).expect("a painted frame").plot_w;
    cx.update_window(window, |_, window, cx| {
        window.resize(gpui_kit::size(px(760.), px(480.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    assert!(chart_settle(cx, window, 400));
    let narrow = cx.update(|cx| model.read(cx).frame.get()).expect("a painted frame").plot_w;
    assert!(narrow < wide - 100.0, "the plot follows the window: {wide} -> {narrow}");
    cx.update_window(window, |_, window, cx| {
        window.resize(gpui_kit::size(px(980.), px(640.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    assert!(chart_settle(cx, window, 400));

    // The overlay paints on the right axis the price of the level drawn here;
    // its position is the one the frame the canvas wrote maps.
    let a = cx.update(|cx| model.read(cx).frame.get()).expect("a painted frame");
    // Near the bottom of the range: a rolling range moves it visibly.
    let level = a.min + (a.max - a.min) * 0.05;
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            model.analysis_mut().drawings =
                vec![Drawing::Level { id: "level".to_owned(), price: level }];
            cx.notify();
        });
    });
    chart_frame(cx, window);

    // The forming candle eases toward a tick, and the frame moves with it.
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |elapsed| elapsed.as_secs_f64());
            // A spike far above the range: the eased range has to follow it
            // over several frames.
            let spike = a.max + (a.max - a.min) * 4.0;
            model.apply_event(LiveEvent::Tick { price: spike, time: now * 1000.0 }, cx);
        });
    });
    assert!(chart_frame(cx, window) >= 1, "a tick starts the chart's animation");
    let frame_a = cx.update(|cx| model.read(cx).frame.get()).expect("a painted frame");
    let scale = cx.update_window(window, |_, window, _| window.scale_factor()).unwrap();
    let color = cx.update(|cx| ui::theme::Palette::current(app, cx).violet);
    let painted_a = cx
        .update_window(window, |_, window, _| level_tag_y(window, scale, color))
        .unwrap()
        .expect("the level tag is painted");
    // The next frames run on the chart's own requests; nothing notifies the
    // model between them, so only the frame it writes can move the overlay.
    for _ in 0..5 {
        assert!(chart_frame(cx, window) >= 1, "the chart animates without the model");
    }
    let frame_b = cx.update(|cx| model.read(cx).frame.get()).expect("a painted frame");
    assert_ne!(frame_a, frame_b, "the canvas keeps writing new frames");
    let painted_b = cx
        .update_window(window, |_, window, _| level_tag_y(window, scale, color))
        .unwrap()
        .expect("the level tag is painted");
    let moved = mapping(&frame_b).y(level) - mapping(&frame_a).y(level);
    assert!(moved.abs() > 2.0, "the frame moved far enough to see ({moved})");
    assert!(
        (painted_b - painted_a - f64::from(scale) * moved).abs() < f64::from(scale),
        "the overlay follows the frame it is notified of: painted {} vs {}",
        painted_b - painted_a,
        f64::from(scale) * moved
    );

    // A candle rollover and an older page leave the chart on its live edge and
    // painting.
    let end = candles.last().map_or(0.0, |candle| candle.time);
    let secs = f64::from(spec.intervals[0].secs);
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            model.apply_event(
                LiveEvent::Tick { price: 90_000.0, time: (end + 2.0 * secs) * 1000.0 },
                cx,
            );
        });
    });
    assert!(chart_settle(cx, window, 400));
    let older: Vec<_> = (0..60)
        .map(|i| crate::market::Candle {
            time: candles[0].time - secs * (60.0 - i as f64),
            open: 83_000.0,
            high: 83_200.0,
            low: 82_800.0,
            close: 83_100.0,
        })
        .collect();
    let accepted = cx.update(|cx| {
        model.update(cx, |model, cx| {
            let page = model.begin_older().expect("a page to page in");
            let accepted = model.accept_history(model.generation(), page.load, older, true);
            model.viewport_changed(cx);
            accepted
        })
    });
    assert!(accepted, "the older page is accepted");
    assert!(chart_settle(cx, window, 400));

    // Pointer gestures: a pan leaves the live edge, a zoom changes the span,
    // and the double click returns to the live edge at the default zoom.
    let overlay = cx.update_window(window, |_, window, _| window.find("Overlay")).unwrap();
    let bounds = overlay.bounds();
    let at = |x: f32, y: f32| bounds.origin + gpui_kit::point(px(x), px(y));
    let mid = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    cx.update_window(window, |_, window, cx| {
        // Dragging the content to the right moves the window into history.
        window.drag(at(mid.0 * 0.35, mid.1 * 0.4), at(mid.0 * 0.65, mid.1 * 0.4), cx);
    })
    .unwrap();
    assert!(chart_settle(cx, window, 400));
    let panned = cx.update(|cx| model.read(cx).viewport().view.end.is_some());
    assert!(panned, "the drag pans the chart into history");
    let visible = cx.update(|cx| model.read(cx).visible_stats());
    assert!(visible.is_some(), "the window reports the stretch it shows");
    assert!(!visible.unwrap().live, "and it is no longer the live stretch");
    let span_before = cx.update(|cx| model.read(cx).viewport().view.span);
    cx.update_window(window, |_, window, cx| {
        // A wheel turned down, as AppKit reports it: a negative delta.
        window.scroll(
            "Overlay",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-40.))),
            cx,
        );
    })
    .unwrap();
    assert!(chart_settle(cx, window, 400));
    let span_after = cx.update(|cx| model.read(cx).viewport().view.span);
    assert!(span_after > span_before, "the wheel zooms out: {span_before} -> {span_after}");

    // A double click returns to live and the default zoom. The chart ignores
    // one within half a second of a drag, as a double click after flinging the
    // view would otherwise undo the fling.
    std::thread::sleep(std::time::Duration::from_millis(520));
    cx.update_window(window, |_, window, cx| {
        window.double_click("Overlay", cx);
    })
    .unwrap();
    assert!(chart_settle(cx, window, 400));
    let (live, span) = cx.update(|cx| {
        let viewport = model.read(cx).viewport();
        (viewport.live(), viewport.view.span)
    });
    assert!(live, "a double click follows the live edge again");
    assert!((span - 60.0 * 60.0).abs() < 1.0, "the default span: {span}");

    // The tool hotkey reaches the model through the window's key context, and
    // two clicks draw a trend line.
    cx.update_window(window, |_, window, cx| window.press("alt-t", cx)).unwrap();
    assert_eq!(
        cx.update(|cx| model.read(cx).analysis().tool),
        Tool::Drawing(ui::chart::model::DrawingKind::Trend)
    );
    let before = cx.update(|cx| model.read(cx).analysis().drawings.len());
    cx.update_window(window, |_, window, cx| {
        window.click_at("Overlay", gpui_kit::point(px(mid.0 * 0.4), px(mid.1 * 0.45)), cx);
        window.click_at("Overlay", gpui_kit::point(px(mid.0 * 0.6), px(mid.1 * 0.3)), cx);
    })
    .unwrap();
    assert!(chart_settle(cx, window, 400));
    assert_eq!(
        cx.update(|cx| model.read(cx).analysis().drawings.len()),
        before + 1,
        "the trend line is drawn"
    );
    // Escape leaves the tool, and the keyboard shortcut removes the drawing.
    cx.update_window(window, |_, window, cx| window.press("escape", cx)).unwrap();
    assert_eq!(cx.update(|cx| model.read(cx).analysis().tool), Tool::Cursor);
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            // Selecting by id picks the drawing the clicks made.
            let last = model.analysis().drawings.last().map(|drawing| drawing.id().to_owned());
            model.analysis_mut().selected = last;
            cx.notify();
        });
    });
    cx.update_window(window, |_, window, cx| window.press("backspace", cx)).unwrap();
    assert_eq!(
        cx.update(|cx| model.read(cx).analysis().drawings.len()),
        before,
        "delete removes the selected drawing"
    );
    cx.update_window(window, |_, window, cx| window.press("cmd-z", cx)).unwrap();
    assert_eq!(
        cx.update(|cx| model.read(cx).analysis().drawings.len()),
        before + 1,
        "undo brings it back"
    );

    // An indicator switched on from the menu.
    cx.update_window(window, |_, window, cx| window.click("chart-indicators-button", cx)).unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.click("chart-indicator-ma1", cx)).unwrap();
    cx.run_until_parked();
    assert!(
        cx.update(|cx| model.read(cx).analysis().indicators.iter().any(|s| s.id == "ma1" && s.on)),
        "the menu switches the indicator on"
    );
    assert!(chart_settle(cx, window, 400));
    assert_eq!(chart_frame(cx, window), 0, "still at rest after all of that");

    // What the window looks like in both themes and both chart types, at both
    // sizes the window manager allows. Fresh models: no spike, no drawings,
    // no open menu.
    for (width, height, size) in [(980.0, 640.0, "980"), (760.0, 480.0, "760")] {
        let (shot, _, _) = chart_live_model(cx, app);
        let shot_window = chart_live_window(cx, app, width, height, shot.clone());
        assert!(chart_settle(cx, shot_window, 400));
        for (name, dark) in [("light", false), ("dark", true)] {
            cx.update(|cx| ui::theme::apply(app, dark, cx));
            chart_png(cx, shot_window, &format!("chart-live-{size}-candle-{name}"));
        }
        if size == "980" {
            // A hovered candle: the crosshair, its tags and the card against
            // the candles they point at.
            let (frame, candle) = cx.update(|cx| {
                let model = shot.read(cx);
                let frame = model.frame.get().expect("a painted frame");
                let middle = (frame.left + frame.right) / 2.0;
                let candle = model
                    .analysis()
                    .candles()
                    .iter()
                    .copied()
                    .find(|candle| candle.time + frame.candle_width >= middle)
                    .expect("a candle under the pointer");
                (frame, candle)
            });
            let map = mapping(&frame);
            let pointer = ui::chart::geometry::Point {
                x: map.x(candle.time + frame.candle_width / 2.0),
                y: map.y(candle.close),
            };
            cx.update(|cx| {
                shot.update(cx, |model, cx| {
                    model.analysis_mut().pointer = Some(pointer);
                    model.analysis_mut().snapped = Some(Anchor {
                        time: candle.time + frame.candle_width / 2.0,
                        price: candle.close,
                    });
                    cx.notify();
                });
            });
            for (name, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                chart_png(cx, shot_window, &format!("chart-live-{size}-hover-{name}"));
            }
            cx.update(|cx| {
                shot.update(cx, |model, cx| {
                    model.analysis_mut().pointer = None;
                    model.analysis_mut().snapped = None;
                    model.set_mode(ui::prefs::Mode::Line, cx);
                });
            });
            assert!(chart_settle(cx, shot_window, 400));
            for (name, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                chart_png(cx, shot_window, &format!("chart-live-{size}-line-{name}"));
            }
        }
    }
    // A symbol whose history loaded empty: the chart says so instead of
    // painting an empty plot.
    let id = crate::api::get_chart_instrument(app).expect("a chart instrument");
    let instrument = crate::api::chart_instrument(app, id.clone()).expect("the instrument");
    let spec = crate::api::chart_spec(app, id).expect("a chart spec");
    let mut market = ui::chart::market::Market::new(spec.intervals[0]);
    let token = market.begin_history();
    market.finish_history(token, Vec::new());
    let handle = app.clone();
    let empty = cx.update(move |cx| {
        cx.new(move |_| ChartModel::with_market(handle, Some(instrument), spec, market))
    });
    let empty_window = chart_live_window(cx, app, 980.0, 640.0, empty);
    for (name, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        chart_png(cx, empty_window, &format!("chart-live-empty-{name}"));
    }
    crate::i18n::set(original);
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
}

pub fn run() {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let dir = std::env::temp_dir().join(format!(
        "candlewick-asset-overviews-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let (shared, _) = Shared::new(
        Settings::default(),
        dir.join("settings.json"),
        Default::default(),
        dir.join("credentials.json"),
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
                            turnover: Some(987_654_321.),
                            source_time: None,
                            change: Some(1_024.5),
                            change_pct: Some(1.23),
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
    hover_frame_sample(&mut cx, &app);
    chart_live_samples(&mut cx, &app);
    display_samples(&mut cx, &app);
    picker_sample(&mut cx, &app, &windows);
    asset_overview_samples(&mut cx, &app);
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
    let _ = std::fs::remove_dir_all(&dir);
}

/// The chart's instrument picker open, in both themes: its list sits close to
/// the popover's edge, and the selected row is the one on show.
fn picker_sample(cx: &mut HeadlessAppContext, app: &App, windows: &[(&str, AnyWindowHandle)]) {
    let chart = windows.iter().find(|(kind, _)| *kind == "chart").unwrap().1;
    cx.update_window(chart, |_, window, cx| {
        assert!(window.try_find("chart-instrument-list").is_none());
        window.click("chart-instrument-trigger", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for (name, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(app, dark, cx));
        render_png(cx, chart, &format!("chart-picker-{name}"));
    }
    cx.update_window(chart, |_, window, _| {
        assert!(window.try_find("chart-instrument-list").is_some(), "the picker opens");
    })
    .unwrap();
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

/// The asset overview's real render and input boundary. Prepared source samples
/// keep the check offline; no provider workers or production settings are opened.
fn asset_overview_samples(cx: &mut HeadlessAppContext, app: &App) {
    use crate::market::{Candle, FeedState, LiveEvent, ProviderId, Stats};
    use ui::chart::{market::Market, model_state::ChartModel};
    let locale = crate::i18n::current();
    crate::i18n::set(crate::i18n::Locale::ZhCn);
    for (symbol, label) in [("150.110022", "fund"), ("118.AU9999", "gold")] {
        for (width, height) in [(980., 640.), (760., 480.)] {
            let nav = label == "fund";
            let provider = ProviderId::Eastmoney.provider();
            let mut instrument = provider.bare(symbol).unwrap();
            instrument.name = Some(if nav { "易方达消费行业股票" } else { "黄金9999" }.into());
            let spec = provider.chart_spec(&instrument);
            let mut market = Market::new(spec.intervals[0]);
            let last_time = 1_793_894_400.; // 2026-11-06 Shanghai midnight.
            let points = (0..400)
                .map(|index| {
                    let value = if nav { 3.005 } else { 874. };
                    let close = value
                        * (1. - f64::from(399 - index) * 0.0003
                            + ((f64::from(index) / 15.).sin() - (399_f64 / 15.).sin()) * 0.01);
                    Candle {
                        time: last_time - f64::from(399 - index) * 86400.,
                        open: if nav { close } else { close - 2. },
                        high: if nav { close } else { close + 3. },
                        low: if nav { close } else { close - 4. },
                        close,
                    }
                })
                .collect();
            let load = market.begin_history();
            market.finish_history(load, points);
            let older = market.begin_older().unwrap();
            market.finish_older(older.load, Vec::new());
            market.apply(LiveEvent::Stats {
                stats: Stats {
                    last: if nav { 3.005 } else { 874. },
                    high: (!nav).then_some(874.),
                    low: (!nav).then_some(865.),
                    volume: None,
                    turnover: (!nav).then_some(343430928.),
                    source_time: Some(last_time * 1000.),
                    change: Some(if nav { 0.002 } else { 5.43 }),
                    change_pct: Some(if nav { 0.0666 } else { 0.625 }),
                    onchain: None,
                },
            });
            market.apply(LiveEvent::State { state: FeedState::Live });
            let model = cx.update(|cx| {
                cx.new(|_| ChartModel::with_market(app.clone(), Some(instrument), spec, market))
            });
            cx.update(|cx| model.update(cx, |model, cx| model.select_overview_range(90, cx)));
            let window = chart_live_window(cx, app, width, height, model.clone());
            for (theme, dark) in [("light", false), ("dark", true)] {
                cx.update(|cx| ui::theme::apply(app, dark, cx));
                assert!(chart_settle(cx, window, 400));
                chart_png(cx, window, &format!("overview-{label}-{width}-{theme}"));
                cx.update_window(window, |_, window, _| {
                    assert!(window.try_find("asset-overview").is_some());
                    assert!(window.try_find("chart-indicators-button").is_none());
                    assert!(window.try_find("chart-mode-candle").is_some() != nav);
                })
                .unwrap();
            }
            // Real keyboard routing cannot enable hidden drawing tools.
            cx.update_window(window, |_, window, cx| window.press("alt-t", cx)).unwrap();
            assert_eq!(
                cx.update(|cx| model.read(cx).analysis().tool),
                ui::chart::analysis::Tool::Cursor
            );
            cx.update(|cx| {
                model.update(cx, |model, cx| {
                    model.select_overview_range(365, cx);
                    assert_eq!(model.viewport().view.span, 365. * 86400.);
                    assert_eq!(model.chart_now(last_time + 4. * 86400.), last_time + 86400.);
                })
            });
            if !nav {
                cx.update_window(window, |_, window, cx| window.click("chart-mode-candle", cx))
                    .unwrap();
                assert_eq!(cx.update(|cx| model.read(cx).mode()), ui::prefs::Mode::Candle);
                chart_png(cx, window, &format!("overview-gold-{width}-candles"));
            }
            cx.update_window(window, |_, window, _| window.remove_window()).unwrap();
        }
    }
    ui::holdings::fixtures::registered_samples(cx, app, &render_png);
    crate::i18n::set(locale);
}
