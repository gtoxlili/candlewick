//! Header: read the instrument, spec, market and mode; use model commands for choices and links.

use super::model_state::ChartModel;
use gpui_kit::{Context, Entity, InteractiveElement, IntoElement, Render, Styled, Window, div};

pub struct Hero {
    pub model: Entity<ChartModel>,
}
impl Render for Hero {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("Hero").size_full()
    }
}

pub struct ModeToggle {
    pub model: Entity<ChartModel>,
}
impl Render for ModeToggle {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("ModeToggle").size_full()
    }
}

pub struct LiveBadge {
    pub model: Entity<ChartModel>,
}
impl Render for LiveBadge {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("LiveBadge").size_full()
    }
}

pub struct InstrumentPicker {
    pub model: Entity<ChartModel>,
}
impl Render for InstrumentPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = self.model.read(cx);
        div().id("InstrumentPicker").size_full()
    }
}
