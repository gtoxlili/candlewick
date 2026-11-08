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
    market::Presentation,
    ui::{prefs::SideTab, theme::Palette, widgets},
};
use canvas::PriceCanvas;
use gpui_kit::{
    AppContext, Context, Entity, Global, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement, Render, SharedString, Styled, Task, TestSupportExt as _, Window,
    component::{Sizable as _, TitleBar},
    div,
    prelude::FluentBuilder as _,
    px,
};
use header::{Hero, InstrumentPicker, LiveBadge, ModeToggle};
use model::DrawingKind;
use model_state::ChartModel;
use overlay::Overlay;
use panels::{FlowHeader, HoverCard, OrderBookPanel, StatsGrid, TradesPanel};
use toolbar::AnalysisBar;

/// What the chart window's keys do, whatever element has focus.
pub mod actions {
    gpui_kit::actions!(
        chart,
        [Cancel, DeleteDrawing, Undo, ToolCursor, ToolLevel, ToolTrend, ToolFib, ToolMeasure]
    );
}

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
        let overlay = cx.new(|cx| Overlay::new(model.clone(), cx));
        let hover = cx.new(|_| HoverCard::new(model.clone()));
        let canvas =
            cx.new(|_| PriceCanvas::new(model.clone(), [overlay.entity_id(), hover.entity_id()]));
        overlay.update(cx, |overlay, _| {
            overlay.set_peers([canvas.entity_id(), hover.entity_id()]);
        });
        bind_keys(cx);
        // The chart takes the keyboard while the window is up: with nothing
        // focused, a key event would never reach the window's element tree.
        window.focus(&overlay.read(cx).focus_handle(), cx);
        let view = Self {
            app,
            canvas,
            overlay,
            hero: cx.new(|_| Hero { model: model.clone() }),
            mode: cx.new(|_| ModeToggle { model: model.clone() }),
            live: cx.new(|_| LiveBadge { model: model.clone() }),
            picker: cx.new(|_| InstrumentPicker { model: model.clone() }),
            order_book: cx.new(|_| OrderBookPanel::new(model.clone())),
            trades: cx.new(|_| TradesPanel::new(model.clone())),
            hover,
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
        self.stats.update(cx, |_, cx| cx.notify());
    }
    fn cancel(&mut self, _: &actions::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.analysis_mut().cancel();
            model.analysis_changed(cx);
        });
    }
    fn delete_drawing(
        &mut self,
        _: &actions::DeleteDrawing,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model.update(cx, |model, cx| {
            if model.analysis().selected.is_some() {
                model.analysis_mut().remove_selected();
                model.analysis_changed(cx);
            }
        });
    }
    fn undo(&mut self, _: &actions::Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.analysis_mut().undo();
            model.analysis_changed(cx);
        });
    }
    fn choose_tool(&mut self, tool: analysis::Tool, cx: &mut Context<Self>) {
        if self.model.read(cx).presentation() != crate::market::Presentation::Trading {
            return;
        }
        self.model.update(cx, |model, cx| {
            model.analysis_mut().set_tool(tool);
            model.analysis_changed(cx);
        });
    }
    fn tool_cursor(&mut self, _: &actions::ToolCursor, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_tool(analysis::Tool::Cursor, cx);
    }
    fn tool_level(&mut self, _: &actions::ToolLevel, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_tool(analysis::Tool::Drawing(DrawingKind::Level), cx);
    }
    fn tool_trend(&mut self, _: &actions::ToolTrend, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_tool(analysis::Tool::Drawing(DrawingKind::Trend), cx);
    }
    fn tool_fib(&mut self, _: &actions::ToolFib, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_tool(analysis::Tool::Drawing(DrawingKind::Fib), cx);
    }
    fn tool_measure(&mut self, _: &actions::ToolMeasure, _: &mut Window, cx: &mut Context<Self>) {
        self.choose_tool(analysis::Tool::Drawing(DrawingKind::Measure), cx);
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
        let presentation = model.presentation();
        let has_book = model.spec().is_none_or(|s| s.book);
        let show_book = has_book && model.side_tab() == SideTab::Book;
        let tabs = if presentation != Presentation::Trading {
            let ranges = [
                (30_u32, t!("chart.overview.month")),
                (90_u32, t!("chart.overview.quarter")),
                (365_u32, t!("chart.overview.year")),
            ];
            let selected =
                ranges.iter().position(|(days, _)| *days == model.overview_days()).unwrap_or(1);
            let labels: Vec<SharedString> =
                ranges.iter().map(|(_, label)| (*label).into()).collect();
            let entity = self.model.clone();
            Some(
                widgets::pill_tabs("overview-range", labels, selected, move |index, _, cx| {
                    if let Some((days, _)) = ranges.get(*index) {
                        entity.update(cx, |model, cx| model.select_overview_range(*days, cx));
                    }
                })
                .small(),
            )
        } else {
            model.spec().and_then(|spec| {
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
            })
        };
        let controls = if presentation == Presentation::Trading {
            controls_row(tabs, self.toolbar.clone(), self.mode.clone(), window).into_any_element()
        } else {
            div()
                .id("asset-overview")
                .test_support()
                .h(px(28.))
                .px_4()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .children(tabs)
                .when(presentation == Presentation::Commodity, |row| {
                    row.child(div().w(px(70.)).h_full().flex_shrink_0().child(self.mode.clone()))
                })
                .into_any_element()
        };
        div()
            .id("chart-window")
            .key_context("Chart")
            .size_full()
            .flex()
            .flex_col()
            .bg(super::theme::glass(cx))
            .text_color(palette.foreground)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::delete_drawing))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::tool_cursor))
            .on_action(cx.listener(Self::tool_level))
            .on_action(cx.listener(Self::tool_trend))
            .on_action(cx.listener(Self::tool_fib))
            .on_action(cx.listener(Self::tool_measure))
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
                            .child(controls)
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
                    .when(presentation == Presentation::Trading, |body| {
                        body.child(
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
                        )
                    }),
            )
    }
}

struct ChartKeysBound;
impl Global for ChartKeysBound {}

/// The chart window's keys. Registered once per `GpuiApp`; the chart's root
/// element carries the `Chart` context they are scoped to.
fn bind_keys(cx: &mut gpui_kit::App) {
    if cx.has_global::<ChartKeysBound>() {
        return;
    }
    cx.set_global(ChartKeysBound);
    cx.bind_keys([
        KeyBinding::new("escape", actions::Cancel, Some("Chart")),
        KeyBinding::new("backspace", actions::DeleteDrawing, Some("Chart")),
        KeyBinding::new("delete", actions::DeleteDrawing, Some("Chart")),
        KeyBinding::new("cmd-z", actions::Undo, Some("Chart")),
        KeyBinding::new("ctrl-z", actions::Undo, Some("Chart")),
        KeyBinding::new("alt-v", actions::ToolCursor, Some("Chart")),
        KeyBinding::new("alt-h", actions::ToolLevel, Some("Chart")),
        KeyBinding::new("alt-t", actions::ToolTrend, Some("Chart")),
        KeyBinding::new("alt-f", actions::ToolFib, Some("Chart")),
        KeyBinding::new("alt-m", actions::ToolMeasure, Some("Chart")),
    ]);
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
