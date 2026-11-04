//! The chart window.

pub mod analysis;
pub mod geometry;
pub mod indicators;
pub mod labels;
pub mod market;
pub mod model;
pub mod model_state;
pub mod viewport;

use crate::app::App;
use gpui_kit::{Context, IntoElement, Render, Task, Window};

pub struct ChartView {
    app: App,
    _observers: Vec<Task<()>>,
}
impl ChartView {
    pub fn new(app: App, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut observers = super::setup(&app, window, cx);
        observers.push(super::observe(&app, crate::app::Topic::ChartInstrument, cx, |_, _| {}));
        observers.push(super::observe_title(&app, window, cx, |view, _| view.title()));
        Self { app, _observers: observers }
    }
}
impl ChartView {
    fn title(&self) -> String {
        let model = self.app.shared().model();
        model
            .chart
            .as_deref()
            .and_then(|id| model.instrument(id))
            .map(|instrument| t!("window.chart", pair = instrument.pair_label()))
            .unwrap_or_default()
    }
}
impl Render for ChartView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::frame(self.title(), cx)
    }
}
