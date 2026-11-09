//! How the chart changes what it shows.
//!
//! A new candle interval does not zoom the chart. The bars and the line already
//! on screen re-form in place into the new interval's, the live edge first, so
//! the change sweeps from the present into the past. Every mark follows the same
//! critically damped spring, started later the further left it sits, from where
//! it was drawn to where the new data puts it; the price axis glides to the new
//! range on the live edge's clock.
//!
//! An interval that still has to load holds the picture on screen, dimmed, until
//! its history arrives. One that takes too long, and a chart whose data goes
//! away, lets the picture sink into the loading wave instead.

use gpui_kit::{SpringConfig, SpringState};

use super::math::smoothstep;
use super::state::{CandleShape, Tick};

/// Milliseconds one mark takes to re-form.
pub(super) const SETTLE: f64 = 380.0;
/// How much later the plot's left edge starts than its right edge.
const SWEEP: f64 = 160.0;
/// Milliseconds from the first mark starting to the last one landing.
pub(super) const DURATION: f64 = SETTLE + SWEEP;
/// A held picture starts dimming after `DIM_FROM` and is fully dimmed at
/// `DIMMED`, by `DIM_DEPTH`: a quick answer never flickers it.
const DIM_FROM: f64 = 120.0;
pub(super) const DIMMED: f64 = 420.0;
const DIM_DEPTH: f64 = 0.5;
/// After this long without the new interval's history, the held picture gives
/// way to the loading wave.
pub(super) const PATIENCE: f64 = 1500.0;
/// Pixels between the samples a re-forming line is drawn through.
const LINE_STEP: f64 = 2.0;

/// Critically damped, so no mark passes through a price it never had on its
/// way: ω₀ = 22/s, ζ = 1.
const SPRING: SpringConfig = SpringConfig::new(484.0, 44.0, 1.0);

/// A drawn curve: its points, and the spline's controls between them.
pub(super) type Curve<'a> = (&'a [(f64, f64)], &'a [[(f64, f64); 3]]);

/// What the chart drew, frozen in pixels.
pub(super) struct Picture {
    pub candles: Vec<CandleShape>,
    pub points: Vec<(f64, f64)>,
    pub controls: Vec<[(f64, f64); 3]>,
    pub tip: Option<(f64, f64)>,
    pub close_y: Option<f64>,
    pub badge: Option<f64>,
    /// The time labels shown, each with the span it reads for.
    pub times: Vec<(Tick, f64)>,
    /// The value range drawn.
    pub range: (f64, f64),
    /// The feed it showed.
    pub series: u64,
}

/// What the chart draws in place of the data, or on the way to it.
pub(super) enum Scene {
    /// The data as it is.
    Live,
    /// The picture of the interval just left, held while the new one loads.
    Hold { from: Picture, held: f64 },
    /// A picture re-forming into the data.
    Reform {
        from: Picture,
        elapsed: f64,
        /// The value range to glide from; `None` jumps to the new one, for a
        /// picture of another feed.
        range: Option<(f64, f64)>,
        /// The dim to brighten from.
        dim: f64,
    },
    /// A picture with no data behind it, sinking into the loading wave.
    Fade(Picture),
}

/// How far a mark has come, 0 to 1, `ms` after it started: the spring's
/// position, scaled to land exactly at `SETTLE`.
pub(super) fn progress(ms: f64) -> f64 {
    if ms <= 0.0 {
        return 0.0;
    }
    if ms >= SETTLE {
        return 1.0;
    }
    spring(ms) / spring(SETTLE)
}

fn spring(ms: f64) -> f64 {
    f64::from(SPRING.step(SpringState::default(), 1.0, (ms / 1000.0) as f32).position)
}

/// How long after the live edge a mark at `x` starts.
pub(super) fn delay(x: f64, plot_x: f64, plot_w: f64) -> f64 {
    SWEEP * (1.0 - ((x - plot_x) / plot_w).clamp(0.0, 1.0))
}

/// The share of its ink a picture held for `held` milliseconds shows.
pub(super) fn dim(held: f64) -> f64 {
    1.0 - DIM_DEPTH * smoothstep((held - DIM_FROM) / (DIMMED - DIM_FROM))
}

/// Between two value ranges: the middle moves straight and the height in
/// proportion, so a range a hundred times taller grows as evenly as a zoom.
pub(super) fn range(from: (f64, f64), to: (f64, f64), t: f64) -> (f64, f64) {
    if t >= 1.0 {
        return to;
    }
    let middle = |r: (f64, f64)| (r.0 + r.1) / 2.0;
    let height = |r: (f64, f64)| (r.1 - r.0).max(1e-12).ln();
    let m = middle(from) + (middle(to) - middle(from)) * t;
    let h = (height(from) + (height(to) - height(from)) * t).exp();
    (m - h / 2.0, m + h / 2.0)
}

/// Bars re-formed from `from` into `to`, paired from the right: the forming
/// bar into the forming bar, and so on into the past. A bar with no partner
/// grows from, or shrinks to, a line at its close. `at` gives the progress of
/// a mark at an x.
pub(super) fn bars(
    from: &[CandleShape],
    to: &[CandleShape],
    out: &mut Vec<CandleShape>,
    at: impl Fn(f64) -> f64,
) {
    out.clear();
    let (m, n) = (from.len(), to.len());
    for k in (0..m.max(n)).rev() {
        let a = (k < m).then(|| from[m - 1 - k]);
        let b = (k < n).then(|| to[n - 1 - k]);
        out.push(match (a, b) {
            (Some(a), Some(b)) => blend(a, b, at(b.x)),
            (None, Some(b)) => blend(flat(b), b, at(b.x)),
            (Some(a), None) => blend(a, flat(a), at(a.x)),
            (None, None) => continue,
        });
    }
}

fn flat(c: CandleShape) -> CandleShape {
    CandleShape { open: c.close, high: c.close, low: c.close, alpha: 0.0, ..c }
}

fn blend(a: CandleShape, b: CandleShape, t: f64) -> CandleShape {
    let mix = |from: f64, to: f64| from + (to - from) * t;
    CandleShape {
        x: mix(a.x, b.x),
        width: mix(a.width, b.width),
        wick: mix(a.wick, b.wick),
        open: mix(a.open, b.open),
        close: mix(a.close, b.close),
        high: mix(a.high, b.high),
        low: mix(a.low, b.low),
        // Red and green blended are brown: a bar takes its new colour at
        // half way, and the sweep carries the change across.
        bull: if t < 0.5 { a.bull } else { b.bull },
        alpha: mix(a.alpha, b.alpha),
        live: if t < 0.5 { a.live } else { b.live },
    }
}

/// A line re-formed from one drawn curve into another. Both are sampled at the
/// same share of their length, on the curve as drawn, every couple of pixels,
/// so the first frame is the old curve and the last is the new one.
pub(super) fn line(
    from: Curve<'_>,
    to: Curve<'_>,
    out: &mut Vec<(f64, f64)>,
    at: impl Fn(f64) -> f64,
) {
    out.clear();
    let ((a, a_controls), (b, b_controls)) = (from, to);
    if a.len() < 2 || b.len() < 2 {
        out.extend_from_slice(b);
        return;
    }
    let (a0, a1) = (a[0].0, a[a.len() - 1].0);
    let (b0, b1) = (b[0].0, b[b.len() - 1].0);
    let samples = ((a1 - a0).max(b1 - b0) / LINE_STEP).ceil().clamp(16.0, 1024.0) as usize;
    for i in 0..=samples {
        let u = i as f64 / samples as f64;
        // Exact at both ends: the tip stays on its pixel.
        let xa = a0 * (1.0 - u) + a1 * u;
        let xb = b0 * (1.0 - u) + b1 * u;
        let ya = curve(a, a_controls, xa);
        let yb = curve(b, b_controls, xb);
        let t = at(xb);
        out.push((xa + (xb - xa) * t, ya + (yb - ya) * t));
    }
}

/// The height of a drawn curve at `x`. The spline puts a segment's control
/// points at its thirds, so x is linear in the segment's parameter.
fn curve(points: &[(f64, f64)], controls: &[[(f64, f64); 3]], x: f64) -> f64 {
    let i = points.partition_point(|p| p.0 <= x).clamp(1, points.len() - 1) - 1;
    let (start, end) = (points[i], points[i + 1]);
    let [c1, c2, _] = controls[i];
    let w = end.0 - start.0;
    let t = if w > 0.0 { ((x - start.0) / w).clamp(0.0, 1.0) } else { 0.0 };
    let s = 1.0 - t;
    s * s * s * start.1 + 3.0 * s * s * t * c1.1 + 3.0 * s * t * t * c2.1 + t * t * t * end.1
}

#[cfg(test)]
mod tests {
    use super::super::math::spline;
    use super::*;

    fn shape(x: f64, close: f64) -> CandleShape {
        CandleShape {
            x,
            width: 6.0,
            wick: 1.0,
            open: close + 10.0,
            close,
            high: close - 5.0,
            low: close + 15.0,
            bull: 1.0,
            alpha: 1.0,
            live: false,
        }
    }

    #[test]
    fn a_mark_follows_the_spring_and_lands_on_time() {
        assert_eq!(progress(0.0), 0.0);
        assert_eq!(progress(SETTLE), 1.0);
        let mut previous = 0.0;
        for ms in (1..SETTLE as u32).step_by(10) {
            let p = progress(f64::from(ms));
            assert!(p > previous && p < 1.0, "{p} at {ms} ms");
            previous = p;
        }
        // It sets off gently and is nine tenths there halfway through: the
        // rest is the settle the eye reads as landing.
        assert!(progress(SETTLE / 10.0) < 0.3);
        assert!(progress(SETTLE / 2.0) > 0.9);
    }

    #[test]
    fn the_sweep_starts_at_the_live_edge() {
        assert_eq!(delay(100.0, 0.0, 100.0), 0.0);
        assert_eq!(delay(0.0, 0.0, 100.0), SWEEP);
        assert_eq!(delay(-50.0, 0.0, 100.0), SWEEP);
    }

    #[test]
    fn a_held_picture_dims_only_once_the_wait_shows() {
        assert_eq!(dim(0.0), 1.0);
        assert_eq!(dim(DIM_FROM), 1.0);
        assert_eq!(dim(DIMMED), 1.0 - DIM_DEPTH);
        assert_eq!(dim(PATIENCE), 1.0 - DIM_DEPTH);
    }

    #[test]
    fn a_range_grows_in_proportion() {
        let from = (99.0, 101.0);
        let to = (0.0, 200.0);
        assert_eq!(range(from, to, 0.0), from);
        assert_eq!(range(from, to, 1.0), to);
        let (lo, hi) = range(from, to, 0.5);
        // Halfway in proportion: twenty high, not a hundred.
        assert!((hi - lo - 20.0).abs() < 1e-9);
        assert!(((lo + hi) / 2.0 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn bars_pair_from_the_live_edge_and_unpaired_ones_grow_from_a_line() {
        let from = [shape(10.0, 50.0), shape(20.0, 60.0)];
        let to = [shape(5.0, 30.0), shape(15.0, 40.0), shape(25.0, 70.0)];
        let mut out = Vec::new();
        bars(&from, &to, &mut out, |_| 0.0);
        assert_eq!(out.len(), 3);
        // The newest bar starts as the newest drawn one.
        assert_eq!(out[2].x, 20.0);
        assert_eq!(out[2].close, 60.0);
        assert_eq!(out[1].x, 10.0);
        // The oldest has no partner: an invisible line at its own close.
        assert_eq!((out[0].x, out[0].alpha), (5.0, 0.0));
        assert_eq!((out[0].open, out[0].high, out[0].low), (30.0, 30.0, 30.0));
        bars(&from, &to, &mut out, |_| 1.0);
        for (a, b) in out.iter().zip(&to) {
            assert_eq!(
                (a.x, a.open, a.close, a.high, a.low, a.alpha),
                (b.x, b.open, b.close, b.high, b.low, b.alpha)
            );
        }
        // More bars before than after: the extra ones fold away where they were.
        bars(&to, &from, &mut out, |_| 1.0);
        assert_eq!(out.len(), 3);
        assert_eq!((out[0].x, out[0].alpha), (5.0, 0.0));
    }

    #[test]
    fn a_line_starts_as_the_drawn_curve_and_ends_as_the_new_one() {
        let a = [(0.0, 10.0), (40.0, 30.0), (80.0, 20.0), (120.0, 50.0)];
        let b = [(10.0, 60.0), (60.0, 15.0), (120.0, 40.0)];
        let (mut slopes, mut ac, mut bc) = (Vec::new(), Vec::new(), Vec::new());
        spline(&a, &mut slopes, &mut ac);
        spline(&b, &mut slopes, &mut bc);
        let mut out = Vec::new();
        for (t, points, controls) in [(0.0, &a[..], &ac), (1.0, &b[..], &bc)] {
            line((&a[..], &ac[..]), (&b[..], &bc[..]), &mut out, |_| t);
            assert_eq!(out.first().unwrap().0, points[0].0);
            assert_eq!(out.last().unwrap().0, points[points.len() - 1].0);
            for p in &out {
                assert!((p.1 - curve(points, controls, p.0)).abs() < 1e-9);
            }
            // The given points themselves are on it.
            for p in points {
                assert!((curve(points, controls, p.0) - p.1).abs() < 1e-9);
            }
        }
    }
}
