//! Analysis drawings and chart input.

mod paint;

use super::{
    analysis::{Press, Tool},
    geometry::{self, Frame, HitPart, Point},
    model::DrawingKind,
    model_state::ChartModel,
};
use crate::{format, ui::theme::Palette};
use gpui_kit::{
    Bounds, Context, CursorStyle, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PinchEvent, Pixels, Render, ScrollEvent, StatefulInteractiveElement, Styled, Window, canvas,
    div, px,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Pan {
    start: f64,
    dragging: bool,
    samples: [(f64, Option<Instant>); 8],
    next: usize,
}

impl Pan {
    fn new(start: f64) -> Self {
        Self { start, dragging: false, samples: [(0., None); 8], next: 0 }
    }

    fn moved(&mut self, x: f64, at: Instant) -> Option<(bool, f64)> {
        let started = !self.dragging;
        if started {
            if (x - self.start).abs() < 4. {
                return None;
            }
            self.dragging = true;
            self.start = x;
        }
        self.samples[self.next % 8] = (x, Some(at));
        self.next += 1;
        Some((started, x - self.start))
    }
}

#[derive(Debug, PartialEq)]
enum WheelAction {
    Scroll(f64),
    Zoom(f64),
}

fn wheel_action(dx: f64, dy: f64, control: bool) -> WheelAction {
    if control {
        WheelAction::Zoom((dy * 0.01).exp())
    } else if dx.abs() > dy.abs() {
        WheelAction::Scroll(dx)
    } else {
        WheelAction::Zoom((dy * 0.002).exp())
    }
}

fn candle_at(
    candles: &[crate::market::Candle],
    time: f64,
    interval: f64,
) -> Option<crate::market::Candle> {
    let index = candles.partition_point(|c| c.time <= time).checked_sub(1)?;
    let candle = candles[index];
    (time < candle.time + interval).then_some(candle)
}

pub struct Overlay {
    model: Entity<ChartModel>,
    focus: FocusHandle,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    cache: Rc<RefCell<paint::TextCache>>,
    pan: Option<Pan>,
    last_drag: Option<Instant>,
    free: bool,
}

impl Overlay {
    pub fn new(model: Entity<ChartModel>, cx: &mut Context<Self>) -> Self {
        Self {
            model,
            focus: cx.focus_handle(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            cache: Rc::new(RefCell::new(paint::TextCache::default())),
            pan: None,
            last_drag: None,
            free: false,
        }
    }
    fn local(&self, p: gpui_kit::Point<Pixels>) -> Point {
        let origin = self.bounds.get().origin;
        Point { x: f32::from(p.x - origin.x) as f64, y: f32::from(p.y - origin.y) as f64 }
    }
    fn frame_at(model: &ChartModel, p: Point) -> Option<Frame> {
        let mut frame = model.frame.get()?;
        let map = geometry::mapping(&frame);
        if map.plot().contains(p) {
            let candles = model.analysis().candles();
            let interval = model.market().map_or(60., |m| f64::from(m.interval()));
            let time = map.time(p.x);
            frame.hover = candle_at(candles, time, interval);
        } else {
            frame.hover = None;
        }
        Some(frame)
    }
    fn down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.free = e.modifiers.alt;
        let p = self.local(e.position);
        let double = e.click_count >= 2
            && self.last_drag.is_none_or(|t| t.elapsed() > Duration::from_millis(500));
        let press = self.model.update(cx, |model, cx| {
            model.analysis_mut().panning = false;
            model.viewport_mut().set_now(epoch_now());
            let frame = Self::frame_at(model, p);
            let interval = model.market().map_or(60., |m| f64::from(m.interval()));
            if double && model.analysis().tool == Tool::Cursor && model.analysis().hovered.is_none()
            {
                model.go_live(cx);
                return Press::Draw;
            }
            let press =
                model.analysis_mut().pointer_down(p, frame.as_ref(), interval, e.modifiers.alt);
            model.analysis_changed(cx);
            press
        });
        self.pan = (press == Press::Pan).then(|| Pan::new(p.x));
        cx.notify();
    }
    fn moved(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.free = e.modifiers.alt;
        let p = self.local(e.position);
        self.model.update(cx, |model, cx| {
            let frame = Self::frame_at(model, p);
            let interval = model.market().map_or(60., |m| f64::from(m.interval()));
            model.analysis_mut().pointer_move(p, frame.as_ref(), interval, e.modifiers.alt);
            model.analysis_changed(cx);
            cx.notify();
        });
        if let Some(pan) = &mut self.pan
            && let Some((started, dx)) = pan.moved(p.x, Instant::now())
        {
            if started {
                self.model.update(cx, |m, cx| {
                    m.analysis_mut().panning = true;
                    m.viewport_mut().set_now(epoch_now());
                    m.viewport_mut().drag_start();
                    m.viewport_changed(cx);
                });
            }
            self.model.update(cx, |m, cx| {
                m.viewport_mut().drag_to(dx);
                m.viewport_changed(cx);
            });
        }
        cx.notify();
    }
    fn up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.pan.is_none() && !self.model.read(cx).analysis().dragging() {
            return;
        }
        let p = self.local(e.position);
        self.model.update(cx, |m, cx| {
            m.analysis_mut().panning = false;
            m.analysis_mut().pointer_up();
            if !self.bounds.get().contains(&e.position) {
                m.analysis_mut().pointer_leave();
            }
            m.analysis_changed(cx);
            cx.notify();
        });
        if let Some(pan) = self.pan.take()
            && pan.dragging
        {
            self.last_drag = Some(Instant::now());
            let now = Instant::now();
            let earliest = pan
                .samples
                .iter()
                .filter_map(|(x, t)| t.map(|t| (*x, t)))
                .filter(|(_, t)| now.duration_since(*t) <= Duration::from_millis(100))
                .min_by_key(|(_, t)| *t);
            let speed = earliest.map_or(0., |(x, t)| {
                let ms = now.duration_since(t).as_secs_f64() * 1000.;
                if ms > 0. { (p.x - x) / ms } else { 0. }
            });
            let glide = self.model.update(cx, |m, cx| {
                m.viewport_mut().set_now(epoch_now());
                let glide = m.viewport_mut().drag_end(speed);
                m.viewport_changed(cx);
                glide
            });
            if glide {
                Self::glide(window, cx);
            }
        }
        cx.notify();
    }
    fn glide(window: &mut Window, cx: &mut Context<Self>) {
        cx.on_next_frame(window, |this, window, cx| {
            let again = this.model.update(cx, |m, cx| {
                m.viewport_mut().set_now(epoch_now());
                let again = m.viewport_mut().glide_tick();
                m.viewport_changed(cx);
                again
            });
            if again {
                Self::glide(window, cx);
            }
        });
    }
    fn scroll(&mut self, e: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = e.delta.pixel_delta(px(16.));
        let dx = -(f32::from(delta.x) as f64);
        let dy = -(f32::from(delta.y) as f64);
        let x = self.local(e.position).x;
        self.model.update(cx, |m, cx| {
            m.viewport_mut().set_now(epoch_now());
            match wheel_action(dx, dy, e.modifiers.control) {
                WheelAction::Scroll(dx) => m.viewport_mut().scroll_by(dx),
                WheelAction::Zoom(factor) => m.viewport_mut().zoom(factor, x),
            }
            m.viewport_changed(cx);
        });
        cx.stop_propagation();
    }
    fn pinch(&mut self, e: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        let x = self.local(e.position).x;
        self.model.update(cx, |m, cx| {
            m.viewport_mut().set_now(epoch_now());
            m.viewport_mut().zoom(1. / (1. + f64::from(e.delta)).max(0.01), x);
            m.viewport_changed(cx);
        });
        cx.stop_propagation();
    }
    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        let mods = e.keystroke.modifiers;
        let handled = self.model.update(cx, |m, cx| {
            let a = m.analysis_mut();
            let handled = match key {
                "escape" => a.cancel(),
                "backspace" | "delete" if a.selected.is_some() => {
                    a.remove_selected();
                    true
                }
                "z" if (mods.platform || mods.control) && !mods.shift => {
                    a.undo();
                    true
                }
                key if mods.alt && !mods.platform && !mods.control => {
                    let tool = match key {
                        "v" => Some(Tool::Cursor),
                        "h" => Some(Tool::Drawing(DrawingKind::Level)),
                        "t" => Some(Tool::Drawing(DrawingKind::Trend)),
                        "f" => Some(Tool::Drawing(DrawingKind::Fib)),
                        "m" => Some(Tool::Drawing(DrawingKind::Measure)),
                        _ => None,
                    };
                    if let Some(tool) = tool {
                        a.set_tool(tool);
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
            m.analysis_changed(cx);
            handled
        });
        if handled {
            cx.stop_propagation();
        }
    }
}

fn epoch_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0., |d| d.as_secs_f64())
}

impl Render for Overlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.clone();
        let state = model.read(cx);
        let palette = Palette::current(state.app(), cx);
        let cursor = if self.pan.as_ref().is_some_and(|p| p.dragging) || state.analysis().dragging()
        {
            CursorStyle::ClosedHand
        } else if state.analysis().tool != Tool::Cursor {
            CursorStyle::Crosshair
        } else if let Some(hit) = &state.analysis().hovered {
            if hit.part == HitPart::Body {
                CursorStyle::PointingHand
            } else {
                CursorStyle::OpenHand
            }
        } else {
            CursorStyle::Crosshair
        };
        let cache = self.cache.clone();
        let bounds = self.bounds.clone();
        let panning = self.pan.as_ref().is_some_and(|p| p.dragging);
        let pressed = self.pan.is_some() || state.analysis().dragging();
        let free = self.free;
        let weak = cx.weak_entity();
        div()
            .id("Overlay")
            .size_full()
            .track_focus(&self.focus)
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::down))
            .on_mouse_move(cx.listener(Self::moved))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::up))
            .on_scroll(cx.listener(Self::scroll))
            .on_pinch(cx.listener(Self::pinch))
            .on_key_down(cx.listener(Self::key))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.pan.is_none() && !this.model.read(cx).analysis().dragging() {
                    this.model.update(cx, |m, cx| {
                        m.analysis_mut().pointer_leave();
                        m.analysis_changed(cx);
                        cx.notify();
                    });
                }
            }))
            .child(
                canvas(
                    move |rect, _, _| {
                        bounds.set(rect);
                    },
                    move |bounds, _, window, cx| {
                        if pressed {
                            let weak = weak.clone();
                            window.on_mouse_event(
                                move |event: &MouseMoveEvent, phase, window, cx| {
                                    if phase == gpui_kit::DispatchPhase::Bubble
                                        && !bounds.contains(&event.position)
                                    {
                                        let _ = weak
                                            .update(cx, |this, cx| this.moved(event, window, cx));
                                    }
                                },
                            );
                        }
                        model.update(cx, |model, cx| {
                            let Some(mut frame) = model.frame.get() else {
                                return;
                            };
                            let interval = model.market().map_or(60., |m| f64::from(m.interval()));
                            if let Some(pointer) = model.analysis().pointer
                                && let Some(next) = Self::frame_at(model, pointer)
                            {
                                frame.hover = next.hover;
                                let before =
                                    (model.analysis().snapped, model.analysis().hovered_candle);
                                model.analysis_mut().pointer_move(
                                    pointer,
                                    Some(&frame),
                                    interval,
                                    free,
                                );
                                let after =
                                    (model.analysis().snapped, model.analysis().hovered_candle);
                                model.analysis_changed(cx);
                                if before != after {
                                    cx.notify();
                                }
                            }
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
                                    panning,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_gestures_pan_horizontally_and_zoom_vertically() {
        assert_eq!(wheel_action(-24., 3., false), WheelAction::Scroll(-24.));
        let WheelAction::Zoom(factor) = wheel_action(0., 100., false) else { panic!() };
        assert!((factor - 1.221_402_758_160_169_9).abs() < 1e-12);
        let WheelAction::Zoom(pinch) = wheel_action(0., -10., true) else { panic!() };
        assert!((pinch - 0.904_837_418_035_959_5).abs() < 1e-12);
    }

    #[test]
    fn pointer_slop_does_not_move_the_view_before_a_drag_starts() {
        let now = Instant::now();
        let mut pan = Pan::new(100.);
        assert_eq!(pan.moved(103., now), None);
        assert_eq!(pan.moved(104., now), Some((true, 0.)));
        assert_eq!(pan.moved(144., now), Some((false, 40.)));
    }

    #[test]
    fn hover_finds_the_candle_covering_time_and_leaves_gaps_empty() {
        let candle = |time| crate::market::Candle { time, open: 1., high: 1., low: 1., close: 1. };
        let candles = [candle(100.), candle(220.)];
        let at = |time| candle_at(&candles, time, 60.).map(|c| c.time);
        assert_eq!(at(99.), None);
        assert_eq!(at(100.), Some(100.));
        assert_eq!(at(159.9), Some(100.));
        assert_eq!(at(160.), None);
        assert_eq!(at(220.), Some(220.));
        assert_eq!(at(280.), None);
    }
}
