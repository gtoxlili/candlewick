//! Offscreen checks for window and widget rendering.

use std::{path::Path, sync::Arc};

use crate::{
    app::App,
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
