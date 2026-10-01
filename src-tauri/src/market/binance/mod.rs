//! Binance spot: public market data, no account or key. With the user's
//! API key, also their account (`account.rs`).

mod account;
mod depth;
mod live;

use std::{borrow::Cow, fmt};

use serde::{
    Deserialize, Deserializer,
    de::{self, IgnoredAny, SeqAccess, Visitor},
};

use super::{
    Candle, Error, Link, ProviderId, Stats, Trade,
    crypto::{self, Exchange, Interval, Pair, Tick},
};
use crate::{model::Instrument, net};

/// Binance's most klines or trades per request.
const MAX_PAGE: usize = 1000;

pub struct Binance;

impl Exchange for Binance {
    const ID: ProviderId = ProviderId::Binance;
    const REST_HOSTS: &'static [&'static str] = &["api.binance.com", "data-api.binance.vision"];
    const SOCKETS: &'static [(&'static str, u16)] =
        &[("stream.binance.com", 443), ("data-stream.binance.vision", 443)];
    // Binance pings every 20 seconds itself.
    const PING: Option<&'static str> = None;
    const INTERVALS: &'static [Interval] = &[
        Interval::new(1, "1秒", "1s"),
        Interval::new(60, "1分", "1m"),
        Interval::new(300, "5分", "5m"),
        Interval::new(900, "15分", "15m"),
        Interval::new(3600, "1小时", "1h"),
        Interval::new(14_400, "4小时", "4h"),
        Interval::new(86_400, "1日", "1d"),
    ];

    type Live = live::Live;

    fn symbol(base: &str, quote: &str) -> String {
        format!("{base}{quote}")
    }

    fn link(instrument: &Instrument) -> Link {
        Link {
            label: "在币安打开",
            url: format!(
                "https://www.binance.com/zh-CN/trade/{}_{}?type=spot",
                net::percent_encode(&instrument.base),
                net::percent_encode(&instrument.quote)
            ),
        }
    }

    async fn pairs() -> Result<Vec<Pair>, Error> {
        let info: ExchangeInfo = get(EXCHANGE_INFO).await?;
        Ok(info.symbols.into_iter().map(RawSymbol::pair).collect())
    }

    async fn history(
        instrument: &Instrument,
        interval: &Interval,
        end: Option<f64>,
        limit: usize,
    ) -> Result<Vec<Candle>, Error> {
        let mut path = format!(
            "/api/v3/klines?symbol={}&interval={}&limit={}",
            net::percent_encode(&instrument.symbol),
            interval.name,
            limit.clamp(1, MAX_PAGE)
        );
        if let Some(end) = end {
            path.push_str(&format!("&endTime={}", (end * 1000.0) as i64 - 1));
        }
        let klines: Vec<Kline> = get(&path).await?;
        Ok(klines.into_iter().map(|k| k.0).collect())
    }

    async fn recent_trades(instrument: &Instrument, limit: usize) -> Result<Vec<Trade>, Error> {
        let path = format!(
            "/api/v3/aggTrades?symbol={}&limit={}",
            net::percent_encode(&instrument.symbol),
            limit.clamp(1, MAX_PAGE)
        );
        let trades: Vec<RawTrade> = get(&path).await?;
        Ok(trades.iter().map(RawTrade::trade).collect())
    }

    /// `/stream?streams=btcusdt@miniTicker/ethusdt@miniTicker`
    fn quotes_path(symbols: &[String]) -> String {
        let streams: Vec<String> = symbols
            .iter()
            // Symbols can be non-ASCII (e.g. 币安人生USDT).
            .map(|symbol| format!("{}@miniTicker", net::percent_encode(&symbol.to_lowercase())))
            .collect();
        format!("/stream?streams={}", streams.join("/"))
    }

    fn quotes_subscribe(_: &[String]) -> Vec<String> {
        Vec::new()
    }

    fn quote(text: &str) -> Option<Tick<'_>> {
        let Envelope { data } = serde_json::from_str::<Envelope<MiniTicker>>(text).ok()?;
        Some(Tick {
            symbol: data.symbol,
            last: crypto::num(&data.last),
            open: crypto::num(&data.open),
        })
    }

    fn live(instrument: &Instrument) -> live::Live {
        live::Live::new(instrument)
    }
}

async fn get<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, Error> {
    crypto::get::<Binance, T>(path).await
}

const EXCHANGE_INFO: &str =
    "/api/v3/exchangeInfo?permissions=SPOT&symbolStatus=TRADING&showPermissionSets=false";

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
            .and_then(|f| f.tick_size.as_deref())
            .unwrap_or_default()
            .to_owned();
        Pair::new(self.symbol, self.base_asset, self.quote_asset, &tick)
    }
}

// Wire formats shared by REST and the websocket.

/// A combined stream's frame.
#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

/// The `<symbol>@miniTicker` stream.
#[derive(Deserialize)]
struct MiniTicker<'a> {
    #[serde(rename = "s", borrow)]
    symbol: Cow<'a, str>,
    #[serde(rename = "c", borrow)]
    last: Cow<'a, str>,
    #[serde(rename = "o", borrow)]
    open: Cow<'a, str>,
}

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
            price: crypto::num(&self.price),
            qty: crypto::num(&self.qty),
            time: self.time as f64,
            sell: self.buyer_maker,
            extended: false,
        }
    }
}

/// `GET /api/v3/ticker/24hr`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestTicker {
    last_price: String,
    open_price: String,
    high_price: String,
    low_price: String,
    volume: String,
    quote_volume: String,
}

impl RestTicker {
    fn stats(&self) -> Stats {
        crypto::stats(
            crypto::num(&self.last_price),
            crypto::num(&self.open_price),
            crypto::num(&self.high_price),
            crypto::num(&self.low_price),
            crypto::num(&self.volume),
            crypto::num(&self.quote_volume),
        )
    }
}

/// The `<symbol>@ticker` stream.
#[derive(Deserialize)]
struct WsTicker {
    c: String,
    o: String,
    h: String,
    l: String,
    v: String,
    q: String,
}

impl WsTicker {
    fn stats(&self) -> Stats {
        let [last, open, high, low, volume, turnover] =
            [&self.c, &self.o, &self.h, &self.l, &self.v, &self.q].map(|s| crypto::num(s));
        crypto::stats(last, open, high, low, volume, turnover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn quotes_come_from_mini_tickers() {
        assert_eq!(
            Binance::quotes_path(&["BTCUSDT".into(), "币安人生USDT".into()]),
            "/stream?streams=btcusdt@miniTicker/%E5%B8%81%E5%AE%89%E4%BA%BA%E7%94%9Fusdt@miniTicker"
        );
        let frame = r#"{"stream":"btcusdt@miniTicker","data":{"e":"24hrMiniTicker","s":"BTCUSDT","c":"84000.5","o":"83000"}}"#;
        let tick = Binance::quote(frame).unwrap();
        assert_eq!((&*tick.symbol, tick.last, tick.open), ("BTCUSDT", 84_000.5, 83_000.0));
    }
}
