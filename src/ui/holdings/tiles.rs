//! The four figures under the list: how the money is placed.

use crate::{format, portfolio::Portfolio};
use gpui_kit::{FontWeight, ParentElement, Styled, div, px};

use super::summary;
use crate::ui::theme::Palette;

/// A value and the color it reads in.
struct Figure {
    text: String,
    /// None reads like ordinary text.
    up: Option<bool>,
}

/// ` · 26`-style share of the total, appended to a tile's value.
pub(super) fn share(value: f64, total: Option<f64>) -> String {
    match total {
        Some(total) if total > 0. => format!(" · {:.0}%", value / total * 100.),
        _ => String::new(),
    }
}

/// The tiles row, as the chart's day statistics.
pub fn tiles(portfolio: &Portfolio, figures: &summary::Figures, palette: Palette) -> gpui_kit::Div {
    let total = portfolio.total;
    let decimals = portfolio.valuation.currency.decimals();
    let unknown = "—".to_owned();
    let day_pnl = match portfolio.change {
        None => Figure { text: unknown.clone(), up: None },
        Some(change) => Figure {
            text: format::signed(change, decimals),
            up: Some(format::direction(change) != format::Direction::Down),
        },
    };
    let crypto = match total {
        None => Figure { text: unknown.clone(), up: None },
        Some(_) => Figure {
            text: format::price(figures.risk, decimals) + &share(figures.risk, total),
            up: None,
        },
    };
    let stablecoins = match total {
        None => Figure { text: unknown, up: None },
        Some(_) => Figure {
            text: format::price(figures.cash, decimals) + &share(figures.cash, total),
            up: None,
        },
    };
    let unrealized = match (total, figures.positions_pnl) {
        (None, _) => Figure { text: "—".to_owned(), up: None },
        (Some(_), None) => Figure { text: t!("holdings.noPositions").to_owned(), up: None },
        (Some(_), Some(pnl)) => Figure {
            text: format::signed(pnl, decimals),
            up: Some(format::direction(pnl) != format::Direction::Down),
        },
    };
    let tile = |label: &'static str, figure: &Figure| {
        let color = match figure.up {
            Some(true) => palette.up,
            Some(false) => palette.down,
            None => palette.foreground,
        };
        div()
            .flex_1()
            .min_w_0()
            .rounded(px(12.))
            .bg(palette.fill)
            .px(px(10.))
            .py(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div().text_size(px(10.)).text_color(palette.muted).whitespace_nowrap().child(label),
            )
            .child(crate::ui::tabular(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .truncate()
                    .text_color(color)
                    .child(figure.text.clone()),
            ))
    };
    div()
        .flex()
        .gap(px(8.))
        .px(px(16.))
        .child(tile(t!("common.holdings.dayPnl"), &day_pnl))
        .child(tile(t!("holdings.crypto"), &crypto))
        .child(tile(t!("holdings.stablecoins"), &stablecoins))
        .child(tile(t!("holdings.unrealized"), &unrealized))
}
