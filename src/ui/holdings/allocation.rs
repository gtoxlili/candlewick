//! The wick: one thin bar showing how the total splits, with its legend.

use super::{series_color, summary};
use crate::ui::theme::Palette;
use gpui_kit::{
    Animation, AnimationExt, AnyElement, DefiniteLength, IntoElement, ParentElement, SharedString,
    Styled, div, px,
};

/// What one stretch of the bar says in the legend.
struct Labeled {
    label: SharedString,
    /// Of the total, 0–100.
    pct: f64,
    series: summary::Series,
}

/// The allocation bar, largest holding first, cash at the end, lit at the
/// end while the numbers are live.
pub fn allocation(shares: &[summary::Share], live: bool, palette: Palette) -> gpui_kit::Div {
    let labeled: Vec<Labeled> = shares
        .iter()
        .map(|share| Labeled {
            label: match &share.name {
                summary::Name::Asset(name) => name.as_str().into(),
                summary::Name::Others => t!("common.holdings.others").into(),
                summary::Name::Stablecoins => t!("holdings.stablecoins").into(),
            },
            pct: share.pct,
            series: share.series,
        })
        .collect();
    let mut bar = div().flex().items_center().gap(px(1.)).h(px(6.)).flex_1();
    if labeled.is_empty() {
        bar = bar.child(div().flex_1().h_full().rounded_full().bg(palette.fill_strong));
    } else {
        let last = labeled.len() - 1;
        bar = bar.children(labeled.iter().enumerate().map(|(index, share)| {
            let mut segment = div()
                .flex_none()
                .min_w(px(2.))
                .flex_basis(DefiniteLength::Fraction(share.pct as f32 / 100.))
                .h_full()
                .bg(series_color(share.series, palette));
            if index == 0 {
                segment = segment.rounded_l(px(3.));
            }
            if index == last {
                segment = segment.rounded_r(px(3.));
            }
            segment
        }));
    }
    let dot: AnyElement = if live {
        let mint = palette.mint;
        div()
            .size(px(10.))
            .rounded_full()
            .bg(mint)
            .with_animation(
                "wick-pulse",
                Animation::new(std::time::Duration::from_millis(2400)).repeat(),
                |element, pulse| element.opacity(0.3 + 0.7 * (1. - (2. * pulse - 1.).powi(2))),
            )
            .into_any_element()
    } else {
        div().size(px(10.)).rounded_full().bg(palette.fill_strong).into_any_element()
    };
    div()
        .px(px(20.))
        .flex()
        .flex_col()
        .child(div().flex().items_center().gap(px(4.)).child(bar).child(dot))
        .child(
            div()
                .mt(px(8.))
                .flex()
                .flex_wrap()
                .gap(px(10.))
                .h(px(16.))
                .overflow_hidden()
                .text_size(px(10.))
                .text_color(palette.muted)
                .children(labeled.iter().map(|share| {
                    let pct = if share.pct < 10. {
                        format!("{:.1}%", share.pct)
                    } else {
                        format!("{:.0}%", share.pct)
                    };
                    crate::ui::tabular(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .whitespace_nowrap()
                            .child(
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(series_color(share.series, palette)),
                            )
                            .child(div().text_color(palette.foreground).child(share.label.clone()))
                            .child(pct),
                    )
                })),
        )
}
