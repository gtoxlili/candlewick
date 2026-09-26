//! What the system bar shows: the macOS menu bar, the Windows taskbar. The
//! pinned entry's ticker, one dropdown row per watchlist entry and a status
//! caption, all derived from the model here; `platform::bar` draws them the
//! way each system does.
//!
//! Other threads call [`request_render`], which collapses any burst of updates
//! into one pass on the thread that owns the bar.

use std::{collections::HashMap, sync::atomic::Ordering};

use tauri::{AppHandle, Manager};

use crate::{
    format::{self, Direction},
    model::{ColorScheme, Instrument, Model, Quote, Session, Shared},
    platform, update, window,
};

/// Everything the bar shows, taken from the model in one go.
#[derive(Debug, Clone)]
pub struct View {
    pub watchlist: Vec<Instrument>,
    pub rows: Vec<Row>,
    pub ticker: Option<Ticker>,
    /// Why a feed is not live; empty while all are.
    pub caption: String,
    pub scheme: ColorScheme,
    /// The version of a downloaded update a restart would run.
    pub update: Option<String>,
}

/// A dropdown row: `BTC` `/USDT` `84,002.01` `−0.24%`.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    /// Shown dimmed after the name, e.g. `/USDT`.
    pub detail: String,
    pub price: String,
    /// Shown dimmed before the change, e.g. `盘后`.
    pub session: Option<&'static str>,
    pub change: Option<(String, Direction)>,
}

/// The pinned entry: plain text on one line, or, with the change shown, the
/// symbol beside a two-row block (price over change).
#[derive(Debug, Clone, PartialEq)]
pub struct Ticker {
    /// `BTC`, or `ETH/BTC` for a non-USD quote; `None` when symbols are hidden.
    pub symbol: Option<String>,
    /// `—` until the first quote arrives.
    pub price: String,
    /// The change; set once there is a quote, if the change is shown.
    pub change: Option<(String, Direction)>,
    pub two_rows: bool,
    /// Prices may be old: the feed is reconnecting.
    pub stale: bool,
}

impl Ticker {
    /// The one-line title, e.g. `BTC 84,050`.
    pub fn line(&self) -> String {
        let mut line = String::new();
        if self.stale {
            line.push_str("⚠︎ ");
        }
        if let Some(symbol) = &self.symbol {
            line.push_str(symbol);
            line.push(' ');
        }
        line.push_str(&self.price);
        line
    }

    /// The whole ticker in words, for screen readers and tooltips.
    pub fn spoken(&self) -> String {
        match &self.change {
            Some((change, _)) => format!("{} {change}", self.line()),
            None => self.line(),
        }
    }
}

/// Which of the two trend colors a move takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hue {
    Green,
    Red,
}

/// The color of a rising or falling number under the user's convention;
/// `None` when it did not move.
pub fn hue(direction: Direction, scheme: ColorScheme) -> Option<Hue> {
    match (direction, scheme) {
        (Direction::Up, ColorScheme::GreenUp) | (Direction::Down, ColorScheme::RedUp) => {
            Some(Hue::Green)
        }
        (Direction::Down, ColorScheme::GreenUp) | (Direction::Up, ColorScheme::RedUp) => {
            Some(Hue::Red)
        }
        (Direction::Flat, _) => None,
    }
}

/// What a click in the dropdown asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The chart of the instrument with this id.
    Chart(String),
    Settings,
    /// Restart into the downloaded update.
    Update,
    Quit,
}

pub fn perform(app: &AppHandle, action: Action) {
    let result = match action {
        Action::Chart(id) => window::open_chart(app, &id),
        Action::Settings => window::open_settings(app),
        Action::Update => {
            update::restart_now(app);
            Ok(())
        }
        Action::Quit => {
            app.exit(0);
            Ok(())
        }
    };
    if let Err(e) = result {
        log::error!("menu action failed: {e}");
    }
}

/// Schedules a redraw on the bar's thread unless one is already pending.
pub fn request_render(app: &AppHandle) {
    let shared = app.state::<Shared>();
    if shared.render_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    if !platform::bar::schedule_render(app) {
        shared.render_pending.store(false, Ordering::Release);
    }
}

/// The current view, for a render that [`request_render`] scheduled (or the
/// first one). Reading it clears the pending flag, so an update arriving from
/// now on schedules another pass.
pub fn view(app: &AppHandle) -> View {
    let shared = app.state::<Shared>();
    shared.render_pending.store(false, Ordering::Release);
    let update = update::ready(app);
    let model = shared.model();
    View {
        watchlist: model.settings.watchlist.clone(),
        rows: rows(&model.settings.watchlist, &model.quotes),
        ticker: ticker(&model),
        caption: caption(&model),
        scheme: model.settings.color_scheme,
        update,
    }
}

/// The pinned entry as the bar shows it; `None` when nothing is pinned.
fn ticker(model: &Model) -> Option<Ticker> {
    let settings = &model.settings;
    let pinned = settings.pinned()?;
    let quote = model.quotes.get(&pinned.id());
    Some(Ticker {
        symbol: settings.show_symbol.then(|| pinned.short_label()),
        price: quote.map_or_else(
            || "—".to_owned(),
            |q| format::price(q.last, format::compact_decimals(q.last, pinned.decimals)),
        ),
        change: quote
            .filter(|_| settings.show_change)
            .and_then(|q| format::change_pct(q.last, q.open))
            .map(|pct| (format::change_signed(pct), format::direction(pct))),
        two_rows: settings.show_change,
        stale: model.stale(pinned.provider),
    })
}

/// The first feed that isn't live explains itself; empty while all are.
fn caption(model: &Model) -> String {
    model
        .settings
        .symbols()
        .into_keys()
        .map(|provider| {
            let status =
                model.feeds.get(&provider).map(|feed| feed.status.clone()).unwrap_or_default();
            status.caption(provider.name())
        })
        .find(|caption| !caption.is_empty())
        .unwrap_or_default()
}

fn rows(watchlist: &[Instrument], quotes: &HashMap<String, Quote>) -> Vec<Row> {
    watchlist
        .iter()
        .map(|instrument| {
            let quote = quotes.get(&instrument.id());
            let (name, detail) = instrument.row_label();
            Row {
                name,
                detail,
                session: quote.and_then(|q| q.session).map(Session::label),
                price: quote.map_or_else(
                    || "—".to_owned(),
                    |q| format::price(q.last, format::decimals(q.last, instrument.decimals)),
                ),
                change: quote
                    .and_then(|q| format::change_pct(q.last, q.open))
                    .map(|pct| (format::change_signed(pct), format::direction(pct))),
            }
        })
        .collect()
}
