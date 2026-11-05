//! Offscreen checks for window and widget rendering.

use std::{path::Path, sync::Arc};

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
    assets::Assets, div, px, size, test::TestWindowExt,
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

pub fn run() {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(Assets),
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
            "chart" => {
                gpui_kit::open_window(options, cx, |window, cx| {
                    let id = crate::api::get_chart_instrument(&handle).unwrap();
                    let instrument = crate::api::chart_instrument(&handle, id.clone()).unwrap();
                    let spec = crate::api::chart_spec(&handle, id).unwrap();
                    let market = ui::chart::market::Market::new(spec.intervals[0]);
                    let model = cx.new(|_| {
                        ui::chart::model_state::ChartModel::with_market(
                            handle.clone(),
                            Some(instrument),
                            spec,
                            market,
                        )
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
    crate::i18n::set(crate::i18n::Locale::Ja);
    app.notify(crate::app::Topic::Locale);
    cx.run_until_parked();
    let chart = windows.iter().find(|(kind, _)| *kind == "chart").unwrap().1;
    for (name, dark) in [("light", false), ("dark", true)] {
        cx.update(|cx| ui::theme::apply(&app, dark, cx));
        render_png(&mut cx, chart, &format!("chart-ja-{name}"));
    }
    crate::i18n::set(locale);
    holdings_samples(&mut cx, &app);
    longbridge_samples(&mut cx, &app);
    keys_samples(&mut cx, &app);
    crate::ui::settings::settings_samples(&mut cx, &app, &render_png);
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
