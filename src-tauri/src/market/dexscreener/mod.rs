//! Pools on decentralized exchanges, on any chain DEX Screener lists: the
//! tokens that trade on-chain only, beside the exchanges' pairs. Prices
//! are in USD, as DEX Screener shows them. Two public services, neither
//! needing a key: DEX Screener for search, the menu bar quotes and the
//! pool's statistics; GeckoTerminal for candles and trades, which DEX
//! Screener doesn't publish (`gecko.rs` on its rate limit).
//!
//! A pool's symbol is `<chain>/<pool address>` as in DEX Screener's URLs:
//! `solana/EP2ib6…`. The instrument's `base` is the token's symbol, its
//! `quote` always `USD`, its `name` the token's name, and its `venue` the
//! chain and DEX by name.

mod chains;
mod gecko;
mod quotes;
mod screener;
mod stream;

use tauri::{AppHandle, ipc::Channel};
use tokio::sync::{oneshot, watch};

use super::{
    BoxFuture, Candidate, Candle, ChartMode, ChartSpec, Error, IntervalSpec, LiveEvent, Provider,
    ProviderId, Search, StatsSpan, Trade,
};
use crate::model::{FeedControl, Instrument, Venue};

pub struct DexScreener;

const ID: ProviderId = ProviderId::DexScreener;
/// Prices are in US dollars whatever the pool's quote token.
const QUOTE: &str = "USD";
/// Pools with less than this in them are noise: a search for a symbol
/// finds dozens of copycats with a few dollars each.
const MIN_LIQUIDITY: f64 = 5_000.0;
const SEARCH_LIMIT: usize = 6;
const MAX_TOKEN_SYMBOL: usize = 20;
const MAX_NAME: usize = 40;

/// A candle interval as GeckoTerminal offers it.
struct IntervalDef {
    secs: u32,
    timeframe: &'static str,
    aggregate: u32,
}

const INTERVALS: [IntervalDef; 6] = [
    IntervalDef { secs: 60, timeframe: "minute", aggregate: 1 },
    IntervalDef { secs: 300, timeframe: "minute", aggregate: 5 },
    IntervalDef { secs: 900, timeframe: "minute", aggregate: 15 },
    IntervalDef { secs: 3600, timeframe: "hour", aggregate: 1 },
    IntervalDef { secs: 14_400, timeframe: "hour", aggregate: 4 },
    IntervalDef { secs: 86_400, timeframe: "day", aggregate: 1 },
];

impl Provider for DexScreener {
    fn id(&self) -> ProviderId {
        ID
    }

    fn validate(&self, instrument: &mut Instrument) -> Result<(), String> {
        let invalid = || t!("error.invalidPool", symbol = instrument.symbol);
        let Some((chain, address)) = parse(&instrument.symbol) else {
            return Err(invalid());
        };
        let base: String = instrument.base.trim().chars().take(MAX_TOKEN_SYMBOL).collect();
        if base.is_empty() {
            return Err(invalid());
        }
        instrument.symbol = pool_symbol(&chain, &address);
        instrument.base = base;
        instrument.quote = QUOTE.to_owned();
        instrument.name = instrument.name.take().and_then(|name| trimmed(&name));
        if let Some(venue) = &mut instrument.venue {
            venue.chain = trimmed(&venue.chain).unwrap_or_else(|| chains::name(&chain));
            venue.dex = venue.dex.take().and_then(|dex| trimmed(&dex));
        }
        Ok(())
    }

    fn bare(&self, symbol: &str) -> Option<Instrument> {
        let (chain, address) = parse(symbol)?;
        let mut instrument = Instrument::bare(ID, &pool_symbol(&chain, &address));
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
            intervals: INTERVALS
                .iter()
                .map(|i| IntervalSpec {
                    secs: i.secs,
                    mode: ChartMode::Candle,
                    aligned: true,
                    regular_only: false,
                })
                .collect(),
            stats_span: StatsSpan::Rolling24h,
            // Trades are sized in the token; the day's volume comes in dollars only.
            volume_unit: Some(instrument.base.clone()).filter(|base| !base.is_empty()),
            turnover_unit: QUOTE.to_owned(),
            day_offset: 0,
            book_steps: Vec::new(),
            book: false,
            onchain: true,
            link: parse(&instrument.symbol)
                .map(|(chain, address)| format!("https://dexscreener.com/{chain}/{address}")),
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
            let def = INTERVALS.iter().find(|i| i.secs == interval).ok_or_else(|| {
                Error::Message(t!("error.unsupportedInterval", seconds = interval))
            })?;
            let (chain, address) = pool(instrument)?;
            gecko::candles(
                chains::gecko_network(&chain),
                &address,
                def.timeframe,
                def.aggregate,
                end,
                limit,
            )
            .await
        })
    }

    fn recent_trades<'a>(
        &'a self,
        instrument: &'a Instrument,
        limit: usize,
    ) -> BoxFuture<'a, Result<Vec<Trade>, Error>> {
        Box::pin(async move {
            let (chain, address) = pool(instrument)?;
            let mut trades = gecko::trades(chains::gecko_network(&chain), &address).await?;
            let keep = limit.min(trades.len());
            trades.drain(..trades.len() - keep);
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

/// `(chain, address)` of a pool symbol, normalized: the chain in lower
/// case, an EVM address in lower case too (DEX Screener returns them
/// checksummed, in mixed case), other addresses as they are (Solana's are
/// case-sensitive). `None` for anything else.
pub(super) fn parse(symbol: &str) -> Option<(String, String)> {
    let (chain, address) = symbol.trim().split_once('/')?;
    let chain = chain.trim().to_ascii_lowercase();
    let address = address.trim();
    let chain_ok = (1..=32).contains(&chain.len())
        && chain.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let address_ok = (1..=160).contains(&address.len())
        && address.bytes().all(|b| b.is_ascii_graphic() && b != b'/');
    if !chain_ok || !address_ok {
        return None;
    }
    let evm = address.len() == 42
        && address.starts_with("0x")
        && address[2..].bytes().all(|b| b.is_ascii_hexdigit());
    let address = if evm { address.to_ascii_lowercase() } else { address.to_owned() };
    Some((chain, address))
}

/// `solana/EP2ib6…`
pub(super) fn pool_symbol(chain: &str, address: &str) -> String {
    match parse(&format!("{chain}/{address}")) {
        Some((chain, address)) => format!("{chain}/{address}"),
        None => format!("{}/{address}", chain.to_ascii_lowercase()),
    }
}

fn pool(instrument: &Instrument) -> Result<(String, String), Error> {
    parse(&instrument.symbol)
        .ok_or_else(|| Error::Message(t!("error.invalidPool", symbol = instrument.symbol)))
}

fn trimmed(text: &str) -> Option<String> {
    let text: String = text.trim().chars().take(MAX_NAME).collect();
    (!text.is_empty()).then_some(text)
}

/// Pools matching the query, the one whose token is named exactly first,
/// then by liquidity; a pasted token or pool address finds its pools.
async fn search(query: &str) -> Search {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Search::default();
    }
    match screener::search(query).await {
        Ok(pairs) => Search { candidates: rank(pairs, query), notes: Vec::new() },
        Err(e) => Search { candidates: Vec::new(), notes: vec![e.to_string()] },
    }
}

fn rank(pairs: Vec<screener::Pair>, query: &str) -> Vec<Candidate> {
    let wanted = token_key(query);
    let mut ranked: Vec<(u8, f64, screener::Pair)> = pairs
        .into_iter()
        .filter(|pair| pair.price().is_some())
        .filter_map(|pair| {
            let liquidity = pair.liquidity().filter(|usd| *usd >= MIN_LIQUIDITY)?;
            let key = token_key(&pair.base_token.symbol);
            let matched = if key == wanted {
                0
            } else if key.starts_with(&wanted) {
                1
            } else {
                2
            };
            Some((matched, liquidity, pair))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.total_cmp(&a.1)));
    ranked.dedup_by(|a, b| a.2.symbol() == b.2.symbol());
    ranked
        .into_iter()
        .take(SEARCH_LIMIT)
        .map(|(_, liquidity, pair)| Candidate {
            instrument: Instrument {
                provider: ID,
                symbol: pair.symbol(),
                base: pair.base_token.symbol.trim().chars().take(MAX_TOKEN_SYMBOL).collect(),
                quote: QUOTE.to_owned(),
                name: trimmed(&pair.base_token.name),
                decimals: None,
                pinned: false,
                venue: Some(Venue {
                    chain: chains::name(&pair.chain_id),
                    dex: Some(chains::dex_name(&pair.dex_id)),
                }),
            },
            manual: false,
            liquidity: Some(liquidity),
        })
        .collect()
}

/// A token symbol as people type it: upper case, without the `$` some
/// tokens wear.
fn token_key(symbol: &str) -> String {
    symbol.trim().trim_start_matches('$').to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_symbols_normalize_the_chain_and_evm_addresses_only() {
        let (chain, address) =
            parse(" Solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx ").unwrap();
        assert_eq!(
            (chain.as_str(), address.as_str()),
            ("solana", "EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx")
        );
        let (_, evm) = parse("base/0xC1d6aaf3b9e8AbA2a3293eBbB090A6d6029c066C").unwrap();
        assert_eq!(evm, "0xc1d6aaf3b9e8aba2a3293ebbb090a6d6029c066c");
        assert!(parse("BTCUSDT").is_none());
        assert!(parse("solana/").is_none());
        assert!(parse("sol ana/abc").is_none());
        assert_eq!(pool_symbol("Base", "0xABC"), "base/0xABC");
    }

    #[test]
    fn validates_and_normalizes_pools() {
        let mut wif = Instrument {
            provider: ID,
            symbol: "Solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx".into(),
            base: " $WIF ".into(),
            quote: "SOL".into(),
            name: Some("  dogwifhat ".into()),
            decimals: None,
            pinned: false,
            venue: Some(Venue { chain: " ".into(), dex: Some("Raydium".into()) }),
        };
        assert!(DexScreener.validate(&mut wif).is_ok());
        assert_eq!(wif.symbol, "solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx");
        assert_eq!(
            (wif.base.as_str(), wif.quote.as_str(), wif.name.as_deref()),
            ("$WIF", "USD", Some("dogwifhat"))
        );
        assert_eq!(wif.venue.as_ref().map(|v| v.chain.as_str()), Some("Solana"));
        wif.base = String::new();
        assert!(DexScreener.validate(&mut wif).is_err());
        wif.symbol = "nonsense".into();
        assert!(DexScreener.validate(&mut wif).is_err());

        let bare = DexScreener.bare("BASE/0xC1d6aaf3b9e8AbA2a3293eBbB090A6d6029c066C").unwrap();
        assert_eq!(bare.symbol, "base/0xc1d6aaf3b9e8aba2a3293ebbb090a6d6029c066c");
        assert_eq!(bare.quote, "USD");
        assert!(DexScreener.bare("").is_none());
    }

    fn pair(chain: &str, address: &str, symbol: &str, liquidity: f64) -> screener::Pair {
        serde_json::from_str(&format!(
            r#"{{"chainId":"{chain}","dexId":"raydium","pairAddress":"{address}","baseToken":{{"address":"a","name":"Token {symbol}","symbol":"{symbol}"}},"quoteToken":{{"address":"b","name":"SOL","symbol":"SOL"}},"priceUsd":"1.5","liquidity":{{"usd":{liquidity}}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn search_ranks_the_named_token_first_then_liquidity_and_drops_dust() {
        let found = rank(
            vec![
                pair("solana", "A", "WIFE", 90_000.0),
                pair("solana", "B", "$WIF", 6_000_000.0),
                pair("base", "C", "WIF", 40_000.0),
                pair("solana", "D", "WIF", 100.0),
                pair("bsc", "E", "DOGWIF", 500_000.0),
            ],
            "wif",
        );
        let symbols: Vec<&str> = found.iter().map(|c| c.instrument.symbol.as_str()).collect();
        assert_eq!(symbols, ["solana/B", "base/C", "solana/A", "bsc/E"]);
        let first = &found[0].instrument;
        assert_eq!((first.base.as_str(), first.quote.as_str()), ("$WIF", "USD"));
        assert_eq!(first.venue.as_ref().unwrap().chain, "Solana");
        assert_eq!(found[0].liquidity, Some(6_000_000.0));
    }

    #[test]
    fn chart_spec_links_to_the_pool() {
        let wif = DexScreener.bare("solana/EP2i").unwrap();
        let spec = DexScreener.chart_spec(&wif);
        assert_eq!(spec.link.as_deref(), Some("https://dexscreener.com/solana/EP2i"));
        assert!(!spec.book && spec.onchain && spec.volume_unit.is_none());
        let mut named = wif.clone();
        named.base = "$WIF".into();
        assert_eq!(DexScreener.chart_spec(&named).volume_unit.as_deref(), Some("$WIF"));
        assert_eq!(spec.intervals.len(), INTERVALS.len());
        assert!(DexScreener.chart_spec(&Instrument::bare(ID, "")).link.is_none());
    }
}
