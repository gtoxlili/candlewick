//! Menu bar quotes: the watched codes' snapshots every [`POLL`] while
//! Shanghai trades, one request per kind (funds one apiece). Out of hours
//! one round settles the closing prices and nothing is asked again until
//! the next session; nothing at all while nobody can see the bar.

use std::{collections::BTreeMap, time::Duration};

use tauri::{AppHandle, Manager};
use tokio::{sync::watch, time::sleep};

use super::{ID, Kind, POLL, api, session};
use crate::{
    bar,
    market::{self, Error},
    model::{FeedControl, Quote, Shared, Status},
};

const MAX_BACKOFF_SECS: u64 = 60;

/// What this provider's part of the control is: its codes, whether the
/// machine is being looked at, and the key.
#[derive(Clone, PartialEq)]
struct Wanted {
    symbols: Vec<String>,
    paused: u8,
    key: Option<String>,
}

impl Wanted {
    fn of(control: &FeedControl) -> Self {
        let (symbols, paused) = market::wanted(control, ID);
        Self { symbols, paused, key: control.credentials.ths.as_ref().map(|k| k.key.clone()) }
    }
}

/// Waits until `wanted` changes; fails once the app is shutting down.
async fn changed(
    control: &mut watch::Receiver<FeedControl>,
    wanted: &Wanted,
) -> Result<(), watch::error::RecvError> {
    loop {
        control.changed().await?;
        if &Wanted::of(&control.borrow_and_update()) != wanted {
            return Ok(());
        }
    }
}

pub async fn run(app: AppHandle, mut control: watch::Receiver<FeedControl>) {
    let mut failures: u32 = 0;
    let mut live = false;
    // The prices have been read since the market last closed.
    let mut settled = false;
    loop {
        let wanted = Wanted::of(&control.borrow_and_update());
        api::configure(wanted.key.clone());
        let resting = if wanted.symbols.is_empty() {
            Some(Status::Idle)
        } else if wanted.key.is_none() {
            Some(Status::NoCredentials)
        } else if wanted.paused != 0 {
            Some(Status::Paused)
        } else {
            None
        };
        if let Some(status) = resting {
            market::set_status(&app, ID, status);
            (failures, live, settled) = (0, false, false);
            if changed(&mut control, &wanted).await.is_err() {
                return;
            }
            continue;
        }
        let closed_for = session::until_open(session::now(), session::trading_today().await);
        let delay = if closed_for > 0 && settled {
            session::rest(closed_for)
        } else {
            if !live {
                market::set_status(&app, ID, Status::Connecting);
            }
            match refresh(&app, &wanted.symbols).await {
                Ok(()) => {
                    (live, failures, settled) = (true, 0, closed_for > 0);
                    market::set_status(&app, ID, Status::Live);
                    if settled { session::rest(closed_for) } else { POLL }
                }
                Err(e) => {
                    log::warn!("tonghuashun quotes failed: {e}");
                    live = false;
                    failures += 1;
                    let delay =
                        Duration::from_secs((1u64 << (failures - 1).min(6)).min(MAX_BACKOFF_SECS));
                    market::set_status(
                        &app,
                        ID,
                        Status::Retrying { retry_in_secs: delay.as_secs() },
                    );
                    delay
                }
            }
        };
        tokio::select! {
            () = sleep(delay) => {}
            result = changed(&mut control, &wanted) => {
                if result.is_err() {
                    return;
                }
                // Other codes or another key: read them right away.
                failures = 0;
                settled = false;
            }
        }
    }
}

/// One round over every watched code. Quotes already in the model stay
/// when a code is missing from the answer; a kind that fails fails the
/// round, after the others have been applied.
async fn refresh(app: &AppHandle, symbols: &[String]) -> Result<(), Error> {
    let mut by_kind: BTreeMap<Kind, Vec<String>> = BTreeMap::new();
    for symbol in symbols {
        if let Some(kind) = api::kind(symbol).await? {
            by_kind.entry(kind).or_default().push(symbol.clone());
        }
    }
    let mut failure = None;
    for (kind, symbols) in &by_kind {
        match api::snapshots(*kind, symbols).await {
            Ok(found) => apply(app, &found),
            Err(e) => failure = Some(e),
        }
    }
    failure.map_or(Ok(()), Err)
}

fn apply(app: &AppHandle, found: &[(String, api::Snapshot)]) {
    let shared = app.state::<Shared>();
    let mut model = shared.model();
    let mut changed = false;
    for (symbol, snapshot) in found {
        let id = market::instrument_id(ID, symbol);
        // A code removed while the request was out gets no quote.
        if model.settings.instrument(&id).is_none() {
            continue;
        }
        let quote = Quote { last: snapshot.last, open: snapshot.prev_close, session: None };
        if model.quotes.get(&id) != Some(&quote) {
            model.quotes.insert(id, quote);
            changed = true;
        }
    }
    drop(model);
    if changed {
        bar::request_render(app);
    }
}
