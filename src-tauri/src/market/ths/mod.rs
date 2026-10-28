//! Tonghuashun (同花顺): China A-shares, indexes (the exchanges' and
//! Tonghuashun's own sector boards) and exchange-traded funds, through its
//! financial data API with the user's own key (`credentials.rs`). No
//! brokerage account is needed, only a Tonghuashun login to issue the key.
//!
//! It is REST only: quotes are snapshots polled while Shanghai trades
//! (`session.rs`), candles are daily (weekly ones are summed from them), and
//! there is no order book or trade list; the chart's trades are what the
//! day's volume grew by between snapshots (`stream.rs`).
//!
//! A symbol is Tonghuashun's code: `600519.SH` (a share), `000001.SH` (an
//! index), `885525.TI` (a board), `510300.SH` (an ETF). Each kind has its own
//! endpoints, and which kind a code is comes from Tonghuashun's search
//! (`api::kind`); the instrument's `base` is the six digits, its `quote`
//! always `CNY`.

mod api;
mod quotes;
mod session;
mod stream;

use std::time::Duration;

use tauri::{AppHandle, ipc::Channel};
use tokio::sync::{oneshot, watch};

use super::{
    BoxFuture, Candidate, Candle, ChartMode, ChartSpec, Error, IntervalSpec, LiveEvent, Provider,
    ProviderId, Search, StatsSpan, Trade,
};
use crate::model::{FeedControl, Instrument};

pub struct Ths;

/// Checks with Tonghuashun that `key` works.
pub async fn check(key: &str) -> Result<(), Error> {
    api::check(key).await
}

/// The key to use from now on, as the settings keep it.
pub fn configure(key: Option<String>) {
    api::configure(key);
}

const ID: ProviderId = ProviderId::Ths;
const QUOTE: &str = "CNY";
/// The markets a code can carry: the Shanghai, Shenzhen and Beijing
/// exchanges, and Tonghuashun's own indexes.
const MARKETS: [&str; 4] = ["SH", "SZ", "BJ", "TI"];
const MAX_NAME: usize = 40;
const SEARCH_LIMIT: usize = 8;
/// Shanghai and Shenzhen publish a level-1 snapshot every 3 seconds; reading
/// faster finds the same one.
const POLL: Duration = Duration::from_secs(3);
const DAY: u32 = 86_400;
const WEEK: u32 = 604_800;
const DAY_MS: i64 = 86_400_000;
/// How far back indexes and funds go, and the longest range a request for
/// them may span; one day short of five years, to stay clear of the edge.
const FIVE_YEARS: i64 = 5 * 365;
/// The longest range a request for a share's candles may span.
const TEN_YEARS: i64 = 10 * 365;

/// What a code is, which decides where its quotes and candles come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Stock,
    /// An exchange's index or one of Tonghuashun's boards.
    Index,
    Etf,
    /// Listed open-ended funds: quotes, but no candles.
    Lof,
}

impl Kind {
    /// From the `asset_type` Tonghuashun gives.
    fn of(asset_type: &str) -> Option<Self> {
        match asset_type {
            "a-share" => Some(Self::Stock),
            "a-share-index" => Some(Self::Index),
            "fund-etf" => Some(Self::Etf),
            "fund-lof" => Some(Self::Lof),
            _ => None,
        }
    }

    /// Cents for shares and index points, a tenth of that for fund units.
    fn decimals(self) -> u8 {
        match self {
            Self::Stock | Self::Index => 2,
            Self::Etf | Self::Lof => 3,
        }
    }

    /// How far its daily candles reach, if it has any.
    fn reach(self) -> Option<Reach> {
        match self {
            Self::Stock => Some(Reach { span: TEN_YEARS, back: None }),
            Self::Index | Self::Etf => Some(Reach { span: FIVE_YEARS, back: Some(FIVE_YEARS) }),
            Self::Lof => None,
        }
    }
}

/// The days of candles a request may cover.
#[derive(Debug, Clone, Copy)]
struct Reach {
    /// From its first day to its last.
    span: i64,
    /// Back from today; a request starting earlier finds nothing at all.
    back: Option<i64>,
}

impl Provider for Ths {
    fn id(&self) -> ProviderId {
        ID
    }

    fn validate(&self, instrument: &mut Instrument) -> Result<(), String> {
        let symbol = parse(&instrument.symbol)
            .filter(|symbol| instrument.base == code(symbol) && instrument.quote == QUOTE)
            .ok_or_else(|| t!("error.invalidStock", symbol = instrument.symbol))?;
        instrument.symbol = symbol;
        instrument.name = instrument.name.take().and_then(|name| trimmed(&name));
        Ok(())
    }

    fn bare(&self, symbol: &str) -> Option<Instrument> {
        let symbol = parse(symbol)?;
        let mut instrument = Instrument::bare(ID, &symbol);
        instrument.base = code(&symbol).to_owned();
        instrument.quote = QUOTE.to_owned();
        Some(instrument)
    }

    fn search<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Search> {
        Box::pin(search(query))
    }

    fn watch(
        &self,
        app: AppHandle,
        control: watch::Receiver<FeedControl>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(quotes::run(app, control))
    }

    fn chart_spec(&self, instrument: &Instrument) -> ChartSpec {
        ChartSpec {
            source: ID,
            intervals: [DAY, WEEK]
                .map(|secs| IntervalSpec {
                    secs,
                    mode: ChartMode::Candle,
                    // Days open at midnight in Shanghai, not UTC.
                    aligned: false,
                    regular_only: true,
                })
                .into(),
            stats_span: StatsSpan::Today,
            volume_unit: None,
            turnover_unit: QUOTE.to_owned(),
            day_offset: session::OFFSET,
            book_steps: Vec::new(),
            book: false,
            onchain: false,
            link: link(&instrument.symbol),
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

    /// Tonghuashun keeps no trades; the stream makes its own.
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
        stop: oneshot::Receiver<()>,
        events: Channel<LiveEvent>,
    ) -> BoxFuture<'static, ()> {
        Box::pin(stream::run(instrument, stop, events))
    }
}

/// A code as Tonghuashun writes it, normalized: six digits, a dot and the
/// market, `600519.SH`. `None` for anything else.
fn parse(symbol: &str) -> Option<String> {
    let symbol = symbol.trim().to_ascii_uppercase();
    let (code, market) = symbol.split_once('.')?;
    let valid = code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit());
    (valid && MARKETS.contains(&market)).then_some(symbol)
}

/// `600519` of `600519.SH`.
fn code(symbol: &str) -> &str {
    symbol.split_once('.').map_or(symbol, |(code, _)| code)
}

fn trimmed(text: &str) -> Option<String> {
    let text: String = text.trim().chars().take(MAX_NAME).collect();
    (!text.is_empty()).then_some(text)
}

fn unknown(symbol: &str) -> Error {
    Error::Message(t!("error.unknownSymbol", source = ID.name(), symbol = symbol))
}

/// The code's page on Tonghuashun's site: shares and funds have one by
/// their six digits; indexes go by codes of Tonghuashun's own.
fn link(symbol: &str) -> Option<String> {
    let symbol = parse(symbol)?;
    match api::known(&symbol)? {
        Kind::Stock | Kind::Etf | Kind::Lof => {
            Some(format!("https://stockpage.10jqka.com.cn/{}/", code(&symbol)))
        }
        Kind::Index => None,
    }
}

/// What someone typed, as Tonghuashun's search reads it: `sh600519` as
/// `600519.SH`, everything else as it is.
fn query_text(query: &str) -> Option<String> {
    let query: String = query.split_whitespace().collect();
    if query.is_empty() {
        return None;
    }
    let upper = query.to_ascii_uppercase();
    let prefixed = ["SH", "SZ", "BJ"]
        .into_iter()
        .find_map(|market| Some((market, upper.strip_prefix(market)?)))
        .and_then(|(market, code)| parse(&format!("{code}.{market}")));
    Some(prefixed.unwrap_or(query))
}

async fn search(query: &str) -> Search {
    let Some(query) = query_text(query) else {
        return Search::default();
    };
    match api::search(&query, SEARCH_LIMIT).await {
        Ok(found) => Search {
            candidates: found
                .into_iter()
                .map(|found| Candidate {
                    instrument: Instrument {
                        provider: ID,
                        base: code(&found.symbol).to_owned(),
                        quote: QUOTE.to_owned(),
                        name: found.name.as_deref().and_then(trimmed),
                        decimals: Some(found.kind.decimals()),
                        pinned: false,
                        venue: None,
                        symbol: found.symbol,
                    },
                    manual: false,
                    liquidity: None,
                })
                .collect(),
            notes: Vec::new(),
        },
        Err(e) => Search { candidates: Vec::new(), notes: vec![e.to_string()] },
    }
}

async fn history(
    instrument: &Instrument,
    interval: u32,
    end: Option<f64>,
    limit: usize,
) -> Result<Vec<Candle>, Error> {
    let weekly = match interval {
        DAY => false,
        WEEK => true,
        _ => return Err(Error::Message(t!("error.unsupportedInterval", seconds = interval))),
    };
    let symbol = &instrument.symbol;
    let kind = api::kind(symbol).await?.ok_or_else(|| unknown(symbol))?;
    let Some(reach) = kind.reach() else {
        return Ok(Vec::new());
    };
    let Some((start, last)) = range(reach, weekly, end, limit, session::now() * 1000) else {
        return Ok(Vec::new());
    };
    let days = api::daily(kind, symbol, start, last).await?;
    let mut candles = if weekly { weeks(&days) } else { days };
    if let Some(end) = end {
        candles.retain(|candle| candle.time < end);
    }
    candles.drain(..candles.len().saturating_sub(limit));
    Ok(candles)
}

/// The days (epoch milliseconds, both included) to ask for `limit` candles
/// that opened before `end` (epoch seconds; none: up to `now`), within what
/// the kind reaches. `None` when that is nothing.
fn range(
    reach: Reach,
    weekly: bool,
    end: Option<f64>,
    limit: usize,
    now: i64,
) -> Option<(i64, i64)> {
    let last = end.map_or(now, |end| (end * 1000.0) as i64 - 1);
    let limit = limit as i64;
    // About 245 trading days a year: three calendar days for two candles,
    // and a fortnight more for the holidays.
    let days = if weekly { limit * 7 } else { limit * 3 / 2 + 14 };
    let mut start = last - days.min(reach.span) * DAY_MS;
    if let Some(back) = reach.back {
        start = start.max(now - back * DAY_MS);
    }
    if weekly {
        // Whole weeks only: a week cut short at its start would open late.
        let monday = session::week_start(start.div_euclid(1000)) * 1000;
        start = if monday == start { monday } else { monday + 7 * DAY_MS };
    }
    (start <= last).then_some((start, last))
}

/// Daily candles, oldest first, summed into weeks that open on Monday in
/// Shanghai.
fn weeks(days: &[Candle]) -> Vec<Candle> {
    let mut weeks: Vec<Candle> = Vec::new();
    for day in days {
        let start = session::week_start(day.time as i64) as f64;
        match weeks.last_mut() {
            Some(week) if week.time == start => {
                week.high = week.high.max(day.high);
                week.low = week.low.min(day.low);
                week.close = day.close;
            }
            _ => weeks.push(Candle { time: start, ..*day }),
        }
    }
    weeks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instrument(symbol: &str, base: &str, quote: &str) -> Instrument {
        Instrument {
            provider: ID,
            symbol: symbol.into(),
            base: base.into(),
            quote: quote.into(),
            name: Some(" 贵州茅台 ".into()),
            decimals: Some(2),
            pinned: false,
            venue: None,
        }
    }

    // contract: settings.json keeps the symbol as Tonghuashun's code, which
    // its endpoints take as `thscode`; `.CSI` indexes have no quotes there.
    #[test]
    fn symbols_are_tonghuashun_codes_the_app_can_quote() {
        let mut moutai = instrument(" 600519.sh ", "600519", "CNY");
        assert!(Ths.validate(&mut moutai).is_ok());
        assert_eq!(
            (moutai.symbol.as_str(), moutai.name.as_deref()),
            ("600519.SH", Some("贵州茅台"))
        );
        for (symbol, base, quote) in [
            ("600519.SH", "000001", "CNY"),
            ("600519.SH", "600519", "USD"),
            ("000510.CSI", "000510", "CNY"),
            ("60051.SH", "60051", "CNY"),
            ("600519", "600519", "CNY"),
        ] {
            assert!(Ths.validate(&mut instrument(symbol, base, quote)).is_err(), "{symbol}");
        }
        let board = Ths.bare("885525.ti").unwrap();
        assert_eq!(
            (board.symbol.as_str(), board.base.as_str(), board.quote.as_str()),
            ("885525.TI", "885525", "CNY")
        );
        assert!(Ths.bare("AAPL.US").is_none());
    }

    #[test]
    fn exchange_prefixed_codes_read_as_tonghuashun_writes_them() {
        assert_eq!(query_text(" sh600519 ").as_deref(), Some("600519.SH"));
        assert_eq!(query_text("BJ920799").as_deref(), Some("920799.BJ"));
        assert_eq!(query_text("贵州 茅台").as_deref(), Some("贵州茅台"));
        assert_eq!(query_text("shanghai").as_deref(), Some("shanghai"));
        assert!(query_text("  ").is_none());
    }

    const NOW: i64 = 1_793_160_000_000; // 2026-10-28 12:00 in Shanghai

    // upstream: index and fund history answers nothing at all for a range
    // starting more than five years back (tried 2026-10-28), and refuses a
    // range over five years (shares: ten).
    #[test]
    fn ranges_stay_within_what_the_kind_reaches() {
        let index = Kind::Index.reach().unwrap();
        let (start, last) = range(index, false, None, 1000, NOW).unwrap();
        assert_eq!(last, NOW);
        assert!(NOW - start <= 5 * 365 * DAY_MS);
        // Paging back past five years finds nothing, without asking.
        let five_years_ago = (NOW - 5 * 366 * DAY_MS) as f64 / 1000.0;
        assert!(range(index, false, Some(five_years_ago), 1000, NOW).is_none());
        let stock = Kind::Stock.reach().unwrap();
        let (start, last) = range(stock, true, Some(1_400_000_000.0), 1000, NOW).unwrap();
        assert!(last - start <= 10 * 365 * DAY_MS);
        assert_eq!(session::week_start(start / 1000) * 1000, start);
        assert!(Kind::Lof.reach().is_none());
    }

    #[test]
    fn days_sum_into_weeks_from_monday() {
        let day = |date: i64, open: f64, high: f64, low: f64, close: f64| Candle {
            time: (date * 86_400 - session::OFFSET) as f64,
            open,
            high,
            low,
            close,
        };
        // 2026-10-23 (Friday), then Monday 26th to Wednesday 28th.
        let friday = crate::calendar::days_from_civil(2026, 10, 23);
        let monday = friday + 3;
        let weeks = weeks(&[
            day(friday, 10.0, 11.0, 9.0, 10.5),
            day(monday, 10.6, 12.0, 10.1, 11.8),
            day(monday + 1, 11.8, 11.9, 9.5, 9.9),
            day(monday + 2, 9.9, 10.4, 9.7, 10.2),
        ]);
        assert_eq!(weeks.len(), 2);
        let week = weeks[1];
        assert_eq!(week.time, (monday * 86_400 - session::OFFSET) as f64);
        assert_eq!((week.open, week.high, week.low, week.close), (10.6, 12.0, 9.5, 10.2));
    }
}
