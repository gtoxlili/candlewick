//! Analysis toolbar: read and change analysis via the model, then call `analysis_changed`.

use super::model_state::ChartModel;
use gpui_kit::{Context, Entity, InteractiveElement, IntoElement, Render, Styled, Window, div};

pub struct AnalysisBar {
    pub model: Entity<ChartModel>,
}
impl Render for AnalysisBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("AnalysisBar").size_full()
    }
}
