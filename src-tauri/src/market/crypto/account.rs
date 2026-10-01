//! An exchange account through the user's API key, only ever read: what the
//! exchanges share in reading it (signing, the server's clock) and the task
//! that keeps its holdings current.
//!
//! Balances and positions refresh every two minutes, every ten seconds while
//! the holdings window is open, and when the dropdown opens on ones older
//! than fifteen seconds. Funding and earn balances change rarely and cost
//! more to ask for, so they refresh at most every five minutes. Prices come
//! from one short look at the exchange's tickers each time; while the
//! holdings window is open, a socket follows the held assets' tickers
//! instead, so the total moves with the market between refreshes.

use std::{
    collections::{BTreeMap, HashMap, btree_map::Entry},
    future::Future,
    sync::atomic::{AtomicI64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, RequestBuilder, StatusCode};
use tauri::{AppHandle, Manager};
use tokio::{
    sync::watch,
    time::{Instant, Interval, MissedTickBehavior, interval_at, sleep, sleep_until},
};
use tokio_tungstenite::tungstenite::Message;

use super::{Exchange, KEEPALIVE, quotes};
use crate::{
    bar,
    credentials::ApiKey,
    http,
    market::{Error, ProviderId},
    model::{FeedControl, Shared},
    net,
    portfolio::{self, Balance, Position, Price, now_ms},
    window,
};

/// What an exchange needs to read an account.
pub trait Account: Exchange {
    /// Checks that `key` works.
    fn check(key: &ApiKey) -> impl Future<Output = Result<(), Error>> + Send;

    /// The balances trading moves (spot, trading and futures wallets) and the
    /// open positions.
    fn trading(
        key: &ApiKey,
    ) -> impl Future<Output = Result<(Vec<Balance>, Vec<Position>), Error>> + Send;

    /// The balances that change rarely: funding and earn.
    fn savings(key: &ApiKey) -> impl Future<Output = Result<Vec<Balance>, Error>> + Send;
}

const BACKGROUND: Duration = Duration::from_secs(120);
const WINDOW_OPEN: Duration = Duration::from_secs(10);
/// Holdings older than this refresh when the dropdown opens.
const FRESH: Duration = Duration::from_secs(15);
const SAVINGS: Duration = Duration::from_secs(300);
/// A pair that didn't answer a price snapshot isn't asked about again for
/// this long: the exchange probably doesn't list it.
const UNLISTED: Duration = Duration::from_secs(6 * 3600);
/// Live ticks reach the window in batches no closer than this.
const LIVE_FLUSH: Duration = Duration::from_secs(1);
/// This long without a frame means the live socket is dead.
const LIVE_SILENCE: Duration = Duration::from_secs(45);

/// This exchange's part of the feed control.
#[derive(Clone, PartialEq)]
struct Wanted {
    key: Option<ApiKey>,
    paused: bool,
    window_open: bool,
}

impl Wanted {
    fn of<E: Account>(control: &FeedControl) -> Self {
        Self {
            key: control.credentials.exchanges.get(&E::ID).cloned(),
            paused: control.paused != 0,
            window_open: control.holdings_open,
        }
    }
}

/// What a refresh keeps for the next one, and for revaluing between them.
#[derive(Default)]
struct Kept {
    savings: Option<(Instant, Vec<Balance>)>,
    unlisted: HashMap<String, Instant>,
    balances: Vec<Balance>,
    positions: Vec<Position>,
    /// By asset, as last seen.
    prices: HashMap<String, Price>,
}

impl Kept {
    /// The assets whose prices the holdings need, and their USDT pairs.
    fn needed<E: Exchange>(&self) -> (Vec<String>, Vec<String>) {
        let mut assets: Vec<&str> = self
            .balances
            .iter()
            .filter(|b| b.amount != 0.0)
            .map(|b| b.asset.as_str())
            .chain(self.positions.iter().map(|p| p.pnl_asset.as_str()))
            .filter(|&asset| asset != "USDT")
            .collect();
        assets.sort_unstable();
        assets.dedup();
        let symbols = assets
            .iter()
            .map(|asset| E::symbol(asset, "USDT"))
            .filter(|symbol| !self.unlisted.contains_key(symbol))
            .collect();
        (assets.into_iter().map(str::to_owned).collect(), symbols)
    }
}

pub async fn run<E: Account>(app: AppHandle, mut control: watch::Receiver<FeedControl>) {
    let mut kept = Kept::default();
    let mut refreshed: Option<Instant> = None;
    let mut nudged = control.borrow().holdings_wanted;
    let mut live: Option<Live> = None;
    let flush = sleep(Duration::ZERO);
    tokio::pin!(flush);
    let mut dirty = false;
    loop {
        let wanted = Wanted::of::<E>(&control.borrow_and_update());
        let Some(key) = wanted.key.clone() else {
            kept = Kept::default();
            refreshed = None;
            live = None;
            portfolio_changed(&app, |accounts| accounts.remove(&E::ID).is_some());
            if control.changed().await.is_err() {
                return;
            }
            continue;
        };
        // The total shows as pending until the first refresh.
        portfolio_changed(&app, |accounts| match accounts.entry(E::ID) {
            Entry::Vacant(slot) => {
                slot.insert(portfolio::Account::new(E::ID));
                true
            }
            Entry::Occupied(_) => false,
        });
        if !wanted.window_open || wanted.paused {
            live = None;
        }
        let interval = if wanted.window_open { WINDOW_OPEN } else { BACKGROUND };
        let due = refreshed.map_or_else(Instant::now, |at| at + interval);
        if !wanted.paused && Instant::now() >= due {
            refresh::<E>(&app, &key, &mut kept, live.as_ref()).await;
            refreshed = Some(Instant::now());
            // The window is open: follow the held assets between refreshes.
            // Connecting is quick; a failure just leaves the snapshots.
            let symbols = kept.needed::<E>().1;
            if wanted.window_open && live.as_ref().is_none_or(|l| l.symbols != symbols) {
                live = Live::open::<E>(symbols).await;
            }
            continue;
        }
        tokio::select! {
            () = sleep_until(due), if !wanted.paused => {}
            changed = control.changed() => {
                if changed.is_err() {
                    return;
                }
                let now = control.borrow().clone();
                let fresh = refreshed.is_some_and(|at| at.elapsed() < FRESH);
                if now.holdings_wanted != nudged {
                    nudged = now.holdings_wanted;
                    if !fresh {
                        refreshed = None;
                    }
                }
                let next = Wanted::of::<E>(&now);
                if next.key != wanted.key {
                    kept = Kept::default();
                    refreshed = None;
                } else if next.window_open && !wanted.window_open && !fresh {
                    refreshed = None;
                }
            }
            tick = Live::next::<E>(&mut live) => match tick {
                Some((asset, price)) => {
                    if kept.prices.get(&asset) != Some(&price) {
                        kept.prices.insert(asset, price);
                        if !dirty {
                            dirty = true;
                            flush.as_mut().reset(Instant::now() + LIVE_FLUSH);
                        }
                    }
                }
                None => live = None,
            },
            () = &mut flush, if dirty => {
                dirty = false;
                revalue::<E>(&app, &kept);
            }
        }
    }
}

async fn refresh<E: Account>(app: &AppHandle, key: &ApiKey, kept: &mut Kept, live: Option<&Live>) {
    let savings_due = kept.savings.as_ref().is_none_or(|(at, _)| at.elapsed() >= SAVINGS);
    let savings = async { if savings_due { Some(E::savings(key).await) } else { None } };
    let (trading, savings) = tokio::join!(E::trading(key), savings);
    match savings {
        Some(Ok(balances)) => kept.savings = Some((Instant::now(), balances)),
        Some(Err(e)) => log::warn!("{} savings: {e}", E::ID.key()),
        None => {}
    }
    let (mut balances, positions) = match trading {
        Ok(trading) => trading,
        Err(e) => {
            log::warn!("{} holdings: {e}", E::ID.key());
            let error = e.to_string();
            portfolio_changed(app, |accounts| {
                let account =
                    accounts.entry(E::ID).or_insert_with(|| portfolio::Account::new(E::ID));
                let changed = account.error.as_ref() != Some(&error);
                account.error = Some(error);
                account.checked = Some(now_ms());
                changed
            });
            return;
        }
    };
    if let Some((_, savings)) = &kept.savings {
        balances.extend(savings.iter().cloned());
    }
    kept.balances = balances;
    kept.positions = positions;
    kept.unlisted.retain(|_, since| since.elapsed() < UNLISTED);
    let (assets, symbols) = kept.needed::<E>();
    // A live socket already carrying every needed pair has fresher prices
    // than a snapshot would.
    let covered = live.is_some_and(|l| symbols.iter().all(|s| l.symbols.contains(s)));
    if !covered {
        match quotes::snapshot::<E>(&symbols).await {
            Ok(found) => {
                for symbol in &symbols {
                    if !found.contains_key(symbol) {
                        kept.unlisted.insert(symbol.clone(), Instant::now());
                    }
                }
                kept.prices = assets
                    .iter()
                    .filter_map(|asset| {
                        let &(last, open) = found.get(&E::symbol(asset, "USDT"))?;
                        Some((asset.clone(), Price { last, open }))
                    })
                    .collect();
            }
            Err(e) => log::warn!("{} prices: {e}", E::ID.key()),
        }
    }
    let holdings = portfolio::value(E::ID, &kept.balances, &kept.positions, &kept.prices);
    let updated = now_ms();
    portfolio_changed(app, |accounts| {
        let account = accounts.entry(E::ID).or_insert_with(|| portfolio::Account::new(E::ID));
        account.holdings = Some(holdings);
        account.updated = Some(updated);
        account.checked = Some(updated);
        account.error = None;
        true
    });
}

/// Prices moved: the same balances, valued again.
fn revalue<E: Account>(app: &AppHandle, kept: &Kept) {
    let holdings = portfolio::value(E::ID, &kept.balances, &kept.positions, &kept.prices);
    portfolio_changed(app, |accounts| match accounts.get_mut(&E::ID) {
        Some(account) if account.holdings.is_some() => {
            account.holdings = Some(holdings);
            true
        }
        _ => false,
    });
}

/// A socket following the held assets' tickers while the holdings window is
/// open.
struct Live {
    /// The pairs it subscribed to.
    symbols: Vec<String>,
    socket: net::Socket,
    keepalive: Interval,
    /// When the last frame arrived.
    heard: Instant,
}

impl Live {
    async fn open<E: Exchange>(symbols: Vec<String>) -> Option<Self> {
        if symbols.is_empty() {
            return None;
        }
        match quotes::subscribe::<E>(&symbols).await {
            Ok(socket) => {
                log::debug!("{} holdings follow {} pairs live", E::ID.key(), symbols.len());
                let mut keepalive = interval_at(Instant::now() + KEEPALIVE, KEEPALIVE);
                keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
                Some(Self { symbols, socket, keepalive, heard: Instant::now() })
            }
            Err(e) => {
                log::warn!("{} live prices: {e}", E::ID.key());
                None
            }
        }
    }

    /// The next price by asset; `None` once the socket is gone (or there
    /// is none, in which case it waits forever). Keepalive frames go out in
    /// between.
    async fn next<E: Exchange>(live: &mut Option<Self>) -> Option<(String, Price)> {
        let Some(this) = live.as_mut() else {
            return std::future::pending().await;
        };
        loop {
            let silence = sleep_until(this.heard + LIVE_SILENCE);
            tokio::select! {
                message = this.socket.next() => {
                    this.heard = Instant::now();
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            if let Some(tick) = E::quote(&text)
                                && tick.last.is_finite()
                                && let Some(asset) = tick.symbol.strip_suffix("USDT").map(str::to_owned)
                                    .or_else(|| tick.symbol.strip_suffix("-USDT").map(str::to_owned))
                            {
                                return Some((asset, Price { last: tick.last, open: tick.open }));
                            }
                        }
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
                        Some(Ok(_)) => {}
                    }
                }
                _ = this.keepalive.tick(), if E::PING.is_some() => {
                    if let Some(ping) = E::PING
                        && this.socket.send(Message::text(ping)).await.is_err()
                    {
                        return None;
                    }
                }
                () = silence => return None,
            }
        }
    }
}

/// Applies `change` to the accounts; when it says something changed, the bar
/// and the holdings window follow.
fn portfolio_changed(
    app: &AppHandle,
    change: impl FnOnce(&mut BTreeMap<ProviderId, portfolio::Account>) -> bool,
) {
    let shared = app.state::<Shared>();
    let portfolio = {
        let mut model = shared.model();
        if !change(&mut model.accounts) {
            return;
        }
        portfolio::Portfolio::of(&model.accounts)
    };
    bar::request_render(app);
    window::emit_portfolio(app, &portfolio);
}

/// Milliseconds on the exchange's clock, as far as it is known: requests
/// signed with a time too far from it are refused.
pub struct Clock(AtomicI64);

impl Clock {
    pub const fn new() -> Self {
        Self(AtomicI64::new(0))
    }

    pub fn now_ms(&self) -> i64 {
        local_ms() + self.0.load(Ordering::Relaxed)
    }

    /// The exchange said it is `server_ms` now.
    pub fn set(&self, server_ms: i64) {
        self.0.store(server_ms - local_ms(), Ordering::Relaxed);
    }
}

fn local_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Sends a signed request to the first of `hosts` that can be reached.
/// Exchanges explain a refusal in the body, so it comes back whatever the
/// status.
pub async fn send(
    hosts: &[&str],
    build: impl Fn(&Client, &str) -> RequestBuilder,
) -> Result<(StatusCode, String), Error> {
    let mut failure = None;
    for host in hosts {
        let sent = async {
            let response = http::send(|client| build(client, host)).await?;
            let status = response.status();
            Ok::<_, http::Error>((status, response.text().await?))
        };
        match sent.await {
            Ok(answer) => return Ok(answer),
            Err(e) => failure = Some(e),
        }
    }
    Err(failure.expect("at least one host").into())
}
