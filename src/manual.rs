//! User-maintained quantities, valued from the app's market feeds. A record
//! identifies the actual instrument; it never stores a user-supplied price.

use std::{collections::HashMap, fs, io, path::Path};

use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::{
    currency::ExchangeRates,
    market::{Feeds, InstrumentMeta, ProviderId},
    model::{Instrument, Quote},
    portfolio::{Asset, Holding as Held, HoldingSource, now_ms},
};

const MAX_HOLDINGS: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub platform: String,
    pub instrument: Instrument,
    pub amount: f64,
    /// Average cost per share, fund unit, token or gram, in instrument.quote.
    pub cost: Option<f64>,
}

impl Input {
    fn validated(mut self) -> Result<Self, Error> {
        self.platform = self.platform.trim().to_owned();
        if self.platform.is_empty()
            || self.platform.chars().count() > 80
            || self.platform.chars().any(char::is_control)
        {
            return Err(Error::Invalid(t!("holdings.manual.invalidName").to_owned()));
        }
        let instrument = &mut self.instrument;
        instrument.provider.provider().validate(instrument).map_err(Error::Invalid)?;
        instrument.pinned = false;
        if !self.amount.is_finite()
            || self.amount <= 0.
            || self.cost.is_some_and(|cost| {
                !cost.is_finite() || cost < 0. || !(self.amount * cost).is_finite()
            })
        {
            return Err(Error::Invalid(t!("holdings.manual.invalidNumber").to_owned()));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Holding {
    pub id: String,
    #[serde(flatten)]
    pub input: Input,
    /// Last edit of the quantity/cost, epoch ms; independent of quote time.
    pub updated: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Value {
    pub metadata: InstrumentMeta,
    #[serde(flatten)]
    pub holding: Holding,
    /// Price and reference in instrument.quote; source time is epoch ms.
    pub price: Option<f64>,
    pub reference: Option<f64>,
    pub price_updated: Option<f64>,
    pub received: Option<f64>,
    /// Value, price move and PnL in portfolio.valuation.currency.
    pub value: Option<f64>,
    pub change: Option<f64>,
    pub pnl: Option<f64>,
    pub stale: bool,
    pub error: Option<String>,
}

/// The valuation's entire read dependency, independent of application state.
pub struct Prices<'a> {
    pub quotes: &'a HashMap<String, Quote>,
    pub feeds: &'a Feeds,
    pub rates: &'a ExchangeRates,
}

impl Prices<'_> {
    fn stale(&self, provider: ProviderId) -> bool {
        self.feeds.get(&provider).is_some_and(|feed| feed.stale)
    }

    fn usd_rate(&self, instrument: &Instrument, metadata: &InstrumentMeta) -> Option<f64> {
        self.rates.usd_rate(&instrument.quote).or_else(|| {
            self.quotes
                .get(&metadata.usd_conversion.as_ref()?.id())
                .map(|quote| quote.last)
                .filter(|last| last.is_finite() && *last > 0.)
        })
    }
}

impl Holding {
    pub fn valued(&self, prices: &Prices) -> (Asset, Value) {
        let instrument = &self.input.instrument;
        let id = instrument.id();
        let metadata = instrument.metadata();
        let quote =
            prices.quotes.get(&id).filter(|quote| quote.last.is_finite() && quote.last > 0.);
        let rate = prices.usd_rate(instrument, &metadata);
        let price = quote.map(|quote| quote.last);
        let reference =
            quote.map(|quote| quote.open).filter(|price| price.is_finite() && *price > 0.);
        let finite = |value: f64| value.is_finite().then_some(value);
        let value =
            price.zip(rate).and_then(|(price, rate)| finite(self.input.amount * price * rate));
        let change = price.zip(reference).zip(rate).and_then(|((price, reference), rate)| {
            finite(self.input.amount * (price - reference) * rate)
        });
        let pnl = price
            .zip(self.input.cost)
            .zip(rate)
            .and_then(|((price, cost), rate)| finite(self.input.amount * (price - cost) * rate));
        let error = if quote.is_none() {
            Some(t!("holdings.manual.noQuote").to_owned())
        } else if rate.is_none() {
            Some(t!("holdings.manual.noRate", currency = instrument.quote.clone()))
        } else if value.is_none() {
            Some(t!("holdings.manual.invalidNumber").to_owned())
        } else {
            None
        };
        let asset = Asset {
            id: metadata.asset_id.clone(),
            asset: if instrument.provider.is_exchange() {
                instrument.base.clone()
            } else {
                instrument.short_label()
            },
            amount: self.input.amount,
            price: price.zip(rate).and_then(|(price, rate)| finite(price * rate)),
            value,
            partial: value.is_none(),
            change,
            change_pct: price
                .zip(reference)
                .and_then(|(price, reference)| crate::portfolio::pct(price, reference)),
            stable: instrument.provider.is_exchange()
                && crate::portfolio::is_stable(&instrument.base),
            held: vec![Held {
                source: HoldingSource::Manual {
                    manual_id: self.id.clone(),
                    platform: self.input.platform.clone(),
                },
                amount: self.input.amount,
            }],
            cost: self.input.cost.zip(rate).map(|(cost, rate)| cost * rate),
            pnl,
        };
        let stale = prices.stale(instrument.provider)
            || error.is_some()
            || metadata
                .usd_conversion
                .as_ref()
                .is_some_and(|conversion| prices.stale(conversion.provider));
        let valued = Value {
            metadata,
            holding: self.clone(),
            price,
            reference,
            price_updated: quote.and_then(|quote| quote.time.source),
            received: quote.map(|quote| quote.time.received).filter(|received| *received > 0.),
            value,
            change,
            pnl,
            stale,
            error,
        };
        (asset, valued)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{}", t!("holdings.manual.notFound"))]
    NotFound,
    #[error("{0}")]
    Unavailable(String),
    #[error("{}", t!("error.saveFailed", error = .0))]
    Io(#[from] io::Error),
}

#[derive(Debug, Default)]
pub struct Store {
    pub entries: Vec<Holding>,
    pub error: Option<String>,
    /// Includes quote-currency conversions. Lookups on every feed frame are O(1).
    pub instruments: HashMap<String, Instrument>,
}

impl Store {
    pub fn load(path: &Path) -> Self {
        let read = || -> Result<Vec<Holding>, io::Error> {
            let entries: Vec<Holding> = match fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(error),
            };
            let mut ids = std::collections::HashSet::new();
            if entries.len() > MAX_HOLDINGS {
                return Err(io::Error::other("too many manual holdings"));
            }
            for entry in &entries {
                if entry.id.is_empty()
                    || !ids.insert(&entry.id)
                    || !entry.updated.is_finite()
                    || entry.updated <= 0.
                    || entry.input.clone().validated().as_ref().ok() != Some(&entry.input)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid manual holding",
                    ));
                }
            }
            Ok(entries)
        };
        match read() {
            Ok(entries) => {
                let mut store = Self { entries, ..Self::default() };
                store.index();
                store
            }
            Err(error) => Self {
                error: Some(t!("holdings.manual.loadFailed", error = error)),
                ..Self::default()
            },
        }
    }

    fn index(&mut self) {
        self.instruments.clear();
        for entry in &self.entries {
            let instrument = &entry.input.instrument;
            self.instruments.insert(instrument.id(), instrument.clone());
            if let Some(conversion) = instrument.metadata().usd_conversion {
                self.instruments.insert(conversion.id(), conversion);
            }
        }
    }

    pub fn create(&mut self, path: &Path, input: Input) -> Result<Holding, Error> {
        if self.entries.len() >= MAX_HOLDINGS {
            return Err(Error::Invalid(t!("holdings.manual.limit", max = MAX_HOLDINGS)));
        }
        let input = input.validated()?;
        let mut bytes = [0u8; 16];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| io::Error::other("system random number generator unavailable"))?;
        let id = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let entry = Holding { id, input, updated: now_ms() };
        let mut next = self.entries.clone();
        next.push(entry.clone());
        self.commit(path, next)?;
        Ok(entry)
    }

    pub fn replace(&mut self, path: &Path, id: &str, input: Input) -> Result<Holding, Error> {
        let input = input.validated()?;
        let mut next = self.entries.clone();
        let entry = next.iter_mut().find(|entry| entry.id == id).ok_or(Error::NotFound)?;
        entry.input = input;
        entry.updated = now_ms();
        let entry = entry.clone();
        self.commit(path, next)?;
        Ok(entry)
    }

    pub fn remove(&mut self, path: &Path, id: &str) -> Result<(), Error> {
        let next: Vec<_> = self.entries.iter().filter(|entry| entry.id != id).cloned().collect();
        if next.len() == self.entries.len() {
            return Err(Error::NotFound);
        }
        self.commit(path, next)
    }

    fn commit(&mut self, path: &Path, next: Vec<Holding>) -> Result<(), Error> {
        if let Some(error) = &self.error {
            return Err(Error::Unavailable(error.clone()));
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&next).map_err(io::Error::from)?)?;
        fs::rename(&tmp, path)?;
        self.entries = next;
        self.index();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        currency::{Currency, ExchangeRates, Rates, Snapshot},
        model::{Quote, Settings, Shared},
        portfolio::{self, Account, Balance, Price, Wallet},
    };

    fn input(provider: ProviderId, symbol: &str, base: &str, quote: &str, amount: f64) -> Input {
        Input {
            platform: "Broker / Alipay".into(),
            instrument: Instrument {
                provider,
                symbol: symbol.into(),
                base: base.into(),
                quote: quote.into(),
                name: None,
                decimals: None,
                pinned: false,
                venue: None,
            },
            amount,
            cost: None,
        }
    }

    fn directory() -> tempfile::TempDir {
        tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap()
    }

    // contract: a registered quantity follows its instrument's live price,
    // in that instrument's currency, alongside synced exchange equity.
    #[test]
    fn quantities_follow_market_prices_and_native_quote_units() {
        let dir = directory();
        let (shared, _) = Shared::new(
            Settings::default(),
            dir.path().join("settings.json"),
            Default::default(),
            dir.path().join("credentials.json"),
        );
        let path = dir.path().join("manual-holdings.json");
        let mut model = shared.model();
        model.exchange_rates = ExchangeRates {
            snapshot: Some(Snapshot {
                date: "2026-11-06".into(),
                usd: Rates { cny: 6.25, jpy: 150., hkd: 7.8 },
            }),
            error: None,
        };
        let instruments = [
            (input(ProviderId::Ths, "510300.SH", "510300", "CNY", 10.), 5., 4.5),
            (input(ProviderId::Longbridge, "700.HK", "700", "HKD", 20.), 7.8, 7.02),
            (input(ProviderId::Eastmoney, "118.AU9999", "AU9999", "CNY", 2.), 800., 780.),
            (input(ProviderId::Eastmoney, "150.000216", "000216", "CNY", 100.), 1.25, 1.20),
        ];
        for (mut input, last, open) in instruments {
            input.cost = Some(open);
            let id = input.instrument.id();
            model.manual.create(&path, input).unwrap();
            crate::market::record_quote(
                &mut model,
                id,
                Quote { last, open, session: None, ..Default::default() },
                Some(1_793_952_000_000.),
            );
        }
        let holdings = portfolio::value(
            ProviderId::Binance,
            &[Balance::new(Wallet::Spot, "BTC", 1.)],
            &[],
            &[("BTC".into(), Price { last: 100., open: 80. })].into(),
        );
        model.accounts.insert(
            ProviderId::Binance,
            Account { holdings: Some(holdings), ..Account::new(ProviderId::Binance) },
        );
        let portfolio = model.portfolio();
        assert!((portfolio.total.unwrap() - 404.).abs() < 1e-9);
        assert!((portfolio.change.unwrap() - 30.).abs() < 1e-9);
        assert!(!portfolio.partial);
        assert_eq!(portfolio.manual_holdings[3].price, Some(1.25));
        assert_eq!(portfolio.manual_holdings[3].price_updated, Some(1_793_952_000_000.));
        assert_eq!(portfolio.manual_holdings[0].holding.input.cost, Some(4.5));
        assert!((portfolio.manual_holdings[0].pnl.unwrap() - 0.8).abs() < 1e-9);
        model.quotes.get_mut("ths:510300.SH").unwrap().last = 6.;
        assert!((model.portfolio().total.unwrap() - 405.6).abs() < 1e-9);
        model.settings.currency = Currency::Cny;
        assert!((model.portfolio().total.unwrap() - 2535.).abs() < 1e-9);
        assert_eq!(
            model.portfolio().manual_holdings[0].price,
            Some(6.),
            "raw prices retain the source currency"
        );
        model.quotes.remove("ths:510300.SH");
        let partial = model.portfolio();
        assert!(partial.partial);
        assert!((partial.change.unwrap() - 182.5).abs() < 1e-9);
        assert_eq!(partial.manual_holdings[0].value, None);
        assert!((partial.total.unwrap() - 2475.).abs() < 1e-9);
        model.exchange_rates.snapshot = None;
        assert_eq!(
            model.portfolio().total,
            Some(100.),
            "unconvertible CNY/HKD records are excluded, not treated as USD"
        );
    }

    // contract: another-coin quote uses that coin's USD feed; partial
    // quantities never turn a known subtotal into a fabricated unit price.
    #[test]
    fn crypto_quote_conversion_and_partial_identity_merge() {
        let dir = directory();
        let (shared, _) = Shared::new(
            Settings::default(),
            dir.path().join("settings.json"),
            Default::default(),
            dir.path().join("credentials.json"),
        );
        let mut model = shared.model();
        let path = dir.path().join("manual-holdings.json");
        model.manual.create(&path, input(ProviderId::Bybit, "BTCETH", "BTC", "ETH", 2.)).unwrap();
        assert_eq!(model.symbols()[&ProviderId::Bybit], ["BTCETH", "ETHUSDT"]);
        for (id, last, open) in [("bybit:BTCETH", 30., 29.), ("bybit:ETHUSDT", 10., 9.)] {
            crate::market::record_quote(
                &mut model,
                id.into(),
                Quote { last, open, session: None, ..Default::default() },
                None,
            );
        }
        let holdings = portfolio::value(
            ProviderId::Binance,
            &[Balance::new(Wallet::Spot, "BTC", 1.)],
            &[],
            &[("BTC".into(), Price { last: 400., open: 360. })].into(),
        );
        model.accounts.insert(
            ProviderId::Binance,
            Account { holdings: Some(holdings), ..Account::new(ProviderId::Binance) },
        );
        let all = model.portfolio();
        assert_eq!(all.total, Some(1000.));
        assert_eq!(all.assets.len(), 1);
        assert_eq!(all.assets[0].id, "crypto:BTC");
        assert_eq!(all.assets[0].amount, 3.);
        assert!((all.assets[0].price.unwrap() - 1000. / 3.).abs() < 1e-9);
        assert_eq!(all.assets[0].held.len(), 2);
        model.manual.create(&path, input(ProviderId::Okx, "BTC-USDT", "BTC", "USDT", 1.)).unwrap();
        let partial = model.portfolio();
        assert_eq!(partial.total, Some(1000.));
        assert!(partial.partial && partial.assets[0].partial);
        assert_eq!(partial.assets[0].amount, 4.);
        assert_eq!(partial.assets[0].price, None);
        assert!((partial.change.unwrap() - 60.).abs() < 1e-9);
    }

    // contract: persistence failures preserve both the saved records and the
    // currently published quantities; a damaged file is never overwritten.
    #[test]
    fn failed_writes_and_damaged_files_preserve_records() {
        let dir = directory();
        let path = dir.path().join("manual-holdings.json");
        let mut store = Store::default();
        let entry =
            store.create(&path, input(ProviderId::Ths, "510300.SH", "510300", "CNY", 10.)).unwrap();
        let reloaded = Store::load(&path);
        assert!(reloaded.error.is_none(), "{:?}", reloaded.error);
        assert_eq!(reloaded.entries, store.entries);
        let blocked = dir.path().join("a-directory");
        fs::create_dir(&blocked).unwrap();
        let mut changed = entry.input.clone();
        changed.amount = 20.;
        assert!(store.replace(&blocked, &entry.id, changed).is_err());
        assert_eq!(store.entries[0], entry);
        fs::write(&path, b"damaged file").unwrap();
        let mut damaged = Store::load(&path);
        assert!(damaged.error.is_some());
        assert!(damaged.create(&path, entry.input).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"damaged file");
    }
}
