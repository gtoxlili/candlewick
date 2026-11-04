//! Overlay: read `frame`, market, viewport and analysis; report gestures through the model.

use super::model_state::ChartModel;
use gpui_kit::{Context, Entity, InteractiveElement, IntoElement, Render, Styled, Window, div};

pub struct Overlay {
    pub model: Entity<ChartModel>,
}
impl Render for Overlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("Overlay").size_full()
    }
}
