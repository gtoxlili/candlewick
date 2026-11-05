//! Chart state and the requests that update it.

use std::{
    cell::Cell,
    collections::BTreeSet,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui_kit::{Context, Task};
use tokio::task::JoinHandle;

use super::{
    analysis::Analysis,
    geometry::Frame,
    market::{Action, LoadToken, Market, OlderPage, PAGE, VisibleStats},
    viewport::{Viewport, ViewportEvent},
};
use crate::{
    api,
    app::{App, Topic},
    market::{Candle, ChartMode, ChartSpec, FeedState, IntervalSpec, LiveEvent},
    model::Instrument,
    ui::{
        self,
        prefs::{IndicatorChoice, Mode, SideTab, UiPrefs},
    },
};

struct Abort<T>(JoinHandle<T>);
impl<T> Drop for Abort<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// State shared by all parts of one chart window. Mutation commands notify observers.
pub struct ChartModel {
    app: App,
    instrument_id: Option<String>,
    instrument: Option<Instrument>,
    spec: Option<ChartSpec>,
    market: Option<Market>,
    visible: Option<VisibleStats>,
    last_direction: i8,
    viewport: Viewport,
    analysis: Analysis,
    analysis_revision: Option<(u32, u64)>,
    prefs: UiPrefs,
    /// The canvas writes the geometry it painted; the overlay reads the same slot.
    pub frame: Rc<Cell<Option<Frame>>>,
    generation: u64,
    market_epoch: u64,
    refreshing: BTreeSet<u32>,
    stream: Option<u32>,
    tasks: Vec<Task<()>>,
    observers: Vec<Task<()>>,
    persist_task: Option<Task<()>>,
    prefs_dirty: bool,
    older_backoff: Duration,
    older_retry_at: Option<std::time::Instant>,
}

impl ChartModel {
    pub fn new(app: App, cx: &mut Context<Self>) -> Self {
        let prefs = UiPrefs::load(&crate::app::config_dir().join("ui.json"));
        let mut model = Self::empty(app, prefs);
        model.observers.push(ui::observe(&model.app, Topic::ChartInstrument, cx, |model, cx| {
            model.sync_instrument(cx)
        }));
        model.observers.push(ui::observe(&model.app, Topic::Settings, cx, |model, cx| {
            model.sync_instrument(cx)
        }));
        model.sync_instrument(cx);
        model
    }

    fn empty(app: App, prefs: UiPrefs) -> Self {
        let mut analysis = Analysis::new();
        for choice in &prefs.chart.indicators {
            analysis.set_indicator(&choice.id, Some(choice.on), Some(choice.period));
        }
        let _ = analysis.take_persist();
        Self {
            app,
            instrument_id: None,
            instrument: None,
            spec: None,
            market: None,
            visible: None,
            last_direction: 0,
            viewport: Viewport::new(now(), 60.0, prefs.chart.bars),
            analysis,
            analysis_revision: None,
            prefs,
            frame: Rc::new(Cell::new(None)),
            generation: 0,
            market_epoch: 0,
            refreshing: BTreeSet::new(),
            stream: None,
            tasks: Vec::new(),
            observers: Vec::new(),
            persist_task: None,
            prefs_dirty: false,
            older_backoff: Duration::ZERO,
            older_retry_at: None,
        }
    }

    /// Construct a chart with prepared data and no observers, stream, or network requests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_market(
        app: App,
        instrument: Option<Instrument>,
        spec: ChartSpec,
        market: Market,
    ) -> Self {
        let mut model = Self::empty(app, UiPrefs::default());
        let secs = market.interval();
        model.instrument_id = instrument.as_ref().map(Instrument::id);
        model.instrument = instrument;
        model.spec = Some(spec);
        model.market = Some(market);
        model.viewport.select_interval(f64::from(secs));
        model.update_history_bounds();
        model.sync_analysis_candles();
        model
    }

    pub fn app(&self) -> &App {
        &self.app
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
    /// The page to ask for next, when a test drives the history itself.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn begin_older(&mut self) -> Option<OlderPage> {
        self.begin_older_at(std::time::Instant::now())
    }
    pub fn instrument(&self) -> Option<&Instrument> {
        self.instrument.as_ref()
    }
    pub fn spec(&self) -> Option<&ChartSpec> {
        self.spec.as_ref()
    }
    pub fn market(&self) -> Option<&Market> {
        self.market.as_ref()
    }
    pub fn visible_stats(&self) -> Option<VisibleStats> {
        self.visible
    }
    pub fn last_direction(&self) -> i8 {
        self.last_direction
    }
    /// What the stretch on screen did. The canvas reports it every frame while
    /// the window follows the live edge, so only a difference the line under
    /// the price can show is worth a repaint.
    pub fn set_visible_stats(&mut self, visible: Option<VisibleStats>, cx: &mut Context<Self>) {
        let same = match (self.visible, visible) {
            (Some(before), Some(after)) => {
                before.live == after.live
                    && (before.change_pct - after.change_pct).abs() < 0.005
                    && (before.amplitude_pct - after.amplitude_pct).abs() < 0.005
                    && (before.span - after.span).abs() < 0.001 * after.span
                    && (before.from - after.from).abs() < 0.005 * after.span
                    && (before.to - after.to).abs() < 0.005 * after.span
            }
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.visible = visible;
            cx.notify();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn market_mut(&mut self) -> Option<&mut Market> {
        self.market.as_mut()
    }
    /// Borrow the market, viewport, and analysis together during an entity update.
    pub fn parts_mut(&mut self) -> (Option<&mut Market>, &mut Viewport, &mut Analysis) {
        (self.market.as_mut(), &mut self.viewport, &mut self.analysis)
    }
    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }
    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }
    pub fn analysis(&self) -> &Analysis {
        &self.analysis
    }
    pub fn analysis_mut(&mut self) -> &mut Analysis {
        &mut self.analysis
    }
    pub fn side_tab(&self) -> SideTab {
        self.prefs.chart.tab
    }
    pub fn book_step(&self) -> usize {
        self.prefs.chart.book_step
    }
    pub fn mode(&self) -> Mode {
        let Some(market) = &self.market else {
            return Mode::Candle;
        };
        self.prefs.chart.modes.get(&market.interval()).copied().unwrap_or(
            match market.current().interval().mode {
                ChartMode::Line => Mode::Line,
                ChartMode::Candle => Mode::Candle,
            },
        )
    }

    pub fn select_interval(&mut self, secs: u32, cx: &mut Context<Self>) {
        let Some(interval) =
            self.spec.as_ref().and_then(|s| s.intervals.iter().copied().find(|i| i.secs == secs))
        else {
            return;
        };
        let Some(market) = &mut self.market else {
            return;
        };
        if market.interval() == secs {
            return;
        }
        let load = market.select_interval(interval);
        self.viewport.select_interval(f64::from(secs));
        self.frame.set(None);
        self.analysis.pointer_leave();
        self.visible = None;
        self.prefs.chart.interval = secs;
        self.update_history_bounds();
        self.sync_analysis_candles();
        if load {
            self.load_history(cx);
        }
        self.viewport_changed(cx);
        self.schedule_save(cx);
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if let Some(market) = &self.market {
            self.prefs.chart.modes.insert(market.interval(), mode);
            self.schedule_save(cx);
            cx.notify();
        }
    }
    pub fn set_side_tab(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        self.prefs.chart.tab = tab;
        self.schedule_save(cx);
        cx.notify();
    }
    pub fn set_book_step(&mut self, step: usize, cx: &mut Context<Self>) {
        self.prefs.chart.book_step = step;
        self.schedule_save(cx);
        cx.notify();
    }
    pub fn select_instrument(&self, id: String) {
        let _ = api::open_chart(&self.app, id);
    }
    pub fn open_link(&self) {
        if let Some(id) = &self.instrument_id {
            let _ = api::open_link(&self.app, id.clone());
        }
    }

    /// Call after changing the viewport. Starts paging and saves zoom when requested.
    pub fn viewport_changed(&mut self, cx: &mut Context<Self>) {
        for event in self.viewport.take_events() {
            match event {
                ViewportEvent::Older => self.load_older(cx),
                ViewportEvent::Zoom(bars) => {
                    self.prefs.chart.bars = bars;
                    self.schedule_save(cx);
                }
                ViewportEvent::Live(_) => {}
            }
        }
        cx.notify();
    }
    /// Call after changing analysis state to persist drawings and indicator choices.
    pub fn analysis_changed(&mut self, cx: &mut Context<Self>) {
        if self.analysis.take_persist() {
            if let Some(id) = &self.instrument_id {
                self.prefs.chart.drawings.insert(id.clone(), self.analysis.drawings.clone());
            }
            self.prefs.chart.indicators = self
                .analysis
                .indicators
                .iter()
                .map(|i| IndicatorChoice { id: i.id.clone(), period: i.period, on: i.on })
                .collect();
            self.schedule_save(cx);
        }
        if self.analysis.take_changed() {
            cx.notify();
        }
    }

    fn sync_instrument(&mut self, cx: &mut Context<Self>) {
        let id = api::get_chart_instrument(&self.app);
        let spec = id.as_ref().and_then(|id| api::chart_spec(&self.app, id.clone()).ok());
        // A settings change can add a provider key, which changes what the
        // provider offers this instrument: intervals, a book. Only the same
        // offer keeps the loaded chart.
        if id == self.instrument_id && spec == self.spec {
            self.instrument =
                id.as_ref().and_then(|id| api::chart_instrument(&self.app, id.clone()).ok());
            return;
        }
        self.stop_stream();
        self.tasks.clear();
        self.generation = self.generation.wrapping_add(1);
        self.market_epoch = self.market_epoch.wrapping_add(1);
        self.refreshing.clear();
        self.instrument_id = id.clone();
        self.instrument =
            id.as_ref().and_then(|id| api::chart_instrument(&self.app, id.clone()).ok());
        self.spec = spec;
        self.market = None;
        self.analysis_revision = None;
        self.viewport = Viewport::new(now(), 60.0, self.prefs.chart.bars);
        self.visible = None;
        self.last_direction = 0;
        self.frame.set(None);
        self.older_backoff = Duration::ZERO;
        self.older_retry_at = None;
        self.analysis = Analysis::new();
        for choice in &self.prefs.chart.indicators {
            self.analysis.set_indicator(&choice.id, Some(choice.on), Some(choice.period));
        }
        let _ = self.analysis.take_persist();
        self.analysis.set_instrument(
            id.clone(),
            id.as_ref()
                .and_then(|id| self.prefs.chart.drawings.get(id))
                .cloned()
                .unwrap_or_default(),
        );
        let interval = self
            .spec
            .as_ref()
            .and_then(|spec| pick_interval(&spec.intervals, self.prefs.chart.interval));
        if let Some(interval) = interval {
            self.market = Some(Market::new(interval));
            self.viewport = Viewport::new(now(), f64::from(interval.secs), self.prefs.chart.bars);
            self.load_history(cx);
            self.load_trades(cx);
            self.connect(cx);
            self.start_refresh_timer(cx);
        }
        cx.notify();
    }

    fn stop_stream(&mut self) {
        if let Some(handle) = self.stream.take() {
            api::chart_stream_stop(&self.app, handle);
        }
    }
    fn keep(&mut self, task: Task<()>) {
        self.tasks.retain(|task| !task.is_ready());
        self.tasks.push(task);
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.instrument_id.clone() else {
            return;
        };
        let (send, mut recv) = tokio::sync::mpsc::unbounded_channel();
        match api::chart_stream(&self.app, id, send) {
            Ok(handle) => {
                self.stream = Some(handle);
                let generation = self.generation;
                let task = cx.spawn(async move |weak, cx| {
                    while let Some(event) = recv.recv().await {
                        if weak
                            .update(cx, |model, cx| {
                                if model.generation == generation {
                                    model.apply_event(event, cx);
                                }
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    let _ = weak.update(cx, |model, cx| {
                        if model.generation == generation {
                            model.stream = None;
                            model.apply_event(LiveEvent::State { state: FeedState::Offline }, cx);
                            model.retry_connect(cx);
                        }
                    });
                });
                self.keep(task);
            }
            Err(_) => {
                self.apply_event(LiveEvent::State { state: FeedState::Offline }, cx);
                self.retry_connect(cx);
            }
        }
    }
    fn retry_connect(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let sleep =
            Abort(ui::run(&self.app, async { tokio::time::sleep(Duration::from_secs(5)).await }));
        let task = cx.spawn(async move |weak, cx| {
            let mut sleep = sleep;
            let _ = (&mut sleep.0).await;
            let _ = weak.update(cx, |model, cx| {
                if model.generation == generation {
                    model.connect(cx);
                }
            });
        });
        self.keep(task);
    }
    pub(crate) fn apply_event(&mut self, event: LiveEvent, cx: &mut Context<Self>) {
        let Some(market) = &mut self.market else {
            return;
        };
        let revision = market.revision();
        if let LiveEvent::Stats { stats } = &event
            && let Some(previous) = market.stats()
            && stats.last != previous.last
        {
            self.last_direction = if stats.last > previous.last { 1 } else { -1 };
        }
        let action = market.apply(event);
        let changed = market.revision() != revision;
        match action {
            Action::None => {}
            Action::LoadHistory => self.load_history(cx),
            Action::Reload => {
                self.market_epoch = self.market_epoch.wrapping_add(1);
                self.refreshing.clear();
                let token = self.market.as_mut().unwrap().reset();
                self.request_history(token, None, PAGE, false, cx);
                self.load_trades(cx);
            }
        }
        let refresh: Vec<_> = self.market.as_ref().unwrap().refresh_needed().collect();
        for interval in refresh {
            self.refresh(interval, cx);
        }
        if changed {
            self.sync_analysis_candles();
            cx.notify();
        }
    }
    fn load_history(&mut self, cx: &mut Context<Self>) {
        let Some(market) = &mut self.market else {
            return;
        };
        let token = market.begin_history();
        self.request_history(token, None, PAGE, false, cx);
    }
    fn begin_older_at(&mut self, now: std::time::Instant) -> Option<OlderPage> {
        if self.older_retry_at.is_some_and(|at| now < at) {
            return None;
        }
        self.market.as_mut().and_then(Market::begin_older)
    }
    fn load_older(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.begin_older_at(std::time::Instant::now()) else {
            return;
        };
        self.request_history(page.load, Some(page.before), page.limit, true, cx);
    }
    pub(crate) fn accept_history(
        &mut self,
        generation: u64,
        token: LoadToken,
        candles: Vec<Candle>,
        older: bool,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        let Some(market) = &mut self.market else {
            return false;
        };
        let revision = market.revision();
        if older {
            if market.finish_older(token, candles) {
                self.older_backoff = Duration::ZERO;
                self.older_retry_at = None;
            }
        } else {
            market.finish_history(token, candles);
        }
        let changed = market.revision() != revision;
        if changed {
            self.update_history_bounds();
            self.sync_analysis_candles();
        }
        changed
    }
    fn request_history(
        &mut self,
        token: LoadToken,
        end: Option<f64>,
        limit: usize,
        older: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.instrument_id.clone() else {
            return;
        };
        let generation = self.generation;
        let app = self.app.clone();
        let request = Abort(ui::run(&self.app, async move {
            api::chart_history(&app, id, token.interval, end, limit).await
        }));
        let task = cx.spawn(async move |weak, cx| {
            let mut request = request;
            let result = (&mut request.0).await;
            let _ = weak.update(cx, |model, cx| {
                if model.generation != generation {
                    return;
                }
                match result {
                    Ok(Ok(candles)) => {
                        if model.accept_history(generation, token, candles, older) {
                            model.viewport_changed(cx);
                        }
                    }
                    _ => {
                        let Some(market) = &mut model.market else {
                            return;
                        };
                        if older {
                            market.fail_older(token);
                            model.older_backoff = (model.older_backoff * 2)
                                .max(Duration::from_secs(2))
                                .min(Duration::from_secs(60));
                            model.older_retry_at =
                                Some(std::time::Instant::now() + model.older_backoff);
                        } else {
                            market.fail_history(token);
                            model.retry_history(token.interval, cx);
                        }
                    }
                }
            });
        });
        self.keep(task);
    }
    fn retry_history(&mut self, interval: u32, cx: &mut Context<Self>) {
        let generation = self.generation;
        let sleep =
            Abort(ui::run(&self.app, async { tokio::time::sleep(Duration::from_secs(5)).await }));
        let task = cx.spawn(async move |weak, cx| {
            let mut sleep = sleep;
            let _ = (&mut sleep.0).await;
            let _ = weak.update(cx, |model, cx| {
                if model.generation == generation
                    && model.market.as_ref().is_some_and(|m| {
                        m.interval() == interval && !m.current().ready() && !m.current().loading()
                    })
                {
                    model.load_history(cx);
                }
            });
        });
        self.keep(task);
    }
    fn load_trades(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.instrument_id.clone() else {
            return;
        };
        let generation = self.generation;
        let app = self.app.clone();
        let request =
            Abort(ui::run(&self.app, async move { api::chart_trades(&app, id, 40).await }));
        let task = cx.spawn(async move |weak, cx| {
            let mut request = request;
            if let Ok(Ok(trades)) = (&mut request.0).await {
                let _ = weak.update(cx, |model, cx| {
                    if model.generation == generation
                        && let Some(market) = &mut model.market
                    {
                        market.push_trades(&trades);
                        cx.notify();
                    }
                });
            }
        });
        self.keep(task);
    }
    fn refresh(&mut self, interval: u32, cx: &mut Context<Self>) {
        let Some(id) = self.instrument_id.clone() else {
            return;
        };
        if !self.market.as_ref().and_then(|m| m.series(interval)).is_some_and(|s| s.ready())
            || !self.refreshing.insert(interval)
        {
            return;
        }
        let generation = self.generation;
        let epoch = self.market_epoch;
        let app = self.app.clone();
        let request = Abort(ui::run(&self.app, async move {
            api::chart_history(&app, id, interval, None, 3).await
        }));
        let task = cx.spawn(async move |weak, cx| {
            let mut request = request;
            let result = (&mut request.0).await;
            let _ = weak.update(cx, |model, cx| {
                if model.generation != generation || model.market_epoch != epoch {
                    return;
                }
                model.refreshing.remove(&interval);
                if let Ok(Ok(candles)) = result
                    && let Some(market) = &mut model.market
                {
                    let revision = market.revision();
                    market.merge_latest(interval, &candles);
                    if revision != market.revision() {
                        model.sync_analysis_candles();
                        cx.notify();
                    }
                }
            });
        });
        self.keep(task);
    }
    fn start_refresh_timer(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let app = self.app.clone();
        let task = cx.spawn(async move |weak, cx| {
            loop {
                let mut sleep = Abort(ui::run(&app, async {
                    tokio::time::sleep(Duration::from_secs(30)).await
                }));
                let _ = (&mut sleep.0).await;
                if weak
                    .update(cx, |model, cx| {
                        if model.generation == generation {
                            let intervals: Vec<_> = model
                                .market
                                .as_ref()
                                .map(|m| {
                                    m.intervals().filter(|i| !i.aligned).map(|i| i.secs).collect()
                                })
                                .unwrap_or_default();
                            for interval in intervals {
                                model.refresh(interval, cx);
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.keep(task);
    }
    fn update_history_bounds(&mut self) {
        if let Some(market) = &self.market {
            let s = market.current();
            self.viewport.set_history(
                s.candles().first().map(|c| c.time).or_else(|| s.live().map(|c| c.time)),
                s.exhausted(),
            );
        }
    }
    fn sync_analysis_candles(&mut self) {
        let Some(market) = &self.market else { return };
        let series = market.current();
        let revision = (market.interval(), series.revision());
        if self.analysis_revision == Some(revision) {
            return;
        }
        let mut candles = series.candles().to_vec();
        if let Some(live) = series.live() {
            candles.push(live);
        }
        self.analysis.set_candles(candles);
        self.analysis_revision = Some(revision);
    }
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.prefs_dirty = true;
        let app = self.app.clone();
        self.persist_task = Some(cx.spawn(async move |weak, cx| {
            let mut delay = Abort(ui::run(&app, async {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }));
            if (&mut delay.0).await.is_ok() {
                let _ = weak.update(cx, |model, _| model.save_prefs());
            }
        }));
    }

    fn save_prefs(&mut self) {
        if !self.prefs_dirty {
            return;
        }
        match self.prefs.save(&crate::app::config_dir().join("ui.json")) {
            Ok(()) => self.prefs_dirty = false,
            Err(error) => log::warn!("saving UI preferences: {error}"),
        }
    }
}
impl Drop for ChartModel {
    fn drop(&mut self) {
        self.stop_stream();
        self.save_prefs();
    }
}
fn pick_interval(intervals: &[IntervalSpec], desired: u32) -> Option<IntervalSpec> {
    intervals
        .iter()
        .copied()
        .find(|i| i.secs == desired)
        .or_else(|| intervals.iter().copied().find(|i| i.secs == 60))
        .or_else(|| intervals.first().copied())
}
fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(secs: u32) -> IntervalSpec {
        IntervalSpec { secs, mode: ChartMode::Candle, aligned: true, regular_only: false }
    }

    #[test]
    fn removing_instrument_resets_window_state_without_network() {
        use crate::model::{Settings, Shared};
        use gpui_kit::{AppContext, TestAppContext};
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut cx = TestAppContext::single();
        let (shared, _) = Shared::new(
            Settings::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        let app = cx.update(|cx| App::new(cx, runtime.handle().clone(), shared));
        let instrument = app.shared().model().settings.watchlist[0].clone();
        let spec = api::chart_spec(&app, instrument.id()).unwrap();
        let market = Market::new(spec.intervals[0]);
        let model = cx.new(|_| ChartModel::with_market(app, Some(instrument), spec, market));
        model.update(&mut cx, |model, cx| {
            model.viewport.view.end = Some(1.0);
            model.analysis.tool =
                super::super::analysis::Tool::Drawing(super::super::model::DrawingKind::Level);
            model.last_direction = 1;
            let old_token = model.market.as_mut().unwrap().begin_history();
            let old_interval = model.market.as_ref().unwrap().current().interval();
            let generation = model.generation();
            model.sync_instrument(cx);
            assert!(model.instrument().is_none());
            assert!(model.market().is_none());
            assert!(model.viewport().live());
            assert_eq!(model.analysis().tool, super::super::analysis::Tool::Cursor);
            assert_eq!(model.last_direction(), 0);
            assert_ne!(model.generation, generation);

            let mut next_market = Market::new(old_interval);
            let new_token = next_market.begin_history();
            assert_eq!(old_token, new_token);
            model.market = Some(next_market);
            let candle = |time| Candle { time, open: 1.0, high: 1.0, low: 1.0, close: 1.0 };
            assert!(!model.accept_history(generation, old_token, vec![candle(60.0)], false));
            assert!(!model.market.as_ref().unwrap().current().ready());
            assert!(model.accept_history(
                model.generation(),
                new_token,
                vec![candle(60.0), candle(120.0)],
                false
            ));
            let page = model.begin_older().unwrap();
            assert_eq!(page.before, 60.0);
            assert_eq!(page.limit, PAGE);
            assert!(model.begin_older().is_none());
        });
    }

    #[test]
    fn remembered_interval_falls_back_to_one_minute_then_first() {
        let intervals = [interval(5), interval(60), interval(300)];
        assert_eq!(pick_interval(&intervals, 300).unwrap().secs, 300);
        assert_eq!(pick_interval(&intervals, 900).unwrap().secs, 60);
        assert_eq!(pick_interval(&intervals[..1], 900).unwrap().secs, 5);
        assert!(pick_interval(&[], 60).is_none());
    }
}
