//! Animation and geometry for a price chart, without window-system types.

use super::super::{geometry::Frame, market::LinePoint, viewport::View};
use super::math::{
    candle_range, cosine, ease_range, grid_interval, lerp, morph_width, spline, time_interval,
};
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
    pub points: Vec<(f64, f64)>,
    pub controls: Vec<[(f64, f64); 3]>,
    pub wave_points: Vec<(f64, f64)>,
    pub wave_controls: Vec<[(f64, f64); 3]>,
    pub line_mode: f64,
    pub line_presence: f64,
    pub reveal: f64,
    pub loading: f64,
    pub scrub: f64,
    pub hover_x: Option<f64>,
    pub close_y: Option<f64>,
    pub tip: Option<(f64, f64)>,
    pub moving: bool,
    pub display_width: f64,
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
    width: f64,
    width_from: f64,
    width_elapsed: f64,
    morph_from: Option<(f64, f64)>,
    morph_to: Option<(f64, f64)>,
    stash: Vec<Candle>,
    stash_live: Option<Candle>,
    stash_price: Option<f64>,
    stash_line: Vec<LinePoint>,
    old: Vec<Candle>,
    old_live: Option<Candle>,
    coarse: Option<f64>,
    grid_fades: Vec<Fade>,
    time_fades: Vec<Fade>,
    slopes: Vec<f64>,
    close_points: Vec<(f64, f64)>,
    last_hover: Option<Candle>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            frame: None,
            candles: Vec::new(),
            grid: Vec::new(),
            times: Vec::new(),
            points: Vec::new(),
            controls: Vec::new(),
            wave_points: Vec::new(),
            wave_controls: Vec::new(),
            line_mode: 0.0,
            line_presence: 0.0,
            reveal: 0.0,
            loading: 0.0,
            scrub: 0.0,
            hover_x: None,
            close_y: None,
            tip: None,
            moving: false,
            display_width: 0.0,
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
            width_from: 0.0,
            width_elapsed: 300.0,
            morph_from: None,
            morph_to: None,
            stash: Vec::new(),
            stash_live: None,
            stash_price: None,
            stash_line: Vec::new(),
            old: Vec::new(),
            old_live: None,
            coarse: None,
            grid_fades: Vec::new(),
            time_fades: Vec::new(),
            slopes: Vec::new(),
            close_points: Vec::new(),
            last_hover: None,
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
fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn ramp(x: f64, a: f64, b: f64) -> f64 {
    smoothstep((x - a) / (b - a))
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
        if self.span.is_none() && input.loading {
            self.loading = 1.0;
        }
        self.moving = approaching(&mut self.loading, flag(input.loading), 0.14, dt, 0.01, snap);
        let reveal_target = flag(!input.loading && has_data);
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
        self.mode.set(flag(input.line_mode));
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
        let jump = std::mem::take(&mut view.snap) || snap;
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
        if self.width == 0.0 {
            self.width = input.candle_width;
            self.width_from = input.candle_width;
        }
        if self.width != input.candle_width {
            self.width_from = self.display_width.max(1e-6);
            self.width = input.candle_width;
            self.width_elapsed = 0.0;
            self.old.clear();
            self.old.extend_from_slice(&self.stash);
            self.old_live = self.stash_live;
            self.morph_from = self.range;
            let target_end = view.end.unwrap_or(input.now + target_span * BUFFER);
            let target_visible =
                select(input.candles, target_end - target_span, target_end, input.candle_width);
            self.morph_to = Some(candle_range(
                target_visible,
                input.live.filter(|c| {
                    c.time + input.candle_width >= target_end - target_span && c.time <= target_end
                }),
            ));
        }
        self.width_elapsed = if snap { 300.0 } else { (self.width_elapsed + dt).min(300.0) };
        let morph = cosine(self.width_elapsed / 300.0);
        self.display_width = morph_width(self.width_from, self.width, self.width_elapsed / 300.0);
        self.moving |= self.width_elapsed < 300.0;

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
                    self.live = Some(Candle {
                        open: raw.open,
                        high: raw.open,
                        low: raw.open,
                        close: raw.open,
                        time: raw.time,
                    });
                    self.birth = 0.0;
                }
            }
            self.moving |= approaching(&mut self.birth, 1.0, 0.2, dt, 0.01, snap);
            let live = self.live.unwrap();
            self.moving |=
                approaching(&mut self.bull, flag(live.close >= live.open), 0.12, dt, 0.01, snap);
        } else if has_data {
            self.live = None;
        }
        let use_stash = !has_data && self.reveal > 0.005 && !self.stash.is_empty();
        if !has_data && !use_stash {
            self.frame = None;
            self.candles.clear();
            self.grid.clear();
            self.times.clear();
            self.tip = None;
            self.points.clear();
            self.controls.clear();
            self.moving |= !snap;
            return;
        }
        let (visible, live) = if use_stash {
            (select(&self.stash, left, self.end, self.width), self.stash_live)
        } else {
            (select(input.candles, left, self.end, self.width), self.live)
        };
        let live = live.filter(|c| c.time + self.display_width >= left && c.time <= self.end);
        let raw_range = candle_range(visible, live);
        let target_range = match (self.morph_from, self.morph_to) {
            (Some(a), Some(b)) if self.width_elapsed < 300.0 => {
                (a.0 + (b.0 - a.0) * morph, a.1 + (b.1 - a.1) * morph)
            }
            _ => raw_range,
        };
        let range = self.range.map_or(target_range, |r| {
            if snap
                || (self.width_elapsed < 300.0
                    && self.morph_from.is_some()
                    && self.morph_to.is_some())
            {
                target_range
            } else {
                ease_range(r, target_range, plot_h, dt)
            }
        });
        self.range = Some(range);
        self.moving |= range != target_range;
        let val_range = (range.1 - range.0).max(1e-3);
        let x = |t: f64| LEFT + (t - left) / span * plot_w;
        let y = |v: f64| TOP + (1.0 - (v - range.0) / val_range) * plot_h;
        let raw_close =
            if use_stash { self.stash_live.map(|c| c.close) } else { input.live.map(|c| c.close) };
        let close = self.close_smooth.step(raw_close, val_range, dt, true, snap);
        let line_close = self.line_smooth.step(raw_close, val_range, dt, false, snap);
        let tick_price = if use_stash { self.stash_price } else { input.price };
        let tick = self.tick_smooth.step(
            tick_price.filter(|_| {
                if use_stash { !self.stash_line.is_empty() } else { !input.line.is_empty() }
            }),
            val_range,
            dt,
            false,
            snap,
        );
        self.moving |= close.zip(raw_close).is_some_and(|(a, b)| a != b)
            || line_close.zip(raw_close).is_some_and(|(a, b)| a != b)
            || tick.zip(tick_price).is_some_and(|(a, b)| a != b);
        self.close_y = close.map(y);
        let hovered = input.hover_x.filter(|x| *x >= LEFT && *x <= LEFT + plot_w);
        self.moving |= approaching(&mut self.scrub, flag(hovered.is_some()), 0.12, dt, 0.01, snap);
        if let Some(pos) = hovered {
            self.hover_x = Some(pos);
            let time = left + (pos - LEFT) / plot_w * span;
            let i = visible.partition_point(|c| c.time <= time);
            self.last_hover = live
                .filter(|c| time >= c.time && time < c.time + self.display_width)
                .or_else(|| {
                    i.checked_sub(1)
                        .and_then(|i| visible.get(i))
                        .copied()
                        .filter(|c| time < c.time + self.display_width)
                });
        } else if self.scrub == 0.0 {
            self.hover_x = None;
            self.last_hover = None;
        }
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
            candle_width: self.display_width,
            hover_x: self.last_hover.and(self.hover_x),
            scrub: self.scrub,
            line_mode: self.line_mode,
            hover: self.last_hover,
        });
        self.line_presence = self.line_mode.max(if self.line_mode >= 0.99 {
            1.0 - self.reveal
        } else {
            (1.0 - self.reveal).powi(3)
        });
        self.candles.clear();
        let collapse = smoothstep(self.reveal) * (1.0 - self.line_mode);
        let candle_alpha = self.reveal * (1.0 - self.line_presence);
        if self.width_elapsed < 300.0 {
            add_candles(
                &mut self.candles,
                &self.old,
                self.old_live,
                self.width_from,
                left,
                self.end,
                plot_w,
                collapse,
                candle_alpha * (1.0 - morph),
                self.birth,
                self.bull,
                self.scrub,
                self.hover_x,
                &x,
                &y,
            );
        }
        add_candles(
            &mut self.candles,
            visible,
            live,
            self.display_width,
            left,
            self.end,
            plot_w,
            collapse,
            candle_alpha * if self.width_elapsed < 300.0 { morph } else { 1.0 },
            self.birth,
            self.bull,
            self.scrub,
            self.hover_x,
            &x,
            &y,
        );
        self.close_points.clear();
        self.close_points
            .extend(visible.iter().map(|c| (c.time + self.display_width / 2.0, c.close)));
        if let Some(live) = live {
            self.close_points.push((input.now, live.close));
        }
        let line_data = if use_stash { self.stash_line.as_slice() } else { input.line };
        self.points.clear();
        if !line_data.is_empty() && (self.density > 0.01 || self.line_mode > 0.05) {
            let first = line_data.partition_point(|p| p.time <= left).saturating_sub(1);
            let last = (line_data.partition_point(|p| p.time <= self.end) + 1).min(line_data.len());
            let mut i = 0;
            for p in &line_data[first.min(last)..last] {
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
                    y(base + (p.value - base) * self.density).clamp(TOP, TOP + plot_h),
                ));
            }
        } else {
            self.points.extend(
                self.close_points.iter().map(|p| (x(p.0), y(p.1).clamp(TOP, TOP + plot_h))),
            );
        }
        let tip_value = line_close.map(|v| v + (tick.unwrap_or(v) - v) * self.density);
        let tip_x = x(input.now);
        self.tip = None;
        if tip_x <= LEFT + plot_w + 1.0
            && let (Some(value), Some(last)) = (tip_value, self.points.last_mut())
        {
            last.1 = y(value).clamp(TOP, TOP + plot_h);
            let tip_x = tip_x + (LEFT + plot_w - tip_x) * (1.0 - self.reveal);
            let append = (tip_x > last.0).then_some((tip_x, last.1));
            if let Some(point) = append {
                self.points.push(point);
            }
            self.tip = self.points.last().copied();
        }
        if self.reveal < 1.0
            && let Some(first) = self.points.first().copied()
        {
            let step = plot_w / 32.0;
            if first.0 > LEFT + plot_w * 0.05 {
                let mut next = first.0 - step;
                while next >= LEFT - step {
                    self.points.insert(0, (next, first.1));
                    next -= step;
                }
            }
        }
        if self.reveal < 1.0 {
            for p in &mut self.points {
                let u = ((p.0 - LEFT) / plot_w).clamp(0.0, 1.0);
                let local = ((self.reveal - (u - 0.5).abs() * 2.0 * 0.4) / 0.6).clamp(0.0, 1.0);
                let wy = wave(u, input.now * 1000.0, plot_h);
                p.1 = wy + (p.1 - wy) * local;
            }
        }
        if self.tip.is_some() {
            self.tip = self.points.last().copied();
        }
        spline(&self.points, &mut self.slopes, &mut self.controls);
        if self.tip.is_some() && self.line_presence > 0.5 {
            self.dot_alpha =
                (self.line_presence - 0.5) * 2.0 * ((self.reveal - 0.3) / 0.7).clamp(0.0, 1.0);
            if self.line_presence > 0.8 && self.reveal > 0.6 && self.scrub < 0.3 {
                self.ring = pulse(now_ms, self.scrub);
            }
        }
        self.badge_value = tip_value;
        if let Some(v) = tip_value {
            let target = y(v).clamp(TOP, TOP + plot_h);
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
        let step = time_interval(target_span, plot_w);
        for tick in &mut self.time_fades {
            tick.target = 0.0;
        }
        // Epoch anchor for sub-day intervals; a local-midnight anchor is supplied for day ticks by the caller's time zone.
        let anchor = if step >= 86400.0 { input.day_anchor } else { 0.0 };
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
                alpha: tick.alpha * ramp(self.reveal, 0.25, 0.6),
            });
        }
        self.time_fades.retain(|t| t.target != 0.0 || t.alpha >= 0.01);
        self.times.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        if !use_stash {
            self.stash.clear();
            self.stash.extend_from_slice(select(input.candles, left, self.end, self.width));
            self.stash_live = live;
            self.stash_price = input.price;
            self.stash_line.clear();
            let first = input.line.partition_point(|p| p.time <= left).saturating_sub(1);
            let last =
                (input.line.partition_point(|p| p.time <= self.end) + 1).min(input.line.len());
            self.stash_line.extend_from_slice(&input.line[first.min(last)..last]);
        }
        if !snap
            && (live.is_some() && self.line_presence < 0.99
                || self.loading > 0.0
                || (self.tip.is_some() && self.line_presence > 0.8 && self.reveal > 0.6))
        {
            self.moving = true;
        }
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
) {
    let body = (width * plot_w / (right - left) * 0.7).max(1.0);
    let wick = (body * 0.15).clamp(0.8, 2.0);
    for (c, is_live) in candles.iter().copied().map(|c| (c, false)).chain(live.map(|c| (c, true))) {
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
}
