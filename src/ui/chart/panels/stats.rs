//! The day's figures under the chart, and the buy/sell split above the
//! trades for pools, which have no order book.

use super::{app_of, buy_share, price_decimals, share_bar, share_ends};
use crate::{
    format,
    market::Stats,
    ui::{tabular, theme::Palette},
};
use gpui_kit::{
    Context, Div, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::FluentBuilder, px,
};

use crate::ui::chart::model_state::ChartModel;

pub struct StatsGrid {
    pub model: Entity<ChartModel>,
}

impl StatsGrid {
    pub fn new(model: Entity<ChartModel>) -> Self {
        Self { model }
    }
}

impl Render for StatsGrid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = app_of(&self.model, cx);
        let palette = Palette::current(&app, cx);
        let (spec, stats, decimals) = {
            let model = self.model.read(cx);
            let market = model.market();
            (
                model.spec().cloned(),
                market.and_then(|m| m.stats()),
                price_decimals(model.instrument(), market.and_then(|m| m.stats()).map(|s| s.last)),
            )
        };
        let items = grid_items(spec.as_ref(), stats, decimals);
        // Five tiles share the width at the narrowest window; a point smaller
        // keeps compact pool figures on one line.
        let small = items.len() > 4;
        div()
            .id("StatsGrid")
            .flex()
            .size_full()
            .gap(px(6.))
            .children(items.into_iter().map(|(label, value)| tile(label, value, small, palette)))
    }
}

/// Exchanges and stocks: the range, volume and turnover. Pools: the range,
/// volume in dollars, and what the pool holds and the token is worth.
fn grid_items(
    spec: Option<&crate::market::ChartSpec>,
    stats: Option<Stats>,
    decimals: usize,
) -> Vec<(String, String)> {
    let Some(spec) = spec else { return Vec::new() };
    let span = span_suffix(spec.stats_span);
    let price = |value: Option<f64>| value.map(|v| format::price(v, decimals));
    let compact = |value: Option<f64>| {
        value.map(|v| format!("{} {}", format::compact(v), spec.turnover_unit))
    };
    let onchain = stats.and_then(|s| s.onchain);
    let dash = || "—".to_owned();
    if spec.onchain {
        // A token without a circulating supply figure shows its fully diluted
        // value instead.
        let worth = if onchain.is_some_and(|o| o.market_cap.is_none() && o.fdv.is_some()) {
            (t!("chart.fdv").to_owned(), compact(onchain.and_then(|o| o.fdv)).unwrap_or_else(dash))
        } else {
            (
                t!("chart.marketCap").to_owned(),
                compact(onchain.and_then(|o| o.market_cap)).unwrap_or_else(dash),
            )
        };
        vec![
            (label("high", span), price(stats.and_then(|s| s.high)).unwrap_or_else(dash)),
            (label("low", span), price(stats.and_then(|s| s.low)).unwrap_or_else(dash)),
            (
                label("volume", span),
                stats
                    .map(|s| format!("{} {}", format::compact(s.turnover), spec.turnover_unit))
                    .unwrap_or_else(dash),
            ),
            (
                t!("chart.liquidity").to_owned(),
                compact(onchain.and_then(|o| o.liquidity)).unwrap_or_else(dash),
            ),
            (worth.0, worth.1),
        ]
    } else {
        let volume_unit = spec.volume_unit.clone().unwrap_or_else(|| t!("chart.shares").to_owned());
        vec![
            (label("high", span), price(stats.and_then(|s| s.high)).unwrap_or_else(dash)),
            (label("low", span), price(stats.and_then(|s| s.low)).unwrap_or_else(dash)),
            (
                label("volume", span),
                stats
                    .and_then(|s| s.volume)
                    .map(|v| format!("{} {}", format::compact(v), volume_unit))
                    .unwrap_or_else(dash),
            ),
            (
                label("turnover", span),
                stats
                    .map(|s| format!("{} {}", format::compact(s.turnover), spec.turnover_unit))
                    .unwrap_or_else(dash),
            ),
        ]
    }
}

/// `chart.high_rolling24h` and its kin, selected by the span.
fn label(kind: &str, span: &str) -> String {
    match (kind, span) {
        ("high", "rolling24h") => t!("chart.high_rolling24h").to_owned(),
        ("high", _) => t!("chart.high_today").to_owned(),
        ("low", "rolling24h") => t!("chart.low_rolling24h").to_owned(),
        ("low", _) => t!("chart.low_today").to_owned(),
        ("volume", "rolling24h") => t!("chart.volume_rolling24h").to_owned(),
        ("volume", _) => t!("chart.volume_today").to_owned(),
        ("turnover", "rolling24h") => t!("chart.turnover_rolling24h").to_owned(),
        (_, _) => t!("chart.turnover_today").to_owned(),
    }
}

fn span_suffix(span: crate::market::StatsSpan) -> &'static str {
    match span {
        crate::market::StatsSpan::Rolling24h => "rolling24h",
        crate::market::StatsSpan::Today => "today",
    }
}

/// A label and value on a tinted fill; the value never wraps.
fn tile(label: impl Into<SharedString>, value: String, small: bool, palette: Palette) -> Div {
    let label: SharedString = label.into();
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .rounded(px(10.))
        .bg(palette.fill)
        .px_2()
        .py_2()
        .child(div().text_size(px(10.)).text_color(palette.muted).whitespace_nowrap().child(label))
        .child(
            div()
                .text_size(px(if small { 11. } else { 12. }))
                .text_color(palette.foreground)
                .truncate()
                .child(tabular(div().child(value))),
        )
}

pub struct FlowHeader {
    pub model: Entity<ChartModel>,
}

impl FlowHeader {
    pub fn new(model: Entity<ChartModel>) -> Self {
        Self { model }
    }
}

impl Render for FlowHeader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = app_of(&self.model, cx);
        let palette = Palette::current(&app, cx);
        let share = self
            .model
            .read(cx)
            .market()
            .and_then(|m| m.stats())
            .and_then(|s| s.onchain)
            .as_ref()
            .and_then(buy_share);
        div()
            .id("FlowHeader")
            .pb(px(10.))
            .child(
                div()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .whitespace_nowrap()
                            .child(t!("chart.trades")),
                    )
                    .when_some(share, |row, _| {
                        row.child(
                            div()
                                .text_size(px(10.))
                                .text_color(palette.muted)
                                .whitespace_nowrap()
                                .child(t!("chart.span_rolling24h")),
                        )
                    }),
            )
            .when_some(share, |this, share| {
                let other = 100.0 - share;
                this.child(
                    div().mt(px(4.)).child(
                        div()
                            .child(share_ends(
                                t!("chart.buyers", share = format!("{share:.0}")),
                                t!("chart.sellers", share = format!("{other:.0}")),
                                palette,
                            ))
                            .child(share_bar(share, palette)),
                    ),
                )
            })
    }
}
