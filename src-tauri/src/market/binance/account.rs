//! A Binance account through a read-only API key: the spot, funding and
//! Simple Earn wallets, and the USDⓈ-M and COIN-M futures accounts.

use futures_util::future::join_all;
use reqwest::Method;
use serde::{Deserialize, de::DeserializeOwned};

use super::Binance;
use crate::{
    credentials::ApiKey,
    http,
    market::{
        Error,
        crypto::{
            self,
            account::{self, Account, Clock},
        },
    },
    net,
    portfolio::{Balance, Position, Price, Wallet},
};

/// Spot and wallet endpoints; `data-api.binance.vision` serves no private ones.
const SPOT: &[&str] = &["api.binance.com", "api-gcp.binance.com"];
const USD_FUTURES: &[&str] = &["fapi.binance.com"];
const COIN_FUTURES: &[&str] = &["dapi.binance.com"];
/// How far a request's time may trail Binance's clock, in milliseconds.
const RECV_WINDOW: u32 = 10_000;

static CLOCK: Clock = Clock::new();

impl Account for Binance {
    async fn check(key: &ApiKey) -> Result<(), Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Restrictions {
            enable_reading: bool,
            #[serde(flatten)]
            others: std::collections::HashMap<String, serde_json::Value>,
        }
        let restrictions: Restrictions =
            get(key, Method::GET, SPOT, "/sapi/v1/account/apiRestrictions", "").await?;
        // Every other `enable…` or `permits…` flag lets the key trade or
        // move funds; `enableFixReadOnly` only reads.
        let allows_more = restrictions.others.iter().any(|(name, value)| {
            (name.starts_with("enable") || name.starts_with("permits"))
                && name != "enableFixReadOnly"
                && value.as_bool() == Some(true)
        });
        if !restrictions.enable_reading {
            return Err(Error::Message("这个 API Key 没有读取权限".to_owned()));
        }
        if allows_more {
            return Err(Error::Message("这个 API Key 可以交易或提币，请换成只读 Key".to_owned()));
        }
        Ok(())
    }

    async fn trading(key: &ApiKey) -> Result<(Vec<Balance>, Vec<Position>), Error> {
        let (spot, usd, coin) = tokio::join!(spot(key), usd_futures(key), coin_futures(key));
        let mut balances = spot?;
        let mut positions = Vec::new();
        // The futures accounts may be missing or out of reach (a key older than
        // the futures account, Portfolio Margin); the rest still shows.
        for futures in [usd, coin] {
            match futures {
                Ok((more, open)) => {
                    balances.extend(more);
                    positions.extend(open);
                }
                Err(e) => log::info!("binance futures: {e}"),
            }
        }
        Ok((balances, positions))
    }

    async fn savings(key: &ApiKey) -> Result<Vec<Balance>, Error> {
        #[derive(Deserialize)]
        struct Funding {
            asset: String,
            free: String,
            locked: String,
            freeze: String,
            withdrawing: String,
        }
        let funding =
            get::<Vec<Funding>>(key, Method::POST, SPOT, "/sapi/v1/asset/get-funding-asset", "");
        let (funding, flexible, locked) = tokio::join!(
            funding,
            earn(key, "/sapi/v1/simple-earn/flexible/position"),
            earn(key, "/sapi/v1/simple-earn/locked/position"),
        );
        let mut balances: Vec<Balance> = funding?
            .into_iter()
            .map(|f| {
                let amount = [&f.free, &f.locked, &f.freeze, &f.withdrawing]
                    .iter()
                    .map(|n| crypto::num(n))
                    .sum();
                Balance::new(Wallet::Funding, f.asset, amount)
            })
            .collect();
        // Earn reads may be refused on their own; the rest still shows.
        for earned in [flexible, locked] {
            match earned {
                Ok(earned) => balances.extend(earned),
                Err(e) => log::info!("binance earn: {e}"),
            }
        }
        Ok(balances)
    }
}

async fn spot(key: &ApiKey) -> Result<Vec<Balance>, Error> {
    #[derive(Deserialize)]
    struct Spot {
        balances: Vec<SpotBalance>,
    }
    #[derive(Deserialize)]
    struct SpotBalance {
        asset: String,
        free: String,
        locked: String,
    }
    let spot: Spot =
        get(key, Method::GET, SPOT, "/api/v3/account", "omitZeroBalances=true").await?;
    Ok(spot
        .balances
        .into_iter()
        // `LDBTC` and the like are receipts for Simple Earn's flexible
        // products, which the earn positions count; LDO is a coin.
        .filter(|b| !b.asset.starts_with("LD") || b.asset == "LDO")
        .map(|b| Balance::new(Wallet::Spot, b.asset, crypto::num(&b.free) + crypto::num(&b.locked)))
        .collect())
}

/// Simple Earn positions, every page of them.
async fn earn(key: &ApiKey, path: &str) -> Result<Vec<Balance>, Error> {
    #[derive(Deserialize)]
    struct Page {
        rows: Vec<Row>,
        total: usize,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Row {
        asset: String,
        /// Flexible products.
        total_amount: Option<String>,
        /// Locked products.
        amount: Option<String>,
    }
    let mut balances = Vec::new();
    for current in 1.. {
        let page: Page =
            get(key, Method::GET, SPOT, path, &format!("current={current}&size=100")).await?;
        let done = page.rows.is_empty() || current * 100 >= page.total;
        balances.extend(page.rows.into_iter().map(|row| {
            let amount = row.total_amount.or(row.amount).map_or(0.0, |a| crypto::num(&a));
            Balance::new(Wallet::Earn, row.asset, amount)
        }));
        if done {
            break;
        }
    }
    Ok(balances)
}

/// The USDⓈ-M account at margin balance, and its positions.
async fn usd_futures(key: &ApiKey) -> Result<(Vec<Balance>, Vec<Position>), Error> {
    #[derive(Deserialize)]
    struct Futures {
        assets: Vec<FuturesAsset>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Risk {
        symbol: String,
        position_amt: String,
        entry_price: String,
        mark_price: String,
        un_realized_profit: String,
        liquidation_price: String,
        /// Signed like the position.
        notional: String,
        margin_asset: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Config {
        leverage: f64,
        /// `CROSSED` or `ISOLATED`.
        margin_type: String,
    }
    let account = get::<Futures>(key, Method::GET, USD_FUTURES, "/fapi/v3/account", "");
    let risks = get::<Vec<Risk>>(key, Method::GET, USD_FUTURES, "/fapi/v3/positionRisk", "");
    let (account, risks) = tokio::join!(account, risks);
    let balances = margin_balances(account?.assets, Wallet::UsdFutures);
    let open: Vec<Risk> =
        risks?.into_iter().filter(|r| crypto::num(&r.position_amt) != 0.0).collect();
    let details = join_all(open.iter().map(|risk| async {
        let query = format!("symbol={}", net::percent_encode(&risk.symbol));
        let config =
            get::<Vec<Config>>(key, Method::GET, USD_FUTURES, "/fapi/v1/symbolConfig", &query);
        let day = day(USD_FUTURES[0], "/fapi/v1/ticker/24hr", &risk.symbol);
        let (config, day) = tokio::join!(config, day);
        (config.ok().and_then(|c| c.into_iter().next()), day)
    }))
    .await;
    let positions = open
        .into_iter()
        .zip(details)
        .map(|(risk, (config, day))| {
            let amount = crypto::num(&risk.position_amt);
            let settle = risk.margin_asset;
            Position {
                kind: if risk.symbol.contains('_') { "U 本位交割" } else { "U 本位永续" },
                long: amount > 0.0,
                size: amount.abs(),
                size_unit: base(&risk.symbol, &settle),
                entry: crypto::num(&risk.entry_price),
                mark: crypto::num(&risk.mark_price),
                liquidation: risk.liquidation_price.parse().ok().filter(|p: &f64| *p > 0.0),
                leverage: config.as_ref().map(|c| c.leverage),
                isolated: config.is_some_and(|c| c.margin_type == "ISOLATED"),
                pnl: crypto::num(&risk.un_realized_profit),
                pnl_asset: settle,
                exposure: crypto::num(&risk.notional),
                day,
                symbol: risk.symbol,
            }
        })
        .collect();
    Ok((balances, positions))
}

/// The COIN-M account at margin balance, and its positions.
async fn coin_futures(key: &ApiKey) -> Result<(Vec<Balance>, Vec<Position>), Error> {
    #[derive(Deserialize)]
    struct Futures {
        assets: Vec<FuturesAsset>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Risk {
        /// `BTCUSD_PERP`, `BTCUSD_250926`.
        symbol: String,
        /// Contracts, negative when short.
        position_amt: String,
        entry_price: String,
        mark_price: String,
        un_realized_profit: String,
        liquidation_price: String,
        leverage: String,
        /// `cross` or `isolated`.
        margin_type: String,
    }
    let account = get::<Futures>(key, Method::GET, COIN_FUTURES, "/dapi/v1/account", "");
    let risks = get::<Vec<Risk>>(key, Method::GET, COIN_FUTURES, "/dapi/v1/positionRisk", "");
    let (account, risks) = tokio::join!(account, risks);
    let balances = margin_balances(account?.assets, Wallet::CoinFutures);
    let open: Vec<Risk> =
        risks?.into_iter().filter(|r| crypto::num(&r.position_amt) != 0.0).collect();
    let days =
        join_all(open.iter().map(|r| day(COIN_FUTURES[0], "/dapi/v1/ticker/24hr", &r.symbol)))
            .await;
    let positions = open
        .into_iter()
        .zip(days)
        .map(|(risk, day)| {
            let contracts = crypto::num(&risk.position_amt);
            let coin = risk.symbol.split("USD").next().unwrap_or_default().to_owned();
            // A BTC contract is worth 100 USD, the others 10.
            let face = if coin == "BTC" { 100.0 } else { 10.0 };
            Position {
                kind: if risk.symbol.ends_with("_PERP") {
                    "币本位永续"
                } else {
                    "币本位交割"
                },
                long: contracts > 0.0,
                size: contracts.abs(),
                size_unit: "张".to_owned(),
                entry: crypto::num(&risk.entry_price),
                mark: crypto::num(&risk.mark_price),
                liquidation: risk.liquidation_price.parse().ok().filter(|p: &f64| *p > 0.0),
                leverage: risk.leverage.parse().ok(),
                isolated: risk.margin_type == "isolated",
                pnl: crypto::num(&risk.un_realized_profit),
                pnl_asset: coin,
                exposure: contracts * face,
                day,
                symbol: risk.symbol,
            }
        })
        .collect();
    Ok((balances, positions))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuturesAsset {
    asset: String,
    /// Wallet balance plus unrealized PnL.
    margin_balance: String,
}

fn margin_balances(assets: Vec<FuturesAsset>, wallet: Wallet) -> Vec<Balance> {
    assets
        .into_iter()
        .map(|a| Balance::new(wallet, a.asset, crypto::num(&a.margin_balance)))
        .filter(|b| b.amount != 0.0)
        .collect()
}

/// `BTCUSDT` → `BTC`, `ETHUSDT_250926` → `ETH`: what a USDⓈ-M size counts.
fn base(symbol: &str, settle: &str) -> String {
    let pair = symbol.split('_').next().unwrap_or(symbol);
    pair.strip_suffix(settle).unwrap_or(pair).to_owned()
}

/// A contract's last price and the one 24 hours before.
async fn day(host: &str, path: &str, symbol: &str) -> Option<Price> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Ticker {
        last_price: String,
        open_price: String,
    }
    // USDⓈ-M answers with one ticker, COIN-M with a list of them.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Tickers {
        One(Ticker),
        Many(Vec<Ticker>),
    }
    let url = format!("https://{host}{path}?symbol={}", net::percent_encode(symbol));
    let ticker = match http::get_json::<Tickers>(&url).await.ok()? {
        Tickers::One(ticker) => ticker,
        Tickers::Many(tickers) => tickers.into_iter().next()?,
    };
    Some(Price { last: crypto::num(&ticker.last_price), open: crypto::num(&ticker.open_price) })
}

/// Calls a signed endpoint: `query` plus the time, signed with the secret.
async fn get<T: DeserializeOwned>(
    key: &ApiKey,
    method: Method,
    hosts: &[&str],
    path: &str,
    query: &str,
) -> Result<T, Error> {
    #[derive(Deserialize)]
    struct Refusal {
        code: i64,
        #[serde(default)]
        msg: String,
    }
    let mut synced = false;
    loop {
        let mut signed = query.to_owned();
        if !signed.is_empty() {
            signed.push('&');
        }
        signed.push_str(&format!("recvWindow={RECV_WINDOW}&timestamp={}", CLOCK.now_ms()));
        let signature = account::hex(account::hmac_sha256(&key.secret, &signed).as_ref());
        let (status, body) = account::send(hosts, |client, host| {
            client
                .request(
                    method.clone(),
                    format!("https://{host}{path}?{signed}&signature={signature}"),
                )
                .header("X-MBX-APIKEY", &key.key)
        })
        .await?;
        if status.is_success() {
            return serde_json::from_str(&body).map_err(|_| http::Error::Format.into());
        }
        let refusal: Refusal =
            serde_json::from_str(&body).map_err(|_| http::Error::Status(status.as_u16()))?;
        if refusal.code == -1021 && !synced {
            sync_clock().await?;
            synced = true;
            continue;
        }
        log::warn!("binance {path}: {} {}", refusal.code, refusal.msg);
        return Err(Error::Message(refused(refusal.code, &refusal.msg)));
    }
}

/// Learns Binance's clock after a request was refused for its time.
async fn sync_clock() -> Result<(), Error> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Time {
        server_time: i64,
    }
    let time: Time = crypto::get::<Binance, _>("/api/v3/time").await?;
    CLOCK.set(time.server_time);
    Ok(())
}

fn refused(code: i64, message: &str) -> String {
    match code {
        -1021 => "本机时间与币安相差太多，请校准系统时间".to_owned(),
        -1022 => "签名不对，请检查 API Secret".to_owned(),
        -2008 | -2014 => "API Key 不存在，请检查是否填错".to_owned(),
        -2015 => {
            "币安拒绝了这个 API Key：Key 无效、没有读取权限，或者绑定的 IP 不包括本机".to_owned()
        }
        _ => format!("{message}（{code}）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The example in Binance's signed-endpoint docs.
    #[test]
    fn signs_like_the_docs() {
        let secret = "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j";
        let payload = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        assert_eq!(
            account::hex(account::hmac_sha256(secret, payload).as_ref()),
            "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71"
        );
    }

    #[test]
    fn futures_sizes_count_the_base() {
        assert_eq!(base("BTCUSDT", "USDT"), "BTC");
        assert_eq!(base("ETHUSDC_250926", "USDC"), "ETH");
        assert_eq!(base("1000PEPEUSDT", "USDT"), "1000PEPE");
    }
}
