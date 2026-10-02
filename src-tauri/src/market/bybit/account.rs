//! A Bybit unified trading account through the user's V5 API key.

use futures_util::future::join_all;
use reqwest::StatusCode;
use serde::{Deserialize, de::DeserializeOwned};

use super::{Bybit, Reply};
use crate::{
    credentials::ApiKey,
    http,
    market::{
        Error,
        crypto::{
            self, Exchange,
            account::{self, Account, Clock},
        },
    },
    portfolio::{Balance, Position, PositionKind, Price, Wallet},
    sign,
};

/// How far a request's time may trail Bybit's clock.
const RECV_WINDOW: &str = "10000";

static CLOCK: Clock = Clock::new();

impl Account for Bybit {
    async fn check(key: &ApiKey) -> Result<(), Error> {
        // Answers any working key, whatever it may do.
        get::<serde::de::IgnoredAny>(key, "/v5/user/query-api", "").await?;
        Ok(())
    }

    async fn trading(key: &ApiKey) -> Result<(Vec<Balance>, Vec<Position>), Error> {
        #[derive(Deserialize)]
        struct Wallets {
            list: Vec<UnifiedWallet>,
        }
        #[derive(Deserialize)]
        struct UnifiedWallet {
            coin: Vec<Coin>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Coin {
            coin: String,
            /// Wallet balance less borrowing, plus unrealized PnL.
            equity: String,
            usd_value: String,
        }
        let wallets = get::<Wallets>(key, "/v5/account/wallet-balance", "accountType=UNIFIED");
        let (wallets, positions) = tokio::join!(wallets, positions(key));
        let balances = wallets?
            .list
            .into_iter()
            .flat_map(|wallet| wallet.coin)
            .map(|coin| Balance {
                usd: coin.usd_value.parse().ok(),
                ..Balance::new(Wallet::Trading, coin.coin, crypto::num(&coin.equity))
            })
            .collect();
        Ok((balances, positions?))
    }

    async fn savings(key: &ApiKey) -> Result<Vec<Balance>, Error> {
        #[derive(Deserialize)]
        struct Funding {
            balance: Vec<FundingCoin>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FundingCoin {
            coin: String,
            wallet_balance: String,
        }
        let funding = get::<Funding>(
            key,
            "/v5/asset/transfer/query-account-coins-balance",
            "accountType=FUND",
        );
        let (funding, flexible, on_chain) =
            tokio::join!(funding, earn(key, "FlexibleSaving"), earn(key, "OnChain"));
        let mut balances: Vec<Balance> = funding?
            .balance
            .into_iter()
            .map(|c| Balance::new(Wallet::Funding, c.coin, crypto::num(&c.wallet_balance)))
            .collect();
        // Earn needs a permission of its own; a key without it still shows the rest.
        for earned in [flexible, on_chain] {
            match earned {
                Ok(earned) => balances.extend(earned),
                Err(e) => log::info!("bybit earn: {e}"),
            }
        }
        Ok(balances)
    }
}

async fn earn(key: &ApiKey, category: &str) -> Result<Vec<Balance>, Error> {
    #[derive(Deserialize)]
    struct Earn {
        list: Vec<Holding>,
    }
    #[derive(Deserialize)]
    struct Holding {
        coin: String,
        amount: String,
    }
    let earn: Earn = get(key, "/v5/earn/position", &format!("category={category}")).await?;
    Ok(earn
        .list
        .into_iter()
        .map(|h| Balance::new(Wallet::Earn, h.coin, crypto::num(&h.amount)))
        .collect())
}

/// What `/v5/position/list` is asked for: USDT- and USDC-settled contracts
/// have to be asked for by settle coin, inverse ones and options come whole.
const POSITION_QUERIES: [(&str, &str); 4] = [
    ("linear", "&settleCoin=USDT"),
    ("linear", "&settleCoin=USDC"),
    ("inverse", ""),
    ("option", ""),
];

async fn positions(key: &ApiKey) -> Result<Vec<Position>, Error> {
    let lists = join_all(POSITION_QUERIES.map(|(category, filter)| async move {
        let mut found = Vec::new();
        let mut cursor = String::new();
        loop {
            let query = format!(
                "category={category}{filter}&limit=200&cursor={}",
                crate::net::percent_encode(&cursor)
            );
            let page: Page = get(key, "/v5/position/list", &query).await?;
            found.extend(page.list.into_iter().filter_map(|raw| raw.position(category)));
            if page.next_page_cursor.is_empty() {
                break;
            }
            cursor = page.next_page_cursor;
        }
        // Each contract's own 24h move.
        let days = join_all(found.iter().map(|p| day(category, &p.symbol))).await;
        for (position, day) in found.iter_mut().zip(days) {
            position.day = day;
        }
        Ok::<_, Error>(found)
    }))
    .await;
    let mut positions = Vec::new();
    for list in lists {
        positions.extend(list?);
    }
    Ok(positions)
}

/// The contract's last price and the one 24 hours before.
async fn day(category: &str, symbol: &str) -> Option<Price> {
    #[derive(Deserialize)]
    struct Tickers {
        list: Vec<Ticker>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Ticker {
        last_price: String,
        prev_price_24h: String,
    }
    let path = format!(
        "/v5/market/tickers?category={category}&symbol={}",
        crate::net::percent_encode(symbol)
    );
    let tickers: Tickers = super::get(&path).await.ok()?;
    let ticker = tickers.list.into_iter().next()?;
    Some(Price { last: crypto::num(&ticker.last_price), open: crypto::num(&ticker.prev_price_24h) })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    list: Vec<RawPosition>,
    #[serde(default)]
    next_page_cursor: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPosition {
    symbol: String,
    /// `Buy`, `Sell`, or empty without a position.
    side: String,
    size: String,
    avg_price: String,
    mark_price: String,
    liq_price: String,
    leverage: String,
    unrealised_pnl: String,
}

impl RawPosition {
    fn position(self, category: &str) -> Option<Position> {
        let size = crypto::num(&self.size);
        if size.is_nan() || size <= 0.0 || self.side.is_empty() {
            return None;
        }
        let long = self.side == "Buy";
        let mark = crypto::num(&self.mark_price);
        // Inverse contracts are a dollar each; the others count the base asset.
        let (kind, size_unit, pnl_asset, exposure) = match category {
            "inverse" => {
                let base = self.symbol.split("USD").next().unwrap_or_default().to_owned();
                (Kind::Inverse, Some("USD".to_owned()), base, size)
            }
            "option" => {
                let base = self.symbol.split('-').next().unwrap_or_default().to_owned();
                (Kind::Option, Some(base), "USDC".to_owned(), 0.0)
            }
            _ => {
                let settle = if self.symbol.contains("USDT") { "USDT" } else { "USDC" };
                let base = self.symbol.split(['-']).next().unwrap_or_default();
                let base =
                    base.strip_suffix(settle).or_else(|| base.strip_suffix("PERP")).unwrap_or(base);
                let kind = if settle == "USDT" { Kind::Usdt } else { Kind::Usdc };
                (kind, Some(base.to_owned()), settle.to_owned(), size * mark)
            }
        };
        Some(Position {
            kind: kind.of(delivery(&self.symbol, category)),
            long,
            size,
            size_unit,
            entry: crypto::num(&self.avg_price),
            mark,
            liquidation: self.liq_price.parse().ok().filter(|p: &f64| *p > 0.0),
            leverage: self.leverage.parse().ok(),
            // Unified accounts set the margin mode for the whole account.
            isolated: false,
            pnl: crypto::num(&self.unrealised_pnl),
            pnl_asset,
            exposure: if long { exposure } else { -exposure },
            day: None,
            symbol: self.symbol,
        })
    }
}

/// Dated contracts: `BTCUSDT-26DEC25`, `BTC-26DEC25`, inverse `BTCUSDZ25`.
fn delivery(symbol: &str, category: &str) -> bool {
    symbol.contains('-') || (category == "inverse" && !symbol.ends_with("USD"))
}

enum Kind {
    Usdt,
    Usdc,
    Inverse,
    Option,
}

impl Kind {
    fn of(self, delivery: bool) -> PositionKind {
        match (self, delivery) {
            (Self::Usdt, false) => PositionKind::UsdtPerpetual,
            (Self::Usdt, true) => PositionKind::UsdtFutures,
            (Self::Usdc, false) => PositionKind::UsdcPerpetual,
            (Self::Usdc, true) => PositionKind::UsdcFutures,
            (Self::Inverse, false) => PositionKind::CoinPerpetual,
            (Self::Inverse, true) => PositionKind::CoinFutures,
            (Self::Option, _) => PositionKind::Option,
        }
    }
}

/// GETs a private V5 endpoint's `result`, signed with `key`.
async fn get<T: DeserializeOwned>(key: &ApiKey, path: &str, query: &str) -> Result<T, Error> {
    let mut synced = false;
    loop {
        let timestamp = CLOCK.now_ms().to_string();
        let payload = format!("{timestamp}{}{RECV_WINDOW}{query}", key.key);
        let signature = sign::hex(sign::hmac_sha256(&key.secret, &payload).as_ref());
        let (status, body) = account::send(Bybit::REST_HOSTS, |client, host| {
            client
                .get(format!("https://{host}{path}?{query}"))
                .header("X-BAPI-API-KEY", &key.key)
                .header("X-BAPI-TIMESTAMP", &timestamp)
                .header("X-BAPI-RECV-WINDOW", RECV_WINDOW)
                .header("X-BAPI-SIGN", &signature)
                .header("X-BAPI-SIGN-TYPE", "2")
        })
        .await?;
        // Some endpoints refuse a key with a bare 401, saying no more.
        if status == StatusCode::UNAUTHORIZED {
            return Err(Error::Message(t!("error.keyRejected").to_owned()));
        }
        let reply: Reply =
            serde_json::from_str(&body).map_err(|_| http::Error::Status(status.as_u16()))?;
        match reply.ret_code {
            0 => {
                return serde_json::from_str(reply.result.get())
                    .map_err(|_| http::Error::Format.into());
            }
            10002 if !synced => {
                sync_clock().await?;
                synced = true;
            }
            code => {
                log::warn!("bybit {path}: {code} {}", reply.ret_msg);
                return Err(Error::Message(refusal(code, &reply.ret_msg)));
            }
        }
    }
}

/// Learns Bybit's clock after a request was refused for its time.
async fn sync_clock() -> Result<(), Error> {
    #[derive(Deserialize)]
    struct Time {
        time: i64,
    }
    let time: Time = crypto::get::<Bybit, _>("/v5/market/time").await?;
    CLOCK.set(time.time);
    Ok(())
}

fn refusal(code: i64, message: &str) -> String {
    let known = match code {
        10002 => t!("error.clockSkew"),
        10003 => t!("error.keyNotFound"),
        10004 => return t!("error.wrongSecret", field = "API Secret"),
        10005 => t!("error.noReadPermission"),
        10010 => t!("error.ipBound"),
        33004 => t!("error.keyExpired"),
        _ => return t!("error.exchangeReply", message = message, code = code),
    };
    known.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bybit's docs give no secret with their example, so these vectors were
    // worked out with Python's hmac and openssl from the documented payload
    // shape: timestamp, key, recv window, query.
    #[test]
    fn signs_like_the_docs_describe() {
        let sign = |payload: &str| {
            sign::hex(sign::hmac_sha256("testsecret0123456789abcdefABCDEF0123", payload).as_ref())
        };
        assert_eq!(
            sign("1658384314791XXXXXXXXXX5000category=option&symbol=BTC-29JUL22-25000-C"),
            "792032b419394fad6c7592ca49202f1c07ba0482c88ed1e2bf270b4a43219b53"
        );
        assert_eq!(
            sign("1790831328391XXXXXXXXXX5000accountType=UNIFIED"),
            "ff2e4dbf79f7c43f78d63f1a734b00ecc4331c1918eb1f95b9e7626adae22fd7"
        );
    }

    fn raw(symbol: &str, side: &str, size: &str) -> RawPosition {
        RawPosition {
            symbol: symbol.into(),
            side: side.into(),
            size: size.into(),
            avg_price: "100".into(),
            mark_price: "110".into(),
            liq_price: "".into(),
            leverage: "10".into(),
            unrealised_pnl: "5".into(),
        }
    }

    #[test]
    fn positions_read_their_contract() {
        let long = raw("1000PEPEUSDT", "Buy", "2").position("linear").unwrap();
        assert_eq!(
            (long.kind, long.size_unit.as_deref(), long.exposure),
            (PositionKind::UsdtPerpetual, Some("1000PEPE"), 220.0)
        );
        assert_eq!(long.liquidation, None);
        let short = raw("BTCUSDZ25", "Sell", "500").position("inverse").unwrap();
        assert_eq!(
            (short.kind, short.pnl_asset.as_str(), short.exposure),
            (PositionKind::CoinFutures, "BTC", -500.0)
        );
        let usdc = raw("ETH-26DEC25", "Buy", "1").position("linear").unwrap();
        assert_eq!(
            (usdc.kind, usdc.size_unit.as_deref()),
            (PositionKind::UsdcFutures, Some("ETH"))
        );
        assert!(raw("BTCUSDT", "", "0").position("linear").is_none());
    }
}
