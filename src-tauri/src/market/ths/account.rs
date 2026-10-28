//! The user's own Tonghuashun watchlist, through the site's web login. A
//! window shows Tonghuashun's own sign-in page (its QR code scanned with the
//! Tonghuashun app, or any other way the page offers); the cookies it leaves
//! are what the site's own pages send, and all the app keeps. It only reads:
//! nothing is ever written back to the account. A login lasts about a month,
//! then the user signs in again.
//!
//! The watchlist comes as Tonghuashun's own codes with a market id. They are
//! matched against the data API's code table ([`api::listings`]), so their
//! quotes come the way the app's watchlist's do (`quotes.rs`); Hong Kong and
//! US stocks, futures and the like have no match there and are only counted.

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Deserializer, Serialize};
use tauri::{AppHandle, Manager, Url};
use tokio::{sync::watch, time::sleep};

use super::{ID, Kind, QUOTE, api, code, trimmed};
use crate::{
    bar,
    credentials::{self, ThsAccount},
    http,
    market::Error,
    model::{FeedControl, Instrument, Shared, SyncStatus, Synced},
    window,
};

/// Tonghuashun's sign-in panel, as the site's community pages embed it
/// (QR code, password or text message); signed in, it lands on their
/// sign-in landing page, on the host the watchlist is read from.
pub const SIGN_IN: &str = "https://upass.10jqka.com.cn/login?isIframe=1&main=3&pannel=1&source=sns\
                           &redir=https://t.10jqka.com.cn/static/loginBack.html";
/// The watchlist as the site's own pages read it; the cookies sent are this
/// URL's.
const WATCHLIST: &str = "https://t.10jqka.com.cn/newcircle/group/getSelfStockWithMarket/";
/// The site serves browsers.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
                          (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
/// The watchlist changes when the user edits it in Tonghuashun's apps.
const SYNC_EVERY: Duration = Duration::from_secs(600);
const RETRY: Duration = Duration::from_secs(60);
/// The `errorCode` of a request without a valid login.
const SIGNED_OUT: i64 = -1;
/// Cookies that make the login, whose expiry is the login's.
const LOGIN_COOKIES: [&str; 4] = ["userid", "ticket", "sess_tk", "user"];

/// An entry of the watchlist: Tonghuashun's code and market id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
struct Entry {
    code: String,
    #[serde(deserialize_with = "text")]
    marketid: String,
}

/// A string, or a number written without quotes.
fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    })
}

/// `{"errorCode": 0, "errorMsg": "", "result": [{"code": …, "marketid": …}]}`
#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "errorCode")]
    error_code: i64,
    #[serde(rename = "errorMsg", default)]
    error_msg: String,
    #[serde(default)]
    result: Option<Vec<Entry>>,
}

enum Failure {
    /// The login ran out.
    SignedOut,
    Failed(Error),
}

async fn read(account: &ThsAccount) -> Result<Vec<Entry>, Failure> {
    let cookie = account
        .cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    let headers = [
        ("Cookie", cookie.as_str()),
        ("User-Agent", USER_AGENT),
        ("Referer", "https://t.10jqka.com.cn/"),
    ];
    let answer: Answer =
        http::get_json_with(WATCHLIST, &headers).await.map_err(|e| Failure::Failed(e.into()))?;
    match answer.error_code {
        0 => Ok(answer.result.unwrap_or_default()),
        SIGNED_OUT => Err(Failure::SignedOut),
        code if answer.error_msg.is_empty() => Err(Failure::Failed(Error::Message(t!(
            "error.sourceCode",
            source = ID.name(),
            code = code
        )))),
        _ => Err(Failure::Failed(Error::Message(t!(
            "error.sourceSays",
            source = ID.name(),
            message = answer.error_msg
        )))),
    }
}

/// The login a sign-in left, from its cookies as `(name, value, expires)`:
/// none until the site has set the user's id. The name shown is the one
/// the site keeps, else the id.
fn account_of(cookies: &[(String, String, Option<i64>)]) -> Option<ThsAccount> {
    let set = |value: &str| !value.is_empty() && value != "deleted";
    let get = |name: &str| {
        cookies.iter().find(|(n, value, _)| n == name && set(value)).map(|(_, value, _)| value)
    };
    let userid = get("userid")?;
    let user = ["u_name", "escapename"]
        .into_iter()
        .find_map(|name| get(name).map(|value| unescape(value)))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| userid.clone());
    let expires = cookies
        .iter()
        .filter(|(name, value, _)| LOGIN_COOKIES.contains(&name.as_str()) && set(value))
        .filter_map(|(_, _, expires)| *expires)
        .min();
    let cookies = cookies
        .iter()
        .filter(|(_, value, _)| set(value))
        .map(|(name, value, _)| (name.clone(), value.clone()))
        .collect();
    Some(ThsAccount { cookies, user, expires })
}

/// A name as the site's cookies write it: JavaScript's `escape` (`%uXXXX`
/// beyond Latin-1) or percent-encoded UTF-8.
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut bytes = Vec::new();
    let mut rest = text;
    let flush = |bytes: &mut Vec<u8>, out: &mut String| {
        match String::from_utf8(std::mem::take(bytes)) {
            Ok(text) => out.push_str(&text),
            // Latin-1, as `escape` writes it.
            Err(e) => out.extend(e.into_bytes().into_iter().map(char::from)),
        }
    };
    while let Some(at) = rest.find('%') {
        if !bytes.is_empty() && at > 0 {
            flush(&mut bytes, &mut out);
        }
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let wide = tail.strip_prefix('u').and_then(|hex| hex.get(..4));
        if let Some(c) =
            wide.and_then(|hex| u32::from_str_radix(hex, 16).ok()).and_then(char::from_u32)
        {
            flush(&mut bytes, &mut out);
            out.push(c);
            rest = &tail[5..];
        } else if let Some(byte) = tail.get(..2).and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
            bytes.push(byte);
            rest = &tail[2..];
        } else {
            flush(&mut bytes, &mut out);
            out.push('%');
            rest = tail;
        }
    }
    flush(&mut bytes, &mut out);
    out.push_str(rest);
    out
}

/// What the data API calls an entry, resolved once each: the code table is
/// only fetched for entries not seen before, and let go after.
#[derive(Default)]
struct Resolved(HashMap<Entry, Option<(Instrument, Kind)>>);

impl Resolved {
    async fn instruments(&mut self, entries: &[Entry]) -> Result<(Vec<Instrument>, usize), Error> {
        let new: Vec<&Entry> = entries.iter().filter(|e| !self.0.contains_key(*e)).collect();
        if !new.is_empty() {
            let listings = api::listings().await?;
            let table = Table::new(&listings);
            for entry in new {
                let found = table.find(entry).map(|listing| (instrument(listing), listing.kind));
                self.0.insert(entry.clone(), found);
            }
        }
        let mut instruments: Vec<Instrument> = Vec::new();
        let mut skipped = 0;
        for entry in entries {
            match self.0.get(entry) {
                Some(Some((instrument, kind))) => {
                    if !instruments.iter().any(|i| i.symbol == instrument.symbol) {
                        api::remember(&instrument.symbol, *kind);
                        instruments.push(instrument.clone());
                    }
                }
                _ => skipped += 1,
            }
        }
        Ok((instruments, skipped))
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
    /// one of its own, `1A0001`); one written as the exchange writes it is
    /// found by its market instead.
    fn find(&self, entry: &Entry) -> Option<&'a api::Listing> {
        let by_market = exchange(&entry.marketid)
            .and_then(|market| self.symbols.get(format!("{}.{market}", entry.code).as_str()));
        by_market.or_else(|| self.tickers.get(entry.code.as_str())).copied()
    }
}

/// The exchange of a Tonghuashun market id, for the ids known for sure:
/// Shanghai's and Shenzhen's indexes and shares, and Beijing.
fn exchange(marketid: &str) -> Option<&'static str> {
    match marketid {
        "16" | "17" => Some("SH"),
        "32" | "33" => Some("SZ"),
        "151" => Some("BJ"),
        _ => None,
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
    account: Option<ThsAccount>,
    /// Quotes need the data API's key.
    key: bool,
    paused: bool,
}

impl Wanted {
    fn of(control: &FeedControl) -> Self {
        Self {
            account: control.credentials.ths_account.clone(),
            key: control.credentials.ths.is_some(),
            paused: control.paused != 0,
        }
    }
}

/// Keeps the user's Tonghuashun watchlist current while signed in there:
/// read every [`SYNC_EVERY`], not at all while nobody can see it, and no
/// more once the login has run out.
pub async fn run(app: AppHandle, mut control: watch::Receiver<FeedControl>) {
    let mut resolved = Resolved::default();
    let mut failures: u32 = 0;
    loop {
        let wanted = Wanted::of(&control.borrow_and_update());
        let delay = match &wanted.account {
            None => {
                resolved = Resolved::default();
                failures = 0;
                publish(&app, |synced| *synced = None);
                None
            }
            Some(_) if wanted.paused => None,
            Some(_) if !wanted.key => {
                let error = t!("error.sourceNeedsCredentials", source = ID.name());
                status(&app, SyncStatus::Failed(error));
                None
            }
            Some(account) => match sync(&app, account, &mut resolved).await {
                Ok(()) => {
                    failures = 0;
                    Some(SYNC_EVERY)
                }
                Err(Failure::SignedOut) => {
                    status(&app, SyncStatus::SignedOut);
                    None
                }
                Err(Failure::Failed(e)) => {
                    log::warn!("tonghuashun watchlist: {e}");
                    status(&app, SyncStatus::Failed(e.to_string()));
                    failures += 1;
                    Some((RETRY * (1 << (failures - 1).min(3))).min(SYNC_EVERY))
                }
            },
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

async fn sync(
    app: &AppHandle,
    account: &ThsAccount,
    resolved: &mut Resolved,
) -> Result<(), Failure> {
    if app.state::<Shared>().model().synced.is_none() {
        publish(app, |synced| *synced = Some(Synced::default()));
    }
    let entries = read(account).await?;
    let (instruments, skipped) = resolved.instruments(&entries).await.map_err(Failure::Failed)?;
    publish(app, |synced| {
        *synced = Some(Synced {
            instruments,
            skipped,
            updated: Some(crate::portfolio::now_ms()),
            status: SyncStatus::Read,
        });
    });
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
            .flat_map(|synced| synced.instruments.iter().map(|i| i.symbol.clone()))
            .collect();
        symbols.sort();
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

/// Keeps a login and lets everything that reads the watchlist know.
fn keep(app: &AppHandle, account: Option<ThsAccount>) {
    let shared = app.state::<Shared>();
    let credentials = {
        let mut stored = shared.credentials();
        stored.ths_account = account;
        if let Err(e) = credentials::save(&shared.credentials_path, &stored) {
            log::error!("cannot save the Tonghuashun login: {e}");
        }
        stored.clone()
    };
    shared.control.send_modify(|control| control.credentials = credentials);
    window::emit_ths(app);
}

/// Forgets the login; the watchlist leaves the dropdown.
pub fn sign_out(app: &AppHandle) {
    keep(app, None);
}

/// Watches the sign-in window until its cookies make a login that reads the
/// watchlist, then keeps the login and closes the window. Ends when the
/// window closes.
pub fn watch_sign_in(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let url = Url::parse(WATCHLIST).expect("a valid URL");
        let mut tried: Option<Vec<(String, String)>> = None;
        loop {
            sleep(Duration::from_secs(1)).await;
            let Some(window) = app.get_webview_window(window::THS_SIGN_IN) else {
                return;
            };
            // Off the runtime's thread: WebView2 answers through the main
            // thread, and deadlocks if asked from it.
            let url = url.clone();
            let found = tauri::async_runtime::spawn_blocking(move || window.cookies_for_url(url));
            let Ok(Ok(cookies)) = found.await else {
                continue;
            };
            let cookies: Vec<(String, String, Option<i64>)> = cookies
                .iter()
                .map(|cookie| {
                    let expires = cookie.expires_datetime().map(|at| at.unix_timestamp());
                    (cookie.name().to_owned(), cookie.value().to_owned(), expires)
                })
                .collect();
            let Some(account) = account_of(&cookies) else {
                continue;
            };
            if tried.as_ref() == Some(&account.cookies) {
                continue;
            }
            tried = Some(account.cookies.clone());
            match read(&account).await {
                Ok(_) => {
                    log::info!("signed in to tonghuashun");
                    keep(&app, Some(account));
                    if let Some(window) = app.get_webview_window(window::THS_SIGN_IN) {
                        let _ = window.close();
                    }
                    return;
                }
                // Halfway there: the page goes on setting cookies.
                Err(Failure::SignedOut) => {}
                Err(Failure::Failed(e)) => {
                    log::warn!("tonghuashun sign-in check: {e}");
                    tried = None;
                }
            }
        }
    });
}

/// The login as the settings window shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub user: String,
    /// Epoch seconds the login runs out at, where known.
    pub expires: Option<i64>,
    /// Codes in the dropdown, and entries nothing here quotes.
    pub count: usize,
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

pub fn view(account: &ThsAccount, synced: Option<&Synced>) -> AccountView {
    let status = synced.map(|s| &s.status);
    AccountView {
        user: account.user.clone(),
        expires: account.expires,
        count: synced.map_or(0, |s| s.instruments.len()),
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

    fn cookie(name: &str, value: &str, expires: Option<i64>) -> (String, String, Option<i64>) {
        (name.to_owned(), value.to_owned(), expires)
    }

    // upstream: the cookies the site clears when a request isn't signed in
    // (`user`, `userid`, `u_name`, `escapename`, `ticket`, `g`), seen 2026-10-28.
    #[test]
    fn a_login_needs_the_user_id_and_keeps_every_cookie_set() {
        assert!(account_of(&[cookie("v", "x", None), cookie("userid", "deleted", None)]).is_none());
        let account = account_of(&[
            cookie("userid", "123456", Some(1_800_000_000)),
            cookie("ticket", "t", Some(1_799_000_000)),
            cookie("escapename", "%u5F20%u4E09", None),
            cookie("g", "", None),
            cookie("v", "x", Some(1_700_000_000)),
        ])
        .unwrap();
        assert_eq!(account.user, "张三");
        // The login's own cookies set its expiry, not a short-lived one beside them.
        assert_eq!(account.expires, Some(1_799_000_000));
        let names: Vec<&str> = account.cookies.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["userid", "ticket", "escapename", "v"]);
        let unnamed = account_of(&[cookie("userid", "123456", None)]).unwrap();
        assert_eq!((unnamed.user.as_str(), unnamed.expires), ("123456", None));
    }

    #[test]
    fn names_read_as_the_site_escapes_them() {
        assert_eq!(unescape("%u5F20%u4E09"), "张三");
        assert_eq!(unescape("%E5%BC%A0%E4%B8%89"), "张三");
        assert_eq!(unescape("mx_123"), "mx_123");
        assert_eq!(unescape("a%20b%"), "a b%");
    }

    fn listing(symbol: &str, ticker: &str, kind: Kind) -> api::Listing {
        api::Listing { symbol: symbol.into(), ticker: ticker.into(), name: None, kind }
    }

    fn entry(code: &str, marketid: &str) -> Entry {
        Entry { code: code.into(), marketid: marketid.into() }
    }

    // upstream: the code table writes the Shanghai Composite as `000001.SH`
    // with Tonghuashun's own code `1A0001` (2026-10-28).
    #[test]
    fn entries_match_by_tonghuashun_code_or_by_market() {
        let listings = [
            listing("000001.SH", "1A0001", Kind::Index),
            listing("000001.SZ", "000001", Kind::Stock),
            listing("885525.TI", "885525", Kind::Index),
            listing("510300.SH", "510300", Kind::Etf),
        ];
        let table = Table::new(&listings);
        let found = |code, market| table.find(&entry(code, market)).map(|l| l.symbol.as_str());
        assert_eq!(found("1A0001", "16"), Some("000001.SH"));
        assert_eq!(found("000001", "33"), Some("000001.SZ"));
        // An index written as the exchange writes it.
        assert_eq!(found("000001", "16"), Some("000001.SH"));
        assert_eq!(found("885525", "48"), Some("885525.TI"));
        assert_eq!(found("510300", "20"), Some("510300.SH"));
        assert_eq!(found("00700", "177"), None);
    }

    #[test]
    fn the_answer_reads_market_ids_written_either_way() {
        let answer: Answer = serde_json::from_str(
            r#"{"errorCode":0,"errorMsg":"","result":[{"code":"600519","marketid":"17"},{"code":"00700","marketid":177}]}"#,
        )
        .unwrap();
        let entries = answer.result.unwrap();
        assert_eq!(entries, [entry("600519", "17"), entry("00700", "177")]);
        // upstream: a request without a valid login, 2026-10-28.
        let refused: Answer = serde_json::from_str(
            r#"{"errorCode":-1,"errorMsg":"当前用户未登录","result":[],"isT":true}"#,
        )
        .unwrap();
        assert_eq!(refused.error_code, SIGNED_OUT);
    }
}
