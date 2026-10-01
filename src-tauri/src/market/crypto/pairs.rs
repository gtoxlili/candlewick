//! Finding a pair: a ranked match over the exchange's list of pairs, loaded
//! on first use and kept while the settings window is open.

use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use super::Exchange;
use crate::{
    market::{Candidate, Error, ProviderId, Search},
    model::Instrument,
};

/// A spot pair being traded.
pub struct Pair {
    pub symbol: String,
    pub base: String,
    pub quote: String,
    /// Decimals of the tick size.
    pub decimals: u8,
    /// Base and quote run together, as people type them: `BTCUSDT`.
    key: String,
}

impl Pair {
    /// `tick` is the tick size as the exchange writes it, e.g. `"0.01"`.
    pub fn new(symbol: String, base: String, quote: String, tick: &str) -> Self {
        let key = format!("{base}{quote}");
        Self { symbol, base, quote, decimals: tick_decimals(tick), key }
    }

    fn instrument(&self, provider: ProviderId) -> Instrument {
        Instrument {
            provider,
            symbol: self.symbol.clone(),
            base: self.base.clone(),
            quote: self.quote.clone(),
            name: None,
            decimals: Some(self.decimals),
            pinned: false,
        }
    }
}

/// `"0.01000000"` → 2, `"1"` → 0
fn tick_decimals(tick: &str) -> u8 {
    let fraction = tick.split_once('.').map_or("", |(_, fraction)| fraction);
    fraction.trim_end_matches('0').len() as u8
}

/// Listings change rarely; one download serves searches for this long.
const TTL: Duration = Duration::from_secs(30 * 60);

/// One exchange's list of pairs.
struct Cache {
    list: Mutex<Option<(Instant, Arc<Vec<Pair>>)>>,
    /// Moves on when the list is dropped, so a download still running then
    /// doesn't fill it again for nobody.
    generation: AtomicU64,
    /// One download at a time: searches typed meanwhile wait for it.
    download: tokio::sync::Mutex<()>,
}

impl Cache {
    const fn new() -> Self {
        Self {
            list: Mutex::new(None),
            generation: AtomicU64::new(0),
            download: tokio::sync::Mutex::const_new(()),
        }
    }

    fn of(provider: ProviderId) -> &'static Self {
        static CACHES: [Cache; ProviderId::EXCHANGES.len()] =
            [const { Cache::new() }; ProviderId::EXCHANGES.len()];
        let index = ProviderId::EXCHANGES.iter().position(|&p| p == provider);
        &CACHES[index.expect("a crypto exchange")]
    }

    fn cached(&self) -> Option<Arc<Vec<Pair>>> {
        let list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        list.as_ref().filter(|(at, _)| at.elapsed() < TTL).map(|(_, list)| list.clone())
    }

    async fn get<F: Future<Output = Result<Vec<Pair>, Error>>>(
        &self,
        download: impl FnOnce() -> F,
    ) -> Result<Arc<Vec<Pair>>, Error> {
        if let Some(list) = self.cached() {
            return Ok(list);
        }
        let _download = self.download.lock().await;
        if let Some(list) = self.cached() {
            return Ok(list);
        }
        let generation = self.generation.load(Ordering::Acquire);
        let list = Arc::new(download().await?);
        let mut cached = self.list.lock().unwrap_or_else(|e| e.into_inner());
        if self.generation.load(Ordering::Acquire) == generation {
            *cached = Some((Instant::now(), list.clone()));
        }
        Ok(list)
    }

    fn drop_list(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        *self.list.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

pub fn drop_cache(provider: ProviderId) {
    Cache::of(provider).drop_list();
}

const LIMIT: usize = 8;
const QUOTE_RANK: [&str; 6] = ["USDT", "USDC", "FDUSD", "BTC", "ETH", "BNB"];

pub async fn search<E: Exchange>(query: &str) -> Search {
    match Cache::of(E::ID).get(E::pairs).await {
        Ok(list) => Search {
            candidates: rank(&list, &normalize(query))
                .into_iter()
                .map(|pair| Candidate { instrument: pair.instrument(E::ID), manual: false })
                .collect(),
            notes: Vec::new(),
        },
        Err(e) => {
            log::warn!("cannot load {} pairs: {e}", E::ID.key());
            Search {
                candidates: parse_pair::<E>(query)
                    .map(|instrument| Candidate { instrument, manual: true })
                    .into_iter()
                    .collect(),
                notes: vec![format!(
                    "无法获取{}交易对列表，请输入完整交易对，如 SOL/USDT",
                    E::ID.name()
                )],
            }
        }
    }
}

/// Exact base or pair first, then prefixes; within each, the common quotes.
fn rank<'a>(pairs: &'a [Pair], query: &str) -> Vec<&'a Pair> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(usize, &Pair)> = pairs
        .iter()
        .filter_map(|pair| {
            let matched = if pair.base == query || pair.key == query {
                0
            } else if pair.base.starts_with(query) {
                1
            } else if pair.key.starts_with(query) {
                2
            } else {
                return None;
            };
            let quote =
                QUOTE_RANK.iter().position(|q| *q == pair.quote).unwrap_or(QUOTE_RANK.len());
            Some((matched * 100 + quote, pair))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.key.cmp(&b.1.key)));
    ranked.into_iter().take(LIMIT).map(|(_, pair)| pair).collect()
}

const MANUAL_QUOTES: [&str; 10] =
    ["FDUSD", "USDT", "USDC", "USD1", "TUSD", "BTC", "ETH", "BNB", "EUR", "TRY"];

/// Without the pair list: `SOL/USDT` (or `-`, `_`), or `SOLUSDT` split at a
/// known quote asset. Decimals stay unknown, so the app picks them by
/// magnitude.
fn parse_pair<E: Exchange>(input: &str) -> Option<Instrument> {
    let raw = input.trim().to_uppercase();
    let (base, quote) = match raw.split_once(['/', '-', '_']) {
        Some((base, quote)) => (normalize(base), normalize(quote)),
        None => {
            let symbol = normalize(&raw);
            let quote =
                MANUAL_QUOTES.iter().find(|q| symbol.ends_with(*q) && symbol.len() > q.len())?;
            (symbol[..symbol.len() - quote.len()].to_owned(), (*quote).to_owned())
        }
    };
    (super::valid_asset(&base) && super::valid_asset(&quote)).then(|| Instrument {
        provider: E::ID,
        symbol: E::symbol(&base, &quote),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{binance::Binance, okx::Okx};

    fn pair(base: &str, quote: &str) -> Pair {
        Pair::new(format!("{base}-{quote}"), base.into(), quote.into(), "0.01")
    }

    #[test]
    fn ranks_exact_then_prefix_then_quote() {
        let pairs =
            [pair("SOL", "BTC"), pair("SOLV", "USDT"), pair("SOL", "USDT"), pair("SOL", "USDC")];
        let ranked = |query: &str| -> Vec<String> {
            rank(&pairs, &normalize(query)).iter().map(|p| p.symbol.clone()).collect()
        };
        assert_eq!(ranked("sol"), ["SOL-USDT", "SOL-USDC", "SOL-BTC", "SOLV-USDT"]);
        // Typed as one word or with a separator, the pair itself comes first.
        assert_eq!(ranked("solbtc")[0], "SOL-BTC");
        assert_eq!(ranked("SOL/BTC")[0], "SOL-BTC");
    }

    #[test]
    fn manual_pairs_take_the_exchanges_spelling() {
        let sol = parse_pair::<Binance>(" solusdt").unwrap();
        assert_eq!(
            (sol.base.as_str(), sol.quote.as_str(), sol.symbol.as_str()),
            ("SOL", "USDT", "SOLUSDT")
        );
        let aeur = parse_pair::<Okx>("sol/aeur").unwrap();
        assert_eq!((aeur.base.as_str(), aeur.quote.as_str()), ("SOL", "AEUR"));
        assert_eq!(aeur.symbol, "SOL-AEUR");
        assert_eq!(parse_pair::<Okx>("sol-usdt").unwrap().symbol, "SOL-USDT");
        assert!(parse_pair::<Binance>("USDT").is_none());
    }

    #[test]
    fn tick_sizes_give_decimals() {
        assert_eq!(tick_decimals("0.01000000"), 2);
        assert_eq!(tick_decimals("1.00000000"), 0);
        assert_eq!(tick_decimals("0.0000000000001"), 13);
        assert_eq!(tick_decimals("1"), 0);
    }
}
