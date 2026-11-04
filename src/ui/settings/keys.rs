//! ApiKeysSection settings.

use super::parts;
use crate::{
    app::{App, Topic},
    ui,
};
use gpui_kit::{Context, IntoElement, Render, Task, Window};

pub struct ApiKeysSection {
    app: App,
    _observers: Vec<Task<()>>,
}
impl ApiKeysSection {
    pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let observers = vec![ui::observe(&app, Topic::Locale, cx, |_, _| {})];
        Self { app, _observers: observers }
    }
}
impl Render for ApiKeysSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        parts::placeholder(t!("settings.holdings.title"), &self.app, cx)
    }
}
