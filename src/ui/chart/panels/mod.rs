//! Panels: the order book and trades beside the chart, the hover card over
//! it, the day's figures below it, and the buy/sell split above the trades
//! where there is no book. They read the market, spec and analysis through
//! [`ChartModel`] and change choices through its commands.

mod order_book;
mod trades;

pub use order_book::OrderBookPanel;
pub use trades::TradesPanel;

use crate::{
    format,
    market::{Book, Level},
    ui::chart::model_state::ChartModel,
};
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, px, relative,
};

use crate::model::Instrument;

pub struct HoverCard {
    pub model: Entity<ChartModel>,
}

impl HoverCard {
    pub fn new(model: Entity<ChartModel>) -> Self {
        Self { model }
    }
}

impl Render for HoverCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("HoverCard")
    }
}

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
        let _ = self.model.read(cx);
        div().id("StatsGrid")
    }
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
        let _ = self.model.read(cx);
        div().id("FlowHeader")
    }
}

/// Decimals for prices in this window: the pair's tick precision when the
/// instrument states it, else about six significant digits of the last price.
pub(crate) fn price_decimals(instrument: Option<&Instrument>, last: Option<f64>) -> usize {
    format::decimals(last.unwrap_or(0.0), instrument.and_then(|i| i.decimals))
}

/// Running totals from the best level outwards.
pub(crate) fn totals(levels: &[Level]) -> Vec<f64> {
    let mut total = 0.0;
    levels.iter().map(|level| (total += level.qty, total).1).collect()
}

/// One scale for both sides of the book, so their depth bars compare.
pub(crate) fn deepest(ask_totals: &[f64], bid_totals: &[f64]) -> f64 {
    let deepest =
        ask_totals.last().copied().unwrap_or(0.0).max(bid_totals.last().copied().unwrap_or(0.0));
    if deepest > 0.0 { deepest } else { 1.0 }
}

/// Share of resting size on the bid side across the visible depth, 0–100.
pub fn bid_share(book: &Book) -> Option<f64> {
    let bids: f64 = book.bids.iter().map(|level| level.qty).sum();
    let asks: f64 = book.asks.iter().map(|level| level.qty).sum();
    (bids + asks > 0.0).then(|| bids / (bids + asks) * 100.0)
}

/// Width of the price column; the size column takes the rest, the trailing
/// column (total or clock) has the last share. The same for heads and rows.
pub(crate) const PRICE_COLUMN: f32 = 0.36;
pub(crate) const TRAILING_COLUMN: f32 = 0.30;

/// How much of the scale one level's running total covers, 0–1.
pub(crate) fn bar_share(total: f64, scale: f64) -> f32 {
    if scale <= 0.0 { 0.0 } else { (total / scale).clamp(0.0, 1.0) as f32 }
}

/// Decimals that show a grouping step exactly: `0.001` → 3, `10` → 0.
pub(crate) fn step_decimals(step: f64) -> usize {
    (-(step.log10() + 1e-9).floor()).max(0.0) as usize
}

/// Column heads and the loading spinner, shared with the trades panel.
pub(crate) use self::order_book::{header_row, loading};

/// The bid/ask (or buy/sell) split bar: a share filling from the left in the
/// up color, the rest in the down color.
pub(crate) fn share_bar(share: f64, palette: crate::ui::theme::Palette) -> impl IntoElement {
    div()
        .h(px(4.))
        .flex()
        .gap(px(2.))
        .overflow_hidden()
        .rounded_full()
        .child(div().rounded_full().bg(palette.up).w(relative((share / 100.0) as f32)))
        .child(div().flex_1().rounded_full().bg(palette.down))
}

/// `share%`/`100 − share%` ends above the bar, in their colors.
pub(crate) fn share_ends(
    left_label: impl Into<SharedString>,
    right_label: impl Into<SharedString>,
    palette: crate::ui::theme::Palette,
) -> impl IntoElement {
    let left: SharedString = left_label.into();
    let right: SharedString = right_label.into();
    div()
        .flex()
        .justify_between()
        .text_size(px(10.))
        .mb(px(4.))
        .child(div().text_color(palette.up).whitespace_nowrap().child(left))
        .child(div().text_color(palette.down).whitespace_nowrap().child(right))
}

/// The app handle the model holds, for reading settings while rendering.
pub(crate) fn app_of(model: &Entity<ChartModel>, cx: &gpui_kit::App) -> crate::app::App {
    model.read(cx).app().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{Book, Level};

    fn level(price: f64, qty: f64) -> Level {
        Level { price, qty }
    }

    fn book(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> Book {
        Book {
            bids: bids.iter().map(|(p, q)| level(*p, *q)).collect(),
            asks: asks.iter().map(|(p, q)| level(*p, *q)).collect(),
        }
    }

    #[test]
    fn totals_run_from_the_best_level_outwards() {
        assert_eq!(
            totals(&[level(1.0, 2.0), level(2.0, 3.0), level(3.0, 0.5)]),
            vec![2.0, 5.0, 5.5]
        );
        assert!(totals(&[]).is_empty());
        assert_eq!(deepest(&[2.0, 5.0], &[3.0, 4.0]), 5.0);
        assert_eq!(deepest(&[2.0], &[]), 2.0);
        assert_eq!(deepest(&[], &[]), 1.0);
        assert_eq!(bar_share(2.5, 5.0), 0.5);
        assert_eq!(bar_share(7.0, 5.0), 1.0);
        assert_eq!(bar_share(-1.0, 5.0), 0.0);
        assert_eq!(bar_share(1.0, 0.0), 0.0);
    }

    #[test]
    fn shares_read_percent_of_their_totals() {
        assert_eq!(bid_share(&book(&[(1.0, 3.0)], &[(2.0, 1.0)])), Some(75.0));
        assert_eq!(bid_share(&book(&[], &[(2.0, 1.0)])), Some(0.0));
        assert_eq!(bid_share(&book(&[], &[])), None);
    }

    #[test]
    fn step_decimals_show_the_step_exactly() {
        assert_eq!(step_decimals(0.001), 3);
        assert_eq!(step_decimals(0.01), 2);
        assert_eq!(step_decimals(0.1), 1);
        assert_eq!(step_decimals(1.0), 0);
        assert_eq!(step_decimals(10.0), 0);
    }
}
