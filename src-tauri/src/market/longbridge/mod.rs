//! Longbridge: US, Hong Kong and China A-share stocks, through the user's own
//! OpenAPI credentials (see `credentials.rs`).

mod api;
mod clock;
mod proto;
mod service;
mod stream;

use serde::Serialize;
use tauri::{AppHandle, ipc::Channel};
use tokio::sync::{oneshot, watch};

use self::{
    clock::Market,
    proto::{adjust, cmd, period, trade_session},
};
use super::{
    BoxFuture, Candidate, Candle, ChartMode, ChartSpec, Error, IntervalSpec, Link, LiveEvent,
    Provider, ProviderId, Search, Trade,
};
use crate::{
    credentials::LongbridgeKeys,
    model::{FeedControl, Instrument},
};

pub struct Longbridge;

struct IntervalDef {
    secs: u32,
    label: &'static str,
    period: i32,
    /// Trades can be bucketed by time alone: every session starts on the
    /// half hour, so candles up to 30 minutes line up with the clock.
    aligned: bool,
    /// Only the regular session counts, as in Longbridge's own daily candles.
    regular_only: bool,
}

const INTERVALS: [IntervalDef; 7] = [
    IntervalDef {
        secs: 60,
        label: "1分",
        period: period::MIN_1,
        aligned: true,
        regular_only: false,
    },
    IntervalDef {
        secs: 300,
        label: "5分",
        period: period::MIN_5,
        aligned: true,
        regular_only: false,
    },
    IntervalDef {
        secs: 900,
        label: "15分",
        period: period::MIN_15,
        aligned: true,
        regular_only: false,
    },
    IntervalDef {
        secs: 1800,
        label: "30分",
        period: period::MIN_30,
        aligned: true,
        regular_only: false,
    },
    IntervalDef {
        secs: 3600,
        label: "1小时",
        period: period::MIN_60,
        aligned: false,
        regular_only: false,
    },
    IntervalDef {
        secs: 86_400,
        label: "1日",
        period: period::DAY,
        aligned: false,
        regular_only: true,
    },
    IntervalDef {
        secs: 604_800,
        label: "1周",
        period: period::WEEK,
        aligned: false,
        regular_only: true,
    },
];

/// Longbridge's most candles per request.
const MAX_PAGE: usize = 1000;

impl Provider for Longbridge {
    fn validate(&self, instrument: &mut Instrument) -> Result<(), String> {
        instrument.symbol = instrument.symbol.trim().to_uppercase();
        let invalid = || format!("无效的股票代码：{}", instrument.symbol);
        let (code, _) = instrument.symbol.rsplit_once('.').ok_or_else(invalid)?;
        let code_ok = !code.is_empty()
            && code.len() <= 12
            && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
        if market_of(&instrument.symbol).is_none() || !code_ok || instrument.base != code {
            return Err(invalid());
        }
        let currency_ok =
            instrument.quote.len() == 3 && instrument.quote.chars().all(|c| c.is_ascii_uppercase());
        if !currency_ok {
            return Err(invalid());
        }
        if let Some(name) = &mut instrument.name {
            *name = name.trim().chars().take(40).collect();
        }
        Ok(())
    }

    fn search<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Search> {
        Box::pin(search(query))
    }

    fn watch(
        &self,
        app: AppHandle,
        control: watch::Receiver<FeedControl>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(service::run(app, control))
    }

    fn chart_spec(&self, instrument: &Instrument) -> ChartSpec {
        let market = market_of(&instrument.symbol);
        ChartSpec {
            source: ProviderId::Longbridge.name(),
            intervals: INTERVALS
                .iter()
                .map(|i| IntervalSpec {
                    secs: i.secs,
                    label: i.label,
                    mode: ChartMode::Candle,
                    aligned: i.aligned,
                    regular_only: i.regular_only,
                })
                .collect(),
            stats_span: "今日",
            volume_unit: "股".to_owned(),
            turnover_unit: instrument.quote.clone(),
            // Daily candles open at the exchange's midnight; labels read its date.
            day_offset: market.map_or(0, |m| m.utc_offset(api::now())),
            // Ten levels at most, and tick sizes that change with the price:
            // nothing worth grouping.
            book_steps: Vec::new(),
            link: Some(Link {
                label: "在长桥打开",
                url: format!("https://longbridge.com/zh-CN/quote/{}", instrument.symbol),
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
        Box::pin(history(instrument, interval, end, limit))
    }

    fn recent_trades<'a>(
        &'a self,
        instrument: &'a Instrument,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Trade>, Error>> {
        Box::pin(async move {
            let request = proto::TradesRequest {
                symbol: instrument.symbol.clone(),
                count: limit.clamp(1, MAX_PAGE) as i32,
            };
            let response: proto::TradesResponse = service::call(cmd::TRADES, &request).await?;
            let mut trades: Vec<Trade> = response.trades.iter().map(stream::trade).collect();
            trades.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.id.cmp(&b.id)));
            Ok(trades)
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

/// What the account may see, from the last login.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub markets: Vec<MarketAccess>,
    /// When the access token expires, epoch seconds.
    pub token_expires: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketAccess {
    pub market: &'static str,
    /// Quote packages, e.g. `LV1 实时行情`.
    pub packages: Vec<String>,
    /// Why there are none, as Longbridge puts it.
    pub note: Option<String>,
}

impl Account {
    fn from_profile(profile: &proto::QuoteProfileResponse, keys: &LongbridgeKeys) -> Self {
        let markets = [("US", "美股"), ("HK", "港股"), ("CN", "A 股")]
            .into_iter()
            .filter_map(|(code, market)| {
                let detail = profile.quote_level_detail.as_ref()?.by_market_code.get(code)?;
                Some(MarketAccess {
                    market,
                    packages: detail.packages.iter().map(|p| p.name.clone()).collect(),
                    note: Some(detail.warning_msg.clone()).filter(|note| !note.is_empty()),
                })
            })
            .collect();
        Self { markets, token_expires: api::token_expiry(&keys.access_token) }
    }
}

/// The account, logging in to find out if needed.
pub async fn account() -> Result<Account, Error> {
    service::account().await
}

/// The account as of the last login, if there was one with these credentials.
pub fn last_account() -> Option<Account> {
    service::last_account()
}

/// New credentials make the last login's account meaningless.
pub fn forget_account() {
    service::forget_account();
}

/// The market a symbol trades in, by its suffix.
pub fn market_of(symbol: &str) -> Option<Market> {
    match symbol.rsplit_once('.')?.1 {
        "US" => Some(Market::Us),
        "HK" => Some(Market::Hk),
        "SH" | "SZ" => Some(Market::Cn),
        _ => None,
    }
}

async fn search(query: &str) -> Search {
    let symbols = guesses(query);
    if symbols.is_empty() {
        return Search::default();
    }
    let request = proto::MultiSecurityRequest { symbol: symbols };
    match service::call::<_, proto::StaticInfoResponse>(cmd::STATIC_INFO, &request).await {
        Ok(response) => Search {
            candidates: response
                .secu_static_info
                .into_iter()
                .filter_map(candidate)
                .map(|instrument| Candidate { instrument, manual: false })
                .collect(),
            notes: Vec::new(),
        },
        Err(e) => Search { candidates: Vec::new(), notes: vec![e.to_string()] },
    }
}

/// The symbols a typed code may stand for: letters are US tickers, up to five
/// digits a Hong Kong code, six digits a Shanghai or Shenzhen one.
fn guesses(query: &str) -> Vec<String> {
    let query: String =
        query.trim().to_uppercase().chars().filter(|c| !c.is_whitespace()).collect();
    if query.is_empty() || query.len() > 16 {
        return Vec::new();
    }
    if market_of(&query).is_some() {
        return vec![query];
    }
    if query.chars().all(|c| c.is_ascii_digit()) {
        return match query.len() {
            1..=5 => vec![format!("{}.HK", query.trim_start_matches('0'))]
                .into_iter()
                .filter(|s| s != ".HK")
                .collect(),
            6 => vec![format!("{query}.SH"), format!("{query}.SZ")],
            _ => Vec::new(),
        };
    }
    let ticker = query.starts_with(|c: char| c.is_ascii_alphabetic())
        && query.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
    if ticker { vec![format!("{query}.US")] } else { Vec::new() }
}

fn candidate(info: proto::StaticInfo) -> Option<Instrument> {
    let market = market_of(&info.symbol)?;
    let (code, _) = info.symbol.rsplit_once('.')?;
    let name = if info.name_cn.is_empty() { info.name_en } else { info.name_cn };
    Some(Instrument {
        provider: ProviderId::Longbridge,
        base: code.to_owned(),
        quote: info.currency,
        name: Some(name).filter(|n| !n.is_empty()),
        decimals: Some(decimals(market, code)),
        pinned: false,
        symbol: info.symbol,
    })
}

/// Price precision by market: cents in the US and for Chinese shares, a
/// tenth of that for Hong Kong quotes and Chinese funds (codes 1… and 5…).
fn decimals(market: Market, code: &str) -> u8 {
    match market {
        Market::Us => 2,
        Market::Hk => 3,
        Market::Cn if code.starts_with(['1', '5']) => 3,
        Market::Cn => 2,
    }
}

async fn history(
    instrument: &Instrument,
    interval: u32,
    end: Option<f64>,
    limit: usize,
) -> Result<Vec<Candle>, Error> {
    let def = INTERVALS
        .iter()
        .find(|i| i.secs == interval)
        .ok_or_else(|| Error::Message(format!("不支持的周期：{interval} 秒")))?;
    let market =
        market_of(&instrument.symbol).ok_or_else(|| Error::Message("无效的股票代码".to_owned()))?;
    // US minute candles include the extended sessions, as the menu bar does.
    let sessions = if market == Market::Us && !def.regular_only {
        trade_session::ALL
    } else {
        trade_session::INTRADAY
    };
    let adjust_type = if def.regular_only { adjust::FORWARD } else { adjust::NONE };
    let count = limit.clamp(1, MAX_PAGE) as i32;
    let symbol = instrument.symbol.clone();
    let response: proto::CandlesticksResponse = match end {
        None => {
            let request = proto::CandlesticksRequest {
                symbol,
                period: def.period,
                count,
                adjust_type,
                trade_session: sessions,
            };
            service::call(cmd::CANDLESTICKS, &request).await?
        }
        Some(end) => {
            // Backwards from just before `end`, in the exchange's time.
            let t = market.local(end as i64 - 1);
            let request = proto::HistoryCandlesticksRequest {
                symbol,
                period: def.period,
                adjust_type,
                query_type: 1,
                offset_request: Some(proto::OffsetQuery {
                    direction: 0,
                    date: format!("{:04}{:02}{:02}", t.year, t.month, t.day),
                    minute: format!("{:02}{:02}", t.hour, t.minute),
                    count,
                }),
                trade_session: sessions,
            };
            service::call(cmd::HISTORY_CANDLESTICKS, &request).await?
        }
    };
    let mut candles: Vec<Candle> = response
        .candlesticks
        .iter()
        .map(|c| Candle {
            time: c.timestamp as f64,
            open: decimal(&c.open),
            high: decimal(&c.high),
            low: decimal(&c.low),
            close: decimal(&c.close),
        })
        // A candle without prices (seen for indexes) can't be drawn.
        .filter(|c| c.low > 0.0 && c.high > 0.0)
        .filter(|c| end.is_none_or(|end| c.time < end))
        .collect();
    candles.sort_by(|a, b| a.time.total_cmp(&b.time));
    candles.dedup_by(|a, b| a.time == b.time);
    Ok(candles)
}

/// The latest price of a full quote and the close its change counts from:
/// the regular session's, or a US extended session's if that traded later.
fn latest(quote: &proto::SecurityQuote) -> (f64, f64, i32) {
    let mut latest = (
        quote.timestamp,
        decimal(&quote.last_done),
        decimal(&quote.prev_close),
        trade_session::INTRADAY,
    );
    for (session, extended) in [
        (trade_session::PRE, &quote.pre_market_quote),
        (trade_session::POST, &quote.post_market_quote),
        (trade_session::OVERNIGHT, &quote.over_night_quote),
    ] {
        if let Some(ext) = extended
            && ext.timestamp > latest.0
            && decimal(&ext.last_done) > 0.0
        {
            latest = (ext.timestamp, decimal(&ext.last_done), decimal(&ext.prev_close), session);
        }
    }
    (latest.1, latest.2, latest.3)
}

fn decimal(text: &str) -> f64 {
    text.parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_symbols_from_codes() {
        assert_eq!(guesses(" aapl "), ["AAPL.US"]);
        assert_eq!(guesses("brk.b"), ["BRK.B.US"]);
        assert_eq!(guesses("00700"), ["700.HK"]);
        assert_eq!(guesses("600519"), ["600519.SH", "600519.SZ"]);
        assert_eq!(guesses("700.hk"), ["700.HK"]);
        assert!(guesses("0").is_empty());
        assert!(guesses("茅台").is_empty());
    }

    #[test]
    fn validates_stock_entries() {
        let mut aapl = Instrument {
            provider: ProviderId::Longbridge,
            symbol: "aapl.us".into(),
            base: "AAPL".into(),
            quote: "USD".into(),
            name: Some(" 苹果 ".into()),
            decimals: Some(2),
            pinned: false,
        };
        assert!(Longbridge.validate(&mut aapl).is_ok());
        assert_eq!((aapl.symbol.as_str(), aapl.name.as_deref()), ("AAPL.US", Some("苹果")));
        aapl.base = "MSFT".into();
        assert!(Longbridge.validate(&mut aapl).is_err());
    }
}
