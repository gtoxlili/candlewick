//! An OKX account through a read-only API key (key, secret and passphrase).

use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::future::join_all;
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::value::RawValue;

use super::Okx;
use crate::{
    calendar,
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

/// Accounts registered in the US or Australia answer only on `us.okx.com`,
/// those in the EEA only on `eea.okx.com`; elsewhere a key is unknown.
const REGIONS: [&[&str]; 3] =
    [&["openapi.okx.com", "www.okx.com"], &["us.okx.com"], &["eea.okx.com"]];
/// The region that last knew the key.
static REGION: AtomicUsize = AtomicUsize::new(0);

static CLOCK: Clock = Clock::new();

impl Account for Okx {
    async fn check(key: &ApiKey) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct Config {
            /// `read_only`, plus `trade` and `withdraw` when allowed.
            perm: String,
        }
        let config: Vec<Config> = get(key, "/api/v5/account/config").await?;
        let perm = config.first().map(|c| c.perm.as_str()).unwrap_or_default();
        if perm.split(',').any(|p| p.trim() != "read_only") {
            return Err(Error::Message("这个 API Key 可以交易或提币，请换成只读 Key".to_owned()));
        }
        Ok(())
    }

    async fn trading(key: &ApiKey) -> Result<(Vec<Balance>, Vec<Position>), Error> {
        #[derive(Deserialize)]
        struct Account {
            details: Vec<Detail>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Detail {
            ccy: String,
            /// Equity, unrealized PnL included.
            eq: String,
            eq_usd: String,
            /// Average cost of the spot holding in USD; empty for stablecoins
            /// and where OKX keeps none.
            open_avg_px: String,
            spot_upl: String,
        }
        let account = get::<Account>(key, "/api/v5/account/balance");
        let positions = get::<RawPosition>(key, "/api/v5/account/positions");
        let (account, positions) = tokio::join!(account, positions);
        let balances = account?
            .into_iter()
            .flat_map(|account| account.details)
            .map(|d| Balance {
                usd: d.eq_usd.parse().ok(),
                cost: d.open_avg_px.parse().ok(),
                pnl: d.spot_upl.parse().ok(),
                ..Balance::new(Wallet::Trading, d.ccy, crypto::num(&d.eq))
            })
            .collect();
        let mut positions: Vec<Position> =
            positions?.into_iter().filter_map(RawPosition::position).collect();
        // Each contract's own 24h move.
        let days = join_all(positions.iter().map(|p| day(&p.symbol))).await;
        for (position, day) in positions.iter_mut().zip(days) {
            position.day = day;
        }
        Ok((balances, positions))
    }

    async fn savings(key: &ApiKey) -> Result<Vec<Balance>, Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Funding {
            ccy: String,
            bal: String,
        }
        #[derive(Deserialize)]
        struct Saving {
            ccy: String,
            amt: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Staking {
            invest_data: Vec<Saving>,
        }
        let (funding, savings, staking) = tokio::join!(
            get::<Funding>(key, "/api/v5/asset/balances"),
            get::<Saving>(key, "/api/v5/finance/savings/balance"),
            get::<Staking>(key, "/api/v5/finance/staking-defi/orders-active"),
        );
        let mut balances: Vec<Balance> = funding?
            .into_iter()
            .map(|f| Balance::new(Wallet::Funding, f.ccy, crypto::num(&f.bal)))
            .collect();
        // ETH and SOL staking (BETH, OKSOL) sit in the trading and funding
        // accounts already, so only these are added. Without them the rest
        // still shows.
        let savings = savings.inspect_err(|e| log::info!("okx savings: {e}")).unwrap_or_default();
        let staking = staking.inspect_err(|e| log::info!("okx staking: {e}")).unwrap_or_default();
        let earned =
            savings.into_iter().chain(staking.into_iter().flat_map(|order| order.invest_data));
        balances.extend(earned.map(|s| Balance::new(Wallet::Earn, s.ccy, crypto::num(&s.amt))));
        Ok(balances)
    }
}

/// The contract's last price and the one 24 hours before.
async fn day(inst_id: &str) -> Option<Price> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Ticker {
        last: String,
        open_24h: String,
    }
    let path = format!("/api/v5/market/ticker?instId={}", net::percent_encode(inst_id));
    let ticker: Ticker = super::get(&path).await.ok()?.into_iter().next()?;
    Some(Price { last: crypto::num(&ticker.last), open: crypto::num(&ticker.open_24h) })
}

/// `GET /api/v5/account/positions`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPosition {
    /// `MARGIN`, `SWAP`, `FUTURES`, `OPTION`.
    inst_type: String,
    inst_id: String,
    /// `long`, `short`, or `net`, where the sign of `pos` tells.
    pos_side: String,
    /// Contracts; for margin, the currency in `pos_ccy`.
    pos: String,
    pos_ccy: String,
    avg_px: String,
    mark_px: String,
    liq_px: String,
    lever: String,
    /// `cross` or `isolated`.
    mgn_mode: String,
    /// In `ccy`.
    upl: String,
    ccy: String,
    notional_usd: String,
}

impl RawPosition {
    fn position(self) -> Option<Position> {
        let pos = crypto::num(&self.pos);
        if pos.is_nan() || pos == 0.0 {
            return None;
        }
        let long = match self.pos_side.as_str() {
            "long" => true,
            "short" => false,
            // Margin: borrowing the quote asset to hold the base is long.
            _ if self.inst_type == "MARGIN" => {
                self.inst_id.starts_with(&format!("{}-", self.pos_ccy))
            }
            _ => pos > 0.0,
        };
        let exposure = crypto::num(&self.notional_usd).abs();
        Some(Position {
            kind: kind(&self.inst_type, &self.inst_id),
            long,
            size: pos.abs(),
            size_unit: if self.inst_type == "MARGIN" { self.pos_ccy } else { "张".to_owned() },
            entry: crypto::num(&self.avg_px),
            mark: crypto::num(&self.mark_px),
            liquidation: self.liq_px.parse().ok().filter(|p: &f64| *p > 0.0),
            leverage: self.lever.parse().ok(),
            isolated: self.mgn_mode == "isolated",
            pnl: crypto::num(&self.upl),
            pnl_asset: self.ccy,
            exposure: if !exposure.is_finite() || self.inst_type == "OPTION" {
                0.0
            } else if long {
                exposure
            } else {
                -exposure
            },
            day: None,
            symbol: self.inst_id,
        })
    }
}

/// `BTC-USDT-SWAP` → `U 本位永续`, `BTC-USD-250926` → `币本位交割`.
fn kind(inst_type: &str, inst_id: &str) -> &'static str {
    let settle = inst_id.split('-').nth(1).unwrap_or_default();
    match (inst_type, settle) {
        ("SWAP", "USDT") => "U 本位永续",
        ("SWAP", "USDC") => "USDC 永续",
        ("SWAP", _) => "币本位永续",
        ("FUTURES", "USDT") => "U 本位交割",
        ("FUTURES", "USDC") => "USDC 交割",
        ("FUTURES", _) => "币本位交割",
        ("OPTION", _) => "期权",
        ("MARGIN", _) => "杠杆",
        _ => "合约",
    }
}

/// `{"code": "0", "msg": "", "data": […]}`; `code` is a number on some
/// refusals, and `data` may be missing.
#[derive(Deserialize)]
struct Reply<'a> {
    #[serde(deserialize_with = "code")]
    code: String,
    #[serde(default)]
    msg: String,
    #[serde(borrow)]
    data: Option<&'a RawValue>,
}

fn code<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Code {
        Text(String),
        Number(i64),
    }
    Ok(match Code::deserialize(deserializer)? {
        Code::Text(text) => text,
        Code::Number(number) => number.to_string(),
    })
}

/// GETs a private endpoint's `data`, signed with `key`, from the region that
/// knows the key.
async fn get<T: DeserializeOwned>(key: &ApiKey, path: &str) -> Result<Vec<T>, Error> {
    let passphrase = key.passphrase.as_deref().unwrap_or_default();
    let first = REGION.load(Ordering::Relaxed);
    let mut synced = false;
    let mut tried = 0;
    loop {
        let region = (first + tried) % REGIONS.len();
        let timestamp = calendar::iso8601_ms(CLOCK.now_ms());
        let prehash = format!("{timestamp}GET{path}");
        let signature = account::base64(account::hmac_sha256(&key.secret, &prehash).as_ref());
        let (status, body) = account::send(REGIONS[region], |client, host| {
            client
                .get(format!("https://{host}{path}"))
                .header("OK-ACCESS-KEY", &key.key)
                .header("OK-ACCESS-SIGN", &signature)
                .header("OK-ACCESS-TIMESTAMP", &timestamp)
                .header("OK-ACCESS-PASSPHRASE", passphrase)
        })
        .await?;
        let reply: Reply =
            serde_json::from_str(&body).map_err(|_| http::Error::Status(status.as_u16()))?;
        match reply.code.as_str() {
            "0" => {
                REGION.store(region, Ordering::Relaxed);
                let data = reply.data.map_or("[]", RawValue::get);
                return serde_json::from_str(data).map_err(|_| http::Error::Format.into());
            }
            // Unknown here: maybe a regional account.
            "50119" if tried + 1 < REGIONS.len() => tried += 1,
            "50102" if !synced => {
                sync_clock().await?;
                synced = true;
            }
            code => {
                log::warn!("okx {path}: {code} {}", reply.msg);
                return Err(Error::Message(refusal(code, &reply.msg)));
            }
        }
    }
}

/// Learns OKX's clock after a request was refused for its time.
async fn sync_clock() -> Result<(), Error> {
    #[derive(Deserialize)]
    struct Time {
        ts: String,
    }
    let time: Vec<Time> = super::get("/api/v5/public/time").await?;
    if let Some(ts) = time.first().and_then(|t| t.ts.parse().ok()) {
        CLOCK.set(ts);
    }
    Ok(())
}

fn refusal(code: &str, message: &str) -> String {
    match code {
        "50102" => "本机时间与 OKX 相差太多，请校准系统时间".to_owned(),
        "50105" => "Passphrase 不对".to_owned(),
        "50110" => "这个 API Key 绑定的 IP 不包括本机".to_owned(),
        "50111" | "50119" => "API Key 不存在，请检查是否填错".to_owned(),
        "50113" => "签名不对，请检查 Secret Key".to_owned(),
        "50120" | "50030" => "这个 API Key 没有读取这项数据的权限".to_owned(),
        "50101" => "这是模拟盘的 API Key，请使用实盘 Key".to_owned(),
        _ => format!("{message}（{code}）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Computed from the docs' example secret and request time; OKX gives the
    // formula but no expected output.
    #[test]
    fn signs_like_the_docs_describe() {
        let sign = |prehash: &str| {
            account::base64(
                account::hmac_sha256("22582BD0CFF14C41EDBF1AB98506286D", prehash).as_ref(),
            )
        };
        assert_eq!(
            sign("2020-12-08T09:08:57.715ZGET/api/v5/account/balance?ccy=BTC"),
            "HiZhvSfMtWJA3uUIVXV3a/bSXNPCWvYFXoGCVS8V4zY="
        );
        assert_eq!(
            sign("2020-12-08T09:08:57.715ZGET/api/v5/account/balance"),
            "AkD5YszBhggtIyjDlmTy/9PpNVntel+1Lff8wh0qpQw="
        );
    }

    #[test]
    fn refusals_carry_codes_either_way() {
        let text: Reply =
            serde_json::from_str(r#"{"msg":"API key doesn't exist","code":"50119"}"#).unwrap();
        assert_eq!(text.code, "50119");
        let number: Reply =
            serde_json::from_str(r#"{"code":404,"msg":"Not Found","data":{}}"#).unwrap();
        assert_eq!(number.code, "404");
    }

    fn raw(inst_type: &str, inst_id: &str, pos_side: &str, pos: &str) -> RawPosition {
        RawPosition {
            inst_type: inst_type.into(),
            inst_id: inst_id.into(),
            pos_side: pos_side.into(),
            pos: pos.into(),
            pos_ccy: "".into(),
            avg_px: "100".into(),
            mark_px: "110".into(),
            liq_px: "".into(),
            lever: "5".into(),
            mgn_mode: "isolated".into(),
            upl: "1".into(),
            ccy: "USDT".into(),
            notional_usd: "550".into(),
        }
    }

    #[test]
    fn positions_read_their_side() {
        let net_short = raw("SWAP", "BTC-USDT-SWAP", "net", "-5").position().unwrap();
        assert_eq!(
            (net_short.kind, net_short.long, net_short.size, net_short.exposure),
            ("U 本位永续", false, 5.0, -550.0)
        );
        let long = raw("FUTURES", "BTC-USD-250926", "long", "3").position().unwrap();
        assert_eq!((long.kind, long.long, long.exposure), ("币本位交割", true, 550.0));
        assert!(raw("SWAP", "ETH-USDT-SWAP", "net", "0").position().is_none());
    }
}
