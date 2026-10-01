//! Holdings on the crypto exchanges the user gave an API key: what
//! each wallet holds and the open derivatives positions, valued in USDT at
//! the exchange's own prices. `market::crypto::account` fetches them; this is
//! what they are worth, as the menu bar's total and the holdings window show it.

use std::collections::{BTreeMap, HashMap};

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

impl Wallet {
    pub fn label(self) -> &'static str {
        match self {
            Self::Spot => "现货",
            Self::Trading => "交易账户",
            Self::Funding => "资金",
            Self::Earn => "理财",
            Self::UsdFutures => "U 本位合约",
            Self::CoinFutures => "币本位合约",
        }
    }
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
    /// `U 本位永续`, `币本位交割`, `期权`…
    pub kind: &'static str,
    pub long: bool,
    /// How much, in `size_unit`.
    pub size: f64,
    /// `BTC`, or `张` for contracts.
    pub size_unit: String,
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
    #[serde(skip)]
    pub exposure: f64,
    /// The contract's own last price and the one 24 hours before, which
    /// tell what its 24h move made of the position.
    #[serde(skip)]
    pub day: Option<Price>,
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
    pub label: &'static str,
    pub value: f64,
}

/// An asset across an exchange's wallets.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub asset: String,
    pub amount: f64,
    /// In USDT; none without a USDT pair or the exchange's own value.
    pub price: Option<f64>,
    pub value: Option<f64>,
    /// The 24h price change, in percent.
    pub change_pct: Option<f64>,
    /// Where it sits, largest first.
    pub wallets: Vec<WalletAmount>,
    pub cost: Option<f64>,
    pub pnl: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletAmount {
    pub label: &'static str,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionValue {
    #[serde(flatten)]
    pub position: Position,
    /// The unrealized PnL in USDT, when `pnl_asset` has a price.
    pub pnl_usd: Option<f64>,
}

/// Values `balances` and `positions` at `prices` (by asset; USDT is worth 1).
pub fn value(
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
            if let Some(price) = price {
                change += amount * (price.last - price.open);
            }
            let mut where_held: Vec<WalletAmount> = held
                .iter()
                .map(|b| WalletAmount { label: b.wallet.label(), amount: b.amount })
                .collect();
            where_held.sort_by(|a, b| b.amount.total_cmp(&a.amount));
            let costed: Vec<&&Balance> = held.iter().filter(|b| b.cost.is_some()).collect();
            Asset {
                asset: asset.to_owned(),
                amount,
                price: price.map(|p| p.last).or_else(|| value.map(|v| v / amount)),
                value,
                change_pct: price.and_then(|p| pct(p.last, p.open)),
                wallets: where_held,
                cost: costed.first().and_then(|b| b.cost),
                pnl: costed.iter().filter_map(|b| b.pnl).reduce(|a, b| a + b),
            }
        })
        .collect();
    assets.sort_by(|a, b| match (a.value, b.value) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.asset.cmp(&b.asset),
    });

    let positions = positions
        .iter()
        .map(|position| {
            if let Some(Price { last, open }) = position.day
                && last > 0.0
            {
                change += position.exposure * (last - open) / last;
            }
            PositionValue {
                position: position.clone(),
                pnl_usd: price_of(&position.pnl_asset).map(|p| position.pnl * p.last),
            }
        })
        .collect();

    Holdings {
        total: wallets.values().sum(),
        change,
        wallets: wallets
            .into_iter()
            .map(|(wallet, value)| WalletValue { wallet, label: wallet.label(), value })
            .collect(),
        assets,
        positions,
    }
}

/// `(last − open) / open` in percent, when there is an `open`.
pub fn pct(last: f64, open: f64) -> Option<f64> {
    (open > 0.0).then(|| (last - open) / open * 100.0)
}

/// One exchange's part of the holdings window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub exchange: ProviderId,
    pub name: &'static str,
    /// The last good refresh; none before the first.
    pub holdings: Option<Holdings>,
    /// When it was taken, epoch milliseconds.
    pub updated: Option<f64>,
    /// Why the last refresh failed, if it did.
    pub error: Option<String>,
}

/// Every exchange with a key, as the holdings window shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Portfolio {
    pub accounts: Vec<Account>,
    /// Over the accounts refreshed at least once, in USDT; none before.
    pub total: Option<f64>,
    pub change: Option<f64>,
}

impl Portfolio {
    pub fn of(accounts: &BTreeMap<ProviderId, Account>) -> Self {
        let totals = Totals::of(accounts);
        Self {
            accounts: accounts.values().cloned().collect(),
            total: totals.map(|t| t.total),
            change: totals.map(|t| t.change),
        }
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
        let holdings = value(&balances, &[], &prices());
        let assets: Vec<(&str, Option<f64>)> =
            holdings.assets.iter().map(|a| (a.asset.as_str(), a.value)).collect();
        assert_eq!(
            assets,
            [("BTC", Some(150.0)), ("USDT", Some(50.0)), ("EUR", Some(7.0)), ("NOPAIR", None)]
        );
        assert_eq!(holdings.total, 207.0);
        // BTC rose 20 on 1.5 held.
        assert_eq!(holdings.change, 30.0);
        assert_eq!(holdings.assets[0].change_pct, Some(25.0));
        let wallets: Vec<(Wallet, f64)> =
            holdings.wallets.iter().map(|w| (w.wallet, w.value)).collect();
        assert_eq!(wallets, [(Wallet::Spot, 100.0), (Wallet::Funding, 57.0), (Wallet::Earn, 50.0)]);
        assert_eq!(holdings.assets[0].wallets[0].label, "现货");
    }

    #[test]
    fn positions_move_with_their_base() {
        let short = Position {
            symbol: "BTCUSDT".into(),
            kind: "U 本位永续",
            long: false,
            size: 2.0,
            size_unit: "BTC".into(),
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
        let holdings =
            value(&[Balance::new(Wallet::UsdFutures, "USDT", 1000.0)], &[short], &prices());
        // The contract rose from 80 to 100: the short, worth 200 now, lost 40.
        assert_eq!(holdings.change, -40.0);
        assert_eq!(holdings.total, 1000.0);
        assert_eq!(holdings.positions[0].pnl_usd, Some(-20.0));
    }

    #[test]
    fn the_portfolio_sums_what_has_loaded() {
        let account = |exchange, holdings: Option<(f64, f64)>| Account {
            exchange,
            name: "",
            holdings: holdings.map(|(total, change)| Holdings {
                total,
                change,
                ..Holdings::default()
            }),
            updated: None,
            error: None,
        };
        let accounts = [
            (ProviderId::Binance, account(ProviderId::Binance, Some((110.0, 10.0)))),
            (ProviderId::Okx, account(ProviderId::Okx, None)),
        ]
        .into();
        let portfolio = Portfolio::of(&accounts);
        assert_eq!((portfolio.total, portfolio.change), (Some(110.0), Some(10.0)));
        assert_eq!(Totals::of(&accounts).and_then(Totals::change_pct), Some(10.0));
        assert_eq!(Portfolio::of(&BTreeMap::new()).total, None);
    }
}
