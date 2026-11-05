//! Candle history and live prices of one instrument: one series per interval,
//! the trades and provider pages that fold into them, the latest trades, and
//! the statistics of the stretch on screen.

#![cfg_attr(not(test), allow(dead_code))]
// Removed when a window uses this module.

use std::collections::{BTreeMap, BTreeSet};

use crate::market::{Book, Candle, FeedState, IntervalSpec, LiveEvent, Stats, Trade};

/// Candles asked for per history request; a provider that pages smaller
/// returns fewer.
pub const PAGE: usize = 1000;
/// Bounds memory and per-frame work however far back someone scrolls.
pub const MAX_CANDLES: usize = 6000;
/// Trades held for the side panel.
pub const MAX_TRADES: usize = 60;
/// Prices held while an interval's history loads.
const MAX_PENDING: usize = 5000;

/// A closed candle's close, at the time the line puts it: the candle's middle,
/// where the candle-to-line morph keeps it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinePoint {
    pub time: f64,
    pub value: f64,
}

/// History and live state of one interval.
///
/// Closed candles are ordered by time, each time once; the forming candle is
/// newer than all of them. A live price only rewrites the forming candle, so
/// it costs the same however long the history is. Everything that changes the
/// data bumps `revision`, which painters use to reuse derived work.
pub struct Series {
    interval: IntervalSpec,
    candles: Vec<Candle>,
    live: Option<Candle>,
    /// The line, built from `candles` on first use after a change.
    line: Option<Vec<LinePoint>>,
    ready: bool,
    /// The history request in flight.
    load: Option<u64>,
    /// Prices seen while the history request was in flight.
    pending: Vec<Pending>,
    exhausted: bool,
    /// The older-page request in flight.
    older: Option<u64>,
    revision: u64,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    price: f64,
    time: f64,
}

impl Series {
    pub fn new(interval: IntervalSpec) -> Self {
        Self {
            interval,
            candles: Vec::new(),
            live: None,
            line: None,
            ready: false,
            load: None,
            pending: Vec::new(),
            exhausted: false,
            older: None,
            revision: 0,
        }
    }

    pub fn interval(&self) -> IntervalSpec {
        self.interval
    }

    /// Closed candles, oldest first.
    pub fn candles(&self) -> &[Candle] {
        &self.candles
    }

    /// The forming candle.
    pub fn live(&self) -> Option<Candle> {
        self.live
    }

    /// The history has loaded, even if it had no candles.
    pub fn ready(&self) -> bool {
        self.ready
    }

    pub fn loading(&self) -> bool {
        self.load.is_some()
    }

    /// Nothing older can be paged in.
    pub fn exhausted(&self) -> bool {
        self.exhausted
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// One trade at `time` (epoch seconds). Returns true when it belongs to a
    /// candle only the provider can start.
    pub fn add(&mut self, price: f64, time: f64) -> bool {
        let secs = f64::from(self.interval.secs);
        if self.interval.aligned {
            let bucket = (time / secs).floor() * secs;
            match self.live {
                None => {
                    // A trade can only start a candle after the last closed
                    // one; anything earlier is already part of history.
                    if self.candles.last().is_none_or(|last| bucket > last.time) {
                        self.live = Some(flat(bucket, price));
                        self.touch();
                    }
                }
                Some(live) if bucket > live.time => {
                    self.close(live);
                    self.live = Some(flat(bucket, price));
                    self.touch();
                }
                Some(live) if bucket == live.time => self.update_live(price),
                Some(_) => {}
            }
            false
        } else {
            // Trading sessions decide where these candles start: trades only
            // move the one forming, and one past its end waits for the
            // provider's next.
            let Some(live) = self.live else {
                return false;
            };
            if time < live.time {
                return false;
            }
            if time >= live.time + secs {
                return true;
            }
            self.update_live(price);
            false
        }
    }

    /// The provider's latest candles, oldest first, for an interval it buckets
    /// itself: its versions replace ours, and a newer one starts the next
    /// candle.
    pub fn merge_latest(&mut self, fresh: &[Candle]) {
        if fresh.is_empty() {
            return;
        }
        for candle in unique_sorted(fresh.to_vec()) {
            match self.live {
                None => {
                    if self.candles.last().is_some_and(|last| candle.time <= last.time) {
                        self.insert_closed(candle);
                    } else {
                        self.live = Some(candle);
                    }
                }
                Some(live) if candle.time < live.time => {
                    if let Some(existing) = self.find(candle.time) {
                        self.candles[existing] = candle;
                    } else {
                        self.insert_closed(candle);
                    }
                }
                Some(live) if candle.time == live.time => self.live = Some(candle),
                Some(live) => {
                    self.close(live);
                    self.live = Some(candle);
                }
            }
        }
        self.line = None;
        self.touch();
    }

    /// The newest page from REST. Pages already scrolled in stay only if they
    /// join up with it: after a long gap they would leave a hole that paging
    /// back never reaches. Returns whether older pages were kept.
    pub fn replace_recent(&mut self, fresh: Vec<Candle>) -> bool {
        let mut fresh = unique_sorted(fresh);
        let live = fresh.pop();
        let from = fresh
            .first()
            .map(|candle| candle.time)
            .or(live.map(|candle| candle.time))
            .unwrap_or(f64::INFINITY);
        let kept = self.candles.partition_point(|candle| candle.time < from);
        let gap = kept > 0 && self.candles[kept - 1].time + f64::from(self.interval.secs) < from;
        if gap {
            self.candles.clear();
        } else {
            self.candles.truncate(kept);
        }
        self.candles.extend(fresh);
        self.cap();
        self.live = live;
        self.line = None;
        self.touch();
        kept > 0 && !gap
    }

    /// An older history page, possibly overlapping what is loaded: its candles
    /// find their place by time, and a candle already present keeps its value.
    pub fn prepend(&mut self, page: &[Candle]) {
        let mut page = unique_sorted(page.to_vec());
        // The forming candle is newer than every closed one; a page that
        // reaches into it cannot close anything.
        page.retain(|candle| self.live.is_none_or(|live| candle.time < live.time));
        if page.is_empty() {
            return;
        }
        merge_sorted(&mut self.candles, page);
        self.cap();
        self.line = None;
        self.touch();
    }

    /// The line: each closed candle's close at its center, so the candle/line
    /// morph keeps its density before and after the window opened.
    pub fn line(&mut self) -> &[LinePoint] {
        self.ensure_line();
        self.line.as_deref().unwrap_or(&[])
    }

    fn ensure_line(&mut self) {
        if self.line.is_some() {
            return;
        }
        let half = f64::from(self.interval.secs) / 2.0;
        self.line = Some(
            self.candles
                .iter()
                .map(|candle| LinePoint { time: candle.time + half, value: candle.close })
                .collect(),
        );
    }

    fn update_live(&mut self, price: f64) {
        let Some(live) = self.live else {
            return;
        };
        self.live = Some(Candle {
            high: live.high.max(price),
            low: live.low.min(price),
            close: price,
            ..live
        });
        self.touch();
    }

    fn close(&mut self, candle: Candle) {
        self.candles.push(candle);
        let excess = self.candles.len().saturating_sub(MAX_CANDLES);
        if excess > 0 {
            self.candles.drain(..excess);
            self.exhausted = false;
        }
        self.line = None;
        self.touch();
    }

    /// A candle before the forming one, placed by time.
    fn insert_closed(&mut self, candle: Candle) {
        let at = self.candles.partition_point(|existing| existing.time < candle.time);
        if let Some(existing) = self.candles.get_mut(at)
            && existing.time == candle.time
        {
            *existing = candle;
            return;
        }
        self.candles.insert(at, candle);
        self.cap();
    }

    fn find(&self, time: f64) -> Option<usize> {
        self.candles.binary_search_by(|candle| candle.time.total_cmp(&time)).ok()
    }

    fn cap(&mut self) {
        let excess = self.candles.len().saturating_sub(MAX_CANDLES);
        if excess > 0 {
            self.candles.drain(..excess);
        }
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

/// A history request the view started, to pass back when it finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadToken {
    pub interval: u32,
    pub token: u64,
}

/// The page before the oldest candle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OlderPage {
    pub load: LoadToken,
    /// The oldest candle loaded: the page ends before this time.
    pub before: f64,
    /// Candles to ask for, never more than the series has room for.
    pub limit: usize,
}

/// What the feed asked the view to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// The current interval has no history: request it.
    LoadHistory,
    /// The stream is live again after a gap: call `reset` and load trades.
    Reload,
}

/// What the chart draws for the selected interval.
#[derive(Debug, Clone, Copy)]
pub struct ChartData<'a> {
    pub interval: u32,
    /// Closed candles, oldest first.
    pub candles: &'a [Candle],
    /// The forming candle.
    pub live: Option<Candle>,
    /// Closes of the closed candles; the chart draws on to `price`.
    pub line: &'a [LinePoint],
    /// Latest trade price.
    pub price: Option<f64>,
    /// Nothing older can be paged in.
    pub exhausted: bool,
    /// History has loaded, even if it had no candles.
    pub loaded: bool,
    /// Changes whenever this data does.
    pub revision: u64,
}

/// What the stretch on screen did: the last `span` seconds while following the
/// live edge, otherwise `from` to `to` (epoch seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleStats {
    pub live: bool,
    pub span: f64,
    pub from: f64,
    pub to: f64,
    pub change_pct: f64,
    pub amplitude_pct: f64,
}

/// Statistics over the candles visible between `left` and `right`, the forming
/// one included. `live` says whether the view follows the live edge, whose
/// ends move with the clock and are not reported.
pub fn visible_stats(
    data: &ChartData<'_>,
    left: f64,
    right: f64,
    span: f64,
    live: bool,
) -> Option<VisibleStats> {
    let secs = f64::from(data.interval);
    // A candle overlaps the window when it ends after `left` and starts
    // before `right`. Both ends are sorted, so only the visible run is read.
    let start = data.candles.partition_point(|candle| candle.time + secs <= left);
    let end = data.candles.partition_point(|candle| candle.time < right);
    let mut first = None;
    let mut last = None;
    let mut high = f64::NEG_INFINITY;
    let mut low = f64::INFINITY;
    let mut seen = 0usize;
    if end > start {
        for candle in &data.candles[start..end] {
            if first.is_none() {
                first = Some(*candle);
            }
            last = Some(*candle);
            high = high.max(candle.high);
            low = low.min(candle.low);
            seen += 1;
        }
    }
    if let Some(candle) = data.live
        && candle.time + secs > left
        && candle.time < right
    {
        if first.is_none() {
            first = Some(candle);
        }
        last = Some(candle);
        high = high.max(candle.high);
        low = low.min(candle.low);
        seen += 1;
    }
    if seen < 2 {
        return None;
    }
    let open = first?.open;
    let close = last?.close;
    Some(VisibleStats {
        live,
        span: round_half_up(span),
        from: if live { 0.0 } else { round_half_up(left) },
        to: if live { 0.0 } else { round_half_up(right) },
        change_pct: round2((close - open) / open * 100.0),
        amplitude_pct: round2((high - low) / low * 100.0),
    })
}

/// The feed of one instrument: every interval looked at, the trades, the
/// order book and the day's statistics. The view drives it with events from
/// the stream and pages from the API; it does no I/O of its own.
pub struct Market {
    /// Every interval looked at so far: switching back is instant.
    series: BTreeMap<u32, Series>,
    current: u32,
    price: Option<f64>,
    stats: Option<Stats>,
    books: Vec<Book>,
    feed: FeedState,
    /// Newest first; the trade id orders them even within one millisecond.
    trades: Vec<Trade>,
    /// Intervals whose provider-made candles need fetching again.
    refresh: BTreeSet<u32>,
    /// The stream went offline: catch up on history once it is live again.
    gap: bool,
    next_token: u64,
    /// Changes whenever the window has anything new to show.
    revision: u64,
    /// Changes whenever the selected interval's chart data does.
    chart_revision: u64,
}

impl Market {
    pub fn new(interval: IntervalSpec) -> Self {
        let mut series = BTreeMap::new();
        series.insert(interval.secs, Series::new(interval));
        Self {
            series,
            current: interval.secs,
            price: None,
            stats: None,
            books: Vec::new(),
            feed: FeedState::Connecting,
            trades: Vec::new(),
            refresh: BTreeSet::new(),
            gap: false,
            next_token: 0,
            revision: 0,
            chart_revision: 0,
        }
    }

    /// Changes whenever anything the window shows does.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn interval(&self) -> u32 {
        self.current
    }

    pub fn current(&self) -> &Series {
        self.series.get(&self.current).expect("the selected interval has a series")
    }

    pub fn series(&self, secs: u32) -> Option<&Series> {
        self.series.get(&secs)
    }

    /// Every interval that has been looked at, so a timer can refresh the ones
    /// the provider buckets itself.
    pub fn intervals(&self) -> impl Iterator<Item = IntervalSpec> + '_ {
        self.series.values().map(Series::interval)
    }

    pub fn price(&self) -> Option<f64> {
        self.price
    }

    pub fn stats(&self) -> Option<Stats> {
        self.stats
    }

    pub fn books(&self) -> &[Book] {
        &self.books
    }

    pub fn feed(&self) -> FeedState {
        self.feed
    }

    pub fn trades(&self) -> &[Trade] {
        &self.trades
    }

    /// Switches the interval, keeping its series. Returns true when the
    /// current interval needs its history requested now.
    pub fn select_interval(&mut self, interval: IntervalSpec) -> bool {
        if interval.secs == self.current {
            return false;
        }
        self.current = interval.secs;
        let series = self.series.entry(interval.secs).or_insert_with(|| Series::new(interval));
        let load = !series.ready && !series.loading();
        self.touch_chart();
        load
    }

    /// Marks the current interval's history as loading. A previous request for
    /// it is replaced, and its answer ignored.
    pub fn begin_history(&mut self) -> LoadToken {
        let token = self.next_token();
        let interval = self.current;
        let series = self.series.get_mut(&interval).expect("the selected interval has a series");
        series.load = Some(token);
        series.ready = false;
        series.older = None;
        series.touch();
        self.touch_chart();
        LoadToken { interval, token }
    }

    /// Folds a history page into the series that asked for it. Returns true
    /// when a trade buffered while loading needs the provider's latest candles.
    /// An answer to a replaced request is ignored.
    pub fn finish_history(&mut self, load: LoadToken, candles: Vec<Candle>) -> bool {
        let Some(series) = self.series.get_mut(&load.interval) else {
            return false;
        };
        if series.load != Some(load.token) {
            return false;
        }
        series.load = None;
        let empty = candles.is_empty();
        let kept = series.replace_recent(candles);
        if !kept {
            series.exhausted = empty;
        }
        let pending = std::mem::take(&mut series.pending);
        let mut behind = false;
        for price in pending {
            if series.add(price.price, price.time) {
                behind = true;
            }
        }
        series.ready = true;
        series.touch();
        if behind {
            self.refresh.insert(load.interval);
        }
        if load.interval == self.current {
            self.touch_chart();
        } else {
            self.touch();
        }
        behind
    }

    /// The history request failed; it can be made again.
    pub fn fail_history(&mut self, load: LoadToken) {
        if let Some(series) = self.series.get_mut(&load.interval)
            && series.load == Some(load.token)
        {
            series.load = None;
        }
    }

    /// After a gap or when the stream starts: forget the other intervals and
    /// load the current one's history again.
    pub fn reset(&mut self) -> LoadToken {
        self.gap = false;
        self.price = None;
        let current = self.current;
        self.series.retain(|secs, _| *secs == current);
        self.refresh.clear();
        self.touch_chart();
        self.begin_history()
    }

    /// Asks for the page before the oldest candle, once the view decides the
    /// retry delay has passed. None while a page is already in flight, the
    /// history is exhausted, or the series is full.
    pub fn begin_older(&mut self) -> Option<OlderPage> {
        let interval = self.current;
        let token = self.next_token();
        let series = self.series.get_mut(&interval)?;
        if !series.ready || series.exhausted || series.older.is_some() {
            return None;
        }
        let before = series.candles.first()?.time;
        let room = MAX_CANDLES.saturating_sub(series.candles.len());
        if room == 0 {
            return None;
        }
        series.older = Some(token);
        Some(OlderPage { load: LoadToken { interval, token }, before, limit: PAGE.min(room) })
    }

    /// Prepends the older page, or marks the history exhausted when the
    /// provider had nothing older. An answer to a replaced request is ignored.
    pub fn finish_older(&mut self, load: LoadToken, page: Vec<Candle>) -> bool {
        let Some(series) = self.series.get_mut(&load.interval) else {
            return false;
        };
        if series.older != Some(load.token) {
            return false;
        }
        series.older = None;
        if page.is_empty() {
            series.exhausted = true;
            series.touch();
        } else {
            series.prepend(&page);
        }
        if load.interval == self.current {
            self.touch_chart();
        } else {
            self.touch();
        }
        true
    }

    /// The older-page request failed; the view retries after its backoff.
    pub fn fail_older(&mut self, load: LoadToken) {
        if let Some(series) = self.series.get_mut(&load.interval)
            && series.older == Some(load.token)
        {
            series.older = None;
        }
    }

    /// Intervals whose latest provider-made candles are being asked for. The
    /// view fetches each and passes the answer to `merge_latest`.
    pub fn refresh_needed(&self) -> impl Iterator<Item = u32> + '_ {
        self.refresh.iter().copied()
    }

    /// The provider's latest candles for an interval it buckets itself.
    pub fn merge_latest(&mut self, interval: u32, candles: &[Candle]) {
        self.refresh.remove(&interval);
        if let Some(series) = self.series.get_mut(&interval)
            && series.ready
        {
            series.merge_latest(candles);
        }
        if interval == self.current {
            self.touch_chart();
        } else {
            self.touch();
        }
    }

    /// One stream event: prices move the series, the rest is kept for the
    /// window. Returns what the feed asked the view to do next.
    pub fn apply(&mut self, event: LiveEvent) -> Action {
        match event {
            LiveEvent::State { state } => self.apply_state(state),
            LiveEvent::Stats { stats } => {
                self.stats = Some(stats);
                self.touch();
                Action::None
            }
            LiveEvent::Book { books } => {
                self.books = books;
                self.touch();
                Action::None
            }
            LiveEvent::Trades { trades } => {
                self.apply_trades(&trades);
                Action::None
            }
            LiveEvent::Tick { price, time } => {
                self.apply_price(price, time / 1000.0, false);
                Action::None
            }
        }
    }

    /// One price at `time` (epoch seconds): the latest, and part of every
    /// cached interval's forming candle. A trade outside the regular session
    /// only moves intervals that count it.
    pub fn apply_price(&mut self, price: f64, time: f64, extended: bool) {
        self.price = Some(price);
        let intervals: Vec<u32> = self.series.keys().copied().collect();
        for interval in intervals {
            let series = self.series.get_mut(&interval).expect("the interval exists");
            if extended && series.interval.regular_only {
                continue;
            }
            if series.ready {
                if series.add(price, time) {
                    self.refresh.insert(interval);
                }
            } else if series.load.is_some() {
                series.pending.push(Pending { price, time });
                if series.pending.len() > MAX_PENDING {
                    let keep = MAX_PENDING / 2;
                    let drop = series.pending.len() - keep;
                    series.pending.drain(..drop);
                }
            }
        }
        self.touch_chart();
    }

    /// New trades, oldest first: they move the series and the side panel.
    pub fn apply_trades(&mut self, trades: &[Trade]) {
        for trade in trades {
            self.apply_price(trade.price, trade.time / 1000.0, trade.extended);
        }
        self.push_trades(trades);
    }

    /// Merges trades into the newest-first list, dropping repeated ids and
    /// keeping the newest few.
    pub fn push_trades(&mut self, trades: &[Trade]) {
        if trades.is_empty() {
            return;
        }
        self.trades.extend_from_slice(trades);
        self.trades.sort_unstable_by_key(|trade| std::cmp::Reverse(trade.id));
        self.trades.dedup_by_key(|trade| trade.id);
        self.trades.truncate(MAX_TRADES);
        self.touch();
    }

    /// The selected interval's candles and prices, with the line built from
    /// the closed candles on first use after they changed.
    pub fn chart_data(&mut self) -> ChartData<'_> {
        let interval = self.current;
        let revision = self.chart_revision;
        let price = self.price;
        let series = self.series.get_mut(&interval).expect("the selected interval has a series");
        series.ensure_line();
        ChartData {
            interval,
            candles: &series.candles,
            live: series.live,
            line: series.line.as_deref().unwrap_or(&[]),
            price: price.or(series.live.map(|candle| candle.close)),
            exhausted: series.exhausted || series.candles.len() >= MAX_CANDLES,
            loaded: series.ready,
            revision,
        }
    }

    fn apply_state(&mut self, state: FeedState) -> Action {
        if state == FeedState::Offline {
            self.gap = true;
        }
        self.set_feed(state);
        if state != FeedState::Live {
            return Action::None;
        }
        if self.gap {
            self.gap = false;
            Action::Reload
        } else if !self.current().ready && !self.current().loading() {
            Action::LoadHistory
        } else {
            Action::None
        }
    }

    fn set_feed(&mut self, state: FeedState) {
        if self.feed != state {
            self.feed = state;
            self.touch();
        }
    }

    fn next_token(&mut self) -> u64 {
        self.next_token = self.next_token.wrapping_add(1);
        self.next_token
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn touch_chart(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.chart_revision = self.chart_revision.wrapping_add(1);
    }
}

fn flat(time: f64, price: f64) -> Candle {
    Candle { time, open: price, high: price, low: price, close: price }
}

/// The page's candles in time order, one per time, with non-finite times
/// dropped. The first of equal times keeps its place.
fn unique_sorted(mut candles: Vec<Candle>) -> Vec<Candle> {
    candles.retain(|candle| candle.time.is_finite());
    candles.sort_by(|a, b| a.time.total_cmp(&b.time));
    candles.dedup_by(|a, b| a.time == b.time);
    candles
}

/// Merges a sorted page into a sorted series by time. A candle already in the
/// series keeps its value.
fn merge_sorted(candles: &mut Vec<Candle>, page: Vec<Candle>) {
    let Some(first) = page.first().map(|candle| candle.time) else {
        return;
    };
    let at = candles.partition_point(|candle| candle.time < first);
    let mut merged = Vec::with_capacity(page.len() + candles.len() - at);
    let mut existing = candles[at..].iter().copied().peekable();
    for candle in page {
        while let Some(next) = existing.peek() {
            if next.time < candle.time {
                merged.push(*next);
                existing.next();
            } else {
                break;
            }
        }
        match existing.peek() {
            Some(next) if next.time == candle.time => {
                merged.push(*next);
                existing.next();
            }
            _ => merged.push(candle),
        }
    }
    merged.extend(existing);
    candles.truncate(at);
    candles.extend(merged);
}

/// JS `Math.round` for the positive spans and epoch times the chart uses.
fn round_half_up(value: f64) -> f64 {
    (value + 0.5).floor()
}

fn round2(value: f64) -> f64 {
    round_half_up(value * 100.0) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{ChartMode, Level};

    fn interval(secs: u32) -> IntervalSpec {
        IntervalSpec { secs, mode: ChartMode::Candle, aligned: true, regular_only: false }
    }

    fn daily() -> IntervalSpec {
        IntervalSpec { secs: 86_400, mode: ChartMode::Candle, aligned: false, regular_only: true }
    }

    fn candle(time: f64, open: f64) -> Candle {
        Candle { time, open, high: open + 5.0, low: open - 3.0, close: open + 2.0 }
    }

    fn times(series: &Series) -> Vec<f64> {
        series.candles().iter().map(|candle| candle.time).collect()
    }

    fn same(left: Option<Candle>, right: Option<Candle>) -> bool {
        match (left, right) {
            (Some(a), Some(b)) => {
                a.time == b.time
                    && a.open == b.open
                    && a.high == b.high
                    && a.low == b.low
                    && a.close == b.close
            }
            (None, None) => true,
            _ => false,
        }
    }

    fn trade(id: u64) -> Trade {
        Trade {
            id,
            price: id as f64,
            qty: 1.0,
            time: id as f64 * 1000.0,
            sell: false,
            extended: false,
        }
    }

    // A page of overlapping history keeps one candle per time, and the candle
    // already loaded wins over the page's copy.
    #[test]
    fn overlapping_pages_keep_one_candle_per_time() {
        let mut series = Series::new(interval(60));
        let kept = series.replace_recent(vec![
            candle(600.0, 1.0),
            candle(660.0, 2.0),
            candle(720.0, 3.0),
            candle(780.0, 4.0),
        ]);
        assert!(!kept);
        assert_eq!(times(&series), vec![600.0, 660.0, 720.0]);
        assert_eq!(series.live().unwrap().time, 780.0);

        series.prepend(&[candle(540.0, 9.0), candle(600.0, 9.0), candle(660.0, 9.0)]);
        assert_eq!(times(&series), vec![540.0, 600.0, 660.0, 720.0]);
        assert_eq!(series.candles()[0].open, 9.0);
        assert_eq!(series.candles()[1].open, 1.0);
        assert_eq!(series.candles()[2].open, 2.0);
    }

    // A page can arrive shuffled, repeat a time, or reach into candles already
    // loaded: the series ends sorted, one candle per time, existing wins.
    #[test]
    fn out_of_order_pages_merge_in_time_order() {
        let mut series = Series::new(interval(60));
        series.replace_recent(vec![candle(600.0, 1.0), candle(660.0, 2.0), candle(720.0, 3.0)]);
        series.prepend(&[
            candle(480.0, 9.0),
            candle(540.0, 8.0),
            candle(480.0, 7.0),
            candle(660.0, 0.5),
            candle(780.0, 0.25),
        ]);
        assert_eq!(times(&series), vec![480.0, 540.0, 600.0, 660.0]);
        assert_eq!(series.candles()[0].open, 9.0);
        assert_eq!(series.candles()[3].open, 2.0);
    }

    #[test]
    fn a_trade_moves_the_forming_candle_and_the_boundary_opens_the_next() {
        let mut series = Series::new(interval(60));
        series.replace_recent(vec![
            candle(600.0, 1.0),
            candle(660.0, 2.0),
            candle(720.0, 3.0),
            candle(780.0, 4.0),
        ]);
        let revisions = series.revision();

        // Part of a candle the history already has.
        assert!(!series.add(99.0, 725.0));
        assert_eq!(series.revision(), revisions);

        // 785 is in the bucket that starts at 780: only the forming candle moves.
        let line_len = series.line().len();
        let line_ptr = series.line().as_ptr();
        assert!(!series.add(110.0, 785.0));
        let live = series.live().unwrap();
        assert_eq!(
            (live.time, live.open, live.high, live.low, live.close),
            (780.0, 4.0, 110.0, 1.0, 110.0)
        );
        assert_eq!(series.candles().len(), 3);
        assert!(series.revision() > revisions);
        // The line covers closed candles only, so the tick did not rebuild it.
        assert_eq!(series.line().as_ptr(), line_ptr);
        assert_eq!(series.line().len(), line_len);

        // 845 starts the next minute: the forming candle closes, a new one opens.
        assert!(!series.add(120.0, 845.0));
        assert_eq!(times(&series), vec![600.0, 660.0, 720.0, 780.0]);
        let closed = *series.candles().last().unwrap();
        assert_eq!((closed.open, closed.high, closed.low, closed.close), (4.0, 110.0, 1.0, 110.0));
        let live = series.live().unwrap();
        assert_eq!(
            (live.time, live.open, live.high, live.low, live.close),
            (840.0, 120.0, 120.0, 120.0, 120.0)
        );
        assert_eq!(series.line().len(), 4);
        assert_eq!(series.line()[3], LinePoint { time: 810.0, value: 110.0 });
    }

    // Trading days start at the provider's own times: a trade inside the
    // forming day moves it, one past its end asks for the provider's page.
    #[test]
    fn provider_bucketed_candles_wait_for_the_next_page() {
        let mut series = Series::new(daily());
        series.replace_recent(vec![candle(0.0, 1.0), candle(86_400.0, 2.0)]);
        assert!(!series.add(3.0, 86_400.0 + 3_600.0));
        let live = series.live().unwrap();
        assert_eq!((live.high, live.low, live.close), (7.0, -1.0, 3.0));
        assert!(!series.add(1.0, 86_399.0));

        let unchanged = series.live();
        let revisions = series.revision();
        assert!(series.add(4.0, 2.0 * 86_400.0));
        assert!(same(series.live(), unchanged));
        assert_eq!(series.revision(), revisions);
    }

    #[test]
    fn empty_pages_clear_or_leave_a_series_alone() {
        let mut series = Series::new(interval(60));
        assert!(!series.replace_recent(Vec::new()));
        assert!(series.candles().is_empty());
        assert!(series.live().is_none());
        assert!(series.line().is_empty());

        series.replace_recent(vec![candle(0.0, 1.0), candle(60.0, 2.0)]);
        assert!(!series.replace_recent(Vec::new()));
        assert!(series.candles().is_empty());
        assert!(series.live().is_none());

        series.replace_recent(vec![candle(0.0, 1.0), candle(60.0, 2.0)]);
        let revisions = series.revision();
        series.merge_latest(&[]);
        series.prepend(&[]);
        assert_eq!(series.revision(), revisions);
        assert_eq!(series.candles().len(), 1);

        let mut market = Market::new(interval(60));
        let load = market.begin_history();
        market.finish_history(load, Vec::new());
        assert!(visible_stats(&market.chart_data(), 0.0, 600.0, 600.0, false).is_none());
    }

    #[test]
    fn visible_stats_cover_only_the_window() {
        let page = || {
            let mut page = Vec::new();
            for i in 0..10 {
                let open = 100.0 + i as f64;
                page.push(Candle {
                    time: i as f64 * 60.0,
                    open,
                    high: open + 5.0,
                    low: open - 3.0,
                    close: open + 2.0,
                });
            }
            page.push(Candle { time: 600.0, open: 110.0, high: 118.0, low: 109.0, close: 115.0 });
            page
        };
        let mut market = Market::new(interval(60));
        let load = market.begin_history();
        market.finish_history(load, page());
        let data = market.chart_data();
        assert_eq!(data.interval, 60);
        assert!(data.revision > 0);
        assert_eq!(data.candles.len(), 10);
        assert_eq!(data.live.unwrap().time, 600.0);
        assert_eq!(data.price, Some(115.0));
        assert!(data.loaded);
        assert!(!data.exhausted);
        assert_eq!(data.line.len(), 10);
        assert_eq!(data.line[0], LinePoint { time: 30.0, value: 102.0 });

        let stats = visible_stats(&data, 120.0, 420.0, 300.0, false).unwrap();
        assert_eq!(
            stats,
            VisibleStats {
                live: false,
                span: 300.0,
                from: 120.0,
                to: 420.0,
                change_pct: 5.88,
                amplitude_pct: 12.12,
            }
        );

        let stats = visible_stats(&data, 480.0, 660.0, 180.0, true).unwrap();
        assert_eq!(
            stats,
            VisibleStats {
                live: true,
                span: 180.0,
                from: 0.0,
                to: 0.0,
                change_pct: 6.48,
                amplitude_pct: 12.38,
            }
        );

        assert!(visible_stats(&data, 120.0, 180.0, 60.0, false).is_none());
    }

    #[test]
    fn buffered_prices_join_the_history_and_ask_for_newer_candles() {
        let mut market = Market::new(daily());
        let load = market.begin_history();
        assert!(market.current().loading());
        assert_eq!(load.token, 1);
        market.apply_price(105.0, 86_400.0 + 100.0, false);
        market.apply_price(106.0, 2.0 * 86_400.0, false);
        assert!(market.finish_history(load, vec![candle(0.0, 100.0), candle(86_400.0, 101.0)]));
        assert!(market.current().ready());
        assert!(!market.current().loading());
        assert_eq!(market.refresh_needed().collect::<Vec<_>>(), vec![86_400]);
        let live = market.current().live().unwrap();
        assert_eq!((live.time, live.high, live.low, live.close), (86_400.0, 106.0, 98.0, 105.0));

        market.merge_latest(86_400, &[candle(2.0 * 86_400.0, 106.0)]);
        assert_eq!(market.refresh_needed().count(), 0);
        assert_eq!(market.current().live().unwrap().time, 2.0 * 86_400.0);
    }

    #[test]
    fn a_replaced_history_request_is_ignored() {
        let mut market = Market::new(interval(60));
        let first = market.begin_history();
        let second = market.begin_history();
        assert!(!market.finish_history(first, vec![candle(0.0, 1.0)]));
        assert!(market.current().loading());
        market.finish_history(second, vec![candle(0.0, 1.0), candle(60.0, 2.0)]);
        assert!(market.current().ready());

        let failed = market.begin_history();
        market.fail_history(failed);
        assert!(!market.current().loading());
        assert!(!market.current().ready());
        let retried = market.begin_history();
        market.finish_history(retried, vec![candle(0.0, 1.0), candle(60.0, 2.0)]);
        assert!(market.current().ready());
    }

    #[test]
    fn older_pages_fill_in_before_the_oldest_candle() {
        let mut market = Market::new(interval(60));
        let load = market.begin_history();
        market
            .finish_history(load, vec![candle(600.0, 1.0), candle(660.0, 2.0), candle(720.0, 3.0)]);
        let page = market.begin_older().unwrap();
        assert_eq!(page.before, 600.0);
        assert_eq!(page.limit, PAGE);
        assert!(market.begin_older().is_none());

        assert!(market.finish_older(page.load, vec![candle(480.0, 9.0), candle(540.0, 8.0)]));
        assert_eq!(times(market.current()), vec![480.0, 540.0, 600.0, 660.0]);

        let failed = market.begin_older().unwrap();
        market.fail_older(failed.load);
        assert!(!market.current().exhausted());

        let end = market.begin_older().unwrap();
        assert!(market.finish_older(end.load, Vec::new()));
        assert!(market.current().exhausted());
        assert!(market.begin_older().is_none());
    }

    #[test]
    fn a_full_series_does_not_page_older() {
        let mut market = Market::new(interval(60));
        let load = market.begin_history();
        let page = (0..MAX_CANDLES + 5).map(|i| candle(i as f64 * 60.0, i as f64)).collect();
        market.finish_history(load, page);
        assert_eq!(market.current().candles().len(), MAX_CANDLES);
        assert!(market.begin_older().is_none());
        assert!(market.chart_data().exhausted);
    }

    #[test]
    fn pending_prices_stay_bounded() {
        let mut market = Market::new(interval(60));
        market.begin_history();
        for i in 0..=MAX_PENDING {
            market.apply_price(i as f64, i as f64, false);
        }
        assert_eq!(market.current().pending.len(), MAX_PENDING / 2);
    }

    #[test]
    fn extended_trades_skip_regular_only_intervals() {
        let mut market = Market::new(IntervalSpec {
            secs: 60,
            mode: ChartMode::Candle,
            aligned: true,
            regular_only: true,
        });
        let load = market.begin_history();
        market.finish_history(load, vec![candle(0.0, 100.0), candle(60.0, 101.0)]);
        let live = market.current().live();
        market.apply_price(200.0, 65.0, true);
        assert!(same(market.current().live(), live));
        assert_eq!(market.price(), Some(200.0));
        market.apply_price(102.0, 65.0, false);
        assert_eq!(market.current().live().unwrap().close, 102.0);

        let mut regular = Market::new(interval(60));
        let load = regular.begin_history();
        regular.finish_history(load, vec![candle(0.0, 100.0), candle(60.0, 101.0)]);
        regular.apply_price(200.0, 65.0, true);
        assert_eq!(regular.current().live().unwrap().close, 200.0);
    }

    #[test]
    fn trades_are_newest_first_and_deduplicated() {
        let mut market = Market::new(interval(60));
        market.push_trades(&[trade(5), trade(3), trade(4)]);
        assert_eq!(market.trades().iter().map(|trade| trade.id).collect::<Vec<_>>(), vec![5, 4, 3]);
        market.push_trades(&[trade(4), trade(6)]);
        assert_eq!(
            market.trades().iter().map(|trade| trade.id).collect::<Vec<_>>(),
            vec![6, 5, 4, 3]
        );

        let many: Vec<Trade> = (0..MAX_TRADES as u64 + 5).map(trade).collect();
        market.push_trades(&many);
        assert_eq!(market.trades().len(), MAX_TRADES);
        assert_eq!(market.trades()[0].id, MAX_TRADES as u64 + 4);
    }

    // The side panels change without invalidating work derived from candles.
    #[test]
    fn chart_revision_ignores_side_panels() {
        let mut market = Market::new(interval(60));
        let load = market.begin_history();
        market.finish_history(load, vec![candle(0.0, 1.0), candle(60.0, 2.0)]);
        let chart = market.chart_data().revision;
        let window = market.revision();
        market.apply(LiveEvent::Book { books: Vec::new() });
        assert_eq!(market.chart_data().revision, chart);
        assert!(market.revision() > window);
        market.apply_price(3.0, 65.0, false);
        assert!(market.chart_data().revision > chart);
    }

    #[test]
    fn applying_trades_moves_the_series_and_the_panel() {
        let mut market = Market::new(interval(60));
        let load = market.begin_history();
        market.finish_history(load, vec![candle(0.0, 100.0), candle(60.0, 101.0)]);
        market.apply_trades(&[Trade {
            id: 1,
            price: 102.0,
            qty: 2.0,
            time: 65_000.0,
            sell: true,
            extended: false,
        }]);
        assert_eq!(market.price(), Some(102.0));
        assert_eq!(market.current().live().unwrap().close, 102.0);
        assert_eq!(market.trades().len(), 1);
    }

    #[test]
    fn the_stream_keeps_the_book_statistics_and_feed_state() {
        let mut market = Market::new(interval(60));
        assert_eq!(market.feed(), FeedState::Connecting);
        assert!(market.stats().is_none());
        assert!(market.books().is_empty());
        assert_eq!(market.price(), None);

        let stats = Stats {
            last: 101.0,
            high: Some(110.0),
            low: Some(90.0),
            volume: Some(5.0),
            turnover: 500.0,
            change: 1.0,
            change_pct: 1.0,
            onchain: None,
        };
        assert_eq!(market.apply(LiveEvent::Stats { stats }), Action::None);
        assert_eq!(market.stats().unwrap().last, 101.0);

        let book = Book { bids: vec![Level { price: 100.0, qty: 1.0 }], asks: Vec::new() };
        assert_eq!(market.apply(LiveEvent::Book { books: vec![book] }), Action::None);
        assert_eq!(market.books().len(), 1);
        assert_eq!(market.books()[0].bids[0].price, 100.0);

        assert_eq!(market.apply(LiveEvent::State { state: FeedState::Live }), Action::LoadHistory);
        assert_eq!(market.feed(), FeedState::Live);
        let load = market.begin_history();
        assert_eq!(market.apply(LiveEvent::State { state: FeedState::Live }), Action::None);
        market.apply(LiveEvent::State { state: FeedState::Offline });
        assert_eq!(market.apply(LiveEvent::State { state: FeedState::Live }), Action::Reload);
        market.fail_history(load);

        let reload = market.reset();
        assert_eq!(reload.interval, 60);
        assert!(market.current().loading());
        assert_eq!(market.price(), None);
    }

    #[test]
    fn switching_intervals_keeps_each_series() {
        let mut market = Market::new(interval(60));
        let load = market.begin_history();
        market.finish_history(load, vec![candle(0.0, 1.0), candle(60.0, 2.0)]);
        let revisions = market.revision();
        assert!(market.select_interval(interval(300)));
        assert_eq!(market.interval(), 300);
        assert!(market.revision() > revisions);
        assert!(!market.current().ready());

        let load = market.begin_history();
        market.finish_history(load, vec![candle(0.0, 10.0), candle(300.0, 20.0)]);
        assert!(!market.select_interval(interval(60)));
        assert_eq!(market.current().live().unwrap().close, 4.0);
        assert_eq!(market.series(60).unwrap().candles().len(), 1);
        assert_eq!(market.series(300).unwrap().candles().len(), 1);
        assert!(market.series(900).is_none());
        assert_eq!(market.intervals().count(), 2);
        assert_eq!(market.series(300).unwrap().interval().secs, 300);
    }
}
