//! The settings window.

use crate::app::App;
use gpui_kit::{Context, IntoElement, Render, Task, Window};

pub struct SettingsView {
    app: App,
    _observers: Vec<Task<()>>,
}
impl SettingsView {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut observers = super::setup(&app, window, cx);
        observers
            .push(super::observe_title(&app, window, cx, |_, _| t!("window.settings").to_owned()));
        Self { app, _observers: observers }
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.app;
        super::frame(t!("window.settings").to_owned(), cx)
    }
}
