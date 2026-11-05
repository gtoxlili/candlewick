//! Between the chart's coordinates and the canvas: mapping a frame, snapping
//! the pointer to candles, and finding the drawing under it.

use crate::market::Candle;

use super::model::{Anchor, Drawing, FIB_LEVELS};

/// A point in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
}

/// One frame as drawn: the plot's rect in logical pixels, the eased time and
/// value ranges, and the candle under the pointer.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// Epoch seconds at the plot's edges.
    pub left: f64,
    pub right: f64,
    /// The value range drawn.
    pub min: f64,
    pub max: f64,
    pub plot_x: f64,
    pub plot_y: f64,
    pub plot_w: f64,
    pub plot_h: f64,
    /// Canvas size in pixels and the chart clock in epoch seconds.
    pub width: f64,
    pub height: f64,
    pub now: f64,
    /// Displayed candle interval, which can be between two selected intervals.
    pub candle_width: f64,
    /// Pointer x and fade while the hovered candle remains visible.
    pub hover_x: Option<f64>,
    pub scrub: f64,
    pub line_mode: f64,
    /// The candle under the pointer; `None` while not hovering.
    pub hover: Option<Candle>,
}

impl Frame {
    /// Whether two frames map time and price to the same pixels, within a
    /// twentieth of a pixel: what another layer painting over the chart
    /// depends on. `now` moves on every frame whether or not anything was
    /// drawn differently, and the live edge drifts with it, so an exact
    /// comparison would repaint those layers on every tick.
    pub fn same_geometry(&self, other: &Self) -> bool {
        let candle = |c: Option<Candle>| c.map(|c| (c.time, c.open, c.high, c.low, c.close));
        let span = (self.right - self.left).abs().max(1e-9);
        let range = (self.max - self.min).abs().max(1e-12);
        let dx = |a: f64, b: f64| ((b - a) / span * self.plot_w).abs() < 0.05;
        let dy = |a: f64, b: f64| ((b - a) / range * self.plot_h).abs() < 0.05;
        let near = |a: f64, b: f64, tolerance: f64| (a - b).abs() < tolerance;
        self.plot_x == other.plot_x
            && self.plot_y == other.plot_y
            && self.plot_w == other.plot_w
            && self.plot_h == other.plot_h
            && dx(self.left, other.left)
            && dx(self.right, other.right)
            && dy(self.min, other.min)
            && dy(self.max, other.max)
            && near(self.candle_width, other.candle_width, 1e-6)
            && self.hover_x.is_none() == other.hover_x.is_none()
            && self.hover_x.zip(other.hover_x).is_none_or(|(a, b)| near(a, b, 0.05))
            && near(self.scrub, other.scrub, 0.002)
            && near(self.line_mode, other.line_mode, 0.002)
            && candle(self.hover) == candle(other.hover)
    }
}

impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        let candle = |c: Option<Candle>| c.map(|c| (c.time, c.open, c.high, c.low, c.close));
        self.left == other.left
            && self.right == other.right
            && self.min == other.min
            && self.max == other.max
            && self.plot_x == other.plot_x
            && self.plot_y == other.plot_y
            && self.plot_w == other.plot_w
            && self.plot_h == other.plot_h
            && self.width == other.width
            && self.height == other.height
            && self.now == other.now
            && self.candle_width == other.candle_width
            && self.hover_x == other.hover_x
            && self.scrub == other.scrub
            && self.line_mode == other.line_mode
            && candle(self.hover) == candle(other.hover)
    }
}

/// Time and price to pixels and back, for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    left: f64,
    span: f64,
    min: f64,
    range: f64,
    plot_x: f64,
    plot_y: f64,
    plot_w: f64,
    plot_h: f64,
}

pub fn mapping(frame: &Frame) -> Mapping {
    Mapping {
        left: frame.left,
        span: nonzero(frame.right - frame.left, 1.0),
        min: frame.min,
        range: nonzero(frame.max - frame.min, 1e-9),
        plot_x: frame.plot_x,
        plot_y: frame.plot_y,
        plot_w: frame.plot_w,
        plot_h: frame.plot_h,
    }
}

/// The chart's fallbacks for a zero range: an empty frame still maps.
fn nonzero(value: f64, fallback: f64) -> f64 {
    if value == 0.0 || value.is_nan() { fallback } else { value }
}

impl Mapping {
    pub fn x(&self, time: f64) -> f64 {
        self.plot_x + ((time - self.left) / self.span) * self.plot_w
    }

    pub fn y(&self, price: f64) -> f64 {
        self.plot_y + (1.0 - (price - self.min) / self.range) * self.plot_h
    }

    pub fn time(&self, x: f64) -> f64 {
        self.left + ((x - self.plot_x) / self.plot_w) * self.span
    }

    pub fn price(&self, y: f64) -> f64 {
        self.min + (1.0 - (y - self.plot_y) / self.plot_h) * self.range
    }

    pub fn plot(&self) -> Rect {
        Rect { x: self.plot_x, y: self.plot_y, w: self.plot_w, h: self.plot_h }
    }
}

/// Pixels within which the pointer sticks to a candle's open, high, low or close.
const SNAP_PX: f64 = 8.0;
const HANDLE_PX: f64 = 7.0;
const LINE_PX: f64 = 5.0;

/// Where a point lands in the chart: on the hovered candle's center and,
/// when close enough, on one of its prices (`free` turns the latter off).
pub fn snap(frame: &Frame, map: &Mapping, p: Point, interval: f64, free: bool) -> Anchor {
    let time = match frame.hover {
        Some(candle) => candle.time + interval / 2.0,
        None => map.time(p.x),
    };
    let mut price = map.price(p.y);
    if let Some(candle) = frame.hover
        && !free
    {
        let mut best = SNAP_PX;
        for level in [candle.open, candle.high, candle.low, candle.close] {
            let d = (map.y(level) - p.y).abs();
            if d < best {
                best = d;
                price = level;
            }
        }
    }
    Anchor { time, price }
}

/// Whether `anchor` sits exactly on one of the candle's prices (it snapped).
pub fn snapped_to_candle(frame: &Frame, anchor: Anchor) -> bool {
    frame.hover.is_some_and(|c| [c.open, c.high, c.low, c.close].contains(&anchor.price))
}

/// A drawing's body, or one of its ends by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitPart {
    Body,
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: String,
    pub part: HitPart,
}

/// How far `p` is from the segment `a`–`b`; the nearest end past either end.
pub fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    };
    (p.x - (a.x + t * dx)).hypot(p.y - (a.y + t * dy))
}

fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Where a fib's level lines run: from the earlier end to the plot's right edge.
pub fn fib_span(a: Point, b: Point, plot: Rect) -> (f64, f64) {
    (a.x.min(b.x), plot.x + plot.w)
}

/// The seven retracement values between `from` and `to`, in the same unit:
/// prices for the labels, pixel y for the lines.
pub fn fib_values(from: f64, to: f64) -> [f64; FIB_LEVELS.len()] {
    FIB_LEVELS.map(|level| from + (to - from) * level)
}

/// The box a measure covers between its two ends.
pub fn measure_box(a: Point, b: Point) -> Rect {
    Rect { x: a.x.min(b.x), y: a.y.min(b.y), w: (b.x - a.x).abs(), h: (b.y - a.y).abs() }
}

/// The topmost drawing under `p`, ends before bodies, latest drawn first.
pub fn hit_test(drawings: &[Drawing], map: &Mapping, p: Point) -> Option<Hit> {
    for drawing in drawings.iter().rev() {
        let Some(hit) = hit_one(drawing, map, p) else { continue };
        return Some(hit);
    }
    None
}

fn hit_one(drawing: &Drawing, map: &Mapping, p: Point) -> Option<Hit> {
    let id = drawing.id();
    if let Drawing::Level { price, .. } = drawing {
        return ((map.y(*price) - p.y).abs() <= LINE_PX).then(|| hit(id, HitPart::Body));
    }
    let (a, b) = match drawing {
        Drawing::Trend { a, b, .. } | Drawing::Fib { a, b, .. } | Drawing::Measure { a, b, .. } => {
            (*a, *b)
        }
        Drawing::Level { .. } => unreachable!("handled above"),
    };
    let pa = Point { x: map.x(a.time), y: map.y(a.price) };
    let pb = Point { x: map.x(b.time), y: map.y(b.price) };
    if distance(pa, p) <= HANDLE_PX {
        return Some(hit(id, HitPart::Start));
    }
    if distance(pb, p) <= HANDLE_PX {
        return Some(hit(id, HitPart::End));
    }
    match drawing {
        Drawing::Trend { .. } => {
            (distance_to_segment(p, pa, pb) <= LINE_PX).then(|| hit(id, HitPart::Body))
        }
        Drawing::Fib { .. } => {
            let (x0, x1) = fib_span(pa, pb, map.plot());
            let on_level = fib_values(pa.y, pb.y)
                .iter()
                .any(|y| p.x >= x0 - LINE_PX && p.x <= x1 && (y - p.y).abs() <= LINE_PX);
            (on_level || distance_to_segment(p, pa, pb) <= LINE_PX).then(|| hit(id, HitPart::Body))
        }
        Drawing::Measure { .. } => {
            let box_ = measure_box(pa, pb);
            (p.x >= box_.x - LINE_PX
                && p.x <= box_.right() + LINE_PX
                && p.y >= box_.y - LINE_PX
                && p.y <= box_.bottom() + LINE_PX)
                .then(|| hit(id, HitPart::Body))
        }
        Drawing::Level { .. } => unreachable!("handled above"),
    }
}

fn hit(id: &str, part: HitPart) -> Hit {
    Hit { id: id.to_owned(), part }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-9;

    fn frame() -> Frame {
        Frame {
            left: 1000.0,
            right: 1600.0,
            min: 90.0,
            max: 110.0,
            plot_x: 40.0,
            plot_y: 10.0,
            plot_w: 600.0,
            plot_h: 300.0,
            width: 720.0,
            height: 340.0,
            now: 1600.0,
            candle_width: 60.0,
            hover_x: None,
            scrub: 0.0,
            line_mode: 0.0,
            hover: None,
        }
    }

    fn candle(time: f64, open: f64, high: f64, low: f64, close: f64) -> Candle {
        Candle { time, open, high, low, close }
    }

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn anchor(time: f64, price: f64) -> Anchor {
        Anchor { time, price }
    }

    fn level(id: &str, price: f64) -> Drawing {
        Drawing::Level { id: id.to_owned(), price }
    }

    fn trend(id: &str, a: Anchor, b: Anchor) -> Drawing {
        Drawing::Trend { id: id.to_owned(), a, b }
    }

    fn fib(id: &str, a: Anchor, b: Anchor) -> Drawing {
        Drawing::Fib { id: id.to_owned(), a, b }
    }

    fn measure(id: &str, a: Anchor, b: Anchor) -> Drawing {
        Drawing::Measure { id: id.to_owned(), a, b }
    }

    fn hit_at(drawing: &Drawing, p: Point) -> Option<Hit> {
        hit_test(std::slice::from_ref(drawing), &mapping(&frame()), p)
    }

    #[test]
    fn a_rect_contains_the_points_on_its_edges() {
        let rect = Rect { x: 40.0, y: 10.0, w: 600.0, h: 300.0 };
        assert!(rect.contains(point(40.0, 10.0)));
        assert!(rect.contains(point(640.0, 310.0)));
        assert!(!rect.contains(point(39.9, 10.0)));
        assert!(!rect.contains(point(640.1, 310.0)));
        assert_eq!((rect.right(), rect.bottom()), (640.0, 310.0));
    }

    #[test]
    fn a_frame_maps_both_ways() {
        let map = mapping(&frame());
        assert_eq!(map.x(1300.0), 340.0);
        assert_eq!(map.y(100.0), 160.0);
        assert_eq!(map.time(340.0), 1300.0);
        assert_eq!(map.price(160.0), 100.0);
        assert_eq!(map.plot(), Rect { x: 40.0, y: 10.0, w: 600.0, h: 300.0 });
    }

    #[test]
    fn an_empty_frame_still_maps() {
        let mut frame = frame();
        frame.right = frame.left;
        frame.max = frame.min;
        let map = mapping(&frame);
        assert_eq!(map.x(1000.0), 40.0);
        assert!(map.y(-0.0).is_finite());
    }

    #[test]
    fn a_candle_snaps_within_eight_pixels() {
        let mut frame = frame();
        frame.hover = Some(candle(1300.0, 100.0, 101.0, 99.0, 100.5));
        let map = mapping(&frame);
        let price = |y| snap(&frame, &map, point(340.0, y), 60.0, false).price;
        // The candle's prices sit at y 160 (open), 145 (high), 175 (low) and 152.5 (close).
        assert_eq!(price(153.0), 100.5);
        assert_eq!(price(146.0), 101.0);
        assert_eq!(price(172.5), 99.0);
        assert_eq!(price(160.0), 100.0);
        // Exactly eight pixels from the nearest price is already loose ground.
        assert!((price(130.0) - map.price(130.0)).abs() < EPSILON);
        // Alt picks the pointer's own price.
        assert!(
            (snap(&frame, &map, point(340.0, 153.0), 60.0, true).price - map.price(153.0)).abs()
                < EPSILON
        );
    }

    #[test]
    fn snapping_to_a_flat_candle_stops_at_eight_pixels() {
        let mut frame = frame();
        frame.hover = Some(candle(1300.0, 100.0, 100.0, 100.0, 100.0));
        let map = mapping(&frame);
        assert_eq!(snap(&frame, &map, point(340.0, 167.9), 60.0, false).price, 100.0);
        assert!(snap(&frame, &map, point(340.0, 168.0), 60.0, false).price != 100.0);
    }

    #[test]
    fn a_snapped_point_sits_on_the_candle_center() {
        let mut frame = frame();
        frame.hover = Some(candle(1300.0, 100.0, 101.0, 99.0, 100.5));
        let map = mapping(&frame);
        let snapped = snap(&frame, &map, point(500.0, 153.0), 60.0, false);
        assert_eq!(snapped.time, 1330.0);
        assert!(snapped_to_candle(&frame, snapped));
        assert!(!snapped_to_candle(&frame, Anchor { time: 1330.0, price: 100.25 }));

        frame.hover = None;
        let loose = snap(&frame, &map, point(500.0, 153.0), 60.0, false);
        assert_eq!(loose.time, 1460.0);
        assert_eq!(loose.price, map.price(153.0));
        assert!(!snapped_to_candle(&frame, loose));
    }

    #[test]
    fn distance_to_a_segment_handles_ends_and_zero_length() {
        let a = point(0.0, 0.0);
        let b = point(10.0, 0.0);
        assert!((distance_to_segment(point(4.0, 3.0), a, b) - 3.0).abs() < EPSILON);
        assert!((distance_to_segment(point(4.0, 0.0), a, b)).abs() < EPSILON);
        // Past an end, the distance is to that end.
        assert!((distance_to_segment(point(13.0, 4.0), a, b) - 5.0).abs() < EPSILON);
        // A zero-length segment is its endpoint.
        assert!((distance_to_segment(point(3.0, 4.0), a, a) - 5.0).abs() < EPSILON);
    }

    #[test]
    fn fib_levels_run_from_the_earlier_end_to_the_plot_edge() {
        let map = mapping(&frame());
        let a = anchor(1100.0, 92.0);
        let b = anchor(1500.0, 108.0);
        let pa = point(map.x(a.time), map.y(a.price));
        let pb = point(map.x(b.time), map.y(b.price));
        assert_eq!(fib_span(pa, pb, map.plot()), (140.0, 640.0));
        assert_eq!(fib_span(pb, pa, map.plot()), (140.0, 640.0));
    }

    #[test]
    fn fib_values_step_through_the_retracements() {
        let values = fib_values(100.0, 200.0);
        let expected = [100.0, 123.6, 138.2, 150.0, 161.8, 178.6, 200.0];
        for (value, expected) in values.iter().zip(expected) {
            assert!((value - expected).abs() < EPSILON, "{value} != {expected}");
        }
        // Pixels work the same way, falling prices too.
        let values = fib_values(300.0, 100.0);
        assert!((values[3] - 200.0).abs() < EPSILON);
        assert!((values[6] - 100.0).abs() < EPSILON);
    }

    #[test]
    fn a_measure_box_spans_its_ends_in_any_order() {
        assert_eq!(
            measure_box(point(140.0, 280.0), point(540.0, 40.0)),
            Rect { x: 140.0, y: 40.0, w: 400.0, h: 240.0 }
        );
        assert_eq!(
            measure_box(point(540.0, 40.0), point(140.0, 280.0)),
            Rect { x: 140.0, y: 40.0, w: 400.0, h: 240.0 }
        );
    }

    #[test]
    fn a_level_is_hit_within_five_pixels() {
        let drawing = level("L", 100.0);
        assert_eq!(
            hit_at(&drawing, point(200.0, 165.0)),
            Some(Hit { id: "L".to_owned(), part: HitPart::Body })
        );
        assert_eq!(hit_at(&drawing, point(200.0, 165.5)), None);
        // Its line crosses the whole plot.
        assert!(hit_at(&drawing, point(0.0, 160.0)).is_some());
    }

    #[test]
    fn a_trend_line_is_hit_on_its_body_and_at_its_ends() {
        let drawing = trend("T", anchor(1100.0, 92.0), anchor(1500.0, 108.0));
        assert_eq!(
            hit_at(&drawing, point(340.0, 160.0)),
            Some(Hit { id: "T".to_owned(), part: HitPart::Body })
        );
        assert_eq!(
            hit_at(&drawing, point(144.0, 278.0)),
            Some(Hit { id: "T".to_owned(), part: HitPart::Start })
        );
        assert_eq!(
            hit_at(&drawing, point(536.0, 42.0)),
            Some(Hit { id: "T".to_owned(), part: HitPart::End })
        );
        assert_eq!(hit_at(&drawing, point(340.0, 170.0)), None);
    }

    #[test]
    fn a_fib_is_hit_on_a_level_line_or_its_swing() {
        let drawing = fib("F", anchor(1100.0, 92.0), anchor(1500.0, 108.0));
        // The 50% level runs from x 140 to the plot's right edge, at y 160.
        assert_eq!(
            hit_at(&drawing, point(600.0, 160.0)),
            Some(Hit { id: "F".to_owned(), part: HitPart::Body })
        );
        assert_eq!(hit_at(&drawing, point(600.0, 166.0)), None);
        // Left of the swing, the level lines do not start.
        assert_eq!(hit_at(&drawing, point(100.0, 160.0)), None);
        // The 23.6% level sits at y 223.36.
        assert_eq!(
            hit_at(&drawing, point(600.0, 223.36)),
            Some(Hit { id: "F".to_owned(), part: HitPart::Body })
        );
    }

    #[test]
    fn a_measure_is_hit_inside_its_box_with_five_pixels_of_slack() {
        let drawing = measure("M", anchor(1100.0, 92.0), anchor(1500.0, 108.0));
        assert_eq!(
            hit_at(&drawing, point(300.0, 200.0)),
            Some(Hit { id: "M".to_owned(), part: HitPart::Body })
        );
        assert_eq!(
            hit_at(&drawing, point(135.0, 200.0)),
            Some(Hit { id: "M".to_owned(), part: HitPart::Body })
        );
        assert_eq!(hit_at(&drawing, point(130.0, 200.0)), None);
    }

    #[test]
    fn the_latest_drawing_wins_and_ends_beat_bodies() {
        let map = mapping(&frame());
        let earlier = level("L", 100.0);
        let later = trend("T", anchor(1100.0, 92.0), anchor(1500.0, 108.0));
        let p = point(340.0, 160.0);
        assert_eq!(
            hit_test(&[earlier.clone(), later.clone()], &map, p),
            Some(Hit { id: "T".to_owned(), part: HitPart::Body })
        );
        assert_eq!(
            hit_test(&[later, earlier], &map, p),
            Some(Hit { id: "L".to_owned(), part: HitPart::Body })
        );
    }

    #[test]
    fn a_collapsed_trend_is_only_its_end() {
        let drawing = trend("Z", anchor(1300.0, 100.0), anchor(1300.0, 100.0));
        assert_eq!(
            hit_at(&drawing, point(344.0, 164.0)),
            Some(Hit { id: "Z".to_owned(), part: HitPart::Start })
        );
        assert_eq!(hit_at(&drawing, point(345.0, 165.0)), None);
    }
}
