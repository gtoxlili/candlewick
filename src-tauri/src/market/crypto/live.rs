//! The chart's live feed for one pair. An exchange's [`Session`] says what to
//! subscribe to and makes sense of each frame; [`run`] does the I/O around it:
//! connecting and reconnecting, keepalive frames, and trades in batches.

use std::{future::Future, time::Duration};

use futures_util::{SinkExt, StreamExt, stream::FuturesUnordered};
use tauri::ipc::Channel;
use tokio::{
    sync::oneshot,
    time::{Instant, MissedTickBehavior, interval_at, sleep, timeout},
};
use tokio_tungstenite::tungstenite::Message;

use super::{Exchange, KEEPALIVE};
use crate::{
    market::{BoxFuture, FeedState, LiveEvent, Trade},
    model::Instrument,
    net,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Every exchange pings or answers ours within 20 seconds, so this much
/// silence means a dead socket (e.g. after a network change).
const SILENCE_LIMIT: Duration = Duration::from_secs(30);
/// Trades arrive dozens a second; the chart eases between these batches.
const FLUSH: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// One pair's feed on one exchange, without the I/O: each connection starts
/// over with [`connected`](Self::connected), and frames go through
/// [`frame`](Self::frame).
pub trait Session: Send + 'static {
    /// What finishes beside the socket, e.g. a REST snapshot of the book.
    type Fetched: Send + 'static;

    /// Where the socket connects.
    fn path(&self) -> String;

    /// A connection is open: start over.
    fn connected(&mut self, out: &mut Out<Self::Fetched>);

    fn frame(&mut self, text: &str, out: &mut Out<Self::Fetched>);

    /// Something [`Out::fetch`] started has finished.
    fn fetched(&mut self, fetched: Self::Fetched, out: &mut Out<Self::Fetched>);
}

/// What a session wants done.
pub struct Out<F> {
    frames: Vec<String>,
    fetches: Vec<BoxFuture<'static, F>>,
    events: Vec<LiveEvent>,
    trades: Vec<Trade>,
}

impl<F> Out<F> {
    fn new() -> Self {
        Self { frames: Vec::new(), fetches: Vec::new(), events: Vec::new(), trades: Vec::new() }
    }

    /// Sends a frame on the socket.
    pub fn send(&mut self, frame: String) {
        self.frames.push(frame);
    }

    /// Runs `task` beside the socket and hands its result to
    /// [`Session::fetched`]; a lost connection drops it.
    pub fn fetch(&mut self, task: impl Future<Output = F> + Send + 'static) {
        self.fetches.push(Box::pin(task));
    }

    pub fn event(&mut self, event: LiveEvent) {
        self.events.push(event);
    }

    /// A trade, oldest first; trades reach the window in batches.
    pub fn trade(&mut self, trade: Trade) {
        self.trades.push(trade);
    }
}

#[cfg(test)]
impl<F> Out<F> {
    pub fn for_test() -> Self {
        Self::new()
    }

    pub fn take_frames(&mut self) -> Vec<String> {
        std::mem::take(&mut self.frames)
    }

    pub fn take_events(&mut self) -> Vec<LiveEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn take_trades(&mut self) -> Vec<Trade> {
        std::mem::take(&mut self.trades)
    }
}

enum Ended {
    Stopped,
    Lost(String),
}

pub async fn run<E: Exchange>(
    instrument: Instrument,
    mut stop: oneshot::Receiver<()>,
    events: Channel<LiveEvent>,
) {
    let mut session = E::live(&instrument);
    let path = session.path();
    // Consecutive failures walk through the servers.
    let mut server = 0;
    let mut failures: u32 = 0;
    loop {
        if !send(&events, LiveEvent::State { state: FeedState::Connecting }) {
            return;
        }
        let (host, port) = E::SOCKETS[server % E::SOCKETS.len()];
        let connecting = timeout(CONNECT_TIMEOUT, net::connect(host, port, &path));
        let connected = tokio::select! {
            result = connecting => result,
            _ = &mut stop => return,
        };
        let reason = match connected {
            Ok(Ok((socket, _))) => {
                failures = 0;
                match pump(&mut session, E::PING, socket, &mut stop, &events).await {
                    Ended::Stopped => return,
                    Ended::Lost(reason) => reason,
                }
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "connection timed out".to_owned(),
        };
        log::info!("chart stream for {} ended: {reason}", instrument.id());
        if !send(&events, LiveEvent::State { state: FeedState::Offline }) {
            return;
        }
        failures += 1;
        server += 1;
        tokio::select! {
            () = sleep(backoff(failures)) => {}
            _ = &mut stop => return,
        }
    }
}

async fn pump<S: Session>(
    session: &mut S,
    ping: Option<&'static str>,
    mut socket: net::Socket,
    stop: &mut oneshot::Receiver<()>,
    events: &Channel<LiveEvent>,
) -> Ended {
    if !send(events, LiveEvent::State { state: FeedState::Live }) {
        return Ended::Stopped;
    }
    let mut out = Out::new();
    session.connected(&mut out);
    let mut fetches = FuturesUnordered::new();
    let mut batch: Vec<Trade> = Vec::new();
    let mut keepalive = interval_at(Instant::now() + KEEPALIVE, KEEPALIVE);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let silence = sleep(SILENCE_LIMIT);
    let flush = sleep(Duration::ZERO);
    tokio::pin!(silence, flush);

    loop {
        for frame in out.frames.drain(..) {
            if let Err(e) = socket.send(Message::text(frame)).await {
                return Ended::Lost(e.to_string());
            }
        }
        fetches.extend(out.fetches.drain(..));
        for event in out.events.drain(..) {
            if !send(events, event) {
                return Ended::Stopped;
            }
        }
        if !out.trades.is_empty() {
            if batch.is_empty() {
                flush.as_mut().reset(Instant::now() + FLUSH);
            }
            batch.append(&mut out.trades);
        }

        tokio::select! {
            _ = &mut *stop => return Ended::Stopped,
            Some(fetched) = fetches.next(), if !fetches.is_empty() => {
                session.fetched(fetched, &mut out);
            }
            message = socket.next() => {
                silence.as_mut().reset(Instant::now() + SILENCE_LIMIT);
                match message {
                    Some(Ok(Message::Text(text))) => session.frame(&text, &mut out),
                    // Pings are answered by tungstenite on the next read.
                    Some(Ok(Message::Close(frame))) => {
                        let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                        return Ended::Lost(format!("the server closed the connection {reason}").trim_end().to_owned());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Ended::Lost(e.to_string()),
                    None => return Ended::Lost("disconnected".to_owned()),
                }
            }
            () = &mut flush, if !batch.is_empty() => {
                if !send(events, LiveEvent::Trades { trades: std::mem::take(&mut batch) }) {
                    return Ended::Stopped;
                }
            }
            _ = keepalive.tick(), if ping.is_some() => {
                if let Some(ping) = ping {
                    out.send(ping.to_owned());
                }
            }
            () = &mut silence => return Ended::Lost("no quotes for too long".to_owned()),
        }
    }
}

/// Waits before another try: none at first, then doubling up to a limit.
pub fn backoff(failures: u32) -> Duration {
    match failures {
        0 => Duration::ZERO,
        n => Duration::from_secs(1 << (n - 1).min(5)).min(MAX_BACKOFF),
    }
}

/// False once the window is gone.
fn send(events: &Channel<LiveEvent>, event: LiveEvent) -> bool {
    events.send(event).is_ok()
}
