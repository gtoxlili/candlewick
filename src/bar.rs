//! What the system bar shows: the macOS menu bar, the Windows taskbar. The
//! ticker (the pinned entry, or the total holdings), one dropdown row per
//! watchlist entry and a status caption, once an exchange has an API key a
//! holdings row with a submenu that sums the holdings up, and signed in to a
//! Tonghuashun account a row whose submenu is the watchlist there and its
//! groups: all derived from the model here; `platform::bar` draws them the
//! way each system does.
//!
//! Other threads call [`request_render`], which collapses any burst of updates
//! into one pass on the thread that owns the bar.

use std::{collections::HashMap, sync::atomic::Ordering};

use crate::app::App;

use crate::{
    format::{self, Direction},
    i18n::{self, Locale},
    model::{ColorScheme, Instrument, Model, Quote, Session, SyncStatus},
    platform,
    portfolio::Totals,
    update, window,
};

/// Everything the bar shows, taken from the model in one go.
#[derive(Debug, Clone)]
pub struct View {
    pub watchlist: Vec<Instrument>,
    /// The total holdings and what they are made of, above the watchlist;
    /// none without an API key.
    pub holdings: Option<Holdings>,
    pub rows: Vec<Row>,
    /// The user's Tonghuashun watchlist, below the watchlist; none unless
    /// signed in to a Tonghuashun account.
    pub synced: Option<SyncedMenu>,
    pub ticker: Option<Ticker>,
    /// Why a feed is not live; empty while all are.
    pub caption: String,
    pub scheme: ColorScheme,
    /// The version of a downloaded update a restart would run.
    pub update: Option<String>,
    /// What all of it is said in; another one rebuilds the menu.
    pub locale: Locale,
}

/// A dropdown row: `BTC` `/USDT` `84,002.01` `−0.24%`.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    /// Shown dimmed after the name, e.g. `/USDT`.
    pub detail: String,
    /// The main number, in the trend color of its direction when it is a
    /// move itself (a PnL); flat for a price.
    pub value: (String, Direction),
    /// Shown dimmed before the change, e.g. `After-hours`.
    pub session: Option<&'static str>,
    pub change: Option<(String, Direction)>,
}

impl Row {
    fn new(name: impl Into<String>, detail: impl Into<String>, value: String) -> Self {
        Self {
            name: name.into(),
            detail: detail.into(),
            value: (value, Direction::Flat),
            session: None,
            change: None,
        }
    }

    fn with_change(mut self, pct: Option<f64>) -> Self {
        self.change = pct.map(|pct| (format::change_signed(pct), format::direction(pct)));
        self
    }
}

/// The holdings row and its submenu: the total, what the day did, each
/// account when there are several, the largest assets and positions.
#[derive(Debug, Clone, PartialEq)]
pub struct Holdings {
    /// `Total USD  12,345.67  +1.24%`; the amount is `—` until the first
    /// refresh.
    pub total: Row,
    /// `24h P&L  +152.30`, once there is a total.
    pub day: Option<Row>,
    /// One per exchange, when there is more than one.
    pub accounts: Vec<Row>,
    /// The most valuable, then `Others (n)` for the rest.
    pub assets: Vec<Row>,
    /// The largest unrealized PnL first.
    pub positions: Vec<Row>,
    /// When the holdings were read, or why they could not be.
    pub caption: String,
}

/// The submenu of the user's Tonghuashun watchlist: a row per code, a
/// submenu per group of theirs, then a line on when it was read or why it
/// couldn't be.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncedMenu {
    /// The instrument of each row.
    pub ids: Vec<String>,
    pub rows: Vec<Row>,
    pub groups: Vec<SyncedGroupMenu>,
    pub caption: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncedGroupMenu {
    pub name: String,
    pub ids: Vec<String>,
    pub rows: Vec<Row>,
}

/// The instruments of a synced watchlist's rows, then each group's name and
/// instruments: the same shape, the same items, which a built menu can
/// update in place.
pub type SyncedShape = (Vec<String>, Vec<(String, Vec<String>)>);

impl SyncedMenu {
    pub fn shape(&self) -> SyncedShape {
        let groups = self.groups.iter().map(|g| (g.name.clone(), g.ids.clone())).collect();
        (self.ids.clone(), groups)
    }
}

/// Rows of the submenu beyond this many assets or positions fold into one.
const MENU_ASSETS: usize = 6;
const MENU_POSITIONS: usize = 4;

/// One item of the holdings submenu, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Slot<'a> {
    /// `View Holdings…`
    Open,
    Separator,
    Header(Section),
    Row(&'a Row),
    /// The dimmed line at the end.
    Caption(&'a str),
}

/// A titled part of the holdings submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Accounts,
    Assets,
    Positions,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::Accounts => t!("common.holdings.accounts"),
            Self::Assets => t!("common.holdings.assets"),
            Self::Positions => t!("common.holdings.positions"),
        }
    }
}

/// How many rows of each kind a submenu holds; the same shape means the
/// same slots, which a built menu can update in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub day: bool,
    pub accounts: usize,
    pub assets: usize,
    pub positions: usize,
}

impl Holdings {
    pub fn shape(&self) -> Shape {
        Shape {
            day: self.day.is_some(),
            accounts: self.accounts.len(),
            assets: self.assets.len(),
            positions: self.positions.len(),
        }
    }

    /// The submenu, item by item: the way in, what the day did, then a
    /// section per kind of row that exists, and when it was read.
    pub fn slots(&self) -> Vec<Slot<'_>> {
        let mut slots = vec![Slot::Open];
        if let Some(day) = &self.day {
            slots.extend([Slot::Separator, Slot::Row(day)]);
        }
        for (section, rows) in [
            (Section::Accounts, &self.accounts),
            (Section::Assets, &self.assets),
            (Section::Positions, &self.positions),
        ] {
            if !rows.is_empty() {
                slots.extend([Slot::Separator, Slot::Header(section)]);
                slots.extend(rows.iter().map(Slot::Row));
            }
        }
        slots.extend([Slot::Separator, Slot::Caption(&self.caption)]);
        slots
    }
}

/// What the bar's text shows: plain text on one line, or, with the change
/// shown, the symbol beside a two-row block (price over change).
#[derive(Debug, Clone, PartialEq)]
pub struct Ticker {
    /// `BTC`, or `ETH/BTC` for a non-USD quote, or `Total`; `None` when
    /// symbols are hidden.
    pub symbol: Option<String>,
    /// `—` until the first quote arrives.
    pub price: String,
    /// The change; set once there is a quote, if the change is shown.
    pub change: Option<(String, Direction)>,
    pub two_rows: bool,
    /// Prices may be old: the feed is reconnecting, or an account could
    /// not be read.
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
    Holdings,
    Settings,
    CheckUpdate,
    /// Restart into the downloaded update.
    Update,
    Quit,
}

pub fn perform(app: &App, action: Action) {
    let result = match action {
        Action::Chart(id) => window::open_chart(app, &id),
        Action::Holdings => window::open_holdings(app),
        Action::Settings => window::open_settings(app),
        Action::CheckUpdate => window::open_settings(app).map(|()| update::check_now(app)),
        Action::Update => {
            update::restart_now(app);
            Ok(())
        }
        Action::Quit => {
            app.exit();
            Ok(())
        }
    };
    if let Err(e) = result {
        log::error!("menu action failed: {e}");
    }
}

/// The dropdown is opening: holdings getting old refresh, so the total in it
/// is current a moment later.
pub fn menu_opening(app: &App) {
    app.shared().control.send_if_modified(|control| {
        let holdings = !control.credentials.exchanges.is_empty();
        if holdings {
            control.holdings_wanted = control.holdings_wanted.wrapping_add(1);
        }
        // So are the quotes of the synced watchlist's submenu.
        let synced = !control.ths_synced.is_empty();
        if synced {
            control.menu_opened = control.menu_opened.wrapping_add(1);
        }
        holdings || synced
    });
}

/// Schedules a redraw on the bar's thread unless one is already pending.
pub fn request_render(app: &App) {
    let shared = app.shared();
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
pub fn view(app: &App) -> View {
    let shared = app.shared();
    shared.render_pending.store(false, Ordering::Release);
    let update = update::ready(app);
    let model = shared.model();
    let portfolio = model.has_holdings().then(|| model.portfolio());
    View {
        watchlist: model.settings.watchlist.clone(),
        holdings: portfolio.as_ref().map(holdings),
        rows: rows(&model.settings.watchlist, &model.quotes),
        synced: synced(&model),
        ticker: ticker(&model, portfolio.as_ref()),
        caption: caption(&model),
        scheme: model.settings.color_scheme,
        update,
        locale: i18n::current(),
    }
}

/// What the bar's text shows: the total holdings when the settings ask for
/// them and an account exists, else the pinned entry; `None` for neither.
fn ticker(model: &Model, portfolio: Option<&crate::portfolio::Portfolio>) -> Option<Ticker> {
    let settings = &model.settings;
    if let Some(portfolio) = portfolio.filter(|_| settings.holdings_in_bar) {
        let valuation = &portfolio.valuation;
        let totals =
            portfolio.total.zip(portfolio.change).map(|(total, change)| Totals { total, change });
        return Some(Ticker {
            symbol: settings.show_symbol.then(|| t!("tray.totalShort").to_owned()),
            price: portfolio.total.map_or_else(
                || "—".to_owned(),
                |total| {
                    format!(
                        "{}{}",
                        valuation.currency.symbol(),
                        format::price(
                            total,
                            format::compact_decimals(
                                total,
                                Some(valuation.currency.decimals() as u8),
                            ),
                        )
                    )
                },
            ),
            change: totals
                .filter(|_| settings.show_change)
                .and_then(Totals::change_pct)
                .map(|pct| (format::change_signed(pct), format::direction(pct))),
            two_rows: settings.show_change,
            stale: portfolio.partial
                || portfolio.manual_holdings.iter().any(|holding| holding.stale)
                || model.accounts.values().any(|account| account.error.is_some()),
        });
    }
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

/// A signed monetary amount in the color of its sign: `+152.30`.
fn signed(amount: f64, decimals: usize) -> (String, Direction) {
    let text = format::signed(amount, decimals);
    let direction = if text.starts_with('+') {
        Direction::Up
    } else if text.starts_with('−') {
        Direction::Down
    } else {
        Direction::Flat
    };
    (text, direction)
}

fn holdings(portfolio: &crate::portfolio::Portfolio) -> Holdings {
    let decimals = portfolio.valuation.currency.decimals();
    let totals =
        portfolio.total.zip(portfolio.change).map(|(total, change)| Totals { total, change });
    let total = Row::new(
        t!("common.holdings.total"),
        format!(" {}", portfolio.valuation.currency.code()),
        portfolio.total.map_or_else(|| "—".to_owned(), |total| format::price(total, decimals)),
    )
    .with_change(totals.and_then(Totals::change_pct));
    let day = portfolio.change.map(|change| Row {
        value: signed(change, decimals),
        ..Row::new(t!("common.holdings.dayPnl"), "", String::new())
    });
    let accounts = if portfolio.accounts.len() > 1 {
        portfolio
            .accounts
            .iter()
            .map(|account| {
                let pct = account
                    .total
                    .zip(account.change)
                    .and_then(|(total, change)| crate::portfolio::pct(total, total - change));
                Row::new(
                    account.exchange.name(),
                    "",
                    account.total.map_or_else(|| "—".to_owned(), |t| format::price(t, decimals)),
                )
                .with_change(pct)
            })
            .collect()
    } else {
        Vec::new()
    };
    let priced: Vec<_> = portfolio.assets.iter().filter(|a| a.value.is_some()).collect();
    let mut assets: Vec<Row> = priced
        .iter()
        .take(MENU_ASSETS)
        .map(|asset| {
            Row::new(
                asset.asset.clone(),
                format!("  {}", format::amount(asset.amount)),
                format::price(asset.value.unwrap_or(0.0), decimals),
            )
            .with_change(asset.change_pct.filter(|_| !asset.stable))
        })
        .collect();
    if priced.len() > MENU_ASSETS {
        let rest: f64 = priced[MENU_ASSETS..].iter().filter_map(|a| a.value).sum();
        assets.push(Row::new(
            t!("tray.otherAssets", count = priced.len() - MENU_ASSETS),
            "",
            format::price(rest, decimals),
        ));
    }
    let positions = portfolio
        .positions
        .iter()
        .take(MENU_POSITIONS)
        .map(|position| {
            let p = &position.position;
            let side =
                if p.long { t!("common.holdings.long") } else { t!("common.holdings.short") };
            let leverage =
                p.leverage.map(|l| format!(" {}x", format::amount(l))).unwrap_or_default();
            Row {
                value: position.pnl_value.map(|pnl| signed(pnl, decimals)).unwrap_or_else(|| {
                    let (text, direction) = signed(p.pnl, 6);
                    (format!("{text} {}", p.pnl_asset), direction)
                }),
                ..Row::new(p.symbol.clone(), format!("  {side}{leverage}"), String::new())
            }
        })
        .collect();
    let caption = portfolio
        .accounts
        .iter()
        .find_map(|account| {
            account.error.as_ref().map(|error| {
                t!("tray.readFailedBecause", exchange = account.exchange.name(), error = error)
            })
        })
        .or_else(|| {
            let latest = portfolio
                .accounts
                .iter()
                .filter_map(|a| a.updated)
                .chain(
                    portfolio.manual_holdings.iter().filter_map(|h| h.received.or(h.price_updated)),
                )
                .fold(0.0, f64::max);
            (latest > 0.0).then(|| t!("common.holdings.updated", time = format::clock(latest)))
        })
        .unwrap_or_else(|| t!("common.holdings.reading").to_owned());
    Holdings { total, day, accounts, assets, positions, caption }
}

fn synced(model: &Model) -> Option<SyncedMenu> {
    let synced = model.synced.as_ref()?;
    let caption = match &synced.status {
        SyncStatus::SignedOut => t!("tray.thsSignedOut").to_owned(),
        SyncStatus::Failed(error) => t!("tray.thsReadFailed", error = error),
        SyncStatus::Reading if synced.updated.is_none() => t!("common.holdings.reading").to_owned(),
        SyncStatus::Reading | SyncStatus::Read => {
            let updated = synced.updated.map_or_else(
                || t!("common.holdings.reading").to_owned(),
                |at| t!("common.holdings.updated", time = format::clock(at)),
            );
            match synced.skipped {
                0 => updated,
                skipped => format!("{updated} · {}", t!("tray.thsSkipped", count = skipped)),
            }
        }
    };
    Some(SyncedMenu {
        ids: synced.instruments.iter().map(Instrument::id).collect(),
        rows: rows(&synced.instruments, &model.quotes),
        groups: synced
            .groups
            .iter()
            .map(|group| SyncedGroupMenu {
                name: group.name.clone(),
                ids: group.instruments.iter().map(Instrument::id).collect(),
                rows: rows(&group.instruments, &model.quotes),
            })
            .collect(),
        caption,
    })
}

/// The first feed that isn't live explains itself; empty while all are.
fn caption(model: &Model) -> String {
    model
        .symbols()
        .into_keys()
        .map(|provider| {
            let status = model.feeds.get(&provider).map(|feed| feed.status).unwrap_or_default();
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
                session: quote.and_then(|q| q.session).map(Session::label),
                ..Row::new(
                    name,
                    detail,
                    quote.map_or_else(
                        || "—".to_owned(),
                        |q| format::price(q.last, format::decimals(q.last, instrument.decimals)),
                    ),
                )
            }
            .with_change(quote.and_then(|q| format::change_pct(q.last, q.open)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A PnL reads with its sign and takes its direction's color; what rounds
    // to zero is flat and unsigned.
    #[test]
    fn signed_amounts_carry_their_direction() {
        assert_eq!(signed(152.3, 2), ("+152.30".to_owned(), Direction::Up));
        assert_eq!(signed(-0.5, 2), ("\u{2212}0.50".to_owned(), Direction::Down));
        assert_eq!(signed(-0.004, 2), ("0.00".to_owned(), Direction::Flat));
        assert_eq!(signed(-0.4, 0), ("0".to_owned(), Direction::Flat));
        assert_eq!(signed(1.4, 0), ("+1".to_owned(), Direction::Up));
    }
}
