//! What the system bar shows: the macOS menu bar, the Windows taskbar. The
//! ticker (the pinned entry, or the total holdings), one dropdown row per
//! watchlist entry and a status caption, and once an exchange has an API key
//! a holdings row with a submenu that sums the holdings up: all derived from
//! the model here; `platform::bar` draws them the way each system does.
//!
//! Other threads call [`request_render`], which collapses any burst of updates
//! into one pass on the thread that owns the bar.

use std::{collections::HashMap, sync::atomic::Ordering};

use tauri::{AppHandle, Manager};

use crate::{
    format::{self, Direction},
    i18n::{self, Locale},
    model::{ColorScheme, Instrument, Model, Quote, Session, Shared},
    platform,
    portfolio::{Portfolio, Totals},
    update, window,
};

/// Everything the bar shows, taken from the model in one go.
#[derive(Debug, Clone)]
pub struct View {
    pub watchlist: Vec<Instrument>,
    /// The total holdings and what they are made of, above the watchlist;
    /// none without an API key.
    pub holdings: Option<Holdings>,
    /// The dropdown shows holdings that must stay out of screen captures
    /// (`Settings::conceal_holdings`).
    pub conceal: bool,
    pub rows: Vec<Row>,
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
    /// `Total USDT  12,345.67  +1.24%`; the amount is `—` until the first
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

pub fn perform(app: &AppHandle, action: Action) {
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
            app.exit(0);
            Ok(())
        }
    };
    if let Err(e) = result {
        log::error!("menu action failed: {e}");
    }
}

/// The dropdown is opening: holdings getting old refresh, so the total in it
/// is current a moment later.
pub fn menu_opening(app: &AppHandle) {
    app.state::<Shared>().control.send_if_modified(|control| {
        let any = !control.credentials.exchanges.is_empty();
        if any {
            control.holdings_wanted = control.holdings_wanted.wrapping_add(1);
        }
        any
    });
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
    let holdings = holdings(&model);
    View {
        watchlist: model.settings.watchlist.clone(),
        conceal: holdings.is_some() && model.settings.conceal_holdings,
        holdings,
        rows: rows(&model.settings.watchlist, &model.quotes),
        ticker: ticker(&model),
        caption: caption(&model),
        scheme: model.settings.color_scheme,
        update,
        locale: i18n::current(),
    }
}

/// What the bar shows for the total while holdings stay out of screenshots.
/// The bar belongs to the system (the menu bar, Explorer's taskbar), so no
/// app can leave it out of a capture; the same dots for any amount say
/// nothing of its size.
const MASK: &str = "••••";

/// What the bar's text shows: the total holdings when the settings ask for
/// them and an account exists, else the pinned entry; `None` for neither.
fn ticker(model: &Model) -> Option<Ticker> {
    let settings = &model.settings;
    if settings.holdings_in_bar && !model.accounts.is_empty() {
        let totals = Totals::of(&model.accounts);
        return Some(Ticker {
            symbol: settings.show_symbol.then(|| t!("common.holdings.total").to_owned()),
            price: totals.map_or_else(
                || "—".to_owned(),
                |t| {
                    if settings.conceal_holdings {
                        MASK.to_owned()
                    } else {
                        format::price(t.total, format::compact_decimals(t.total, Some(2)))
                    }
                },
            ),
            change: totals
                .filter(|_| settings.show_change)
                .and_then(Totals::change_pct)
                .map(|pct| (format::change_signed(pct), format::direction(pct))),
            two_rows: settings.show_change,
            stale: model.accounts.values().any(|account| account.error.is_some()),
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

/// A signed USDT amount in the color of its sign: `+152.30`.
fn signed(amount: f64) -> (String, Direction) {
    let text = format::price(amount.abs(), 2);
    match format::direction(amount) {
        Direction::Up => (format!("+{text}"), Direction::Up),
        Direction::Down => (format!("\u{2212}{text}"), Direction::Down),
        Direction::Flat => (text, Direction::Flat),
    }
}

fn holdings(model: &Model) -> Option<Holdings> {
    if model.accounts.is_empty() {
        return None;
    }
    let totals = Totals::of(&model.accounts);
    let portfolio = Portfolio::of(&model.accounts);
    let total = Row::new(
        t!("common.holdings.total"),
        " USDT",
        totals.map_or_else(|| "—".to_owned(), |t| format::price(t.total, 2)),
    )
    .with_change(totals.and_then(Totals::change_pct));
    let day = totals.map(|t| Row {
        value: signed(t.change),
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
                    account.total.map_or_else(|| "—".to_owned(), |t| format::price(t, 2)),
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
                format::price(asset.value.unwrap_or(0.0), 2),
            )
            .with_change(asset.change_pct.filter(|_| !asset.stable))
        })
        .collect();
    if priced.len() > MENU_ASSETS {
        let rest: f64 = priced[MENU_ASSETS..].iter().filter_map(|a| a.value).sum();
        assets.push(Row::new(
            t!("tray.otherAssets", count = priced.len() - MENU_ASSETS),
            "",
            format::price(rest, 2),
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
            let pnl = position.pnl_usd.unwrap_or(p.pnl);
            Row {
                value: signed(pnl),
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
            let latest = portfolio.accounts.iter().filter_map(|a| a.updated).fold(0.0, f64::max);
            (latest > 0.0).then(|| t!("common.holdings.updated", time = format::clock(latest)))
        })
        .unwrap_or_else(|| t!("common.holdings.reading").to_owned());
    Some(Holdings { total, day, accounts, assets, positions, caption })
}

/// The first feed that isn't live explains itself; empty while all are.
fn caption(model: &Model) -> String {
    model
        .settings
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
        assert_eq!(signed(152.3), ("+152.30".to_owned(), Direction::Up));
        assert_eq!(signed(-0.5), ("\u{2212}0.50".to_owned(), Direction::Down));
        assert_eq!(signed(-0.004), ("0.00".to_owned(), Direction::Flat));
    }

    // requirement: with holdings concealed, a screenshot must not tell how
    // much the user holds, and the bar is in every full-screen screenshot.
    #[test]
    fn a_concealed_total_in_the_bar_says_nothing_of_its_size() {
        use crate::{
            market::{Feeds, ProviderId},
            model::Settings,
            portfolio::{self, Account, Balance, Price, Wallet},
        };
        use std::collections::BTreeMap;

        let bar = |btc: f64, show_change: bool| {
            let prices = [("BTC".to_owned(), Price { last: 84_000.0, open: 80_000.0 })].into();
            let holdings = portfolio::value(
                ProviderId::Binance,
                &[Balance::new(Wallet::Spot, "BTC", btc)],
                &[],
                &prices,
            );
            let account = Account { holdings: Some(holdings), ..Account::new(ProviderId::Binance) };
            let model = Model {
                settings: Settings {
                    holdings_in_bar: true,
                    conceal_holdings: true,
                    show_change,
                    ..Settings::default()
                },
                quotes: HashMap::new(),
                feeds: Feeds::new(),
                chart: None,
                accounts: BTreeMap::from([(ProviderId::Binance, account)]),
            };
            ticker(&model).expect("the total takes the bar")
        };
        for show_change in [false, true] {
            assert_eq!(bar(0.0002, show_change), bar(120.0, show_change));
        }
        let line = bar(120.0, false).line();
        assert!(!line.contains(|c: char| c.is_ascii_digit()), "{line}");
    }
}
