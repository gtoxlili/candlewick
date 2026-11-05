//! Offscreen checks for window and widget rendering.

use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};

use crate::{
    app::App,
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
