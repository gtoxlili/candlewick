//! The chart window.

pub mod analysis;
pub mod geometry;
pub mod indicators;
pub mod labels;
pub mod market;
pub mod model;
pub mod viewport;

use crate::app::App;
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct ChartView {
    app: App,
}
impl ChartView {
    pub fn new(app: App) -> Self {
        Self { app }
    }
}
impl Render for ChartView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.app;
        div().whitespace_nowrap().child({
            let model = self.app.shared().model();
            model
                .chart
                .as_deref()
                .and_then(|id| model.instrument(id))
                .map(|instrument| t!("window.chart", pair = instrument.pair_label()))
                .unwrap_or_default()
        })
    }
}
