//! Menu bar quotes for pools: DEX Screener's statistics, polled every
//! [`POLL`] for every watched pool in one request per chain. Nothing is
//! polled while nobody can see the bar.

use std::{collections::BTreeMap, time::Duration};

use crate::app::App;
use tokio::{sync::watch, time::sleep};

use super::{ID, gecko, parse, screener};
use crate::{
    market,
    model::{FeedControl, Quote, Status},
};

/// DEX Screener's edge turns a response over every 30 seconds; twice that
/// catches each new one within moments.
const POLL: Duration = Duration::from_secs(15);
const MAX_BACKOFF_SECS: u64 = 60;

pub async fn run(app: App, mut control: watch::Receiver<FeedControl>) {
    // The key as saved; the settings command hands over changes directly.
    gecko::configure(control.borrow().credentials.coingecko.clone().map(|k| k.key));
    let mut failures: u32 = 0;
    let mut live = false;
    loop {
        let wanted = market::wanted(&control.borrow_and_update(), ID);
        let (symbols, paused) = &wanted;
        if symbols.is_empty() || *paused != 0 {
            let status = if symbols.is_empty() { Status::Idle } else { Status::Paused };
            market::set_status(&app, ID, status);
            failures = 0;
            live = false;
            if market::changed(&mut control, ID, &wanted).await.is_err() {
                return;
            }
            continue;
        }
        if !live {
            market::set_status(&app, ID, Status::Connecting);
        }
        let delay = match refresh(&app, symbols).await {
            Ok(()) => {
                live = true;
                failures = 0;
                market::set_status(&app, ID, Status::Live);
                POLL
            }
            Err(e) => {
                log::warn!("dexscreener quotes failed: {e}");
                live = false;
                failures += 1;
                let delay =
                    Duration::from_secs((1u64 << (failures - 1).min(6)).min(MAX_BACKOFF_SECS));
                market::set_status(&app, ID, Status::Retrying { retry_in_secs: delay.as_secs() });
                delay
            }
        };
        tokio::select! {
            () = sleep(delay) => {}
            changed = market::changed(&mut control, ID, &wanted) => {
                if changed.is_err() {
                    return;
                }
                // Another pool set: ask again right away.
                failures = 0;
            }
        }
    }
}

/// One round over every chain's pools. Quotes already in the model stay
/// when a pool is missing from the answer; a chain that fails fails the
/// round, after the others have been applied.
async fn refresh(app: &App, symbols: &[String]) -> Result<(), market::Error> {
    let mut by_chain: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for symbol in symbols {
        if let Some((chain, address)) = parse(symbol) {
            by_chain.entry(chain).or_default().push(address);
        }
    }
    let mut failure = None;
    for (chain, addresses) in &by_chain {
        match screener::pairs(chain, addresses).await {
            Ok(pairs) => apply(app, &pairs),
            Err(e) => failure = Some(e),
        }
    }
    failure.map_or(Ok(()), Err)
}

fn apply(app: &App, pairs: &[screener::Pair]) {
    let shared = app.shared();
    let mut model = shared.model();
    let mut changed = false;
    for pair in pairs {
        let (Some(last), Some(open)) = (pair.price(), pair.open()) else { continue };
        let id = market::instrument_id(ID, &pair.symbol());
        let quote = Quote { last, open, session: None, ..Default::default() };
        changed |= market::record_quote(&mut model, id, quote, None);
    }
    drop(model);
    if changed {
        market::quotes_changed(app);
    }
}
