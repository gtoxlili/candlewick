//! GeckoTerminal's public API (`api.geckoterminal.com/api/v2`): a pool's
//! candles and trades, which DEX Screener doesn't publish. Keyless, and
//! strictly rate limited: the docs say 30 calls a minute, from here about
//! five before it answers 429 for most of the minute. A 429 closes a gate
//! for as long as it asks (`Retry-After`, else [`DEFAULT_WAIT`]); requests
//! meanwhile fail at once, without reaching it, so a chart retrying its
//! history doesn't spend the budget on refusals.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, de::DeserializeOwned};

use crate::{
    calendar, http,
    market::{Candle, Error, Trade},
    net,
};

const HOST: &str = "https://api.geckoterminal.com/api/v2";
/// Pins the response format to the version this code reads.
const ACCEPT: &str = "application/json;version=20230302";
/// Candles per request, the most it gives.
pub const MAX_PAGE: usize = 1000;
const DEFAULT_WAIT: Duration = Duration::from_secs(30);
const MAX_WAIT: Duration = Duration::from_secs(120);

/// Closed until this instant after a 429.
static GATE: Mutex<Option<Instant>> = Mutex::new(None);

/// How much longer the gate stays closed, if it is.
pub fn blocked() -> Option<Duration> {
    let gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    gate.and_then(|until| until.checked_duration_since(Instant::now())).filter(|d| !d.is_zero())
}

fn throttled(wait: Duration) -> Error {
    Error::Message(t!(
        "error.rateLimited",
        source = "GeckoTerminal",
        seconds = wait.as_secs().max(1)
    ))
}

async fn get<T: DeserializeOwned>(path: &str) -> Result<T, Error> {
    if let Some(left) = blocked() {
        return Err(throttled(left));
    }
    match http::get_json_with(&format!("{HOST}{path}"), &[("Accept", ACCEPT)]).await {
        Ok(value) => Ok(value),
        Err(http::Error::Throttled(retry_after)) => {
            let wait = retry_after.map_or(DEFAULT_WAIT, Duration::from_secs).min(MAX_WAIT);
            *GATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + wait);
            log::info!("geckoterminal rate limited, pausing for {}s", wait.as_secs());
            Err(throttled(wait))
        }
        Err(e) => Err(e.into()),
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
}
