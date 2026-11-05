//! The chart window.

pub mod analysis;
pub mod canvas;
pub mod geometry;
pub mod header;
pub mod indicators;
pub mod labels;
pub mod market;
pub mod model;
pub mod model_state;
pub mod overlay;
pub mod panels;
pub mod toolbar;
pub mod viewport;

use crate::{
    app::{App, Topic},
    ui::{prefs::SideTab, theme::Palette, widgets},
};
use canvas::PriceCanvas;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window,
    component::{Sizable as _, TitleBar},
    div, px,
};
use header::{Hero, InstrumentPicker, LiveBadge, ModeToggle};
use model_state::ChartModel;
use overlay::Overlay;
use panels::{FlowHeader, HoverCard, OrderBookPanel, StatsGrid, TradesPanel};
use toolbar::AnalysisBar;

pub struct ChartView {
    app: App,
    model: Entity<ChartModel>,
    canvas: Entity<PriceCanvas>,
    overlay: Entity<Overlay>,
    hero: Entity<Hero>,
    mode: Entity<ModeToggle>,
    live: Entity<LiveBadge>,
    picker: Entity<InstrumentPicker>,
    order_book: Entity<OrderBookPanel>,
    trades: Entity<TradesPanel>,
    hover: Entity<HoverCard>,
    stats: Entity<StatsGrid>,
    flow: Entity<FlowHeader>,
    toolbar: Entity<AnalysisBar>,
    _observers: Vec<Task<()>>,
}
impl ChartView {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = cx.new(|cx| ChartModel::new(app.clone(), cx));
        Self::with_model(app, model, window, cx)
    }

    /// Build a chart around a prepared model, without starting a feed.
    pub fn with_model(
        app: App,
        model: Entity<ChartModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut observers = super::setup(&app, window, cx);
        observers.push(super::observe_title(&app, window, cx, |view, cx| {
            view.notify_header(cx);
            view.title()
        }));
        observers.push(super::observe(&app, Topic::Settings, cx, |view, cx| {
            view.notify_header(cx);
        }));
        let view = Self {
            app,
            canvas: cx.new(|_| PriceCanvas { model: model.clone() }),
            overlay: cx.new(|cx| Overlay::new(model.clone(), cx)),
            hero: cx.new(|_| Hero { model: model.clone() }),
            mode: cx.new(|_| ModeToggle { model: model.clone() }),
            live: cx.new(|_| LiveBadge { model: model.clone() }),
            picker: cx.new(|_| InstrumentPicker { model: model.clone() }),
            order_book: cx.new(|_| OrderBookPanel::new(model.clone())),
            trades: cx.new(|_| TradesPanel::new(model.clone())),
            hover: cx.new(|_| HoverCard::new(model.clone())),
            stats: cx.new(|_| StatsGrid::new(model.clone())),
            flow: cx.new(|_| FlowHeader::new(model.clone())),
            toolbar: cx.new(|cx| AnalysisBar::new(model.clone(), window, cx)),
            model,
            _observers: observers,
        };
        cx.observe(&view.model, |view, _, cx| {
            view.notify_header(cx);
            cx.notify();
        })
        .detach();
        view
    }

    /// Mark the header's child views for rendering when the model, the
    /// language or the watchlist changes.
    fn notify_header(&self, cx: &mut gpui_kit::App) {
        self.hero.update(cx, |_, cx| cx.notify());
        self.mode.update(cx, |_, cx| cx.notify());
        self.live.update(cx, |_, cx| cx.notify());
        self.picker.update(cx, |_, cx| cx.notify());
    }
    fn title(&self) -> String {
        let model = self.app.shared().model();
        model
            .chart
            .as_deref()
            .and_then(|id| model.instrument(id))
            .map(|instrument| t!("window.chart", pair = instrument.pair_label()))
            .unwrap_or_default()
    }
}
impl Render for ChartView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::current(&self.app, cx);
        let model = self.model.read(cx);
        let has_book = model.spec().is_none_or(|s| s.book);
        let show_book = has_book && model.side_tab() == SideTab::Book;
        let tabs = model.spec().and_then(|spec| {
            let current = model.market()?.interval();
            let intervals: Vec<u32> = spec.intervals.iter().map(|i| i.secs).collect();
            let selected = intervals.iter().position(|secs| *secs == current)?;
            let labels: Vec<SharedString> =
                intervals.iter().map(|secs| interval_label(*secs).into()).collect();
            let entity = self.model.clone();
            Some(
                widgets::pill_tabs("chart-interval", labels, selected, move |index, _, cx| {
                    if let Some(secs) = intervals.get(*index) {
                        entity.update(cx, |model, cx| model.select_interval(*secs, cx));
                    }
                })
                .small(),
            )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(palette.foreground)
            .child(
                TitleBar::new()
                    .bg(gpui_kit::component::Theme::global(cx).transparent)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .child(self.picker.clone()),
                    )
                    .child(
                        div()
                            .w(px(112.))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_end()
                            .child(self.live.clone()),
                    )
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .pr(px(8.))
                            .child(header::link_button(&self.model, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .gap_3()
                            .pb_4()
                            .child(div().h(px(66.)).child(self.hero.clone()))
                            .child(controls_row(
                                tabs,
                                self.toolbar.clone(),
                                self.mode.clone(),
                                window,
                            ))
                            .child(
                                widgets::panel(
                                    div()
                                        .relative()
                                        .size_full()
                                        .child(
                                            div().absolute().inset_0().child(self.canvas.clone()),
                                        )
                                        .child(
                                            div().absolute().inset_0().child(self.overlay.clone()),
                                        )
                                        .child(
                                            div()
                                                .absolute()
                                                .top_0()
                                                .left_0()
                                                .child(self.hover.clone()),
                                        ),
                                    palette,
                                )
                                .mx_4()
                                .flex_1()
                                .min_h_0(),
                            )
                            .child(div().h(px(60.)).px_4().child(self.stats.clone())),
                    )
                    .child(
                        div()
                            .w(px(280.))
                            .flex_shrink_0()
                            .min_h_0()
                            .border_l_1()
                            .border_color(palette.border)
                            .pt_2()
                            .flex()
                            .flex_col()
                            .child(div().flex_shrink_0().px_3().child(if has_book {
                                let selected = usize::from(model.side_tab() == SideTab::Trades);
                                let entity = self.model.clone();
                                div()
                                    .pb(px(10.))
                                    .child(widgets::pill_tabs(
                                        "chart-side-tab",
                                        [t!("chart.book").into(), t!("chart.trades").into()],
                                        selected,
                                        move |index, _, cx| {
                                            let tab = if *index == 0 {
                                                SideTab::Book
                                            } else {
                                                SideTab::Trades
                                            };
                                            entity.update(cx, |model, cx| {
                                                model.set_side_tab(tab, cx)
                                            });
                                        },
                                    ))
                                    .into_any_element()
                            } else {
                                self.flow.clone().into_any_element()
                            }))
                            .child(div().flex_1().min_h_0().child(if show_book {
                                self.order_book.clone().into_any_element()
                            } else {
                                self.trades.clone().into_any_element()
                            })),
                    ),
            )
    }
}

/// The interval, analysis and chart-type controls. At the narrowest window
/// width the analysis controls take a line of their own, so every interval
/// stays visible instead of sliding under an edge.
fn controls_row(
    tabs: Option<gpui_kit::component::tab::TabBar>,
    toolbar: Entity<AnalysisBar>,
    mode: Entity<ModeToggle>,
    window: &Window,
) -> gpui_kit::Div {
    let toolbar = || div().h_full().flex_shrink_0().child(toolbar.clone());
    let mode = || div().w(px(70.)).h_full().flex_shrink_0().child(mode.clone());
    if window.viewport_size().width < px(900.) {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .h(px(28.))
                    .px_4()
                    .flex()
                    .items_center()
                    .child(div().flex_1().min_w_0().children(tabs)),
            )
            .child(
                div()
                    .h(px(28.))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1())
                    .child(toolbar())
                    .child(mode()),
            )
    } else {
        div()
            .h(px(28.))
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().children(tabs))
            .child(toolbar())
            .child(mode())
    }
}

pub(crate) fn interval_label(secs: u32) -> String {
    if secs.is_multiple_of(604_800) {
        t!("interval.week", n = secs / 604_800).to_string()
    } else if secs.is_multiple_of(86_400) {
        t!("interval.day", n = secs / 86_400).to_string()
    } else if secs.is_multiple_of(3_600) {
        t!("interval.hour", n = secs / 3_600).to_string()
    } else if secs.is_multiple_of(60) {
        t!("interval.minute", n = secs / 60).to_string()
    } else {
        t!("interval.second", n = secs).to_string()
    }
}
