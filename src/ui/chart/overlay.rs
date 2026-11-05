//! Analysis drawings over the price chart.

mod paint;

use std::{cell::RefCell, rc::Rc};

use super::model_state::ChartModel;
use crate::{format, ui::theme::Palette};
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled, Window,
    canvas, div,
};

pub struct Overlay {
    model: Entity<ChartModel>,
    cache: Rc<RefCell<paint::TextCache>>,
}
impl Overlay {
    pub fn new(model: Entity<ChartModel>, _: &mut Context<Self>) -> Self {
        Self { model, cache: Rc::new(RefCell::new(paint::TextCache::default())) }
    }
}
impl Render for Overlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.clone();
        let palette = Palette::current(model.read(cx).app(), cx);
        let cache = self.cache.clone();
        div().id("Overlay").size_full().child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    model.update(cx, |model, cx| {
                        let Some(frame) = model.frame.get() else { return };
                        let interval = model.market().map_or(60., |m| f64::from(m.interval()));
                        let decimals = format::decimals(
                            model.market().and_then(|m| m.stats()).map_or(0., |s| s.last),
                            model.instrument().and_then(|i| i.decimals),
                        );
                        let day_offset = model.spec().map_or(0, |s| s.day_offset);
                        paint::paint(
                            bounds,
                            frame,
                            model.analysis(),
                            paint::PaintOptions {
                                palette,
                                interval,
                                decimals,
                                day_offset,
                                panning: false,
                            },
                            &mut cache.borrow_mut(),
                            window,
                            cx,
                        );
                    });
                },
            )
            .size_full(),
        )
    }
}
