//! A realistic portfolio for rendering checks: two exchanges, priced and
//! unpriced assets, costs, dust under 1 USDT, and positions near and far
//! from liquidation.

use std::collections::BTreeMap;

use crate::{
    market::ProviderId,
    portfolio::{self, Account, Balance, Position, Price, Wallet},
};

fn prices() -> std::collections::HashMap<String, Price> {
    [
        ("BTC", 84049.95, 82100.0),
        ("ETH", 3412.1, 3450.0),
        ("SOL", 182.33, 178.2),
        ("DOGE", 0.26421, 0.27100),
        ("LINK", 12.534, 12.401),
        ("PEPE", 0.0000121, 0.0000118),
    ]
    .map(|(asset, last, open)| (asset.to_owned(), Price { last, open }))
    .into()
}

/// Every exchange with a key, as the app would keep them after a good read.
pub fn accounts() -> BTreeMap<ProviderId, Account> {
    let prices = prices();
    let binance_balances = [
        Balance::new(Wallet::Spot, "BTC", 0.841),
        Balance::new(Wallet::Spot, "ETH", 4.2),
        Balance::new(Wallet::Spot, "SOL", 18.4),
        Balance::new(Wallet::Funding, "USDT", 12500.0),
        Balance::new(Wallet::Earn, "USDT", 8000.0),
        Balance::new(Wallet::Spot, "DOGE", 42000.0),
        Balance::new(Wallet::Spot, "PEPE", 21000000.0),
        // No USDT pair and the exchange values it itself.
        Balance { usd: Some(7.0), ..Balance::new(Wallet::Funding, "EUR", 6.0) },
        Balance::new(Wallet::Spot, "NOPAIR", 3.0),
    ];
    let binance_positions = [
        Position {
            symbol: "BTCUSDT".to_owned(),
            kind: portfolio::PositionKind::UsdtPerpetual,
            long: true,
            size: 0.25,
            size_unit: Some("BTC".to_owned()),
            entry: 79800.0,
            mark: 84049.95,
            liquidation: Some(71500.0),
            leverage: Some(10.0),
            isolated: false,
            pnl: 1062.49,
            pnl_asset: "USDT".to_owned(),
            exposure: 21012.49,
            day: Some(Price { last: 84049.95, open: 82100.0 }),
        },
        Position {
            symbol: "ETHUSDT".to_owned(),
            kind: portfolio::PositionKind::UsdtPerpetual,
            long: false,
            size: 3.0,
            size_unit: Some("ETH".to_owned()),
            entry: 3300.0,
            mark: 3412.1,
            liquidation: Some(3695.0),
            leverage: Some(5.0),
            isolated: true,
            pnl: -336.3,
            pnl_asset: "USDT".to_owned(),
            exposure: -10236.3,
            day: Some(Price { last: 3412.1, open: 3450.0 }),
        },
    ];
    let okx_balances = [
        Balance {
            cost: Some(79000.0),
            pnl: Some(4200.0),
            ..Balance::new(Wallet::Trading, "BTC", 0.062)
        },
        Balance::new(Wallet::Trading, "LINK", 150.0),
        Balance::new(Wallet::Funding, "USDC", 4300.0),
    ];
    let okx_positions = [Position {
        symbol: "BTC-USD-SWAP".to_owned(),
        kind: portfolio::PositionKind::CoinPerpetual,
        long: true,
        size: 100.0,
        size_unit: Some("USD".to_owned()),
        entry: 81000.0,
        mark: 84049.95,
        liquidation: Some(83600.0),
        leverage: Some(20.0),
        isolated: true,
        pnl: 0.0037,
        pnl_asset: "BTC".to_owned(),
        exposure: 8405.0,
        day: Some(Price { last: 84049.95, open: 82100.0 }),
    }];

    let mut accounts = BTreeMap::new();
    for (exchange, balances, positions) in [
        (ProviderId::Binance, &binance_balances[..], &binance_positions[..]),
        (ProviderId::Okx, &okx_balances[..], &okx_positions[..]),
    ] {
        let holdings = portfolio::value(exchange, balances, positions, &prices);
        accounts.insert(
            exchange,
            Account {
                exchange,
                holdings: Some(holdings),
                updated: Some(portfolio::now_ms() - 4000.0),
                error: None,
                checked: Some(portfolio::now_ms() - 3000.0),
            },
        );
    }
    accounts
}
