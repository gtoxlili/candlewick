//! The holdings window.

pub mod summary;

use crate::app::App;
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct HoldingsView {
    app: App,
}
impl HoldingsView {
    pub fn new(app: App) -> Self {
        Self { app }
    }
}
impl Render for HoldingsView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.app;
        div().whitespace_nowrap().child(t!("window.holdings").to_owned())
    }
}
