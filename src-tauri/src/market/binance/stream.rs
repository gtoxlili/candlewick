//! The chart's live feed for one pair: every trade, the top 20 levels of the
//! book and the 24h ticker over one websocket, after a REST snapshot of the
//! ticker and book so the window fills before the first pushes.

use std::{borrow::Cow, time::Duration};

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::value::RawValue;
use tauri::ipc::Channel;
use tokio::{
    sync::oneshot,
    time::{Instant, sleep, timeout},
};
use tokio_tungstenite::tungstenite::Message;

use super::{RawBook, RawTrade, RestTicker, WS_HOSTS, WsTicker, get};
use crate::{
    market::{FeedState, LiveEvent, Trade},
    model::Instrument,
    net,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The book and ticker push every second, so this much silence means a dead
/// socket (e.g. after a network change).
const SILENCE_LIMIT: Duration = Duration::from_secs(30);
/// Trades arrive dozens a second; the chart eases between these batches.
const FLUSH: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

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
    let lower = net::percent_encode(&symbol.to_lowercase());
    let path = format!("/stream?streams={lower}@aggTrade/{lower}@depth20/{lower}@ticker");
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
                match pump(symbol, socket, &mut stop, &events).await {
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
        let delay = Duration::from_secs(1 << (failures - 1).min(5)).min(MAX_BACKOFF);
        tokio::select! {
            () = sleep(delay) => {}
            _ = &mut stop => return,
        }
    }
}

async fn pump(
    symbol: &str,
    mut socket: net::Socket,
    stop: &mut oneshot::Receiver<()>,
    events: &Channel<LiveEvent>,
) -> Ended {
    if !send(events, LiveEvent::State { state: FeedState::Live }) {
        return Ended::Stopped;
    }
    let snapshot = snapshot(symbol);
    let silence = sleep(SILENCE_LIMIT);
    let flush = sleep(Duration::ZERO);
    tokio::pin!(snapshot, silence, flush);
    let mut snapshot_pending = true;
    let mut batch: Vec<Trade> = Vec::new();

    loop {
        tokio::select! {
            _ = &mut *stop => return Ended::Stopped,
            list = &mut snapshot, if snapshot_pending => {
                snapshot_pending = false;
                for event in list {
                    if !send(events, event) {
                        return Ended::Stopped;
                    }
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

/// The ticker and book as they are now; whatever fails is left to the pushes.
async fn snapshot(symbol: &str) -> Vec<LiveEvent> {
    let symbol = net::percent_encode(symbol);
    let ticker_path = format!("/api/v3/ticker/24hr?symbol={symbol}");
    let book_path = format!("/api/v3/depth?symbol={symbol}&limit=20");
    let (ticker, book) = tokio::join!(get::<RestTicker>(&ticker_path), get::<RawBook>(&book_path));
    let mut events = Vec::new();
    if let Ok(ticker) = ticker {
        events.push(LiveEvent::Stats { stats: ticker.stats() });
    }
    if let Ok(book) = book {
        events.push(LiveEvent::Book { book: book.book() });
    }
    events
}

enum Push {
    /// Batched before it goes to the window.
    Trade(Trade),
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
        "depth20" => serde_json::from_str::<RawBook>(data)
            .ok()
            .map(|b| Push::Event(LiveEvent::Book { book: b.book() })),
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
