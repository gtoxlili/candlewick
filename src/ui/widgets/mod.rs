//! Pieces shared by more than one window.
// Removed when a window uses this module.
#![cfg_attr(not(test), allow(dead_code))]

use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::{
    App, Div, ElementId, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};

use super::{tabular, theme::Palette};

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

/// A compact label and numeric value on a tinted fill.
pub fn stat_tile(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    palette: Palette,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .rounded(px(10.))
        .bg(palette.fill)
        .p_3()
        .child(div().text_color(palette.muted).whitespace_nowrap().child(label.into()))
        .child(tabular(
            div().text_color(palette.foreground).whitespace_nowrap().child(value.into()),
        ))
}
