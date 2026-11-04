//! The user's own Tonghuashun watchlist, through Tonghuashun's client
//! sign-in (thsdk's runtime, `thsdk.rs`). The settings window shows a QR
//! code from Tonghuashun, scanned and confirmed in the Tonghuashun app; the
//! session that leaves is the runtime's own file in the app's config folder
//! (`thsdk/account.session`, good for about a month). With it the watchlist
//! and its groups are read, never written.
//!
//! Entries come as Tonghuashun's codes with its client's market codes
//! (`USHA`, `USZJ`…). They are matched against the data API's code table
//! ([`api::listings`]), so their quotes come the way the app's watchlist's
//! do (`quotes.rs`); Hong Kong and US stocks, futures and the like have no
//! match there and are only counted.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use base64::Engine as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::{
    sync::{oneshot, watch},
    time::sleep,
};

use super::{
    ID, Kind, QUOTE, api, code,
    thsdk::{Failure, Runtime},
    trimmed,
};
use crate::{
    bar, credentials, http,
    market::Error,
    model::{FeedControl, Instrument, Shared, SyncStatus, Synced, SyncedGroup},
    window,
};

/// The watchlist changes when the user edits it in Tonghuashun's apps.
const SYNC_EVERY: Duration = Duration::from_secs(600);
const RETRY: Duration = Duration::from_secs(60);
/// How long a QR code waits to be scanned and confirmed.
const QR_TIMEOUT: Duration = Duration::from_secs(180);
const QR_POLL: Duration = Duration::from_secs(1);
/// A call's longest wait for Tonghuashun.
const CALL: Duration = Duration::from_secs(30);

/// One helper at a time: they share the session file.
static HELPER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Where thsdk's runtime keeps the session: the config folder's `thsdk`,
/// readable by the user only.
fn session_dir(app: &AppHandle) -> Result<PathBuf, Error> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| Error::Message(t!("error.noConfigDir").to_owned()))?
        .join("thsdk");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&dir).map_err(|e| Error::Message(t!("error.saveFailed", error = e)))?;
    Ok(dir)
}

/// An entry of the watchlist: Tonghuashun's code and its client's market.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
struct Entry {
    code: String,
    #[serde(default)]
    market: String,
}

#[derive(Default, Deserialize)]
struct List {
    #[serde(default, deserialize_with = "or_empty")]
    securities: Vec<Entry>,
}

#[derive(Default, Deserialize)]
struct Groups {
    /// The groups' order in the user's apps; ids can repeat.
    #[serde(default, deserialize_with = "or_empty")]
    group_order: Vec<i64>,
    #[serde(default, deserialize_with = "or_empty")]
    groups: HashMap<String, Group>,
}

#[derive(Deserialize)]
struct Group {
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default, deserialize_with = "or_empty")]
    securities: Vec<Entry>,
}

/// The library is Go, which writes a list or map it never filled (an empty
/// watchlist, an empty group) as `null`.
fn or_empty<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl Groups {
    /// The groups as the user orders them, any the order misses after.
    fn ordered(mut self) -> Vec<Group> {
        let mut ordered = Vec::new();
        for id in self.group_order {
            if let Some(group) = self.groups.remove(&id.to_string()) {
                ordered.push(group);
            }
        }
        let mut rest: Vec<Group> = self.groups.into_values().collect();
        rest.sort_by_key(|group| group.id);
        ordered.extend(rest);
        ordered
    }
}

enum Read {
    /// No session, or it ran out.
    SignedOut,
    Failed(Error),
}

impl From<Failure> for Read {
    fn from(failure: Failure) -> Self {
        if failure.refused("session_unavailable") || failure.refused("not_authenticated") {
            Read::SignedOut
        } else {
            Read::Failed(failure.into())
        }
    }
}

/// The watchlist and its groups, as the account has them.
async fn read(app: &AppHandle) -> Result<(Vec<Entry>, Vec<Group>), Read> {
    let dir = session_dir(app).map_err(Read::Failed)?;
    let _helper = HELPER.lock().await;
    let mut runtime = Runtime::start().map_err(Read::Failed)?;
    let auth = runtime.call("auth", Some(json!({ "session_dir": dir })), CALL).await?;
    if auth.get("authenticated") != Some(&Value::Bool(true)) {
        return Err(Read::SignedOut);
    }
    let list: List = parse(runtime.call("get_account_watchlist", None, CALL).await?)?;
    let groups: Groups = parse(runtime.call("get_account_watchlist_groups", None, CALL).await?)?;
    Ok((list.securities, groups.ordered()))
}

/// An answer's data as `T`; no data at all, as Go writes a nil answer, is empty.
fn parse<T: DeserializeOwned + Default>(value: Value) -> Result<T, Read> {
    serde_json::from_value::<Option<T>>(value).map(Option::unwrap_or_default).map_err(|e| {
        log::warn!("tonghuashun watchlist: {e}");
        Read::Failed(Error::Message(t!("error.sourceBadData", source = ID.name())))
    })
}

/// What the data API calls an entry, resolved once each: the code table is
/// only fetched for entries not seen before, and let go after.
#[derive(Default)]
struct Resolved(HashMap<Entry, Option<(Instrument, Kind)>>);

impl Resolved {
    async fn resolve<'a>(&mut self, entries: impl Iterator<Item = &'a Entry>) -> Result<(), Error> {
        let new: Vec<&Entry> = entries.filter(|e| !self.0.contains_key(*e)).collect();
        if new.is_empty() {
            return Ok(());
        }
        let listings = api::listings().await?;
        let table = Table::new(&listings);
        for entry in new {
            let found = table.find(entry).map(|listing| (instrument(listing), listing.kind));
            self.0.insert(entry.clone(), found);
        }
        Ok(())
    }

    /// The instruments of `entries` that resolved, each once, in order.
    fn instruments(&self, entries: &[Entry]) -> Vec<Instrument> {
        let mut instruments: Vec<Instrument> = Vec::new();
        for entry in entries {
            if let Some(Some((instrument, kind))) = self.0.get(entry)
                && !instruments.iter().any(|i| i.symbol == instrument.symbol)
            {
                api::remember(&instrument.symbol, *kind);
                instruments.push(instrument.clone());
            }
        }
        instruments
    }

    /// How many different entries nothing here quotes.
    fn unmatched<'a>(&self, entries: impl Iterator<Item = &'a Entry>) -> usize {
        let mut unmatched: Vec<&Entry> =
            entries.filter(|e| matches!(self.0.get(*e), Some(None))).collect();
        unmatched.sort_by(|a, b| (&a.code, &a.market).cmp(&(&b.code, &b.market)));
        unmatched.dedup();
        unmatched.len()
    }
}

/// The code table, by Tonghuashun's own codes and by the data API's.
struct Table<'a> {
    tickers: HashMap<&'a str, &'a api::Listing>,
    symbols: HashMap<&'a str, &'a api::Listing>,
}

impl<'a> Table<'a> {
    fn new(listings: &'a [api::Listing]) -> Self {
        Self {
            tickers: listings.iter().map(|l| (l.ticker.as_str(), l)).collect(),
            symbols: listings.iter().map(|l| (l.symbol.as_str(), l)).collect(),
        }
    }

    /// Tonghuashun's codes are unique across the table (an index can have
    /// one of its own, `1A0001` for `000001.SH`); one written as the
    /// exchange writes it is found by its market instead.
    fn find(&self, entry: &Entry) -> Option<&'a api::Listing> {
        let by_market = exchange(&entry.market)
            .and_then(|market| self.symbols.get(format!("{}.{market}", entry.code).as_str()));
        by_market.or_else(|| self.tickers.get(entry.code.as_str())).copied()
    }
}

/// The exchange of one of Tonghuashun's client markets: `USHA` (Shanghai
/// shares), `USHJ` (its funds), `USZA`, `USZJ` (Shenzhen's)…
fn exchange(market: &str) -> Option<&'static str> {
    if market.starts_with("USH") {
        Some("SH")
    } else if market.starts_with("USZ") {
        Some("SZ")
    } else {
        None
    }
}

fn instrument(listing: &api::Listing) -> Instrument {
    Instrument {
        provider: ID,
        symbol: listing.symbol.clone(),
        base: code(&listing.symbol).to_owned(),
        quote: QUOTE.to_owned(),
        name: listing.name.as_deref().and_then(trimmed),
        decimals: Some(listing.kind.decimals()),
        pinned: false,
        venue: None,
    }
}

/// What the watchlist sync depends on.
#[derive(Clone, PartialEq)]
struct Wanted {
    signed_in: bool,
    /// Quotes need the data API's key.
    key: bool,
    paused: bool,
}

impl Wanted {
    fn of(control: &FeedControl) -> Self {
        Self {
            signed_in: control.credentials.ths_signed_in,
            key: control.credentials.ths.is_some(),
            paused: control.paused != 0,
        }
    }
}

/// Keeps the user's Tonghuashun watchlist current while signed in there:
/// read every [`SYNC_EVERY`], not at all while nobody can see it, and no
/// more once the session has run out.
pub async fn run(app: AppHandle, mut control: watch::Receiver<FeedControl>) {
    let mut resolved = Resolved::default();
    let mut failures: u32 = 0;
    loop {
        let wanted = Wanted::of(&control.borrow_and_update());
        let delay = if !wanted.signed_in {
            resolved = Resolved::default();
            failures = 0;
            publish(&app, |synced| *synced = None);
            None
        } else if wanted.paused {
            None
        } else if !wanted.key {
            let error = t!("error.sourceNeedsCredentials", source = ID.name());
            status(&app, SyncStatus::Failed(error));
            None
        } else {
            match sync(&app, &mut resolved).await {
                Ok(()) => {
                    failures = 0;
                    Some(SYNC_EVERY)
                }
                Err(Read::SignedOut) => {
                    status(&app, SyncStatus::SignedOut);
                    None
                }
                Err(Read::Failed(e)) => {
                    log::warn!("tonghuashun watchlist: {e}");
                    status(&app, SyncStatus::Failed(e.to_string()));
                    failures += 1;
                    Some((RETRY * (1 << (failures - 1).min(3))).min(SYNC_EVERY))
                }
            }
        };
        let changed = changed(&mut control, &wanted);
        let ended = match delay {
            Some(delay) => tokio::select! {
                () = sleep(delay) => false,
                result = changed => result.is_err(),
            },
            None => changed.await.is_err(),
        };
        if ended {
            return;
        }
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

async fn sync(app: &AppHandle, resolved: &mut Resolved) -> Result<(), Read> {
    if app.state::<Shared>().model().synced.is_none() {
        publish(app, |synced| *synced = Some(Synced::default()));
    }
    let (list, groups) = read(app).await?;
    let all = || list.iter().chain(groups.iter().flat_map(|g| &g.securities));
    resolved.resolve(all()).await.map_err(Read::Failed)?;
    let synced = Synced {
        instruments: resolved.instruments(&list),
        groups: groups
            .iter()
            .map(|group| SyncedGroup {
                name: group.name.clone(),
                instruments: resolved.instruments(&group.securities),
            })
            .filter(|group| !group.instruments.is_empty())
            .collect(),
        skipped: resolved.unmatched(all()),
        updated: Some(crate::portfolio::now_ms()),
        status: SyncStatus::Read,
    };
    publish(app, |current| *current = Some(synced));
    Ok(())
}

fn status(app: &AppHandle, status: SyncStatus) {
    publish(app, |synced| synced.get_or_insert_with(Synced::default).status = status);
}

/// Changes the synced watchlist, and with it the codes quoted for it, the
/// dropdown and the settings window.
fn publish(app: &AppHandle, change: impl FnOnce(&mut Option<Synced>)) {
    let shared = app.state::<Shared>();
    let symbols = {
        let mut model = shared.model();
        let before = model.synced.clone();
        change(&mut model.synced);
        if model.synced == before {
            return;
        }
        let mut symbols: Vec<String> = model
            .synced
            .iter()
            .flat_map(Synced::all)
            .map(|instrument| instrument.symbol.clone())
            .collect();
        symbols.sort();
        symbols.dedup();
        // Quotes of codes that left the watchlist go with them.
        let kept: Vec<String> =
            model.quotes.keys().filter(|id| model.instrument(id).is_some()).cloned().collect();
        model.quotes.retain(|id, _| kept.contains(id));
        symbols
    };
    shared.control.send_if_modified(|control| {
        let changed = control.ths_synced != symbols;
        control.ths_synced = symbols;
        changed
    });
    bar::request_render(app);
    window::emit_ths(app);
}

/// Records whether the user is signed in, and lets everything that reads
/// the watchlist know.
fn keep(app: &AppHandle, signed_in: bool) {
    let shared = app.state::<Shared>();
    let credentials = {
        let mut stored = shared.credentials();
        stored.ths_signed_in = signed_in;
        if let Err(e) = credentials::save(&shared.credentials_path, &stored) {
            log::error!("cannot save the Tonghuashun sign-in: {e}");
        }
        stored.clone()
    };
    shared.control.send_modify(|control| control.credentials = credentials);
    window::emit_ths(app);
}

/// Signs out: the session ends and its file goes; the watchlist leaves the
/// dropdown.
pub fn sign_out(app: &AppHandle) {
    cancel_sign_in();
    keep(app, false);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(dir) = session_dir(&app) else { return };
        let _helper = HELPER.lock().await;
        if let Ok(mut runtime) = Runtime::start() {
            let params = Some(json!({ "session_dir": dir }));
            if let Err(e) = runtime.call("logout", params, CALL).await {
                log::warn!("tonghuashun sign-out: {e:?}");
            }
        }
        let _ = std::fs::remove_file(dir.join("account.session"));
    });
}

/// A sign-in under way, as the settings window shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignIn {
    /// The QR code to scan in the Tonghuashun app, a PNG data URL; none
    /// until it has arrived.
    pub qr: Option<String>,
    pub state: SignInState,
    /// Why it failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignInState {
    /// Waiting for the code, then for it to be scanned and confirmed.
    Waiting,
    /// The code ran out unscanned; another one takes asking again.
    Expired,
    Failed,
}

struct Job {
    view: SignIn,
    /// Sending (or dropping) it stops the sign-in.
    cancel: Option<oneshot::Sender<()>>,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);

fn job() -> std::sync::MutexGuard<'static, Option<Job>> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// The sign-in under way or how the last one ended, if any.
pub fn sign_in_view() -> Option<SignIn> {
    job().as_ref().map(|job| job.view.clone())
}

/// Starts signing in with a QR code, unless that is already under way.
pub fn sign_in(app: &AppHandle) {
    let (cancel, cancelled) = oneshot::channel();
    {
        let mut job = job();
        if job.as_ref().is_some_and(|job| job.view.state == SignInState::Waiting) {
            return;
        }
        *job = Some(Job {
            view: SignIn { qr: None, state: SignInState::Waiting, error: None },
            cancel: Some(cancel),
        });
    }
    window::emit_ths(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let ended = scan(&app, cancelled).await;
        let signed_in = matches!(ended, Scan::SignedIn);
        {
            let mut job = job();
            match ended {
                Scan::SignedIn | Scan::Cancelled => *job = None,
                Scan::Expired => {
                    if let Some(job) = job.as_mut() {
                        (job.view.state, job.cancel) = (SignInState::Expired, None);
                    }
                }
                Scan::Failed(e) => {
                    if let Some(job) = job.as_mut() {
                        job.view = SignIn {
                            qr: None,
                            state: SignInState::Failed,
                            error: Some(e.to_string()),
                        };
                        job.cancel = None;
                    }
                }
            }
        }
        if signed_in {
            keep(&app, true);
        } else {
            window::emit_ths(&app);
        }
    });
}

/// Stops a sign-in under way, and forgets how the last one ended.
pub fn cancel_sign_in() {
    if let Some(mut job) = job().take()
        && let Some(cancel) = job.cancel.take()
    {
        let _ = cancel.send(());
    }
}

enum Scan {
    SignedIn,
    Cancelled,
    Expired,
    Failed(Error),
}

/// thsdk's QR sign-in: a code, then its state every [`QR_POLL`] until it is
/// confirmed in the app, runs out, or the user stops it. A session still
/// good needs no code at all.
async fn scan(app: &AppHandle, mut cancelled: oneshot::Receiver<()>) -> Scan {
    let dir = match session_dir(app) {
        Ok(dir) => dir,
        Err(e) => return Scan::Failed(e),
    };
    let _helper = HELPER.lock().await;
    let mut runtime = match Runtime::start() {
        Ok(runtime) => runtime,
        Err(e) => return Scan::Failed(e),
    };
    let params = json!({
        "session_dir": dir,
        "timeout_seconds": QR_TIMEOUT.as_secs(),
        "poll_interval_ms": QR_POLL.as_millis() as u64,
    });
    let deadline = Instant::now() + QR_TIMEOUT;
    let mut state = runtime.call("auth_qrcode", Some(params), Duration::from_secs(20)).await;
    let mut shown = String::new();
    loop {
        let challenge: Challenge = match state {
            Ok(value) => serde_json::from_value(value).unwrap_or_default(),
            Err(e) => return Scan::Failed(e.into()),
        };
        if challenge.authenticated || challenge.state == "authenticated" {
            return Scan::SignedIn;
        }
        if ENDED.contains(&challenge.state.as_str()) || Instant::now() >= deadline {
            return Scan::Expired;
        }
        if !challenge.image_url.is_empty() && challenge.image_url != shown {
            match qr_image(&challenge.image_url).await {
                Ok(qr) => {
                    shown = challenge.image_url;
                    if let Some(job) = job().as_mut() {
                        job.view.qr = Some(qr);
                    }
                    window::emit_ths(app);
                }
                Err(e) => log::warn!("tonghuashun QR code: {e}"),
            }
        }
        tokio::select! {
            _ = &mut cancelled => {
                let stop = Some(json!({ "cancel": true }));
                let _ = runtime.call("auth_qrcode", stop, Duration::from_secs(5)).await;
                return Scan::Cancelled;
            }
            () = sleep(QR_POLL) => {}
        }
        state = runtime.call("auth_qrcode", None, Duration::from_secs(10)).await;
    }
}

/// States in which a QR code no longer leads anywhere.
const ENDED: [&str; 5] = ["expired", "timeout", "cancelled", "canceled", "failed"];

/// The state of a QR sign-in, as thsdk reports it.
#[derive(Default, Deserialize)]
struct Challenge {
    #[serde(default)]
    authenticated: bool,
    /// `created`, `waiting`, …, `authenticated`.
    #[serde(default)]
    state: String,
    /// Tonghuashun's picture of the code.
    #[serde(default)]
    image_url: String,
}

/// The code's picture as a data URL the settings page can show.
async fn qr_image(url: &str) -> Result<String, Error> {
    let response = http::send(|client| client.get(url)).await?;
    if !response.status().is_success() {
        return Err(http::Error::Status(response.status().as_u16()).into());
    }
    let bytes = response.bytes().await.map_err(http::Error::from)?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// The login as the settings window shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    /// Codes in the watchlist, groups shown, and entries nothing here quotes.
    pub count: usize,
    pub groups: usize,
    pub skipped: usize,
    /// Epoch milliseconds of the last good read.
    pub updated: Option<f64>,
    pub state: SyncState,
    /// Why the last read failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    Reading,
    Read,
    SignedOut,
    Failed,
}

pub fn view(synced: Option<&Synced>) -> AccountView {
    let status = synced.map(|s| &s.status);
    AccountView {
        count: synced.map_or(0, |s| s.instruments.len()),
        groups: synced.map_or(0, |s| s.groups.len()),
        skipped: synced.map_or(0, |s| s.skipped),
        updated: synced.and_then(|s| s.updated),
        state: match status {
            None | Some(SyncStatus::Reading) => SyncState::Reading,
            Some(SyncStatus::Read) => SyncState::Read,
            Some(SyncStatus::SignedOut) => SyncState::SignedOut,
            Some(SyncStatus::Failed(_)) => SyncState::Failed,
        },
        error: match status {
            Some(SyncStatus::Failed(error)) => Some(error.clone()),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(symbol: &str, ticker: &str, kind: Kind) -> api::Listing {
        api::Listing { symbol: symbol.into(), ticker: ticker.into(), name: None, kind }
    }

    fn entry(code: &str, market: &str) -> Entry {
        Entry { code: code.into(), market: market.into() }
    }

    // upstream: the code table writes the Shanghai Composite as `000001.SH`
    // with Tonghuashun's own code `1A0001`; the account's entries come with
    // client markets such as `USHA`, `USHJ`, `USZA`, `USZJ` (2026-10-28).
    #[test]
    fn entries_match_by_tonghuashun_code_or_by_market() {
        let listings = [
            listing("000001.SH", "1A0001", Kind::Index),
            listing("000001.SZ", "000001", Kind::Stock),
            listing("885525.TI", "885525", Kind::Index),
            listing("515030.SH", "515030", Kind::Etf),
        ];
        let table = Table::new(&listings);
        let found = |code, market| table.find(&entry(code, market)).map(|l| l.symbol.as_str());
        assert_eq!(found("1A0001", "USHI"), Some("000001.SH"));
        assert_eq!(found("000001", "USZA"), Some("000001.SZ"));
        // An index written as the exchange writes it.
        assert_eq!(found("000001", "USHI"), Some("000001.SH"));
        assert_eq!(found("885525", "URFI"), Some("885525.TI"));
        assert_eq!(found("515030", "USHJ"), Some("515030.SH"));
        assert_eq!(found("00700", "UHKM"), None);
    }

    // upstream: `get_account_watchlist_groups` on 2026-10-28, whose order
    // repeats an id.
    #[test]
    fn groups_follow_the_users_order_once_each() {
        let groups: Groups = serde_json::from_str(
            r#"{"version":286,"group_order":[36,35,36],"groups":{
                "35":{"id":35,"name":"算力芯片","securities":[{"code":"688256","market":"USHA"}],"version":277},
                "36":{"id":36,"name":"科技ETF","securities":[{"code":"515030","market":"USHJ"}],"version":276},
                "37":{"id":37,"name":"其他","securities":[],"version":1}}}"#,
        )
        .unwrap();
        let names: Vec<String> = groups.ordered().into_iter().map(|g| g.name).collect();
        assert_eq!(names, ["科技ETF", "算力芯片", "其他"]);
    }

    // upstream: `get_account_watchlist_groups` on 2026-10-28 for an account
    // with empty groups, each `"securities": null`; hq 2.0.2 tags
    // `securities`, `group_order` and `groups` without `omitempty`, and Go
    // writes a nil slice, map or answer as null.
    #[test]
    fn go_nulls_read_as_empty() {
        let list: Option<List> = parse(json!({ "securities": null, "version": 0 })).ok();
        assert!(list.is_some_and(|list| list.securities.is_empty()));
        let nothing: Option<List> = parse(Value::Null).ok();
        assert!(nothing.is_some_and(|list| list.securities.is_empty()));
        let groups: Option<Groups> = parse(json!({
            "group_order": [35],
            "groups": { "35": { "id": 35, "name": "空的", "securities": null } }
        }))
        .ok();
        let groups = groups.map(Groups::ordered).unwrap_or_default();
        assert!(groups.len() == 1 && groups[0].securities.is_empty());
        let none: Option<Groups> = parse(json!({ "group_order": null, "groups": null })).ok();
        assert!(none.is_some_and(|groups| groups.ordered().is_empty()));
    }

    // upstream: `auth_qrcode` while waiting, and `auth` without a session,
    // on 2026-10-28.
    #[test]
    fn sign_in_answers_parse() {
        let waiting: Challenge = serde_json::from_str(
            r#"{"authenticated":false,"image_url":"https://upass.10jqka.com.cn/scan/creatImg?qrid=usk_1","scan_url":"http://mobile.10jqka.com.cn/?qrid=usk_1&source=PC","state":"waiting"}"#,
        )
        .unwrap();
        assert!(!waiting.authenticated && waiting.state == "waiting");
        assert!(waiting.image_url.ends_with("qrid=usk_1"));
        let failure = Failure::Refused(
            serde_json::from_str(r#"{"code":"session_unavailable","message":"no session"}"#)
                .unwrap(),
        );
        assert!(matches!(Read::from(failure), Read::SignedOut));
    }
}
