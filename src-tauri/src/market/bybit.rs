//! Bybit spot over the V5 API: public market data, no account or key.

use std::{borrow::Cow, convert::Infallible};

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{json, value::RawValue};

use super::{
    Candle, Error, Link, LiveEvent, ProviderId, Trade,
    crypto::{self, Entry, Exchange, Interval, Ladder, Out, Pair, Session, Tick, Units},
};
use crate::{http, model::Instrument, net};

/// Bybit's most klines per request.
const MAX_KLINES: usize = 1000;
/// Bybit's most recent spot trades per request.
const MAX_TRADES: usize = 60;
/// Levels per side the order book stream keeps, every 200 ms. Spot offers 1,
/// 50, 200 and 1000; the most reach far enough to group by 1000 ticks.
const BOOK_DEPTH: usize = 1000;

pub struct Bybit;

impl Exchange for Bybit {
    const ID: ProviderId = ProviderId::Bybit;
    const REST_HOSTS: &'static [&'static str] = &["api.bybit.com", "api.bytick.com"];
    const SOCKETS: &'static [(&'static str, u16)] =
        &[("stream.bybit.com", 443), ("stream.bytick.com", 443)];
    const PING: Option<&'static str> = Some(r#"{"op":"ping"}"#);
    // Bybit has no one-second klines.
    const INTERVALS: &'static [Interval] = &[
        Interval::new(60, "1分", "1"),
        Interval::new(300, "5分", "5"),
        Interval::new(900, "15分", "15"),
        Interval::new(3600, "1小时", "60"),
        Interval::new(14_400, "4小时", "240"),
        Interval::new(86_400, "1日", "D"),
    ];

    type Live = Live;

    fn symbol(base: &str, quote: &str) -> String {
        format!("{base}{quote}")
    }

    fn link(instrument: &Instrument) -> Link {
        Link {
            label: "在 Bybit 打开",
            url: format!(
                "https://www.bybit.com/trade/spot/{}/{}",
                net::percent_encode(&instrument.base),
                net::percent_encode(&instrument.quote)
            ),
        }
    }

    async fn pairs() -> Result<Vec<Pair>, Error> {
        let list: List<RawInstrument> = get("/v5/market/instruments-info?category=spot").await?;
        Ok(list
            .list
            .into_iter()
            .filter(|i| i.status == "Trading")
            .map(|i| Pair::new(i.symbol, i.base_coin, i.quote_coin, &i.price_filter.tick_size))
            .collect())
    }

    async fn history(
        instrument: &Instrument,
        interval: &Interval,
        end: Option<f64>,
        limit: usize,
    ) -> Result<Vec<Candle>, Error> {
        let mut path = format!(
            "/v5/market/kline?category=spot&symbol={}&interval={}&limit={}",
            net::percent_encode(&instrument.symbol),
            interval.name,
            limit.clamp(1, MAX_KLINES)
        );
        // `end` includes candles that open at it.
        if let Some(end) = end {
            path.push_str(&format!("&end={}", (end * 1000.0) as i64 - 1));
        }
        let list: List<Vec<String>> = get(&path).await?;
        // Newest first.
        Ok(list.list.iter().rev().filter_map(|row| crypto::candle(row)).collect())
    }

    async fn recent_trades(instrument: &Instrument, limit: usize) -> Result<Vec<Trade>, Error> {
        let path = format!(
            "/v5/market/recent-trade?category=spot&symbol={}&limit={}",
            net::percent_encode(&instrument.symbol),
            limit.clamp(1, MAX_TRADES)
        );
        let list: List<RestTrade> = get(&path).await?;
        // Newest first.
        Ok(list.list.iter().rev().filter_map(RestTrade::trade).collect())
    }

    fn quotes_path(_: &[String]) -> String {
        SPOT.to_owned()
    }

    /// One request per pair: one unknown symbol fails a request whole.
    fn quotes_subscribe(symbols: &[String]) -> Vec<String> {
        symbols
            .iter()
            .map(|symbol| subscribe("subscribe", &[format!("tickers.{symbol}")]))
            .collect()
    }

    fn quote(text: &str) -> Option<Tick<'_>> {
        let push: Push = serde_json::from_str(text).ok()?;
        if !push.topic?.starts_with("tickers.") {
            return None;
        }
        let ticker: WsTicker = serde_json::from_str(push.data?.get()).ok()?;
        Some(Tick {
            symbol: ticker.symbol,
            last: crypto::num(&ticker.last_price),
            open: crypto::num(&ticker.prev_price_24h),
        })
    }

    fn live(instrument: &Instrument) -> Live {
        Live {
            symbol: instrument.symbol.clone(),
            steps: crypto::book_steps(instrument),
            ladder: Ladder::default(),
            update: None,
        }
    }
}

/// The public spot stream.
const SPOT: &str = "/v5/public/spot";

fn subscribe(op: &str, topics: &[String]) -> String {
    json!({ "op": op, "args": topics }).to_string()
}

/// GETs a V5 endpoint's `result`.
async fn get<T: DeserializeOwned>(path: &str) -> Result<T, Error> {
    let reply: Reply = crypto::get::<Bybit, _>(path).await?;
    if reply.ret_code != 0 {
        return Err(Error::Message(format!("Bybit：{}", reply.ret_msg)));
    }
    serde_json::from_str(reply.result.get()).map_err(|_| http::Error::Format.into())
}

/// `{"retCode": 0, "retMsg": "OK", "result": …}`; the result is `{}` on errors.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    ret_code: i64,
    ret_msg: String,
    result: Box<RawValue>,
}

#[derive(Deserialize)]
struct List<T> {
    list: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawInstrument {
    symbol: String,
    base_coin: String,
    quote_coin: String,
    status: String,
    price_filter: PriceFilter,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceFilter {
    tick_size: String,
}

/// Bybit numbers spot trades `{engine}{16-digit counter}`, past what the
/// webview's numbers hold exactly. The counter alone fits and still grows
/// with every trade of a pair.
fn trade_id(exec_id: &str) -> Option<u64> {
    exec_id.parse::<u64>().ok().map(|id| id % 10u64.pow(16))
}

/// `GET /v5/market/recent-trade`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestTrade {
    exec_id: String,
    price: String,
    size: String,
    /// The taker's.
    side: String,
    time: String,
}

impl RestTrade {
    fn trade(&self) -> Option<Trade> {
        Some(Trade {
            id: trade_id(&self.exec_id)?,
            price: crypto::num(&self.price),
            qty: crypto::num(&self.size),
            time: crypto::num(&self.time),
            sell: self.side == "Sell",
            extended: false,
        })
    }
}

/// A frame of the public stream: a topic's push, or a reply to a request.
#[derive(Deserialize)]
struct Push<'a> {
    #[serde(borrow)]
    topic: Option<Cow<'a, str>>,
    #[serde(rename = "type", borrow)]
    kind: Option<Cow<'a, str>>,
    #[serde(borrow)]
    data: Option<&'a RawValue>,
    success: Option<bool>,
    #[serde(borrow)]
    ret_msg: Option<Cow<'a, str>>,
}

/// `tickers.{symbol}`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsTicker<'a> {
    #[serde(borrow)]
    symbol: Cow<'a, str>,
    last_price: String,
    prev_price_24h: String,
    high_price_24h: String,
    low_price_24h: String,
    volume_24h: String,
    turnover_24h: String,
}

/// `publicTrade.{symbol}`
#[derive(Deserialize)]
struct WsTrade {
    #[serde(rename = "i")]
    id: String,
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "v")]
    qty: String,
    #[serde(rename = "T")]
    time: i64,
    /// The taker's.
    #[serde(rename = "S")]
    side: String,
}

/// `orderbook.{depth}.{symbol}`: the levels on a snapshot, the changed ones
/// on a delta.
#[derive(Deserialize)]
struct WsBook {
    #[serde(rename = "b")]
    bids: Vec<Entry>,
    #[serde(rename = "a")]
    asks: Vec<Entry>,
    /// Goes up by one with every message.
    #[serde(rename = "u")]
    update: u64,
}

pub struct Live {
    symbol: String,
    steps: Vec<Units>,
    ladder: Ladder,
    /// The last book message applied; none while waiting for a snapshot.
    update: Option<u64>,
}

impl Live {
    fn book_topic(&self) -> String {
        format!("orderbook.{BOOK_DEPTH}.{}", self.symbol)
    }

    fn book(&mut self, kind: &str, book: WsBook, out: &mut Out<Infallible>) {
        match (kind, self.update) {
            ("snapshot", _) => self.ladder.replace(&book.bids, &book.asks, BOOK_DEPTH),
            ("delta", Some(last)) if book.update == last + 1 => {
                self.ladder.update_best(&book.bids, &book.asks, BOOK_DEPTH);
            }
            // Waiting for the snapshot a new subscription brings.
            ("delta", None) => return,
            ("delta", Some(_)) => {
                // A message went missing: subscribing again brings a snapshot.
                log::info!("order book for {} lost sync", self.symbol);
                self.update = None;
                let topic = [self.book_topic()];
                out.send(subscribe("unsubscribe", &topic));
                out.send(subscribe("subscribe", &topic));
                return;
            }
            _ => return,
        }
        self.update = Some(book.update);
        out.event(crypto::book_event(&self.ladder, &self.steps));
    }
}

impl Session for Live {
    type Fetched = Infallible;

    fn path(&self) -> String {
        SPOT.to_owned()
    }

    fn connected(&mut self, out: &mut Out<Infallible>) {
        self.ladder.clear();
        self.update = None;
        let symbol = &self.symbol;
        let topics =
            [format!("publicTrade.{symbol}"), self.book_topic(), format!("tickers.{symbol}")];
        out.send(subscribe("subscribe", &topics));
    }

    fn frame(&mut self, text: &str, out: &mut Out<Infallible>) {
        let Ok(push) = serde_json::from_str::<Push>(text) else { return };
        if push.success == Some(false) {
            log::warn!("Bybit refused a request: {}", push.ret_msg.unwrap_or_default());
            return;
        }
        let (Some(topic), Some(data)) = (push.topic, push.data) else { return };
        let data = data.get();
        match topic.split('.').next() {
            Some("publicTrade") => {
                let Ok(trades) = serde_json::from_str::<Vec<WsTrade>>(data) else { return };
                for trade in trades {
                    let Some(id) = trade_id(&trade.id) else { continue };
                    out.trade(Trade {
                        id,
                        price: crypto::num(&trade.price),
                        qty: crypto::num(&trade.qty),
                        time: trade.time as f64,
                        sell: trade.side == "Sell",
                        extended: false,
                    });
                }
            }
            Some("orderbook") => {
                if let Ok(book) = serde_json::from_str::<WsBook>(data) {
                    self.book(push.kind.as_deref().unwrap_or_default(), book, out);
                }
            }
            Some("tickers") => {
                let Ok(t) = serde_json::from_str::<WsTicker>(data) else { return };
                let stats = crypto::stats(
                    crypto::num(&t.last_price),
                    crypto::num(&t.prev_price_24h),
                    crypto::num(&t.high_price_24h),
                    crypto::num(&t.low_price_24h),
                    crypto::num(&t.volume_24h),
                    crypto::num(&t.turnover_24h),
                );
                out.event(LiveEvent::Stats { stats });
            }
            _ => {}
        }
    }

    fn fetched(&mut self, fetched: Infallible, _: &mut Out<Infallible>) {
        match fetched {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::crypto::book_tests::prices;

    fn live() -> Live {
        Live {
            symbol: "BTCUSDT".into(),
            steps: Vec::new(),
            ladder: Ladder::default(),
            update: None,
        }
    }

    fn book_frame(kind: &str, update: u64, bids: &str, asks: &str) -> String {
        format!(
            r#"{{"topic":"orderbook.1000.BTCUSDT","ts":1,"type":"{kind}","data":{{"s":"BTCUSDT","b":{bids},"a":{asks},"u":{update},"seq":9}},"cts":1}}"#
        )
    }

    fn bids(event: &LiveEvent) -> Vec<(f64, f64)> {
        match event {
            LiveEvent::Book { books } => prices(&books[0].bids),
            _ => panic!("not a book"),
        }
    }

    #[test]
    fn a_missed_book_message_subscribes_again() {
        let (mut live, mut out) = (live(), Out::for_test());
        live.connected(&mut out);
        assert_eq!(
            out.take_frames(),
            [
                r#"{"args":["publicTrade.BTCUSDT","orderbook.1000.BTCUSDT","tickers.BTCUSDT"],"op":"subscribe"}"#
            ]
        );
        // Deltas before the snapshot wait for it.
        live.frame(&book_frame("delta", 6, r#"[["99","1"]]"#, "[]"), &mut out);
        live.frame(
            &book_frame("snapshot", 7, r#"[["100","1"],["99","2"]]"#, r#"[["101","1"]]"#),
            &mut out,
        );
        live.frame(&book_frame("delta", 8, r#"[["100","0"]]"#, "[]"), &mut out);
        let events = out.take_events();
        assert_eq!(
            events.iter().map(bids).collect::<Vec<_>>(),
            [vec![(100.0, 1.0), (99.0, 2.0)], vec![(99.0, 2.0)]]
        );
        // 9 never came.
        live.frame(&book_frame("delta", 10, r#"[["98","1"]]"#, "[]"), &mut out);
        assert!(out.take_events().is_empty());
        assert_eq!(
            out.take_frames(),
            [
                r#"{"args":["orderbook.1000.BTCUSDT"],"op":"unsubscribe"}"#,
                r#"{"args":["orderbook.1000.BTCUSDT"],"op":"subscribe"}"#,
            ]
        );
        live.frame(&book_frame("delta", 11, r#"[["98","1"]]"#, "[]"), &mut out);
        live.frame(&book_frame("snapshot", 12, r#"[["97","3"]]"#, "[]"), &mut out);
        assert_eq!(out.take_events().iter().map(bids).collect::<Vec<_>>(), [vec![(97.0, 3.0)]]);
    }

    #[test]
    fn trades_and_tickers_come_from_their_topics() {
        let (mut live, mut out) = (live(), Out::for_test());
        live.frame(
            r#"{"topic":"publicTrade.BTCUSDT","ts":1,"type":"snapshot","data":[{"i":"2290000001221173210","T":1790827843818,"p":"83874.4","v":"0.000477","S":"Sell","s":"BTCUSDT","BT":false}]}"#,
            &mut out,
        );
        let trades = out.take_trades();
        assert_eq!(trades[0].id, 1_221_173_210);
        assert!(trades[0].sell);
        assert_eq!((trades[0].price, trades[0].time), (83_874.4, 1_790_827_843_818.0));

        let ticker = r#"{"topic":"tickers.BTCUSDT","ts":1,"type":"snapshot","cs":1,"data":{"symbol":"BTCUSDT","lastPrice":"110","highPrice24h":"120","lowPrice24h":"90","prevPrice24h":"100","volume24h":"5","turnover24h":"500","price24hPcnt":"0.1","usdIndexPrice":"110"}}"#;
        live.frame(ticker, &mut out);
        let LiveEvent::Stats { stats } = out.take_events().remove(0) else { panic!("no stats") };
        assert_eq!(
            (stats.last, stats.change, stats.change_pct, stats.turnover),
            (110.0, 10.0, 10.0, 500.0)
        );
        let tick = Bybit::quote(ticker).unwrap();
        assert_eq!((&*tick.symbol, tick.last, tick.open), ("BTCUSDT", 110.0, 100.0));
        // Replies to requests are not ticks.
        assert!(Bybit::quote(r#"{"success":true,"ret_msg":"pong","op":"ping"}"#).is_none());
    }

    #[test]
    fn trade_ids_keep_the_counter() {
        assert_eq!(trade_id("2280000001772638955"), Some(1_772_638_955));
        assert!(trade_id("x").is_none());
    }
}
