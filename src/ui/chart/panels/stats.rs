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
        let items = {
            let model = self.model.read(cx);
            let market = model.market();
            let stats = market.and_then(|m| m.stats());
            let decimals = price_decimals(model.instrument(), stats.map(|s| s.last));
            grid_items(model, stats, decimals)
        };
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
/// Fund NAVs and daily commodities: publication date, reference/range extremes.
fn grid_items(model: &ChartModel, stats: Option<Stats>, decimals: usize) -> Vec<(String, String)> {
    let Some(spec) = model.spec() else { return Vec::new() };
    let span = span_suffix(spec.stats_span);
    let price = |value: Option<f64>| value.map(|v| format::price(v, decimals));
    let compact = |value: Option<f64>| {
        value.map(|v| format!("{} {}", format::compact(v), spec.turnover_unit))
    };
    let onchain = stats.and_then(|s| s.onchain);
    let dash = || "—".to_owned();
    match spec.presentation {
        crate::market::Presentation::NetAssetValue => {
            let candles = model.analysis().candles();
            let source_ms = stats
                .and_then(|s| s.source_time)
                .or_else(|| candles.last().map(|c| c.time * 1000.));
            let previous = stats
                .and_then(|s| Some(s.last - s.change?))
                .filter(|v| v.is_finite() && *v > 0.)
                .or_else(|| candles.iter().rev().nth(1).map(|c| c.close));
            let (high, low) = visible_extremes(model, candles);
            let priced = |value: Option<f64>| {
                value
                    .map(|v| format!("{} {}", format::price(v, decimals), spec.turnover_unit))
                    .unwrap_or_else(dash)
            };
            vec![
                (
                    t!("chart.overview.navDate").to_owned(),
                    source_ms.map(|ms| source_date(ms, spec.day_offset, true)).unwrap_or_else(dash),
                ),
                (t!("chart.overview.previousValue").to_owned(), priced(previous)),
                (t!("chart.overview.rangeHigh").to_owned(), priced(high)),
                (t!("chart.overview.rangeLow").to_owned(), priced(low)),
            ]
        }
        crate::market::Presentation::Commodity => {
            let candles = model.analysis().candles();
            let source_ms = stats
                .and_then(|s| s.source_time)
                .or_else(|| candles.last().map(|c| c.time * 1000.));
            let per_gram = format!("{}/{}", spec.turnover_unit, t!("chart.overview.perGram"));
            let priced = |value: Option<f64>| {
                value
                    .map(|v| format!("{} {per_gram}", format::price(v, decimals)))
                    .unwrap_or_else(dash)
            };
            vec![
                (label("high", span), priced(stats.and_then(|s| s.high))),
                (label("low", span), priced(stats.and_then(|s| s.low))),
                (
                    label("turnover", span),
                    stats.and_then(|s| compact(s.turnover)).unwrap_or_else(dash),
                ),
                (
                    t!("chart.overview.quoteTime").to_owned(),
                    source_ms
                        .map(|ms| source_date(ms, spec.day_offset, false))
                        .unwrap_or_else(dash),
                ),
            ]
        }
        crate::market::Presentation::Trading if spec.onchain => {
            // A token without a circulating supply figure shows its fully diluted
            // value instead.
            let worth = if onchain.is_some_and(|o| o.market_cap.is_none() && o.fdv.is_some()) {
                (
                    t!("chart.fdv").to_owned(),
                    compact(onchain.and_then(|o| o.fdv)).unwrap_or_else(dash),
                )
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
                    stats.and_then(|s| compact(s.turnover)).unwrap_or_else(dash),
                ),
                (
                    t!("chart.liquidity").to_owned(),
                    compact(onchain.and_then(|o| o.liquidity)).unwrap_or_else(dash),
                ),
                (worth.0, worth.1),
            ]
        }
        crate::market::Presentation::Trading => {
            let volume_unit =
                spec.volume_unit.clone().unwrap_or_else(|| t!("chart.shares").to_owned());
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
                    stats.and_then(|s| compact(s.turnover)).unwrap_or_else(dash),
                ),
            ]
        }
    }
}

fn visible_extremes(
    model: &ChartModel,
    candles: &[crate::market::Candle],
) -> (Option<f64>, Option<f64>) {
    let view = &model.viewport().view;
    let slice = match (view.left, view.right) {
        (Some(left), Some(right)) => {
            let start = candles.partition_point(|c| c.time + 86_400. < left);
            let end = candles.partition_point(|c| c.time <= right);
            candles.get(start..end).filter(|s| !s.is_empty()).unwrap_or(candles)
        }
        _ => candles,
    };
    let high = slice.iter().map(|c| c.high).reduce(f64::max);
    let low = slice.iter().map(|c| c.low).reduce(f64::min);
    (high, low)
}

fn source_date(ms: f64, day_offset: i64, date_only: bool) -> String {
    let shifted = (ms / 1000.).floor() as i64 + day_offset;
    let parts = crate::calendar::civil(shifted);
    if date_only || (parts.hour == 0 && parts.minute == 0) {
        format!("{:04}-{:02}-{:02}", parts.year, parts.month, parts.day)
    } else {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            parts.year, parts.month, parts.day, parts.hour, parts.minute
        )
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
        crate::market::StatsSpan::Publication => "publication",
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
