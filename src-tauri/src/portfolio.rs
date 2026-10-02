//! Holdings on the crypto exchanges the user gave an API key: what
//! each wallet holds and the open derivatives positions, valued in USDT at
//! the exchange's own prices. `market::crypto::account` fetches them; this is
//! what they are worth, as the dropdown's total and the holdings window show
//! it. [`value`] prices one exchange's balances; [`Portfolio::of`] merges
//! every exchange into the one picture the window and the menu draw from.

use std::{
    collections::{BTreeMap, HashMap},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::market::ProviderId;

/// Where a balance sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Wallet {
    /// Binance's spot wallet.
    Spot,
    /// Bybit's unified trading account and OKX's trading account: spot and
    /// derivatives together, at equity (unrealized PnL included).
    Trading,
    Funding,
    Earn,
    /// Binance's USDⓈ-M futures wallet, at margin balance (unrealized PnL
    /// included).
    UsdFutures,
    /// Binance's COIN-M futures wallet, the same way.
    CoinFutures,
}

/// An asset in a wallet, as the exchange reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Balance {
    pub wallet: Wallet,
    pub asset: String,
    pub amount: f64,
    /// The exchange's own value in USD, for an asset the valuation can't
    /// price (no USDT pair).
    pub usd: Option<f64>,
    /// Average cost in USD, where the exchange keeps one (OKX).
    pub cost: Option<f64>,
    /// Unrealized PnL in USD against that cost.
    pub pnl: Option<f64>,
}

impl Balance {
    pub fn new(wallet: Wallet, asset: impl Into<String>, amount: f64) -> Self {
        Self { wallet, asset: asset.into(), amount, usd: None, cost: None, pnl: None }
    }
}

/// An open derivatives position.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    /// As the exchange writes it: `BTCUSDT`, `BTCUSD_PERP`, `BTC-USDT-SWAP`.
    pub symbol: String,
    pub kind: PositionKind,
    pub long: bool,
    /// How much, in `size_unit`.
    pub size: f64,
    /// The base asset, or none for a number of contracts.
    pub size_unit: Option<String>,
    pub entry: f64,
    pub mark: f64,
    pub liquidation: Option<f64>,
    pub leverage: Option<f64>,
    /// Isolated margin, not cross.
    pub isolated: bool,
    /// Unrealized, in `pnl_asset`.
    pub pnl: f64,
    pub pnl_asset: String,
    /// Exposure in USD at the mark price, negative when short.
    pub exposure: f64,
    /// The contract's own last price and the one 24 hours before, which
    /// tell what its 24h move made of the position.
    #[serde(skip)]
    pub day: Option<Price>,
}

/// What a position is: what its margin and PnL are settled in, and
/// whether it expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PositionKind {
    UsdtPerpetual,
    UsdtFutures,
    UsdcPerpetual,
    UsdcFutures,
    /// Settled in the coin itself (inverse).
    CoinPerpetual,
    CoinFutures,
    Option,
    /// A spot margin position (OKX).
    Margin,
    Other,
}

/// A price in USDT: now, and 24 hours before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub last: f64,
    pub open: f64,
}

/// One exchange's holdings, valued.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holdings {
    /// In USDT.
    pub total: f64,
    /// What the 24h price moves made of today's holdings, in USDT.
    pub change: f64,
    pub wallets: Vec<WalletValue>,
    /// Most valuable first; those without a price last.
    pub assets: Vec<Asset>,
    pub positions: Vec<PositionValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletValue {
    pub wallet: Wallet,
    pub value: f64,
}

/// An asset, across the wallets of one exchange or of all of them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub asset: String,
    pub amount: f64,
    /// In USDT; none without a USDT pair or the exchange's own value.
    pub price: Option<f64>,
    pub value: Option<f64>,
    /// What the 24h price move made of this amount, in USDT.
    pub change: Option<f64>,
    /// The 24h price change, in percent.
    pub change_pct: Option<f64>,
    /// A dollar stablecoin: cash, as far as the holdings are concerned.
    pub stable: bool,
    /// Where it sits, largest first.
    pub held: Vec<Holding>,
    pub cost: Option<f64>,
    pub pnl: Option<f64>,
}

/// Some of an asset in one wallet of one exchange.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holding {
    pub exchange: ProviderId,
    pub wallet: Wallet,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionValue {
    pub exchange: ProviderId,
    #[serde(flatten)]
    pub position: Position,
    /// The unrealized PnL in USDT, when `pnl_asset` has a price.
    pub pnl_usd: Option<f64>,
    /// What the contract's 24h move made of the position, in USDT.
    pub change: Option<f64>,
}

/// Dollar stablecoins count as cash.
pub fn is_stable(asset: &str) -> bool {
    matches!(
        asset,
        "USDT"
            | "USDC"
            | "FDUSD"
            | "USD1"
            | "TUSD"
            | "BUSD"
            | "DAI"
            | "USDE"
            | "PYUSD"
            | "USDD"
            | "USDP"
            | "RLUSD"
            | "USDS"
            | "USDG"
            | "EURC"
    )
}

/// Values `balances` and `positions` at `prices` (by asset; USDT is worth 1).
pub fn value(
    exchange: ProviderId,
    balances: &[Balance],
    positions: &[Position],
    prices: &HashMap<String, Price>,
) -> Holdings {
    let price_of = |asset: &str| match asset {
        "USDT" => Some(Price { last: 1.0, open: 1.0 }),
        _ => prices.get(asset).copied(),
    };

    let mut by_asset: BTreeMap<&str, Vec<&Balance>> = BTreeMap::new();
    for balance in balances.iter().filter(|b| b.amount != 0.0) {
        by_asset.entry(&balance.asset).or_default().push(balance);
    }
    let mut wallets: BTreeMap<Wallet, f64> = BTreeMap::new();
    let mut change = 0.0;
    let mut assets: Vec<Asset> = by_asset
        .into_iter()
        .map(|(asset, held)| {
            let price = price_of(asset);
            let amount: f64 = held.iter().map(|b| b.amount).sum();
            let mut value = None;
            for balance in &held {
                let worth = match price {
                    Some(price) => Some(balance.amount * price.last),
                    None => balance.usd,
                };
                if let Some(worth) = worth {
                    *wallets.entry(balance.wallet).or_default() += worth;
                    *value.get_or_insert(0.0) += worth;
                }
            }
            let moved = price.map(|p| amount * (p.last - p.open));
            change += moved.unwrap_or(0.0);
            let mut where_held: Vec<Holding> = held
                .iter()
                .map(|b| Holding { exchange, wallet: b.wallet, amount: b.amount })
                .collect();
            where_held.sort_by(|a, b| b.amount.total_cmp(&a.amount));
            let costed: Vec<&&Balance> = held.iter().filter(|b| b.cost.is_some()).collect();
            Asset {
                asset: asset.to_owned(),
                amount,
                price: price.map(|p| p.last).or_else(|| value.map(|v| v / amount)),
                value,
                change: moved,
                change_pct: price.and_then(|p| pct(p.last, p.open)),
                stable: is_stable(asset),
                held: where_held,
                cost: costed.first().and_then(|b| b.cost),
                pnl: costed.iter().filter_map(|b| b.pnl).reduce(|a, b| a + b),
            }
        })
        .collect();
    sort_assets(&mut assets);

    let positions = positions
        .iter()
        .map(|position| {
            let moved = position
                .day
                .filter(|day| day.last > 0.0)
                .map(|Price { last, open }| position.exposure * (last - open) / last);
            change += moved.unwrap_or(0.0);
            PositionValue {
                exchange,
                position: position.clone(),
                pnl_usd: price_of(&position.pnl_asset).map(|p| position.pnl * p.last),
                change: moved,
            }
        })
        .collect();

    Holdings {
        total: wallets.values().sum(),
        change,
        wallets: wallets.into_iter().map(|(wallet, value)| WalletValue { wallet, value }).collect(),
        assets,
        positions,
    }
}

/// Most valuable first; those without a price last, by name.
fn sort_assets(assets: &mut [Asset]) {
    assets.sort_by(|a, b| match (a.value, b.value) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.asset.cmp(&b.asset),
    });
}

/// Epoch milliseconds, as holdings carry times.
pub fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

/// `(last − open) / open` in percent, when there is an `open`.
pub fn pct(last: f64, open: f64) -> Option<f64> {
    (open > 0.0).then(|| (last - open) / open * 100.0)
}

/// One exchange's account, as the app keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub exchange: ProviderId,
    /// The last good refresh; none before the first.
    pub holdings: Option<Holdings>,
    /// When it was taken, epoch milliseconds.
    pub updated: Option<f64>,
    /// Why the last refresh failed, if it did.
    pub error: Option<String>,
    /// When the last refresh, good or not, finished.
    pub checked: Option<f64>,
}

impl Account {
    pub fn new(exchange: ProviderId) -> Self {
        Self { exchange, holdings: None, updated: None, error: None, checked: None }
    }
}

/// One exchange's part of the portfolio: its totals and where they sit,
/// without the assets (the portfolio merges those).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub exchange: ProviderId,
    /// None before the first good refresh.
    pub total: Option<f64>,
    pub change: Option<f64>,
    pub wallets: Vec<WalletValue>,
    /// Epoch milliseconds of the last good refresh.
    pub updated: Option<f64>,
    pub error: Option<String>,
}

/// Every exchange with a key, as one picture: each account's totals, and
/// the assets and positions of all of them together.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Portfolio {
    pub accounts: Vec<AccountView>,
    /// Over the accounts refreshed at least once, in USDT; none before.
    pub total: Option<f64>,
    pub change: Option<f64>,
    /// Merged by asset across the accounts, most valuable first.
    pub assets: Vec<Asset>,
    /// Largest unrealized PnL (either way) first.
    pub positions: Vec<PositionValue>,
}

impl Portfolio {
    pub fn of(accounts: &BTreeMap<ProviderId, Account>) -> Self {
        let totals = Totals::of(accounts);
        let valued = accounts.values().filter_map(|a| a.holdings.as_ref());
        let mut positions: Vec<PositionValue> =
            valued.clone().flat_map(|h| h.positions.iter().cloned()).collect();
        positions.sort_by(|a, b| {
            let size = |p: &PositionValue| p.pnl_usd.unwrap_or(p.position.pnl).abs();
            size(b).total_cmp(&size(a))
        });
        Self {
            accounts: accounts
                .values()
                .map(|account| AccountView {
                    exchange: account.exchange,
                    total: account.holdings.as_ref().map(|h| h.total),
                    change: account.holdings.as_ref().map(|h| h.change),
                    wallets: account
                        .holdings
                        .as_ref()
                        .map(|h| h.wallets.clone())
                        .unwrap_or_default(),
                    updated: account.updated,
                    error: account.error.clone(),
                })
                .collect(),
            total: totals.map(|t| t.total),
            change: totals.map(|t| t.change),
            assets: merge(valued.flat_map(|h| h.assets.iter())),
            positions,
        }
    }
}

/// The same asset on several exchanges becomes one line: amounts, values
/// and moves add up, and `held` lists every wallet of every exchange.
fn merge<'a>(assets: impl Iterator<Item = &'a Asset>) -> Vec<Asset> {
    let mut by_asset: BTreeMap<&str, Asset> = BTreeMap::new();
    for asset in assets {
        match by_asset.get_mut(asset.asset.as_str()) {
            None => {
                by_asset.insert(&asset.asset, asset.clone());
            }
            Some(merged) => {
                merged.amount += asset.amount;
                merged.value = add(merged.value, asset.value);
                merged.change = add(merged.change, asset.change);
                merged.pnl = add(merged.pnl, asset.pnl);
                merged.price = merged.price.or(asset.price);
                merged.change_pct = merged.change_pct.or(asset.change_pct);
                merged.cost = merged.cost.or(asset.cost);
                merged.held.extend(asset.held.iter().cloned());
            }
        }
    }
    let mut merged: Vec<Asset> = by_asset.into_values().collect();
    for asset in &mut merged {
        asset.held.sort_by(|a, b| b.amount.total_cmp(&a.amount));
        // Priced on one exchange, valued by the other: one price for the whole.
        if asset.price.is_none() {
            asset.price = asset.value.map(|v| v / asset.amount);
        }
    }
    sort_assets(&mut merged);
    merged
}

/// A sum that is unknown only while both parts are.
fn add(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (None, None) => None,
        _ => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    }
}

/// The sum over the accounts refreshed at least once, in USDT.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Totals {
    pub total: f64,
    pub change: f64,
}

impl Totals {
    /// None before any account has refreshed.
    pub fn of(accounts: &BTreeMap<ProviderId, Account>) -> Option<Self> {
        accounts.values().filter_map(|a| a.holdings.as_ref()).fold(None, |sum, h| {
            let Self { total, change } = sum.unwrap_or(Self { total: 0.0, change: 0.0 });
            Some(Self { total: total + h.total, change: change + h.change })
        })
    }

    /// The 24h change in percent of what the holdings were worth a day ago.
    pub fn change_pct(self) -> Option<f64> {
        pct(self.total, self.total - self.change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prices() -> HashMap<String, Price> {
        [("BTC", 100.0, 80.0), ("ETH", 10.0, 10.0)]
            .map(|(asset, last, open)| (asset.to_owned(), Price { last, open }))
            .into()
    }

    #[test]
    fn assets_add_up_across_wallets() {
        let balances = [
            Balance::new(Wallet::Spot, "BTC", 1.0),
            Balance::new(Wallet::Earn, "BTC", 0.5),
            Balance::new(Wallet::Funding, "USDT", 50.0),
            // No USDT pair, but the exchange values it.
            Balance { usd: Some(7.0), ..Balance::new(Wallet::Funding, "EUR", 6.0) },
            Balance::new(Wallet::Spot, "NOPAIR", 3.0),
            Balance::new(Wallet::Spot, "ETH", 0.0),
        ];
        let holdings = value(ProviderId::Binance, &balances, &[], &prices());
        let assets: Vec<(&str, Option<f64>)> =
            holdings.assets.iter().map(|a| (a.asset.as_str(), a.value)).collect();
        assert_eq!(
            assets,
            [("BTC", Some(150.0)), ("USDT", Some(50.0)), ("EUR", Some(7.0)), ("NOPAIR", None)]
        );
        assert_eq!(holdings.total, 207.0);
        // BTC rose 20 on 1.5 held.
        assert_eq!(holdings.change, 30.0);
        assert_eq!(holdings.assets[0].change, Some(30.0));
        assert_eq!(holdings.assets[0].change_pct, Some(25.0));
        assert!(holdings.assets[1].stable && !holdings.assets[0].stable);
        let wallets: Vec<(Wallet, f64)> =
            holdings.wallets.iter().map(|w| (w.wallet, w.value)).collect();
        assert_eq!(wallets, [(Wallet::Spot, 100.0), (Wallet::Funding, 57.0), (Wallet::Earn, 50.0)]);
        assert_eq!(holdings.assets[0].held[0].wallet, Wallet::Spot);
    }

    #[test]
    fn positions_move_with_their_base() {
        let short = Position {
            symbol: "BTCUSDT".into(),
            kind: PositionKind::UsdtPerpetual,
            long: false,
            size: 2.0,
            size_unit: Some("BTC".into()),
            entry: 90.0,
            mark: 100.0,
            liquidation: None,
            leverage: Some(5.0),
            isolated: false,
            pnl: -20.0,
            pnl_asset: "USDT".into(),
            exposure: -200.0,
            day: Some(Price { last: 100.0, open: 80.0 }),
        };
        let holdings = value(
            ProviderId::Binance,
            &[Balance::new(Wallet::UsdFutures, "USDT", 1000.0)],
            &[short],
            &prices(),
        );
        // The contract rose from 80 to 100: the short, worth 200 now, lost 40.
        assert_eq!(holdings.change, -40.0);
        assert_eq!(holdings.positions[0].change, Some(-40.0));
        assert_eq!(holdings.total, 1000.0);
        assert_eq!(holdings.positions[0].pnl_usd, Some(-20.0));
    }

    fn account(exchange: ProviderId, balances: &[Balance]) -> Account {
        Account {
            holdings: Some(value(exchange, balances, &[], &prices())),
            updated: Some(1.0),
            ..Account::new(exchange)
        }
    }

    // The window and the menu show one line per asset however many
    // exchanges hold it, with each exchange's wallets listed under it.
    #[test]
    fn the_portfolio_merges_assets_across_exchanges() {
        let accounts: BTreeMap<ProviderId, Account> = [
            (
                ProviderId::Binance,
                account(
                    ProviderId::Binance,
                    &[
                        Balance::new(Wallet::Spot, "BTC", 1.0),
                        Balance::new(Wallet::Spot, "USDT", 10.0),
                    ],
                ),
            ),
            (
                ProviderId::Okx,
                account(ProviderId::Okx, &[Balance::new(Wallet::Trading, "BTC", 0.5)]),
            ),
            (ProviderId::Bybit, Account::new(ProviderId::Bybit)),
        ]
        .into();
        let portfolio = Portfolio::of(&accounts);
        assert_eq!((portfolio.total, portfolio.change), (Some(160.0), Some(30.0)));
        let btc = &portfolio.assets[0];
        assert_eq!((btc.asset.as_str(), btc.amount, btc.value), ("BTC", 1.5, Some(150.0)));
        assert_eq!(btc.change, Some(30.0));
        let held: Vec<(ProviderId, f64)> =
            btc.held.iter().map(|h| (h.exchange, h.amount)).collect();
        assert_eq!(held, [(ProviderId::Binance, 1.0), (ProviderId::Okx, 0.5)]);
        assert_eq!(portfolio.accounts.len(), 3);
        // Accounts come in ProviderId order: Binance, Bybit (never read), OKX.
        assert_eq!(portfolio.accounts[1].total, None);
        let pct = Totals::of(&accounts).and_then(Totals::change_pct).unwrap();
        assert!((pct - 30.0 / 130.0 * 100.0).abs() < 1e-9);
        assert_eq!(Portfolio::of(&BTreeMap::new()).total, None);
    }
}
