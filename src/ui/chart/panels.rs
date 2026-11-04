//! Panels: read the instrument, spec, market, viewport and analysis; update side tab and book step through the model.

use super::model_state::ChartModel;
use gpui_kit::{Context, Entity, InteractiveElement, IntoElement, Render, Styled, Window, div};

pub struct OrderBookPanel {
    pub model: Entity<ChartModel>,
}
impl Render for OrderBookPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("OrderBookPanel").size_full()
    }
}

pub struct TradesPanel {
    pub model: Entity<ChartModel>,
}
impl Render for TradesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("TradesPanel").size_full()
    }
}

pub struct HoverCard {
    pub model: Entity<ChartModel>,
}
impl Render for HoverCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("HoverCard").size_full()
    }
}

pub struct StatsGrid {
    pub model: Entity<ChartModel>,
}
impl Render for StatsGrid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("StatsGrid").size_full()
    }
}

pub struct FlowHeader {
    pub model: Entity<ChartModel>,
}
impl Render for FlowHeader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("FlowHeader").size_full()
    }
}
