//! What someone draws on the chart, in the chart's own coordinates: epoch
//! seconds and price, so a drawing stays put across intervals, zooms and pans.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

/// A point on the chart.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    /// Epoch seconds, at a candle's center once snapped.
    pub time: f64,
    pub price: f64,
}

/// The kind of a drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingKind {
    /// A horizontal price level.
    Level,
    /// A straight line between two points.
    Trend,
    /// Fibonacci retracement levels between a swing's two ends.
    Fib,
    /// A measured stretch: price and time between two points.
    Measure,
}

impl DrawingKind {
    /// Points a drawing of this kind takes before it is complete.
    pub fn anchors_needed(self) -> usize {
        match self {
            DrawingKind::Level => 1,
            _ => 2,
        }
    }
}

/// One drawing on the chart. The JSON form is `{"kind":"level",…}` and so on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Drawing {
    /// A horizontal price level.
    Level { id: String, price: f64 },
    /// A straight line between two points.
    Trend { id: String, a: Anchor, b: Anchor },
    /// Fibonacci retracement levels between the ends of a swing.
    Fib { id: String, a: Anchor, b: Anchor },
    /// A stretch measured in price and time.
    Measure { id: String, a: Anchor, b: Anchor },
}

impl Drawing {
    pub fn id(&self) -> &str {
        match self {
            Drawing::Level { id, .. }
            | Drawing::Trend { id, .. }
            | Drawing::Fib { id, .. }
            | Drawing::Measure { id, .. } => id,
        }
    }

    pub fn kind(&self) -> DrawingKind {
        match self {
            Drawing::Level { .. } => DrawingKind::Level,
            Drawing::Trend { .. } => DrawingKind::Trend,
            Drawing::Fib { .. } => DrawingKind::Fib,
            Drawing::Measure { .. } => DrawingKind::Measure,
        }
    }

    /// The drawing's level price, or the price of its first end.
    pub fn price(&self) -> f64 {
        match self {
            Drawing::Level { price, .. } => *price,
            Drawing::Trend { a, .. } | Drawing::Fib { a, .. } | Drawing::Measure { a, .. } => {
                a.price
            }
        }
    }
}

/// Retracement levels, as every charting tool lists them.
pub const FIB_LEVELS: [f64; 7] = [0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0];

/// A drawing from the points clicked, in order. A level ignores `b`.
pub fn make_drawing(kind: DrawingKind, a: Anchor, b: Option<Anchor>) -> Drawing {
    let id = next_id();
    let b = b.unwrap_or(a);
    match kind {
        DrawingKind::Level => Drawing::Level { id, price: a.price },
        DrawingKind::Trend => Drawing::Trend { id, a, b },
        DrawingKind::Fib => Drawing::Fib { id, a, b },
        DrawingKind::Measure => Drawing::Measure { id, a, b },
    }
}

/// The movable ends of a drawing; a level has none, it moves as a whole.
pub fn anchors_of(drawing: &Drawing) -> Vec<Anchor> {
    match drawing {
        Drawing::Level { .. } => Vec::new(),
        Drawing::Trend { a, b, .. } | Drawing::Fib { a, b, .. } | Drawing::Measure { a, b, .. } => {
            vec![*a, *b]
        }
    }
}

/// The drawing shifted by `dt` seconds and `dp` in price.
pub fn moved(drawing: &Drawing, dt: f64, dp: f64) -> Drawing {
    let shift = |anchor: Anchor| Anchor { time: anchor.time + dt, price: anchor.price + dp };
    match drawing {
        Drawing::Level { id, price } => Drawing::Level { id: id.clone(), price: price + dp },
        Drawing::Trend { id, a, b } => {
            Drawing::Trend { id: id.clone(), a: shift(*a), b: shift(*b) }
        }
        Drawing::Fib { id, a, b } => Drawing::Fib { id: id.clone(), a: shift(*a), b: shift(*b) },
        Drawing::Measure { id, a, b } => {
            Drawing::Measure { id: id.clone(), a: shift(*a), b: shift(*b) }
        }
    }
}

/// The drawing with end `index` (0 or 1) at `anchor`; a level moves as a whole.
pub fn with_anchor(drawing: &Drawing, index: usize, anchor: Anchor) -> Drawing {
    if let Drawing::Level { id, .. } = drawing {
        return Drawing::Level { id: id.clone(), price: anchor.price };
    }
    let (a, b) = match drawing {
        Drawing::Trend { a, b, .. } | Drawing::Fib { a, b, .. } | Drawing::Measure { a, b, .. } => {
            (*a, *b)
        }
        Drawing::Level { .. } => unreachable!("handled above"),
    };
    let (a, b) = if index == 0 { (anchor, b) } else { (a, anchor) };
    match drawing {
        Drawing::Trend { id, .. } => Drawing::Trend { id: id.clone(), a, b },
        Drawing::Fib { id, .. } => Drawing::Fib { id: id.clone(), a, b },
        Drawing::Measure { id, .. } => Drawing::Measure { id: id.clone(), a, b },
        Drawing::Level { .. } => unreachable!("handled above"),
    }
}

/// The drawings as JSON, the form they are kept in.
pub fn drawings_to_json(drawings: &[Drawing]) -> Result<String, serde_json::Error> {
    serde_json::to_string(drawings)
}

/// The drawings in `raw`, skipping entries that are not drawings. A value
/// that is not a JSON array at all gives no drawings.
pub fn drawings_from_json(raw: &str) -> Vec<Drawing> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(raw) else {
        return Vec::new();
    };
    items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect()
}

/// Ids only need to be unique within a chart's drawings: the milliseconds
/// they were made at, then a counter for two made in the same millisecond.
fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let mut id = base36(millis);
    id.push_str(&base36(count));
    id
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = [0u8; 13];
    let mut at = buf.len();
    loop {
        at -= 1;
        buf[at] = DIGITS[(value % 36) as usize];
        value /= 36;
        if value == 0 {
            break;
        }
    }
    std::str::from_utf8(&buf[at..]).expect("base36 digits are ASCII").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(time: f64, price: f64) -> Anchor {
        Anchor { time, price }
    }

    #[test]
    fn the_retracements_rise_from_the_near_end_to_the_far_one() {
        assert_eq!(FIB_LEVELS.len(), 7);
        assert_eq!(FIB_LEVELS[0], 0.0);
        assert_eq!(FIB_LEVELS[6], 1.0);
        assert!(FIB_LEVELS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_level_needs_one_point_and_the_others_two() {
        assert_eq!(DrawingKind::Level.anchors_needed(), 1);
        assert_eq!(DrawingKind::Trend.anchors_needed(), 2);
        assert_eq!(DrawingKind::Fib.anchors_needed(), 2);
        assert_eq!(DrawingKind::Measure.anchors_needed(), 2);
    }

    #[test]
    fn made_drawings_keep_the_points_clicked() {
        let a = anchor(1000.0, 100.0);
        let b = anchor(1120.0, 110.0);
        let level = make_drawing(DrawingKind::Level, a, Some(b));
        assert_eq!(level.kind(), DrawingKind::Level);
        assert_eq!(level.price(), 100.0);
        assert!(matches!(level, Drawing::Level { ref id, .. } if !id.is_empty()));

        let trend = make_drawing(DrawingKind::Trend, a, Some(b));
        assert_eq!(trend, Drawing::Trend { id: trend.id().to_owned(), a, b });

        // A second point never arrived: the drawing is degenerate, not lost.
        let short = make_drawing(DrawingKind::Fib, a, None);
        assert_eq!(anchors_of(&short), vec![a, a]);

        assert_ne!(level.id(), trend.id());
        assert_eq!(anchors_of(&level), Vec::new());
    }

    #[test]
    fn moving_a_drawing_shifts_both_ends_and_a_level_its_price() {
        let a = anchor(1000.0, 100.0);
        let b = anchor(1120.0, 110.0);
        let trend = Drawing::Trend { id: "t".to_owned(), a, b };
        assert_eq!(
            moved(&trend, 60.0, -2.5),
            Drawing::Trend {
                id: "t".to_owned(),
                a: anchor(1060.0, 97.5),
                b: anchor(1180.0, 107.5),
            }
        );
        let level = Drawing::Level { id: "l".to_owned(), price: 100.0 };
        assert_eq!(moved(&level, 60.0, -2.5), Drawing::Level { id: "l".to_owned(), price: 97.5 });
    }

    #[test]
    fn an_end_moves_without_moving_the_other() {
        let a = anchor(1000.0, 100.0);
        let b = anchor(1120.0, 110.0);
        let measure = Drawing::Measure { id: "m".to_owned(), a, b };
        let moved_a = with_anchor(&measure, 0, anchor(900.0, 90.0));
        assert_eq!(anchors_of(&moved_a), vec![anchor(900.0, 90.0), b]);
        let moved_b = with_anchor(&measure, 1, anchor(1200.0, 120.0));
        assert_eq!(anchors_of(&moved_b), vec![a, anchor(1200.0, 120.0)]);
        // Index 1 and anything else both address the second end.
        assert_eq!(with_anchor(&measure, 2, anchor(1200.0, 120.0)), moved_b);

        let level = Drawing::Level { id: "l".to_owned(), price: 100.0 };
        assert_eq!(
            with_anchor(&level, 0, anchor(1000.0, 105.0)),
            Drawing::Level { id: "l".to_owned(), price: 105.0 }
        );
    }

    #[test]
    fn drawings_round_trip_through_json() {
        let drawings = vec![
            Drawing::Level { id: "a".to_owned(), price: 100.5 },
            Drawing::Trend {
                id: "b".to_owned(),
                a: anchor(1000.0, 100.0),
                b: anchor(1120.0, 110.0),
            },
            Drawing::Fib { id: "c".to_owned(), a: anchor(1000.0, 100.0), b: anchor(1120.0, 110.0) },
            Drawing::Measure {
                id: "d".to_owned(),
                a: anchor(1000.0, 100.0),
                b: anchor(1120.0, 110.0),
            },
        ];
        let json = drawings_to_json(&drawings).unwrap();
        assert_eq!(drawings_from_json(&json), drawings);
        assert!(json.starts_with(r#"[{"kind":"level","id":"a","price":100.5}"#), "{json}");
        assert!(drawings_from_json("not json").is_empty());
        assert!(drawings_from_json(r#"{"kind":"level"}"#).is_empty());
    }

    #[test]
    fn loading_skips_entries_that_are_not_drawings() {
        let raw = r#"[
            {"kind":"level","id":"a","price":100.5},
            {"kind":"level","id":"b","price":"high"},
            {"kind":"trend","id":"c","a":{"time":1,"price":2},"b":{"time":3,"price":4}},
            {"kind":"measure","id":"d","a":{"time":1},"b":{"time":3,"price":4}},
            {"kind":"unknown","id":"e"},
            {"id":"f"}
        ]"#;
        let drawings = drawings_from_json(raw);
        assert_eq!(
            drawings,
            vec![
                Drawing::Level { id: "a".to_owned(), price: 100.5 },
                Drawing::Trend { id: "c".to_owned(), a: anchor(1.0, 2.0), b: anchor(3.0, 4.0) },
            ]
        );
    }
}
