//! Animation and geometry for a price chart, without window-system types.

use super::super::{geometry::Frame, market::LinePoint, viewport::View};
use super::math::{
    candle_range, cosine, ease_range, grid_interval, lerp, ramp, smoothstep, spline, time_interval,
};
use super::reform::{self, Picture, Scene};
use crate::market::Candle;

const LEFT: f64 = 14.0;
const TOP: f64 = 16.0;
const BOTTOM: f64 = 28.0;
const BUFFER: f64 = 0.015;

#[derive(Clone, Copy)]
pub(super) struct Input<'a> {
    pub candles: &'a [Candle],
    pub live: Option<Candle>,
    pub line: &'a [LinePoint],
    pub price: Option<f64>,
    /// The feed the data comes from. Another feed is another chart, which the
    /// picture on screen does not re-form into.
    pub series: u64,
    pub candle_width: f64,
    pub now: f64,
    pub day_anchor: f64,
    pub width: f64,
    pub height: f64,
    pub axis_width: f64,
    pub loading: bool,
    pub line_mode: bool,
    pub hover_x: Option<f64>,
    pub reduce_motion: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CandleShape {
    pub x: f64,
    pub width: f64,
    pub wick: f64,
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    pub bull: f64,
    pub alpha: f64,
    pub live: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Tick {
    pub value: f64,
    pub pos: f64,
    pub alpha: f64,
}

#[derive(Clone, Copy)]
struct Fade {
    key: f64,
    alpha: f64,
    target: f64,
    fresh: bool,
}

#[derive(Clone, Copy)]
struct Transition {
    from: f64,
    target: f64,
    elapsed: f64,
    duration: f64,
}
impl Transition {
    fn new(value: f64, duration: f64) -> Self {
        Self { from: value, target: value, elapsed: duration, duration }
    }
    fn value(self) -> f64 {
        self.from + (self.target - self.from) * cosine(self.elapsed / self.duration)
    }
    fn set(&mut self, target: f64) {
        if self.target != target {
            self.from = self.value();
            self.target = target;
            self.elapsed = 0.0;
        }
    }
    fn advance(&mut self, dt: f64, snap: bool) -> f64 {
        self.elapsed = if snap { self.duration } else { (self.elapsed + dt).min(self.duration) };
        self.value()
    }
    fn moving(self) -> bool {
        self.elapsed < self.duration
    }
}

#[derive(Clone, Copy, Default)]
struct SmoothValue(Option<f64>);
impl SmoothValue {
    fn step(
        &mut self,
        value: Option<f64>,
        range: f64,
        dt: f64,
        fixed: bool,
        snap: bool,
    ) -> Option<f64> {
        let Some(target) = value else {
            self.0 = None;
            return None;
        };
        let current = self.0.unwrap_or(target);
        let gap = (current - target).abs();
        let speed = if fixed { 0.25 } else { 0.08 + (1.0 - (gap / range).min(1.0)) * 0.2 };
        let threshold = range * if fixed { 0.0005 } else { 0.001 };
        let next = if snap || gap < threshold { target } else { lerp(current, target, speed, dt) };
        self.0 = Some(next);
        Some(next)
    }
}

pub(super) struct State {
    pub frame: Option<Frame>,
    pub candles: Vec<CandleShape>,
    pub grid: Vec<Tick>,
    pub times: Vec<Tick>,
    /// The span `times` read for.
    pub times_span: f64,
    /// The time labels of a picture the chart is leaving, each with the span
    /// it reads for.
    pub ghosts: Vec<(Tick, f64)>,
    pub points: Vec<(f64, f64)>,
    pub controls: Vec<[(f64, f64); 3]>,
    pub wave_points: Vec<(f64, f64)>,
    pub wave_controls: Vec<[(f64, f64); 3]>,
    pub line_mode: f64,
    pub line_presence: f64,
    pub reveal: f64,
    pub loading: f64,
    pub scrub: f64,
    /// The share of their ink the marks show: below 1 while a held picture
    /// waits for the interval it is leaving for.
    pub dim: f64,
    pub hover_x: Option<f64>,
    pub close_y: Option<f64>,
    pub tip: Option<(f64, f64)>,
    /// Whether the live value sits off the plot while the view is pinned in
    /// history: the dashed value line is not drawn then.
    pub off_plot: bool,
    /// Candles whose geometry this frame built, for tests.
    pub candles_visited: usize,
    pub moving: bool,
    pub bull: f64,
    pub badge_value: Option<f64>,
    pub badge_y: f64,
    pub momentum: f64,
    pub breath: f64,
    pub live_glow: f64,
    pub dot_alpha: f64,
    pub ring: Option<(f64, f64)>,
    span: Option<f64>,
    end: f64,
    lag: f64,
    panned: bool,
    range: Option<(f64, f64)>,
    live: Option<Candle>,
    birth: f64,
    close_smooth: SmoothValue,
    line_smooth: SmoothValue,
    tick_smooth: SmoothValue,
    mode: Transition,
    density: f64,
    density_from: f64,
    density_target: f64,
    density_elapsed: f64,
    /// Seconds per candle; 0 before the first frame.
    width: f64,
    series: Option<u64>,
    scene: Scene,
    /// How much of the layers drawn in the frame's coordinates to show.
    marks: f64,
    coarse: Option<f64>,
    grid_fades: Vec<Fade>,
    time_fades: Vec<Fade>,
    slopes: Vec<f64>,
    close_points: Vec<(f64, f64)>,
    last_hover: Option<Candle>,
    /// Where a re-form writes, kept between frames.
    reformed_candles: Vec<CandleShape>,
    reformed_points: Vec<(f64, f64)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            frame: None,
            candles: Vec::new(),
            grid: Vec::new(),
            times: Vec::new(),
            times_span: 0.0,
            ghosts: Vec::new(),
            points: Vec::new(),
            controls: Vec::new(),
            wave_points: Vec::new(),
            wave_controls: Vec::new(),
            line_mode: 0.0,
            line_presence: 0.0,
            reveal: 0.0,
            loading: 0.0,
            scrub: 0.0,
            dim: 1.0,
            hover_x: None,
            close_y: None,
            tip: None,
            off_plot: false,
            candles_visited: 0,
            moving: false,
            bull: 0.5,
            badge_value: None,
            badge_y: 0.0,
            momentum: 1.0,
            breath: 0.22,
            live_glow: 0.12,
            dot_alpha: 0.0,
            ring: None,
            span: None,
            end: 0.0,
            lag: 0.0,
            panned: false,
            range: None,
            live: None,
            birth: 0.0,
            close_smooth: SmoothValue::default(),
            line_smooth: SmoothValue::default(),
            tick_smooth: SmoothValue::default(),
            mode: Transition::new(0.0, 500.0),
            density: 0.0,
            density_from: 0.0,
            density_target: 0.0,
            density_elapsed: 350.0,
            width: 0.0,
            series: None,
            scene: Scene::Live,
            marks: 0.0,
            coarse: None,
            grid_fades: Vec::new(),
            time_fades: Vec::new(),
            slopes: Vec::new(),
            close_points: Vec::new(),
            last_hover: None,
            reformed_candles: Vec::new(),
            reformed_points: Vec::new(),
        }
    }
}

fn approaching(
    value: &mut f64,
    target: f64,
    speed: f64,
    dt: f64,
    threshold: f64,
    snap: bool,
) -> bool {
    *value = if snap { target } else { lerp(*value, target, speed, dt) };
    if (*value - target).abs() < threshold {
        *value = target;
    }
    *value != target
}
fn flag(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}
fn pulse(now_ms: f64, dim: f64) -> Option<(f64, f64)> {
    let t = (now_ms % 1500.0) / 900.0;
    (t < 1.0).then_some((9.0 + 12.0 * t, 0.35 * (1.0 - t) * (1.0 - dim * 3.0)))
}
fn wave(t: f64, now_ms: f64, height: f64) -> f64 {
    let s = now_ms * 0.001;
    TOP + height
        * (0.5
            + 0.07
                * (0.55 * (9.4 * t + s).sin()
                    + 0.3 * (15.7 * t + 1.3 * s).sin()
                    + 0.15 * (4.2 * t + 0.7 * s).sin()))
}
fn select(candles: &[Candle], left: f64, right: f64, width: f64) -> &[Candle] {
    let first = candles.partition_point(|c| c.time + width < left);
    let last = candles.partition_point(|c| c.time <= right);
    &candles[first.min(last)..last]
}
fn key9(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    let scale = 10f64.powf(8.0 - value.abs().log10().floor());
    (value * scale).round() / scale
}
fn set_fade(fades: &mut Vec<Fade>, key: f64, target: f64, initial: f64) {
    if let Some(item) = fades.iter_mut().find(|item| item.key == key) {
        item.target = target;
    } else {
        fades.push(Fade { key, alpha: target * initial, target, fresh: initial > 0.0 });
    }
}

impl State {
    /// Advance the chart by a bounded frame duration and write the geometry actually drawn.
    pub fn advance(&mut self, dt_ms: f64, input: Input<'_>, view: &mut View) {
        let dt = dt_ms.clamp(0.0, 50.0);
        self.candles_visited = 0;
        let snap = input.reduce_motion;
        let now_ms = input.now * 1000.0;
        self.breath = 0.22 + 0.08 * (now_ms / 1200.0 * std::f64::consts::PI).sin();
        self.live_glow = 0.12 + 0.08 * (now_ms * 0.004).sin();
        self.dot_alpha = 0.0;
        self.ring = None;
        let plot_w = (input.width - LEFT - input.axis_width).max(1.0);
        let plot_h = (input.height - TOP - BOTTOM).max(1.0);
        self.wave_points.clear();
        for i in 0..=32 {
            let t = f64::from(i) / 32.0;
            self.wave_points.push((LEFT + t * plot_w, wave(t, input.now * 1000.0, plot_h)));
        }
        spline(&self.wave_points, &mut self.slopes, &mut self.wave_controls);
        let has_data = input.candles.len() >= 2;
        let retimed = self.follow(&input, has_data, dt, snap);
        let holding = matches!(self.scene, Scene::Hold { .. });
        // A held picture stands in for the data: no loading wave, no reveal.
        let loading = input.loading && !holding;
        if self.span.is_none() && loading {
            self.loading = 1.0;
        }
        self.moving = approaching(&mut self.loading, flag(loading), 0.14, dt, 0.01, snap);
        let reveal_target = flag(holding || (!loading && has_data));
        self.moving |= approaching(
            &mut self.reveal,
            reveal_target,
            if reveal_target > 0.0 { 0.09 } else { 0.14 },
            dt,
            0.005,
            snap,
        );
        if self.reveal < 0.01 {
            self.range = None;
        }
        // A held picture keeps its mode; the new interval's starts with it.
        if !holding {
            self.mode.set(flag(input.line_mode));
        }
        self.line_mode = self.mode.advance(dt, snap);
        self.moving |= self.mode.moving();
        let density_target =
            flag(input.line_mode && self.line_mode >= 0.3 && !input.line.is_empty());
        if density_target != self.density_target {
            self.density_from = self.density;
            self.density_target = density_target;
            self.density_elapsed = 0.0;
        }
        self.density_elapsed = if snap { 350.0 } else { (self.density_elapsed + dt).min(350.0) };
        let t = self.density_elapsed / 350.0;
        self.density = self.density_from
            + (self.density_target - self.density_from) * (1.0 - (1.0 - t).powi(2));
        self.moving |= self.density_elapsed < 350.0;

        let target_span = view.span.max(1e-6);
        let mut span = self.span.unwrap_or(target_span);
        if self.span.is_none() {
            self.end = view.end.unwrap_or(input.now + target_span * BUFFER);
        }
        // Another interval re-forms in place rather than zooming.
        let jump = std::mem::take(&mut view.snap) || snap || retimed;
        span = if jump { target_span } else { lerp(span.ln(), target_span.ln(), 0.2, dt).exp() };
        if (span.ln() - target_span.ln()).abs() < 1e-4 {
            span = target_span;
        }
        self.moving |= span != target_span;
        self.span = Some(span);
        let settle = 0.5 * span / plot_w;
        let live_end = input.now + span * BUFFER;
        if let Some(end) = view.end {
            self.panned = true;
            self.end = if jump { end } else { lerp(self.end, end, 0.2, dt) };
            if (self.end - end).abs() < settle {
                self.end = end;
            }
            self.moving |= self.end != end;
        } else {
            if self.panned {
                self.lag = (live_end - self.end).max(0.0);
                self.panned = false;
            }
            self.lag = if jump { 0.0 } else { lerp(self.lag, 0.0, 0.2, dt) };
            if self.lag < settle {
                self.lag = 0.0;
            }
            self.moving |= self.lag != 0.0;
            self.end = live_end - self.lag;
        }
        let left = self.end - span;
        view.left = Some(left);
        view.right = Some(self.end);
        view.plot_x = Some(LEFT);
        view.plot_w = Some(plot_w);
        view.now = input.now;
        view.buffer = BUFFER;

        if let Some(raw) = input.live {
            match &mut self.live {
                Some(live) if live.time == raw.time => {
                    for (v, target) in [
                        (&mut live.open, raw.open),
                        (&mut live.high, raw.high),
                        (&mut live.low, raw.low),
                        (&mut live.close, raw.close),
                    ] {
                        self.moving |=
                            approaching(v, target, 0.25, dt, 1e-9 * target.abs().max(1.0), snap);
                    }
                }
                _ => {
                    // A picture re-forming carries the forming candle in from
                    // the one it showed; it is not born again.
                    let carried = !matches!(self.scene, Scene::Live);
                    self.live = Some(if carried {
                        raw
                    } else {
                        Candle {
                            open: raw.open,
                            high: raw.open,
                            low: raw.open,
                            close: raw.open,
                            time: raw.time,
                        }
                    });
                    self.birth = flag(carried);
                }
            }
            self.moving |= approaching(&mut self.birth, 1.0, 0.2, dt, 0.01, snap);
            let live = self.live.unwrap();
            self.moving |=
                approaching(&mut self.bull, flag(live.close >= live.open), 0.12, dt, 0.01, snap);
        } else if has_data {
            self.live = None;
        }
        if matches!(self.scene, Scene::Hold { .. } | Scene::Fade(_)) {
            self.show_picture(&input, left, target_span, plot_w, plot_h, dt, snap, now_ms);
            return;
        }
        if !has_data {
            self.frame = None;
            self.candles.clear();
            self.grid.clear();
            self.times.clear();
            self.ghosts.clear();
            self.tip = None;
            self.points.clear();
            self.controls.clear();
            // The wave behind the empty state is the loading animation; an
            // empty chart that has finished loading holds still.
            self.moving |= self.loading > 0.0;
            return;
        }
        let visible = select(input.candles, left, self.end, self.width);
        let live = self.live.filter(|c| c.time + self.width >= left && c.time <= self.end);
        let raw_range = candle_range(visible, live);
        let reform = match &self.scene {
            Scene::Reform { elapsed, range, dim, .. } => Some((*elapsed, *range, *dim)),
            _ => None,
        };
        // The live edge's clock: the axis and everything not tied to one mark.
        let settled = reform.map_or(1.0, |(elapsed, ..)| reform::progress(elapsed));
        let range = match reform {
            Some((_, Some(from), _)) => reform::range(from, raw_range, settled),
            Some((_, None, _)) => raw_range,
            None => self.range.map_or(raw_range, |r| {
                if snap { raw_range } else { ease_range(r, raw_range, plot_h, dt) }
            }),
        };
        self.range = Some(range);
        self.moving |= range != raw_range || reform.is_some();
        self.dim = reform.map_or(1.0, |(_, _, dim)| dim + (1.0 - dim) * settled);
        let val_range = (range.1 - range.0).max(1e-3);
        let x = |t: f64| LEFT + (t - left) / span * plot_w;
        let y = |v: f64| TOP + (1.0 - (v - range.0) / val_range) * plot_h;
        // Re-forming marks head for where the new range puts them; the axis
        // gets there on its own clock.
        let marks_range = if reform.is_some() { raw_range } else { range };
        let marks_val = (marks_range.1 - marks_range.0).max(1e-3);
        let ym = |v: f64| TOP + (1.0 - (v - marks_range.0) / marks_val) * plot_h;
        let raw_close = input.live.map(|c| c.close);
        let close = self.close_smooth.step(raw_close, val_range, dt, true, snap);
        let line_close = self.line_smooth.step(raw_close, val_range, dt, false, snap);
        let tick = self.tick_smooth.step(
            input.price.filter(|_| !input.line.is_empty()),
            val_range,
            dt,
            false,
            snap,
        );
        self.moving |= close.zip(raw_close).is_some_and(|(a, b)| a != b)
            || line_close.zip(raw_close).is_some_and(|(a, b)| a != b)
            || tick.zip(input.price).is_some_and(|(a, b)| a != b);
        self.close_y = close.map(ym);
        let hovered = input.hover_x.filter(|x| *x >= LEFT && *x <= LEFT + plot_w);
        self.moving |= approaching(&mut self.scrub, flag(hovered.is_some()), 0.12, dt, 0.01, snap);
        if let Some(pos) = hovered {
            self.hover_x = Some(pos);
            let time = left + (pos - LEFT) / plot_w * span;
            let i = visible.partition_point(|c| c.time <= time);
            self.last_hover =
                live.filter(|c| time >= c.time && time < c.time + self.width).or_else(|| {
                    i.checked_sub(1)
                        .and_then(|i| visible.get(i))
                        .copied()
                        .filter(|c| time < c.time + self.width)
                });
        } else if self.scrub == 0.0 {
            self.hover_x = None;
            self.last_hover = None;
        }
        // Layers in the frame's coordinates come back once the axis has landed.
        let marks = reform.map_or(1.0, |(elapsed, ..)| flag(elapsed >= reform::SETTLE));
        self.moving |= approaching(&mut self.marks, marks, 0.2, dt, 0.01, snap);
        self.frame = Some(Frame {
            left,
            right: self.end,
            min: range.0,
            max: range.1,
            plot_x: LEFT,
            plot_y: TOP,
            plot_w,
            plot_h,
            width: input.width,
            height: input.height,
            now: input.now,
            candle_width: self.width,
            hover_x: self.last_hover.and(self.hover_x),
            scrub: self.scrub,
            line_mode: self.line_mode,
            hover: self.last_hover,
            settled: self.marks * smoothstep(self.reveal),
        });
        self.line_presence = self.line_mode.max(if self.line_mode >= 0.99 {
            1.0 - self.reveal
        } else {
            (1.0 - self.reveal).powi(3)
        });
        self.candles.clear();
        let collapse = smoothstep(self.reveal) * (1.0 - self.line_mode);
        let candle_alpha = self.reveal * (1.0 - self.line_presence);
        self.candles_visited += add_candles(
            &mut self.candles,
            visible,
            live,
            self.width,
            left,
            self.end,
            plot_w,
            collapse,
            candle_alpha,
            self.birth,
            self.bull,
            self.scrub,
            self.hover_x,
            &x,
            &ym,
        );
        if let Scene::Reform { from, elapsed, .. } = &self.scene {
            let at = |px: f64| reform::progress(elapsed - reform::delay(px, LEFT, plot_w));
            reform::bars(&from.candles, &self.candles, &mut self.reformed_candles, at);
            std::mem::swap(&mut self.candles, &mut self.reformed_candles);
            // The price line stays on the forming bar it marks.
            if let Some(forming) = self.candles.iter().rev().find(|c| c.live) {
                self.close_y = Some(forming.close);
            }
        }
        self.close_points.clear();
        self.close_points.extend(visible.iter().map(|c| (c.time + self.width / 2.0, c.close)));
        if let Some(live) = live {
            self.close_points.push((input.now, live.close));
        }
        self.points.clear();
        if !input.line.is_empty() && (self.density > 0.01 || self.line_mode > 0.05) {
            let first = input.line.partition_point(|p| p.time <= left).saturating_sub(1);
            let last =
                (input.line.partition_point(|p| p.time <= self.end) + 1).min(input.line.len());
            let mut i = 0;
            for p in &input.line[first.min(last)..last] {
                while i + 1 < self.close_points.len() && self.close_points[i + 1].0 < p.time {
                    i += 1;
                }
                let base = if let Some(a) = self.close_points.get(i) {
                    if let Some(b) = self.close_points.get(i + 1) {
                        let t = ((p.time - a.0) / (b.0 - a.0).max(1e-9)).clamp(0.0, 1.0);
                        a.1 + (b.1 - a.1) * t
                    } else {
                        a.1
                    }
                } else {
                    p.value
                };
                self.points.push((
                    x(p.time),
                    ym(base + (p.value - base) * self.density).clamp(TOP, TOP + plot_h),
                ));
            }
        } else {
            self.points.extend(
                self.close_points.iter().map(|p| (x(p.0), ym(p.1).clamp(TOP, TOP + plot_h))),
            );
        }
        let tip_value = line_close.map(|v| v + (tick.unwrap_or(v) - v) * self.density);
        let tip_x = x(input.now);
        self.tip = None;
        self.off_plot = tip_value.is_some() && tip_x > LEFT + plot_w + 1.0 && view.end.is_some();
        if tip_x <= LEFT + plot_w + 1.0
            && let (Some(value), Some(last)) = (tip_value, self.points.last_mut())
        {
            last.1 = ym(value).clamp(TOP, TOP + plot_h);
            let tip_x = tip_x + (LEFT + plot_w - tip_x) * (1.0 - self.reveal);
            let append = (tip_x > last.0).then_some((tip_x, last.1));
            if let Some(point) = append {
                self.points.push(point);
            }
            self.tip = self.points.last().copied();
        }
        self.reveal_line(plot_w, plot_h, now_ms);
        if self.tip.is_some() {
            self.tip = self.points.last().copied();
        }
        spline(&self.points, &mut self.slopes, &mut self.controls);
        if let Scene::Reform { from, elapsed, .. } = &self.scene {
            let at = |px: f64| reform::progress(elapsed - reform::delay(px, LEFT, plot_w));
            reform::line(
                (from.points.as_slice(), from.controls.as_slice()),
                (self.points.as_slice(), self.controls.as_slice()),
                &mut self.reformed_points,
                at,
            );
            std::mem::swap(&mut self.points, &mut self.reformed_points);
            if self.tip.is_some() {
                self.tip = self.points.last().copied();
            }
            spline(&self.points, &mut self.slopes, &mut self.controls);
        }
        self.dot(now_ms);
        self.badge_value = tip_value;
        if let Some(v) = tip_value {
            // A re-forming badge rides its dot.
            let target = match (reform, self.tip) {
                (Some(_), Some(tip)) => tip.1,
                _ => y(v).clamp(TOP, TOP + plot_h),
            };
            if self.badge_y == 0.0 {
                self.badge_y = target;
            }
            self.moving |= approaching(&mut self.badge_y, target, 0.35, dt, 0.5, snap);
        }
        if self.points.len() >= 5 {
            let start = self.points.len().saturating_sub(20);
            let recent = &self.points[start..];
            let lo = recent.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
            let hi = recent.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
            let delta = recent[recent.len() - 5].1 - recent[recent.len() - 1].1;
            let target = if delta > (hi - lo) * 0.12 {
                1.0
            } else if delta < -(hi - lo) * 0.12 {
                0.0
            } else {
                self.momentum
            };
            self.moving |= approaching(&mut self.momentum, target, 0.12, dt, 0.01, snap);
        }
        self.update_grid(range, plot_h, dt, snap);
        // The new interval's time labels take over from the picture's as the
        // axis lands.
        let gate = reform.map_or(1.0, |_| ramp(settled, 0.35, 1.0));
        self.update_times(left, span, target_span, plot_w, input.day_anchor, dt, snap, gate);
        self.ghosts.clear();
        if let Scene::Reform { from, .. } = &self.scene {
            let fade = 1.0 - ramp(settled, 0.0, 0.5);
            self.ghosts.extend(
                from.times
                    .iter()
                    .map(|(tick, span)| (Tick { alpha: tick.alpha * fade, ..*tick }, *span)),
            );
        }
    }

    /// Keep up with what the input shows, changing scene when it changes.
    /// Returns whether the interval changed.
    fn follow(&mut self, input: &Input<'_>, has_data: bool, dt: f64, snap: bool) -> bool {
        let interval = self.width != 0.0 && self.width != input.candle_width;
        let feed = self.series.is_some_and(|series| series != input.series);
        // Something worth carrying over is on screen.
        let shown = self.frame.is_some() && self.reveal > 0.05;
        // Time passes for a running scene first; one that starts now shows its
        // first frame as it was left.
        let scene = match std::mem::replace(&mut self.scene, Scene::Live) {
            Scene::Hold { from, held } => Scene::Hold { from, held: held + dt },
            Scene::Reform { from, elapsed, range, dim } => {
                Scene::Reform { from, elapsed: elapsed + dt, range, dim }
            }
            scene => scene,
        };
        let scene = if feed {
            match scene {
                Scene::Hold { from, .. } | Scene::Fade(from) => Scene::Fade(from),
                _ if shown => Scene::Fade(self.capture()),
                _ => Scene::Live,
            }
        } else if interval {
            match scene {
                // Still waiting: keep the dim where it is, and wait afresh.
                Scene::Hold { from, held } => Scene::Hold { from, held: held.min(reform::DIMMED) },
                Scene::Fade(from) => Scene::Fade(from),
                _ if shown => Scene::Hold { from: self.capture(), held: 0.0 },
                _ => Scene::Live,
            }
        } else {
            scene
        };
        self.scene = match scene {
            Scene::Hold { from, .. } if has_data => {
                Scene::Reform { range: Some(from.range), from, elapsed: 0.0, dim: self.dim }
            }
            Scene::Hold { from, held } if !input.loading || held >= reform::PATIENCE => {
                Scene::Fade(from)
            }
            Scene::Reform { .. } if !has_data => Scene::Fade(self.capture()),
            Scene::Reform { elapsed, .. } if snap || elapsed >= reform::DURATION => Scene::Live,
            // Data again before the picture has sunk: it re-forms into the data.
            Scene::Fade(from) if has_data && self.reveal > 0.05 && !snap => {
                let same = from.series == input.series;
                let from = self.capture();
                Scene::Reform {
                    range: same.then_some(from.range),
                    from,
                    elapsed: 0.0,
                    dim: self.dim,
                }
            }
            Scene::Fade(_) if has_data || self.reveal < 0.005 => Scene::Live,
            Scene::Live if !has_data && self.frame.is_some() && self.reveal > 0.005 => {
                Scene::Fade(self.capture())
            }
            scene => scene,
        };
        if interval || feed {
            // Another interval's forming candle is another candle.
            self.live = None;
        }
        self.width = input.candle_width;
        self.series = Some(input.series);
        interval
    }

    /// What the last frame drew, frozen, for a scene to start from. Its
    /// buffers move into the picture, and the layers drawn in the frame's
    /// coordinates hide until the chart is back on data.
    fn capture(&mut self) -> Picture {
        let mut times: Vec<(Tick, f64)> =
            self.times.iter().map(|tick| (*tick, self.times_span)).collect();
        times.append(&mut self.ghosts);
        times.sort_by(|a, b| a.0.pos.total_cmp(&b.0.pos));
        self.times.clear();
        self.time_fades.clear();
        self.marks = 0.0;
        Picture {
            candles: std::mem::take(&mut self.candles),
            points: std::mem::take(&mut self.points),
            controls: std::mem::take(&mut self.controls),
            tip: self.tip,
            close_y: self.close_y,
            badge: self.badge_value,
            times,
            range: self.frame.map_or((0.0, 1.0), |frame| (frame.min, frame.max)),
            series: self.series.unwrap_or_default(),
        }
    }

    /// Draw a picture in place of the data: held while the interval it waits
    /// for loads, or sinking into the loading wave as the reveal runs out.
    #[allow(clippy::too_many_arguments)]
    fn show_picture(
        &mut self,
        input: &Input<'_>,
        left: f64,
        target_span: f64,
        plot_w: f64,
        plot_h: f64,
        dt: f64,
        snap: bool,
        now_ms: f64,
    ) {
        let (from, held) = match &self.scene {
            Scene::Hold { from, held } => (from, Some(*held)),
            Scene::Fade(from) => (from, None),
            _ => return,
        };
        let range = from.range;
        self.range = Some(range);
        if let Some(held) = held {
            self.dim = reform::dim(held);
            // Waiting on the history, and on the patience running out.
            self.moving = true;
        }
        self.moving |= approaching(&mut self.scrub, 0.0, 0.12, dt, 0.01, snap);
        if self.scrub == 0.0 {
            self.hover_x = None;
            self.last_hover = None;
        }
        self.marks = 0.0;
        self.frame = Some(Frame {
            left,
            right: self.end,
            min: range.0,
            max: range.1,
            plot_x: LEFT,
            plot_y: TOP,
            plot_w,
            plot_h,
            width: input.width,
            height: input.height,
            now: input.now,
            candle_width: self.width,
            hover_x: None,
            scrub: self.scrub,
            line_mode: self.line_mode,
            hover: None,
            settled: 0.0,
        });
        self.line_presence = self.line_mode.max(if self.line_mode >= 0.99 {
            1.0 - self.reveal
        } else {
            (1.0 - self.reveal).powi(3)
        });
        // Held, the picture stays as drawn. Sinking, its bars fold onto their
        // closes and its line settles into the wave, as a revealed chart's do
        // in reverse.
        let sinking = held.is_none();
        let collapse = if sinking { smoothstep(self.reveal) * (1.0 - self.line_mode) } else { 1.0 };
        let fade = if sinking { self.reveal * (1.0 - self.line_presence) } else { 1.0 };
        self.candles.clear();
        self.candles.extend(from.candles.iter().map(|c| {
            let fold = |v: f64| c.close + (v - c.close) * collapse;
            CandleShape {
                open: fold(c.open),
                high: fold(c.high),
                low: fold(c.low),
                alpha: c.alpha * fade,
                ..*c
            }
        }));
        self.points.clear();
        self.points.extend_from_slice(&from.points);
        self.tip = from.tip;
        self.close_y = from.close_y;
        self.badge_value = from.badge;
        self.off_plot = false;
        let label = if sinking { ramp(self.reveal, 0.25, 0.6) } else { 1.0 };
        self.ghosts.clear();
        self.ghosts.extend(
            from.times
                .iter()
                .map(|(tick, span)| (Tick { alpha: tick.alpha * label, ..*tick }, *span)),
        );
        self.times.clear();
        self.times_span = target_span;
        if sinking {
            self.reveal_line(plot_w, plot_h, now_ms);
            if self.tip.is_some() {
                self.tip = self.points.last().copied();
            }
            spline(&self.points, &mut self.slopes, &mut self.controls);
        } else if let Scene::Hold { from, .. } = &self.scene {
            self.controls.clear();
            self.controls.extend_from_slice(&from.controls);
        }
        self.dot(now_ms);
        if let Some(tip) = self.tip {
            self.moving |= approaching(&mut self.badge_y, tip.1, 0.35, dt, 0.5, snap);
        }
        self.update_grid(range, plot_h, dt, snap);
    }

    /// While the chart is not fully revealed, the line reaches across the plot
    /// and settles into the loading wave, from the edges in.
    fn reveal_line(&mut self, plot_w: f64, plot_h: f64, now_ms: f64) {
        if self.reveal >= 1.0 {
            return;
        }
        if let Some(first) = self.points.first().copied() {
            let step = plot_w / 32.0;
            if first.0 > LEFT + plot_w * 0.05 {
                let mut next = first.0 - step;
                while next >= LEFT - step {
                    self.points.insert(0, (next, first.1));
                    next -= step;
                }
            }
        }
        for p in &mut self.points {
            let u = ((p.0 - LEFT) / plot_w).clamp(0.0, 1.0);
            let local = ((self.reveal - (u - 0.5).abs() * 2.0 * 0.4) / 0.6).clamp(0.0, 1.0);
            let wy = wave(u, now_ms, plot_h);
            p.1 = wy + (p.1 - wy) * local;
        }
    }

    /// The live dot at the line's tip, and its ring.
    fn dot(&mut self, now_ms: f64) {
        if self.tip.is_some() && self.line_presence > 0.5 {
            self.dot_alpha =
                (self.line_presence - 0.5) * 2.0 * ((self.reveal - 0.3) / 0.7).clamp(0.0, 1.0);
            if self.line_presence > 0.8 && self.reveal > 0.6 && self.scrub < 0.3 {
                self.ring = pulse(now_ms, self.scrub);
            }
        }
    }

    /// Price gridlines for `range`, fading in and out as they come and go.
    fn update_grid(&mut self, range: (f64, f64), plot_h: f64, dt: f64, snap: bool) {
        let val_range = (range.1 - range.0).max(1e-3);
        let y = |v: f64| TOP + (1.0 - (v - range.0) / val_range) * plot_h;
        let coarse = grid_interval(val_range, plot_h, self.coarse);
        self.coarse = Some(coarse);
        let fine = coarse / 2.0;
        let fine_alpha = ((fine * plot_h / val_range - 40.0) / 20.0).clamp(0.0, 1.0);
        for tick in &mut self.grid_fades {
            tick.target = 0.0;
        }
        let mut value = (range.0 / fine).ceil() * fine;
        while value <= range.1 {
            let py = y(value);
            let edge = ((py - TOP).min(TOP + plot_h - py) / 32.0).clamp(0.0, 1.0);
            let major = (value / coarse - (value / coarse).round()).abs() < 0.01;
            set_fade(
                &mut self.grid_fades,
                key9(value),
                edge * if major { 1.0 } else { fine_alpha },
                0.18,
            );
            value += fine;
        }
        self.grid.clear();
        for tick in &mut self.grid_fades {
            let speed = if tick.target > tick.alpha { 0.18 } else { 0.12 };
            if tick.fresh {
                tick.fresh = false;
                if snap {
                    tick.alpha = tick.target;
                } else {
                    self.moving |= tick.alpha != tick.target;
                }
            } else {
                self.moving |= approaching(&mut tick.alpha, tick.target, speed, dt, 0.02, snap);
            }
            self.grid.push(Tick {
                value: tick.key,
                pos: y(tick.key),
                alpha: tick.alpha * ramp(self.reveal, 0.25, 0.6),
            });
        }
        self.grid_fades.retain(|t| t.target != 0.0 || t.alpha >= 0.01);
        self.grid.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    }

    /// Time labels for the stretch from `left`, spaced for `target_span`.
    /// `gate` holds them back while another picture's labels hand over.
    #[allow(clippy::too_many_arguments)]
    fn update_times(
        &mut self,
        left: f64,
        span: f64,
        target_span: f64,
        plot_w: f64,
        day_anchor: f64,
        dt: f64,
        snap: bool,
        gate: f64,
    ) {
        let x = |t: f64| LEFT + (t - left) / span * plot_w;
        let step = time_interval(target_span, plot_w);
        for tick in &mut self.time_fades {
            tick.target = 0.0;
        }
        // Epoch anchor for sub-day intervals; a local-midnight anchor is supplied for day ticks by the caller's time zone.
        let anchor = if step >= 86400.0 { day_anchor } else { 0.0 };
        let first = (left - step - anchor) / step;
        let mut t = if step >= 86400.0 { first.floor() } else { first.ceil() } * step + anchor;
        for _ in 0..30 {
            if t > self.end + step {
                break;
            }
            let px = x(t);
            let alpha = ((px - LEFT).min(LEFT + plot_w - px) / 50.0).clamp(0.0, 1.0);
            set_fade(&mut self.time_fades, (t * 100.0).round() / 100.0, alpha, 0.0);
            t += step;
        }
        self.times.clear();
        for tick in &mut self.time_fades {
            self.moving |= approaching(&mut tick.alpha, tick.target, 0.08, dt, 0.02, snap);
            self.times.push(Tick {
                value: tick.key,
                pos: x(tick.key),
                alpha: tick.alpha * ramp(self.reveal, 0.25, 0.6) * gate,
            });
        }
        self.time_fades.retain(|t| t.target != 0.0 || t.alpha >= 0.01);
        self.times.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        self.times_span = target_span;
    }
}

#[allow(clippy::too_many_arguments)]
fn add_candles(
    out: &mut Vec<CandleShape>,
    candles: &[Candle],
    live: Option<Candle>,
    width: f64,
    left: f64,
    right: f64,
    plot_w: f64,
    collapse: f64,
    alpha: f64,
    birth: f64,
    bull: f64,
    scrub: f64,
    hover: Option<f64>,
    x: &impl Fn(f64) -> f64,
    y: &impl Fn(f64) -> f64,
) -> usize {
    let body = (width * plot_w / (right - left) * 0.7).max(1.0);
    let wick = (body * 0.15).clamp(0.8, 2.0);
    let mut visited = 0;
    for (c, is_live) in candles.iter().copied().map(|c| (c, false)).chain(live.map(|c| (c, true))) {
        visited += 1;
        let px = x(c.time + width / 2.0);
        if px + body / 2.0 < LEFT || px - body / 2.0 > LEFT + plot_w {
            continue;
        }
        let draw = |v| y(c.close + (v - c.close) * collapse);
        let dim = hover.map_or(0.0, |h| ((px - h) / (1.5 * body)).clamp(0.0, 1.0) * scrub * 0.5);
        out.push(CandleShape {
            x: px,
            width: body,
            wick,
            open: draw(c.open),
            close: draw(c.close),
            high: draw(c.high),
            low: draw(c.low),
            bull: if is_live { bull } else { flag(c.close >= c.open) },
            alpha: alpha * (1.0 - dim) * if is_live { birth } else { 1.0 },
            live: is_live,
        });
    }
    visited
}

#[cfg(test)]
mod tests {
    use super::super::super::viewport::Viewport;
    use super::*;

    fn candle(time: f64, price: f64) -> Candle {
        Candle { time, open: price, high: price + 2.0, low: price - 2.0, close: price + 1.0 }
    }
    fn input(candles: &[Candle]) -> Input<'_> {
        Input {
            candles,
            live: None,
            line: &[],
            price: None,
            series: 0,
            candle_width: 60.0,
            now: 3600.0,
            day_anchor: 1_704_067_200.0,
            width: 650.0,
            height: 360.0,
            axis_width: 64.0,
            loading: false,
            line_mode: false,
            hover_x: None,
            reduce_motion: false,
        }
    }
    #[test]
    fn live_ring_expands_and_fades_over_nine_hundred_milliseconds() {
        assert_eq!(pulse(0.0, 0.0), Some((9.0, 0.35)));
        let (radius, alpha) = pulse(450.0, 0.0).unwrap();
        assert_eq!(radius, 15.0);
        assert!((alpha - 0.175).abs() < 1e-12);
        assert!(pulse(900.0, 0.0).is_none());
    }

    #[test]
    fn fixed_frame_sequence_has_deterministic_geometry_and_view_writeback() {
        let candles: Vec<_> =
            (0..60).map(|i| candle(f64::from(i) * 60.0, 100.0 + f64::from(i))).collect();
        let mut a = State::default();
        let mut b = State::default();
        let mut va = Viewport::new(3600.0, 60.0, 30.0).view;
        let mut vb = va.clone();
        for dt in [16.67, 8.0, 33.0, 100.0, 16.67] {
            a.advance(dt, input(&candles), &mut va);
            b.advance(dt, input(&candles), &mut vb);
            assert_eq!(va, vb);
            assert_eq!(a.frame.unwrap().left, b.frame.unwrap().left);
            assert_eq!(a.frame.unwrap().min, b.frame.unwrap().min);
            assert_eq!(a.candles.len(), b.candles.len());
            assert_eq!(a.reveal, b.reveal);
        }
        assert_eq!(va.right, Some(3627.0));
        assert_eq!(va.left, Some(1827.0));
        assert_eq!(va.plot_x, Some(14.0));
        assert_eq!(va.plot_w, Some(572.0));
        assert_eq!(va.now, 3600.0);
        assert_eq!(va.buffer, 0.015);
        assert_eq!(a.candles.len(), 30);
        assert!(a.candles.iter().all(|c| c.x >= 14.0 && c.x <= 586.0));
    }
    #[test]
    fn pinned_history_does_not_replace_its_last_point_with_the_live_tip() {
        let candles: Vec<_> =
            (0..60).map(|i| candle(f64::from(i) * 60.0, 100.0 + f64::from(i))).collect();
        let mut state = State::default();
        let mut view = Viewport::new(3600.0, 60.0, 15.0).view;
        view.end = Some(1800.0);
        view.snap = true;
        let mut data = input(&candles);
        data.live = Some(candle(3600.0, 170.0));
        data.line_mode = true;
        data.reduce_motion = true;
        state.advance(16.67, data, &mut view);
        assert!(state.tip.is_none());
        assert!((state.points.last().unwrap().0 - (14.0 + 930.0 / 900.0 * 572.0)).abs() < 1e-9);
        assert_eq!(state.frame.unwrap().right, 1800.0);
    }
    #[test]
    fn large_history_only_emits_visible_candles_and_settles_when_pinned() {
        let candles: Vec<_> = (0..50_000).map(|i| candle(f64::from(i) * 60.0, 100.0)).collect();
        let mut state = State::default();
        let mut view = Viewport::new(3_000_000.0, 60.0, 60.0).view;
        view.end = Some(2_999_000.0);
        view.snap = true;
        let mut data = input(&candles);
        data.now = 3_000_000.0;
        for _ in 0..300 {
            state.advance(16.67, data, &mut view);
        }
        assert!(state.candles.len() <= 61);
        assert!(!state.moving);
        let capacity = (
            state.candles.capacity(),
            state.grid.capacity(),
            state.times.capacity(),
            state.points.capacity(),
        );
        state.advance(16.67, data, &mut view);
        assert_eq!(
            capacity,
            (
                state.candles.capacity(),
                state.grid.capacity(),
                state.times.capacity(),
                state.points.capacity()
            )
        );
    }

    #[test]
    fn a_narrow_window_over_a_long_history_visits_only_what_it_draws() {
        let candles: Vec<_> = (0..50_000)
            .map(|i| {
                let price = 100.0 + (f64::from(i) * 0.07).sin() * 5.0;
                Candle {
                    time: f64::from(i) * 60.0,
                    open: price,
                    high: price + 2.0,
                    low: price - 2.0,
                    close: price + 1.0,
                }
            })
            .collect();
        let mut state = State::default();
        // 200 one-minute candles on screen, at the end of the history.
        let mut view = Viewport::new(3_000_000.0, 60.0, 200.0).view;
        view.end = Some(2_999_000.0);
        view.snap = true;
        let mut data = input(&candles);
        data.now = 3_000_000.0;
        for _ in 0..40 {
            state.advance(16.67, data, &mut view);
        }
        assert!(
            state.candles_visited <= 205,
            "built geometry for {} candles of 50,000",
            state.candles_visited
        );
        assert!(state.candles.len() <= 201);
        let started = std::time::Instant::now();
        for _ in 0..100 {
            state.advance(16.67, data, &mut view);
        }
        let per_frame = started.elapsed() / 100;
        assert!(per_frame < std::time::Duration::from_millis(5), "{per_frame:?} per frame");
        assert!(!state.moving, "the chart settles");
    }

    #[test]
    fn a_chart_following_the_live_edge_settles_between_ticks() {
        let candles: Vec<_> =
            (0..120).map(|i| candle(3600.0 - f64::from(119 - i) * 60.0, 100.0)).collect();
        let mut state = State::default();
        let mut view = Viewport::new(3600.0, 60.0, 60.0).view;
        let mut data = input(&candles);
        for frame in 0..300 {
            data.now = 3600.0 + f64::from(frame) * 0.016_67;
            state.advance(16.67, data, &mut view);
        }
        assert!(!state.moving, "the live edge alone does not ask for frames");
    }

    #[test]
    fn a_live_candle_that_stops_ticking_lets_the_chart_settle() {
        let candles: Vec<_> =
            (0..60).map(|i| candle(f64::from(i) * 60.0, 100.0 + f64::from(i))).collect();
        let mut state = State::default();
        let mut view = Viewport::new(3600.0, 60.0, 60.0).view;
        let mut data = input(&candles);
        data.now = 3660.0;
        data.live = Some(candle(3600.0, 160.0));
        data.price = Some(161.0);
        data.line = &[];
        for _ in 0..300 {
            state.advance(16.67, data, &mut view);
        }
        assert!(!state.moving, "a quiet live candle does not ask for frames");
    }

    #[test]
    fn line_mode_transition_reaches_midpoint_after_half_a_second_fraction() {
        let candles: Vec<_> = (0..60).map(|i| candle(f64::from(i) * 60.0, 100.0)).collect();
        let mut state = State::default();
        let mut view = Viewport::new(3600.0, 60.0, 60.0).view;
        let mut data = input(&candles);
        for _ in 0..100 {
            state.advance(50.0, data, &mut view);
        }
        let body = (state.candles[1].open - state.candles[1].close).abs();
        assert!(body > 0.0);
        data.line_mode = true;
        for _ in 0..5 {
            state.advance(50.0, data, &mut view);
        }
        assert!((state.line_mode - 0.5).abs() < 1e-12);
        assert!(((state.candles[1].open - state.candles[1].close).abs() - body / 2.0).abs() < 1e-9);
        for _ in 0..5 {
            state.advance(50.0, data, &mut view);
        }
        assert_eq!(state.line_mode, 1.0);
        assert_eq!(state.candles[1].open, state.candles[1].close);
    }

    /// `count` closed candles of `secs` each, the last one closing at `now`,
    /// each priced by `price`.
    fn history(secs: f64, count: u32, price: impl Fn(u32) -> f64) -> Vec<Candle> {
        (0..count).map(|i| candle(3600.0 - f64::from(count - i) * secs, price(i))).collect()
    }

    fn settle(state: &mut State, data: Input<'_>, view: &mut View) {
        for _ in 0..300 {
            state.advance(16.67, data, view);
        }
        assert!(!state.moving, "the chart settles");
    }

    fn shapes(candles: &[CandleShape]) -> Vec<(f64, f64, f64, f64, f64, f64)> {
        candles.iter().map(|c| (c.x, c.width, c.open, c.close, c.high, c.low)).collect()
    }

    /// The geometry a chart that only ever showed `data` settles on.
    fn settled_on(data: Input<'_>, secs: f64) -> State {
        let mut state = State::default();
        let mut view = Viewport::new(3600.0, secs, 60.0).view;
        settle(&mut state, data, &mut view);
        state
    }

    fn range(state: &State) -> (f64, f64) {
        state.frame.map(|frame| (frame.min, frame.max)).expect("a frame")
    }

    #[test]
    fn another_interval_re_forms_the_picture_in_place() {
        let minutes = history(60.0, 120, |i| 100.0 + f64::from(i % 7));
        let fives = history(300.0, 120, |i| 60.0 + f64::from(i % 11) * 8.0);
        let mut state = State::default();
        let mut viewport = Viewport::new(3600.0, 60.0, 60.0);
        let mut data = input(&minutes);
        settle(&mut state, data, &mut viewport.view);
        let before = shapes(&state.candles);
        let old_range = range(&state);
        assert_eq!(state.frame.unwrap().settled, 1.0);

        viewport.select_interval(300.0);
        data.candles = &fives;
        data.candle_width = 300.0;
        state.advance(16.67, data, &mut viewport.view);
        // The first frame is the picture that was on screen, over the span the
        // new interval shows: no zoom between them.
        assert_eq!(shapes(&state.candles), before);
        assert_eq!(range(&state), old_range);
        assert_eq!(viewport.view.right.unwrap() - viewport.view.left.unwrap(), 18_000.0);
        assert_eq!(state.frame.unwrap().settled, 0.0);

        let target = settled_on(data, 300.0);
        let new_range = range(&target);
        let hull = (old_range.0.min(new_range.0), old_range.1.max(new_range.1));
        let plot = (TOP, TOP + target.frame.unwrap().plot_h);
        let mut previous_live = f64::INFINITY;
        for frame in 1..=40 {
            state.advance(16.67, data, &mut viewport.view);
            let (lo, hi) = range(&state);
            assert!(
                lo >= hull.0 - 1e-9 && hi <= hull.1 + 1e-9,
                "the axis stays between the ranges"
            );
            for c in &state.candles {
                assert!(c.high >= plot.0 - 1e-9 && c.low <= plot.1 + 1e-9, "bars stay on the plot");
            }
            // The live edge leads the sweep: the right side is further along.
            let along = |c: &CandleShape, i: usize| {
                let (from, to) = (
                    before[before.len() - 1 - i].3,
                    target.candles[target.candles.len() - 1 - i].close,
                );
                if (to - from).abs() < 1e-9 { 1.0 } else { (c.close - from) / (to - from) }
            };
            let n = state.candles.len();
            if frame < 20 {
                let right = along(&state.candles[n - 1], 0);
                let left = along(&state.candles[0], n - 1);
                assert!(right >= left - 1e-9, "frame {frame}: {right} behind {left}");
                assert!(right <= previous_live + 1.0);
                previous_live = right;
            }
        }
        // Landed: exactly what a chart that only ever showed five-minute bars draws.
        assert!(matches!(state.scene, Scene::Live));
        assert_eq!(shapes(&state.candles), shapes(&target.candles));
        assert_eq!(range(&state), new_range);
        assert_eq!(state.dim, 1.0);
        settle(&mut state, data, &mut viewport.view);
        assert_eq!(state.frame.unwrap().settled, 1.0);
    }

    #[test]
    fn an_interval_that_has_to_load_holds_the_picture_until_its_history_arrives() {
        let minutes = history(60.0, 120, |i| 100.0 + f64::from(i % 7));
        let fives = history(300.0, 120, |i| 60.0 + f64::from(i % 11) * 8.0);
        let mut state = State::default();
        let mut viewport = Viewport::new(3600.0, 60.0, 60.0);
        let mut data = input(&minutes);
        settle(&mut state, data, &mut viewport.view);
        let before = shapes(&state.candles);
        let old_range = range(&state);

        viewport.select_interval(300.0);
        data.candles = &[];
        data.candle_width = 300.0;
        data.loading = true;
        for _ in 0..30 {
            state.advance(16.67, data, &mut viewport.view);
            // Held as drawn: no placeholder range, no loading wave, no reveal.
            assert_eq!(shapes(&state.candles), before);
            assert_eq!(range(&state), old_range);
            assert_eq!((state.reveal, state.loading), (1.0, 0.0));
            assert!(state.moving, "the wait keeps time");
        }
        assert!(state.dim < 0.6, "a wait that shows dims the picture: {}", state.dim);

        data.candles = &fives;
        data.loading = false;
        state.advance(16.67, data, &mut viewport.view);
        assert_eq!(shapes(&state.candles), before, "it re-forms from the held picture");
        for _ in 0..40 {
            state.advance(16.67, data, &mut viewport.view);
        }
        let target = settled_on(data, 300.0);
        assert_eq!(shapes(&state.candles), shapes(&target.candles));
        assert_eq!(state.dim, 1.0);
    }

    #[test]
    fn a_history_that_takes_too_long_lets_the_picture_sink_into_the_loading_wave() {
        let minutes = history(60.0, 120, |i| 100.0 + f64::from(i % 7));
        let mut state = State::default();
        let mut viewport = Viewport::new(3600.0, 60.0, 60.0);
        let mut data = input(&minutes);
        settle(&mut state, data, &mut viewport.view);
        let old_range = range(&state);
        viewport.select_interval(300.0);
        data.candles = &[];
        data.candle_width = 300.0;
        data.loading = true;
        let patience = (reform::PATIENCE / 16.67).ceil() as usize;
        for _ in 0..patience {
            state.advance(16.67, data, &mut viewport.view);
            assert_eq!(state.reveal, 1.0);
        }
        let mut sunk = false;
        for _ in 0..120 {
            state.advance(16.67, data, &mut viewport.view);
            match state.frame {
                // On its way down, the picture keeps the range it was drawn in.
                Some(frame) => assert_eq!((frame.min, frame.max), old_range),
                None => sunk = true,
            }
        }
        assert!(sunk && state.loading == 1.0, "the loading wave takes over");
    }

    #[test]
    fn switching_again_mid_way_starts_from_what_is_on_screen() {
        let minutes = history(60.0, 120, |i| 100.0 + f64::from(i % 7));
        let fives = history(300.0, 120, |i| 60.0 + f64::from(i % 11) * 8.0);
        let fifteens = history(900.0, 120, |i| 140.0 - f64::from(i % 5) * 9.0);
        let mut state = State::default();
        let mut viewport = Viewport::new(3600.0, 60.0, 60.0);
        let mut data = input(&minutes);
        settle(&mut state, data, &mut viewport.view);
        viewport.select_interval(300.0);
        data.candles = &fives;
        data.candle_width = 300.0;
        for _ in 0..8 {
            state.advance(16.67, data, &mut viewport.view);
        }
        let midway = shapes(&state.candles);
        let midway_range = range(&state);
        viewport.select_interval(900.0);
        data.candles = &fifteens;
        data.candle_width = 900.0;
        state.advance(16.67, data, &mut viewport.view);
        assert_eq!(shapes(&state.candles), midway);
        assert_eq!(range(&state), midway_range);
    }

    #[test]
    fn a_line_re_forms_from_its_tip() {
        let minutes = history(60.0, 120, |i| 100.0 + f64::from(i % 7));
        let fives = history(300.0, 120, |i| 60.0 + f64::from(i % 11) * 8.0);
        let mut state = State::default();
        let mut viewport = Viewport::new(3600.0, 60.0, 60.0);
        let mut data = input(&minutes);
        data.live = Some(candle(3600.0, 104.0));
        data.line_mode = true;
        settle(&mut state, data, &mut viewport.view);
        let (points, controls) = (state.points.clone(), state.controls.clone());
        let tip = state.tip.expect("a tip");
        viewport.select_interval(300.0);
        data.candles = &fives;
        data.candle_width = 300.0;
        data.live = Some(candle(3600.0, 104.0));
        state.advance(16.67, data, &mut viewport.view);
        // The first frame traces the drawn curve, ending on the same tip.
        assert_eq!(state.tip, Some(tip));
        let mut probe = Vec::new();
        reform::line(
            (&points[..], &controls[..]),
            (&points[..], &controls[..]),
            &mut probe,
            |_| 0.0,
        );
        assert_eq!(state.points, probe);
        for _ in 0..40 {
            state.advance(16.67, data, &mut viewport.view);
        }
        let target = settled_on(data, 300.0);
        assert_eq!(state.points, target.points);
    }

    #[test]
    fn another_feed_sinks_the_picture_instead_of_re_forming_it() {
        let minutes = history(60.0, 120, |i| 100.0 + f64::from(i % 7));
        let mut state = State::default();
        let mut viewport = Viewport::new(3600.0, 60.0, 60.0);
        let mut data = input(&minutes);
        settle(&mut state, data, &mut viewport.view);
        let old_range = range(&state);
        data.series = 1;
        data.candles = &[];
        data.loading = true;
        state.advance(16.67, data, &mut viewport.view);
        assert!(matches!(state.scene, Scene::Fade(_)));
        assert!(state.reveal < 1.0);
        assert_eq!(range(&state), old_range);
    }
}
