//! Time-independent chart calculations.

use crate::market::Candle;

pub(super) fn lerp(current: f64, target: f64, speed: f64, dt_ms: f64) -> f64 {
    current + (target - current) * (1.0 - (1.0 - speed).powf(dt_ms / 16.67))
}

pub(super) fn cosine(t: f64) -> f64 {
    (1.0 - (t.clamp(0.0, 1.0) * std::f64::consts::PI).cos()) / 2.0
}

pub(super) fn morph_width(from: f64, to: f64, t: f64) -> f64 {
    (from.ln() + (to.ln() - from.ln()) * cosine(t)).exp()
}

pub(super) fn candle_range(candles: &[Candle], live: Option<Candle>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for candle in candles.iter().copied().chain(live) {
        min = min.min(candle.low);
        max = max.max(candle.high);
    }
    if !min.is_finite() || !max.is_finite() {
        return (99.0, 101.0);
    }
    let range = max - min;
    if range == 0.0 { (min - 0.2, max + 0.2) } else { (min - range * 0.12, max + range * 0.12) }
}

pub(super) fn ease_range(
    current: (f64, f64),
    target: (f64, f64),
    height: f64,
    dt: f64,
) -> (f64, f64) {
    let range = (current.1 - current.0).max(1e-3);
    let gap = ((current.0 - target.0).abs() + (current.1 - target.1).abs()) / range;
    let speed = 0.15 + (1.0 - gap.min(1.0)) * 0.2;
    let threshold = 0.5 * range / height.max(1.0);
    let bound = |a: f64, b: f64| {
        let value = lerp(a, b, speed, dt);
        if (value - b).abs() < threshold { b } else { value }
    };
    (bound(current.0, target.0), bound(current.1, target.1))
}

pub(super) fn time_interval(span: f64, width: f64) -> f64 {
    let mut step = if span <= 15.0 {
        2.0
    } else if span <= 30.0 {
        5.0
    } else if span <= 60.0 {
        10.0
    } else if span <= 120.0 {
        15.0
    } else if span <= 300.0 {
        30.0
    } else if span <= 600.0 {
        60.0
    } else if span <= 1800.0 {
        300.0
    } else if span <= 3600.0 {
        600.0
    } else if span <= 14400.0 {
        1800.0
    } else if span <= 43200.0 {
        3600.0
    } else if span <= 86400.0 {
        7200.0
    } else if span <= 604800.0 {
        86400.0
    } else {
        604800.0
    };
    while step * width / span < 60.0 && step < span {
        step *= 2.0;
    }
    step
}

pub(super) fn grid_interval(range: f64, height: f64, previous: Option<f64>) -> f64 {
    if let Some(step) = previous {
        let px = step * height / range;
        if (18.0..=144.0).contains(&px) {
            return step;
        }
    }
    let base = 10f64.powf(range.log10().ceil());
    let mut best = base;
    for divisors in [[2.0, 2.5, 2.0], [2.0, 2.0, 2.5], [2.5, 2.0, 2.0]] {
        let mut step = base;
        let mut i = 0;
        loop {
            let next = step / divisors[i % 3];
            if next * height / range < 36.0 {
                break;
            }
            step = next;
            i += 1;
        }
        best = best.min(step);
    }
    best
}

/// Cubic controls for a monotone curve. `slopes` is reusable scratch storage.
pub(super) fn spline(
    points: &[(f64, f64)],
    slopes: &mut Vec<f64>,
    controls: &mut Vec<[(f64, f64); 3]>,
) {
    slopes.clear();
    controls.clear();
    if points.len() < 2 {
        return;
    }
    slopes.resize(points.len(), 0.0);
    let secant = |i: usize| {
        let h = points[i + 1].0 - points[i].0;
        if h > 0.0 { (points[i + 1].1 - points[i].1) / h } else { 0.0 }
    };
    slopes[0] = secant(0);
    slopes[points.len() - 1] = secant(points.len() - 2);
    for (i, slope) in slopes.iter_mut().enumerate().take(points.len() - 1).skip(1) {
        let a = secant(i - 1);
        let b = secant(i);
        *slope = if a * b <= 0.0 { 0.0 } else { (a + b) / 2.0 };
    }
    for i in 0..points.len() - 1 {
        let d = secant(i);
        if d == 0.0 {
            slopes[i] = 0.0;
            slopes[i + 1] = 0.0;
            continue;
        }
        let a = slopes[i] / d;
        let b = slopes[i + 1] / d;
        let sum = a * a + b * b;
        if sum > 9.0 {
            let scale = 3.0 / sum.sqrt();
            slopes[i] = scale * a * d;
            slopes[i + 1] = scale * b * d;
        }
    }
    for i in 0..points.len() - 1 {
        let h = (points[i + 1].0 - points[i].0) / 3.0;
        controls.push([
            (points[i].0 + h, points[i].1 + h * slopes[i]),
            (points[i + 1].0 - h, points[i + 1].1 - h * slopes[i + 1]),
            points[i + 1],
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_easing_is_frame_rate_independent() {
        assert!((lerp(0.0, 1.0, 0.2, 16.67) - 0.2).abs() < 1e-12);
        assert!((lerp(lerp(0.0, 1.0, 0.2, 8.335), 1.0, 0.2, 8.335) - 0.2).abs() < 1e-12);
        let mut n = 0.0;
        for _ in 0..100 {
            n = lerp(n, 1.0, 0.2, 16.67);
        }
        assert!((n - 1.0).abs() < 1e-9);
    }

    #[test]
    fn width_transition_is_cosine_in_log_space() {
        assert!((morph_width(60.0, 600.0, 0.0) - 60.0).abs() < 1e-9);
        assert!((morph_width(60.0, 600.0, 0.5) - 36000f64.sqrt()).abs() < 1e-9);
        assert!((morph_width(60.0, 600.0, 1.0) - 600.0).abs() < 1e-9);
    }

    #[test]
    fn candle_range_adds_the_specified_margin_and_eases() {
        let candles = [Candle { time: 0.0, open: 100.0, high: 110.0, low: 90.0, close: 105.0 }];
        assert_eq!(candle_range(&candles, None), (87.6, 112.4));
        assert_eq!(candle_range(&[], None), (99.0, 101.0));
        let flat = [Candle { time: 0.0, open: 100.0, high: 100.0, low: 100.0, close: 100.0 }];
        assert_eq!(candle_range(&flat, None), (99.8, 100.2));
        let eased = ease_range((90.0, 110.0), (80.0, 120.0), 400.0, 16.67);
        assert_eq!(eased, (88.5, 111.5));
    }

    #[test]
    fn axis_steps_follow_the_view_and_pixel_spacing() {
        assert_eq!(time_interval(60.0, 600.0), 10.0);
        assert_eq!(time_interval(3600.0, 600.0), 600.0);
        assert_eq!(time_interval(86400.0, 600.0), 14400.0);
        assert_eq!(grid_interval(100.0, 360.0, None), 10.0);
        assert_eq!(grid_interval(100.0, 360.0, Some(20.0)), 20.0);
    }

    #[test]
    fn spline_does_not_overshoot_monotone_data() {
        let points = [(0.0, 0.0), (1.0, 1.0), (2.0, 1.01), (3.0, 4.0)];
        let mut slopes = Vec::new();
        let mut controls = Vec::new();
        spline(&points, &mut slopes, &mut controls);
        for (i, segment) in controls.iter().enumerate() {
            for n in 0..=100 {
                let t = f64::from(n) / 100.0;
                let y = (1.0 - t).powi(3) * points[i].1
                    + 3.0 * (1.0 - t).powi(2) * t * segment[0].1
                    + 3.0 * (1.0 - t) * t * t * segment[1].1
                    + t.powi(3) * segment[2].1;
                assert!(y >= points[i].1 - 1e-10 && y <= points[i + 1].1 + 1e-10);
            }
        }
    }
}
