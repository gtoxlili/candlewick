//! What the holdings window says about a portfolio: how the money splits,
//! what moved it, and the figures in the tiles. Pure functions over the
//! app's portfolio, so the window only lays them out.

use crate::portfolio::{Asset, Portfolio, PositionValue};

/// Assets below this USD value are folded away, independent of display currency.
const SMALL: f64 = 1.0;

/// The largest holdings get a series color each; the rest fold together,
/// cash last.
const SERIES: usize = 5;

/// Which stretch of the allocation bar a value is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Series {
    One,
    Two,
    Three,
    Four,
    Five,
    Rest,
    Cash,
}

impl Series {
    /// The series of the `rank`th non-cash holding.
    fn ranked(rank: usize) -> Self {
        match rank {
            0 => Self::One,
            1 => Self::Two,
            2 => Self::Three,
            3 => Self::Four,
            4 => Self::Five,
            _ => Self::Rest,
        }
    }
}

/// What a share of the allocation names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Name {
    Asset(String),
    /// The non-cash assets past the fifth, summed.
    Others,
    /// The stablecoins, summed.
    Stablecoins,
}

/// One stretch of the allocation bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Share {
    pub name: Name,
    pub value: f64,
    /// Of the total, 0-100.
    pub pct: f64,
    pub series: Series,
}

/// How the total splits: the largest holdings, the others together, then
/// stablecoins.
pub fn allocation(assets: &[Asset], total: f64) -> Vec<Share> {
    if total.is_nan() || total <= 0.0 {
        return Vec::new();
    }
    let mut shares = Vec::with_capacity(SERIES + 2);
    let mut rank = 0;
    let mut others = 0.0;
    let mut cash = 0.0;
    for asset in assets {
        let Some(value) = asset.value.filter(|value| *value > 0.0) else {
            continue;
        };
        if asset.stable {
            cash += value;
        } else if rank < SERIES {
            shares.push(Share {
                name: Name::Asset(asset.asset.clone()),
                value,
                pct: value / total * 100.0,
                series: Series::ranked(rank),
            });
            rank += 1;
        } else {
            others += value;
        }
    }
    if others > 0.0 {
        shares.push(Share {
            name: Name::Others,
            value: others,
            pct: others / total * 100.0,
            series: Series::Rest,
        });
    }
    if cash > 0.0 {
        shares.push(Share {
            name: Name::Stablecoins,
            value: cash,
            pct: cash / total * 100.0,
            series: Series::Cash,
        });
    }
    shares
}

/// The series each asset is drawn with, in the order `assets` has them.
pub fn series(assets: &[Asset]) -> Vec<Series> {
    let mut rank = 0;
    assets
        .iter()
        .map(|asset| {
            if asset.stable {
                return Series::Cash;
            }
            if asset.value.is_some_and(|value| value > 0.0) {
                let series = Series::ranked(rank);
                rank += 1;
                series
            } else {
                Series::Rest
            }
        })
        .collect()
}

/// The figures in the tiles under the list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Figures {
    /// Value in assets that move with the market.
    pub risk: f64,
    /// Value in stablecoins.
    pub cash: f64,
    /// Valued unrealized PnL over open positions; none without any positions.
    pub positions_pnl: Option<f64>,
    /// What the 24h moves made of the positions, in the valuation currency.
    pub positions_change: f64,
}

pub fn figures(portfolio: &Portfolio) -> Figures {
    let mut risk = 0.0;
    let mut cash = 0.0;
    for asset in &portfolio.assets {
        let Some(value) = asset.value else {
            continue;
        };
        if asset.stable {
            cash += value;
        } else {
            risk += value;
        }
    }
    let positions_pnl = (!portfolio.positions.is_empty())
        .then(|| portfolio.positions.iter().filter_map(|position| position.pnl_value).sum());
    let positions_change =
        portfolio.positions.iter().map(|position| position.change.unwrap_or(0.0)).sum();
    Figures { risk, cash, positions_pnl, positions_change }
}

/// One part of the line under the total; the window words and joins them.
#[derive(Debug, Clone, PartialEq)]
pub enum InsightPart {
    /// An asset and its move today, in the valuation currency.
    Mover { asset: String, change: f64 },
    /// What the 24h moves made of the open positions, in the valuation currency.
    Positions { change: f64 },
    /// The stablecoins' share of the total, in whole percent.
    StableShare { pct: u32 },
}

/// What moved the money today and how much of it is cash: the two largest
/// movers, the positions, then the stablecoins.
pub fn insight(portfolio: &Portfolio) -> Option<Vec<InsightPart>> {
    let total = portfolio.total?;
    if total.is_nan() || total <= 0.0 {
        return None;
    }
    let mut movers: [Option<(String, f64)>; 2] = [None, None];
    let mut cash = 0.0;
    for asset in &portfolio.assets {
        if asset.stable {
            cash += asset.value.unwrap_or(0.0);
            continue;
        }
        let Some(change) = asset.change else {
            continue;
        };
        if change.abs() < 0.005 {
            continue;
        }
        let slot = if movers[0].as_ref().is_none_or(|(_, first)| change.abs() > first.abs()) {
            0
        } else if movers[1].as_ref().is_none_or(|(_, second)| change.abs() > second.abs()) {
            1
        } else {
            continue;
        };
        if slot == 0 {
            movers[1] = movers[0].take();
        }
        movers[slot] = Some((asset.asset.clone(), change));
    }
    let positions_change: f64 =
        portfolio.positions.iter().map(|position| position.change.unwrap_or(0.0)).sum();
    let mut parts = Vec::with_capacity(4);
    parts.extend(
        movers.into_iter().flatten().map(|(asset, change)| InsightPart::Mover { asset, change }),
    );
    if !portfolio.positions.is_empty() && positions_change.abs() >= 0.005 {
        parts.push(InsightPart::Positions { change: positions_change });
    }
    parts.push(InsightPart::StableShare { pct: (cash / total * 100.0).round() as u32 });
    Some(parts)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Value,
    Change,
}

/// The assets the list shows, in `sort` order; small ones only when asked
/// for, and always listed apart.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed<'a> {
    pub shown: Vec<&'a Asset>,
    pub small: Vec<&'a Asset>,
}

pub fn listed(assets: &[Asset], sort: Sort, show_small: bool, rate: f64) -> Listed<'_> {
    let mut small = Vec::new();
    let mut shown = Vec::with_capacity(assets.len());
    for asset in assets {
        let is_small = asset.value.unwrap_or(0.) < SMALL * rate;
        if is_small {
            small.push(asset);
        }
        if show_small || !is_small {
            shown.push(asset);
        }
    }
    if sort == Sort::Change {
        shown.sort_by(|a, b| {
            let move_of = |asset: &Asset| asset.change.unwrap_or(0.0).abs();
            move_of(b).total_cmp(&move_of(a))
        });
    }
    Listed { shown, small }
}

/// How far the mark is from liquidation, as a share of the mark; none
/// without a liquidation price.
pub fn liquidation_distance(position: &PositionValue) -> Option<f64> {
    let position = &position.position;
    let liquidation = position.liquidation?;
    (position.mark > 0.0).then(|| (position.mark - liquidation).abs() / position.mark)
}

/// Return on the margin behind a position, in percent, when the leverage is
/// known.
pub fn return_on_margin(position: &PositionValue) -> Option<f64> {
    let pnl = position.pnl_value?;
    let leverage = position.position.leverage?;
    if leverage <= 0.0 || leverage.is_nan() {
        return None;
    }
    let margin = position.exposure_value.abs() / leverage;
    (margin > 0.0).then(|| pnl / margin * 100.0)
}

#[cfg(test)]
mod tests {
    use crate::{
        market::ProviderId,
        portfolio::{Asset, Holding, Position, PositionKind, PositionValue, Wallet},
    };

    use super::*;

    fn asset(name: &str, value: Option<f64>, change: Option<f64>, stable: bool) -> Asset {
        Asset {
            asset: name.to_owned(),
            amount: 1.0,
            price: value,
            value,
            change,
            change_pct: change.map(|_| 1.0),
            stable,
            held: vec![Holding {
                exchange: ProviderId::Binance,
                wallet: Wallet::Spot,
                amount: 1.0,
            }],
            cost: None,
            pnl: None,
        }
    }

    fn position(symbol: &str, pnl: f64, pnl_asset: &str, pnl_value: Option<f64>) -> PositionValue {
        PositionValue {
            exchange: ProviderId::Binance,
            position: Position {
                symbol: symbol.to_owned(),
                kind: PositionKind::UsdtPerpetual,
                long: true,
                size: 1.0,
                size_unit: Some("BTC".to_owned()),
                entry: 1.0,
                mark: 0.0,
                liquidation: None,
                leverage: None,
                isolated: false,
                pnl,
                pnl_asset: pnl_asset.to_owned(),
                exposure: 0.0,
                day: None,
            },
            pnl_value,
            exposure_value: 0.0,
            change: None,
        }
    }

    fn traded(
        mut value: PositionValue,
        exposure: f64,
        leverage: Option<f64>,
        change: Option<f64>,
        mark: f64,
        liquidation: Option<f64>,
    ) -> PositionValue {
        value.position.exposure = exposure;
        value.exposure_value = exposure;
        value.position.leverage = leverage;
        value.position.mark = mark;
        value.position.liquidation = liquidation;
        value.change = change;
        value
    }

    fn assets() -> Vec<Asset> {
        vec![
            asset("BTC", Some(60000.0), Some(1200.5), false),
            asset("ETH", Some(20000.0), Some(-300.25), false),
            asset("USDT", Some(15000.0), Some(0.0), true),
            asset("SOL", Some(5000.0), Some(2.0), false),
            asset("USDC", Some(5000.0), Some(0.0), true),
            asset("DOGE", Some(2000.0), Some(-1.5), false),
            asset("LINK", Some(1000.0), Some(0.004), false),
            asset("ADA", Some(500.0), Some(50.0), false),
            asset("DUST", Some(0.5), Some(-0.01), false),
            asset("NOPRICE", None, None, false),
        ]
    }

    fn positions() -> Vec<PositionValue> {
        vec![
            traded(
                position("BTCUSDT", 500.0, "USDT", Some(500.0)),
                40000.0,
                Some(10.0),
                Some(120.5),
                60000.0,
                Some(50000.0),
            ),
            traded(
                position("ETHUSDT", -200.0, "USDT", Some(-200.0)),
                -10000.0,
                Some(5.0),
                Some(-80.0),
                2000.0,
                None,
            ),
            traded(
                position("BTCUSD_PERP", 0.1, "BTC", None),
                1000.0,
                Some(2.0),
                Some(0.0),
                60000.0,
                Some(55000.0),
            ),
        ]
    }

    fn portfolio() -> Portfolio {
        Portfolio {
            valuation: Default::default(),
            accounts: Vec::new(),
            total: Some(108500.5),
            change: Some(-500.25),
            assets: assets(),
            positions: positions(),
        }
    }

    #[test]
    fn allocation_splits_the_total() {
        let assets = assets();
        let shares = allocation(&assets, 108500.5);
        let names: Vec<Name> = shares.iter().map(|share| share.name.clone()).collect();
        assert_eq!(
            names,
            [
                Name::Asset("BTC".to_owned()),
                Name::Asset("ETH".to_owned()),
                Name::Asset("SOL".to_owned()),
                Name::Asset("DOGE".to_owned()),
                Name::Asset("LINK".to_owned()),
                Name::Others,
                Name::Stablecoins,
            ]
        );
        assert_eq!(
            shares.iter().map(|share| share.value).collect::<Vec<_>>(),
            [60000.0, 20000.0, 5000.0, 2000.0, 1000.0, 500.5, 20000.0]
        );
        let pct: Vec<f64> = shares.iter().map(|share| share.pct).collect();
        for (found, expected) in pct.iter().zip([
            55.29928433509524,
            18.433094778365078,
            4.6082736945912695,
            1.8433094778365078,
            0.9216547389182539,
            0.461288196828586,
            18.433094778365078,
        ]) {
            assert!((found - expected).abs() < 1e-9, "{found} != {expected}");
        }
        assert_eq!(
            shares.iter().map(|share| share.series).collect::<Vec<_>>(),
            [
                Series::One,
                Series::Two,
                Series::Three,
                Series::Four,
                Series::Five,
                Series::Rest,
                Series::Cash,
            ]
        );
        assert!(allocation(&assets, 0.0).is_empty());
    }

    #[test]
    fn series_follow_the_allocation_order() {
        assert_eq!(
            series(&assets()),
            [
                Series::One,
                Series::Two,
                Series::Cash,
                Series::Three,
                Series::Cash,
                Series::Four,
                Series::Five,
                Series::Rest,
                Series::Rest,
                Series::Rest,
            ]
        );
    }

    #[test]
    fn figures_add_up() {
        let sums = figures(&portfolio());
        assert_eq!(sums.risk, 88500.5);
        assert_eq!(sums.cash, 20000.0);
        assert_eq!(sums.positions_pnl, Some(300.));
        assert_eq!(sums.positions_change, 40.5);
        let empty = Portfolio { positions: Vec::new(), ..portfolio() };
        assert_eq!(figures(&empty).positions_pnl, None);
    }

    #[test]
    fn insight_names_the_two_largest_movers() {
        let portfolio = portfolio();
        assert_eq!(
            insight(&portfolio),
            Some(vec![
                InsightPart::Mover { asset: "BTC".to_owned(), change: 1200.5 },
                InsightPart::Mover { asset: "ETH".to_owned(), change: -300.25 },
                InsightPart::Positions { change: 40.5 },
                InsightPart::StableShare { pct: 18 },
            ])
        );
        let unknown = Portfolio { total: None, ..portfolio };
        assert_eq!(insight(&unknown), None);
    }

    #[test]
    fn listed_folds_small_assets_away() {
        let assets = assets();
        let by_value = listed(&assets, Sort::Value, false, 1.);
        assert_eq!(
            by_value.shown.iter().map(|asset| asset.asset.as_str()).collect::<Vec<_>>(),
            ["BTC", "ETH", "USDT", "SOL", "USDC", "DOGE", "LINK", "ADA"]
        );
        assert_eq!(
            by_value.small.iter().map(|asset| asset.asset.as_str()).collect::<Vec<_>>(),
            ["DUST", "NOPRICE"]
        );
        let all = listed(&assets, Sort::Value, true, 1.);
        assert_eq!(all.shown.len(), 10);
        let by_change = listed(&assets, Sort::Change, false, 1.);
        assert_eq!(
            by_change.shown.iter().map(|asset| asset.asset.as_str()).collect::<Vec<_>>(),
            ["BTC", "ETH", "ADA", "SOL", "DOGE", "LINK", "USDT", "USDC"]
        );
        // contract: changing the display currency does not change which
        // assets are folded away.
        for (currency, rate) in
            [(crate::currency::Currency::Cny, 7.), (crate::currency::Currency::Jpy, 150.5)]
        {
            let converted = Portfolio { assets: assets.clone(), ..Default::default() }
                .converted(crate::currency::Valuation { currency, rate, ..Default::default() });
            let listed = listed(&converted.assets, Sort::Value, false, rate);
            assert_eq!(
                listed.shown.iter().map(|asset| &asset.asset).collect::<Vec<_>>(),
                by_value.shown.iter().map(|asset| &asset.asset).collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn positions_report_their_distance_and_return() {
        let positions = positions();
        assert_eq!(liquidation_distance(&positions[0]), Some(1.0 / 6.0));
        assert_eq!(liquidation_distance(&positions[1]), None);
        assert_eq!(liquidation_distance(&positions[2]), Some(1.0 / 12.0));
        assert_eq!(return_on_margin(&positions[0]), Some(12.5));
        assert_eq!(return_on_margin(&positions[1]), Some(-10.0));
        assert_eq!(return_on_margin(&positions[2]), None);
        let converted =
            Portfolio { positions, ..Default::default() }.converted(crate::currency::Valuation {
                currency: crate::currency::Currency::Cny,
                rate: 7.,
                ..Default::default()
            });
        assert_eq!(return_on_margin(&converted.positions[0]), Some(12.5));
        assert_eq!(return_on_margin(&converted.positions[1]), Some(-10.));
        assert_eq!(liquidation_distance(&converted.positions[0]), Some(1. / 6.));
    }
}
