//! The hovered candle's figures in a small card beside the pointer: its
//! start, how it closed against its open, its open, high, low and close, its
//! range, and each indicator's value there. The overlay owns the pointer;
//! this card only reads where it landed.

use super::{app_of, price_decimals};
use crate::{
    format,
    market::Candle,
    ui::{tabular, theme::Palette},
};
use gpui_kit::{
    Context, Div, Entity, Hsla, IntoElement, ParentElement, Render, SharedString, Styled, TextRun,
    Window, WindowTextSystem, div, prelude::FluentBuilder, px,
};

use crate::ui::chart::{geometry::Point, labels, model_state::ChartModel};

/// Distance from the pointer to the card's near corner.
const OFFSET: f64 = 14.0;
/// Space kept between the card and the plot's edges.
const MARGIN: f64 = 6.0;
/// The card's narrowest look, whatever the numbers say.
const MIN_WIDTH: f64 = 176.0;

/// Where the card sits: beside the pointer, flipped to stay inside the plot,
/// never closer than the margin to its top-left corner.
pub fn place(pointer: Point, card: (f64, f64), bounds: (f64, f64)) -> (f64, f64) {
    let (cw, ch) = card;
    let (bw, bh) = bounds;
    let mut x = pointer.x + OFFSET;
    if x + cw > bw - MARGIN {
        x = pointer.x - OFFSET - cw;
    }
    let mut y = pointer.y + OFFSET;
    if y + ch > bh - MARGIN {
        y = pointer.y - OFFSET - ch;
    }
    (x.max(MARGIN), y.max(MARGIN))
}

pub struct HoverCard {
    pub model: Entity<ChartModel>,
}

impl HoverCard {
    pub fn new(model: Entity<ChartModel>) -> Self {
        Self { model }
    }
}

impl Render for HoverCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = app_of(&self.model, cx);
        let palette = Palette::current(&app, cx);
        let model = self.model.read(cx);
        let analysis = model.analysis();
        // The pointer snaps to a candle's centre, half an interval after its
        // opening time; the last candle opening at or before it is the one.
        let interval = model.market().map(|market| market.interval()).unwrap_or(60);
        let index = analysis.snapped.as_ref().and_then(|snapped| {
            candle_index(analysis.candles(), snapped.time, f64::from(interval))
        });
        let hovered = index.and_then(|index| analysis.candles().get(index).copied());
        let (candle, readings, time_label, decimals) = match (hovered, analysis.pointer) {
            (Some(candle), Some(_)) => {
                let readings: Vec<(String, Hsla, Option<f64>)> = analysis
                    .values_at(index)
                    .into_iter()
                    .map(|reading| {
                        (
                            reading.spec.label(),
                            indicator_color(&reading.spec.id, palette),
                            reading.value,
                        )
                    })
                    .collect();
                let day_offset = model.spec().map(|s| s.day_offset).unwrap_or(0);
                let decimals = price_decimals(
                    model.instrument(),
                    model.market().and_then(|m| m.stats()).map(|s| s.last),
                );
                (
                    Some(candle),
                    readings,
                    labels::candle_label(candle.time, interval, day_offset),
                    decimals,
                )
            }
            _ => (None, Vec::new(), String::new(), 2),
        };

        // The card follows the pointer inside the plot the canvas painted.
        let frame = model.frame.get();
        let pointer = model.analysis().pointer;
        match (candle, pointer, frame) {
            (Some(candle), Some(pointer), Some(frame)) => {
                let (w, h) =
                    card_size(window.text_system(), &time_label, candle, decimals, &readings);
                let (x, y) = place(
                    pointer,
                    (w, h),
                    (frame.plot_x + frame.plot_w, frame.plot_y + frame.plot_h),
                );
                div()
                    .absolute()
                    .left(px(x as f32))
                    .top(px(y as f32))
                    .min_w(px(MIN_WIDTH as f32))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.popover)
                    .px_3()
                    .py_2()
                    .text_size(px(12.))
                    .shadow_sm()
                    .child(content(time_label, candle, decimals, &readings, palette))
            }
            _ => div(),
        }
    }
}

/// The last candle whose opening time is at or before `time`.
fn candle_index(candles: &[Candle], time: f64, interval: f64) -> Option<usize> {
    let index = candles.partition_point(|candle| candle.time <= time).checked_sub(1)?;
    (time <= candles[index].time + interval).then_some(index)
}

/// One figure: a muted label left, the number right-aligned and set in the
/// value's color when it has one.
fn figure(label: &str, value: String, value_color: Option<Hsla>, palette: Palette) -> Div {
    let label = SharedString::from(label);
    let value = tabular(
        div()
            .whitespace_nowrap()
            .when_some(value_color, |value, color| value.text_color(color))
            .child(value),
    );
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .justify_between()
        .gap(px(8.))
        .child(div().text_color(palette.muted).whitespace_nowrap().child(label))
        .child(value)
}

fn content(
    time_label: String,
    candle: Candle,
    decimals: usize,
    readings: &[(String, Hsla, Option<f64>)],
    palette: Palette,
) -> Div {
    let pct =
        if candle.open > 0.0 { (candle.close - candle.open) / candle.open * 100.0 } else { 0.0 };
    let change_color = match format::direction(pct) {
        format::Direction::Up => palette.up,
        format::Direction::Down => palette.down,
        format::Direction::Flat => palette.muted,
    };
    let amplitude =
        if candle.low > 0.0 { (candle.high - candle.low) / candle.low * 100.0 } else { 0.0 };
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap(px(16.))
                .child(div().text_color(palette.muted).whitespace_nowrap().child(time_label))
                .child(
                    div()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(change_color)
                        .whitespace_nowrap()
                        .child(tabular(div().child(format::change_signed(pct)))),
                ),
        )
        .child(
            div()
                .mt(px(6.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .gap(px(14.))
                        .child(figure(
                            t!("chart.ohlc.open"),
                            format::price(candle.open, decimals),
                            None,
                            palette,
                        ))
                        .child(figure(
                            t!("chart.ohlc.high"),
                            format::price(candle.high, decimals),
                            None,
                            palette,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(14.))
                        .child(figure(
                            t!("chart.ohlc.low"),
                            format::price(candle.low, decimals),
                            None,
                            palette,
                        ))
                        .child(figure(
                            t!("chart.ohlc.close"),
                            format::price(candle.close, decimals),
                            Some(change_color),
                            palette,
                        )),
                ),
        )
        .child(
            div()
                .mt(px(6.))
                .text_size(px(10.))
                .text_color(palette.muted)
                .whitespace_nowrap()
                .child(t!("chart.amplitude", value = format!("{amplitude:.2}"))),
        )
        .when(!readings.is_empty(), |card| {
            card.child(
                div()
                    .mt(px(6.))
                    .pt(px(6.))
                    .border_t_1()
                    .border_color(palette.border)
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(readings.iter().map(|(label, color, value)| {
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(10.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_color(palette.muted)
                                    .whitespace_nowrap()
                                    .child(div().size(px(6.)).rounded_full().bg(*color))
                                    .child(label.clone()),
                            )
                            .child(tabular(
                                div().whitespace_nowrap().child(
                                    value
                                        .map(|v| format::price(v, decimals))
                                        .unwrap_or_else(|| "—".to_owned()),
                                ),
                            ))
                    })),
            )
        })
}

/// The hue of an indicator, stable per slot so the legend never recolors.
pub(crate) fn indicator_color(id: &str, palette: Palette) -> Hsla {
    match id {
        "ma1" => palette.violet,
        "ma2" => palette.amber,
        "ma3" => palette.mint,
        "boll" => palette.glow,
        _ => palette.foreground,
    }
}

/// The card's size, measured from its widest line: two OHLC pairs share a
/// row, so the height is the first line, the pairs, the range and the
/// indicator rows.
fn card_size(
    text_system: &WindowTextSystem,
    time_label: &str,
    candle: Candle,
    decimals: usize,
    readings: &[(String, Hsla, Option<f64>)],
) -> (f64, f64) {
    let figure_w = |label: &str, value: &str| {
        text_width(text_system, label, 12.0) + 8.0 + text_width(text_system, value, 12.0)
    };
    let labels = [
        t!("chart.ohlc.open"),
        t!("chart.ohlc.high"),
        t!("chart.ohlc.low"),
        t!("chart.ohlc.close"),
    ];
    let values = [
        format::price(candle.open, decimals),
        format::price(candle.high, decimals),
        format::price(candle.low, decimals),
        format::price(candle.close, decimals),
    ];
    let pct =
        if candle.open > 0.0 { (candle.close - candle.open) / candle.open * 100.0 } else { 0.0 };
    let mut widest = text_width(text_system, time_label, 12.0)
        + 16.0
        + text_width(text_system, &format::change_signed(pct), 12.0);
    let first = figure_w(labels[0], &values[0]).max(figure_w(labels[1], &values[1]));
    let second = figure_w(labels[2], &values[2]).max(figure_w(labels[3], &values[3]));
    widest = widest.max(first + 14.0 + second);
    for (label, _, value) in readings {
        let value = value.map(|v| format::price(v, decimals)).unwrap_or_else(|| "—".to_owned());
        widest = widest.max(figure_w(label, &value));
    }
    let width = widest + 24.0 + 2.0;
    let mut height = 16.0 + 18.0 + 6.0 + 2.0 * 18.0 + 2.0 + 6.0 + 15.0;
    if !readings.is_empty() {
        height +=
            12.0 + readings.len() as f64 * 18.0 + (readings.len().saturating_sub(1)) as f64 * 2.0;
    }
    (width.max(MIN_WIDTH), height + 24.0)
}

/// The width of one line of 12-pixel text, as the card renders it.
fn text_width(text_system: &WindowTextSystem, text: &str, size: f64) -> f64 {
    let run = TextRun {
        len: text.len(),
        font: Default::default(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    text_system.layout_line(text, px(size as f32), std::slice::from_ref(&run), None).width.to_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn the_card_sits_beside_the_pointer_until_an_edge_flips_it() {
        // In the middle: down and right of the pointer.
        assert_eq!(place(point(100., 100.), (80., 60.), (600., 400.)), (114., 114.));
        // Near the right edge: flips left.
        assert_eq!(place(point(590., 100.), (80., 60.), (600., 400.)), (496., 114.));
        // Near the bottom: flips up.
        assert_eq!(place(point(100., 380.), (80., 60.), (600., 400.)), (114., 306.));
        // Corner: flips both ways.
        assert_eq!(place(point(595., 395.), (80., 60.), (600., 400.)), (501., 321.));
        // A card taller than the room still keeps its margin.
        assert_eq!(place(point(10., 10.), (2000., 1000.), (600., 400.)), (6., 6.));
    }

    #[test]
    fn the_hovered_candle_is_the_one_the_pointer_snaps_into() {
        let candle = |time| Candle { time, open: 1.0, high: 1.0, low: 1.0, close: 1.0 };
        let candles = [candle(60.0), candle(120.0), candle(180.0)];
        assert_eq!(candle_index(&candles, 90.0, 60.0), Some(0));
        assert_eq!(candle_index(&candles, 125.0, 60.0), Some(1));
        assert_eq!(candle_index(&candles, 180.0, 60.0), Some(2));
        assert_eq!(candle_index(&candles, 240.0, 60.0), Some(2));
        assert_eq!(candle_index(&candles, 245.0, 60.0), None);
        assert_eq!(candle_index(&candles, 30.0, 60.0), None);
        assert_eq!(candle_index(&[], 60.0, 60.0), None);
    }

    #[test]
    fn indicator_hues_are_stable_per_slot() {
        let palette = Palette::new(false, crate::model::ColorScheme::GreenUp);
        assert_eq!(indicator_color("ma1", palette), palette.violet);
        assert_eq!(indicator_color("ma2", palette), palette.amber);
        assert_eq!(indicator_color("ma3", palette), palette.mint);
        assert_eq!(indicator_color("boll", palette), palette.glow);
        assert_eq!(indicator_color("anything", palette), palette.foreground);
    }
}
