//! Price canvas: read the market, viewport, mode and spec; write the painted geometry to `frame`.

use super::model_state::ChartModel;
use gpui_kit::{Context, Entity, InteractiveElement, IntoElement, Render, Styled, Window, div};

pub struct PriceCanvas {
    pub model: Entity<ChartModel>,
}
impl Render for PriceCanvas {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("PriceCanvas").size_full()
    }
}
