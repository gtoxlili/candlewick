//! The chart's live feed for one pair over one websocket: every trade, the
//! 24h ticker, and the whole book (a REST snapshot kept current by the diff
//! stream) grouped each way the chart offers. A REST ticker fills the window
//! before the first pushes.

use std::{borrow::Cow, future::Future, pin::Pin, time::Duration};

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::value::RawValue;
use tauri::ipc::Channel;
use tokio::{
    sync::oneshot,
    time::{Instant, sleep, timeout},
};
use tokio_tungstenite::tungstenite::Message;

use super::{
    RawTrade, RestTicker, WS_HOSTS, WsTicker,
    depth::{self, Depth, Diff, Snapshot, Sync},
    get,
};
use crate::{
    market::{Error, FeedState, LiveEvent, Stats, Trade},
    model::Instrument,
    net,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Binance pings every 20 seconds, so this much silence means a dead socket
/// (e.g. after a network change).
const SILENCE_LIMIT: Duration = Duration::from_secs(30);
/// Trades arrive dozens a second; the chart eases between these batches.
const FLUSH: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

type BookSnapshot = Pin<Box<dyn Future<Output = Result<Snapshot, Error>> + Send>>;

enum Ended {
    Stopped,
    Lost(String),
}

pub(super) async fn run(
    instrument: Instrument,
    mut stop: oneshot::Receiver<()>,
    events: Channel<LiveEvent>,
) {
    let symbol = &instrument.symbol;
    let steps = depth::steps(instrument.decimals);
    let lower = net::percent_encode(&symbol.to_lowercase());
    let path = format!("/stream?streams={lower}@aggTrade/{lower}@depth/{lower}@ticker");
    // Consecutive failures alternate between the hosts.
    let mut host = 0;
    let mut failures: u32 = 0;
    loop {
        if !send(&events, LiveEvent::State { state: FeedState::Connecting }) {
            return;
        }
        let connecting =
            timeout(CONNECT_TIMEOUT, net::connect(WS_HOSTS[host % WS_HOSTS.len()], &path));
        let connected = tokio::select! {
            result = connecting => result,
            _ = &mut stop => return,
        };
        let reason = match connected {
            Ok(Ok((socket, _))) => {
                failures = 0;
                match pump(symbol, &steps, socket, &mut stop, &events).await {
                    Ended::Stopped => return,
                    Ended::Lost(reason) => reason,
                }
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "连接超时".to_owned(),
        };
        log::info!("chart stream for {symbol} ended: {reason}");
        if !send(&events, LiveEvent::State { state: FeedState::Offline }) {
            return;
        }
        failures += 1;
        host += 1;
        tokio::select! {
            () = sleep(backoff(failures)) => {}
            _ = &mut stop => return,
        }
    }
}

async fn pump(
    symbol: &str,
    steps: &[u64],
    mut socket: net::Socket,
    stop: &mut oneshot::Receiver<()>,
    events: &Channel<LiveEvent>,
) -> Ended {
    if !send(events, LiveEvent::State { state: FeedState::Live }) {
        return Ended::Stopped;
    }
    let ticker = ticker(symbol);
    let silence = sleep(SILENCE_LIMIT);
    let flush = sleep(Duration::ZERO);
    tokio::pin!(ticker, silence, flush);
    let mut ticker_pending = true;
    let mut batch: Vec<Trade> = Vec::new();
    // Diffs wait from the start for the snapshot they apply to.
    let mut depth = Depth::default();
    let mut snapshot = Some(book_snapshot(symbol, Duration::ZERO));
    // Snapshots taken since a pushed diff last applied: a book that keeps
    // losing sync backs off instead of spending the IP's request weight.
    let mut resyncs: u32 = 1;

    loop {
        tokio::select! {
            _ = &mut *stop => return Ended::Stopped,
            stats = &mut ticker, if ticker_pending => {
                ticker_pending = false;
                if let Some(stats) = stats
                    && !send(events, LiveEvent::Stats { stats })
                {
                    return Ended::Stopped;
                }
            }
            result = next(&mut snapshot) => {
                snapshot = None;
                let synced = match result {
                    Ok(fresh) => depth.snapshot(fresh),
                    Err(e) => {
                        log::info!("order book snapshot for {symbol} failed: {e}");
                        Sync::Lost
                    }
                };
                if let Sync::Lost = synced {
                    snapshot = Some(book_snapshot(symbol, backoff(resyncs)));
                    resyncs += 1;
                } else if !send(events, LiveEvent::Book { books: depth.books(steps) }) {
                    return Ended::Stopped;
                }
            }
            message = socket.next() => {
                silence.as_mut().reset(Instant::now() + SILENCE_LIMIT);
                match message {
                    Some(Ok(Message::Text(text))) => match parse(&text) {
                        Some(Push::Trade(trade)) => {
                            if batch.is_empty() {
                                flush.as_mut().reset(Instant::now() + FLUSH);
                            }
                            batch.push(trade);
                        }
                        Some(Push::Diff(diff)) => match depth.diff(diff) {
                            Sync::Changed => {
                                resyncs = 0;
                                if !send(events, LiveEvent::Book { books: depth.books(steps) }) {
                                    return Ended::Stopped;
                                }
                            }
                            Sync::Unchanged => {}
                            Sync::Lost => {
                                log::info!("order book for {symbol} lost sync");
                                if snapshot.is_none() {
                                    snapshot = Some(book_snapshot(symbol, backoff(resyncs)));
                                    resyncs += 1;
                                }
                            }
                        },
                        Some(Push::Event(event)) => {
                            if !send(events, event) {
                                return Ended::Stopped;
                            }
                        }
                        None => {}
                    },
                    // Pings are answered by tungstenite on the next read.
                    Some(Ok(Message::Close(frame))) => {
                        let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                        return Ended::Lost(format!("服务器关闭了连接 {reason}").trim_end().to_owned());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Ended::Lost(e.to_string()),
                    None => return Ended::Lost("连接已断开".to_owned()),
                }
            }
            () = &mut flush, if !batch.is_empty() => {
                if !send(events, LiveEvent::Trades { trades: std::mem::take(&mut batch) }) {
                    return Ended::Stopped;
                }
            }
            () = &mut silence => return Ended::Lost("长时间没有收到行情".to_owned()),
        }
    }
}

/// The ticker as it is now; if this fails, the pushes bring it.
async fn ticker(symbol: &str) -> Option<Stats> {
    let path = format!("/api/v3/ticker/24hr?symbol={}", net::percent_encode(symbol));
    get::<RestTicker>(&path).await.ok().map(|ticker| ticker.stats())
}

/// The whole book as it is after `delay`.
fn book_snapshot(symbol: &str, delay: Duration) -> BookSnapshot {
    let path = format!(
        "/api/v3/depth?symbol={}&limit={}",
        net::percent_encode(symbol),
        depth::SNAPSHOT_LEVELS
    );
    Box::pin(async move {
        sleep(delay).await;
        get::<Snapshot>(&path).await
    })
}

/// Resolves with the pending snapshot, or never while there is none.
async fn next(pending: &mut Option<BookSnapshot>) -> Result<Snapshot, Error> {
    match pending {
        Some(snapshot) => snapshot.await,
        None => std::future::pending().await,
    }
}

/// Waits before another try: none at first, then doubling up to a limit.
fn backoff(failures: u32) -> Duration {
    match failures {
        0 => Duration::ZERO,
        n => Duration::from_secs(1 << (n - 1).min(5)).min(MAX_BACKOFF),
    }
}

enum Push {
    /// Batched before it goes to the window.
    Trade(Trade),
    Diff(Diff),
    Event(LiveEvent),
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    stream: Cow<'a, str>,
    #[serde(borrow)]
    data: &'a RawValue,
}

fn parse(text: &str) -> Option<Push> {
    let envelope: Envelope = serde_json::from_str(text).ok()?;
    let data = envelope.data.get();
    match envelope.stream.rsplit_once('@')?.1 {
        "aggTrade" => serde_json::from_str::<RawTrade>(data).ok().map(|t| Push::Trade(t.trade())),
        "depth" => serde_json::from_str::<Diff>(data).ok().map(Push::Diff),
        "ticker" => serde_json::from_str::<WsTicker>(data)
            .ok()
            .map(|t| Push::Event(LiveEvent::Stats { stats: t.stats() })),
        _ => None,
    }
}

/// False once the window is gone.
fn send(events: &Channel<LiveEvent>, event: LiveEvent) -> bool {
    events.send(event).is_ok()
}
