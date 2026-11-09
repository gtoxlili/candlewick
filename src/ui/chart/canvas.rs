//! Price chart canvas.
//!
//! The canvas owns the chart's animation state. It advances that state while
//! it renders — before its own paint and before the sibling views that read
//! the frame it writes — and asks for the next frame only while something is
//! still moving. Text is shaped once per value and colour, and formatted once
//! per value, so a settled chart paints without allocating.

mod math;
mod reform;
mod state;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use gpui_kit::{
    App as GpuiApp, Bounds, BoxShadow, ContentMask, Context, Entity, EntityId, Hsla,
    InteractiveElement, IntoElement, ParentElement, Path, PathBuilder, Pixels, Point, Render, Rgba,
    ShapedLine, SharedString, Styled, TestSupportExt as _, TextAlign, TextRun, Window, canvas, div,
    fill, font, linear_color_stop, linear_gradient, point, px, quad, rgb, size,
};

use super::{geometry::Frame, labels, market::visible_stats, model_state::ChartModel};
use crate::{
    format,
    ui::{prefs::Mode, theme::Palette},
};
use state::{Input, State};

/// How far the left edge erases what it covers, in CSS pixels.
const LEFT_FADE: f64 = 40.0;
/// The least room between two price labels: one line of axis text.
const GRID_LABEL_GAP: f64 = 14.0;
/// The label cache keys text by what it shows, in one of these roles.
const LABEL_GRID: u8 = 0;
const LABEL_TIME: u8 = 1;
const LABEL_EMPTY: u8 = 2;
const LABEL_BADGE: u8 = 3;
const LABEL_TEMPLATE: u8 = 4;

pub struct PriceCanvas {
    pub model: Entity<ChartModel>,
    /// Views that paint what the canvas writes into `model.frame`.
    pub frame_readers: [EntityId; 2],
    painter: Rc<RefCell<Painter>>,
    /// One next-frame callback is enough while frames are not delivered.
    tick_scheduled: bool,
}

impl PriceCanvas {
    pub fn new(model: Entity<ChartModel>, frame_readers: [EntityId; 2]) -> Self {
        Self {
            model,
            frame_readers,
            painter: Rc::new(RefCell::new(Painter::new())),
            tick_scheduled: false,
        }
    }
}

impl Render for PriceCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = gpui_kit::component::Theme::global(cx).is_dark();
        let palette = {
            let model = self.model.read(cx);
            Palette::current(model.app(), cx)
        };
        self.painter.borrow_mut().set_dark(dark);
        // The frame is advanced after this one is drawn, so every view that
        // paints it paints the same one: the canvas, the overlay above it, and
        // the hover card.
        self.schedule_tick(window, cx);
        let painter = self.painter.clone();
        let prepaint_painter = painter.clone();
        let model = self.model.clone();
        let prepaint_model = model.clone();
        div().id("PriceCanvas").test_support().size_full().child(
            canvas(
                move |bounds, window, cx| {
                    let mut painter = prepaint_painter.borrow_mut();
                    if painter.bounds != Some(bounds) {
                        // The first frame, or a window that changed size: lay the
                        // chart out again here, where the size is exact, with no
                        // time passing, and let the tick after this frame keep
                        // animating from the new size.
                        painter.advance(&prepaint_model, bounds, window, cx, Some(0.0));
                    }
                },
                move |bounds, (), window, cx| {
                    let mut painter = painter.borrow_mut();
                    draw(&mut painter, palette, dark, bounds, &model, window, cx);
                },
            )
            .size_full(),
        )
    }
}

impl PriceCanvas {
    /// Ask for one tick after the current frame is drawn. Held back while one
    /// is already outstanding, so a window the platform stops drawing does not
    /// collect callbacks.
    fn schedule_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tick_scheduled {
            return;
        }
        self.tick_scheduled = true;
        let canvas = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            if let Some(canvas) = canvas.upgrade() {
                canvas.update(cx, |canvas, cx| canvas.tick(window, cx));
            }
        });
    }

    /// Advance the chart's animation by the time since the last tick, and mark
    /// what paints it for the next frame. A settled chart marks nothing, so no
    /// frame follows.
    fn tick(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.tick_scheduled = false;
        let Some(bounds) = self.painter.borrow().bounds else {
            return;
        };
        let (changed, moving) = {
            let mut painter = self.painter.borrow_mut();
            let changed = painter.advance(&self.model, bounds, window, cx, None);
            (changed, painter.chart.moving)
        };
        if changed {
            for reader in self.frame_readers {
                GpuiApp::notify(cx, reader);
            }
        }
        if changed || moving {
            cx.notify();
        }
    }
}

#[derive(Default)]
struct TextCache {
    /// The formatted text, keyed by what it shows: a settled label formats
    /// once, however often its alpha moves.
    strings: HashMap<(u8, u64, usize), SharedString>,
    /// The shaped line at one alpha step, keyed as the strings are plus the
    /// step.
    lines: HashMap<(u8, u64, usize, u8), ShapedLine>,
}

impl TextCache {
    fn clear(&mut self) {
        self.strings.clear();
        self.lines.clear();
    }

    fn evict(&mut self) {
        if self.strings.len() > 512 || self.lines.len() > 4096 {
            self.clear();
        }
    }

    /// The shaped line for `value`, or `None` when it is fully transparent.
    #[allow(clippy::too_many_arguments)]
    fn line<'a>(
        &'a mut self,
        kind: u8,
        value: f64,
        extra: usize,
        alpha: f64,
        color: Hsla,
        pixels: f32,
        window: &Window,
        text: impl FnOnce() -> String,
    ) -> Option<&'a ShapedLine> {
        let level = (alpha.clamp(0.0, 1.0) * 100.0).round() as u8;
        if level == 0 {
            return None;
        }
        let key = (kind, value.to_bits(), extra, level);
        if !self.lines.contains_key(&key) {
            self.evict();
            let text = self
                .strings
                .entry((kind, value.to_bits(), extra))
                .or_insert_with(|| SharedString::from(text()))
                .clone();
            let text_font = if cfg!(target_os = "macos") {
                font(if kind == LABEL_EMPTY { ".SystemUIFont" } else { "SF Mono" })
            } else {
                gpui_kit::Font {
                    features: crate::ui::tabular_features(),
                    ..window.text_style().font()
                }
            };
            let run = TextRun {
                len: text.len(),
                font: text_font,
                color: rgba(color, f64::from(level) / 100.0),
                ..Default::default()
            };
            let line = window.text_system().shape_line(text, px(pixels), &[run], None);
            self.lines.insert(key, line);
        }
        self.lines.get(&key)
    }
}

struct Painter {
    chart: State,
    last: Option<Instant>,
    text: TextCache,
    dark: bool,
    /// The price labels drawn this frame.
    visible_grid: Vec<state::Tick>,
    /// The time labels drawn this frame: tick, text width, and the span it reads for.
    visible_times: Vec<(state::Tick, f64, f64)>,
    badge_width: f64,
    dt: f64,
    day_anchor: f64,
    axis_key: Option<(u64, usize)>,
    axis_width: f64,
    /// The element bounds the state was last advanced for; `None` until the
    /// first paint tells them.
    bounds: Option<Bounds<Pixels>>,
}

impl Painter {
    fn new() -> Self {
        Self {
            chart: State::default(),
            last: None,
            text: TextCache::default(),
            dark: false,
            visible_grid: Vec::new(),
            visible_times: Vec::new(),
            badge_width: 0.0,
            dt: 16.67,
            day_anchor: local_day_anchor(),
            axis_key: None,
            axis_width: 64.0,
            bounds: None,
        }
    }

    fn set_dark(&mut self, dark: bool) {
        if self.dark != dark {
            self.dark = dark;
            self.text.clear();
        }
    }

    /// Advance the chart's animation by one frame and write the geometry it
    /// will paint into the model's shared frame. Returns whether the geometry
    /// other layers paint over moved (a new frame written with only a new
    /// clock does not count). `dt` overrides the measured time (0 re-lays out
    /// a frame that only changed size).
    fn advance(
        &mut self,
        model: &Entity<ChartModel>,
        bounds: Bounds<Pixels>,
        window: &Window,
        cx: &mut GpuiApp,
        dt: Option<f64>,
    ) -> bool {
        let clock = cx.background_executor().now();
        let dt = dt.unwrap_or_else(|| {
            self.last
                .map_or(16.67, |last| clock.saturating_duration_since(last).as_secs_f64() * 1000.0)
                .min(50.0)
        });
        self.last = Some(clock);
        self.dt = dt;
        self.bounds = Some(bounds);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        let width = f64::from(f32::from(bounds.size.width));
        let height = f64::from(f32::from(bounds.size.height));
        let mouse = window.mouse_position();
        let mx = f64::from(f32::from(mouse.x - bounds.origin.x));
        let my = f64::from(f32::from(mouse.y - bounds.origin.y));
        let reduce_motion = cx.reduce_motion();
        let chart = &mut self.chart;
        let axis_key = &mut self.axis_key;
        let axis_width = &mut self.axis_width;
        let day_anchor = self.day_anchor;
        model.update(cx, |model, cx| {
            model.update_clock(now);
            let now = model.chart_now(now);
            let line_mode = model.mode() == Mode::Line;
            let tick_decimals = model.instrument().and_then(|i| i.decimals);
            let series = model.generation();
            let dragging = model.analysis().dragging();
            let frame = model.frame.clone();
            let (market, viewport, _) = model.parts_mut();
            let glide = viewport.glide_tick();
            let Some(market) = market else {
                chart.advance(
                    dt,
                    Input {
                        candles: &[],
                        live: None,
                        line: &[],
                        price: None,
                        series,
                        candle_width: 60.0,
                        now,
                        day_anchor,
                        width,
                        height,
                        axis_width: 64.0,
                        loading: false,
                        line_mode: false,
                        hover_x: None,
                        reduce_motion,
                    },
                    &mut viewport.view,
                );
                chart.moving |= glide;
                return write_frame(&frame, chart.frame);
            };
            let data = market.chart_data();
            let price = data.price.or(data.live.map(|c| c.close)).unwrap_or(1.0);
            let decimals = format::decimals(price, tick_decimals);
            let key = (price.to_bits(), decimals);
            if *axis_key != Some(key) {
                *axis_width = (16.0
                    + 6.8
                        * format::price(if price == 0.0 { 1.0 } else { price }, decimals).len()
                            as f64)
                    .max(64.0);
                *axis_key = Some(key);
            }
            let hover_x =
                (my >= 0.0 && my <= height && !dragging && !viewport.dragging()).then_some(mx);
            let input = Input {
                candles: data.candles,
                live: data.live,
                line: data.line,
                price: data.price,
                series,
                candle_width: f64::from(data.interval),
                now,
                day_anchor,
                width,
                height,
                axis_width: *axis_width,
                loading: !data.loaded && data.candles.is_empty() && data.live.is_none(),
                line_mode,
                hover_x,
                reduce_motion,
            };
            chart.advance(dt, input, &mut viewport.view);
            chart.moving |= glide;
            let visible = match (viewport.view.left, viewport.view.right) {
                (Some(left), Some(right)) => {
                    visible_stats(&data, left, right, right - left, viewport.view.end.is_none())
                }
                _ => None,
            };
            let changed = write_frame(&frame, chart.frame);
            model.set_visible_stats(visible, cx);
            changed
        })
    }
}

/// Keep the frame the other layers read in step with the one just written.
/// Returns whether the geometry they paint over moved: a fresh clock alone
/// does not count, or a chart that is merely ticking would repaint them.
fn write_frame(frame: &Rc<Cell<Option<Frame>>>, next: Option<Frame>) -> bool {
    let previous = frame.get();
    if previous == next {
        return false;
    }
    frame.set(next);
    match (previous, next) {
        (Some(before), Some(after)) => !before.same_geometry(&after),
        (None, None) => false,
        _ => true,
    }
}

fn local_day_anchor() -> f64 {
    let t = 1_704_110_400_i64;
    let parts = format::local_time_parts(t);
    let day_shift = if parts.year < 2024 {
        -1.0
    } else if parts.day > 1 {
        1.0
    } else {
        0.0
    };
    t as f64 - f64::from(parts.hour * 3600 + parts.minute * 60 + parts.second) - day_shift * 86400.0
}

fn rgba(color: Hsla, alpha: f64) -> Hsla {
    let mut c: Rgba = color.into();
    c.a *= alpha.clamp(0.0, 1.0) as f32;
    c.into()
}
fn mix(a: Hsla, b: Hsla, t: f64) -> Hsla {
    let a: Rgba = a.into();
    let b: Rgba = b.into();
    let t = t.clamp(0.0, 1.0) as f32;
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
    .into()
}
fn rect(bounds: Bounds<Pixels>, x: f64, y: f64, w: f64, h: f64) -> Bounds<Pixels> {
    Bounds::new(
        bounds.origin + point(px(x as f32), px(y as f32)),
        size(px(w.max(0.0) as f32), px(h.max(0.0) as f32)),
    )
}
fn at(bounds: Bounds<Pixels>, p: (f64, f64)) -> Point<Pixels> {
    bounds.origin + point(px(p.0 as f32), px(p.1 as f32))
}
#[allow(clippy::too_many_arguments)]
fn square(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    color: Hsla,
    radius: f64,
) {
    window.paint_quad(quad(
        rect(bounds, x, y, w, h),
        px(radius as f32),
        color,
        px(0.0),
        gpui_kit::transparent_black(),
        Default::default(),
    ));
}

/// Which format a time label takes over `span`, as `labels::axis_label`
/// chooses it: seconds, minutes, or dates. Part of the label's cache key.
fn label_kind(span: f64) -> usize {
    if span <= 300.0 {
        0
    } else if span <= 172_800.0 {
        1
    } else {
        2
    }
}

/// The share of a mark at `x` the left edge leaves visible.
fn fade_at(x: f64, plot_x: f64) -> f64 {
    ((x - plot_x) / LEFT_FADE).clamp(0.0, 1.0)
}

/// A dashed horizontal line. `fade` erases it over the first 40 px from
/// `plot_x`, one dash at a time, so nothing is allocated to erase it.
#[allow(clippy::too_many_arguments)]
fn dashed(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    x: f64,
    y: f64,
    width: f64,
    dash: f64,
    gap: f64,
    color: Hsla,
    fade: bool,
    plot_x: f64,
) {
    let mut p = 0.0;
    while p < width {
        let length = (width - p).min(dash);
        let alpha = if fade { fade_at(x + p, plot_x) } else { 1.0 };
        if alpha > 0.003 {
            window.paint_quad(fill(rect(bounds, x + p, y, length, 1.0), rgba(color, alpha)));
        }
        p += dash + gap;
    }
}

/// The cubic path of the line, filled down to `bottom` when asked.
fn path(
    bounds: Bounds<Pixels>,
    points: &[(f64, f64)],
    controls: &[[(f64, f64); 3]],
    fill_path: bool,
    bottom: f64,
) -> Option<Path<Pixels>> {
    let first = *points.first()?;
    let mut path = if fill_path { PathBuilder::fill() } else { PathBuilder::stroke(px(2.0)) };
    path.move_to(at(bounds, first));
    for [a, b, end] in controls {
        path.cubic_bezier_to(at(bounds, *end), at(bounds, *a), at(bounds, *b));
    }
    if fill_path {
        path.line_to(at(bounds, (points.last()?.0, bottom)));
        path.line_to(at(bounds, (first.0, bottom)));
        path.close();
    }
    path.build().ok()
}

/// The area under the line between two x positions, down to `bottom`. Built
/// from the sampled points rather than the spline: it only ever paints the
/// 40 px the left edge fades out, where the difference is sub-pixel.
fn area_band(
    bounds: Bounds<Pixels>,
    points: &[(f64, f64)],
    x0: f64,
    x1: f64,
    bottom: f64,
) -> Option<Path<Pixels>> {
    let interp = |x: f64| -> Option<f64> {
        let mut previous = points.first().copied()?;
        for point in &points[1..] {
            if point.0 >= x {
                let dx = point.0 - previous.0;
                let t = if dx > 0.0 { ((x - previous.0) / dx).clamp(0.0, 1.0) } else { 0.0 };
                return Some(previous.1 + (point.1 - previous.1) * t);
            }
            previous = *point;
        }
        Some(previous.1)
    };
    let y0 = interp(x0)?;
    let y1 = interp(x1)?;
    let mut path = PathBuilder::fill();
    path.move_to(at(bounds, (x0, y0)));
    for point in points.iter().filter(|p| p.0 > x0 && p.0 < x1) {
        path.line_to(at(bounds, *point));
    }
    path.line_to(at(bounds, (x1, y1)));
    path.line_to(at(bounds, (x1, bottom)));
    path.line_to(at(bounds, (x0, bottom)));
    path.close();
    path.build().ok()
}

/// One frame's ink: the theme's neutrals at the alphas the chart draws them.
#[derive(Clone, Copy)]
struct Ink {
    grid: Hsla,
    label: Hsla,
    time: Hsla,
}

impl Ink {
    fn new(dark: bool) -> Self {
        let white: Hsla = rgb(if dark { 0xffffff } else { 0x000000 }).into();
        Self {
            grid: rgba(white, 0.06),
            label: rgba(white, if dark { 0.4 } else { 0.35 }),
            time: rgba(white, if dark { 0.35 } else { 0.3 }),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    painter: &mut Painter,
    palette: Palette,
    dark: bool,
    bounds: Bounds<Pixels>,
    model: &Entity<ChartModel>,
    window: &mut Window,
    cx: &mut GpuiApp,
) {
    let decimals = {
        let model = model.read(cx);
        let price = model.market().and_then(|m| m.stats()).map_or(0.0, |s| s.last);
        format::decimals(price, model.instrument().and_then(|i| i.decimals))
    };
    let ink = Ink::new(dark);
    let state = &painter.chart;
    let Some(frame) = state.frame else {
        draw_loading(state, bounds, palette, ink.label, &mut painter.text, window, cx);
        return;
    };
    let px0 = frame.plot_x;
    let py = frame.plot_y;
    let pw = frame.plot_w;
    let ph = frame.plot_h;
    painter.visible_grid.clear();
    for tick in &state.grid {
        if tick.alpha <= 0.0 || tick.pos < py - 10.0 || tick.pos > py + ph + 10.0 {
            continue;
        }
        dashed(window, bounds, px0, tick.pos, pw, 1.0, 3.0, rgba(ink.grid, tick.alpha), true, px0);
        // Labels closer than a line apart, as when the range grows fast, give
        // way to the more visible one.
        match painter.visible_grid.last_mut() {
            Some(previous) if tick.pos - previous.pos < GRID_LABEL_GAP => {
                if tick.alpha > previous.alpha {
                    *previous = *tick;
                }
            }
            _ => painter.visible_grid.push(*tick),
        }
    }
    for tick in &painter.visible_grid {
        let shaped = painter.text.line(
            LABEL_GRID,
            tick.value,
            decimals,
            tick.alpha,
            ink.label,
            11.0,
            window,
            || format::price(tick.value, decimals),
        );
        if let Some(shaped) = shaped {
            let _ = shaped.paint(
                at(bounds, (px0 + pw + 8.0, tick.pos - 7.0)),
                px(14.0),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
    let clip = ContentMask { bounds: rect(bounds, px0 - 1.0, py, pw + 2.0, ph) };
    window.with_content_mask(Some(clip), |window| {
        if state.line_presence > 0.001 && state.points.len() > 1 {
            if state.line_mode > 0.01
                && let Some(fill_path) = path(bounds, &state.points, &state.controls, true, py + ph)
            {
                let fill_alpha = state.line_presence
                    * state.line_mode
                    * state.reveal
                    * state.dim
                    * if dark { 0.12 } else { 0.08 };
                let gradient = |alpha| {
                    linear_gradient(
                        180.0,
                        linear_color_stop(rgba(palette.glow, alpha), 0.0),
                        linear_color_stop(rgba(palette.glow, 0.0), 1.0),
                    )
                };
                let split = state
                    .hover_x
                    .filter(|_| state.scrub > 0.0)
                    .map(|hover| hover.clamp(px0, px0 + pw));
                let main_from = match split {
                    Some(x) => x.max(px0 + LEFT_FADE),
                    None => px0 + LEFT_FADE,
                };
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: rect(bounds, main_from, py, px0 + pw + 1.0 - main_from, ph),
                    }),
                    |window| {
                        let alpha = split.map_or(fill_alpha, |x| {
                            if x < px0 + LEFT_FADE {
                                fill_alpha
                            } else {
                                fill_alpha * (1.0 - 0.6 * state.scrub)
                            }
                        });
                        window.paint_path(fill_path, gradient(alpha));
                    },
                );
                // The left edge fades the fill out in four steps, each a small
                // polygon of the points inside it.
                for step in 1..=4 {
                    let x0 = px0 + LEFT_FADE * f64::from(step - 1) / 4.0;
                    let x1 = px0 + LEFT_FADE * f64::from(step) / 4.0;
                    if let Some(band) = area_band(bounds, &state.points, x0, x1, py + ph) {
                        window.paint_path(band, gradient(fill_alpha * f64::from(step) / 4.0));
                    }
                }
            }
            if let Some(line) = path(bounds, &state.points, &state.controls, false, py + ph) {
                let blend = (state.reveal * 3.0).min(1.0)
                    * if state.line_presence > 0.001 {
                        state.line_mode / state.line_presence
                    } else {
                        1.0
                    };
                let color = mix(ink.label, palette.glow, blend);
                let breath = if state.reveal < 1.0 {
                    let breath = state.breath;
                    breath + (1.0 - breath) * state.reveal
                } else {
                    1.0
                };
                let alpha = state.line_presence * breath * state.dim;
                if let Some(hover) = state.hover_x.filter(|_| state.scrub > 0.0) {
                    let split = hover.clamp(px0, px0 + pw);
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: rect(bounds, px0 - 1.0, py, split - px0 + 1.0, ph),
                        }),
                        |window| {
                            window.paint_path(line.clone(), rgba(color, alpha));
                        },
                    );
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: rect(bounds, split, py, px0 + pw + 1.0 - split, ph),
                        }),
                        |window| {
                            window.paint_path(line, rgba(color, alpha * (1.0 - 0.6 * state.scrub)));
                        },
                    );
                } else {
                    let first = state.points.first().map_or(px0, |p| p.0);
                    let last = state.points.last().map_or(px0 + pw, |p| p.0);
                    let width = (last - first).max(1.0);
                    let start = ((px0 - first) / width).clamp(0.0, 1.0) as f32;
                    let end = ((px0 + LEFT_FADE - first) / width).clamp(start as f64, 1.0) as f32;
                    window.paint_path(
                        line,
                        linear_gradient(
                            90.0,
                            linear_color_stop(rgba(color, 0.0), start),
                            linear_color_stop(rgba(color, alpha), end),
                        ),
                    );
                }
            }
        }
        if let Some(close_y) = state.close_y
            && state.line_presence < 0.99
        {
            let alpha = if state.reveal <= 0.4 {
                0.0
            } else {
                ((state.reveal - 0.4) / 0.4).clamp(0.0, 1.0)
            };
            dashed(
                window,
                bounds,
                px0,
                close_y,
                pw,
                4.0,
                4.0,
                rgba(
                    mix(palette.down, palette.up, state.bull),
                    alpha * (1.0 - state.line_presence) * 0.4 * state.dim,
                ),
                true,
                px0,
            );
            if state.line_presence > 0.01 && !state.off_plot {
                dashed(
                    window,
                    bounds,
                    px0,
                    close_y.clamp(py, py + ph),
                    pw,
                    4.0,
                    4.0,
                    rgba(
                        palette.glow,
                        alpha * state.line_presence * 0.4 * (1.0 - 0.2 * state.scrub) * state.dim,
                    ),
                    true,
                    px0,
                );
            }
        }
        for candle in &state.candles {
            let color =
                mix(mix(palette.down, palette.up, candle.bull), palette.glow, state.line_presence);
            let alpha = candle.alpha * fade_at(candle.x, px0) * state.dim;
            if alpha <= 0.0 {
                continue;
            }
            let top = candle.open.min(candle.close);
            let bottom = candle.open.max(candle.close);
            if top - candle.high > 0.5 {
                square(
                    window,
                    bounds,
                    candle.x - candle.wick / 2.0,
                    candle.high,
                    candle.wick,
                    top - candle.high,
                    rgba(color, alpha),
                    candle.wick / 2.0,
                );
            }
            if candle.low - bottom > 0.5 {
                square(
                    window,
                    bounds,
                    candle.x - candle.wick / 2.0,
                    bottom,
                    candle.wick,
                    candle.low - bottom,
                    rgba(color, alpha),
                    candle.wick / 2.0,
                );
            }
            square(
                window,
                bounds,
                candle.x - candle.width / 2.0,
                top,
                candle.width,
                (bottom - top).max(1.0),
                rgba(color, alpha),
                if candle.width > 6.0 { 1.5 } else { 0.0 },
            );
            if candle.live {
                let shadow_bounds = rect(
                    bounds,
                    candle.x - candle.width / 2.0,
                    top,
                    candle.width,
                    (bottom - top).max(1.0),
                );
                let shadow =
                    BoxShadow::new(px(0.0), px(0.0), rgba(color, alpha * 0.2)).blur_radius(px(8.0));
                window.paint_drop_shadows(shadow_bounds, px(1.5).into(), &[shadow]);
                let glow = state.live_glow * alpha;
                square(
                    window,
                    bounds,
                    candle.x - candle.width / 2.0,
                    top,
                    candle.width,
                    (bottom - top).max(1.0),
                    rgba(color, glow),
                    1.5,
                );
            }
        }
        if let Some((x, y)) = state.tip
            && state.line_presence > 0.5
        {
            let alpha = state.dot_alpha * state.dim;
            if let Some((radius, ring_alpha)) = state.ring {
                let mut p = PathBuilder::stroke(px(1.5));
                p.move_to(at(bounds, (x + radius, y)));
                p.arc_to(
                    point(px(radius as f32), px(radius as f32)),
                    px(0.0),
                    false,
                    true,
                    at(bounds, (x - radius, y)),
                );
                p.arc_to(
                    point(px(radius as f32), px(radius as f32)),
                    px(0.0),
                    false,
                    true,
                    at(bounds, (x + radius, y)),
                );
                if let Ok(p) = p.build() {
                    window.paint_path(p, rgba(palette.glow, alpha * ring_alpha));
                }
            }
            let outer: Hsla = rgb(if dark { 0x282828 } else { 0xffffff }).into();
            let shadow = BoxShadow::new(px(0.0), px(1.0), rgba(gpui_kit::black(), 0.2 * alpha))
                .blur_radius(px((6.0 * (1.0 - state.scrub)) as f32));
            window.paint_drop_shadows(
                rect(bounds, x - 6.5, y - 6.5, 13.0, 13.0),
                px(6.5).into(),
                &[shadow],
            );
            square(window, bounds, x - 6.5, y - 6.5, 13.0, 13.0, rgba(outer, alpha * 0.95), 6.5);
            square(
                window,
                bounds,
                x - 3.5,
                y - 3.5,
                7.0,
                7.0,
                rgba(mix(palette.glow, outer, state.scrub * 0.7), alpha),
                3.5,
            );
        }
    });
    // The axis baseline fades out at the left edge like everything else.
    for step in 0..4 {
        let x = px0 + LEFT_FADE * f64::from(step) / 4.0;
        let w = LEFT_FADE / 4.0;
        window.paint_quad(fill(
            rect(bounds, x, py + ph, w, 1.0),
            rgba(ink.grid, fade_at(x + w / 2.0, px0)),
        ));
    }
    window.paint_quad(fill(rect(bounds, px0 + LEFT_FADE, py + ph, pw - LEFT_FADE, 1.0), ink.grid));
    // The time labels, merged in order with those of a picture the chart is
    // leaving. Where two would overlap, the more visible one is drawn, so a
    // handover swaps them rather than piling them up.
    painter.visible_times.clear();
    let mut current = state.times.iter().map(|tick| (*tick, state.times_span)).peekable();
    let mut ghosts = state.ghosts.iter().copied().peekable();
    let merged = std::iter::from_fn(|| match (current.peek(), ghosts.peek()) {
        (Some(a), Some(b)) if b.0.pos < a.0.pos => ghosts.next(),
        (Some(_), _) => current.next(),
        (None, _) => ghosts.next(),
    });
    for (tick, span) in merged {
        if tick.alpha <= 0.0 || tick.pos < px0 || tick.pos > px0 + pw {
            continue;
        }
        let shaped = painter.text.line(
            LABEL_TIME,
            tick.value,
            label_kind(span),
            tick.alpha,
            ink.time,
            11.0,
            window,
            || labels::axis_label(tick.value, span),
        );
        let Some(shaped) = shaped else { continue };
        let tw = f64::from(f32::from(shaped.width()));
        if let Some((previous, previous_width, previous_span)) = painter.visible_times.last_mut()
            && tick.pos - tw / 2.0 < previous.pos + *previous_width / 2.0 + 8.0
        {
            if tick.alpha > previous.alpha {
                *previous = tick;
                *previous_width = tw;
                *previous_span = span;
            }
            continue;
        }
        painter.visible_times.push((tick, tw, span));
    }
    for (tick, tw, span) in &painter.visible_times {
        window.paint_quad(fill(
            rect(bounds, tick.pos, py + ph, 1.0, 5.0),
            rgba(ink.grid, tick.alpha),
        ));
        if let Some(shaped) = painter.text.line(
            LABEL_TIME,
            tick.value,
            label_kind(*span),
            tick.alpha,
            ink.time,
            11.0,
            window,
            || labels::axis_label(tick.value, *span),
        ) {
            let _ = shaped.paint(
                at(bounds, (tick.pos - tw / 2.0, py + ph + 7.0)),
                px(14.0),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
    if state.line_mode > 0.5
        && let Some(value) = state.badge_value
        && state.tip.is_some()
    {
        let alpha = ((state.line_mode - 0.5) * 2.0).clamp(0.0, 1.0)
            * ((state.reveal - 0.25) / 0.25).clamp(0.0, 1.0)
            * state.dim;
        let color = mix(palette.down, palette.up, state.momentum);
        let template_width = painter
            .text
            .line(LABEL_TEMPLATE, value, decimals, 1.0, gpui_kit::white(), 11.0, window, || {
                format::price(value, decimals)
                    .chars()
                    .map(|c| if c.is_ascii_digit() { '8' } else { c })
                    .collect()
            })
            .map_or(0.0, |line| f64::from(f32::from(line.width())));
        if painter.badge_width == 0.0 {
            painter.badge_width = template_width;
        }
        painter.badge_width = math::lerp(painter.badge_width, template_width, 0.15, painter.dt);
        if (painter.badge_width - template_width).abs() < 0.3 || cx.reduce_motion() {
            painter.badge_width = template_width;
        } else {
            window.request_animation_frame();
        }
        let bx = px0 + pw - 2.0;
        let by = state.badge_y - 11.0;
        square(window, bounds, bx, by, painter.badge_width + 20.0, 22.0, rgba(color, alpha), 5.0);
        let mut tail = PathBuilder::fill();
        tail.move_to(at(bounds, (bx - 5.0, state.badge_y)));
        tail.line_to(at(bounds, (bx + 1.0, state.badge_y - 2.5)));
        tail.line_to(at(bounds, (bx + 1.0, state.badge_y + 2.5)));
        tail.close();
        if let Ok(tail) = tail.build() {
            window.paint_path(tail, rgba(color, alpha));
        }
        if let Some(shaped) = painter.text.line(
            LABEL_BADGE,
            value,
            decimals,
            alpha,
            gpui_kit::white(),
            11.0,
            window,
            || format::price(value, decimals),
        ) {
            let _ = shaped.paint(
                at(bounds, (bx + 10.0, by + 4.0)),
                px(16.0),
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_loading(
    state: &State,
    bounds: Bounds<Pixels>,
    palette: Palette,
    ink: Hsla,
    cache: &mut TextCache,
    window: &mut Window,
    cx: &mut GpuiApp,
) {
    let w = f64::from(f32::from(bounds.size.width));
    let h = f64::from(f32::from(bounds.size.height));
    let pw = (w - 78.0).max(1.0);
    let ph = (h - 44.0).max(1.0);
    let empty = 1.0 - state.loading;
    let text = cache
        .line(
            LABEL_EMPTY,
            0.0,
            crate::i18n::current() as usize,
            empty,
            rgba(ink, 0.35),
            12.0,
            window,
            || t!("chart.emptyData").to_string(),
        )
        .map(|line| (line.clone(), f64::from(f32::from(line.width()))));
    let tw = text.as_ref().map_or(0.0, |(_, width)| *width);
    if let Some(wave) = path(bounds, &state.wave_points, &state.wave_controls, false, 0.0) {
        let breath = state.breath;
        if state.loading > 0.0 {
            window.paint_path(
                wave.clone(),
                rgba(mix(ink, palette.glow, state.line_mode), breath * state.loading),
            );
        }
        if empty > 0.0 {
            let center = 14.0 + pw / 2.0;
            let left = ((center - tw / 2.0 - 20.0 - 14.0) / pw).clamp(0.0, 1.0) as f32;
            let right = ((center + tw / 2.0 + 20.0 - 14.0) / pw).clamp(0.0, 1.0) as f32;
            let fade = (30.0 / pw) as f32;
            let color = rgba(ink, breath * empty);
            window.paint_path(
                wave.clone(),
                linear_gradient(
                    90.0,
                    linear_color_stop(color, (left - fade).max(0.0)),
                    linear_color_stop(rgba(color, 0.0), left),
                ),
            );
            window.paint_path(
                wave,
                linear_gradient(
                    90.0,
                    linear_color_stop(rgba(color, 0.0), right),
                    linear_color_stop(color, (right + fade).min(1.0)),
                ),
            );
        }
    }
    if empty > 0.01
        && let Some((text, tw)) = text
    {
        let _ = text.paint(
            at(bounds, (14.0 + (pw - tw) / 2.0, 16.0 + ph / 2.0 - 6.0)),
            px(15.0),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}
