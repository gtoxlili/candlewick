//! Binance spot: public market data, no account or key.

mod stream;
mod ticker;

use std::{
    fmt,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeOwned, IgnoredAny, SeqAccess, Visitor},
};
use tauri::{AppHandle, ipc::Channel};
use tokio::sync::{oneshot, watch};

use super::{
    BoxFuture, Candidate, Candle, ChartMode, ChartSpec, Error, IntervalSpec, Link, LiveEvent,
    Provider, ProviderId, Search, Stats, Trade,
};
use crate::{
    http,
    model::{FeedControl, Instrument},
    net,
};

/// The second host of each pair serves the same public market data and is
/// reachable from some networks where the first is not.
const REST_HOSTS: [&str; 2] = ["api.binance.com", "data-api.binance.vision"];
const WS_HOSTS: [&str; 2] = ["stream.binance.com", "data-stream.binance.vision"];

/// Seconds, label, Binance's name, and how the chart first shows it: one-second
/// candles are mostly noise, a line reads better.
const INTERVALS: [(u32, &str, &str, ChartMode); 7] = [
    (1, "1秒", "1s", ChartMode::Line),
    (60, "1分", "1m", ChartMode::Candle),
    (300, "5分", "5m", ChartMode::Candle),
    (900, "15分", "15m", ChartMode::Candle),
    (3600, "1小时", "1h", ChartMode::Candle),
    (14_400, "4小时", "4h", ChartMode::Candle),
    (86_400, "1日", "1d", ChartMode::Candle),
];

/// Binance's most klines or trades per request.
const MAX_PAGE: usize = 1000;

pub struct Binance;

impl Provider for Binance {
    fn validate(&self, instrument: &mut Instrument) -> Result<(), String> {
        instrument.symbol = instrument.symbol.trim().to_uppercase();
        instrument.base = instrument.base.trim().to_uppercase();
        instrument.quote = instrument.quote.trim().to_uppercase();
        let valid = |s: &str, max: usize| {
            !s.is_empty() && s.chars().count() <= max && s.chars().all(char::is_alphanumeric)
        };
        if !valid(&instrument.symbol, 24)
            || !valid(&instrument.base, 16)
            || !valid(&instrument.quote, 12)
        {
            return Err(format!("无效的交易对：{}", instrument.symbol));
        }
        if instrument.symbol != format!("{}{}", instrument.base, instrument.quote) {
            return Err(format!("交易对与币种不匹配：{}", instrument.symbol));
        }
        Ok(())
    }

    fn search<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Search> {
        Box::pin(search(query))
    }

    fn drop_search_cache(&self) {
        GENERATION.fetch_add(1, Ordering::AcqRel);
        *PAIRS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn watch(
        &self,
        app: AppHandle,
        control: watch::Receiver<FeedControl>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(ticker::run(app, control))
    }

    fn chart_spec(&self, instrument: &Instrument) -> ChartSpec {
        ChartSpec {
            source: ProviderId::Binance.name(),
            intervals: INTERVALS
                .iter()
                .map(|&(secs, label, _, mode)| IntervalSpec {
                    secs,
                    label,
                    mode,
                    aligned: true,
                    regular_only: false,
                })
                .collect(),
            stats_span: "24h",
            volume_unit: instrument.base.clone(),
            turnover_unit: instrument.quote.clone(),
            // Daily candles open at 00:00 UTC.
            day_offset: 0,
            link: Some(Link {
                label: "在币安打开",
                url: format!(
                    "https://www.binance.com/zh-CN/trade/{}_{}?type=spot",
                    net::percent_encode(&instrument.base),
                    net::percent_encode(&instrument.quote)
                ),
            }),
        }
    }

    fn history<'a>(
        &'a self,
        instrument: &'a Instrument,
        interval: u32,
        end: Option<f64>,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Candle>, Error>> {
        Box::pin(async move {
            let name = INTERVALS
                .iter()
                .find(|(secs, ..)| *secs == interval)
                .map(|(_, _, name, _)| *name)
                .ok_or_else(|| Error::Message(format!("不支持的周期：{interval} 秒")))?;
            let mut path = format!(
                "/api/v3/klines?symbol={}&interval={name}&limit={}",
                net::percent_encode(&instrument.symbol),
                limit.clamp(1, MAX_PAGE)
            );
            if let Some(end) = end {
                path.push_str(&format!("&endTime={}", (end * 1000.0) as i64 - 1));
            }
            let klines: Vec<Kline> = get(&path).await?;
            Ok(klines.into_iter().map(|k| k.0).collect())
        })
    }

    fn recent_trades<'a>(
        &'a self,
        instrument: &'a Instrument,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Trade>, Error>> {
        Box::pin(async move {
            let path = format!(
                "/api/v3/aggTrades?symbol={}&limit={}",
                net::percent_encode(&instrument.symbol),
                limit.clamp(1, MAX_PAGE)
            );
            let trades: Vec<RawTrade> = get(&path).await?;
            Ok(trades.iter().map(RawTrade::trade).collect())
        })
    }

    fn stream(
        &self,
        instrument: Instrument,
        stop: oneshot::Receiver<()>,
        events: Channel<LiveEvent>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(stream::run(instrument, stop, events))
    }
}

/// GETs a public endpoint, trying each host in turn.
async fn get<T: DeserializeOwned>(path: &str) -> Result<T, Error> {
    let mut failure = None;
    for host in REST_HOSTS {
        match http::get_json(&format!("https://{host}{path}")).await {
            Ok(value) => return Ok(value),
            Err(e) => failure = Some(e),
        }
    }
    Err(failure.expect("at least one host").into())
}

// Search: a ranked match over the list of pairs, loaded on first use.

/// A tradable spot pair.
struct Pair {
    symbol: String,
    base: String,
    quote: String,
    /// Decimals of the tick size.
    decimals: u8,
}

impl Pair {
    fn instrument(&self) -> Instrument {
        Instrument {
            provider: ProviderId::Binance,
            symbol: self.symbol.clone(),
            base: self.base.clone(),
            quote: self.quote.clone(),
            name: None,
            decimals: Some(self.decimals),
            pinned: false,
        }
    }
}

const EXCHANGE_INFO: &str =
    "/api/v3/exchangeInfo?permissions=SPOT&symbolStatus=TRADING&showPermissionSets=false";
/// Listings change rarely; one download serves searches for this long.
const PAIRS_TTL: Duration = Duration::from_secs(30 * 60);

static PAIRS: Mutex<Option<(Instant, Arc<Vec<Pair>>)>> = Mutex::new(None);
/// Moves on when the cache is dropped, so a download still running then
/// doesn't fill it again for nobody.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// One download at a time: searches typed meanwhile wait for it.
static DOWNLOAD: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

async fn pairs() -> Result<Arc<Vec<Pair>>, Error> {
    let cached = || {
        let pairs = PAIRS.lock().unwrap_or_else(|e| e.into_inner());
        pairs.as_ref().filter(|(at, _)| at.elapsed() < PAIRS_TTL).map(|(_, list)| list.clone())
    };
    if let Some(list) = cached() {
        return Ok(list);
    }
    let _download = DOWNLOAD.lock().await;
    if let Some(list) = cached() {
        return Ok(list);
    }
    let generation = GENERATION.load(Ordering::Acquire);
    let info: ExchangeInfo = get(EXCHANGE_INFO).await?;
    let list: Arc<Vec<Pair>> = Arc::new(info.symbols.into_iter().map(RawSymbol::pair).collect());
    let mut cached = PAIRS.lock().unwrap_or_else(|e| e.into_inner());
    if GENERATION.load(Ordering::Acquire) == generation {
        *cached = Some((Instant::now(), list.clone()));
    }
    Ok(list)
}

#[derive(Deserialize)]
struct ExchangeInfo {
    symbols: Vec<RawSymbol>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSymbol {
    symbol: String,
    base_asset: String,
    quote_asset: String,
    filters: Vec<RawFilter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFilter {
    filter_type: String,
    tick_size: Option<String>,
}

impl RawSymbol {
    fn pair(self) -> Pair {
        let tick = self
            .filters
            .iter()
            .find(|f| f.filter_type == "PRICE_FILTER")
            .and_then(|f| f.tick_size.as_deref());
        Pair {
            symbol: self.symbol,
            base: self.base_asset,
            quote: self.quote_asset,
            decimals: tick_decimals(tick),
        }
    }
}

/// `"0.01000000"` → 2, `"1.00000000"` → 0
fn tick_decimals(tick_size: Option<&str>) -> u8 {
    let fraction = tick_size.and_then(|t| t.split_once('.')).map_or("", |(_, f)| f);
    fraction.trim_end_matches('0').len() as u8
}

const SEARCH_LIMIT: usize = 8;
const QUOTE_RANK: [&str; 6] = ["USDT", "USDC", "FDUSD", "BTC", "ETH", "BNB"];

async fn search(query: &str) -> Search {
    match pairs().await {
        Ok(list) => Search {
            candidates: rank(&list, &normalize(query))
                .into_iter()
                .map(|pair| Candidate { instrument: pair.instrument(), manual: false })
                .collect(),
            notes: Vec::new(),
        },
        Err(e) => {
            log::warn!("cannot load Binance pairs: {e}");
            Search {
                candidates: parse_pair(query)
                    .map(|instrument| Candidate { instrument, manual: true })
                    .into_iter()
                    .collect(),
                notes: vec!["无法获取币安交易对列表，请输入完整交易对，如 SOLUSDT".to_owned()],
            }
        }
    }
}

/// Exact base or symbol first, then prefixes; within each, the common quotes.
fn rank<'a>(pairs: &'a [Pair], query: &str) -> Vec<&'a Pair> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(usize, &Pair)> = pairs
        .iter()
        .filter_map(|pair| {
            let matched = if pair.base == query || pair.symbol == query {
                0
            } else if pair.base.starts_with(query) {
                1
            } else if pair.symbol.starts_with(query) {
                2
            } else {
                return None;
            };
            let quote =
                QUOTE_RANK.iter().position(|q| *q == pair.quote).unwrap_or(QUOTE_RANK.len());
            Some((matched * 100 + quote, pair))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.symbol.cmp(&b.1.symbol)));
    ranked.into_iter().take(SEARCH_LIMIT).map(|(_, pair)| pair).collect()
}

const MANUAL_QUOTES: [&str; 10] =
    ["FDUSD", "USDT", "USDC", "USD1", "TUSD", "BTC", "ETH", "BNB", "EUR", "TRY"];

/// Without the pair list: `SOL/USDT`, or `SOLUSDT` split at a known quote
/// asset. Decimals stay unknown, so the app picks them by magnitude.
fn parse_pair(input: &str) -> Option<Instrument> {
    let raw = input.trim().to_uppercase();
    let (base, quote) = match raw.split_once('/') {
        Some((base, quote)) => (normalize(base), normalize(quote)),
        None => {
            let symbol = normalize(&raw);
            let quote =
                MANUAL_QUOTES.iter().find(|q| symbol.ends_with(*q) && symbol.len() > q.len())?;
            (symbol[..symbol.len() - quote.len()].to_owned(), (*quote).to_owned())
        }
    };
    // The rule `validate` enforces: letters and digits only (any script).
    let valid = |s: &str| !s.is_empty() && s.chars().all(char::is_alphanumeric);
    (valid(&base) && valid(&quote)).then(|| Instrument {
        provider: ProviderId::Binance,
        symbol: format!("{base}{quote}"),
        base,
        quote,
        name: None,
        decimals: None,
        pinned: false,
    })
}

/// Upper case without spaces or the separators people type in pair names.
fn normalize(s: &str) -> String {
    s.trim()
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '/' | '_' | '-'))
        .collect()
}

// Wire formats shared by REST and the websocket.

/// `[openTime, "open", "high", "low", "close", …]`
struct Kline(Candle);

impl<'de> Deserialize<'de> for Kline {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KlineVisitor;

        impl<'de> Visitor<'de> for KlineVisitor {
            type Value = Kline;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a kline array")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Kline, A::Error> {
                let missing = || de::Error::custom("short kline");
                let open_ms: i64 = seq.next_element()?.ok_or_else(missing)?;
                let mut price = || -> Result<f64, A::Error> {
                    let text: String = seq.next_element()?.ok_or_else(missing)?;
                    text.parse().map_err(de::Error::custom)
                };
                let (open, high, low, close) = (price()?, price()?, price()?, price()?);
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Kline(Candle { time: open_ms as f64 / 1000.0, open, high, low, close }))
            }
        }

        deserializer.deserialize_seq(KlineVisitor)
    }
}

/// An aggregate trade, over REST and the websocket alike.
#[derive(Deserialize)]
struct RawTrade {
    #[serde(rename = "a")]
    id: u64,
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "q")]
    qty: String,
    #[serde(rename = "T")]
    time: i64,
    /// The buyer was the maker, so the taker sold.
    #[serde(rename = "m")]
    buyer_maker: bool,
}

impl RawTrade {
    fn trade(&self) -> Trade {
        Trade {
            id: self.id,
            price: self.price.parse().unwrap_or(f64::NAN),
            qty: self.qty.parse().unwrap_or(f64::NAN),
            time: self.time as f64,
            sell: self.buyer_maker,
            extended: false,
        }
    }
}

/// Top of the book as `[price, qty]` pairs, best first.
#[derive(Deserialize)]
struct RawBook {
    bids: Vec<(String, String)>,
    asks: Vec<(String, String)>,
}

impl RawBook {
    fn book(&self) -> super::Book {
        let levels = |side: &[(String, String)]| {
            side.iter()
                .map(|(price, qty)| super::Level {
                    price: price.parse().unwrap_or(f64::NAN),
                    qty: qty.parse().unwrap_or(f64::NAN),
                })
                .collect()
        };
        super::Book { bids: levels(&self.bids), asks: levels(&self.asks) }
    }
}

/// `GET /api/v3/ticker/24hr`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestTicker {
    last_price: String,
    high_price: String,
    low_price: String,
    volume: String,
    quote_volume: String,
    price_change: String,
    price_change_percent: String,
}

impl RestTicker {
    fn stats(&self) -> Stats {
        stats([
            &self.last_price,
            &self.high_price,
            &self.low_price,
            &self.volume,
            &self.quote_volume,
            &self.price_change,
            &self.price_change_percent,
        ])
    }
}

/// The `<symbol>@ticker` stream.
#[derive(Deserialize)]
struct WsTicker {
    c: String,
    h: String,
    l: String,
    v: String,
    q: String,
    p: String,
    #[serde(rename = "P")]
    pct: String,
}

impl WsTicker {
    fn stats(&self) -> Stats {
        stats([&self.c, &self.h, &self.l, &self.v, &self.q, &self.p, &self.pct])
    }
}

/// Last, high, low, volume, turnover, change, change %.
fn stats(fields: [&String; 7]) -> Stats {
    let [last, high, low, volume, turnover, change, change_pct] =
        fields.map(|s| s.parse().unwrap_or(f64::NAN));
    Stats { last, high, low, volume, turnover, change, change_pct }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(symbol: &str, base: &str, quote: &str) -> Pair {
        Pair { symbol: symbol.into(), base: base.into(), quote: quote.into(), decimals: 2 }
    }

    #[test]
    fn ranks_exact_then_prefix_then_quote() {
        let pairs = [
            pair("SOLBTC", "SOL", "BTC"),
            pair("SOLVUSDT", "SOLV", "USDT"),
            pair("SOLUSDT", "SOL", "USDT"),
            pair("SOLUSDC", "SOL", "USDC"),
        ];
        let symbols: Vec<&str> = rank(&pairs, "SOL").iter().map(|p| p.symbol.as_str()).collect();
        assert_eq!(symbols, ["SOLUSDT", "SOLUSDC", "SOLBTC", "SOLVUSDT"]);
    }

    #[test]
    fn manual_pairs_split_at_a_known_quote() {
        let sol = parse_pair(" solusdt").unwrap();
        assert_eq!(
            (sol.base.as_str(), sol.quote.as_str(), sol.symbol.as_str()),
            ("SOL", "USDT", "SOLUSDT")
        );
        let aeur = parse_pair("sol/aeur").unwrap();
        assert_eq!((aeur.base.as_str(), aeur.quote.as_str()), ("SOL", "AEUR"));
        assert!(parse_pair("USDT").is_none());
        assert_eq!(tick_decimals(Some("0.01000000")), 2);
        assert_eq!(tick_decimals(Some("1.00000000")), 0);
    }

    #[test]
    fn klines_keep_the_first_five_fields() {
        let json =
            r#"[[1700000000000,"1.5","2","1","1.75","100",1700000059999,"150",3,"50","75","0"]]"#;
        let klines: Vec<Kline> = serde_json::from_str(json).unwrap();
        let c = klines[0].0;
        assert_eq!(
            (c.time, c.open, c.high, c.low, c.close),
            (1_700_000_000.0, 1.5, 2.0, 1.0, 1.75)
        );
    }
}
