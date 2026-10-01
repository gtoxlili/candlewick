//! Menu bar quotes: one websocket carries every watched pair of the exchange
//! into the shared model. It is dropped whenever the pair set changes or
//! nobody can see the menu bar (sleep, display off), and re-established with
//! exponential backoff after failures. [`snapshot`] uses the same socket for
//! one look at a set of pairs.

use std::{collections::HashMap, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tauri::{AppHandle, Manager};
use tokio::{
    sync::watch,
    time::{Instant, MissedTickBehavior, interval_at, sleep, timeout, timeout_at},
};
use tokio_tungstenite::tungstenite::Message;

use super::{Exchange, KEEPALIVE};
use crate::{
    bar, market,
    model::{FeedControl, Quote, Shared, Status},
    net,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Every exchange pings or answers ours within 20 seconds, so this much
/// silence means a dead link (typically a network change the socket never
/// noticed).
const SILENCE_LIMIT: Duration = Duration::from_secs(45);
/// Exchanges push each pair as its own frame: Binance all of them within
/// moments once a second, which waiting this long turns into a single redraw.
const COALESCE: Duration = Duration::from_millis(120);
/// OKX pushes a pair up to ten times a second; the bar redraws no more often
/// than this, whichever exchange it is.
const REDRAW_INTERVAL: Duration = Duration::from_secs(1);
/// A session that lasted this long was healthy, so reconnect immediately
/// (exchanges close connections after a day or so).
const HEALTHY_SESSION: Duration = Duration::from_secs(60);
const MAX_BACKOFF_SECS: u64 = 60;

enum End {
    Reconfigure,
    Shutdown,
    Lost(String),
}

pub async fn run<E: Exchange>(app: AppHandle, mut control: watch::Receiver<FeedControl>) {
    // Start from the server that last worked; failures walk to the next.
    let mut preferred = 0;
    let mut failures: u32 = 0;
    loop {
        let wanted = market::wanted(&control.borrow_and_update(), E::ID);
        let (symbols, paused) = &wanted;
        if symbols.is_empty() || *paused != 0 {
            let status = if symbols.is_empty() { Status::Idle } else { Status::Paused };
            market::set_status(&app, E::ID, status);
            if market::changed(&mut control, E::ID, &wanted).await.is_err() {
                return;
            }
            continue;
        }

        market::set_status(&app, E::ID, Status::Connecting);
        let server = (preferred + failures as usize) % E::SOCKETS.len();
        let (host, port) = E::SOCKETS[server];
        let started = Instant::now();
        let path = E::quotes_path(symbols);
        let connecting = net::connect(host, port, &path);
        let end = tokio::select! {
            result = timeout(CONNECT_TIMEOUT, connecting) => match result {
                Err(_) => End::Lost("连接超时".to_owned()),
                Ok(Err(e)) => End::Lost(e.to_string()),
                Ok(Ok((socket, route))) => {
                    log::info!("streaming {} quotes from {host} ({route})", E::ID.key());
                    preferred = server;
                    pump::<E>(&app, &mut control, &wanted, socket, route).await
                }
            },
            changed = market::changed(&mut control, E::ID, &wanted) => match changed {
                Ok(()) => End::Reconfigure,
                Err(_) => End::Shutdown,
            },
        };

        let reason = match end {
            End::Shutdown => return,
            End::Reconfigure => {
                failures = 0;
                continue;
            }
            End::Lost(reason) => reason,
        };
        log::warn!("{} quotes from {host} ended: {reason}", E::ID.key());
        if started.elapsed() >= HEALTHY_SESSION {
            failures = 0;
            continue;
        }
        failures += 1;
        let delay = Duration::from_secs((1u64 << (failures - 1).min(6)).min(MAX_BACKOFF_SECS));
        market::set_status(
            &app,
            E::ID,
            Status::Retrying { reason, retry_in_secs: delay.as_secs() },
        );
        tokio::select! {
            () = sleep(delay) => {}
            changed = market::changed(&mut control, E::ID, &wanted) => if changed.is_err() { return },
        }
    }
}

async fn pump<E: Exchange>(
    app: &AppHandle,
    control: &mut watch::Receiver<FeedControl>,
    wanted: &(Vec<String>, u8),
    mut socket: net::Socket,
    route: net::Route,
) -> End {
    for frame in E::quotes_subscribe(&wanted.0) {
        if let Err(e) = socket.send(Message::text(frame)).await {
            return End::Lost(e.to_string());
        }
    }
    market::set_status(app, E::ID, Status::Live(route));
    let mut keepalive = interval_at(Instant::now() + KEEPALIVE, KEEPALIVE);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let silence = sleep(SILENCE_LIMIT);
    let flush = sleep(Duration::ZERO);
    tokio::pin!(silence, flush);
    let mut dirty = false;
    let mut redrawn: Option<Instant> = None;

    loop {
        tokio::select! {
            changed = market::changed(control, E::ID, wanted) => {
                return if changed.is_ok() { End::Reconfigure } else { End::Shutdown };
            }
            message = socket.next() => {
                silence.as_mut().reset(Instant::now() + SILENCE_LIMIT);
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if apply::<E>(app, &text) && !dirty {
                            dirty = true;
                            let mut at = Instant::now() + COALESCE;
                            if let Some(redrawn) = redrawn {
                                at = at.max(redrawn + REDRAW_INTERVAL);
                            }
                            flush.as_mut().reset(at);
                        }
                    }
                    // Pings are answered by tungstenite on the next read.
                    Some(Ok(Message::Close(frame))) => {
                        let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                        return End::Lost(format!("服务器关闭了连接 {reason}").trim_end().to_owned());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return End::Lost(e.to_string()),
                    None => return End::Lost("连接已断开".to_owned()),
                }
            }
            _ = keepalive.tick(), if E::PING.is_some() => {
                if let Some(ping) = E::PING
                    && let Err(e) = socket.send(Message::text(ping)).await
                {
                    return End::Lost(e.to_string());
                }
            }
            () = &mut flush, if dirty => {
                dirty = false;
                redrawn = Some(Instant::now());
                bar::request_render(app);
            }
            () = &mut silence => return End::Lost("长时间没有收到行情".to_owned()),
        }
    }
}

/// Stores one frame's quote. Returns whether anything visible changed.
fn apply<E: Exchange>(app: &AppHandle, text: &str) -> bool {
    let Some(tick) = E::quote(text) else {
        return false;
    };
    if !tick.last.is_finite() {
        return false;
    }
    let quote = Quote { last: tick.last, open: tick.open, session: None };
    let id = market::instrument_id(E::ID, &tick.symbol);
    let shared = app.state::<Shared>();
    let mut model = shared.model();
    if let Some(existing) = model.quotes.get_mut(&id) {
        let changed = existing.last != quote.last || existing.open != quote.open;
        *existing = quote;
        return changed;
    }
    // Ignore frames for pairs removed while this connection was still open.
    if !model.settings.watchlist.iter().any(|i| i.provider == E::ID && i.symbol == tick.symbol) {
        return false;
    }
    model.quotes.insert(id, quote);
    true
}

/// How long [`snapshot`] waits for pairs to answer once subscribed; a pair
/// the exchange doesn't list never does.
const SNAPSHOT_WAIT: Duration = Duration::from_secs(3);

/// A socket subscribed to the tickers of `symbols`, on the first of the
/// exchange's servers that answers.
pub async fn subscribe<E: Exchange>(symbols: &[String]) -> Result<net::Socket, String> {
    let path = E::quotes_path(symbols);
    let mut failure = String::new();
    let mut connected = None;
    for &(host, port) in E::SOCKETS {
        match timeout(CONNECT_TIMEOUT, net::connect(host, port, &path)).await {
            Ok(Ok((socket, _))) => {
                connected = Some(socket);
                break;
            }
            Ok(Err(e)) => failure = e.to_string(),
            Err(_) => failure = "连接超时".to_owned(),
        }
    }
    let mut socket = connected.ok_or(failure)?;
    for frame in E::quotes_subscribe(symbols) {
        socket.send(Message::text(frame)).await.map_err(|e| e.to_string())?;
    }
    Ok(socket)
}

/// The last price and the one 24 hours before of each of `symbols` that
/// answers within [`SNAPSHOT_WAIT`], by symbol, from a short-lived socket:
/// each pair's first frame is all it needs.
pub async fn snapshot<E: Exchange>(
    symbols: &[String],
) -> Result<HashMap<String, (f64, f64)>, String> {
    let mut prices = HashMap::new();
    if symbols.is_empty() {
        return Ok(prices);
    }
    let mut socket = subscribe::<E>(symbols).await?;
    let deadline = Instant::now() + SNAPSHOT_WAIT;
    while prices.len() < symbols.len() {
        let Ok(message) = timeout_at(deadline, socket.next()).await else { break };
        match message {
            Some(Ok(Message::Text(text))) => {
                if let Some(tick) = E::quote(&text)
                    && tick.last.is_finite()
                {
                    prices.insert(tick.symbol.into_owned(), (tick.last, tick.open));
                }
            }
            Some(Ok(_)) => {}
            Some(Err(_)) | None => break,
        }
    }
    let _ = socket.close(None).await;
    Ok(prices)
}
