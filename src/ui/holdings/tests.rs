//! Layout-level checks: what the strings say before they are placed.

use crate::{
    i18n::{self, Locale},
    market::ProviderId,
    portfolio::{Asset, Holding, Wallet},
};

use super::{assets, hero, tiles};

fn asset(name: &str, held: &[(ProviderId, Wallet, f64)]) -> Asset {
    Asset {
        asset: name.to_owned(),
        amount: held.iter().map(|(_, _, amount)| *amount).sum(),
        price: Some(1.0),
        value: Some(1.0),
        change: None,
        change_pct: None,
        stable: false,
        held: held
            .iter()
            .map(|(exchange, wallet, amount)| Holding {
                exchange: *exchange,
                wallet: *wallet,
                amount: *amount,
            })
            .collect(),
        cost: None,
        pnl: None,
    }
}

#[test]
fn where_held_names_the_wallet_simply_when_there_is_one() {
    i18n::set(Locale::En);
    let one = asset("BTC", &[(ProviderId::Binance, Wallet::Spot, 1.0)]);
    assert_eq!(assets::where_held(&one, false), "Spot");
    assert_eq!(assets::where_held(&one, true), "Binance Spot");

    let split = asset(
        "BTC",
        &[(ProviderId::Binance, Wallet::Spot, 1.0), (ProviderId::Okx, Wallet::Trading, 0.5)],
    );
    assert_eq!(assets::where_held(&split, false), "Spot 1 · Trading 0.5");
    assert_eq!(assets::where_held(&split, true), "Binance Spot 1 · OKX Trading 0.5");
}

#[test]
fn the_day_is_a_percent_of_yesterdays_worth() {
    assert!((hero::day_pct(Some(110.0), Some(10.0)).unwrap() - 10.0).abs() < 1e-9);
    assert_eq!(hero::day_pct(Some(10.0), Some(10.0)), None);
    assert_eq!(hero::day_pct(None, Some(1.0)), None);
    assert_eq!(hero::day_pct(Some(1.0), None), None);
}

#[test]
fn tile_shares_round_to_whole_percent() {
    assert_eq!(tiles::share(26.0, Some(100.0)), " · 26%");
    assert_eq!(tiles::share(0.5, Some(0.0)), "");
    assert_eq!(tiles::share(1.0, None), "");
}
