//! OKX spot: public market data, no account or key. With the user's
//! read-only key, also their account (`account.rs`).

mod account;

use std::{borrow::Cow, convert::Infallible};

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{json, value::RawValue};

use super::{
    Candle, Error, Link, LiveEvent, ProviderId, Trade,
    crypto::{self, Entry, Exchange, Interval, Ladder, Out, Pair, Session, Tick, Units},
};
use crate::{model::Instrument, net};

/// OKX's most candles per request.
const MAX_CANDLES: usize = 300;
/// OKX's most recent trades per request.
const MAX_TRADES: usize = 500;
/// Levels per side the `books` channel keeps, every 100 ms.
const BOOK_DEPTH: usize = 400;

pub struct Okx;

impl Exchange for Okx {
    const ID: ProviderId = ProviderId::Okx;
    const REST_HOSTS: &'static [&'static str] = &["openapi.okx.com", "www.okx.com"];
    // 8443 is the documented port; 443 answers too, for proxies that only
    // tunnel to it.
    const SOCKETS: &'static [(&'static str, u16)] = &[("ws.okx.com", 8443), ("ws.okx.com", 443)];
    const PING: Option<&'static str> = Some("ping");
    // `1Dutc`: plain `1D` candles open at 00:00 in Hong Kong.
    const INTERVALS: &'static [Interval] = &[
        Interval::new(1, "1秒", "1s"),
        Interval::new(60, "1分", "1m"),
        Interval::new(300, "5分", "5m"),
        Interval::new(900, "15分", "15m"),
        Interval::new(3600, "1小时", "1H"),
        Interval::new(14_400, "4小时", "4H"),
        Interval::new(86_400, "1日", "1Dutc"),
    ];

    type Live = Live;

    fn symbol(base: &str, quote: &str) -> String {
        format!("{base}-{quote}")
    }

    fn link(instrument: &Instrument) -> Link {
        Link {
            label: "在 OKX 打开",
            url: format!(
                "https://www.okx.com/zh-hans/trade-spot/{}",
                net::percent_encode(&instrument.symbol.to_lowercase())
            ),
        }
    }

    async fn pairs() -> Result<Vec<Pair>, Error> {
        let instruments: Vec<RawInstrument> =
            get("/api/v5/public/instruments?instType=SPOT").await?;
        Ok(instruments
            .into_iter()
            .filter(|i| i.state == "live")
            .map(|i| Pair::new(i.inst_id, i.base_ccy, i.quote_ccy, &i.tick_sz))
            .collect())
    }

    /// The latest candles come from `candles`, older ones from
    /// `history-candles`, which reaches back years.
    async fn history(
        instrument: &Instrument,
        interval: &Interval,
        end: Option<f64>,
        limit: usize,
    ) -> Result<Vec<Candle>, Error> {
        let query = format!(
            "instId={}&bar={}&limit={}",
            net::percent_encode(&instrument.symbol),
            interval.name,
            limit.clamp(1, MAX_CANDLES)
        );
        let path = match end {
            // `after`: those that opened before it.
            Some(end) => {
                format!("/api/v5/market/history-candles?{query}&after={}", (end * 1000.0) as i64)
            }
            None => format!("/api/v5/market/candles?{query}"),
        };
        let rows: Vec<Vec<String>> = get(&path).await?;
        // Newest first.
        Ok(rows.iter().rev().filter_map(|row| crypto::candle(row)).collect())
    }

    async fn recent_trades(instrument: &Instrument, limit: usize) -> Result<Vec<Trade>, Error> {
        let path = format!(
            "/api/v5/market/trades?instId={}&limit={}",
            net::percent_encode(&instrument.symbol),
            limit.clamp(1, MAX_TRADES)
        );
        let trades: Vec<RawTrade> = get(&path).await?;
        // Newest first.
        Ok(trades.iter().rev().filter_map(RawTrade::trade).collect())
    }

    fn quotes_path(_: &[String]) -> String {
        PUBLIC.to_owned()
    }

    /// An unknown pair fails on its own, so one request carries them all.
    fn quotes_subscribe(symbols: &[String]) -> Vec<String> {
        let args: Vec<_> = symbols.iter().map(|symbol| arg("tickers", symbol)).collect();
        vec![request("subscribe", &args)]
    }

    fn quote(text: &str) -> Option<Tick<'_>> {
        let push: Push = serde_json::from_str(text).ok()?;
        if push.arg?.channel != "tickers" {
            return None;
        }
        let [ticker]: [WsTicker; 1] = serde_json::from_str(push.data?.get()).ok()?;
        Some(Tick {
            symbol: ticker.inst_id,
            last: crypto::num(&ticker.last),
            open: crypto::num(&ticker.open_24h),
        })
    }

    fn live(instrument: &Instrument) -> Live {
        Live {
            inst_id: instrument.symbol.clone(),
            steps: crypto::book_steps(instrument),
            ladder: Ladder::default(),
            seq: None,
        }
    }
}

/// The public channels.
const PUBLIC: &str = "/ws/v5/public";

fn arg(channel: &str, inst_id: &str) -> serde_json::Value {
    json!({ "channel": channel, "instId": inst_id })
}

fn request(op: &str, args: &[serde_json::Value]) -> String {
    json!({ "op": op, "args": args }).to_string()
}

/// GETs an endpoint's `data`.
async fn get<T: DeserializeOwned>(path: &str) -> Result<Vec<T>, Error> {
    let reply: Reply<T> = crypto::get::<Okx, _>(path).await?;
    if reply.code != "0" {
        return Err(Error::Message(format!("OKX：{}", reply.msg)));
    }
    Ok(reply.data)
}

/// `{"code": "0", "msg": "", "data": […]}`
#[derive(Deserialize)]
struct Reply<T> {
    code: String,
    msg: String,
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawInstrument {
    inst_id: String,
    base_ccy: String,
    quote_ccy: String,
    tick_sz: String,
    state: String,
}

/// A trade over REST, or a taker order's fills at one price over the
/// `trades` channel, where the id is the last fill's.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTrade {
    trade_id: String,
    px: String,
    sz: String,
    /// The taker's.
    side: String,
    ts: String,
}

impl RawTrade {
    fn trade(&self) -> Option<Trade> {
        Some(Trade {
            id: self.trade_id.parse().ok()?,
            price: crypto::num(&self.px),
            qty: crypto::num(&self.sz),
            time: crypto::num(&self.ts),
            sell: self.side == "sell",
            extended: false,
        })
    }
}

/// A frame of the public channels: a push, or an event such as an error.
#[derive(Deserialize)]
struct Push<'a> {
    #[serde(borrow)]
    event: Option<Cow<'a, str>>,
    #[serde(borrow)]
    msg: Option<Cow<'a, str>>,
    #[serde(borrow)]
    arg: Option<Arg<'a>>,
    #[serde(borrow)]
    action: Option<Cow<'a, str>>,
    #[serde(borrow)]
    data: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct Arg<'a> {
    #[serde(borrow)]
    channel: Cow<'a, str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsTicker<'a> {
    #[serde(borrow)]
    inst_id: Cow<'a, str>,
    last: String,
    open_24h: String,
    high_24h: String,
    low_24h: String,
    /// In the base asset.
    vol_24h: String,
    /// In the quote asset, for spot.
    vol_ccy_24h: String,
}

/// The `books` channel: all levels on a snapshot, the changed ones on an
/// update.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WsBook {
    bids: Vec<Entry>,
    asks: Vec<Entry>,
    seq_id: i64,
    /// The `seq_id` of the message before; equal to it on a heartbeat with no
    /// changes, and greater after a reset.
    prev_seq_id: i64,
}

pub struct Live {
    inst_id: String,
    steps: Vec<Units>,
    ladder: Ladder,
    /// The `seq_id` of the last book message applied; none while waiting for
    /// a snapshot.
    seq: Option<i64>,
}

impl Live {
    fn book(&mut self, action: &str, book: WsBook, out: &mut Out<Infallible>) {
        match (action, self.seq) {
            ("snapshot", _) => self.ladder.replace(&book.bids, &book.asks, BOOK_DEPTH),
            ("update", Some(seq)) if book.prev_seq_id == seq => {
                if book.bids.is_empty() && book.asks.is_empty() {
                    // A heartbeat.
                    self.seq = Some(book.seq_id);
                    return;
                }
                self.ladder.update_best(&book.bids, &book.asks, BOOK_DEPTH);
            }
            // Waiting for the snapshot a new subscription brings.
            ("update", None) => return,
            ("update", Some(_)) => {
                // A message went missing: subscribing again brings a snapshot.
                log::info!("order book for {} lost sync", self.inst_id);
                self.seq = None;
                let args = [arg("books", &self.inst_id)];
                out.send(request("unsubscribe", &args));
                out.send(request("subscribe", &args));
                return;
            }
            _ => return,
        }
        self.seq = Some(book.seq_id);
        out.event(crypto::book_event(&self.ladder, &self.steps));
    }
}

impl Session for Live {
    type Fetched = Infallible;

    fn path(&self) -> String {
        PUBLIC.to_owned()
    }

    fn connected(&mut self, out: &mut Out<Infallible>) {
        self.ladder.clear();
        self.seq = None;
        let args = ["trades", "books", "tickers"].map(|channel| arg(channel, &self.inst_id));
        out.send(request("subscribe", &args));
    }

    fn frame(&mut self, text: &str, out: &mut Out<Infallible>) {
        // The answer to our keepalive.
        if text == "pong" {
            return;
        }
        let Ok(push) = serde_json::from_str::<Push>(text) else { return };
        if push.event.as_deref() == Some("error") {
            log::warn!("OKX refused a request: {}", push.msg.unwrap_or_default());
            return;
        }
        let (Some(arg), Some(data)) = (push.arg, push.data) else { return };
        let data = data.get();
        match &*arg.channel {
            "trades" => {
                let Ok(trades) = serde_json::from_str::<Vec<RawTrade>>(data) else { return };
                for trade in trades.iter().filter_map(RawTrade::trade) {
                    out.trade(trade);
                }
            }
            "books" => {
                let Ok([book]) = serde_json::from_str::<[WsBook; 1]>(data) else { return };
                self.book(push.action.as_deref().unwrap_or_default(), book, out);
            }
            "tickers" => {
                let Ok([t]) = serde_json::from_str::<[WsTicker; 1]>(data) else { return };
                let stats = crypto::stats(
                    crypto::num(&t.last),
                    crypto::num(&t.open_24h),
                    crypto::num(&t.high_24h),
                    crypto::num(&t.low_24h),
                    crypto::num(&t.vol_24h),
                    crypto::num(&t.vol_ccy_24h),
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
        Live { inst_id: "BTC-USDT".into(), steps: Vec::new(), ladder: Ladder::default(), seq: None }
    }

    fn book_frame(action: &str, prev: i64, seq: i64, asks: &str) -> String {
        format!(
            r#"{{"arg":{{"channel":"books","instId":"BTC-USDT"}},"action":"{action}","data":[{{"asks":{asks},"bids":[],"ts":"1","checksum":0,"prevSeqId":{prev},"seqId":{seq}}}]}}"#
        )
    }

    fn asks(events: Vec<LiveEvent>) -> Vec<Vec<(f64, f64)>> {
        events
            .iter()
            .map(|event| match event {
                LiveEvent::Book { books } => prices(&books[0].asks),
                _ => panic!("not a book"),
            })
            .collect()
    }

    #[test]
    fn book_messages_chain_by_sequence() {
        let (mut live, mut out) = (live(), Out::for_test());
        live.connected(&mut out);
        assert_eq!(
            out.take_frames(),
            [
                r#"{"args":[{"channel":"trades","instId":"BTC-USDT"},{"channel":"books","instId":"BTC-USDT"},{"channel":"tickers","instId":"BTC-USDT"}],"op":"subscribe"}"#
            ]
        );
        live.frame(
            &book_frame("snapshot", -1, 10, r#"[["101","1","0","1"],["102","2","0","3"]]"#),
            &mut out,
        );
        live.frame(&book_frame("update", 10, 15, r#"[["101","0","0","0"]]"#), &mut out);
        // A heartbeat changes nothing; after a reset the sequence goes down.
        live.frame(&book_frame("update", 15, 15, "[]"), &mut out);
        live.frame(&book_frame("update", 15, 3, r#"[["103","1","0","1"]]"#), &mut out);
        assert_eq!(
            asks(out.take_events()),
            [
                vec![(101.0, 1.0), (102.0, 2.0)],
                vec![(102.0, 2.0)],
                vec![(102.0, 2.0), (103.0, 1.0)]
            ]
        );
        // 3 → 8 skips whatever came between.
        live.frame(&book_frame("update", 8, 9, "[]"), &mut out);
        assert!(out.take_events().is_empty());
        assert_eq!(
            out.take_frames(),
            [
                r#"{"args":[{"channel":"books","instId":"BTC-USDT"}],"op":"unsubscribe"}"#,
                r#"{"args":[{"channel":"books","instId":"BTC-USDT"}],"op":"subscribe"}"#,
            ]
        );
        live.frame(&book_frame("update", 9, 10, r#"[["104","1","0","1"]]"#), &mut out);
        assert!(out.take_events().is_empty());
    }

    #[test]
    fn trades_tickers_and_rest_rows() {
        let (mut live, mut out) = (live(), Out::for_test());
        live.frame("pong", &mut out);
        live.frame(
            r#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","tradeId":"880785954","px":"2698.7","sz":"0.01571","side":"sell","ts":"1790827943174","count":"2","source":"0","seqId":1}]}"#,
            &mut out,
        );
        let trades = out.take_trades();
        assert_eq!((trades[0].id, trades[0].qty, trades[0].sell), (880_785_954, 0.01571, true));

        let ticker = r#"{"arg":{"channel":"tickers","instId":"BTC-USDT"},"data":[{"instType":"SPOT","instId":"BTC-USDT","last":"90","lastSz":"1","askPx":"91","askSz":"1","bidPx":"89","bidSz":"1","open24h":"100","high24h":"110","low24h":"80","sodUtc0":"95","sodUtc8":"96","volCcy24h":"9000","vol24h":"100","ts":"1"}]}"#;
        live.frame(ticker, &mut out);
        let LiveEvent::Stats { stats } = out.take_events().remove(0) else { panic!("no stats") };
        assert_eq!(
            (stats.change, stats.change_pct, stats.volume, stats.turnover),
            (-10.0, -10.0, 100.0, 9000.0)
        );
        let tick = Okx::quote(ticker).unwrap();
        assert_eq!((&*tick.symbol, tick.last, tick.open), ("BTC-USDT", 90.0, 100.0));
        assert!(Okx::quote(r#"{"event":"subscribe","arg":{"channel":"tickers","instId":"BTC-USDT"},"connId":"a"}"#).is_none());

        let row: Vec<String> =
            ["1790827740000", "1", "2", "0.5", "1.5", "2.3", "193797.4", "193797.4", "0"]
                .map(String::from)
                .into();
        let c = crypto::candle(&row).unwrap();
        assert_eq!((c.time, c.open, c.high, c.low, c.close), (1_790_827_740.0, 1.0, 2.0, 0.5, 1.5));
        assert_eq!(Okx::symbol("BTC", "USDT"), "BTC-USDT");
    }
}
