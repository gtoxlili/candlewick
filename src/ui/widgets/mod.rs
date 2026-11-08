//! Pieces shared by more than one window.

use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::{
    App, Div, ElementId, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};

use super::{tabular, theme::Palette};

/// Source-defined quantity units, named once for every instrument consumer.
pub fn quantity_unit(unit: crate::market::QuantityUnit) -> &'static str {
    use crate::market::QuantityUnit;
    match unit {
        QuantityUnit::Tokens => t!("common.quantity.tokens"),
        QuantityUnit::Shares => t!("common.quantity.shares"),
        QuantityUnit::FundUnits => t!("common.quantity.fundUnits"),
        QuantityUnit::Grams => t!("common.quantity.grams"),
        QuantityUnit::Units => t!("common.quantity.units"),
    }
}

/// A signed change in the user's up/down colors.
pub fn change_badge(label: impl Into<SharedString>, change: f64, palette: Palette) -> Div {
    let color = if change > 0. {
        palette.up
    } else if change < 0. {
        palette.down
    } else {
        palette.muted
    };
    let fill = if change == 0. { palette.fill_strong } else { color.alpha(0.12) };
    tabular(
        div()
            .rounded_full()
            .px_2()
            .py_1()
            .bg(fill)
            .text_color(color)
            .whitespace_nowrap()
            .child(label.into()),
    )
}

/// A segmented control with a selected index and click callback.
pub fn pill_tabs(
    id: impl Into<ElementId>,
    labels: impl IntoIterator<Item = SharedString>,
    selected: usize,
    on_click: impl Fn(&usize, &mut Window, &mut App) + 'static,
) -> TabBar {
    TabBar::new(id)
        .segmented()
        .selected_index(selected)
        .children(labels.into_iter().map(|label| Tab::new().label(label).flex_1()))
        .on_click(on_click)
}

/// A translucent raised container for content.
pub fn panel(content: impl IntoElement, palette: Palette) -> Div {
    div().rounded(px(18.)).border_1().border_color(palette.border).bg(palette.panel).child(content)
}

#[derive(Clone, Copy)]
pub enum Status {
    Live,
    Busy,
    Error,
    Stale,
}

/// A colored dot for live, busy, error, or stale status.
pub fn status_dot(status: Status, palette: Palette) -> Div {
    let color = match status {
        Status::Live => palette.mint,
        Status::Busy => palette.amber,
        Status::Error => palette.red,
        Status::Stale => palette.faint,
    };
    div().size(px(6.)).rounded_full().bg(color)
}
