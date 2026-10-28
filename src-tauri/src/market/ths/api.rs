//! Tonghuashun's financial data API (`fuyao.aicubes.cn`): REST, the user's
//! key in an `X-api-key` header, and every answer in an envelope `{code,
//! message, data}` whose `code` is 0 on success. Refusals come back as HTTP
//! 200 with a code of their own; a throttled request may also be a 429.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, MutexGuard},
};

use serde::{Deserialize, de::DeserializeOwned};

use super::{ID, Kind, parse};
use crate::{
    http,
    market::{Candle, Error},
    net,
};

const HOST: &str = "https://fuyao.aicubes.cn";
const HEADER: &str = "X-api-key";
/// What a search looks among: the kinds the app can quote.
const ASSET_TYPES: &str = "a-share,a-share-index,fund-etf,fund-lof";

/// Result codes (`/docs/api-reference/overview`). A wrong or revoked key
/// comes back as 2003, not the 2001 the docs give.
const OK: i64 = 0;
const NO_KEY: i64 = 2001;
const REFUSED: i64 = 2003;
const NOT_FOUND: i64 = 3001;
const NO_DATA: i64 = 3002;
const THROTTLED: i64 = 4001;

static KEY: Mutex<Option<String>> = Mutex::new(None);
/// What each code is, as searches found it: it decides the endpoints.
/// `None` for a code a search for it didn't find, so it isn't asked again.
static KINDS: LazyLock<Mutex<HashMap<String, Option<Kind>>>> = LazyLock::new(Mutex::default);

/// The key to use from now on, as the settings keep it.
pub fn configure(key: Option<String>) {
    *KEY.lock().unwrap_or_else(|e| e.into_inner()) = key;
}

fn configured() -> Result<String, Error> {
    let key = KEY.lock().unwrap_or_else(|e| e.into_inner()).clone();
    key.ok_or_else(|| Error::Message(t!("error.sourceNeedsCredentials", source = ID.name())))
}

fn kinds() -> MutexGuard<'static, HashMap<String, Option<Kind>>> {
    KINDS.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Deserialize)]
struct Envelope<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

/// The `data` of most endpoints: a list, `null` when empty.
#[derive(Deserialize)]
struct Items<T> {
    item: Option<Vec<T>>,
}

impl<T> Items<T> {
    fn list(self) -> Vec<T> {
        self.item.unwrap_or_default()
    }
}

/// What a request came back with.
enum Answer<T> {
    Data(T),
    /// Refused, with the API's code and its words.
    Refused(i64, String),
}

async fn get<T: DeserializeOwned>(key: &str, path: &str) -> Result<Answer<T>, Error> {
    let url = format!("{HOST}{path}");
    let envelope: Envelope<T> = http::get_json_with(&url, &[(HEADER, key)]).await?;
    match envelope.code {
        OK => envelope.data.map(Answer::Data).ok_or_else(bad_data),
        THROTTLED => Err(http::Error::Throttled(None).into()),
        code => Ok(Answer::Refused(code, envelope.message)),
    }
}

/// [`get`] with the configured key, any refusal an error.
async fn fetch<T: DeserializeOwned>(path: &str) -> Result<T, Error> {
    match get(&configured()?, path).await? {
        Answer::Data(data) => Ok(data),
        Answer::Refused(code, message) => Err(refusal(code, message)),
    }
}

fn refusal(code: i64, message: String) -> Error {
    match code {
        NO_KEY | REFUSED => Error::Message(t!("error.thsKey").to_owned()),
        _ if message.is_empty() => {
            Error::Message(t!("error.sourceCode", source = ID.name(), code = code))
        }
        _ => Error::Message(t!("error.sourceSays", source = ID.name(), message = message)),
    }
}

fn bad_data() -> Error {
    Error::Message(t!("error.sourceBadData", source = ID.name()))
}

/// Whether Tonghuashun takes `key`, asked with the smallest quote there is.
pub async fn check(key: &str) -> Result<(), Error> {
    let path = "/api/a-share-index/prices/snapshot?thscodes=000001.SH";
    match get::<serde::de::IgnoredAny>(key, path).await? {
        Answer::Data(_) => Ok(()),
        Answer::Refused(code, message) => Err(refusal(code, message)),
    }
}

#[derive(Deserialize)]
struct Ticker {
    thscode: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    asset_type: String,
}

/// A code a search found.
pub struct Found {
    pub symbol: String,
    pub name: Option<String>,
    pub kind: Kind,
}

/// Codes whose code or name contains `query`, best first, among the kinds
/// the app quotes; each one's kind is remembered.
pub async fn search(query: &str, limit: usize) -> Result<Vec<Found>, Error> {
    let path = format!(
        "/api/meta/tickers/search?q={}&asset_type={ASSET_TYPES}&limit={limit}",
        net::percent_encode(query)
    );
    let tickers: Items<Ticker> = fetch(&path).await?;
    let found: Vec<Found> = tickers
        .list()
        .into_iter()
        .filter_map(|ticker| {
            Some(Found {
                symbol: parse(ticker.thscode.as_deref()?)?,
                kind: Kind::of(&ticker.asset_type)?,
                name: ticker.name,
            })
        })
        .collect();
    let mut kinds = kinds();
    for found in &found {
        kinds.insert(found.symbol.clone(), Some(found.kind));
    }
    Ok(found)
}

/// Remembers what `symbol` is, as a listing said.
pub fn remember(symbol: &str, kind: Kind) {
    kinds().insert(symbol.to_owned(), Some(kind));
}

#[derive(Deserialize)]
struct Listed {
    thscode: Option<String>,
    #[serde(default)]
    ticker: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    asset_type: String,
}

/// A code the data API lists among the kinds the app quotes.
pub struct Listing {
    pub symbol: String,
    /// Tonghuashun's own code for it, as its apps write it: the six digits,
    /// or for some indexes a code of its own (`1A0001` for `000001.SH`).
    pub ticker: String,
    pub name: Option<String>,
    pub kind: Kind,
}

/// Every code the app could quote, some 9,000 of them (2 MB): one request.
pub async fn listings() -> Result<Vec<Listing>, Error> {
    let path = format!("/api/meta/tickers/list?asset_type={ASSET_TYPES}&limit=10000");
    let listed: Items<Listed> = fetch(&path).await?;
    Ok(listed
        .list()
        .into_iter()
        .filter_map(|listed| {
            let symbol = parse(listed.thscode.as_deref()?)?;
            Some(Listing {
                ticker: listed.ticker.unwrap_or_else(|| super::code(&symbol).to_owned()),
                kind: Kind::of(&listed.asset_type)?,
                name: listed.name,
                symbol,
            })
        })
        .collect())
}

/// What `symbol` is, if a search has found it.
pub fn known(symbol: &str) -> Option<Kind> {
    kinds().get(symbol).copied().flatten()
}

/// What `symbol` is, searching for it the first time; `None` when
/// Tonghuashun lists no such code among the kinds the app quotes (one
/// delisted since it was added, say).
pub async fn kind(symbol: &str) -> Result<Option<Kind>, Error> {
    if let Some(kind) = kinds().get(symbol).copied() {
        return Ok(kind);
    }
    let found = search(symbol, 5).await?;
    let kind = found.into_iter().find(|f| f.symbol == symbol).map(|f| f.kind);
    if kind.is_none() {
        log::warn!("tonghuashun lists no {symbol}");
        kinds().insert(symbol.to_owned(), None);
    }
    Ok(kind)
}

#[derive(Deserialize)]
struct Row {
    thscode: Option<String>,
    last_price: Option<f64>,
    prev_price: Option<f64>,
    high_price: Option<f64>,
    low_price: Option<f64>,
    volume: Option<f64>,
    turnover: Option<f64>,
}

/// A code's price and its day so far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    pub last: f64,
    /// The previous close: what the day's change counts from.
    pub prev_close: f64,
    pub high: Option<f64>,
    pub low: Option<f64>,
    /// Shares (fund units, for an index its members' shares) traded today.
    pub volume: f64,
    /// Yuan traded today.
    pub turnover: f64,
}

impl Row {
    fn snapshot(&self) -> Option<Snapshot> {
        let positive = |value: Option<f64>| value.filter(|v| *v > 0.0);
        let prev_close = positive(self.prev_price);
        // Before its first trade of the day a code has no price of its own.
        let last = positive(self.last_price).or(prev_close)?;
        Some(Snapshot {
            last,
            prev_close: prev_close.unwrap_or(last),
            high: positive(self.high_price),
            low: positive(self.low_price),
            volume: self.volume.unwrap_or(0.0),
            turnover: self.turnover.unwrap_or(0.0),
        })
    }
}

/// Codes per quote request for shares and indexes.
const BATCH: usize = 50;

/// Snapshots of codes of one kind, by code: shares and indexes [`BATCH`] to
/// a request, funds one apiece. Codes it doesn't know are left out.
pub async fn snapshots(kind: Kind, symbols: &[String]) -> Result<Vec<(String, Snapshot)>, Error> {
    let (path, batch) = match kind {
        Kind::Stock => ("/api/a-share/prices/snapshot?thscodes=", BATCH),
        Kind::Index => ("/api/a-share-index/prices/snapshot?thscodes=", BATCH),
        Kind::Etf | Kind::Lof => ("/api/fund/market/snapshot?thscode=", 1),
    };
    let mut found = Vec::new();
    for batch in symbols.chunks(batch) {
        found.extend(rows(&format!("{path}{}", batch.join(","))).await?);
    }
    Ok(found
        .into_iter()
        .filter_map(|row| Some((parse(row.thscode.as_deref()?)?, row.snapshot()?)))
        .collect())
}

/// One code's snapshot.
pub async fn snapshot(symbol: &str) -> Result<Snapshot, Error> {
    let kind = kind(symbol).await?.ok_or_else(|| super::unknown(symbol))?;
    let found = snapshots(kind, &[symbol.to_owned()]).await?;
    found.into_iter().next().map(|(_, snapshot)| snapshot).ok_or_else(|| super::unknown(symbol))
}

/// The rows a quote request found: none for codes the API doesn't know or
/// has nothing on yet.
async fn rows(path: &str) -> Result<Vec<Row>, Error> {
    match get::<Items<Row>>(&configured()?, path).await? {
        Answer::Data(items) => Ok(items.list()),
        Answer::Refused(NOT_FOUND | NO_DATA, _) => Ok(Vec::new()),
        Answer::Refused(code, message) => Err(refusal(code, message)),
    }
}

#[derive(Deserialize)]
struct Bar {
    date_ms: i64,
    open_price: Option<f64>,
    high_price: Option<f64>,
    low_price: Option<f64>,
    close_price: Option<f64>,
}

/// Daily candles of a code from `start` to `end` (epoch milliseconds, both
/// included), oldest first, each at its day's midnight in Shanghai. Shares
/// come forward-adjusted, which Tonghuashun does by taking dividends off:
/// far enough back the prices turn negative, and those days are left out.
/// Funds come forward-adjusted too, indexes as they are; LOFs have none.
pub async fn daily(kind: Kind, symbol: &str, start: i64, end: i64) -> Result<Vec<Candle>, Error> {
    let (path, adjust) = match kind {
        Kind::Stock => ("/api/a-share/prices/historical", "&adjust=forward"),
        Kind::Index => ("/api/a-share-index/prices/historical", ""),
        Kind::Etf => ("/api/fund/market/historical", ""),
        Kind::Lof => return Ok(Vec::new()),
    };
    let bars: Items<Bar> =
        fetch(&format!("{path}?thscode={symbol}&interval=1d&start={start}&end={end}{adjust}"))
            .await?;
    let mut candles: Vec<Candle> = bars
        .list()
        .into_iter()
        .filter_map(|bar| {
            let candle = Candle {
                time: (bar.date_ms / 1000) as f64,
                open: bar.open_price?,
                high: bar.high_price?,
                low: bar.low_price?,
                close: bar.close_price?,
            };
            let drawable =
                [candle.open, candle.high, candle.low, candle.close].iter().all(|p| *p > 0.0);
            drawable.then_some(candle)
        })
        .collect();
    candles.sort_by(|a, b| a.time.total_cmp(&b.time));
    candles.dedup_by(|a, b| a.time == b.time);
    Ok(candles)
}

#[derive(Deserialize)]
struct TradingDay {
    date_ms: i64,
}

/// The days Shanghai traded or trades on, from a year ago through today,
/// as the epoch milliseconds of their midnight.
pub async fn trading_days() -> Result<Vec<i64>, Error> {
    let days: Items<TradingDay> = fetch("/api/a-share/calendar/trading-days").await?;
    Ok(days.list().into_iter().map(|day| day.date_ms).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(json: &str) -> Option<Snapshot> {
        serde_json::from_str::<Row>(json).unwrap().snapshot()
    }

    // upstream: /api/a-share/prices/snapshot, as answered on 2026-10-28.
    #[test]
    fn a_code_without_a_trade_today_is_priced_at_its_last_close() {
        let traded = row(
            r#"{"thscode":"600519.SH","last_price":1331.45,"prev_price":1355.9,"high_price":1354.98,"low_price":1330.17,"volume":2201241,"turnover":2951089600}"#,
        )
        .unwrap();
        assert_eq!((traded.last, traded.prev_close, traded.volume), (1331.45, 1355.9, 2_201_241.0));
        let untraded =
            row(r#"{"thscode":"600519.SH","last_price":0,"prev_price":1355.9,"high_price":0}"#)
                .unwrap();
        assert_eq!((untraded.last, untraded.high, untraded.volume), (1355.9, None, 0.0));
        assert!(row(r#"{"thscode":"600519.SH","last_price":null,"prev_price":null}"#).is_none());
    }

    // upstream: a revoked key's answer, and a history range with no days in
    // it, as answered on 2026-10-28.
    #[test]
    fn refusals_and_empty_lists_parse() {
        let refused: Envelope<Items<Row>> = serde_json::from_str(
            r#"{"code":2003,"message":"Invalid or revoked API key","data":null,"request_id":"x"}"#,
        )
        .unwrap();
        assert_eq!((refused.code, refused.data.is_none()), (2003, true));
        let empty: Envelope<Items<Bar>> = serde_json::from_str(
            r#"{"code":0,"message":"success","data":{"timestamp":null,"item":[],"adjust":null}}"#,
        )
        .unwrap();
        assert!(empty.data.unwrap().list().is_empty());
    }
}
