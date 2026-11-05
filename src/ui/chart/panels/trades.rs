//! The latest trades: price, size and clock, newest first, with new rows
//! lit for a moment as they land.

use std::time::{Duration, Instant};

use super::{PRICE_COLUMN, TRAILING_COLUMN, app_of, header_row, loading, price_decimals};
use crate::{
    format,
    market::{FeedState, Trade},
    ui::{tabular, theme::Palette},
};
use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Task, Window, div, prelude::FluentBuilder, px, relative,
};

use crate::ui::chart::model_state::ChartModel;

/// How long a landed trade stays lit.
const FLASH: Duration = Duration::from_millis(600);

pub struct TradesPanel {
    pub model: Entity<ChartModel>,
    /// The newest trade seen; newer ones flash until the deadline passes.
    seen_through: Option<u64>,
    flash_until: Option<Instant>,
    timer: Option<Task<()>>,
}

impl TradesPanel {
    pub fn new(model: Entity<ChartModel>) -> Self {
        Self { model, seen_through: None, flash_until: None, timer: None }
    }

    /// Lights trades newer than the newest seen and schedules the unlight.
    /// Returns whether rows are lit now.
    fn watch_new(&mut self, trades: &[Trade], cx: &mut Context<Self>) -> bool {
        if let Some(newest) = trades.first() {
            match self.seen_through {
                // The first page is history, not news.
                None => self.seen_through = Some(newest.id),
                Some(seen) if newest.id > seen => {
                    self.seen_through = Some(newest.id);
                    self.flash_until = Some(Instant::now() + FLASH);
                    self.timer = Some(cx.spawn(async move |view, cx| {
                        cx.background_executor().timer(FLASH).await;
                        let _ = view.update(cx, |_, cx| cx.notify());
                    }));
                }
                Some(_) => {}
            }
        }
        self.flash_until.is_some_and(|until| Instant::now() < until)
    }
}

impl Render for TradesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = app_of(&self.model, cx);
        let palette = Palette::current(&app, cx);
        let (spec, decimals, connecting, rows) = {
            let model = self.model.read(cx);
            let market = model.market();
            let spec = model.spec().cloned();
            let decimals =
                price_decimals(model.instrument(), market.and_then(|m| m.stats()).map(|s| s.last));
            let connecting = market.is_none_or(|m| m.feed() == FeedState::Connecting);
            (spec, decimals, connecting, market.map(|m| m.trades().to_vec()).unwrap_or_default())
        };
        let volume_unit: SharedString = spec
            .as_ref()
            .and_then(|s| s.volume_unit.clone())
            .unwrap_or_else(|| t!("chart.shares").to_owned())
            .into();
        let price_unit: SharedString =
            spec.as_ref().map(|s| s.turnover_unit.clone()).unwrap_or_default().into();
        let flashing = self.watch_new(&rows, cx);
        let flash_from = self.seen_through.unwrap_or_default();

        let list: AnyElement = if connecting {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .text_color(palette.muted)
                .child(loading())
                .into_any_element()
        } else if rows.is_empty() {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .text_color(palette.muted)
                .text_size(px(12.))
                .child(t!("chart.noTrades"))
                .into_any_element()
        } else {
            // The list is virtualized: only the visible rows are built.
            gpui_kit::uniform_list("trades", rows.len(), move |range, _, _| {
                range
                    .map(|ix| trade_row(ix, &rows[ix], flash_from, flashing, decimals, palette))
                    .collect::<Vec<_>>()
            })
            .flex_1()
            .min_h_0()
            .size_full()
            .into_any_element()
        };
        div()
            .id("TradesPanel")
            .flex()
            .flex_col()
            .size_full()
            .text_size(px(12.))
            .child(header_row(price_unit, volume_unit, t!("chart.time").to_owned(), palette))
            .child(list)
    }
}

fn trade_row(
    ix: usize,
    trade: &Trade,
    flash_from: u64,
    flashing: bool,
    decimals: usize,
    palette: Palette,
) -> AnyElement {
    let fresh = flashing && trade.id >= flash_from;
    let side = if trade.sell { palette.down } else { palette.up };
    div()
        .id(gpui_kit::ElementId::Name(format!("trade-{ix}").into()))
        .relative()
        .h(px(20.))
        .w_full()
        .flex()
        .items_center()
        .px_3()
        .when(fresh, |row| row.bg(palette.fill_strong))
        .child(
            div()
                .w(relative(PRICE_COLUMN))
                .flex_none()
                .text_color(side)
                .whitespace_nowrap()
                .child(tabular(div().child(format::price(trade.price, decimals)))),
        )
        .child(
            div()
                .flex_1()
                .text_right()
                .whitespace_nowrap()
                .child(tabular(div().child(format::qty(trade.qty)))),
        )
        .child(
            div()
                .w(relative(TRAILING_COLUMN))
                .flex_none()
                .text_right()
                .whitespace_nowrap()
                .text_color(palette.muted)
                .child(tabular(div().child(format::clock(trade.time)))),
        )
        .into_any_element()
}
