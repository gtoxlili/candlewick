//! Price chart canvas.

mod math;
mod state;

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use gpui_kit::{
    AnyElement, App as GpuiApp, Bounds, BoxShadow, ContentMask, Context, Element, ElementId,
    Entity, GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement, LayoutId,
    ParentElement, PathBuilder, Pixels, Render, Rgba, ShapedLine, Styled, TextAlign, TextRun,
    Window, canvas, div, fill, font, linear_color_stop, linear_gradient, point, px, quad, rgb,
    size,
};

use super::{labels, model_state::ChartModel};
use crate::{
    format,
    ui::{prefs::Mode, theme::Palette},
};
use state::{Input, State};

pub struct PriceCanvas {
    pub model: Entity<ChartModel>,
}
impl Render for PriceCanvas {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let palette = Palette::current(model.app(), cx);
        let dark = gpui_kit::component::Theme::global(cx).is_dark();
        div().id("PriceCanvas").size_full().child(PriceElement {
            model: self.model.clone(),
            palette,
            dark,
        })
    }
}

struct PriceElement {
    model: Entity<ChartModel>,
    palette: Palette,
    dark: bool,
}
impl IntoElement for PriceElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
struct Painter {
    chart: State,
    last: Option<Instant>,
    labels: HashMap<(u8, u64, usize, u8), ShapedLine>,
    dark: bool,
    visible_times: Vec<(state::Tick, f64)>,
    badge_width: f64,
    dt: f64,
    day_anchor: f64,
    axis_key: Option<(u64, usize)>,
    axis_width: f64,
}
impl Painter {
    fn new(dark: bool) -> Self {
        Self {
            chart: State::default(),
            last: None,
            labels: HashMap::new(),
            dark,
            visible_times: Vec::new(),
            badge_width: 0.0,
            dt: 16.67,
            day_anchor: local_day_anchor(),
            axis_key: None,
            axis_width: 64.0,
        }
    }
}
impl Element for PriceElement {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        Some("price-canvas-painter".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) -> (LayoutId, Self::RequestLayoutState) {
        window.with_element_state(id.unwrap(), |state: Option<Rc<RefCell<Painter>>>, window| {
            let state = state.unwrap_or_else(|| Rc::new(RefCell::new(Painter::new(self.dark))));
            let painter = state.clone();
            let model = self.model.clone();
            let palette = self.palette;
            let dark = self.dark;
            let mut element = canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    paint_canvas(
                        &mut painter.borrow_mut(),
                        &model,
                        palette,
                        dark,
                        bounds,
                        window,
                        cx,
                    );
                },
            )
            .size_full()
            .into_any_element();
            ((element.request_layout(window, cx), element), state)
        })
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        element.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut GpuiApp,
    ) {
        element.paint(window, cx);
    }
}

fn paint_canvas(
    painter: &mut Painter,
    model: &Entity<ChartModel>,
    palette: Palette,
    dark: bool,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut GpuiApp,
) {
    if painter.labels.len() > 4096 {
        painter.labels.clear();
    }
    if painter.dark != dark {
        painter.labels.clear();
        painter.dark = dark;
    }
    let clock = cx.background_executor().now();
    let dt = painter
        .last
        .map_or(16.67, |last| clock.saturating_duration_since(last).as_secs_f64() * 1000.0)
        .min(50.0);
    painter.last = Some(clock);
    painter.dt = dt;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
    let width = f64::from(f32::from(bounds.size.width));
    let height = f64::from(f32::from(bounds.size.height));
    let mouse = window.mouse_position();
    let mx = f64::from(f32::from(mouse.x - bounds.origin.x));
    let my = f64::from(f32::from(mouse.y - bounds.origin.y));
    let mut decimals = 2;
    let mut span = 3600.0;
    let reduce_motion = cx.reduce_motion();
    model.update(cx, |model, _| {
        let line_mode = model.mode() == Mode::Line;
        let tick_decimals = model.instrument().and_then(|i| i.decimals);
        let dragging = model.analysis().dragging();
        let frame = model.frame.clone();
        let (market, viewport, _) = model.parts_mut();
        viewport.set_now(now);
        let glide = viewport.glide_tick();
        span = viewport.view.span;
        let Some(market) = market else {
            painter.chart.advance(
                dt,
                Input {
                    candles: &[],
                    live: None,
                    line: &[],
                    price: None,
                    candle_width: 60.0,
                    now,
                    day_anchor: painter.day_anchor,
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
            frame.set(painter.chart.frame);
            return;
        };
        let data = market.chart_data();
        let price = data.price.or(data.live.map(|c| c.close)).unwrap_or(1.0);
        decimals = format::decimals(price, tick_decimals);
        let key = (price.to_bits(), decimals);
        if painter.axis_key != Some(key) {
            painter.axis_width = (16.0
                + 6.8
                    * format::price(if price == 0.0 { 1.0 } else { price }, decimals).len() as f64)
                .max(64.0);
            painter.axis_key = Some(key);
        }
        let axis_width = painter.axis_width;
        let hover_x =
            (my >= 0.0 && my <= height && !dragging && !viewport.dragging()).then_some(mx);
        let input = Input {
            candles: data.candles,
            live: data.live,
            line: data.line,
            price: data.price,
            candle_width: f64::from(data.interval),
            now,
            day_anchor: painter.day_anchor,
            width,
            height,
            axis_width,
            loading: !data.loaded && data.candles.is_empty() && data.live.is_none(),
            line_mode,
            hover_x,
            reduce_motion,
        };
        painter.chart.advance(dt, input, &mut viewport.view);
        painter.chart.moving |= glide;
        frame.set(painter.chart.frame);
    });
    if painter.chart.moving {
        window.request_animation_frame();
    }
    draw(painter, palette, dark, bounds, decimals, span, window, cx);
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
fn at(bounds: Bounds<Pixels>, p: (f64, f64)) -> gpui_kit::Point<Pixels> {
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
) {
    let mut p = 0.0;
    while p < width {
        window.paint_quad(fill(rect(bounds, x + p, y, (width - p).min(dash), 1.0), color));
        p += dash + gap;
    }
}
#[allow(clippy::too_many_arguments)]
fn label<'a>(
    cache: &'a mut HashMap<(u8, u64, usize, u8), ShapedLine>,
    kind: u8,
    value: f64,
    extra: usize,
    alpha: f64,
    text: impl FnOnce() -> String,
    color: Hsla,
    window: &mut Window,
) -> &'a ShapedLine {
    let level = (alpha.clamp(0.0, 1.0) * 100.0).round() as u8;
    let key = (kind, value.to_bits(), extra, level);
    cache.entry(key).or_insert_with(|| {
        let text = text();
        let run = TextRun {
            len: text.len(),
            font: font(if kind == 2 { ".SystemUIFont" } else { "SF Mono" }),
            color: rgba(color, f64::from(level) / 100.0),
            ..Default::default()
        };
        window.text_system().shape_line(
            text.into(),
            px(if kind == 2 { 12.0 } else { 11.0 }),
            &[run],
            None,
        )
    })
}
fn path(
    bounds: Bounds<Pixels>,
    points: &[(f64, f64)],
    controls: &[[(f64, f64); 3]],
    fill_path: bool,
    bottom: f64,
) -> Option<gpui_kit::Path<Pixels>> {
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

#[allow(clippy::too_many_arguments)]
fn draw(
    painter: &mut Painter,
    palette: Palette,
    dark: bool,
    bounds: Bounds<Pixels>,
    decimals: usize,
    span: f64,
    window: &mut Window,
    cx: &mut GpuiApp,
) {
    let state = &painter.chart;
    let ink: Hsla = rgb(if dark { 0xffffff } else { 0x000000 }).into();
    let grid_color = rgba(ink, 0.06);
    let label_color = rgba(ink, if dark { 0.4 } else { 0.35 });
    let time_color = rgba(ink, if dark { 0.35 } else { 0.3 });
    let Some(frame) = state.frame else {
        draw_loading(state, bounds, palette, label_color, &mut painter.labels, window, cx);
        return;
    };
    let px0 = frame.plot_x;
    let py = frame.plot_y;
    let pw = frame.plot_w;
    let ph = frame.plot_h;
    for tick in &state.grid {
        if tick.alpha <= 0.0 || tick.pos < py - 10.0 || tick.pos > py + ph + 10.0 {
            continue;
        }
        dashed(window, bounds, px0, tick.pos, pw, 1.0, 3.0, rgba(grid_color, tick.alpha));
        let shaped = label(
            &mut painter.labels,
            0,
            tick.value,
            decimals,
            tick.alpha,
            || format::price(tick.value, decimals),
            label_color,
            window,
        );
        let _ = shaped.paint(
            at(bounds, (px0 + pw + 8.0, tick.pos - 7.0)),
            px(14.0),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
    let clip = ContentMask { bounds: rect(bounds, px0 - 1.0, py, pw + 2.0, ph) };
    window.with_content_mask(Some(clip), |window| {
        if state.line_presence > 0.001 && state.points.len() > 1 {
            if state.line_mode > 0.01
                && let Some(path) = path(bounds, &state.points, &state.controls, true, py + ph)
            {
                let fill_alpha = state.line_presence
                    * state.line_mode
                    * state.reveal
                    * if dark { 0.12 } else { 0.08 };
                let gradient = |alpha| {
                    linear_gradient(
                        180.0,
                        linear_color_stop(rgba(palette.glow, alpha), 0.0),
                        linear_color_stop(rgba(palette.glow, 0.0), 1.0),
                    )
                };
                if let Some(hover) = state.hover_x.filter(|_| state.scrub > 0.0) {
                    let split = hover.clamp(px0, px0 + pw);
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: rect(bounds, px0 - 1.0, py, split - px0 + 1.0, ph),
                        }),
                        |window| {
                            window.paint_path(path.clone(), gradient(fill_alpha));
                        },
                    );
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: rect(bounds, split, py, px0 + pw + 1.0 - split, ph),
                        }),
                        |window| {
                            window
                                .paint_path(path, gradient(fill_alpha * (1.0 - 0.6 * state.scrub)));
                        },
                    );
                } else {
                    window.paint_path(path, gradient(fill_alpha));
                }
            }
            if let Some(path) = path(bounds, &state.points, &state.controls, false, py + ph) {
                let blend = (state.reveal * 3.0).min(1.0)
                    * if state.line_presence > 0.001 {
                        state.line_mode / state.line_presence
                    } else {
                        1.0
                    };
                let color = mix(label_color, palette.glow, blend);
                let breath = if state.reveal < 1.0 {
                    let breath = state.breath;
                    breath + (1.0 - breath) * state.reveal
                } else {
                    1.0
                };
                let alpha = state.line_presence * breath;
                if let Some(hover) = state.hover_x.filter(|_| state.scrub > 0.0) {
                    let split = hover.clamp(px0, px0 + pw);
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: rect(bounds, px0 - 1.0, py, split - px0 + 1.0, ph),
                        }),
                        |window| {
                            window.paint_path(path.clone(), rgba(color, alpha));
                        },
                    );
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: rect(bounds, split, py, px0 + pw + 1.0 - split, ph),
                        }),
                        |window| {
                            window.paint_path(path, rgba(color, alpha * (1.0 - 0.6 * state.scrub)));
                        },
                    );
                } else {
                    let first = state.points.first().map_or(px0, |p| p.0);
                    let last = state.points.last().map_or(px0 + pw, |p| p.0);
                    let width = (last - first).max(1.0);
                    let start = ((px0 - first) / width).clamp(0.0, 1.0) as f32;
                    let end = ((px0 + 40.0 - first) / width).clamp(start as f64, 1.0) as f32;
                    window.paint_path(
                        path,
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
                    alpha * (1.0 - state.line_presence) * 0.4,
                ),
            );
            if state.line_presence > 0.01 {
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
                        alpha * state.line_presence * 0.4 * (1.0 - 0.2 * state.scrub),
                    ),
                );
            }
        }
        for candle in &state.candles {
            let color =
                mix(mix(palette.down, palette.up, candle.bull), palette.glow, state.line_presence);
            let alpha = candle.alpha * ((candle.x - px0) / 40.0).clamp(0.0, 1.0);
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
            let alpha = state.dot_alpha;
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
    window.paint_quad(fill(rect(bounds, px0, py + ph, pw, 1.0), grid_color));
    painter.visible_times.clear();
    for tick in &state.times {
        if tick.alpha <= 0.0 || tick.pos < px0 || tick.pos > px0 + pw {
            continue;
        }
        let shaped = label(
            &mut painter.labels,
            1,
            tick.value,
            if span <= 300.0 {
                0
            } else if span <= 172800.0 {
                1
            } else {
                2
            },
            tick.alpha,
            || labels::axis_label(tick.value, span),
            time_color,
            window,
        );
        let tw = f64::from(f32::from(shaped.width()));
        if let Some((previous, previous_width)) = painter.visible_times.last_mut()
            && tick.pos - tw / 2.0 < previous.pos + *previous_width / 2.0 + 8.0
        {
            if tick.alpha > previous.alpha {
                *previous = *tick;
                *previous_width = tw;
            }
            continue;
        }
        painter.visible_times.push((*tick, tw));
    }
    for (tick, tw) in &painter.visible_times {
        let shaped = label(
            &mut painter.labels,
            1,
            tick.value,
            if span <= 300.0 {
                0
            } else if span <= 172800.0 {
                1
            } else {
                2
            },
            tick.alpha,
            || labels::axis_label(tick.value, span),
            time_color,
            window,
        );
        window.paint_quad(fill(
            rect(bounds, tick.pos, py + ph, 1.0, 5.0),
            rgba(grid_color, tick.alpha),
        ));
        let _ = shaped.paint(
            at(bounds, (tick.pos - tw / 2.0, py + ph + 7.0)),
            px(14.0),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
    if state.line_mode > 0.5
        && let Some(value) = state.badge_value
        && state.tip.is_some()
    {
        let alpha = ((state.line_mode - 0.5) * 2.0).clamp(0.0, 1.0)
            * ((state.reveal - 0.25) / 0.25).clamp(0.0, 1.0);
        let color = mix(palette.down, palette.up, state.momentum);
        let template_width = {
            let template = label(
                &mut painter.labels,
                4,
                value,
                decimals,
                1.0,
                || {
                    format::price(value, decimals)
                        .chars()
                        .map(|c| if c.is_ascii_digit() { '8' } else { c })
                        .collect()
                },
                gpui_kit::white(),
                window,
            );
            f64::from(f32::from(template.width()))
        };
        if painter.badge_width == 0.0 {
            painter.badge_width = template_width;
        }
        painter.badge_width = math::lerp(painter.badge_width, template_width, 0.15, painter.dt);
        if (painter.badge_width - template_width).abs() < 0.3 || cx.reduce_motion() {
            painter.badge_width = template_width;
        } else {
            window.request_animation_frame();
        }
        let shaped = label(
            &mut painter.labels,
            3,
            value,
            decimals,
            alpha,
            || format::price(value, decimals),
            gpui_kit::white(),
            window,
        );
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

#[allow(clippy::too_many_arguments)]
fn draw_loading(
    state: &State,
    bounds: Bounds<Pixels>,
    palette: Palette,
    ink: Hsla,
    cache: &mut HashMap<(u8, u64, usize, u8), ShapedLine>,
    window: &mut Window,
    cx: &mut GpuiApp,
) {
    let w = f64::from(f32::from(bounds.size.width));
    let h = f64::from(f32::from(bounds.size.height));
    let pw = (w - 78.0).max(1.0);
    let ph = (h - 44.0).max(1.0);
    let empty = 1.0 - state.loading;
    let text = label(
        cache,
        2,
        0.0,
        crate::i18n::current() as usize,
        empty,
        || t!("chart.emptyData").to_string(),
        rgba(ink, 0.35),
        window,
    )
    .clone();
    let tw = f64::from(f32::from(text.width()));
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
    if empty > 0.01 {
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
