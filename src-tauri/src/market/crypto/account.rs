//! An exchange account through the user's read-only API key: what the
//! exchanges share in reading it (signing, the server's clock) and the task
//! that keeps its holdings current.
//!
//! Holdings refresh every two minutes, every ten seconds while the holdings
//! window is open, and when the dropdown opens on ones older than fifteen
//! seconds. Funding and earn balances change rarely and cost more to ask for,
//! so they refresh at most every five minutes. Prices come from one short
//! look at the exchange's tickers each time.

use std::{
    collections::{BTreeMap, HashMap, btree_map::Entry},
    future::Future,
    sync::atomic::{AtomicI64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine as _;
use reqwest::{Client, RequestBuilder, StatusCode};
use ring::hmac;
use tauri::{AppHandle, Manager};
use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};

use super::{Exchange, quotes};
use crate::{
    bar,
    credentials::ApiKey,
    http,
    market::{Error, ProviderId},
    model::{FeedControl, Shared},
    portfolio::{self, Balance, Position, Price},
    window,
};

/// What an exchange needs to read an account.
pub trait Account: Exchange {
    /// Checks that `key` works and can neither trade nor withdraw.
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

/// What a refresh keeps for the next one.
#[derive(Default)]
struct Kept {
    savings: Option<(Instant, Vec<Balance>)>,
    unlisted: HashMap<String, Instant>,
}

pub async fn run<E: Account>(app: AppHandle, mut control: watch::Receiver<FeedControl>) {
    let mut kept = Kept::default();
    let mut refreshed: Option<Instant> = None;
    let mut nudged = control.borrow().holdings_wanted;
    loop {
        let wanted = Wanted::of::<E>(&control.borrow_and_update());
        let Some(key) = wanted.key.clone() else {
            kept = Kept::default();
            refreshed = None;
            portfolio_changed(&app, |accounts| accounts.remove(&E::ID).is_some());
            if control.changed().await.is_err() {
                return;
            }
            continue;
        };
        // The total shows as pending until the first refresh.
        portfolio_changed(&app, |accounts| match accounts.entry(E::ID) {
            Entry::Vacant(slot) => {
                slot.insert(empty(E::ID));
                true
            }
            Entry::Occupied(_) => false,
        });
        let interval = if wanted.window_open { WINDOW_OPEN } else { BACKGROUND };
        let due = refreshed.map_or_else(Instant::now, |at| at + interval);
        if !wanted.paused && Instant::now() >= due {
            refresh::<E>(&app, &key, &mut kept).await;
            refreshed = Some(Instant::now());
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
        }
    }
}

async fn refresh<E: Account>(app: &AppHandle, key: &ApiKey, kept: &mut Kept) {
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
                let account = accounts.entry(E::ID).or_insert_with(|| empty(E::ID));
                let changed = account.error.as_ref() != Some(&error);
                account.error = Some(error);
                changed
            });
            return;
        }
    };
    if let Some((_, savings)) = &kept.savings {
        balances.extend(savings.iter().cloned());
    }
    let prices = prices::<E>(&balances, &positions, &mut kept.unlisted).await;
    let holdings = portfolio::value(&balances, &positions, &prices);
    let updated =
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64);
    portfolio_changed(app, |accounts| {
        let account = accounts.entry(E::ID).or_insert_with(|| empty(E::ID));
        account.holdings = Some(holdings);
        account.updated = Some(updated);
        account.error = None;
        true
    });
}

fn empty(exchange: ProviderId) -> portfolio::Account {
    portfolio::Account {
        exchange,
        name: exchange.name(),
        holdings: None,
        updated: None,
        error: None,
    }
}

/// USDT prices of what `balances` and `positions` hold, by asset.
async fn prices<E: Account>(
    balances: &[Balance],
    positions: &[Position],
    unlisted: &mut HashMap<String, Instant>,
) -> HashMap<String, Price> {
    unlisted.retain(|_, since| since.elapsed() < UNLISTED);
    let mut assets: Vec<&str> = balances
        .iter()
        .filter(|b| b.amount != 0.0)
        .map(|b| b.asset.as_str())
        .chain(positions.iter().map(|p| p.pnl_asset.as_str()))
        .filter(|&asset| asset != "USDT")
        .collect();
    assets.sort_unstable();
    assets.dedup();
    let symbols: Vec<String> = assets
        .iter()
        .map(|asset| E::symbol(asset, "USDT"))
        .filter(|symbol| !unlisted.contains_key(symbol))
        .collect();
    let found = match quotes::snapshot::<E>(&symbols).await {
        Ok(found) => found,
        Err(e) => {
            log::warn!("{} prices: {e}", E::ID.key());
            return HashMap::new();
        }
    };
    for symbol in &symbols {
        if !found.contains_key(symbol) {
            unlisted.insert(symbol.clone(), Instant::now());
        }
    }
    assets
        .into_iter()
        .filter_map(|asset| {
            let &(last, open) = found.get(&E::symbol(asset, "USDT"))?;
            Some((asset.to_owned(), Price { last, open }))
        })
        .collect()
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

pub fn hmac_sha256(secret: &str, message: &str) -> hmac::Tag {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes()), message.as_bytes())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
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
