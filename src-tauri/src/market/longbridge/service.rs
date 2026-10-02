//! The Longbridge quote socket, shared by the menu bar, the chart window and
//! one-off requests. It connects while anything needs it, authenticates with a
//! one-time password, keeps the subscriptions everyone asked for, and
//! reconnects after failures.
//!
//! Frames (all integers big-endian): a header byte (low nibble: 1 request,
//! 2 response, 3 push; 0x10 a signature follows the body; 0x20 the body is
//! gzipped), the command code, then
//! - request: request id (u32), timeout in ms (u16), body length (u24), body;
//! - response: request id (u32), status (u8, 0 = ok), body length (u24), body;
//! - push: body length (u24), body.

use std::{
    collections::HashMap,
    io::Read,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use flate2::read::GzDecoder;
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tauri::{AppHandle, Manager};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::{Instant, sleep, timeout},
};
use tokio_tungstenite::tungstenite::{Message, protocol::CloseFrame};

use super::{
    Account, api,
    clock::Market,
    proto::{self, cmd, sub_type, trade_session},
};
use crate::{
    bar,
    credentials::{self, LongbridgeKeys},
    market::{self, Error, ProviderId},
    model::{FeedControl, Quote, Session, Shared, Status},
    net,
};

const PROVIDER: ProviderId = ProviderId::Longbridge;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// The server pings the socket; this much silence means a dead link. The
/// official SDK waits as long.
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(120);
/// Kept open this long after the last user leaves, so a chart window closed
/// and opened again doesn't cost a new login.
const IDLE_GRACE: Duration = Duration::from_secs(60);
/// Pushes for several symbols arrive within moments; one redraw covers them.
const COALESCE: Duration = Duration::from_millis(120);
const MAX_BACKOFF_SECS: u64 = 60;
/// Tokens are renewed once they have less than this left.
const RENEW_BEFORE: i64 = 15 * 86_400;
const TOKEN_LIFETIME: i64 = 90 * 86_400;

/// What the chart window gets for the symbols it watches.
pub enum Push {
    Quote(proto::PushQuote),
    Depth(proto::PushDepth),
    Trades(proto::PushTrade),
    /// The socket went down; pushes stop until [`Push::Resumed`].
    Offline,
    /// The socket is (back) up and subscribed: state may have moved on.
    Resumed,
}

enum Command {
    Call { cmd: u8, body: Vec<u8>, reply: oneshot::Sender<Result<Vec<u8>, Error>> },
    Account { reply: oneshot::Sender<Result<Account, Error>> },
    Watch { id: u64, symbol: String, sink: mpsc::UnboundedSender<Push> },
    Unwatch { id: u64 },
}

/// Open from the start, so requests made before the service runs wait for it.
struct Commands {
    tx: mpsc::UnboundedSender<Command>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<Command>>>,
}

static COMMANDS: LazyLock<Commands> = LazyLock::new(|| {
    let (tx, rx) = mpsc::unbounded_channel();
    Commands { tx, rx: Mutex::new(Some(rx)) }
});
static ACCOUNT: Mutex<Option<Account>> = Mutex::new(None);

pub(super) async fn run(app: AppHandle, control: watch::Receiver<FeedControl>) {
    let Some(rx) = COMMANDS.rx.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    Service {
        app,
        control,
        commands: rx,
        watchers: HashMap::new(),
        waiting: Vec::new(),
        accounts: Vec::new(),
        tracked: HashMap::new(),
    }
    .run()
    .await;
}

/// Sends a request over the socket, connecting first if needed.
pub(super) async fn call<Req, Resp>(cmd: u8, request: &Req) -> Result<Resp, Error>
where
    Req: prost::Message,
    Resp: prost::Message + Default,
{
    let (reply, answer) = oneshot::channel();
    send(Command::Call { cmd, body: request.encode_to_vec(), reply })?;
    let body = answer.await.map_err(|_| stopped())??;
    Resp::decode(body.as_slice()).map_err(|_| bad_data())
}

/// The account's quote permissions, connecting to find out if needed.
pub(super) async fn account() -> Result<Account, Error> {
    let (reply, answer) = oneshot::channel();
    send(Command::Account { reply })?;
    answer.await.map_err(|_| stopped())?
}

/// What was learned about the account at the last login, if anything.
pub(super) fn last_account() -> Option<Account> {
    ACCOUNT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub(super) fn forget_account() {
    *ACCOUNT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Pushes for `symbol` until the returned watch is dropped.
pub(super) fn watch(symbol: &str) -> Result<Watch, Error> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (sink, pushes) = mpsc::unbounded_channel();
    send(Command::Watch { id, symbol: symbol.to_owned(), sink })?;
    Ok(Watch { id, pushes })
}

pub(super) struct Watch {
    id: u64,
    pub pushes: mpsc::UnboundedReceiver<Push>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = send(Command::Unwatch { id: self.id });
    }
}

fn send(command: Command) -> Result<(), Error> {
    COMMANDS.tx.send(command).map_err(|_| stopped())
}

fn stopped() -> Error {
    Error::Message(t!("error.sourceStopped", source = PROVIDER.name()))
}

fn not_configured() -> Error {
    Error::Message(t!("error.sourceNeedsCredentials", source = PROVIDER.name()))
}

fn bad_data() -> Error {
    Error::Message(t!("error.sourceBadData", source = PROVIDER.name()))
}

fn disconnected() -> String {
    t!("error.sourceDisconnected", source = PROVIDER.name())
}

/// The server closed the connection: why in its own words, where its close
/// frame has any.
fn closed(frame: Option<CloseFrame>) -> String {
    match frame.map(|frame| frame.reason.to_string()).filter(|reason| !reason.is_empty()) {
        Some(reason) => t!("error.sourceSays", source = PROVIDER.name(), message = reason),
        None => t!("error.sourceClosed", source = PROVIDER.name()),
    }
}

type Reply = oneshot::Sender<Result<Vec<u8>, Error>>;

/// Latest quote state of a menu bar symbol, to turn pushes into prices with
/// a change.
struct Tracked {
    market: Market,
    /// Last price of the regular session: the close the extended sessions
    /// are measured against.
    regular_last: f64,
    /// The regular session's reference: the previous day's close.
    prev_close: f64,
    session: i32,
    /// Exchange-local day of the last update, `YYYYMMDD`.
    day: i64,
}

struct Service {
    app: AppHandle,
    control: watch::Receiver<FeedControl>,
    commands: mpsc::UnboundedReceiver<Command>,
    watchers: HashMap<u64, (String, mpsc::UnboundedSender<Push>)>,
    /// Calls waiting for a connection.
    waiting: Vec<(u8, Vec<u8>, Reply)>,
    accounts: Vec<oneshot::Sender<Result<Account, Error>>>,
    tracked: HashMap<String, Tracked>,
}

/// What this provider needs from the shared control.
#[derive(Clone, PartialEq)]
struct Wanted {
    keys: Option<LongbridgeKeys>,
    symbols: Vec<String>,
    paused: u8,
}

impl Wanted {
    fn of(control: &FeedControl) -> Self {
        let (symbols, paused) = market::wanted(control, PROVIDER);
        Self { keys: control.credentials.longbridge.clone(), symbols, paused }
    }
}

enum End {
    Reconfigure,
    Idle,
    Shutdown,
    Lost(String),
}

impl Service {
    async fn run(mut self) {
        let mut failures: u32 = 0;
        loop {
            let wanted = Wanted::of(&self.control.borrow_and_update());
            let Some(keys) = wanted.keys.clone().filter(|_| self.needed(&wanted)) else {
                if wanted.keys.is_none() {
                    self.fail_waiting(&not_configured());
                }
                self.idle_status(&wanted);
                if !self.wait_for_work().await {
                    return;
                }
                continue;
            };

            market::set_status(&self.app, PROVIDER, Status::Connecting);
            let started = Instant::now();
            let end = match self.connect(&keys).await {
                Ok((connection, account, route)) => {
                    log::info!("longbridge connected ({route})");
                    failures = 0;
                    self.publish_account(&account);
                    market::set_status(&self.app, PROVIDER, Status::Live);
                    // A renewed token has reached the control meanwhile.
                    let wanted = Wanted::of(&self.control.borrow_and_update());
                    self.pump(connection, wanted).await
                }
                Err(Connect::Shutdown) => return,
                Err(Connect::Failed(e)) => End::Lost(e.to_string()),
            };
            let reason = match end {
                End::Shutdown => return,
                End::Reconfigure | End::Idle => continue,
                End::Lost(reason) => reason,
            };
            log::warn!("longbridge connection ended: {reason}");
            self.fail_waiting(&Error::Message(reason.clone()));
            for (_, sink) in self.watchers.values() {
                let _ = sink.send(Push::Offline);
            }
            if started.elapsed() < Duration::from_secs(60) {
                failures += 1;
            } else {
                failures = 1;
            }
            let delay = Duration::from_secs((1u64 << (failures - 1).min(6)).min(MAX_BACKOFF_SECS));
            market::set_status(
                &self.app,
                PROVIDER,
                Status::Retrying { retry_in_secs: delay.as_secs() },
            );
            let deadline = Instant::now() + delay;
            loop {
                tokio::select! {
                    () = tokio::time::sleep_until(deadline) => break,
                    changed = self.control.changed() => match changed {
                        Ok(()) if Wanted::of(&self.control.borrow()) != wanted => break,
                        Ok(()) => {}
                        Err(_) => return,
                    },
                    command = self.commands.recv() => match command {
                        Some(command) => self.handle_offline(command),
                        None => return,
                    },
                }
            }
        }
    }

    /// Something needs the socket: the menu bar (unless the machine sleeps),
    /// an open chart, or a request.
    fn needed(&self, wanted: &Wanted) -> bool {
        (!wanted.symbols.is_empty() && wanted.paused == 0)
            || !self.watchers.is_empty()
            || !self.waiting.is_empty()
            || !self.accounts.is_empty()
    }

    fn idle_status(&self, wanted: &Wanted) {
        let status = if wanted.symbols.is_empty() {
            Status::Idle
        } else if wanted.keys.is_none() {
            Status::NoCredentials
        } else {
            Status::Paused
        };
        market::set_status(&self.app, PROVIDER, status);
    }

    /// Returns false once the app is shutting down.
    async fn wait_for_work(&mut self) -> bool {
        tokio::select! {
            changed = self.control.changed() => changed.is_ok(),
            command = self.commands.recv() => match command {
                Some(command) => {
                    self.handle_offline(command);
                    true
                }
                None => false,
            },
        }
    }

    fn handle_offline(&mut self, command: Command) {
        let configured = self.control.borrow().credentials.longbridge.is_some();
        match command {
            Command::Call { reply, .. } if !configured => {
                let _ = reply.send(Err(not_configured()));
            }
            Command::Account { reply } if !configured => {
                let _ = reply.send(Err(not_configured()));
            }
            Command::Call { cmd, body, reply } => self.waiting.push((cmd, body, reply)),
            Command::Account { reply } => self.accounts.push(reply),
            Command::Watch { id, symbol, sink } => {
                self.watchers.insert(id, (symbol, sink));
            }
            Command::Unwatch { id } => {
                self.watchers.remove(&id);
            }
        }
    }

    fn fail_waiting(&mut self, error: &Error) {
        for (_, _, reply) in self.waiting.drain(..) {
            let _ = reply.send(Err(Error::Message(error.to_string())));
        }
        for reply in self.accounts.drain(..) {
            let _ = reply.send(Err(Error::Message(error.to_string())));
        }
    }

    fn publish_account(&mut self, account: &Account) {
        *ACCOUNT.lock().unwrap_or_else(|e| e.into_inner()) = Some(account.clone());
        for reply in self.accounts.drain(..) {
            let _ = reply.send(Ok(account.clone()));
        }
    }

    /// Renews the token if it is about to expire, then logs in.
    async fn connect(
        &self,
        keys: &LongbridgeKeys,
    ) -> Result<(Connection, Account, net::Route), Connect> {
        let login = async {
            let keys = self.renew(keys).await;
            let hosts = api::hosts().await;
            let otp = api::one_time_password(&keys).await?;
            // Names and messages come back in this language.
            let language = super::language();
            let headers = [("accept-language", language)];
            let (socket, route) = net::connect_with_headers(
                hosts.quote,
                443,
                "/v2?version=1&codec=1&platform=9",
                &headers,
            )
            .await
            .map_err(|e| Error::Message(e.to_string()))?;
            let mut connection = Connection::new(socket);
            let mut metadata = HashMap::new();
            metadata.insert("accept-language".to_owned(), language.to_owned());
            metadata.insert("need_over_night_quote".to_owned(), "true".to_owned());
            let _session: proto::Session = connection
                .handshake(cmd::AUTH, &proto::AuthRequest { token: otp, metadata })
                .await?;
            let profile: proto::QuoteProfileResponse = connection
                .handshake(
                    cmd::QUOTE_PROFILE,
                    &proto::QuoteProfileRequest { language: language.to_owned() },
                )
                .await?;
            Ok::<_, Error>((connection, Account::from_profile(&profile), route))
        };
        tokio::select! {
            result = timeout(CONNECT_TIMEOUT, login) => match result {
                Ok(Ok(connected)) => Ok(connected),
                Ok(Err(e)) => Err(Connect::Failed(e)),
                Err(_) => Err(Connect::Failed(Error::Message(t!(
                    "error.sourceConnectTimeout",
                    source = PROVIDER.name()
                )))),
            },
            // Nothing else is served while logging in, but the app may quit.
            () = self.app_closing() => Err(Connect::Shutdown),
        }
    }

    async fn app_closing(&self) {
        let mut control = self.control.clone();
        while control.changed().await.is_ok() {}
    }

    /// A token with little time left is swapped for a fresh one, saved for
    /// the next launch. Failing that, the old one serves while it lasts.
    async fn renew(&self, keys: &LongbridgeKeys) -> LongbridgeKeys {
        let Some(expiry) = api::token_expiry(&keys.access_token) else {
            return keys.clone();
        };
        if expiry - api::now() > RENEW_BEFORE {
            return keys.clone();
        }
        match api::refresh_token(keys, api::now() + TOKEN_LIFETIME).await {
            Ok(token) => {
                let renewed = LongbridgeKeys { access_token: token, ..keys.clone() };
                let shared = self.app.state::<Shared>();
                let mut stored = shared.credentials();
                stored.longbridge = Some(renewed.clone());
                if let Err(e) = credentials::save(&shared.credentials_path, &stored) {
                    log::warn!("cannot save the renewed Longbridge token: {e}");
                }
                let credentials = stored.clone();
                drop(stored);
                shared.control.send_modify(|control| control.credentials = credentials);
                log::info!("renewed the Longbridge access token");
                renewed
            }
            Err(e) => {
                log::warn!("cannot renew the Longbridge access token: {e}");
                keys.clone()
            }
        }
    }

    async fn pump(&mut self, mut connection: Connection, mut wanted: Wanted) -> End {
        // Calls queued while offline go out first.
        for (cmd, body, reply) in std::mem::take(&mut self.waiting) {
            if let Err(e) = connection.send(cmd, &body, Pending::Call(reply)).await {
                return End::Lost(e.to_string());
            }
        }
        let mut subscribed: HashMap<String, i32> = HashMap::new();
        if let Err(e) =
            self.sync_subscriptions(&mut connection, &wanted, &mut subscribed, true).await
        {
            return End::Lost(e.to_string());
        }
        for (_, sink) in self.watchers.values() {
            let _ = sink.send(Push::Resumed);
        }

        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let flush = sleep(Duration::ZERO);
        tokio::pin!(flush);
        let mut dirty = false;
        let mut idle_since: Option<Instant> = None;

        loop {
            tokio::select! {
                message = connection.socket.next() => {
                    let frame = match message {
                        Some(Ok(Message::Binary(data))) => parse_frame(&data),
                        Some(Ok(Message::Ping(_))) => {
                            connection.last_ping = Instant::now();
                            continue;
                        }
                        Some(Ok(Message::Close(frame))) => return End::Lost(closed(frame)),
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => return End::Lost(e.to_string()),
                        None => return End::Lost(disconnected()),
                    };
                    match frame {
                        Some(Frame::Response { id, status, body }) => {
                            let settled = connection.inflight.remove(&id).map(|(_, p)| p);
                            if settled.is_some_and(|pending| self.settle(pending, status, body)) && !dirty {
                                dirty = true;
                                flush.as_mut().reset(Instant::now() + COALESCE);
                            }
                        }
                        Some(Frame::Push { cmd, body }) => {
                            if self.dispatch(&mut connection, cmd, &body).await && !dirty {
                                dirty = true;
                                flush.as_mut().reset(Instant::now() + COALESCE);
                            }
                        }
                        None => log::debug!("longbridge: unreadable frame"),
                    }
                }
                command = self.commands.recv() => {
                    let Some(command) = command else { return End::Shutdown };
                    let result = match command {
                        Command::Call { cmd, body, reply } => connection.send(cmd, &body, Pending::Call(reply)).await,
                        Command::Account { reply } => {
                            let _ = reply.send(last_account().ok_or_else(stopped));
                            Ok(())
                        }
                        Command::Watch { id, symbol, sink } => {
                            self.watchers.insert(id, (symbol, sink));
                            self.sync_subscriptions(&mut connection, &wanted, &mut subscribed, false).await
                        }
                        Command::Unwatch { id } => {
                            self.watchers.remove(&id);
                            self.sync_subscriptions(&mut connection, &wanted, &mut subscribed, false).await
                        }
                    };
                    if let Err(e) = result {
                        return End::Lost(e.to_string());
                    }
                }
                changed = self.control.changed() => {
                    if changed.is_err() {
                        return End::Shutdown;
                    }
                    let now = Wanted::of(&self.control.borrow_and_update());
                    if now.keys != wanted.keys {
                        return End::Reconfigure;
                    }
                    if now != wanted {
                        wanted = now;
                        if let Err(e) = self.sync_subscriptions(&mut connection, &wanted, &mut subscribed, false).await {
                            return End::Lost(e.to_string());
                        }
                        // Asleep with nothing else to serve: no need to wait out the grace.
                        if wanted.paused != 0 && !self.needed(&wanted) && connection.inflight.is_empty() {
                            let _ = connection.socket.close(None).await;
                            return End::Idle;
                        }
                    }
                }
                _ = tick.tick() => {
                    if connection.last_ping.elapsed() > HEARTBEAT_TIMEOUT {
                        return End::Lost(t!("error.sourceSilent", source = PROVIDER.name()));
                    }
                    connection.expire_requests();
                    if self.needed(&wanted) || !connection.inflight.is_empty() {
                        idle_since = None;
                    } else if idle_since.get_or_insert_with(Instant::now).elapsed() > IDLE_GRACE {
                        let _ = connection.socket.close(None).await;
                        return End::Idle;
                    }
                }
                () = &mut flush, if dirty => {
                    dirty = false;
                    bar::request_render(&self.app);
                }
            }
        }
    }

    /// Brings the socket's subscriptions in line with what the menu bar and
    /// chart windows need: quotes for the former, quotes, book and trades for
    /// the latter.
    async fn sync_subscriptions(
        &mut self,
        connection: &mut Connection,
        wanted: &Wanted,
        subscribed: &mut HashMap<String, i32>,
        fresh: bool,
    ) -> Result<(), Error> {
        let mut desired: HashMap<String, i32> = HashMap::new();
        if wanted.paused == 0 {
            for symbol in &wanted.symbols {
                *desired.entry(symbol.clone()).or_default() |= 1 << sub_type::QUOTE;
            }
        }
        for (symbol, _) in self.watchers.values() {
            *desired.entry(symbol.clone()).or_default() |=
                1 << sub_type::QUOTE | 1 << sub_type::DEPTH | 1 << sub_type::TRADE;
        }

        let mut dropped: HashMap<i32, Vec<String>> = HashMap::new();
        for (symbol, &had) in subscribed.iter() {
            let keep = desired.get(symbol).copied().unwrap_or(0);
            let flags = had & !keep;
            if flags != 0 {
                dropped.entry(flags).or_default().push(symbol.clone());
            }
        }
        for (flags, symbols) in dropped {
            let request = proto::UnsubscribeRequest {
                symbol: symbols,
                sub_type: sub_types(flags),
                unsub_all: false,
            };
            connection
                .send(cmd::UNSUBSCRIBE, &request.encode_to_vec(), Pending::Ignore("unsubscribe"))
                .await?;
        }

        let mut added: HashMap<i32, Vec<String>> = HashMap::new();
        for (symbol, &want) in &desired {
            let had = subscribed.get(symbol).copied().unwrap_or(0);
            let flags = want & !had;
            if flags != 0 {
                added.entry(flags).or_default().push(symbol.clone());
            }
        }
        for (flags, symbols) in added {
            let request = proto::SubscribeRequest {
                symbol: symbols,
                sub_type: sub_types(flags),
                is_first_push: true,
            };
            connection
                .send(cmd::SUBSCRIBE, &request.encode_to_vec(), Pending::Ignore("subscribe"))
                .await?;
        }
        *subscribed = desired;

        // Quotes missed while asleep are refreshed on waking.
        if wanted.paused != 0 {
            self.tracked.clear();
        }
        // Menu bar symbols need their reference prices before the pushes mean
        // anything: all of them after a (re)connect, new ones otherwise.
        let symbols: Vec<String> = wanted
            .symbols
            .iter()
            .filter(|symbol| fresh || !self.tracked.contains_key(*symbol))
            .cloned()
            .collect();
        self.tracked.retain(|symbol, _| wanted.symbols.contains(symbol));
        if !symbols.is_empty() && wanted.paused == 0 {
            let request = proto::MultiSecurityRequest { symbol: symbols };
            connection.send(cmd::QUOTE, &request.encode_to_vec(), Pending::Snapshot).await?;
        }
        Ok(())
    }

    /// A response arrived for `pending`. Returns whether the menu bar changed.
    fn settle(&mut self, pending: Pending, status: u8, body: Vec<u8>) -> bool {
        let result = if status == 0 { Ok(body) } else { Err(response_error(status, &body)) };
        let mut changed = false;
        match pending {
            Pending::Call(reply) => {
                let _ = reply.send(result);
            }
            Pending::Ignore(what) => {
                if let Err(e) = result {
                    log::warn!("longbridge {what} failed: {e}");
                }
            }
            Pending::Snapshot => match result.and_then(|body| {
                proto::QuoteResponse::decode(body.as_slice()).map_err(|_| bad_data())
            }) {
                Ok(response) => {
                    for quote in response.secu_quote {
                        changed |= self.apply_snapshot(&quote);
                    }
                }
                Err(e) => log::warn!("longbridge quote snapshot failed: {e}"),
            },
        }
        changed
    }

    /// Hands a push to the menu bar and the watching charts. Returns whether
    /// the menu bar changed.
    async fn dispatch(&mut self, connection: &mut Connection, cmd: u8, body: &[u8]) -> bool {
        match cmd {
            cmd::PUSH_QUOTE => {
                let Ok(push) = proto::PushQuote::decode(body) else { return false };
                let changed = self.apply_push(connection, &push).await;
                self.forward(&push.symbol, || Push::Quote(push.clone()));
                changed
            }
            cmd::PUSH_DEPTH => {
                if let Ok(push) = proto::PushDepth::decode(body) {
                    self.forward(&push.symbol, || Push::Depth(push.clone()));
                }
                false
            }
            cmd::PUSH_TRADES => {
                if let Ok(push) = proto::PushTrade::decode(body) {
                    self.forward(&push.symbol, || Push::Trades(push.clone()));
                }
                false
            }
            _ => false,
        }
    }

    fn forward(&mut self, symbol: &str, push: impl Fn() -> Push) {
        self.watchers.retain(|_, (watched, sink)| watched != symbol || sink.send(push()).is_ok());
    }

    /// A menu bar symbol's full quote: the regular session, and the US
    /// extended sessions, whichever traded last. Returns whether it changed.
    fn apply_snapshot(&mut self, quote: &proto::SecurityQuote) -> bool {
        let Some(market) = super::market_of(&quote.symbol) else { return false };
        let (last, reference, session) = super::latest(quote);
        let (regular_last, prev_close) = (decimal(&quote.last_done), decimal(&quote.prev_close));
        let timestamp =
            [&quote.pre_market_quote, &quote.post_market_quote, &quote.over_night_quote]
                .into_iter()
                .flatten()
                .map(|ext| ext.timestamp)
                .fold(quote.timestamp, i64::max);
        self.tracked.insert(
            quote.symbol.clone(),
            Tracked { market, regular_last, prev_close, session, day: day_of(market, timestamp) },
        );
        self.store(&quote.symbol, last, reference, session)
    }

    /// A pushed change. A new session or trading day moves the reference
    /// price, so it asks for a fresh snapshot.
    async fn apply_push(&mut self, connection: &mut Connection, push: &proto::PushQuote) -> bool {
        const END_OF_DAY: i32 = 1;
        let last = decimal(&push.last_done);
        if push.tag == END_OF_DAY || last <= 0.0 {
            return false;
        }
        let Some(tracked) = self.tracked.get_mut(&push.symbol) else { return false };
        let day = day_of(tracked.market, push.timestamp);
        if push.trade_session != tracked.session || day != tracked.day {
            tracked.session = push.trade_session;
            tracked.day = day;
            let request = proto::MultiSecurityRequest { symbol: vec![push.symbol.clone()] };
            if let Err(e) =
                connection.send(cmd::QUOTE, &request.encode_to_vec(), Pending::Snapshot).await
            {
                log::warn!("longbridge snapshot request failed: {e}");
            }
        }
        let reference = if push.trade_session == trade_session::INTRADAY {
            tracked.regular_last = last;
            tracked.prev_close
        } else {
            tracked.regular_last
        };
        let session = push.trade_session;
        self.store(&push.symbol, last, reference, session)
    }

    /// Writes a menu bar quote; returns whether it changed.
    fn store(&self, symbol: &str, last: f64, reference: f64, session: i32) -> bool {
        let quote = Quote { last, open: reference, session: session_of(session) };
        let id = market::instrument_id(PROVIDER, symbol);
        let shared = self.app.state::<Shared>();
        let mut model = shared.model();
        if !model.settings.watchlist.iter().any(|i| i.provider == PROVIDER && i.symbol == symbol) {
            return false;
        }
        model.quotes.insert(id, quote) != Some(quote)
    }
}

enum Connect {
    Failed(Error),
    Shutdown,
}

enum Pending {
    Call(Reply),
    /// The menu bar's quote snapshot.
    Snapshot,
    /// Subscriptions: only failures matter, and only for the log.
    Ignore(&'static str),
}

struct Connection {
    socket: net::Socket,
    next_id: u32,
    inflight: HashMap<u32, (Instant, Pending)>,
    last_ping: Instant,
}

impl Connection {
    fn new(socket: net::Socket) -> Self {
        Self { socket, next_id: 0, inflight: HashMap::new(), last_ping: Instant::now() }
    }

    async fn send(&mut self, cmd: u8, body: &[u8], pending: Pending) -> Result<(), Error> {
        self.next_id = self.next_id.wrapping_add(1);
        let frame = request_frame(cmd, self.next_id, body);
        self.inflight.insert(self.next_id, (Instant::now() + REQUEST_TIMEOUT, pending));
        self.socket
            .send(Message::Binary(frame.into()))
            .await
            .map_err(|e| Error::Message(e.to_string()))
    }

    /// During login nothing else runs: send, then read until the answer.
    async fn handshake<Req: prost::Message, Resp: prost::Message + Default>(
        &mut self,
        cmd: u8,
        request: &Req,
    ) -> Result<Resp, Error> {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        let frame = request_frame(cmd, id, &request.encode_to_vec());
        self.socket
            .send(Message::Binary(frame.into()))
            .await
            .map_err(|e| Error::Message(e.to_string()))?;
        loop {
            let data = match self.socket.next().await {
                Some(Ok(Message::Binary(data))) => data,
                Some(Ok(Message::Close(frame))) => return Err(Error::Message(closed(frame))),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(Error::Message(e.to_string())),
                None => return Err(Error::Message(disconnected())),
            };
            if let Some(Frame::Response { id: answered, status, body }) = parse_frame(&data)
                && answered == id
            {
                if status != 0 {
                    return Err(response_error(status, &body));
                }
                return Resp::decode(body.as_slice()).map_err(|_| bad_data());
            }
        }
    }

    fn expire_requests(&mut self) {
        let now = Instant::now();
        let expired: Vec<u32> =
            self.inflight.iter().filter(|(_, (at, _))| *at < now).map(|(id, _)| *id).collect();
        for id in expired {
            if let Some((_, Pending::Call(reply))) = self.inflight.remove(&id) {
                let timeout = t!("error.sourceTimeout", source = PROVIDER.name());
                let _ = reply.send(Err(Error::Message(timeout)));
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        for (_, (_, pending)) in self.inflight.drain() {
            if let Pending::Call(reply) = pending {
                let _ = reply.send(Err(Error::Message(disconnected())));
            }
        }
    }
}

fn request_frame(cmd: u8, id: u32, body: &[u8]) -> Vec<u8> {
    const REQUEST: u8 = 1;
    let timeout_ms = REQUEST_TIMEOUT.as_millis() as u16;
    let mut frame = Vec::with_capacity(11 + body.len());
    frame.push(REQUEST);
    frame.push(cmd);
    frame.extend_from_slice(&id.to_be_bytes());
    frame.extend_from_slice(&timeout_ms.to_be_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    frame.extend_from_slice(body);
    frame
}

enum Frame {
    Response { id: u32, status: u8, body: Vec<u8> },
    Push { cmd: u8, body: Vec<u8> },
}

fn parse_frame(data: &[u8]) -> Option<Frame> {
    const RESPONSE: u8 = 2;
    const PUSH: u8 = 3;
    const GZIP: u8 = 0x20;
    let (&header, rest) = data.split_first()?;
    let length = |bytes: &[u8]| -> Option<usize> {
        let [a, b, c] = bytes.get(..3)?.try_into().ok()?;
        Some(u32::from_be_bytes([0, a, b, c]) as usize)
    };
    let (frame, body): (Frame, &[u8]) = match header & 0x0f {
        RESPONSE => {
            let id = u32::from_be_bytes(rest.get(1..5)?.try_into().ok()?);
            let status = *rest.get(5)?;
            let len = length(rest.get(6..)?)?;
            (Frame::Response { id, status, body: Vec::new() }, rest.get(9..9 + len)?)
        }
        PUSH => {
            let len = length(rest.get(1..)?)?;
            (Frame::Push { cmd: *rest.first()?, body: Vec::new() }, rest.get(4..4 + len)?)
        }
        _ => return None,
    };
    let body = if header & GZIP != 0 {
        let mut plain = Vec::new();
        GzDecoder::new(body).read_to_end(&mut plain).ok()?;
        plain
    } else {
        body.to_vec()
    };
    Some(match frame {
        Frame::Response { id, status, .. } => Frame::Response { id, status, body },
        Frame::Push { cmd, .. } => Frame::Push { cmd, body },
    })
}

fn response_error(status: u8, body: &[u8]) -> Error {
    match proto::Error::decode(body) {
        Ok(error) if !error.msg.is_empty() => {
            Error::Message(t!("error.sourceSays", source = PROVIDER.name(), message = error.msg))
        }
        Ok(error) => {
            Error::Message(t!("error.sourceCode", source = PROVIDER.name(), code = error.code))
        }
        Err(_) => {
            Error::Message(t!("error.sourceStatus", source = PROVIDER.name(), status = status))
        }
    }
}

fn sub_types(flags: i32) -> Vec<i32> {
    [sub_type::QUOTE, sub_type::DEPTH, sub_type::TRADE]
        .into_iter()
        .filter(|t| flags & (1 << t) != 0)
        .collect()
}

fn decimal(text: &str) -> f64 {
    text.parse().unwrap_or(0.0)
}

fn day_of(market: Market, epoch: i64) -> i64 {
    let t = market.local(epoch);
    t.year * 10_000 + i64::from(t.month) * 100 + i64::from(t.day)
}

fn session_of(session: i32) -> Option<Session> {
    match session {
        trade_session::PRE => Some(Session::Pre),
        trade_session::POST => Some(Session::Post),
        trade_session::OVERNIGHT => Some(Session::Overnight),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let frame = request_frame(cmd::QUOTE, 7, b"abc");
        assert_eq!(frame, [1, 11, 0, 0, 0, 7, 0x3a, 0x98, 0, 0, 3, b'a', b'b', b'c']);

        let response = [2, 11, 0, 0, 0, 7, 0, 0, 0, 2, b'o', b'k'];
        let Some(Frame::Response { id, status, body }) = parse_frame(&response) else { panic!() };
        assert_eq!((id, status, body.as_slice()), (7, 0, b"ok".as_slice()));

        let push = [3, 101, 0, 0, 1, b'x'];
        let Some(Frame::Push { cmd, body }) = parse_frame(&push) else { panic!() };
        assert_eq!((cmd, body.as_slice()), (101, b"x".as_slice()));
        assert!(parse_frame(&[3, 101, 0, 0, 9, b'x']).is_none());
    }
}
