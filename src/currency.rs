//! Daily USD/CNY and USD/JPY display rates. Account prices stay in USD;
//! conversion happens once when a portfolio is read.

use std::{fs, path::Path, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::{
    app::{App, Topic},
    bar, http,
    model::FeedControl,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Cny,
    #[default]
    Usd,
    Jpy,
}

impl Currency {
    pub const ALL: [Self; 3] = [Self::Cny, Self::Usd, Self::Jpy];

    pub fn decimals(self) -> usize {
        if self == Self::Jpy { 0 } else { 2 }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Cny => "¥",
            Self::Usd => "$",
            Self::Jpy => "JP¥",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Cny => "CNY",
            Self::Usd => "USD",
            Self::Jpy => "JPY",
        }
    }
}

/// Also the on-disk cache and the provider's response. Unknown rates in the
/// response are ignored, keeping the cache small.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub date: String,
    pub usd: Rates,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rates {
    pub cny: f64,
    pub jpy: f64,
}

impl Snapshot {
    fn valid(&self) -> bool {
        [self.usd.cny, self.usd.jpy].iter().all(|rate| rate.is_finite() && *rate > 0.)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExchangeRates {
    pub snapshot: Option<Snapshot>,
    pub error: Option<String>,
}

/// All portfolio amounts use this currency and this one rate. The native
/// contract fields retain their own units.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Valuation {
    pub currency: Currency,
    /// Target currency units per USD for the currency actually displayed.
    pub rate: f64,
    pub date: Option<String>,
    pub error: Option<String>,
}

impl Default for Valuation {
    fn default() -> Self {
        Self { currency: Currency::Usd, rate: 1., date: None, error: None }
    }
}

impl ExchangeRates {
    pub fn load(path: &Path) -> Self {
        let snapshot = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Snapshot>(&bytes).ok())
            .filter(Snapshot::valid);
        Self { snapshot, error: None }
    }

    pub fn valuation(&self, currency: Currency) -> Valuation {
        let Some(snapshot) = self.snapshot.as_ref().filter(|_| currency != Currency::Usd) else {
            return Valuation::default();
        };
        let rate = match currency {
            Currency::Cny => snapshot.usd.cny,
            Currency::Jpy => snapshot.usd.jpy,
            Currency::Usd => 1.,
        };
        Valuation { currency, rate, date: Some(snapshot.date.clone()), error: self.error.clone() }
    }
}

const URL: &str =
    "https://cdn.jsdelivr.net/npm/@fawazahmed0/currency-api@latest/v1/currencies/usd.min.json";
const REFRESH: Duration = Duration::from_secs(3600);
const RETRY: Duration = Duration::from_secs(60);

async fn fetch() -> Result<Snapshot, http::Error> {
    let snapshot: Snapshot = http::get_json(URL).await?;
    if !snapshot.valid() {
        return Err(http::Error::Format);
    }
    Ok(snapshot)
}

fn save(path: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(snapshot)?)?;
    fs::rename(tmp, path)
}

/// One worker for all accounts and windows. Sleeps while the machine is
/// paused or there are no account keys; setting changes do not add workers.
pub async fn watch(app: App, mut control: watch::Receiver<FeedControl>) {
    let path = app.shared().settings_path.with_file_name("exchange-rates.json");
    let mut due = tokio::time::Instant::now();
    loop {
        let active = {
            let wanted = control.borrow_and_update();
            wanted.paused == 0 && !wanted.credentials.exchanges.is_empty()
        };
        if active && tokio::time::Instant::now() >= due {
            let result = fetch().await;
            let delay = if result.is_ok() { REFRESH } else { RETRY };
            match result {
                Ok(snapshot) => {
                    if app.shared().model().exchange_rates.snapshot.as_ref() != Some(&snapshot)
                        && let Err(e) = save(&path, &snapshot)
                    {
                        log::warn!("cannot cache exchange rates: {e}");
                    }
                    let mut model = app.shared().model();
                    model.exchange_rates.snapshot = Some(snapshot);
                    model.exchange_rates.error = None;
                }
                Err(error) => {
                    log::warn!("exchange rates: {error}");
                    app.shared().model().exchange_rates.error = Some(error.to_string());
                }
            }
            app.notify(Topic::Portfolio);
            bar::request_render(&app);
            due = tokio::time::Instant::now() + delay;
        }
        tokio::select! {
            _ = tokio::time::sleep_until(due), if active => {},
            changed = control.changed() => if changed.is_err() { return; },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // contract: USD is the application's base valuation unit. A missing
    // or failed foreign exchange rate must not prevent USD values from being shown.
    #[test]
    fn usd_works_without_rates_and_foreign_currencies_use_daily_rates() {
        let mut rates = ExchangeRates::default();
        assert_eq!(rates.valuation(Currency::Usd).rate, 1.);
        assert_eq!(rates.valuation(Currency::Cny), Valuation::default());
        rates.error = Some("network unavailable".into());
        assert_eq!(rates.valuation(Currency::Usd), Valuation::default());
        let snapshot: Snapshot =
            serde_json::from_str(r#"{"date":"2026-11-06","usd":{"cny":6.65,"jpy":150.5}}"#)
                .unwrap();
        assert!(snapshot.valid());
        rates.snapshot = Some(snapshot);
        assert_eq!(rates.valuation(Currency::Cny).rate, 6.65);
        assert_eq!(rates.valuation(Currency::Jpy).rate, 150.5);
        assert_eq!(Currency::Jpy.decimals(), 0);
        assert_eq!(crate::format::price(1234.56, Currency::Jpy.decimals()), "1,235");
        assert_eq!(crate::format::signed(-0.4, Currency::Jpy.decimals()), "0");
        assert!(rates.valuation(Currency::Cny).error.is_some());
        assert_eq!(serde_json::to_string(&Currency::Cny).unwrap(), "\"CNY\"");
        assert_eq!(serde_json::from_str::<Currency>("\"USD\"").unwrap(), Currency::Usd);
        assert_eq!(serde_json::from_str::<Currency>("\"JPY\"").unwrap(), Currency::Jpy);
        assert!(serde_json::from_str::<Currency>("\"USDT\"").is_err());
    }

    #[test]
    fn invalid_conversion_rates_are_rejected() {
        for rate in [0., -1., f64::NAN, f64::INFINITY] {
            for usd in [Rates { cny: rate, jpy: 150.5 }, Rates { cny: 6.65, jpy: rate }] {
                assert!(!Snapshot { date: "2026-11-06".into(), usd }.valid());
            }
        }
        assert!(serde_json::from_str::<Snapshot>(r#"{"date":"2026-11-06","usd":{}}"#).is_err());
    }
}
