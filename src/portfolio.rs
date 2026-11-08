//! Holdings on the crypto exchanges the user gave an API key: what
//! each wallet holds and the open derivatives positions, valued in USD at
//! the exchange's own prices. `market::crypto::account` fetches them; this is
//! what they are worth, as the dropdown's total and the holdings window show
//! it. [`value`] prices one exchange's balances; [`Portfolio::of`] merges
//! every exchange into the one picture the window and the menu draw from.

use std::{
    collections::{BTreeMap, HashMap},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::currency::Valuation;
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
    /// Dollar exposure at the mark price, negative when short.
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

/// A dollar valuation price: now, and 24 hours before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub last: f64,
    pub open: f64,
}

/// One exchange's holdings, valued.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holdings {
    /// In USD.
    pub total: f64,
    /// What the 24h price moves made of today's holdings, in USD.
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
    /// Stable identity, independent of display name and quote source.
    pub id: String,
    pub asset: String,
    pub amount: f64,
    /// In the valuation currency; none without a pair or exchange value.
    pub price: Option<f64>,
    pub value: Option<f64>,
    /// Some quantities have no valuation; value sums only the priced parts.
    pub partial: bool,
    /// What the source-reference price move made of this amount, in the valuation currency.
    pub change: Option<f64>,
    /// The source-reference price change, in percent.
    pub change_pct: Option<f64>,
    /// A dollar stablecoin: cash, as far as the holdings are concerned.
    pub stable: bool,
    /// Where it sits, largest first.
    pub held: Vec<Holding>,
    pub cost: Option<f64>,
    pub pnl: Option<f64>,
}

/// Part of an asset in an exchange wallet or a registered platform.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holding {
    #[serde(flatten)]
    pub source: HoldingSource,
    pub amount: f64,
}

/// Exchange locations keep their existing wire fields. Manual locations
/// carry the user-provided platform and the editable record's ID.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum HoldingSource {
    Exchange { exchange: ProviderId, wallet: Wallet },
    Manual { manual_id: String, platform: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionValue {
    pub exchange: ProviderId,
    #[serde(flatten)]
    pub position: Position,
    /// Unrealized PnL in the valuation currency, when `pnl_asset` has a price.
    pub pnl_value: Option<f64>,
    /// Signed exposure in the portfolio's valuation currency.
    pub exposure_value: f64,
    /// What the contract's 24h move made of the position, in the valuation currency.
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

/// Values `balances` and `positions` at `prices` (by asset; USD and USDT are valued 1:1).
pub fn value(
    exchange: ProviderId,
    balances: &[Balance],
    positions: &[Position],
    prices: &HashMap<String, Price>,
) -> Holdings {
    let price_of = |asset: &str| match asset {
        "USD" | "USDT" => Some(Price { last: 1.0, open: 1.0 }),
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
                .map(|b| Holding {
                    source: HoldingSource::Exchange { exchange, wallet: b.wallet },
                    amount: b.amount,
                })
                .collect();
            where_held.sort_by(|a, b| b.amount.total_cmp(&a.amount));
            let cost = if amount != 0. {
                held.iter()
                    .map(|b| b.cost.map(|cost| cost * (b.amount / amount)))
                    .sum::<Option<f64>>()
            } else {
                None
            };
            let pnl = held.iter().map(|b| b.pnl).sum::<Option<f64>>();
            Asset {
                id: format!("crypto:{asset}"),
                asset: asset.to_owned(),
                amount,
                price: price
                    .map(|p| p.last)
                    .or_else(|| value.and_then(|v| (amount != 0.).then_some(v / amount))),
                value,
                partial: price.is_none() && held.iter().any(|balance| balance.usd.is_none()),
                change: moved,
                change_pct: price.and_then(|p| pct(p.last, p.open)),
                stable: is_stable(asset),
                held: where_held,
                cost,
                pnl,
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
                pnl_value: price_of(&position.pnl_asset).map(|p| position.pnl * p.last),
                exposure_value: position.exposure,
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

/// Synced exchange equity and registered quantities, valued as one portfolio.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Portfolio {
    pub valuation: Valuation,
    pub accounts: Vec<AccountView>,
    /// Sum of valued holdings in `valuation.currency`; none before any valuation.
    pub total: Option<f64>,
    pub change: Option<f64>,
    /// Merged by asset across the accounts, most valuable first.
    pub assets: Vec<Asset>,
    /// Largest unrealized PnL (either way) first.
    pub positions: Vec<PositionValue>,
    /// Quantities/costs in instrument units, plus their live valuation.
    pub manual_holdings: Vec<crate::manual::Value>,
    pub partial: bool,
    pub manual_error: Option<String>,
}

impl Portfolio {
    pub fn of(
        accounts: &BTreeMap<ProviderId, Account>,
        registered: impl IntoIterator<Item = (Asset, crate::manual::Value)>,
        manual_error: Option<&str>,
    ) -> Self {
        let totals = Totals::of(accounts);
        let mut total = totals.map(|totals| totals.total);
        let mut change = totals.map(|totals| totals.change);
        let mut partial = manual_error.is_some()
            || accounts.values().any(|account| {
                account
                    .holdings
                    .as_ref()
                    .is_none_or(|holdings| holdings.assets.iter().any(|asset| asset.partial))
            });
        let mut manual_assets = Vec::new();
        let mut manual_holdings = Vec::new();
        for (asset, value) in registered {
            partial |= asset.partial;
            total = add(total, asset.value);
            change = add(change, asset.change);
            manual_assets.push(asset);
            manual_holdings.push(value);
        }
        let valued = accounts.values().filter_map(|a| a.holdings.as_ref());
        let mut positions: Vec<PositionValue> =
            valued.clone().flat_map(|h| h.positions.iter().cloned()).collect();
        positions.sort_by(|a, b| {
            let size = |p: &PositionValue| p.pnl_value.unwrap_or(0.0).abs();
            size(b).total_cmp(&size(a))
        });
        Self {
            valuation: Valuation::default(),
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
            total,
            change,
            assets: merge(valued.flat_map(|h| h.assets.iter()).chain(manual_assets.iter())),
            positions,
            manual_holdings,
            partial,
            manual_error: manual_error.map(str::to_owned),
        }
    }

    /// Converts only monetary valuation fields. Token amounts, contract
    /// prices and their units, and percentage changes retain their meaning.
    pub fn converted(mut self, valuation: Valuation) -> Self {
        if valuation.currency == crate::currency::Currency::Usd {
            self.valuation = valuation;
            return self;
        }
        let convert = |v: Option<f64>| v.map(|v| v * valuation.rate);
        self.total = convert(self.total);
        self.change = convert(self.change);
        for account in &mut self.accounts {
            account.total = convert(account.total);
            account.change = convert(account.change);
            for wallet in &mut account.wallets {
                wallet.value *= valuation.rate;
            }
        }
        for asset in &mut self.assets {
            asset.price = convert(asset.price);
            asset.value = convert(asset.value);
            asset.change = convert(asset.change);
            asset.cost = convert(asset.cost);
            asset.pnl = convert(asset.pnl);
        }
        for manual in &mut self.manual_holdings {
            manual.value = convert(manual.value);
            manual.change = convert(manual.change);
            manual.pnl = convert(manual.pnl);
        }
        for position in &mut self.positions {
            position.pnl_value = convert(position.pnl_value);
            position.exposure_value *= valuation.rate;
            position.change = convert(position.change);
        }
        self.valuation = valuation;
        self
    }
}

/// The same asset on several exchanges becomes one line: amounts, values
/// and moves add up, and `held` lists every wallet of every exchange.
fn merge<'a>(assets: impl Iterator<Item = &'a Asset>) -> Vec<Asset> {
    let mut by_asset: BTreeMap<&str, Asset> = BTreeMap::new();
    for asset in assets {
        match by_asset.get_mut(asset.id.as_str()) {
            None => {
                by_asset.insert(&asset.id, asset.clone());
            }
            Some(merged) => {
                let amount = merged.amount + asset.amount;
                merged.cost = merged.cost.zip(asset.cost).and_then(|(a, b)| {
                    (amount != 0.)
                        .then(|| a * (merged.amount / amount) + b * (asset.amount / amount))
                });
                merged.amount += asset.amount;
                merged.value = add(merged.value, asset.value);
                merged.change = merged.change.zip(asset.change).map(|(a, b)| a + b);
                merged.pnl = merged.pnl.zip(asset.pnl).map(|(a, b)| a + b);
                merged.partial |= asset.partial;
                merged.price = if merged.partial {
                    None
                } else {
                    merged.value.and_then(|value| (amount != 0.).then_some(value / amount))
                };
                merged.change_pct = merged
                    .value
                    .zip(merged.change)
                    .and_then(|(value, change)| pct(value, value - change));
                merged.held.extend(asset.held.iter().cloned());
            }
        }
    }
    let mut merged: Vec<Asset> = by_asset.into_values().collect();
    for asset in &mut merged {
        asset.held.sort_by(|a, b| b.amount.total_cmp(&a.amount));
        // Priced on one exchange, valued by the other: one price for the whole.
        if asset.price.is_none() && !asset.partial && asset.amount != 0. {
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

/// The sum over the accounts refreshed at least once, in USD.
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
        assert!(matches!(
            holdings.assets[0].held[0].source,
            HoldingSource::Exchange { wallet: Wallet::Spot, .. }
        ));
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
        assert_eq!(holdings.positions[0].pnl_value, Some(-20.0));
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
        let portfolio = Portfolio::of(&accounts, std::iter::empty(), None);
        assert_eq!((portfolio.total, portfolio.change), (Some(160.0), Some(30.0)));
        assert!(portfolio.partial, "Bybit has not supplied a valuation yet");
        let btc = &portfolio.assets[0];
        assert_eq!((btc.asset.as_str(), btc.amount, btc.value), ("BTC", 1.5, Some(150.0)));
        assert_eq!(btc.change, Some(30.0));
        let held: Vec<(ProviderId, f64)> = btc
            .held
            .iter()
            .map(|h| match h.source {
                HoldingSource::Exchange { exchange, .. } => (exchange, h.amount),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(held, [(ProviderId::Binance, 1.0), (ProviderId::Okx, 0.5)]);
        assert_eq!(portfolio.accounts.len(), 3);
        // Accounts come in ProviderId order: Binance, Bybit (never read), OKX.
        assert_eq!(portfolio.accounts[1].total, None);
        let pct = Totals::of(&accounts).and_then(Totals::change_pct).unwrap();
        assert!((pct - 30.0 / 130.0 * 100.0).abs() < 1e-9);
        assert_eq!(Portfolio::of(&BTreeMap::new(), std::iter::empty(), None).total, None);
    }

    // contract: the app values USDT and USD 1:1. Display conversion scales
    // monetary values while keeping contract prices and quantities native.
    #[test]
    fn portfolio_values_have_one_currency_and_keep_native_contract_units() {
        let balances = [
            Balance::new(Wallet::Spot, "USDT", 100.),
            Balance::new(Wallet::Spot, "BTC", 1.),
            Balance {
                usd: Some(10.),
                cost: Some(5.),
                pnl: Some(3.),
                ..Balance::new(Wallet::Spot, "EUR", 2.)
            },
        ];
        let position = Position {
            symbol: "BTCUSD_PERP".into(),
            kind: PositionKind::CoinPerpetual,
            long: true,
            size: 1.,
            size_unit: Some("BTC".into()),
            entry: 80.,
            mark: 100.,
            liquidation: Some(50.),
            leverage: Some(5.),
            isolated: false,
            pnl: 0.1,
            pnl_asset: "BTC".into(),
            exposure: 50.,
            day: Some(Price { last: 100., open: 80. }),
        };
        let holdings =
            value(ProviderId::Binance, &balances, std::slice::from_ref(&position), &prices());
        let mut account = Account::new(ProviderId::Binance);
        account.holdings = Some(holdings);
        let portfolio =
            Portfolio::of(&[(ProviderId::Binance, account)].into(), std::iter::empty(), None);
        assert_eq!(portfolio.total, Some(210.));
        assert_eq!(portfolio.valuation.currency, crate::currency::Currency::Usd);
        let valuation = Valuation {
            currency: crate::currency::Currency::Cny,
            rate: 7.,
            date: Some("2026-11-06".into()),
            error: None,
        };
        let cny = portfolio.clone().converted(valuation.clone());
        // USDT 100 + BTC 100 + EUR's USD 10 = USD 210.
        assert_eq!(cny.total, Some(1470.));
        assert_eq!(cny.change, Some(210.)); // BTC 20 + USD exposure 50 * 20%.
        assert_eq!(cny.accounts[0].wallets[0].value, 1470.);
        let eur = cny.assets.iter().find(|a| a.asset == "EUR").unwrap();
        assert_eq!(
            (eur.amount, eur.price, eur.value, eur.cost, eur.pnl),
            (2., Some(35.), Some(70.), Some(35.), Some(21.))
        );
        let btc = cny.assets.iter().find(|a| a.asset == "BTC").unwrap();
        assert_eq!((btc.amount, btc.price, btc.change_pct), (1., Some(700.), Some(25.)));
        assert_eq!(cny.positions[0].position, position);
        assert_eq!(cny.positions[0].pnl_value, Some(70.));
        assert_eq!(cny.positions[0].exposure_value, 350.);
        assert_eq!(cny.positions[0].change, Some(70.));
        let json = serde_json::to_value(&cny).unwrap();
        assert_eq!(json["valuation"]["currency"], "CNY");
        assert_eq!(json["positions"][0]["pnlValue"], 70.);

        let jpy = portfolio.clone().converted(Valuation {
            currency: crate::currency::Currency::Jpy,
            rate: 150.5,
            ..valuation.clone()
        });
        assert_eq!(jpy.total, Some(31_605.));
        assert_eq!(jpy.positions[0].pnl_value, Some(1_505.));
        assert_eq!(jpy.positions[0].exposure_value, 7_525.);
        assert_eq!(jpy.positions[0].position, position);

        let available = portfolio.clone().converted(
            crate::currency::ExchangeRates::default().valuation(crate::currency::Currency::Jpy),
        );
        assert_eq!(available, portfolio);
    }
}
