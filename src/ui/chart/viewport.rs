//! Which stretch of time the chart shows. Gestures move targets on one value
//! the chart reads every frame, so panning and zooming need no other redraw;
//! events report what the page itself shows: whether the view follows the live
//! edge, a request to page in older candles, and the zoom to remember.

#![cfg_attr(not(test), allow(dead_code))]
// Removed when a window uses this module.

pub const DEFAULT_BARS: f64 = 60.0;
const MIN_BARS: f64 = 15.0;
const MAX_BARS: f64 = 360.0;
/// How quickly a flung drag slows down (time constant).
const GLIDE_MS: f64 = 325.0;
/// Slower than this (px/ms) a released drag just stops.
const GLIDE_MIN: f64 = 0.05;

pub fn clamp_bars(bars: f64) -> f64 {
    bars.clamp(MIN_BARS, MAX_BARS)
}

/// The targets the chart eases toward, and what its last frame drew: epoch
/// seconds and CSS pixels. The chart writes `left` to `buffer` every frame;
/// it reads `span`, `end` and takes `snap`.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// Visible seconds to ease toward.
    pub span: f64,
    /// Epoch seconds at the right edge to ease toward; `None` follows the live edge.
    pub end: Option<f64>,
    /// Jump to `span`/`end` on the next frame instead of easing.
    pub snap: bool,
    /// The edges drawn, in epoch seconds.
    pub left: Option<f64>,
    pub right: Option<f64>,
    /// The plot area's left x and width, in CSS pixels.
    pub plot_x: Option<f64>,
    pub plot_w: Option<f64>,
    /// The gap after the live edge, as a share of the span.
    pub buffer: f64,
}

/// What a gesture changed that the page itself shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewportEvent {
    /// The view started or stopped following the live edge.
    Live(bool),
    /// The view came within a screen of the oldest loaded candle.
    Older,
    /// The zoom to remember, in bars.
    Zoom(f64),
}

#[derive(Clone, Copy, Debug)]
struct Glide {
    /// Pixels per millisecond at release.
    velocity: f64,
    /// Epoch seconds of the last frame it moved on.
    last: f64,
}

#[derive(Clone, Debug)]
pub struct Viewport {
    pub view: View,
    /// Seconds per candle.
    secs: f64,
    /// Epoch seconds, the chart's clock.
    now: f64,
    /// Epoch seconds of the oldest loaded candle; infinite while unknown.
    oldest: f64,
    exhausted: bool,
    drag_from: Option<f64>,
    glide: Option<Glide>,
    events: Vec<ViewportEvent>,
}

impl Viewport {
    /// `now`: epoch seconds. `secs`: seconds per candle. `bars`: candles on screen.
    pub fn new(now: f64, secs: f64, bars: f64) -> Self {
        Self {
            view: View {
                span: clamp_bars(bars) * secs,
                end: None,
                snap: false,
                left: None,
                right: None,
                plot_x: None,
                plot_w: None,
                buffer: 0.0,
            },
            secs,
            now,
            oldest: f64::INFINITY,
            exhausted: true,
            drag_from: None,
            glide: None,
            events: Vec::new(),
        }
    }

    /// The chart's clock for this frame, epoch seconds.
    pub fn set_now(&mut self, now: f64) {
        self.now = now;
    }

    pub fn live(&self) -> bool {
        self.view.end.is_none()
    }

    /// The one-frame jump flag, read once and cleared: the chart jumps to
    /// the targets instead of easing toward them.
    pub fn take_snap(&mut self) -> bool {
        std::mem::take(&mut self.view.snap)
    }

    pub fn take_events(&mut self) -> Vec<ViewportEvent> {
        std::mem::take(&mut self.events)
    }

    /// New candle size (or pair): the same number of bars, back at the live edge.
    pub fn select_interval(&mut self, secs: f64) {
        let bars = self.view.span / self.secs;
        self.secs = secs;
        self.stop_glide();
        self.view.span = clamp_bars(bars) * secs;
        // Pinned to the live edge until this interval's history says how far back it goes.
        self.oldest = f64::INFINITY;
        self.exhausted = true;
        self.set_end(None, false);
    }

    /// Where the loaded history starts, so panning stops there (or pages in more).
    pub fn set_history(&mut self, oldest: Option<f64>, exhausted: bool) {
        self.oldest = oldest.unwrap_or(f64::INFINITY);
        self.exhausted = exhausted;
        // The oldest candles can go (a reload, the cap): keep the view on what is left.
        if let Some(end) = self.view.end {
            self.move_to(end, false);
        }
        self.want_older();
    }

    pub fn drag_start(&mut self) {
        self.stop_glide();
        self.drag_from = Some(self.view.right.unwrap_or_else(|| self.target_end()));
    }

    /// `dx`: pixels moved since `drag_start`; content follows the pointer.
    pub fn drag_to(&mut self, dx: f64) {
        let Some(from) = self.drag_from else { return };
        self.move_to(from - dx * self.secs_per_px(), true);
    }

    /// `velocity`: pixels per millisecond at release. Returns whether the view
    /// glides on, and needs a frame per step until [`glide_tick`] says stop.
    pub fn drag_end(&mut self, velocity: f64) -> bool {
        self.drag_from = None;
        if velocity.abs() >= GLIDE_MIN {
            self.glide = Some(Glide { velocity, last: self.now });
            return true;
        }
        false
    }

    /// Advances a released fling by the time since the frame before. Call once
    /// per frame after [`set_now`]; true while another frame is needed.
    pub fn glide_tick(&mut self) -> bool {
        let Some(mut glide) = self.glide else { return false };
        let dt_ms = ((self.now - glide.last) * 1000.0).max(0.0);
        glide.last = self.now;
        glide.velocity *= (-dt_ms / GLIDE_MS).exp();
        let before = self.view.end;
        if glide.velocity.abs() >= GLIDE_MIN / 4.0 && before.is_some() {
            let end = self.target_end() - glide.velocity * dt_ms * self.secs_per_px();
            self.move_to(end, true);
            // Stopped by the edge of history or reached the present.
            if self.view.end != before && self.view.end.is_some() {
                self.glide = Some(glide);
                return true;
            }
        }
        self.glide = None;
        false
    }

    /// Horizontal scroll (trackpad swipe); positive moves toward the present.
    pub fn scroll_by(&mut self, dx: f64) {
        self.stop_glide();
        self.move_to(self.target_end() + dx * self.secs_per_px(), true);
    }

    /// `factor` above 1 shows more time. At the live edge the newest candle stays
    /// put; in history the time under `x` (CSS pixels in the chart) does.
    pub fn zoom(&mut self, factor: f64, x: f64) {
        self.stop_glide();
        let span = clamp_bars(self.view.span * factor / self.secs) * self.secs;
        if span == self.view.span {
            return;
        }
        match self.anchor_geometry() {
            Some((left, right, plot_x, plot_w)) if !self.live() => {
                let u = ((x - plot_x) / plot_w).clamp(0.0, 1.0);
                let anchor = left + u * (right - left);
                self.view.span = span;
                self.move_to(anchor + (1.0 - u) * span, false);
            }
            _ => self.view.span = span,
        }
        self.events.push(ViewportEvent::Zoom(span / self.secs));
        self.want_older();
    }

    pub fn go_live(&mut self) {
        self.stop_glide();
        self.set_end(None, false);
    }

    pub fn reset(&mut self) {
        self.view.span = DEFAULT_BARS * self.secs;
        self.events.push(ViewportEvent::Zoom(DEFAULT_BARS));
        self.go_live();
    }

    fn target_end(&self) -> f64 {
        self.view.end.unwrap_or_else(|| self.live_end(self.view.span))
    }

    fn live_end(&self, span: f64) -> f64 {
        self.now + span * self.view.buffer
    }

    fn secs_per_px(&self) -> f64 {
        let fallback = self.view.span / 600.0;
        let (Some(left), Some(right), Some(plot_w)) =
            (self.view.left, self.view.right, self.view.plot_w)
        else {
            return fallback;
        };
        if plot_w == 0.0 || plot_w.is_nan() {
            return fallback;
        }
        (right - left) / plot_w
    }

    /// The edges the last frame drew, when zoom has a point to anchor on.
    fn anchor_geometry(&self) -> Option<(f64, f64, f64, f64)> {
        let left = self.view.left?;
        let right = self.view.right?;
        let plot_x = self.view.plot_x?;
        let plot_w = self.view.plot_w?;
        if plot_w == 0.0 || plot_w.is_nan() {
            return None;
        }
        Some((left, right, plot_x, plot_w))
    }

    /// Right edge, kept between the loaded history and the present.
    fn move_to(&mut self, end: f64, snap: bool) {
        let span = self.view.span;
        let live_end = self.live_end(span);
        if end >= live_end {
            self.set_end(None, snap);
            return;
        }
        // At least a quarter of the view stays on candles; with less history than
        // that there is nowhere to pan, so the view keeps following.
        let clamped = end.max(self.oldest + span / 4.0);
        self.set_end(if clamped >= live_end { None } else { Some(clamped) }, snap);
        self.want_older();
    }

    fn set_end(&mut self, end: Option<f64>, snap: bool) {
        let was_live = self.live();
        self.view.end = end;
        if snap {
            self.view.snap = true;
        }
        if was_live != self.live() {
            self.events.push(ViewportEvent::Live(self.live()));
        }
    }

    fn want_older(&mut self) {
        if self.exhausted || !self.oldest.is_finite() {
            return;
        }
        if self.target_end() - self.view.span < self.oldest + self.view.span {
            self.events.push(ViewportEvent::Older);
        }
    }

    fn stop_glide(&mut self) {
        self.glide = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_700_000_000.0;

    fn viewport() -> Viewport {
        Viewport::new(NOW, 60.0, 60.0)
    }

    /// What the chart reports after drawing `span` seconds ending at `end`.
    fn drawn(vp: &mut Viewport, span: f64, end: f64) {
        vp.view.left = Some(end - span);
        vp.view.right = Some(end);
        vp.view.plot_x = Some(40.0);
        vp.view.plot_w = Some(600.0);
    }

    #[test]
    fn bars_are_clamped_to_what_the_toolbar_offers() {
        assert_eq!(clamp_bars(10.0), 15.0);
        assert_eq!(clamp_bars(60.0), 60.0);
        assert_eq!(clamp_bars(400.0), 360.0);
    }

    #[test]
    fn a_new_viewport_follows_the_live_edge() {
        let mut vp = viewport();
        assert_eq!(vp.view.span, 3600.0);
        assert_eq!(vp.view.end, None);
        assert!(vp.live());
        assert!(!vp.view.snap);
        assert!(vp.take_events().is_empty());
    }

    #[test]
    fn zooming_out_at_the_live_edge_keeps_the_newest_candle_in_place() {
        let mut vp = viewport();
        vp.zoom(2.0, 300.0);
        assert_eq!(vp.view.span, 7200.0);
        assert_eq!(vp.view.end, None);
        assert_eq!(vp.take_events(), vec![ViewportEvent::Zoom(120.0)]);
    }

    #[test]
    fn zooming_in_history_holds_the_time_under_the_cursor() {
        let mut vp = viewport();
        vp.set_history(Some(NOW - 3600.0), false);
        vp.view.end = Some(NOW - 600.0);
        drawn(&mut vp, 3600.0, NOW - 600.0);
        vp.take_events();
        vp.zoom(0.5, 340.0);
        assert_eq!(vp.view.span, 1800.0);
        assert_eq!(vp.view.end, Some(NOW - 1500.0));
        assert!(!vp.live());
        assert_eq!(
            vp.take_events(),
            vec![ViewportEvent::Older, ViewportEvent::Zoom(30.0), ViewportEvent::Older]
        );
    }

    #[test]
    fn a_zoom_that_cannot_change_the_span_does_nothing() {
        let mut vp = viewport();
        vp.view.span = 360.0 * 60.0;
        vp.view.end = Some(NOW - 1800.0);
        drawn(&mut vp, 360.0 * 60.0, NOW - 1800.0);
        vp.zoom(2.0, 340.0);
        assert_eq!(vp.view.span, 360.0 * 60.0);
        assert_eq!(vp.view.end, Some(NOW - 1800.0));
        assert!(vp.take_events().is_empty());
    }

    #[test]
    fn dragging_pans_between_the_history_and_the_present() {
        let mut vp = viewport();
        drawn(&mut vp, 3600.0, NOW);
        vp.set_history(Some(NOW - 3600.0), false);
        assert_eq!(vp.take_events(), vec![ViewportEvent::Older]);

        vp.drag_start();
        vp.drag_to(100.0);
        assert_eq!(vp.view.end, Some(NOW - 600.0));
        assert!(vp.view.snap);
        assert_eq!(vp.take_events(), vec![ViewportEvent::Live(false), ViewportEvent::Older]);

        // A quarter of the view stays on candles: 900 seconds here.
        vp.drag_to(1000.0);
        assert_eq!(vp.view.end, Some(NOW - 2700.0));
        assert_eq!(vp.take_events(), vec![ViewportEvent::Older]);

        vp.drag_to(-100.0);
        assert_eq!(vp.view.end, None);
        assert_eq!(vp.take_events(), vec![ViewportEvent::Live(true)]);
    }

    #[test]
    fn scrolling_toward_the_present_returns_to_the_live_edge() {
        let mut vp = viewport();
        vp.set_history(Some(NOW - 3600.0), true);
        vp.view.end = Some(NOW - 600.0);
        drawn(&mut vp, 3600.0, NOW - 600.0);
        vp.take_events();
        vp.scroll_by(100.0);
        assert!(vp.live());
        assert_eq!(vp.take_events(), vec![ViewportEvent::Live(true)]);
    }

    #[test]
    fn a_fling_glides_and_slows_down() {
        let mut vp = viewport();
        vp.set_history(Some(NOW - 10_000.0), true);
        drawn(&mut vp, 3600.0, NOW - 600.0);
        vp.view.end = Some(NOW - 600.0);
        vp.take_events();

        vp.drag_start();
        vp.drag_to(0.0);
        assert!(vp.drag_end(0.5));
        // Six seconds per pixel: 3600 / 600.
        vp.set_now(NOW + 0.016);
        assert!(vp.glide_tick());
        assert!((vp.view.end.unwrap() - 1_699_999_354.305_851_7).abs() < 1e-3);
        vp.set_now(NOW + 0.032);
        assert!(vp.glide_tick());
        assert!((vp.view.end.unwrap() - 1_699_999_310.806_785_3).abs() < 1e-3);
    }

    #[test]
    fn a_slow_release_does_not_glide() {
        let mut vp = viewport();
        vp.drag_start();
        assert!(!vp.drag_end(GLIDE_MIN / 2.0));
        vp.set_now(NOW + 0.016);
        assert!(!vp.glide_tick());
    }

    #[test]
    fn a_fling_stops_at_the_present() {
        let mut vp = viewport();
        vp.set_history(Some(NOW - 10_000.0), true);
        drawn(&mut vp, 3600.0, NOW - 100.0);
        vp.view.end = Some(NOW - 100.0);
        vp.take_events();
        vp.drag_start();
        vp.drag_to(0.0);
        assert!(vp.drag_end(-5.0));
        vp.set_now(NOW + 0.016);
        assert!(!vp.glide_tick());
        assert!(vp.live());
        assert_eq!(vp.take_events(), vec![ViewportEvent::Live(true)]);
    }

    #[test]
    fn a_reload_that_drops_the_oldest_candles_keeps_the_view_on_what_is_left() {
        let mut vp = viewport();
        vp.set_history(Some(NOW - 3600.0), false);
        vp.view.end = Some(NOW - 1000.0);
        vp.take_events();
        vp.set_history(Some(NOW - 500.0), false);
        assert!(vp.live());
        // Both the clamp and the history update ask for older candles.
        assert_eq!(
            vp.take_events(),
            vec![ViewportEvent::Live(true), ViewportEvent::Older, ViewportEvent::Older]
        );
    }

    #[test]
    fn reset_returns_to_the_default_zoom_at_the_live_edge() {
        let mut vp = viewport();
        vp.set_history(Some(NOW - 3600.0), true);
        vp.view.end = Some(NOW - 600.0);
        vp.zoom(0.5, 340.0);
        vp.take_events();
        vp.reset();
        assert_eq!(vp.view.span, 3600.0);
        assert!(vp.live());
        assert_eq!(
            vp.take_events(),
            vec![ViewportEvent::Zoom(DEFAULT_BARS), ViewportEvent::Live(true)]
        );
    }

    #[test]
    fn another_interval_keeps_the_bar_count_and_waits_for_history() {
        let mut vp = viewport();
        vp.set_history(Some(NOW - 3600.0), false);
        vp.view.end = Some(NOW - 600.0);
        vp.take_events();
        vp.select_interval(300.0);
        assert_eq!(vp.view.span, 60.0 * 300.0);
        assert!(vp.live());
        assert_eq!(vp.take_events(), vec![ViewportEvent::Live(true)]);
        // History is unknown again: panning cannot move yet.
        vp.drag_start();
        vp.drag_to(100.0);
        assert!(vp.live());
    }

    #[test]
    fn the_chart_reads_the_snap_flag_once() {
        let mut vp = viewport();
        vp.drag_start();
        vp.drag_to(100.0);
        assert!(vp.take_snap());
        assert!(!vp.take_snap());
    }
}
