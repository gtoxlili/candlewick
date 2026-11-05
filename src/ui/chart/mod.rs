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
    app::App,
    ui::{prefs::SideTab, theme::Palette, widgets},
};
use canvas::PriceCanvas;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window, div, px,
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
        observers.push(super::observe_title(&app, window, cx, |view, _| view.title()));
        cx.observe(&model, |_, _, cx| cx.notify()).detach();
        Self {
            app,
            canvas: cx.new(|_| PriceCanvas { model: model.clone() }),
            overlay: cx.new(|cx| Overlay::new(model.clone(), cx)),
            hero: cx.new(|_| Hero { model: model.clone() }),
            mode: cx.new(|_| ModeToggle { model: model.clone() }),
            live: cx.new(|_| LiveBadge { model: model.clone() }),
            picker: cx.new(|_| InstrumentPicker { model: model.clone() }),
            order_book: cx.new(|_| OrderBookPanel { model: model.clone() }),
            trades: cx.new(|_| TradesPanel { model: model.clone() }),
            hover: cx.new(|_| HoverCard { model: model.clone() }),
            stats: cx.new(|_| StatsGrid { model: model.clone() }),
            flow: cx.new(|_| FlowHeader { model: model.clone() }),
            toolbar: cx.new(|_| AnalysisBar { model: model.clone() }),
            model,
            _observers: observers,
        }
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            Some(widgets::pill_tabs("chart-interval", labels, selected, move |index, _, cx| {
                if let Some(secs) = intervals.get(*index) {
                    entity.update(cx, |model, cx| model.select_interval(*secs, cx));
                }
            }))
        });
        super::frame(self.title(), cx)
            .child(
                div()
                    .h(px(30.))
                    .px_4()
                    .flex()
                    .items_center()
                    .child(div().flex_1().h_full().child(self.picker.clone()))
                    .child(div().w(px(100.)).h_full().child(self.live.clone())),
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
                            .child(div().h(px(74.)).px_5().pt_2().child(self.hero.clone()))
                            .child(
                                div()
                                    .h(px(28.))
                                    .px_4()
                                    .flex()
                                    .items_center()
                                    .child(div().flex_1().min_w_0().children(tabs))
                                    .child(div().w(px(150.)).h_full().child(self.toolbar.clone()))
                                    .child(div().w(px(70.)).h_full().child(self.mode.clone())),
                            )
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
                            .child(div().h(px(32.)).px_3().child(if has_book {
                                div().into_any_element()
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

fn interval_label(secs: u32) -> String {
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
