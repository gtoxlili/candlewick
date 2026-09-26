//! Binance miniTicker stream → quotes in the shared model.
//!
//! One websocket carries every configured symbol. It is dropped whenever the
//! symbol set changes or nobody can see the menu bar (sleep, display off),
//! and re-established with exponential backoff after failures.

use std::{borrow::Cow, time::Duration};

use futures_util::StreamExt;
use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tokio::{
    sync::watch,
    time::{Instant, sleep, timeout},
};
use tokio_tungstenite::tungstenite::Message;

use crate::{
    model::{FeedControl, Quote, Shared, Status},
    net, tray, window,
};

/// Market-data endpoints; consecutive failures alternate between them.
const HOSTS: [&str; 2] = ["stream.binance.com", "data-stream.binance.vision"];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The server pings every 20 s, so this much silence means a dead link
/// (typically a network change the socket never noticed).
const SILENCE_LIMIT: Duration = Duration::from_secs(45);
/// Binance pushes each symbol as its own frame within the same second;
/// waiting this long turns them into a single redraw.
const COALESCE: Duration = Duration::from_millis(120);
/// A session that lasted this long was healthy, so reconnect immediately
/// (Binance closes every connection after 24 h).
const HEALTHY_SESSION: Duration = Duration::from_secs(60);
const MAX_BACKOFF_SECS: u64 = 60;

pub fn spawn(app: AppHandle, control: watch::Receiver<FeedControl>) {
    tauri::async_runtime::spawn(run(app, control));
}

enum End {
    Reconfigure,
    Shutdown,
    Lost(String),
}

async fn run(app: AppHandle, mut control: watch::Receiver<FeedControl>) {
    // Start from the endpoint that last worked; failures walk to the next.
    let mut preferred = 0;
    let mut failures: u32 = 0;
    loop {
        let wanted = control.borrow_and_update().clone();
        if wanted.symbols.is_empty() || wanted.paused != 0 {
            set_status(&app, if wanted.paused != 0 { Status::Paused } else { Status::NoCoins });
            if control.changed().await.is_err() {
                return;
            }
            continue;
        }

        set_status(&app, Status::Connecting);
        let host_index = (preferred + failures as usize) % HOSTS.len();
        let host = HOSTS[host_index];
        let started = Instant::now();
        let path = stream_path(&wanted.symbols);
        let connecting = net::connect(host, &path);
        let end = tokio::select! {
            result = timeout(CONNECT_TIMEOUT, connecting) => match result {
                Err(_) => End::Lost("连接超时".to_owned()),
                Ok(Err(e)) => End::Lost(e.to_string()),
                Ok(Ok((socket, route))) => {
                    log::info!("streaming from {host} ({route})");
                    preferred = host_index;
                    pump(&app, &mut control, socket, route).await
                }
            },
            changed = control.changed() => match changed {
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
        log::warn!("stream from {host} ended: {reason}");
        if started.elapsed() >= HEALTHY_SESSION {
            failures = 0;
            continue;
        }
        failures += 1;
        let delay = Duration::from_secs((1u64 << (failures - 1).min(6)).min(MAX_BACKOFF_SECS));
        set_status(&app, Status::Retrying { reason, retry_in_secs: delay.as_secs() });
        tokio::select! {
            () = sleep(delay) => {}
            changed = control.changed() => if changed.is_err() { return },
        }
    }
}

async fn pump(
    app: &AppHandle,
    control: &mut watch::Receiver<FeedControl>,
    mut socket: net::Socket,
    route: net::Route,
) -> End {
    set_status(app, Status::Live(route));
    let silence = sleep(SILENCE_LIMIT);
    let flush = sleep(Duration::ZERO);
    tokio::pin!(silence, flush);
    let mut dirty = false;

    loop {
        tokio::select! {
            changed = control.changed() => {
                return if changed.is_ok() { End::Reconfigure } else { End::Shutdown };
            }
            message = socket.next() => {
                silence.as_mut().reset(Instant::now() + SILENCE_LIMIT);
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if apply(app, &text) && !dirty {
                            dirty = true;
                            flush.as_mut().reset(Instant::now() + COALESCE);
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
            () = &mut flush, if dirty => {
                dirty = false;
                tray::request_render(app);
            }
            () = &mut silence => return End::Lost("长时间没有收到行情".to_owned()),
        }
    }
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    data: MiniTicker<'a>,
}

#[derive(Deserialize)]
struct MiniTicker<'a> {
    #[serde(rename = "s", borrow)]
    symbol: Cow<'a, str>,
    #[serde(rename = "c", borrow)]
    last: Cow<'a, str>,
    #[serde(rename = "o", borrow)]
    open: Cow<'a, str>,
}

/// Stores one miniTicker update. Returns whether anything visible changed.
fn apply(app: &AppHandle, text: &str) -> bool {
    let Ok(Envelope { data }) = serde_json::from_str::<Envelope>(text) else {
        return false;
    };
    let (Ok(last), Ok(open)) = (data.last.parse::<f64>(), data.open.parse::<f64>()) else {
        return false;
    };
    let quote = Quote { last, open };
    let shared = app.state::<Shared>();
    let mut model = shared.model();
    if let Some(existing) = model.quotes.get_mut(data.symbol.as_ref()) {
        let changed = existing.last != last || existing.open != open;
        *existing = quote;
        return changed;
    }
    // Ignore frames for coins removed while this connection was still open.
    if !model.settings.coins.iter().any(|c| c.symbol == data.symbol) {
        return false;
    }
    model.quotes.insert(data.symbol.into_owned(), quote);
    true
}

/// `/stream?streams=btcusdt@miniTicker/ethusdt@miniTicker`
fn stream_path(symbols: &[String]) -> String {
    let mut path = String::from("/stream?streams=");
    for (i, symbol) in symbols.iter().enumerate() {
        if i > 0 {
            path.push('/');
        }
        // Symbols can be non-ASCII (e.g. 币安人生USDT).
        path.push_str(&net::percent_encode(&symbol.to_lowercase()));
        path.push_str("@miniTicker");
    }
    path
}

fn set_status(app: &AppHandle, status: Status) {
    let shared = app.state::<Shared>();
    {
        let mut model = shared.model();
        if model.status == status {
            return;
        }
        match status {
            Status::Live(_) => model.stale = false,
            // Paused: nothing arrives while asleep, so on wake the prices on
            // screen are old until the stream is live again.
            Status::Retrying { .. } | Status::Paused => model.stale = true,
            Status::NoCoins | Status::Connecting => {}
        }
        model.status = status.clone();
    }
    tray::request_render(app);
    window::emit_status(app, &status);
}
