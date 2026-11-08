//! Published CNY fund NAVs and Shanghai Gold Exchange Au99.99 quotes from
//! Eastmoney's public website endpoints. Fund quantities are shares; gold
//! quantities are grams. NAVs carry their publication date, not a live-price
//! timestamp. Income-per-10,000-share series are not unit NAVs and are refused.

use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex},
    time::Duration,
};

use futures_util::{StreamExt as _, stream};
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    sync::{mpsc::UnboundedSender, oneshot, watch},
    time::{Instant, sleep},
};

use crate::{
    app::App,
    calendar, http,
    model::{FeedControl, Instrument, Quote, Status},
};

use super::{
    BoxFuture, Candidate, Candle, ChartMode, ChartSpec, Error, FeedState, IntervalSpec, LiveEvent,
    Provider, ProviderId, Search, Stats, StatsSpan, Trade,
};

pub struct Eastmoney;

const ID: ProviderId = ProviderId::Eastmoney;
const QUOTE: &str = "CNY";
const GOLD: &str = "118.AU9999";
const SEARCH_URL: &str = "https://searchapi.eastmoney.com/api/suggest/get";
const REFERER: &[(&str, &str)] = &[("Referer", "https://fundf10.eastmoney.com/")];
const FUND_POLL: Duration = Duration::from_secs(15 * 60);
const GOLD_POLL: Duration = Duration::from_secs(15);
const CONCURRENCY: usize = 4;
const SEARCH_LIMIT: usize = 8;
const DAY: u32 = 86_400;
const SHANGHAI_OFFSET: i64 = 8 * 3600;

// Only source-confirmed, CNY fund identifiers enter this cache. A persisted
// instrument is checked again after launch before its NAV is used.
static FUNDS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Symbol {
    Fund(String),
    Gold,
}

impl Symbol {
    fn parse(symbol: &str) -> Option<Self> {
        let symbol = symbol.trim();
        if symbol.eq_ignore_ascii_case(GOLD) {
            return Some(Self::Gold);
        }
        let code = symbol.strip_prefix("150.")?;
        (code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| Self::Fund(code.to_owned()))
    }

    fn symbol(&self) -> String {
        match self {
            Self::Fund(code) => format!("150.{code}"),
            Self::Gold => GOLD.to_owned(),
        }
    }

    fn base(&self) -> &str {
        match self {
            Self::Fund(code) => code,
            Self::Gold => "AU9999",
        }
    }

    fn poll(&self) -> Duration {
        match self {
            Self::Fund(_) => FUND_POLL,
            Self::Gold => GOLD_POLL,
        }
    }

    fn instrument(&self, name: Option<String>) -> Instrument {
        Instrument {
            provider: ID,
            symbol: self.symbol(),
            base: self.base().to_owned(),
            quote: QUOTE.to_owned(),
            name,
            decimals: Some(if matches!(self, Self::Gold) { 2 } else { 4 }),
            pinned: false,
            venue: None,
        }
    }
}

impl Provider for Eastmoney {
    fn id(&self) -> ProviderId {
        ID
    }

    fn instrument_meta(&self, instrument: &Instrument) -> super::InstrumentMeta {
        let fund = matches!(Symbol::parse(&instrument.symbol), Some(Symbol::Fund(_)));
        super::InstrumentMeta {
            asset_id: instrument.id(),
            quantity_unit: if fund {
                super::QuantityUnit::FundUnits
            } else {
                super::QuantityUnit::Grams
            },
            price_kind: if fund {
                super::PriceKind::Nav { utc_offset: SHANGHAI_OFFSET }
            } else {
                super::PriceKind::Market
            },
            usd_conversion: None,
        }
    }

    fn validate(&self, instrument: &mut Instrument) -> Result<(), String> {
        let symbol = Symbol::parse(&instrument.symbol)
            .filter(|symbol| instrument.base == symbol.base() && instrument.quote == QUOTE)
            .ok_or_else(|| t!("error.invalidStock", symbol = instrument.symbol))?;
        instrument.symbol = symbol.symbol();
        instrument.name = instrument
            .name
            .take()
            .map(|name| name.trim().chars().take(80).collect::<String>())
            .filter(|name| !name.is_empty());
        Ok(())
    }

    fn bare(&self, symbol: &str) -> Option<Instrument> {
        Some(Symbol::parse(symbol)?.instrument(None))
    }

    fn search<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Search> {
        Box::pin(search(query))
    }

    fn watch(&self, app: App, control: watch::Receiver<FeedControl>) -> BoxFuture<'static, ()> {
        Box::pin(watch_quotes(app, control))
    }

    fn chart_spec(&self, instrument: &Instrument) -> ChartSpec {
        ChartSpec {
            source: ID,
            // No candle history or trade tape is promised. The chart can
            // receive polled statistics and display its empty-history state.
            intervals: vec![IntervalSpec {
                secs: DAY,
                mode: ChartMode::Line,
                aligned: false,
                regular_only: false,
            }],
            stats_span: StatsSpan::Today,
            volume_unit: None,
            turnover_unit: QUOTE.to_owned(),
            day_offset: SHANGHAI_OFFSET,
            book_steps: Vec::new(),
            book: false,
            onchain: false,
            link: Symbol::parse(&instrument.symbol).map(|symbol| match symbol {
                Symbol::Fund(code) => format!("https://fund.eastmoney.com/{code}.html"),
                Symbol::Gold => "https://quote.eastmoney.com/q/118.AU9999.html".to_owned(),
            }),
        }
    }

    fn history<'a>(
        &'a self,
        _instrument: &'a Instrument,
        _interval: u32,
        _end: Option<f64>,
        _limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Candle>, Error>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn recent_trades<'a>(
        &'a self,
        _instrument: &'a Instrument,
        _limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Trade>, Error>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn stream(
        &self,
        instrument: Instrument,
        mut stop: oneshot::Receiver<()>,
        events: UnboundedSender<LiveEvent>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let Some(symbol) = Symbol::parse(&instrument.symbol) else { return };
            let mut failures = 0u32;
            let mut live = false;
            loop {
                let _ = events.send(LiveEvent::State {
                    state: if live { FeedState::Live } else { FeedState::Connecting },
                });
                let result = tokio::select! {
                    _ = &mut stop => return,
                    _ = events.closed() => return,
                    result = snapshot(&symbol) => result,
                };
                let delay = match result {
                    Ok(snapshot) => {
                        failures = 0;
                        live = true;
                        if events.send(LiveEvent::State { state: FeedState::Live }).is_err()
                            || events.send(LiveEvent::Stats { stats: snapshot.stats() }).is_err()
                        {
                            return;
                        }
                        symbol.poll()
                    }
                    Err(error) => {
                        log::debug!("eastmoney quote of {}: {error}", symbol.symbol());
                        live = false;
                        failures = failures.saturating_add(1);
                        let _ = events.send(LiveEvent::State { state: FeedState::Offline });
                        backoff(failures)
                    }
                };
                tokio::select! {
                    _ = &mut stop => return,
                    _ = events.closed() => return,
                    _ = sleep(delay) => {},
                }
            }
        })
    }
}

fn bad_data() -> Error {
    Error::Message(t!("error.sourceBadData", source = ID.name()))
}

fn unknown(symbol: &str) -> Error {
    Error::Message(t!("error.unknownSymbol", source = ID.name(), symbol = symbol))
}

#[derive(Deserialize)]
struct Suggestions {
    #[serde(rename = "QuotationCodeTable")]
    table: SuggestionTable,
}

#[derive(Deserialize)]
struct SuggestionTable {
    #[serde(rename = "Data", default)]
    data: Option<Vec<Suggestion>>,
    #[serde(rename = "Status")]
    status: i64,
}

#[derive(Deserialize)]
struct Suggestion {
    #[serde(rename = "Code")]
    code: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Classify")]
    classify: String,
    #[serde(rename = "QuoteID")]
    quote_id: String,
}

impl Suggestion {
    fn supported(&self) -> Option<Symbol> {
        let symbol = Symbol::parse(&self.quote_id)?;
        match &symbol {
            Symbol::Fund(code)
                if self.classify == "OTCFUND" && code == &self.code && cny_name(&self.name) =>
            {
                Some(symbol)
            }
            Symbol::Gold if self.classify == "SGE" && self.code.eq_ignore_ascii_case("AU9999") => {
                Some(symbol)
            }
            _ => None,
        }
    }
}

// Foreign-currency share classes are distinct securities; this source's
// holding instrument is explicitly CNY. Names that denote such share
// classes must never be re-labelled as yuan NAVs.
fn cny_name(name: &str) -> bool {
    if name.contains("人民币") {
        return true;
    }
    let upper = name.to_ascii_uppercase();
    ![
        "美元", "美金", "港币", "港元", "欧元", "英镑", "日元", "澳元", "加元", "瑞郎", "现汇",
        "现钞", "USD", "HKD", "EUR", "GBP", "JPY", "AUD", "CAD", "CHF",
    ]
    .iter()
    .any(|currency| upper.contains(currency))
}

async fn suggestions(query: &str) -> Result<Vec<Suggestion>, Error> {
    let mut url = reqwest::Url::parse(SEARCH_URL).map_err(|_| bad_data())?;
    url.query_pairs_mut()
        .append_pair("input", query)
        .append_pair("type", "14")
        .append_pair("count", "30");
    let found: Suggestions = http::get_json(url.as_str()).await?;
    if found.table.status != 0 {
        return Err(bad_data());
    }
    Ok(found.table.data.unwrap_or_default())
}

async fn search(query: &str) -> Search {
    let query = query.trim();
    if query.is_empty() {
        return Search::default();
    }
    let found = match suggestions(query).await {
        Ok(found) => found,
        Err(error) => return Search { candidates: Vec::new(), notes: vec![error.to_string()] },
    };
    let mut matches: Vec<_> = found
        .into_iter()
        .filter_map(|found| found.supported().map(|symbol| (symbol, found.name)))
        .take(SEARCH_LIMIT)
        .collect();
    // A broad Chinese gold search can be filled entirely by unrelated gold
    // stocks. Resolve this exact source security rather than inventing a row.
    if matches.iter().all(|(symbol, _)| !matches!(symbol, Symbol::Gold))
        && (query.contains("黄金")
            || ["gold", "au9999", "au99.99", GOLD]
                .iter()
                .any(|gold| query.eq_ignore_ascii_case(gold)))
        && let Ok(gold) = suggestions("AU9999").await
    {
        matches.extend(gold.into_iter().filter_map(|found| {
            found
                .supported()
                .and_then(|symbol| matches!(symbol, Symbol::Gold).then_some((symbol, found.name)))
        }));
    }
    // Discovery consumes source metadata only. Price availability and NAV
    // semantics are checked by the quote worker, never on the global search path.
    let candidates = matches
        .into_iter()
        .map(|(symbol, name)| {
            if let Symbol::Fund(code) = &symbol {
                FUNDS.lock().unwrap_or_else(|e| e.into_inner()).insert(code.clone());
            }
            Candidate { instrument: symbol.instrument(Some(name)), manual: false, liquidity: None }
        })
        .collect();
    Search { candidates, notes: Vec::new() }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Snapshot {
    last: f64,
    reference: Option<f64>,
    /// Epoch milliseconds. A fund's date is Shanghai local midnight.
    source_time: Option<f64>,
}

impl Snapshot {
    fn stats(self) -> Stats {
        let change = self.reference.map(|reference| self.last - reference);
        Stats {
            last: self.last,
            high: None,
            low: None,
            volume: None,
            turnover: 0.,
            change,
            change_pct: self
                .reference
                .and_then(|reference| crate::portfolio::pct(self.last, reference)),
            onchain: None,
        }
    }
}

async fn snapshot(symbol: &Symbol) -> Result<Snapshot, Error> {
    match symbol {
        Symbol::Fund(code) => {
            ensure_fund(code).await?;
            let url = format!(
                "https://api.fund.eastmoney.com/f10/lsjz?fundCode={code}&pageIndex=1&pageSize=2"
            );
            let found: NavEnvelope = http::get_json_with(&url, REFERER).await?;
            found.snapshot()
        }
        Symbol::Gold => {
            let url = format!(
                "https://push2.eastmoney.com/api/qt/stock/get?secid={GOLD}&fields=f43,f58,f59,f60,f86"
            );
            let found: GoldEnvelope = http::get_json(&url).await?;
            found.snapshot()
        }
    }
}

async fn ensure_fund(code: &str) -> Result<(), Error> {
    if FUNDS.lock().unwrap_or_else(|e| e.into_inner()).contains(code) {
        return Ok(());
    }
    suggestions(code)
        .await?
        .into_iter()
        .find(
            |found| matches!(found.supported(), Some(Symbol::Fund(ref matched)) if matched == code),
        )
        .ok_or_else(|| unknown(code))?;
    FUNDS.lock().unwrap_or_else(|e| e.into_inner()).insert(code.to_owned());
    Ok(())
}

#[derive(Deserialize)]
struct NavEnvelope {
    #[serde(rename = "Data")]
    data: Option<NavData>,
    #[serde(rename = "ErrCode")]
    error: i64,
}

#[derive(Deserialize)]
struct NavData {
    #[serde(rename = "LSJZList")]
    navs: Vec<Nav>,
    #[serde(rename = "SYType")]
    income_type: Option<String>,
}

#[derive(Deserialize)]
struct Nav {
    #[serde(rename = "FSRQ")]
    date: String,
    #[serde(rename = "DWJZ")]
    value: String,
}

impl NavEnvelope {
    fn snapshot(self) -> Result<Snapshot, Error> {
        if self.error != 0 {
            return Err(bad_data());
        }
        let data = self.data.ok_or_else(bad_data)?;
        if data.income_type.as_ref().is_some_and(|kind| !kind.is_empty()) {
            return Err(bad_data());
        }
        let latest = data.navs.first().ok_or_else(bad_data)?;
        let source_time = nav_time(&latest.date).ok_or_else(bad_data)?;
        let reference = data
            .navs
            .get(1)
            .filter(|previous| nav_time(&previous.date).is_some_and(|time| time < source_time))
            .and_then(|previous| positive(&previous.value));
        Ok(Snapshot {
            last: positive(&latest.value).ok_or_else(bad_data)?,
            reference,
            source_time: Some(source_time),
        })
    }
}

fn positive(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|value| value.is_finite() && *value > 0.)
}

fn nav_time(date: &str) -> Option<f64> {
    if date.len() != 10
        || date.as_bytes().get(4) != Some(&b'-')
        || date.as_bytes().get(7) != Some(&b'-')
    {
        return None;
    }
    let year: i64 = date.get(..4)?.parse().ok()?;
    let month: u32 = date.get(5..7)?.parse().ok()?;
    let day: u32 = date.get(8..)?.parse().ok()?;
    if !(1900..=2200).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = calendar::days_from_civil(year, month, day);
    let verified = calendar::civil(days * DAY as i64);
    if (verified.year, verified.month, verified.day) != (year, month, day) {
        return None;
    }
    Some(((days * DAY as i64 - SHANGHAI_OFFSET) * 1000) as f64)
}

#[derive(Deserialize)]
struct GoldEnvelope {
    rc: i64,
    data: Option<GoldData>,
}

#[derive(Deserialize)]
struct GoldData {
    f43: Value,
    f59: u32,
    #[serde(default)]
    f60: Value,
    #[serde(default)]
    f86: Value,
}

impl GoldEnvelope {
    fn snapshot(self) -> Result<Snapshot, Error> {
        if self.rc != 0 {
            return Err(bad_data());
        }
        let data = self.data.ok_or_else(bad_data)?;
        if data.f59 > 8 {
            return Err(bad_data());
        }
        let number =
            |value: &Value| value.as_f64().filter(|value| value.is_finite() && *value > 0.);
        let scale = 10f64.powi(data.f59 as i32);
        Ok(Snapshot {
            last: number(&data.f43).ok_or_else(bad_data)? / scale,
            reference: number(&data.f60).map(|reference| reference / scale),
            source_time: number(&data.f86).map(|time| time * 1000.),
        })
    }
}

fn backoff(failures: u32) -> Duration {
    Duration::from_secs((5u64 << failures.saturating_sub(1).min(6)).min(300))
}

async fn watch_quotes(app: App, mut control: watch::Receiver<FeedControl>) {
    let mut due: HashMap<String, Instant> = HashMap::new();
    let mut failures: HashMap<String, u32> = HashMap::new();
    loop {
        let wanted = super::wanted(&control.borrow_and_update(), ID);
        let (symbols, paused) = &wanted;
        let active: HashSet<_> = symbols.iter().map(String::as_str).collect();
        due.retain(|symbol, _| active.contains(symbol.as_str()));
        failures.retain(|symbol, _| active.contains(symbol.as_str()));
        if symbols.is_empty() || *paused != 0 {
            super::set_status(
                &app,
                ID,
                if symbols.is_empty() { Status::Idle } else { Status::Paused },
            );
            due.clear();
            failures.clear();
            if super::changed(&mut control, ID, &wanted).await.is_err() {
                return;
            }
            continue;
        }
        let now = Instant::now();
        let asked: Vec<_> = symbols
            .iter()
            .filter(|symbol| due.get(*symbol).is_none_or(|at| *at <= now))
            .cloned()
            .collect();
        if !asked.is_empty() {
            if due.is_empty() {
                super::set_status(&app, ID, Status::Connecting);
            }
            let round = stream::iter(asked.into_iter().map(|symbol| async move {
                let result = match Symbol::parse(&symbol) {
                    Some(parsed) => snapshot(&parsed).await,
                    None => Err(unknown(&symbol)),
                };
                (symbol, result)
            }))
            .buffer_unordered(CONCURRENCY)
            .collect::<Vec<_>>();
            let result = tokio::select! {
                result = round => result,
                changed = super::changed(&mut control, ID, &wanted) => {
                    if changed.is_err() { return; }
                    continue;
                }
            };
            let mut changed = false;
            {
                let mut model = app.shared().model();
                for (symbol, result) in result {
                    let id = super::instrument_id(ID, &symbol);
                    if model.instrument(&id).is_none() {
                        due.insert(symbol, Instant::now() + FUND_POLL);
                        continue;
                    }
                    match result {
                        Ok(snapshot) => {
                            failures.remove(&symbol);
                            changed |= super::record_quote(
                                &mut model,
                                id,
                                Quote {
                                    last: snapshot.last,
                                    open: snapshot.reference.unwrap_or(0.),
                                    session: None,
                                    ..Default::default()
                                },
                                snapshot.source_time,
                            );
                            let poll =
                                Symbol::parse(&symbol).map_or(FUND_POLL, |symbol| symbol.poll());
                            due.insert(symbol, Instant::now() + poll);
                        }
                        Err(error) => {
                            log::warn!("eastmoney quote of {symbol}: {error}");
                            let attempts = failures.entry(symbol.clone()).or_default();
                            *attempts = attempts.saturating_add(1);
                            due.insert(symbol, Instant::now() + backoff(*attempts));
                        }
                    }
                }
            }
            if changed {
                super::quotes_changed(&app);
            }
            if failures.is_empty() {
                super::set_status(&app, ID, Status::Live);
            } else {
                let wait = failures
                    .keys()
                    .filter_map(|symbol| due.get(symbol))
                    .copied()
                    .min()
                    .map(|at| at.saturating_duration_since(Instant::now()).as_secs())
                    .unwrap_or(5);
                super::set_status(&app, ID, Status::Retrying { retry_in_secs: wait });
            }
        }
        let delay = due.values().copied().min().unwrap_or_else(Instant::now);
        tokio::select! {
            _ = tokio::time::sleep_until(delay) => {},
            changed = super::changed(&mut control, ID, &wanted) => if changed.is_err() { return; },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: Eastmoney public endpoints captured 2026-11-08. The latest
    // fund NAV has an actual valuation date and a previous published NAV,
    // unlike a stream timestamp or a rolling 24h market reference.
    #[test]
    fn fund_navs_keep_the_published_date_and_reject_income_series() {
        let response = r#"{"Data":{"LSJZList":[{"FSRQ":"2026-11-06","DWJZ":"3.0022"},{"FSRQ":"2026-11-05","DWJZ":"3.0545"}],"SYType":null},"ErrCode":0}"#;
        let snapshot = serde_json::from_str::<NavEnvelope>(response).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.last, 3.0022);
        assert_eq!(snapshot.reference, Some(3.0545));
        let mut first_nav: Value = serde_json::from_str(response).unwrap();
        first_nav["Data"]["LSJZList"].as_array_mut().unwrap().pop();
        let first_nav =
            serde_json::from_value::<NavEnvelope>(first_nav).unwrap().snapshot().unwrap();
        assert_eq!(first_nav.last, 3.0022);
        assert_eq!(first_nav.reference, None);
        let stats = serde_json::to_value(first_nav.stats()).unwrap();
        assert_eq!(stats["last"], 3.0022);
        assert!(stats["change"].is_null() && stats["changePct"].is_null());
        assert_eq!(
            calendar::iso8601_ms(snapshot.source_time.unwrap() as i64),
            "2026-11-05T16:00:00.000Z"
        );
        let income = response.replace("\"SYType\":null", "\"SYType\":\"每万份收益\"");
        assert!(serde_json::from_str::<NavEnvelope>(&income).unwrap().snapshot().is_err());
        for date in ["2026-02-30", "2026-13-01", "not-a-date"] {
            assert!(nav_time(date).is_none());
        }
    }

    // upstream: AU9999 scales its quote by f59 and supplies epoch seconds.
    // Source placeholders must fail rather than creating a zero-price asset.
    #[test]
    fn gold_quotes_scale_prices_and_preserve_source_time() {
        let response = r#"{"rc":0,"data":{"f43":87400,"f59":2,"f60":86857,"f86":1793989794}}"#;
        let snapshot = serde_json::from_str::<GoldEnvelope>(response).unwrap().snapshot().unwrap();
        assert_eq!(snapshot.last, 874.);
        assert_eq!(snapshot.reference, Some(868.57));
        assert_eq!(snapshot.source_time, Some(1793989794000.));
        let mut omitted: Value = serde_json::from_str(response).unwrap();
        omitted["data"].as_object_mut().unwrap().remove("f60");
        assert!(
            serde_json::from_value::<GoldEnvelope>(omitted)
                .unwrap()
                .snapshot()
                .unwrap()
                .stats()
                .change
                .is_none()
        );
        let no_reference = response.replace("86857", "\"-\"");
        assert!(
            serde_json::from_str::<GoldEnvelope>(&no_reference).unwrap().snapshot().is_ok(),
            "a missing previous close must not discard the current gold price"
        );
        let missing = response.replace("87400", "\"-\"");
        assert!(serde_json::from_str::<GoldEnvelope>(&missing).unwrap().snapshot().is_err());
    }

    // Contract: a platform's purchase of a fund uses its published NAV,
    // distinct from the same fund's exchange-traded market price.
    #[test]
    fn search_accepts_only_identified_supported_securities() {
        let found: Suggestions = serde_json::from_str(r#"{"QuotationCodeTable":{"Data":[{"Code":"161725","Name":"白酒基金LOF","Classify":"Fund","QuoteID":"0.161725"},{"Code":"161725","Name":"招商中证白酒指数(LOF)A","Classify":"OTCFUND","QuoteID":"150.161725"},{"Code":"AU9999","Name":"黄金9999","Classify":"SGE","QuoteID":"118.AU9999"},{"Code":"000000","Name":"美元现汇份额","Classify":"OTCFUND","QuoteID":"150.000000"}],"Status":0}}"#).unwrap();
        let symbols: Vec<_> = found
            .table
            .data
            .unwrap()
            .iter()
            .filter_map(Suggestion::supported)
            .map(|symbol| symbol.symbol())
            .collect();
        assert_eq!(symbols, ["150.161725", "118.AU9999"]);
        assert!(Symbol::parse("150.123456/../../").is_none());
    }
}
