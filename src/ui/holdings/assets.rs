//! The asset list, like the order book's ladder: compact rows of
//! right-aligned numbers, a bar behind the value for its share of the total.

use std::sync::Arc;

use crate::{format, portfolio::Asset};
use gpui_kit::{
    DefiniteLength, FontWeight, InteractiveElement as _, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, UniformListScrollHandle,
    component::scroll::ScrollableElement as _, div, px, uniform_list,
};

use super::{series_color, summary};
use crate::ui::{theme::Palette, widgets};

/// Column widths as shares of the row, with and without the cost column:
/// asset, amount, price, price move, value (, cost).
const COLUMNS: [f32; 5] = [0.268, 0.179, 0.179, 0.161, 0.213];
const COLUMNS_COSTED: [f32; 6] = [0.224, 0.149, 0.149, 0.134, 0.179, 0.164];

const ROW_HEIGHT: f32 = 38.;

/// One row of the list, everything already said: strings laid out when the
/// portfolio or the language changed, colors resolved when painting.
pub struct Row {
    pub name: String,
    pub where_held: String,
    pub amount: String,
    /// "—" without a price.
    pub price: String,
    /// The source-reference change in percent and for this quantity.
    pub pct: Option<String>,
    pub change: Option<String>,
    pub value: String,
    pub cost: Option<String>,
    pub pnl: Option<String>,
    /// The share bar: how wide, of the row, and in which series color.
    pub bar: f32,
    pub series: summary::Series,
    pub stable: bool,
    pub pct_down: bool,
    pub change_down: bool,
    pub pnl_down: bool,
}

impl Row {
    pub(super) fn new(
        asset: &Asset,
        series: summary::Series,
        total: f64,
        widest_move: f64,
        sort: summary::Sort,
        several: bool,
        decimals: usize,
    ) -> Self {
        let move_of = asset.change.unwrap_or(0.);
        let bar = match sort {
            summary::Sort::Value => (asset.value.unwrap_or(0.) / total.max(1.)) as f32,
            summary::Sort::Change => (move_of.abs() / widest_move) as f32,
        };
        let down = |value: f64| format::direction(value) == format::Direction::Down;
        Self {
            name: asset.asset.clone(),
            where_held: where_held(asset, several),
            amount: format::amount(asset.amount),
            price: asset
                .price
                .map(|price| {
                    format::price(
                        price,
                        if asset.stable { decimals } else { format::decimals(price, None) },
                    )
                })
                .unwrap_or_else(|| "—".to_owned()),
            pct: asset.change_pct.map(format::change_signed),
            change: asset.change.map(|change| format::signed(change, decimals)),
            value: asset
                .value
                .map(|value| {
                    if asset.partial {
                        format!("{} *", format::price(value, decimals))
                    } else {
                        format::price(value, decimals)
                    }
                })
                .unwrap_or_else(|| "—".to_owned()),
            cost: asset.cost.map(|cost| format::price(cost, format::decimals(cost, None))),
            pnl: asset.pnl.map(|pnl| format::signed(pnl, decimals)),
            bar: bar.clamp(0., 1.),
            series,
            stable: asset.stable,
            pct_down: asset.change_pct.is_some_and(down),
            change_down: asset.change.is_some_and(down),
            pnl_down: asset.pnl.is_some_and(down),
        }
    }
}

/// "Binance Spot 0.5 · OKX Trading 0.26", or just the wallet when there is
/// one of them on one exchange.
pub(super) fn where_held(asset: &Asset, several: bool) -> String {
    let name_of = |held: &crate::portfolio::Holding| {
        let location = match &held.source {
            crate::portfolio::HoldingSource::Exchange { exchange, wallet } => {
                let wallet = super::aside::wallet_name(*wallet);
                if several {
                    format!("{} {wallet}", super::aside::provider_name(*exchange))
                } else {
                    wallet.to_string()
                }
            }
            crate::portfolio::HoldingSource::Manual { platform, .. } => {
                format!("{platform} · {}", t!("holdings.manual.label"))
            }
        };
        if asset.held.len() == 1 {
            location
        } else {
            format!("{location} {}", format::amount(held.amount))
        }
    };
    asset.held.iter().map(name_of).collect::<Vec<_>>().join(" · ")
}

/// The list panel: header, the virtualized rows, and the small-asset toggle.
#[allow(clippy::too_many_arguments)]
pub fn panel(
    rows: &Arc<Vec<Row>>,
    costed: bool,
    small_count: usize,
    small_value: f64,
    currency: crate::currency::Currency,
    show_small: bool,
    scroll: &UniformListScrollHandle,
    palette: Palette,
    toggle_small: impl Fn(&mut gpui_kit::App) + 'static,
) -> gpui_kit::Div {
    let columns: &[f32] = if costed { &COLUMNS_COSTED } else { &COLUMNS };
    let mut header = div()
        .h(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .px(px(12.))
        .border_b_1()
        .border_color(palette.border)
        .text_size(px(10.))
        .text_color(palette.muted);
    let labels: [SharedString; 5] = [
        t!("holdings.asset").into(),
        t!("holdings.amount").into(),
        t!("holdings.price").into(),
        t!("common.holdings.dayPnl").into(),
        t!("holdings.value", currency = currency.code()).into(),
    ];
    for (index, label) in labels.into_iter().enumerate() {
        let mut cell =
            div().w(DefiniteLength::Fraction(columns[index])).flex_none().flex().justify_end();
        if index == 0 {
            cell = cell.justify_start();
        } else {
            cell = cell.pl(px(6.));
        }
        header = header.child(cell.child(div().whitespace_nowrap().child(label)));
    }
    if costed {
        header = header.child(
            div()
                .w(DefiniteLength::Fraction(columns[5]))
                .flex_none()
                .flex()
                .pl(px(6.))
                .justify_end()
                .child(div().whitespace_nowrap().child(t!("holdings.costPnl"))),
        );
    }

    let list_rows = rows.clone();
    let list = uniform_list("assets", rows.len(), move |range, _, _| {
        range.map(|index| row_element(&list_rows[index], columns, costed, palette)).collect()
    })
    .track_scroll(scroll)
    .size_full();

    let mut panel = widgets::panel(div(), palette)
        .mx(px(16.))
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(header)
        .child(div().relative().flex_1().min_h_0().child(list).vertical_scrollbar(scroll));
    if small_count > 0 {
        let label = match (show_small, small_count == 1) {
            (true, true) => {
                t!("holdings.smallHideOne", count = small_count)
            }
            (true, false) => {
                t!("holdings.smallHide", count = small_count)
            }
            (false, true) => {
                t!("holdings.smallMoreOne", count = small_count)
            }
            (false, false) => {
                t!("holdings.smallMore", count = small_count)
            }
        };
        panel = panel.child(
            div()
                .id("small-toggle")
                .flex()
                .flex_none()
                .h(px(32.))
                .items_center()
                .justify_between()
                .px(px(12.))
                .text_size(px(10.))
                .text_color(palette.muted)
                .border_t_1()
                .border_color(palette.border)
                .hover(|style| style.text_color(palette.foreground))
                .cursor_pointer()
                .child(div().whitespace_nowrap().child(label))
                .child(crate::ui::tabular(div().whitespace_nowrap().child(t!(
                    "holdings.smallTotal",
                    value = format::price(small_value, currency.decimals())
                ))))
                .on_click(move |_, _, cx| toggle_small(cx)),
        );
    }
    panel
}

/// One asset row: the share bar behind everything, the name and where it
/// sits on the left, right-aligned numbers after.
fn row_element(row: &Row, columns: &[f32], costed: bool, palette: Palette) -> gpui_kit::Div {
    let bar_color = series_color(row.series, palette);
    let tone = |down: bool| if down { palette.down } else { palette.up };
    let cell = |index: usize| {
        let mut cell =
            div().w(DefiniteLength::Fraction(columns[index])).flex_none().flex().justify_end();
        if index > 0 {
            cell = cell.pl(px(6.));
        }
        cell
    };
    let name = div()
        .w(DefiniteLength::Fraction(columns[0]))
        .flex_none()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(row.name.clone()),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(palette.muted)
                .truncate()
                .child(row.where_held.clone()),
        );
    // The price-move cell: percent on top, what it made below; stables read faint
    // and keep the second line to themselves.
    let pct_text = row.pct.clone().unwrap_or_else(|| "—".to_owned());
    let pct_color = match (row.stable, row.pct.is_some()) {
        (true, _) => palette.faint,
        (false, true) => tone(row.pct_down),
        (false, false) => palette.muted,
    };
    let change = (!row.stable)
        .then(|| row.change.clone().map(|text| (text, tone(row.change_down))))
        .flatten();
    let twenty_four = cell(3).child(
        div()
            .flex()
            .flex_col()
            .items_end()
            .child(
                div().text_size(px(12.)).text_color(pct_color).whitespace_nowrap().child(pct_text),
            )
            .children(change.map(|(text, color)| {
                div()
                    .text_size(px(10.))
                    .opacity(0.8)
                    .text_color(color)
                    .whitespace_nowrap()
                    .child(text)
            })),
    );
    let mut element = div()
        .relative()
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .px(px(12.))
        .child(
            div()
                .absolute()
                .top(px(2.))
                .bottom(px(2.))
                .right_0()
                .w(DefiniteLength::Fraction(row.bar))
                .rounded_l(px(3.))
                .opacity(0.09)
                .bg(bar_color),
        )
        .child(name)
        .child(cell(1).child(crate::ui::tabular(
            div().text_size(px(12.)).whitespace_nowrap().child(row.amount.clone()),
        )))
        .child(
            cell(2).child(crate::ui::tabular(
                div()
                    .text_size(px(12.))
                    .text_color(palette.muted)
                    .whitespace_nowrap()
                    .child(row.price.clone()),
            )),
        )
        .child(twenty_four)
        .child(
            cell(4).child(crate::ui::tabular(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .child(row.value.clone()),
            )),
        );
    if costed {
        let cost = row
            .cost
            .clone()
            .map(|text| (text, palette.muted))
            .unwrap_or_else(|| ("—".to_owned(), palette.faint));
        let pnl = row.pnl.clone().map(|text| (text, tone(row.pnl_down)));
        element = element.child(
            cell(5).child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(crate::ui::tabular(
                        div()
                            .text_size(px(12.))
                            .text_color(cost.1)
                            .whitespace_nowrap()
                            .child(cost.0),
                    ))
                    .children(pnl.map(|(text, color)| {
                        crate::ui::tabular(
                            div()
                                .text_size(px(10.))
                                .text_color(color)
                                .whitespace_nowrap()
                                .child(text),
                        )
                    })),
            ),
        );
    }
    element
}
