//! The order book: resting depth on both sides of the last price, grouped by
//! a step the provider offers, with each row's running total as a bar behind
//! the numbers.

use super::{
    PRICE_COLUMN, TRAILING_COLUMN, app_of, bar_share, bid_share, deepest, price_decimals,
    share_bar, share_ends, step_decimals, totals,
};
use crate::{
    format,
    market::{Book, Level},
    ui::{tabular, theme::Palette},
};
use gpui_kit::{
    Context, Div, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
    relative,
};

use crate::ui::chart::model_state::ChartModel;

/// Rows shown per side; the containers clip whatever does not fit.
const ROWS: usize = 30;

pub struct OrderBookPanel {
    pub model: Entity<ChartModel>,
}

impl OrderBookPanel {
    pub fn new(model: Entity<ChartModel>) -> Self {
        Self { model }
    }
}

impl Render for OrderBookPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = app_of(&self.model, cx);
        let palette = Palette::current(&app, cx);
        let model = self.model.read(cx);
        let spec = model.spec();
        let market = model.market();
        let steps = spec.map(|s| s.book_steps.as_slice()).unwrap_or(&[]);
        let step = model.book_step().min(steps.len().saturating_sub(1));
        let decimals =
            price_decimals(model.instrument(), market.and_then(|m| m.stats()).map(|s| s.last));
        // Grouped prices need no more decimals than their step.
        let row_decimals =
            steps.get(step).map(|size| decimals.min(step_decimals(*size))).unwrap_or(decimals);
        let book = market.and_then(|m| m.books().get(step));
        let last = market.and_then(|m| m.stats()).map(|s| s.last);
        let direction = model.last_direction();
        let volume_unit: SharedString = spec
            .and_then(|s| s.volume_unit.clone())
            .unwrap_or_else(|| t!("chart.shares").to_owned())
            .into();
        let price_unit: SharedString =
            spec.map(|s| s.turnover_unit.clone()).unwrap_or_default().into();

        let mut column =
            div().id("OrderBookPanel").flex().flex_col().size_full().text_size(px(12.));
        match book {
            // The first book has not arrived yet.
            None => {
                column = column.child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_color(palette.muted)
                        .child(loading()),
                );
            }
            // Closed markets, halted stocks and indexes have no resting orders.
            Some(book) if book.asks.is_empty() && book.bids.is_empty() => {
                column = column
                    .child(header_row(
                        price_unit,
                        volume_unit,
                        t!("chart.total").to_owned(),
                        palette,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .text_color(palette.muted)
                            .text_size(px(12.))
                            .child(t!("chart.noOrders")),
                    );
            }
            Some(book) => {
                if let Some(share) = bid_share(book) {
                    let other = 100.0 - share;
                    column = column.child(
                        div().px_3().pb_2().flex_shrink_0().child(
                            div()
                                .child(share_ends(
                                    t!("chart.bid", share = format!("{share:.0}")),
                                    t!("chart.ask", share = format!("{other:.0}")),
                                    palette,
                                ))
                                .child(share_bar(share, palette)),
                        ),
                    );
                }
                column = column
                    .child(header_row(
                        price_unit,
                        volume_unit,
                        t!("chart.total").to_owned(),
                        palette,
                    ))
                    .child(asks(book, row_decimals, palette))
                    .child(middle_row(
                        &self.model,
                        last,
                        direction,
                        book,
                        decimals,
                        steps,
                        step,
                        palette,
                    ))
                    .child(bids(book, row_decimals, palette));
            }
        }
        column
    }
}

/// A quiet spinner while the first data has not arrived.
pub(crate) fn loading() -> impl IntoElement {
    gpui_kit::component::spinner::Spinner::new()
}

/// The column heads: what the prices and sizes count.
pub(crate) fn header_row(
    price_unit: SharedString,
    volume_unit: SharedString,
    trailing: String,
    palette: Palette,
) -> Div {
    let price_label: SharedString = if price_unit.is_empty() {
        t!("chart.price").to_owned().into()
    } else {
        t!("chart.priceIn", unit = price_unit).into()
    };
    let size_label: SharedString = if volume_unit.is_empty() {
        t!("chart.size").to_owned().into()
    } else {
        t!("chart.sizeIn", unit = volume_unit).into()
    };
    div()
        .h(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .px_3()
        .text_size(px(10.))
        .text_color(palette.muted)
        .child(div().w(relative(PRICE_COLUMN)).flex_none().whitespace_nowrap().child(price_label))
        .child(div().flex_1().text_right().whitespace_nowrap().child(size_label))
        .child(
            div()
                .w(relative(TRAILING_COLUMN))
                .flex_none()
                .text_right()
                .whitespace_nowrap()
                .child(trailing),
        )
}

/// The asks, best at the bottom next to the price; farther levels clip at
/// the top.
fn asks(book: &Book, decimals: usize, palette: Palette) -> Div {
    let ask_totals = totals(&book.asks);
    let scale = deepest(&ask_totals, &totals(&book.bids));
    div().flex().flex_1().min_h_0().flex_col().justify_end().overflow_hidden().children(
        book.asks
            .iter()
            .take(ROWS)
            .rev()
            .zip(ask_totals.iter().take(ROWS).rev())
            .map(|(level, total)| side_row(level, *total, scale, false, decimals, palette)),
    )
}

/// The bids, best at the top; farther levels clip at the bottom.
fn bids(book: &Book, decimals: usize, palette: Palette) -> Div {
    let bid_totals = totals(&book.bids);
    let scale = deepest(&totals(&book.asks), &bid_totals);
    div().flex().flex_1().min_h_0().flex_col().overflow_hidden().children(
        book.bids
            .iter()
            .take(ROWS)
            .zip(bid_totals.iter().take(ROWS))
            .map(|(level, total)| side_row(level, *total, scale, true, decimals, palette)),
    )
}

fn side_row(
    level: &Level,
    total: f64,
    scale: f64,
    bid: bool,
    decimals: usize,
    palette: Palette,
) -> Div {
    let side = if bid { palette.up } else { palette.down };
    let width = bar_share(total, scale);
    div()
        .relative()
        .h(px(20.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .px_3()
        .child(
            div()
                .absolute()
                .top(px(1.))
                .bottom(px(1.))
                .right_0()
                .w(relative(width))
                .rounded_l(px(3.))
                .bg(side.alpha(0.1)),
        )
        .child(
            div()
                .w(relative(PRICE_COLUMN))
                .flex_none()
                .text_color(side)
                .whitespace_nowrap()
                .child(tabular(div().child(format::price(level.price, decimals)))),
        )
        .child(
            div()
                .flex_1()
                .text_right()
                .whitespace_nowrap()
                .child(tabular(div().child(format::qty(level.qty)))),
        )
        .child(
            div()
                .w(relative(TRAILING_COLUMN))
                .flex_none()
                .text_right()
                .whitespace_nowrap()
                .text_color(palette.muted)
                .child(tabular(div().child(format::qty(total)))),
        )
}

/// The last price between the sides, with how it last moved, and either the
/// grouping picker or the spread.
#[allow(clippy::too_many_arguments)]
fn middle_row(
    model: &Entity<ChartModel>,
    last: Option<f64>,
    direction: i8,
    book: &Book,
    decimals: usize,
    steps: &[f64],
    step: usize,
    palette: Palette,
) -> Div {
    let text = last.map(|last| format::price(last, decimals)).unwrap_or_else(|| "—".to_owned());
    let (arrow, trend_color) = match direction.signum() {
        1 => ("↑", palette.up),
        -1 => ("↓", palette.down),
        _ => ("", palette.foreground),
    };
    let trailing: gpui_kit::AnyElement = if steps.len() > 1 {
        step_picker(model, steps, step, palette).into_any_element()
    } else {
        spread_pill(book, decimals, palette).into_any_element()
    };
    div()
        .h(px(40.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
        .px_3()
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(trend_color)
                .whitespace_nowrap()
                .child(tabular(div().child(text))),
        )
        .child(div().text_size(px(12.)).text_color(trend_color).child(arrow))
        .child(div().ml_auto().child(trailing))
}

fn spread_pill(book: &Book, decimals: usize, palette: Palette) -> Div {
    let spread = match (book.asks.first(), book.bids.first()) {
        (Some(ask), Some(bid)) => Some(ask.price - bid.price),
        _ => None,
    };
    div().when_some(spread, |row, spread| {
        row.rounded_full()
            .px_2()
            .py(px(2.))
            .bg(palette.fill)
            .text_size(px(10.))
            .text_color(palette.muted)
            .whitespace_nowrap()
            .child(t!("chart.spread", value = format::price(spread, decimals)))
    })
}

/// The step the book is grouped by, opening a menu of the others.
fn step_picker(
    model: &Entity<ChartModel>,
    steps: &[f64],
    selected: usize,
    palette: Palette,
) -> impl IntoElement {
    use gpui_kit::component::popover::Popover;
    let label = steps
        .get(selected)
        .map(|step| format::price(*step, step_decimals(*step)))
        .unwrap_or_default();
    let entity = model.clone();
    let steps: Vec<f64> = steps.to_vec();
    Popover::new("book-step")
        .anchor(gpui_kit::Anchor::BottomRight)
        .trigger(
            gpui_kit::component::button::Button::new("book-step-trigger")
                .outline()
                .label(label.clone())
                .text_size(px(10.))
                .px_2()
                .py(px(2.))
                .text_color(palette.muted),
        )
        .content(move |_, _, _| {
            let entity = entity.clone();
            div().flex().flex_col().py_1().min_w(px(90.)).children(steps.iter().enumerate().map(
                |(i, step)| {
                    let entity = entity.clone();
                    div()
                        .id(gpui_kit::ElementId::Name(format!("book-step-{i}").into()))
                        .px_3()
                        .py_1()
                        .text_size(px(12.))
                        .when(i == selected, |row| row.bg(palette.fill))
                        .whitespace_nowrap()
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |model, cx| model.set_book_step(i, cx));
                        })
                        .child(tabular(div().child(format::price(*step, step_decimals(*step)))))
                },
            ))
        })
}
