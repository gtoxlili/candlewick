//! WatchlistSection settings.

use super::parts;
use crate::{
    app::{App, Topic},
    ui,
};
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Task, Window};

pub struct WatchlistSection {
    app: App,
    _observers: Vec<Task<()>>,
}
impl WatchlistSection {
    pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let observers = vec![ui::observe(&app, Topic::Locale, cx, |_, _| {})];
        Self { app, _observers: observers }
    }
}
impl Render for WatchlistSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::div()
            .flex()
            .flex_col()
            .gap_5()
            .child(parts::placeholder(t!("settings.watchlist.title"), &self.app, cx))
            .child(parts::placeholder(t!("settings.crypto.title"), &self.app, cx))
    }
}
