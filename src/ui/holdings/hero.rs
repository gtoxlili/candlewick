//! The total, its day, and one line on what did it.

use crate::{format, portfolio::Portfolio};
use gpui_kit::{Animation, AnimationExt, FontWeight, IntoElement, ParentElement, Styled, div, px};

use super::mix;
use crate::ui::{theme::Palette, widgets};

/// The day's change in percent of what the holdings were worth a day ago.
pub(super) fn day_pct(total: Option<f64>, change: Option<f64>) -> Option<f64> {
    match (total, change) {
        (Some(total), Some(change)) => {
            (total - change > 0.).then(|| change / (total - change) * 100.)
        }
        _ => None,
    }
}

/// The one big number and what the day did to it.
pub fn hero(
    portfolio: &Portfolio,
    insight: &Option<Vec<super::summary::InsightPart>>,
    flash: (usize, Option<bool>),
    palette: Palette,
) -> gpui_kit::Div {
    let (total, change) = (portfolio.total, portfolio.change);
    let pct = day_pct(total, change);
    let number = match total {
        None => div().w(px(224.)).h(px(34.)).rounded(px(8.)).bg(palette.fill_strong),
        Some(total) => {
            let plain = div()
                .text_size(px(34.))
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .child(format::price(total, 2));
            let number = match (flash.1, change) {
                // A new total replays the flash: the number starts in the
                // day's color and fades back to the text color.
                (Some(down), Some(change)) => {
                    let flash_color = if change < 0. || down { palette.down } else { palette.up };
                    let foreground = palette.foreground;
                    plain
                        .with_animation(
                            ("hero-flash", flash.0),
                            Animation::new(std::time::Duration::from_secs(1))
                                .with_easing(|t| 1. - (1. - t) * (1. - t) * (1. - t)),
                            move |element, t| element.text_color(mix(flash_color, foreground, t)),
                        )
                        .into_any_element()
                }
                _ => plain.into_any_element(),
            };
            div().flex().items_baseline().gap(px(6.)).child(number).child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.muted)
                    .whitespace_nowrap()
                    .child("USDT"),
            )
        }
    };
    let badge = match (total, change, pct) {
        (Some(_), Some(change), Some(pct)) => Some(widgets::change_badge(
            format!("{} · {} 24h", format::change_signed(pct), format::signed(change, 2)),
            change,
            palette,
        )),
        _ => None,
    };
    let line = insight_line(insight, total.is_none(), palette);
    div()
        .px(px(20.))
        .pt(px(8.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().flex().items_baseline().gap(px(12.)).child(number).children(badge))
        .child(line)
}

/// "BTC +1,320.20 · Positions +755.10 · Stablecoins 26%".
fn insight_line(
    insight: &Option<Vec<super::summary::InsightPart>>,
    reading: bool,
    palette: Palette,
) -> gpui_kit::Div {
    use super::summary::InsightPart;
    let parts: Vec<String> = insight
        .iter()
        .flatten()
        .map(|part| match part {
            InsightPart::Mover { asset, change } => {
                format!("{asset} {}", format::signed(*change, 2))
            }
            InsightPart::Positions { change } => {
                t!("holdings.positionsMove", value = format::signed(*change, 2))
            }
            InsightPart::StableShare { pct } => t!("holdings.stableShare", pct = pct),
        })
        .collect();
    let text = if parts.is_empty() {
        if reading { t!("holdings.readingAccounts").to_owned() } else { String::new() }
    } else {
        parts.join(" · ")
    };
    crate::ui::tabular(
        div()
            .h(px(16.))
            .text_size(px(12.))
            .text_color(palette.muted)
            .whitespace_nowrap()
            .child(text),
    )
}
