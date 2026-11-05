//! Panels: the order book and trades beside the chart, the hover card over
//! it, the day's figures below it, and the buy/sell split above the trades
//! where there is no book. They read the market, spec and analysis through
//! [`ChartModel`] and change choices through its commands.

mod hover;
mod order_book;
mod stats;
mod trades;

pub use hover::HoverCard;
pub use order_book::OrderBookPanel;
pub use stats::{FlowHeader, StatsGrid};
pub use trades::TradesPanel;

use crate::{
    format,
    market::{Book, Level, Onchain},
    ui::chart::model_state::ChartModel,
};
use gpui_kit::{Entity, IntoElement, ParentElement, SharedString, Styled, div, px, relative};

use super::market::VisibleStats;
use crate::model::Instrument;

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

/// Share of the day's swaps that bought the token, 0–100.
pub fn buy_share(onchain: &Onchain) -> Option<f64> {
    let total = f64::from(onchain.buys + onchain.sells);
    (total > 0.0).then(|| f64::from(onchain.buys) / total * 100.0)
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

/// One readable line about the stretch on screen, plus, while live, the order
/// book's pressure, or for a pool which way the day's swaps lean. `None`
/// until the stretch can be read.
pub fn insight(
    visible: Option<VisibleStats>,
    book: Option<&Book>,
    onchain: Option<&Onchain>,
) -> Option<String> {
    let visible = visible?;
    let stretch = if visible.live {
        t!("chart.lastSpan", span = format::duration(visible.span))
    } else {
        format::range(visible.from, visible.to)
    };
    let mut parts = vec![
        format!("{stretch} {}", format::change_signed(visible.change_pct)),
        t!("chart.amplitude", value = format!("{:.2}", visible.amplitude_pct)),
    ];
    if visible.live {
        if let Some(share) = book.and_then(bid_share) {
            let other = 100.0 - share;
            parts.push(if share >= 55.0 {
                t!("chart.bidsLead", share = format!("{share:.0}"))
            } else if share <= 45.0 {
                t!("chart.asksLead", share = format!("{other:.0}"))
            } else {
                t!("chart.balanced").to_owned()
            });
        }
        if let Some(share) = onchain.and_then(buy_share) {
            let other = 100.0 - share;
            parts.push(if share >= 55.0 {
                t!("chart.buyersLead", share = format!("{share:.0}"))
            } else if share <= 45.0 {
                t!("chart.sellersLead", share = format!("{other:.0}"))
            } else {
                t!("chart.flowBalanced").to_owned()
            });
        }
    }
    Some(parts.join(" · "))
}

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
    use crate::market::{Book, Level, Onchain};

    fn level(price: f64, qty: f64) -> Level {
        Level { price, qty }
    }

    fn book(bids: &[(f64, f64)], asks: &[(f64, f64)]) -> Book {
        Book {
            bids: bids.iter().map(|(p, q)| level(*p, *q)).collect(),
            asks: asks.iter().map(|(p, q)| level(*p, *q)).collect(),
        }
    }

    fn onchain(buys: u32, sells: u32) -> Onchain {
        Onchain { liquidity: None, market_cap: None, fdv: None, buys, sells }
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
        assert_eq!(buy_share(&onchain(7, 3)), Some(70.0));
        assert_eq!(buy_share(&onchain(0, 0)), None);
    }

    #[test]
    fn insight_reads_the_stretch_and_live_pressure() {
        let visible = VisibleStats {
            live: true,
            span: 3600.0,
            from: 0.0,
            to: 0.0,
            change_pct: 1.234,
            amplitude_pct: 2.5,
        };
        let plain = insight(Some(visible), None, None).unwrap();
        assert!(plain.contains("+1.23%") && plain.contains("2.50%"));
        assert_eq!(plain.matches(" · ").count(), 1);
        let bids = book(&[(1.0, 6.0)], &[(2.0, 4.0)]);
        let led = insight(Some(visible), Some(&bids), Some(&onchain(3, 7))).unwrap();
        assert!(led.contains("60%") && led.contains("70%"));
        assert_eq!(led.matches(" · ").count(), 3);
        let even = book(&[(1.0, 5.0)], &[(2.0, 5.0)]);
        let balanced = insight(Some(visible), Some(&even), Some(&onchain(5, 5))).unwrap();
        assert!(!balanced.contains("60%") && !balanced.contains("70%"));
        assert_eq!(balanced.matches(" · ").count(), 3);
        assert!(insight(None, Some(&bids), None).is_none());
        let past = VisibleStats { live: false, span: 0.0, ..visible };
        let line = insight(Some(past), Some(&bids), Some(&onchain(3, 7))).unwrap();
        assert!(!line.contains("60%") && !line.contains("70%"));
        assert_eq!(line.matches(" · ").count(), 1);
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
