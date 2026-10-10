//! A pool's candles and trades, which DEX Screener doesn't publish. Two ways
//! in to the same data:
//!
//! - GeckoTerminal's public API, keyless. Its docs say 30 calls a minute;
//!   from here about five before it answers 429 for most of the minute, a
//!   limit shared by everyone behind one IP.
//! - CoinGecko's API with the user's own key: the same `/networks/…` paths
//!   under `/api/v3/onchain`, on the host the key's plan uses. A Demo key
//!   (100 calls a minute, 10,000 a month) goes to `api.coingecko.com`, a
//!   paid one to `pro-api.coingecko.com`. Which it is, and a paid plan's
//!   limits, come from probing the key once ([`probe`]); a Demo key can't
//!   read its plan (`/key` is paid-only), so it is paced as a Demo key.
//!
//! A 429 closes the route that got it for as long as it asks
//! (`Retry-After`, else [`DEFAULT_WAIT`]); requests meanwhile take the
//! other route or fail at once without reaching the network, so a chart
//! retrying its history doesn't spend the budget on refusals. A paid plan
//! keeps a reserve of its monthly credit for history, out of polling's reach.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    calendar, http,
    market::{Candle, Error, Trade},
    net,
};

const PUBLIC_HOST: &str = "https://api.geckoterminal.com/api/v2";
const DEMO_HOST: &str = "https://api.coingecko.com/api/v3";
const PRO_HOST: &str = "https://pro-api.coingecko.com/api/v3";
const ONCHAIN: &str = "/onchain";
const DEMO_HEADER: &str = "x-cg-demo-api-key";
const PRO_HEADER: &str = "x-cg-pro-api-key";
/// Pins GeckoTerminal's response format to the version this code reads.
const ACCEPT: &str = "application/json;version=20230302";
/// Candles per request, the most either gives.
pub const MAX_PAGE: usize = 1000;
const DEFAULT_WAIT: Duration = Duration::from_secs(30);
const MAX_WAIT: Duration = Duration::from_secs(120);
/// A key that couldn't be probed (network, a refusal) is tried again after this.
const PROBE_RETRY: Duration = Duration::from_secs(60);
/// How often a paid plan's usage is read again, so the reserve holds.
const RECHECK: Duration = Duration::from_secs(6 * 3600);
/// Monthly credit a paid plan keeps for history and searches; below it,
/// polling goes the public way.
const RESERVE: u32 = 200;
/// CoinGecko's error codes (docs.coingecko.com/docs/errors-and-rate-limits).
const PLAN_RESTRICTED: u32 = 10005;
const PRO_KEY_ON_DEMO_HOST: u32 = 10010;

/// What a CoinGecko key turned out to be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "tier")]
pub enum Access {
    /// A Demo key: 100 calls a minute and 10,000 a month, which it can't report.
    Demo,
    /// A paid plan, as `/key` describes it.
    Paid(Plan),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// `Basic`, `Analyst`, `Lite`, `Pro`…
    #[serde(rename = "plan")]
    pub name: String,
    #[serde(rename = "rate_limit_request_per_minute")]
    pub per_minute: u32,
    #[serde(rename = "monthly_call_credit")]
    pub monthly: u32,
    #[serde(rename = "current_remaining_monthly_calls")]
    pub remaining: u32,
}

/// How often the chart's stream may ask, by the way in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    /// Between trade polls while trades keep coming…
    pub trades: Duration,
    /// …and at most while the pool is quiet.
    pub trades_max: Duration,
    /// Between reads of the hourly candles for the day's range.
    pub range: Duration,
}

/// Keyless, and a Demo key: data turns over every 30 to 60 seconds.
const PUBLIC_PACE: Pace = Pace {
    trades: Duration::from_secs(20),
    trades_max: Duration::from_secs(60),
    range: Duration::from_secs(600),
};
/// Paid plans refresh every 10 seconds.
const PAID_PACE: Pace = Pace {
    trades: Duration::from_secs(10),
    trades_max: Duration::from_secs(30),
    range: Duration::from_secs(300),
};

struct State {
    key: Option<String>,
    /// What the key was found to be; `None` until probed.
    access: Option<Access>,
    /// Not before this: the last probe failed.
    probe_after: Option<Instant>,
    /// When a paid plan's usage was read.
    checked: Option<Instant>,
    /// Closed until then after a 429.
    public_gate: Option<Instant>,
    keyed_gate: Option<Instant>,
}

static STATE: Mutex<State> = Mutex::new(State {
    key: None,
    access: None,
    probe_after: None,
    checked: None,
    public_gate: None,
    keyed_gate: None,
});
/// One probe at a time: requests arriving meanwhile wait for its answer.
static PROBING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The key to use from now on (`None` for none): probed on first use.
pub fn configure(key: Option<String>) {
    let mut s = state();
    if s.key != key {
        s.access = None;
        s.checked = None;
        s.probe_after = None;
        s.keyed_gate = None;
    }
    s.key = key;
}

/// What the configured key was last found to be, if it has been probed.
pub fn access() -> Option<Access> {
    state().access.clone()
}

/// Probes a key as the settings page asks, and remembers the answer if it
/// is the configured key's.
pub async fn check(key: &str) -> Result<Access, Error> {
    let found = probe(key).await?;
    let mut s = state();
    if s.key.as_deref() == Some(key) {
        s.access = Some(found.clone());
        s.checked = Some(Instant::now());
        s.probe_after = None;
    }
    Ok(found)
}

/// How often the stream may poll with the way in it has now.
pub fn pace() -> Pace {
    match route(&state()) {
        Route::Keyed { access: Access::Paid(_), .. } => PAID_PACE,
        _ => PUBLIC_PACE,
    }
}

/// How much longer the way in that would be used next stays closed, if it is.
pub fn blocked() -> Option<Duration> {
    let s = state();
    let gate = match route(&s) {
        Route::Keyed { .. } => s.keyed_gate,
        Route::Public | Route::Unknown => s.public_gate,
    };
    gate.and_then(|until| until.checked_duration_since(Instant::now())).filter(|d| !d.is_zero())
}

/// The way in a request takes.
enum Route {
    Public,
    Keyed {
        host: &'static str,
        header: &'static str,
        key: String,
        access: Access,
    },
    /// A key is configured but not yet probed.
    Unknown,
}

fn route(s: &State) -> Route {
    let Some(key) = &s.key else { return Route::Public };
    let now = Instant::now();
    if s.keyed_gate.is_some_and(|until| until > now) {
        return Route::Public;
    }
    match &s.access {
        None => {
            if s.probe_after.is_some_and(|after| after > now) {
                Route::Public
            } else {
                Route::Unknown
            }
        }
        Some(Access::Paid(plan)) if plan.remaining <= RESERVE => Route::Public,
        Some(Access::Paid(plan)) if s.checked.is_some_and(|at| now - at > RECHECK) => {
            let _ = plan;
            Route::Unknown
        }
        Some(access) => {
            let (host, header) = match access {
                Access::Demo => (DEMO_HOST, DEMO_HEADER),
                Access::Paid(_) => (PRO_HOST, PRO_HEADER),
            };
            Route::Keyed { host, header, key: key.clone(), access: access.clone() }
        }
    }
}

/// The route to use now, probing the key first if it hasn't been.
async fn resolve() -> Route {
    loop {
        let (key, known) = {
            let s = state();
            match route(&s) {
                Route::Unknown => (s.key.clone(), false),
                other => return other,
            }
        };
        let Some(key) = key else { return Route::Public };
        let _probing = PROBING.lock().await;
        // Another request may have probed while this one waited.
        {
            let s = state();
            if !matches!(route(&s), Route::Unknown) {
                continue;
            }
        }
        let _ = known;
        let result = probe(&key).await;
        let mut s = state();
        if s.key.as_deref() != Some(key.as_str()) {
            continue;
        }
        match result {
            Ok(access) => {
                log::info!("coingecko key: {}", describe(&access));
                s.access = Some(access);
                s.checked = Some(Instant::now());
                s.probe_after = None;
            }
            Err(e) => {
                log::warn!("coingecko key could not be checked: {e}");
                // A paid plan's stale figures still beat none.
                if s.access.is_none() {
                    s.probe_after = Some(Instant::now() + PROBE_RETRY);
                } else {
                    s.checked = Some(Instant::now());
                }
            }
        }
    }
}

fn describe(access: &Access) -> String {
    match access {
        Access::Demo => "Demo plan".to_owned(),
        Access::Paid(plan) => format!(
            "{} plan, {}/min, {} of {} calls left this month",
            plan.name, plan.per_minute, plan.remaining, plan.monthly
        ),
    }
}

/// What a key is: the Demo host first, where most keys belong. `/key` is
/// paid-only, so a Demo key is refused as plan-restricted, which still says
/// it is a key; a paid key is refused as being on the wrong host, and asked
/// again on its own.
async fn probe(key: &str) -> Result<Access, Error> {
    match request::<Plan>(DEMO_HOST, "/key", &[(DEMO_HEADER, key)]).await {
        Reply::Ok(plan) => Ok(classify(plan)),
        Reply::Refused { code: PLAN_RESTRICTED, .. } => Ok(Access::Demo),
        Reply::Refused { code: PRO_KEY_ON_DEMO_HOST, .. } => {
            match request::<Plan>(PRO_HOST, "/key", &[(PRO_HEADER, key)]).await {
                Reply::Ok(plan) => Ok(classify(plan)),
                Reply::Refused { message, .. } => Err(rejected(message)),
                Reply::Failed(e) => Err(e.into()),
            }
        }
        Reply::Refused { message, .. } => Err(rejected(message)),
        Reply::Failed(e) => Err(e.into()),
    }
}

/// `/key` answering on the Demo host would name the Demo plan itself.
fn classify(plan: Plan) -> Access {
    if plan.name.eq_ignore_ascii_case("demo") { Access::Demo } else { Access::Paid(plan) }
}

fn rejected(message: String) -> Error {
    if message.is_empty() {
        Error::Message(t!("error.coingeckoKey").to_owned())
    } else {
        Error::Message(t!("error.sourceSays", source = "CoinGecko", message = message))
    }
}

/// What a request came back with.
enum Reply<T> {
    Ok(T),
    /// CoinGecko said no, with its code (0 when the body had none).
    Refused {
        code: u32,
        message: String,
    },
    Failed(http::Error),
}

/// `{"status": {"error_code": 10011, "error_message": "…"}}`
#[derive(Deserialize)]
struct Refusal {
    status: RefusalStatus,
}

#[derive(Deserialize)]
struct RefusalStatus {
    #[serde(default)]
    error_code: u32,
    #[serde(default)]
    error_message: String,
}

async fn request<T: DeserializeOwned>(
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> Reply<T> {
    let url = format!("{host}{path}");
    let response = match http::send(|client| {
        headers
            .iter()
            .fold(client.get(&url), |request, (name, value)| request.header(*name, *value))
    })
    .await
    {
        Ok(response) => response,
        Err(e) => return Reply::Failed(e),
    };
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        return Reply::Failed(http::Error::Throttled(retry_after));
    }
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(e) => return Reply::Failed(e.into()),
    };
    if status.is_success() {
        return match serde_json::from_slice(&body) {
            Ok(value) => Reply::Ok(value),
            Err(_) => Reply::Failed(http::Error::Format),
        };
    }
    match serde_json::from_slice::<Refusal>(&body) {
        Ok(refusal) => Reply::Refused {
            code: refusal.status.error_code,
            message: refusal.status.error_message,
        },
        Err(_) => Reply::Failed(http::Error::Status(status.as_u16())),
    }
}

fn throttled(source: &str, wait: Duration) -> Error {
    Error::Message(t!("error.rateLimited", source = source, seconds = wait.as_secs().max(1)))
}

/// GETs a `/networks/…` path the way in that is open now.
async fn get<T: DeserializeOwned>(path: &str) -> Result<T, Error> {
    match resolve().await {
        Route::Keyed { host, header, key, access } => {
            let reply = request(host, &format!("{ONCHAIN}{path}"), &[(header, &key)]).await;
            match reply {
                Reply::Ok(value) => {
                    if let Access::Paid(_) = access
                        && let Some(Access::Paid(plan)) = &mut state().access
                    {
                        plan.remaining = plan.remaining.saturating_sub(1);
                    }
                    Ok(value)
                }
                Reply::Failed(http::Error::Throttled(retry_after)) => {
                    let wait = retry_after.map_or(DEFAULT_WAIT, Duration::from_secs).min(MAX_WAIT);
                    state().keyed_gate = Some(Instant::now() + wait);
                    log::info!("coingecko rate limited, public route for {}s", wait.as_secs());
                    Err(throttled("CoinGecko", wait))
                }
                Reply::Refused { message, .. } => {
                    // Out of credit, or the key stopped working: the public
                    // route for a while, and a fresh look at the key after.
                    let mut s = state();
                    s.keyed_gate = Some(Instant::now() + MAX_WAIT);
                    s.access = None;
                    Err(rejected(message))
                }
                Reply::Failed(e) => Err(e.into()),
            }
        }
        Route::Public | Route::Unknown => {
            if let Some(left) = state()
                .public_gate
                .and_then(|until| until.checked_duration_since(Instant::now()))
                .filter(|d| !d.is_zero())
            {
                return Err(throttled("GeckoTerminal", left));
            }
            match request(PUBLIC_HOST, path, &[("Accept", ACCEPT)]).await {
                Reply::Ok(value) => Ok(value),
                Reply::Failed(http::Error::Throttled(retry_after)) => {
                    let wait = retry_after.map_or(DEFAULT_WAIT, Duration::from_secs).min(MAX_WAIT);
                    state().public_gate = Some(Instant::now() + wait);
                    log::info!("geckoterminal rate limited, pausing for {}s", wait.as_secs());
                    Err(throttled("GeckoTerminal", wait))
                }
                Reply::Refused { message, .. } => Err(Error::Message(t!(
                    "error.sourceSays",
                    source = "GeckoTerminal",
                    message = message
                ))),
                Reply::Failed(e) => Err(e.into()),
            }
        }
    }
}

/// Candles in USD, oldest first: the latest `limit`, or those that opened
/// before `end`. `timeframe` is `minute`, `hour` or `day` and `aggregate`
/// how many of them per candle. Intervals without a swap come as flat
/// candles at the previous close, as exchanges list them.
pub async fn candles(
    network: &str,
    address: &str,
    timeframe: &str,
    aggregate: u32,
    end: Option<f64>,
    limit: usize,
) -> Result<Vec<Candle>, Error> {
    let mut path = format!(
        "/networks/{}/pools/{}/ohlcv/{timeframe}?aggregate={aggregate}&limit={}&currency=usd&include_empty_intervals=true",
        net::percent_encode(network),
        net::percent_encode(address),
        limit.clamp(1, MAX_PAGE)
    );
    if let Some(end) = end {
        path.push_str(&format!("&before_timestamp={}", end as i64));
    }
    let response: Ohlcv = get(&path).await?;
    let mut candles: Vec<Candle> = response
        .data
        .attributes
        .ohlcv_list
        .iter()
        .filter_map(|row| match row[..] {
            [time, open, high, low, close, ..] => Some(Candle { time, open, high, low, close }),
            _ => None,
        })
        .filter(|c| c.low > 0.0 && end.is_none_or(|end| c.time < end))
        .collect();
    candles.sort_by(|a, b| a.time.total_cmp(&b.time));
    candles.dedup_by(|a, b| a.time == b.time);
    Ok(candles)
}

#[derive(Deserialize)]
struct Ohlcv {
    data: OhlcvData,
}

#[derive(Deserialize)]
struct OhlcvData {
    attributes: OhlcvAttributes,
}

#[derive(Deserialize)]
struct OhlcvAttributes {
    /// `[time, open, high, low, close, volume]`, newest first.
    ohlcv_list: Vec<Vec<f64>>,
}

/// The pool's last 300 trades of the past 24 hours, oldest first.
pub async fn trades(network: &str, address: &str) -> Result<Vec<Trade>, Error> {
    let path = format!(
        "/networks/{}/pools/{}/trades",
        net::percent_encode(network),
        net::percent_encode(address)
    );
    let response: Trades = get(&path).await?;
    let mut trades: Vec<Trade> = response.data.iter().filter_map(RawTrade::trade).collect();
    trades.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.id.cmp(&b.id)));
    Ok(trades)
}

#[derive(Deserialize)]
struct Trades {
    data: Vec<RawTrade>,
}

#[derive(Deserialize)]
struct RawTrade {
    /// `solana_<block>_<tx hash>_<index>_<timestamp>`: one per swap.
    id: String,
    attributes: TradeAttributes,
}

#[derive(Deserialize)]
struct TradeAttributes {
    /// `2026-10-10T17:46:25Z`
    block_timestamp: String,
    /// `buy` or `sell`, of the base token.
    kind: String,
    from_token_amount: String,
    to_token_amount: String,
    price_from_in_usd: String,
    price_to_in_usd: String,
}

impl RawTrade {
    /// A buy swaps the quote token for the base token, so the base token's
    /// price and amount are the `to` side; a sell's are the `from` side.
    fn trade(&self) -> Option<Trade> {
        let a = &self.attributes;
        let secs = epoch(&a.block_timestamp)?;
        let buy = a.kind == "buy";
        let (price, qty) = if buy {
            (&a.price_to_in_usd, &a.to_token_amount)
        } else {
            (&a.price_from_in_usd, &a.from_token_amount)
        };
        let price: f64 = price.parse().ok()?;
        let qty: f64 = qty.parse().ok()?;
        if !(price.is_finite() && price > 0.0 && qty.is_finite()) {
            return None;
        }
        Some(Trade {
            // Ordered by time, told apart within a second by the swap's
            // own id; the same swap gets the same id on every poll.
            id: (secs as u64) << 20 | (fnv1a(self.id.as_bytes()) & 0xf_ffff),
            price,
            qty,
            time: secs as f64 * 1000.0,
            sell: !buy,
            extended: false,
        })
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Epoch seconds of an RFC 3339 UTC timestamp (`2026-10-10T17:46:25Z`,
/// fractions allowed).
fn epoch(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let time = time.trim_end_matches('Z').split(['.', '+']).next()?;
    let mut time = time.split(':');
    let hour: i64 = time.next()?.parse().ok()?;
    let minute: i64 = time.next()?.parse().ok()?;
    let second: i64 = time.next()?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    Some(calendar::days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_parse_to_epoch_seconds() {
        assert_eq!(epoch("2026-10-10T17:46:25Z"), Some(1_791_654_385));
        assert_eq!(epoch("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(epoch("2026-13-01T00:00:00Z"), None);
        assert_eq!(epoch("yesterday"), None);
    }

    #[test]
    fn trades_take_the_base_side_and_stable_ordered_ids() {
        let json = r#"{"data":[
          {"id":"solana_455340556_45BS_30_1791653923","type":"trade","attributes":{"block_number":455340556,"block_timestamp":"2026-10-10T17:38:36Z","tx_hash":"45BS","kind":"buy","from_token_amount":"1.316965484","to_token_amount":"662.570033","price_from_in_usd":"109.91","price_to_in_usd":"0.218478","price_from_in_currency_token":"1.0","price_to_in_currency_token":"0.00198","volume_in_usd":"144.75"}},
          {"id":"solana_455342702_yQfo_10_1791654466","type":"trade","attributes":{"block_number":455342702,"block_timestamp":"2026-10-10T17:46:25Z","tx_hash":"yQfo","kind":"sell","from_token_amount":"6.253593","to_token_amount":"0.012368535","price_from_in_usd":"0.217794","price_to_in_usd":"110.11","price_from_in_currency_token":"0.00197","price_to_in_currency_token":"1.0","volume_in_usd":"1.36"}}
        ]}"#;
        let trades: Trades = serde_json::from_str(json).unwrap();
        let mut list: Vec<Trade> = trades.data.iter().filter_map(RawTrade::trade).collect();
        list.sort_by(|a, b| a.time.total_cmp(&b.time));
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].price, list[0].qty, list[0].sell), (0.218478, 662.570033, false));
        assert_eq!((list[1].price, list[1].qty, list[1].sell), (0.217794, 6.253593, true));
        assert!(list[0].id < list[1].id && list[1].id < 1 << 53);
        assert_eq!(trades.data[1].trade().unwrap().id, list[1].id);
    }

    #[test]
    fn candle_rows_read_in_order() {
        let json = r#"{"data":{"id":"x","type":"ohlcv_request_response","attributes":{"ohlcv_list":[[1791654360,0.2184,0.2184,0.2177,0.2177,1.36],[1791653880,0.2185,0.2185,0.2184,0.2184,144.7]]}},"meta":{}}"#;
        let parsed: Ohlcv = serde_json::from_str(json).unwrap();
        let rows = parsed.data.attributes.ohlcv_list;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0], 1_791_653_880.0);
    }

    // `/key` as CoinGecko answers it, and its refusals.
    #[test]
    fn plans_and_refusals_parse() {
        let json = r#"{"plan":"Analyst","rate_limit_request_per_minute":500,"monthly_call_credit":500000,"current_total_monthly_calls":2,"current_remaining_monthly_calls":499998,"api_key_rate_limit_request_per_minute":500,"api_key_monthly_call_credit":500000,"api_key_current_total_monthly_calls":2}"#;
        let plan: Plan = serde_json::from_str(json).unwrap();
        assert_eq!(
            (plan.name.as_str(), plan.per_minute, plan.monthly, plan.remaining),
            ("Analyst", 500, 500_000, 499_998)
        );
        assert_eq!(classify(plan.clone()), Access::Paid(plan));
        let demo: Plan = serde_json::from_str(r#"{"plan":"Demo","rate_limit_request_per_minute":100,"monthly_call_credit":10000,"current_remaining_monthly_calls":9000}"#).unwrap();
        assert_eq!(classify(demo), Access::Demo);
        let refusal: Refusal =
            serde_json::from_str(r#"{"status":{"error_code":10011,"error_message":"wrong host"}}"#)
                .unwrap();
        assert_eq!(
            (refusal.status.error_code, refusal.status.error_message.as_str()),
            (10011, "wrong host")
        );
        assert_eq!(serde_json::to_string(&Access::Demo).unwrap(), r#"{"tier":"demo"}"#);
    }

    // A key goes its way once known; a paid plan near its credit's end polls the public way.
    #[test]
    fn routes_follow_the_key_and_its_credit() {
        let plan =
            |remaining| Plan { name: "Basic".into(), per_minute: 300, monthly: 100_000, remaining };
        let mut s = State {
            key: None,
            access: None,
            probe_after: None,
            checked: None,
            public_gate: None,
            keyed_gate: None,
        };
        assert!(matches!(route(&s), Route::Public));
        s.key = Some("k".into());
        assert!(matches!(route(&s), Route::Unknown));
        s.probe_after = Some(Instant::now() + Duration::from_secs(60));
        assert!(matches!(route(&s), Route::Public));
        s.probe_after = None;
        s.access = Some(Access::Demo);
        assert!(matches!(route(&s), Route::Keyed { host: DEMO_HOST, header: DEMO_HEADER, .. }));
        s.access = Some(Access::Paid(plan(5000)));
        s.checked = Some(Instant::now());
        assert!(matches!(route(&s), Route::Keyed { host: PRO_HOST, header: PRO_HEADER, .. }));
        s.access = Some(Access::Paid(plan(RESERVE)));
        assert!(matches!(route(&s), Route::Public));
        s.access = Some(Access::Paid(plan(5000)));
        s.keyed_gate = Some(Instant::now() + Duration::from_secs(30));
        assert!(matches!(route(&s), Route::Public));
    }
}
