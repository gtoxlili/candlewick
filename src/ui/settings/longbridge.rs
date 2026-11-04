//! LongbridgeSection settings.

use super::parts;
use crate::{
    app::{App, Topic},
    ui,
};
use gpui_kit::{Context, IntoElement, Render, Task, Window};

pub struct LongbridgeSection {
    app: App,
    _observers: Vec<Task<()>>,
}
impl LongbridgeSection {
    pub fn new(app: App, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let observers = vec![ui::observe(&app, Topic::Locale, cx, |_, _| {})];
        Self { app, _observers: observers }
    }
}
impl Render for LongbridgeSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        parts::placeholder(t!("common.provider.longbridge"), &self.app, cx)
    }
}
