//! The analysis layer's state and gestures: the active tool, the drawings of
//! the instrument on screen (selected, hovered, dragged or half drawn), the
//! indicators switched on and their values. The chart hands it pointer events
//! and the latest frame; painting reads it every frame.

use crate::market::Candle;

use super::{
    geometry::{self, Frame, Hit, HitPart, Point},
    indicators::{self, IndicatorSeries, IndicatorSpec},
    model::{self, Anchor, Drawing, DrawingKind},
};

/// What a click on the chart does: pans and selects, or starts a drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Cursor,
    Drawing(DrawingKind),
}

/// What a press became.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Draw,
    Drag,
    Pan,
}

/// A drawing in progress: the points placed so far.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub kind: DrawingKind,
    pub points: Vec<Anchor>,
}

/// An indicator switched on, with its value at the candle looked at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndicatorValue<'a> {
    pub spec: &'a IndicatorSpec,
    /// None before the warm-up, or past either end of the candles.
    pub value: Option<f64>,
}

#[derive(Clone, Debug)]
struct Drag {
    id: String,
    part: HitPart,
    /// Where the press landed, unsnapped.
    from: Anchor,
    original: Drawing,
}

const UNDO_DEPTH: usize = 20;

#[derive(Debug)]
pub struct Analysis {
    pub tool: Tool,
    pub drawings: Vec<Drawing>,
    pub selected: Option<String>,
    pub hovered: Option<Hit>,
    pub draft: Option<Draft>,
    pub pointer: Option<Point>,
    /// Where the pointer lands in the chart, snapped to the candle under it.
    pub snapped: Option<Anchor>,
    /// Index of the candle under the pointer in `candles()`.
    pub hovered_candle: Option<usize>,
    /// Whether a pointer drag is panning the chart.
    pub panning: bool,
    pub indicators: Vec<IndicatorSpec>,
    pub series: Vec<IndicatorSeries>,
    /// The candles the indicators were computed over, oldest first.
    candles: Vec<Candle>,
    instrument: Option<String>,
    drag: Option<Drag>,
    history: Vec<Vec<Drawing>>,
    changed: bool,
    persist: bool,
}

impl Default for Analysis {
    fn default() -> Self {
        Self::new()
    }
}

impl Analysis {
    pub fn new() -> Self {
        Self {
            tool: Tool::Cursor,
            drawings: Vec::new(),
            selected: None,
            hovered: None,
            draft: None,
            pointer: None,
            snapped: None,
            hovered_candle: None,
            panning: false,
            indicators: indicators::default_indicators(),
            series: Vec::new(),
            candles: Vec::new(),
            instrument: None,
            drag: None,
            history: Vec::new(),
            changed: false,
            persist: false,
        }
    }

    /// The candles the indicators were computed over, oldest first.
    pub fn candles(&self) -> &[Candle] {
        &self.candles
    }

    /// The instrument whose drawings are open.
    #[cfg(test)]
    pub fn instrument(&self) -> Option<&str> {
        self.instrument.as_deref()
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    #[cfg(test)]
    pub fn drafting(&self) -> bool {
        self.draft.is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    /// Whether anything the window shows changed, and the flag is now clear.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Whether the drawings or indicator settings should be written out.
    pub fn take_persist(&mut self) -> bool {
        std::mem::take(&mut self.persist)
    }

    /// Another instrument: its own drawings, nothing selected or half drawn.
    /// The caller has already read `drawings` from the instrument's store.
    pub fn set_instrument(&mut self, instrument: Option<String>, drawings: Vec<Drawing>) {
        self.instrument = instrument;
        self.drawings = drawings;
        self.selected = None;
        self.hovered = None;
        self.draft = None;
        self.drag = None;
        self.history.clear();
        self.changed = true;
    }

    /// Picks a tool; picking the active one again puts the cursor back.
    pub fn set_tool(&mut self, tool: Tool) {
        let next = if tool == self.tool && tool != Tool::Cursor { Tool::Cursor } else { tool };
        self.tool = next;
        self.draft = None;
        if next != Tool::Cursor {
            self.selected = None;
        }
        self.changed = true;
    }

    /// The indicator settings read back from the viewer's store.
    #[cfg(test)]
    pub fn set_indicators(&mut self, indicators: Vec<IndicatorSpec>) {
        self.indicators = indicators;
        self.recompute();
        self.changed = true;
    }

    pub fn set_indicator(&mut self, id: &str, on: Option<bool>, period: Option<usize>) {
        for spec in &mut self.indicators {
            if spec.id != id {
                continue;
            }
            if let Some(on) = on {
                spec.on = on;
            }
            if let Some(period) = period {
                spec.period = IndicatorSpec::clamped_period(period);
            }
        }
        self.persist = true;
        self.recompute();
        self.changed = true;
    }

    /// The candles on screen (closed ones and the forming one), oldest first.
    pub fn set_candles(&mut self, candles: Vec<Candle>) {
        self.candles = candles;
        self.recompute();
    }

    fn recompute(&mut self) {
        self.series = self
            .indicators
            .iter()
            .filter(|spec| spec.on)
            .map(|spec| indicators::compute(spec, &self.candles))
            .collect();
    }

    /// The position of the candle opening at `time`, or `None`.
    pub fn index_of(&self, time: f64) -> Option<usize> {
        let (mut lo, mut hi) = (0i64, self.candles.len() as i64 - 1);
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let at = self.candles[mid as usize].time;
            if at == time {
                return Some(mid as usize);
            }
            if at < time {
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        None
    }

    /// Each indicator switched on, with its value at candle `index` (the latest when None).
    pub fn values_at(&self, index: Option<usize>) -> Vec<IndicatorValue<'_>> {
        self.series
            .iter()
            .map(|series| {
                let value = index
                    .or_else(|| series.values.len().checked_sub(1))
                    .and_then(|i| series.values.get(i).copied())
                    .filter(|value| value.is_finite());
                IndicatorValue { spec: &series.spec, value }
            })
            .collect()
    }

    /// The primary button went down at `p` (logical pixels in the chart). With a
    /// drawing tool it places a point; on a drawing it starts a drag; else the
    /// chart may pan. `free` (Alt) turns snapping off.
    pub fn pointer_down(
        &mut self,
        p: Point,
        frame: Option<&Frame>,
        interval: f64,
        free: bool,
    ) -> Press {
        let Some(frame) = frame else { return Press::Pan };
        let map = geometry::mapping(frame);
        if !map.plot().contains(p) {
            return Press::Pan;
        }
        let anchor = geometry::snap(frame, &map, p, interval, free);
        if let Tool::Drawing(kind) = self.tool {
            let mut points = self.draft.take().map(|draft| draft.points).unwrap_or_default();
            points.push(anchor);
            if points.len() >= kind.anchors_needed() {
                let drawing = model::make_drawing(kind, points[0], points.get(1).copied());
                self.drawings.push(drawing);
                self.tool = Tool::Cursor;
                self.persist = true;
            } else {
                self.draft = Some(Draft { kind, points });
            }
            self.changed = true;
            return Press::Draw;
        }
        if let Some(hit) = geometry::hit_test(&self.drawings, &map, p)
            && let Some(original) =
                self.drawings.iter().find(|drawing| drawing.id() == hit.id).cloned()
        {
            let from = Anchor { time: map.time(p.x), price: map.price(p.y) };
            self.selected = Some(hit.id.clone());
            self.drag = Some(Drag { id: hit.id, part: hit.part, from, original });
            self.changed = true;
            return Press::Drag;
        }
        if self.selected.is_some() {
            self.selected = None;
            self.changed = true;
        }
        Press::Pan
    }

    pub fn pointer_move(&mut self, p: Point, frame: Option<&Frame>, interval: f64, free: bool) {
        self.pointer = Some(p);
        let Some(frame) = frame else {
            self.snapped = None;
            self.hovered_candle = None;
            if self.hovered.take().is_some() {
                self.changed = true;
            }
            return;
        };
        let map = geometry::mapping(frame);
        self.hovered_candle = if map.plot().contains(p) {
            frame.hover.and_then(|candle| self.index_of(candle.time))
        } else {
            None
        };
        self.snapped = if map.plot().contains(p) {
            Some(geometry::snap(frame, &map, p, interval, free))
        } else {
            None
        };
        if let Some(drag) = &self.drag {
            let here = Anchor { time: map.time(p.x), price: map.price(p.y) };
            let next = match drag.part {
                HitPart::Body => model::moved(
                    &drag.original,
                    here.time - drag.from.time,
                    here.price - drag.from.price,
                ),
                HitPart::Start => {
                    model::with_anchor(&drag.original, 0, self.snapped.unwrap_or(here))
                }
                HitPart::End => model::with_anchor(&drag.original, 1, self.snapped.unwrap_or(here)),
            };
            let id = drag.id.clone();
            if let Some(drawing) = self.drawings.iter_mut().find(|drawing| drawing.id() == id) {
                *drawing = next;
            }
            return;
        }
        let hit = if self.tool == Tool::Cursor && self.snapped.is_some() {
            geometry::hit_test(&self.drawings, &map, p)
        } else {
            None
        };
        let unchanged = match (&hit, &self.hovered) {
            (Some(hit), Some(hovered)) => hit.id == hovered.id && hit.part == hovered.part,
            (None, None) => true,
            _ => false,
        };
        if !unchanged {
            self.hovered = hit;
            self.changed = true;
        }
    }

    pub fn pointer_up(&mut self) {
        if self.drag.is_none() {
            return;
        }
        self.drag = None;
        self.persist = true;
        self.changed = true;
    }

    pub fn pointer_leave(&mut self) {
        self.pointer = None;
        self.snapped = None;
        self.hovered_candle = None;
        if self.hovered.is_some() {
            self.hovered = None;
            self.changed = true;
        }
    }

    /// Escape: drops a half-drawn shape, else the selection, else the tool.
    /// Returns whether it did anything.
    pub fn cancel(&mut self) -> bool {
        if self.draft.is_some() {
            self.draft = None;
            self.tool = Tool::Cursor;
        } else if self.selected.is_some() {
            self.selected = None;
        } else if self.tool != Tool::Cursor {
            self.tool = Tool::Cursor;
        } else {
            return false;
        }
        self.changed = true;
        true
    }

    pub fn remove_selected(&mut self) {
        let Some(selected) = self.selected.take() else { return };
        self.remember();
        self.drawings.retain(|drawing| drawing.id() != selected);
        self.hovered = None;
        self.persist = true;
        self.changed = true;
    }

    pub fn clear_all(&mut self) {
        if self.drawings.is_empty() {
            return;
        }
        self.remember();
        self.drawings.clear();
        self.selected = None;
        self.hovered = None;
        self.persist = true;
        self.changed = true;
    }

    /// Brings back what the last removal took.
    pub fn undo(&mut self) {
        let Some(previous) = self.history.pop() else { return };
        self.drawings = previous;
        self.persist = true;
        self.changed = true;
    }

    fn remember(&mut self) {
        if self.history.len() >= UNDO_DEPTH {
            self.history.remove(0);
        }
        self.history.push(self.drawings.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            settled: 1.0,
        }
    }

    fn hovering() -> Frame {
        Frame {
            hover: Some(Candle { time: 1300.0, open: 100.0, high: 101.0, low: 99.0, close: 100.5 }),
            ..frame()
        }
    }

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn candles() -> Vec<Candle> {
        let closes =
            [44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.1, 45.42, 45.84, 46.08, 45.89, 46.03];
        closes
            .iter()
            .enumerate()
            .map(|(i, close)| Candle {
                time: 1000.0 + i as f64 * 60.0,
                open: close - 0.2,
                high: close + 0.5,
                low: close - 0.5,
                close: *close,
            })
            .collect()
    }

    fn analysis_with_level() -> Analysis {
        let mut analysis = Analysis::new();
        analysis.set_instrument(Some("btc".to_owned()), Vec::new());
        analysis.set_candles(candles());
        analysis.set_tool(Tool::Drawing(DrawingKind::Level));
        let press = analysis.pointer_down(point(340.0, 160.0), Some(&frame()), 60.0, false);
        assert_eq!(press, Press::Draw);
        assert_eq!(analysis.drawings.len(), 1);
        assert_eq!(analysis.drawings[0].price(), 100.0);
        analysis.take_changed();
        analysis.take_persist();
        analysis
    }

    #[test]
    fn a_new_analysis_has_the_cursor_and_no_drawings() {
        let analysis = Analysis::new();
        assert_eq!(analysis.tool, Tool::Cursor);
        assert!(analysis.drawings.is_empty());
        assert!(!analysis.can_undo());
        assert_eq!(analysis.indicators.len(), 5);
        assert!(analysis.series.is_empty());
        assert!(!analysis.dragging());
        assert!(!analysis.drafting());
    }

    #[test]
    fn another_instrument_starts_clean() {
        let mut analysis = analysis_with_level();
        analysis.pointer_down(point(200.0, 160.0), Some(&frame()), 60.0, false);
        analysis.pointer_up();
        analysis.pointer_move(point(200.0, 160.0), Some(&frame()), 60.0, false);
        assert!(analysis.selected.is_some());
        assert!(analysis.hovered.is_some());
        analysis.remove_selected();
        assert!(analysis.can_undo());

        analysis.set_instrument(Some("eth".to_owned()), Vec::new());
        assert_eq!(analysis.instrument(), Some("eth"));
        assert!(analysis.drawings.is_empty());
        assert!(analysis.selected.is_none());
        assert!(analysis.hovered.is_none());
        assert!(!analysis.can_undo());
        assert!(analysis.take_changed());
    }

    #[test]
    fn another_instrument_drops_a_half_drawn_shape() {
        let mut analysis = analysis_with_level();
        analysis.set_tool(Tool::Drawing(DrawingKind::Trend));
        analysis.pointer_down(point(140.0, 280.0), Some(&frame()), 60.0, false);
        assert!(analysis.drafting());

        analysis.set_instrument(Some("eth".to_owned()), Vec::new());
        assert!(analysis.draft.is_none());
        assert!(!analysis.drafting());
        assert_eq!(analysis.tool, Tool::Drawing(DrawingKind::Trend));
    }

    #[test]
    fn one_click_of_the_level_tool_draws_a_level_and_puts_the_cursor_back() {
        let mut analysis = Analysis::new();
        analysis.set_tool(Tool::Drawing(DrawingKind::Level));
        analysis.take_changed();
        let press = analysis.pointer_down(point(340.0, 160.0), Some(&frame()), 60.0, false);
        assert_eq!(press, Press::Draw);
        assert_eq!(analysis.tool, Tool::Cursor);
        assert_eq!(analysis.drawings[0].kind(), DrawingKind::Level);
        assert_eq!(analysis.drawings[0].price(), 100.0);
        assert!(analysis.take_changed());
        assert!(analysis.take_persist());
    }

    #[test]
    fn two_clicks_of_a_line_tool_keep_a_draft_between_them() {
        let mut analysis = Analysis::new();
        analysis.set_tool(Tool::Drawing(DrawingKind::Trend));
        analysis.take_changed();
        let press = analysis.pointer_down(point(140.0, 280.0), Some(&frame()), 60.0, false);
        assert_eq!(press, Press::Draw);
        assert!(analysis.drafting());
        assert_eq!(analysis.tool, Tool::Drawing(DrawingKind::Trend));
        assert!(analysis.take_changed());

        analysis.pointer_move(point(540.0, 40.0), Some(&frame()), 60.0, false);
        assert_eq!(analysis.snapped, Some(Anchor { time: 1500.0, price: 108.0 }));
        let press = analysis.pointer_down(point(540.0, 40.0), Some(&frame()), 60.0, false);
        assert_eq!(press, Press::Draw);
        assert!(!analysis.drafting());
        assert_eq!(analysis.tool, Tool::Cursor);
        let drawing = &analysis.drawings[0];
        assert_eq!(
            model::anchors_of(drawing),
            vec![Anchor { time: 1100.0, price: 92.0 }, Anchor { time: 1500.0, price: 108.0 }]
        );
    }

    #[test]
    fn escape_drops_a_half_drawn_shape() {
        let mut analysis = Analysis::new();
        analysis.set_tool(Tool::Drawing(DrawingKind::Measure));
        assert_eq!(
            analysis.pointer_down(point(140.0, 280.0), Some(&frame()), 60.0, false),
            Press::Draw
        );
        assert!(analysis.cancel());
        assert!(!analysis.drafting());
        assert_eq!(analysis.tool, Tool::Cursor);
        assert!(!analysis.cancel());
    }

    #[test]
    fn escape_clears_a_selection_and_leaves_the_cursor() {
        let mut analysis = analysis_with_level();
        analysis.pointer_down(point(200.0, 160.0), Some(&frame()), 60.0, false);
        analysis.pointer_up();
        assert!(analysis.selected.is_some());
        assert!(analysis.cancel());
        assert!(analysis.selected.is_none());
        assert_eq!(analysis.tool, Tool::Cursor);
        assert!(!analysis.cancel());
    }

    #[test]
    fn escape_puts_a_tool_back_to_the_cursor() {
        let mut analysis = Analysis::new();
        analysis.set_tool(Tool::Drawing(DrawingKind::Level));
        assert!(analysis.cancel());
        assert_eq!(analysis.tool, Tool::Cursor);
    }

    #[test]
    fn picking_the_active_tool_again_returns_to_the_cursor() {
        let mut analysis = Analysis::new();
        analysis.set_tool(Tool::Drawing(DrawingKind::Fib));
        assert_eq!(analysis.tool, Tool::Drawing(DrawingKind::Fib));
        analysis.set_tool(Tool::Drawing(DrawingKind::Fib));
        assert_eq!(analysis.tool, Tool::Cursor);
    }

    #[test]
    fn a_press_on_a_drawing_selects_it_and_drags_its_body() {
        let mut analysis = analysis_with_level();
        let press = analysis.pointer_down(point(200.0, 160.0), Some(&frame()), 60.0, false);
        assert_eq!(press, Press::Drag);
        assert!(analysis.dragging());
        assert_eq!(analysis.selected.as_deref(), Some(analysis.drawings[0].id()));
        assert!(analysis.take_changed());

        // Ten pixels right and ten up: 10 seconds, 0.6667 in price.
        analysis.pointer_move(point(210.0, 150.0), Some(&frame()), 60.0, false);
        assert!(!analysis.take_changed());
        let price = analysis.drawings[0].price();
        assert!((price - 100.666_666_666_666_67).abs() < 1e-9, "{price}");

        analysis.pointer_up();
        assert!(!analysis.dragging());
        assert!(analysis.take_changed());
        assert!(analysis.take_persist());
    }

    #[test]
    fn a_press_away_from_every_drawing_pans_and_clears_the_selection() {
        let mut analysis = analysis_with_level();
        analysis.pointer_down(point(200.0, 160.0), Some(&frame()), 60.0, false);
        analysis.pointer_up();
        assert_eq!(
            analysis.pointer_down(point(340.0, 50.0), Some(&frame()), 60.0, false),
            Press::Pan
        );
        assert!(analysis.selected.is_none());
        assert!(!analysis.dragging());
    }

    #[test]
    fn a_press_outside_the_plot_pans() {
        let mut analysis = Analysis::new();
        assert_eq!(
            analysis.pointer_down(point(10.0, 160.0), Some(&frame()), 60.0, false),
            Press::Pan
        );
        assert_eq!(analysis.pointer_down(point(340.0, 160.0), None, 60.0, false), Press::Pan);
    }

    #[test]
    fn the_pointer_lands_on_the_hovered_candle_and_its_prices() {
        let mut analysis = Analysis::new();
        analysis.set_tool(Tool::Drawing(DrawingKind::Level));
        analysis.pointer_down(point(340.0, 153.0), Some(&hovering()), 60.0, false);
        assert_eq!(analysis.drawings[0].price(), 100.5);

        analysis.set_tool(Tool::Drawing(DrawingKind::Level));
        analysis.pointer_down(point(340.0, 153.0), Some(&hovering()), 60.0, true);
        assert!((analysis.drawings[1].price() - 100.466_666_666_666_67).abs() < 1e-9);
    }

    #[test]
    fn dragging_an_end_snaps_it_to_the_candle_under_the_pointer() {
        let mut analysis = Analysis::new();
        analysis.set_tool(Tool::Drawing(DrawingKind::Trend));
        analysis.pointer_down(point(140.0, 280.0), Some(&frame()), 60.0, false);
        analysis.pointer_down(point(540.0, 40.0), Some(&frame()), 60.0, false);
        analysis.pointer_up();

        let press = analysis.pointer_down(point(144.0, 278.0), Some(&frame()), 60.0, false);
        assert_eq!(press, Press::Drag);
        analysis.pointer_move(point(340.0, 153.0), Some(&hovering()), 60.0, false);
        assert_eq!(
            model::anchors_of(&analysis.drawings[0]),
            vec![Anchor { time: 1330.0, price: 100.5 }, Anchor { time: 1500.0, price: 108.0 }]
        );
        analysis.pointer_up();
    }

    #[test]
    fn moving_with_the_cursor_over_a_drawing_hovers_it() {
        let mut analysis = analysis_with_level();
        analysis.pointer_move(point(340.0, 20.0), Some(&frame()), 60.0, false);
        assert_eq!(analysis.hovered, None);
        analysis.pointer_move(point(200.0, 160.0), Some(&frame()), 60.0, false);
        assert!(analysis.take_changed());
        let hovered = analysis.hovered.clone().unwrap();
        assert_eq!(hovered.id, analysis.drawings[0].id());
        assert_eq!(hovered.part, HitPart::Body);
        // The same spot again is not news.
        analysis.pointer_move(point(200.0, 160.5), Some(&frame()), 60.0, false);
        assert!(!analysis.take_changed());
        analysis.pointer_leave();
        assert!(analysis.take_changed());
        assert!(analysis.hovered.is_none());
        assert!(analysis.pointer.is_none());
    }

    #[test]
    fn hover_index_and_snapped_point_clear_when_pointer_leaves() {
        let mut analysis = Analysis::new();
        analysis.set_candles(candles());
        analysis.pointer_move(point(340., 153.), Some(&hovering()), 60., false);
        assert_eq!(analysis.hovered_candle, Some(5));
        assert_eq!(analysis.snapped.unwrap().price, 100.5);
        analysis.pointer_leave();
        assert_eq!(analysis.hovered_candle, None);
        assert_eq!(analysis.snapped, None);
        assert_eq!(analysis.pointer, None);
    }

    #[test]
    fn removing_and_undoing_keeps_the_previous_drawings() {
        let mut analysis = analysis_with_level();
        analysis.pointer_down(point(200.0, 160.0), Some(&frame()), 60.0, false);
        analysis.pointer_up();
        analysis.take_persist();

        analysis.remove_selected();
        assert!(analysis.drawings.is_empty());
        assert!(analysis.selected.is_none());
        assert!(analysis.can_undo());
        assert!(analysis.take_persist());
        analysis.undo();
        assert_eq!(analysis.drawings.len(), 1);
        assert!(!analysis.can_undo());
        assert_eq!(analysis.drawings[0].price(), 100.0);
    }

    #[test]
    fn clearing_all_can_be_undone_and_an_empty_clear_is_not_history() {
        let mut analysis = analysis_with_level();
        analysis.clear_all();
        assert!(analysis.drawings.is_empty());
        assert!(analysis.can_undo());
        analysis.undo();
        assert_eq!(analysis.drawings.len(), 1);
        assert!(!analysis.can_undo());

        let mut empty = Analysis::new();
        empty.clear_all();
        empty.remove_selected();
        assert!(!empty.can_undo());
    }

    #[test]
    fn the_undo_stack_keeps_the_last_twenty_steps() {
        let mut analysis = Analysis::new();
        for i in 0..25 {
            let id = format!("d{i}");
            analysis.drawings.push(Drawing::Level { id: id.clone(), price: i as f64 });
            analysis.selected = Some(id);
            analysis.remove_selected();
        }
        assert!(analysis.drawings.is_empty());
        assert_eq!(analysis.history.len(), UNDO_DEPTH);
        // The first five removals fell off the far end.
        assert_eq!(
            analysis.history[0],
            vec![Drawing::Level { id: "d5".to_owned(), price: 5.0 }],
            "the oldest snapshot is the fifth drawing"
        );
        for _ in 0..UNDO_DEPTH {
            analysis.undo();
        }
        assert_eq!(analysis.drawings, vec![Drawing::Level { id: "d5".to_owned(), price: 5.0 }]);
        assert!(!analysis.can_undo());
    }

    #[test]
    fn indicators_recompute_over_the_candles_on_hand() {
        let mut analysis = Analysis::new();
        assert!(!analysis.take_changed());
        analysis.set_candles(candles());
        assert!(!analysis.take_changed());
        analysis.set_indicator("ma1", Some(true), Some(7));
        assert!(analysis.take_changed());
        assert!(analysis.take_persist());
        assert_eq!(analysis.series.len(), 1);
        let values = analysis.values_at(None);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].spec.label(), "MA7");
        assert!((values[0].value.unwrap() - 45.598_571_428_571_425).abs() < 1e-12);
        // Before the warm-up, and past the end: no value.
        assert_eq!(analysis.values_at(Some(5))[0].value, None);
        assert_eq!(analysis.values_at(Some(99))[0].value, None);
    }

    #[test]
    fn indicator_periods_are_clamped_and_out_of_range_ones_cleared() {
        let mut analysis = Analysis::new();
        analysis.set_indicator("ma1", Some(true), Some(1));
        assert_eq!(analysis.indicators[0].period, 2);
        analysis.set_indicator("ma1", None, Some(999));
        assert_eq!(analysis.indicators[0].period, 500);
        assert!(analysis.indicators[0].on);
        analysis.set_indicator("ma1", Some(false), None);
        assert!(!analysis.indicators[0].on);
        assert!(analysis.series.is_empty());
    }

    #[test]
    fn loaded_indicators_replace_the_defaults() {
        let mut analysis = Analysis::new();
        let specs = vec![IndicatorSpec {
            id: "boll".to_owned(),
            kind: indicators::IndicatorKind::Bollinger,
            period: 20,
            on: true,
        }];
        analysis.set_indicators(specs);
        analysis.set_candles(candles());
        assert_eq!(analysis.series.len(), 1);
        assert_eq!(analysis.series[0].spec.label(), "BOLL(20,2)");
    }

    #[test]
    fn candles_are_found_by_their_opening_time() {
        let mut analysis = Analysis::new();
        analysis.set_candles(candles());
        assert_eq!(analysis.candles().len(), 12);
        assert_eq!(analysis.index_of(1000.0), Some(0));
        assert_eq!(analysis.index_of(1060.0), Some(1));
        assert_eq!(analysis.index_of(1660.0), Some(11));
        assert_eq!(analysis.index_of(1061.0), None);
        assert_eq!(Analysis::new().index_of(1000.0), None);
    }
}
