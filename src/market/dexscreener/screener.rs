//! DEX Screener's public API (`api.dexscreener.com`): pool search and pool
//! statistics, 300 requests a minute, no key. Its edge caches a response
//! for 30 seconds (search: 60), so polling faster only re-reads the cache.

use serde::Deserialize;

use super::pool_symbol;
use crate::{
    http,
    market::{Error, Onchain, Stats},
    net,
};

const HOST: &str = "https://api.dexscreener.com";
/// Pools per `pairs` request.
pub const BATCH: usize = 30;

/// A pool as DEX Screener describes it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    pub chain_id: String,
    pub dex_id: String,
    pub pair_address: String,
    pub base_token: Token,
    #[serde(default)]
    price_usd: Option<String>,
    #[serde(default)]
    price_change: Option<Window>,
    #[serde(default)]
    volume: Option<Window>,
    #[serde(default)]
    liquidity: Option<Liquidity>,
    #[serde(default)]
    pub fdv: Option<f64>,
    #[serde(default)]
    pub market_cap: Option<f64>,
    #[serde(default)]
    txns: Option<Txns>,
}

#[derive(Debug, Deserialize)]
pub struct Token {
    pub name: String,
    pub symbol: String,
}

/// A figure over the last 24 hours (the shorter windows aren't used).
#[derive(Debug, Deserialize)]
struct Window {
    #[serde(default)]
    h24: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Liquidity {
    #[serde(default)]
    usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Txns {
    #[serde(default)]
    h24: Option<Count>,
}

#[derive(Debug, Deserialize)]
struct Count {
    buys: u32,
    sells: u32,
}

#[derive(Deserialize)]
struct Pairs {
    /// Null when nothing matched.
    #[serde(default)]
    pairs: Option<Vec<Pair>>,
}

impl Pair {
    /// The instrument symbol of this pool, as `validate` writes it.
    pub fn symbol(&self) -> String {
        pool_symbol(&self.chain_id, &self.pair_address)
    }

    /// The base token's price in USD.
    pub fn price(&self) -> Option<f64> {
        let price: f64 = self.price_usd.as_deref()?.parse().ok()?;
        (price.is_finite() && price > 0.0).then_some(price)
    }

    /// The price 24 hours ago, undone from the change; the price itself for
    /// a pool too new to have one.
    pub fn open(&self) -> Option<f64> {
        let price = self.price()?;
        let pct = self.price_change.as_ref().and_then(|c| c.h24).unwrap_or(0.0);
        let open = price / (1.0 + pct / 100.0);
        Some(if open.is_finite() && open > 0.0 { open } else { price })
    }

    pub fn liquidity(&self) -> Option<f64> {
        self.liquidity.as_ref().and_then(|l| l.usd).filter(|usd| usd.is_finite())
    }

    /// The chart's statistics; the 24-hour range comes from elsewhere.
    pub fn stats(&self, high: Option<f64>, low: Option<f64>) -> Option<Stats> {
        let last = self.price()?;
        let open = self.open()?;
        let change = last - open;
        let txns = self.txns.as_ref().and_then(|t| t.h24.as_ref());
        Some(Stats {
            last,
            high,
            low,
            volume: None,
            turnover: self.volume.as_ref().and_then(|v| v.h24).unwrap_or(0.0),
            change,
            change_pct: if open > 0.0 { change / open * 100.0 } else { 0.0 },
            onchain: Some(Onchain {
                liquidity: self.liquidity(),
                market_cap: self.market_cap.filter(|v| v.is_finite()),
                fdv: self.fdv.filter(|v| v.is_finite()),
                buys: txns.map_or(0, |t| t.buys),
                sells: txns.map_or(0, |t| t.sells),
            }),
        })
    }
}

/// Pools matching a name, symbol or address, as DEX Screener ranks them
/// (not by liquidity: the caller sorts).
pub async fn search(query: &str) -> Result<Vec<Pair>, Error> {
    let found: Pairs =
        http::get_json(&format!("{HOST}/latest/dex/search?q={}", net::percent_encode(query)))
            .await?;
    Ok(found.pairs.unwrap_or_default())
}

/// The pools at `addresses` on `chain`, [`BATCH`] per request; pools the
/// service doesn't know are left out.
pub async fn pairs(chain: &str, addresses: &[String]) -> Result<Vec<Pair>, Error> {
    let mut all = Vec::with_capacity(addresses.len());
    for batch in addresses.chunks(BATCH) {
        let list: Vec<String> = batch.iter().map(|a| net::percent_encode(a)).collect();
        let url =
            format!("{HOST}/latest/dex/pairs/{}/{}", net::percent_encode(chain), list.join(","));
        let found: Pairs = http::get_json(&url).await?;
        all.extend(found.pairs.unwrap_or_default());
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAIR: &str = r#"{"chainId":"solana","dexId":"raydium","pairAddress":"EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx","baseToken":{"address":"EKpQ","name":"dogwifhat","symbol":"$WIF"},"quoteToken":{"address":"So11","name":"Wrapped SOL","symbol":"SOL"},"priceNative":"0.001977","priceUsd":"0.2175","txns":{"h24":{"buys":743,"sells":684}},"volume":{"h24":289043.58,"h6":55192.61},"priceChange":{"h1":-0.12,"h24":2.88},"liquidity":{"usd":6141965.07,"base":14099109,"quote":27955},"fdv":217289257,"marketCap":217289257}"#;

    #[test]
    fn a_pair_gives_price_change_and_statistics() {
        let pair: Pair = serde_json::from_str(PAIR).unwrap();
        assert_eq!(pair.symbol(), "solana/EP2ib6dYdEeqD8MfE2ezHCxX3kP3K2eLKkirfPm5eyMx");
        assert_eq!(pair.price(), Some(0.2175));
        let open = pair.open().unwrap();
        assert!((open - 0.2175 / 1.0288).abs() < 1e-9);
        let stats = pair.stats(Some(0.23), None).unwrap();
        assert!((stats.change_pct - 2.88).abs() < 1e-9);
        assert_eq!(stats.turnover, 289_043.58);
        assert_eq!((stats.high, stats.low, stats.volume), (Some(0.23), None, None));
        let onchain = stats.onchain.unwrap();
        assert_eq!((onchain.buys, onchain.sells), (743, 684));
        assert_eq!(onchain.liquidity, Some(6_141_965.07));
    }

    // A pool minutes old has no 24-hour change yet, and nothing matched is null.
    #[test]
    fn missing_figures_are_tolerated() {
        let json = r#"{"chainId":"base","dexId":"uniswap","pairAddress":"0xAbC","baseToken":{"address":"0x1","name":"New","symbol":"NEW"},"quoteToken":{"address":"0x2","name":"WETH","symbol":"WETH"},"priceUsd":"0.5","priceChange":{"m5":1.0}}"#;
        let pair: Pair = serde_json::from_str(json).unwrap();
        assert_eq!(pair.open(), Some(0.5));
        // Only a full EVM address is folded to lower case.
        assert_eq!(pair.symbol(), "base/0xAbC");
        let stats = pair.stats(None, None).unwrap();
        assert_eq!(stats.change_pct, 0.0);
        assert_eq!(stats.onchain.unwrap().buys, 0);
        let none: Pairs =
            serde_json::from_str(r#"{"schemaVersion":"1.0.0","pairs":null}"#).unwrap();
        assert!(none.pairs.is_none());
    }
}
