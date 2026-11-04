//! The holdings window.

pub mod summary;

use crate::app::App;
use gpui_kit::{Context, IntoElement, Render, Task, Window};

pub struct HoldingsView {
    app: App,
    _observers: Vec<Task<()>>,
}
impl HoldingsView {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut observers = super::setup(&app, window, cx);
        observers
            .push(super::observe_title(&app, window, cx, |_, _| t!("window.holdings").to_owned()));
        Self { app, _observers: observers }
    }
}
impl Render for HoldingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.app;
        super::frame(t!("window.holdings").to_owned(), cx)
    }
}
