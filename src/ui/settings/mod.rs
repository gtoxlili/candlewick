//! The settings window.

use crate::app::App;
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct SettingsView {
    app: App,
}
impl SettingsView {
    pub fn new(app: App) -> Self {
        Self { app }
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.app;
        div().whitespace_nowrap().child(t!("window.settings").to_owned())
    }
}
