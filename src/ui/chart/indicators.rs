//! Moving averages and Bollinger Bands over the candles on hand. Each series
//! has one value per candle and NaN until it has enough history.

use crate::market::Candle;

/// The indicators the toolbar offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorKind {
    Simple,
    Exponential,
    Bollinger,
}

/// One indicator switched on or off, with its window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndicatorSpec {
    pub id: String,
    pub kind: IndicatorKind,
    /// Candles in the window, [`MIN_PERIOD`] to [`MAX_PERIOD`].
    pub period: usize,
    pub on: bool,
}

/// The common set, off until switched on.
pub fn default_indicators() -> Vec<IndicatorSpec> {
    let spec =
        |id: &str, kind, period| IndicatorSpec { id: id.to_owned(), kind, period, on: false };
    vec![
        spec("ma1", IndicatorKind::Simple, 7),
        spec("ma2", IndicatorKind::Simple, 25),
        spec("ma3", IndicatorKind::Simple, 99),
        spec("ema", IndicatorKind::Exponential, 12),
        spec("boll", IndicatorKind::Bollinger, 20),
    ]
}

/// Standard deviations the bands sit from the middle.
pub const BOLL_MULT: f64 = 2.0;
pub const MIN_PERIOD: usize = 2;
pub const MAX_PERIOD: usize = 500;

impl IndicatorSpec {
    /// A period the indicators and the toolbar both accept.
    pub fn clamped_period(period: usize) -> usize {
        period.clamp(MIN_PERIOD, MAX_PERIOD)
    }

    /// "MA7", "EMA12", "BOLL(20,2)".
    pub fn label(&self) -> String {
        match self.kind {
            IndicatorKind::Simple => format!("MA{}", self.period),
            IndicatorKind::Exponential => format!("EMA{}", self.period),
            IndicatorKind::Bollinger => format!("BOLL({},{})", self.period, BOLL_MULT),
        }
    }
}

/// An indicator's values, one per candle (NaN before it has enough history).
#[derive(Clone, Debug, PartialEq)]
pub struct IndicatorSeries {
    pub spec: IndicatorSpec,
    pub values: Vec<f64>,
    /// Bollinger Bands only.
    pub upper: Option<Vec<f64>>,
    pub lower: Option<Vec<f64>>,
}

/// `candles` oldest first, the forming one last.
pub fn compute(spec: &IndicatorSpec, candles: &[Candle]) -> IndicatorSeries {
    let n = candles.len();
    let period = IndicatorSpec::clamped_period(spec.period);
    let mut values = vec![f64::NAN; n];
    match spec.kind {
        IndicatorKind::Simple => {
            let mut sum = 0.0;
            for i in 0..n {
                sum += candles[i].close;
                if i >= period {
                    sum -= candles[i - period].close;
                }
                if i >= period - 1 {
                    values[i] = sum / period as f64;
                }
            }
            IndicatorSeries { spec: spec.clone(), values, upper: None, lower: None }
        }
        IndicatorKind::Exponential => {
            let k = 2.0 / (period as f64 + 1.0);
            let mut sum = 0.0;
            for i in 0..n {
                let close = candles[i].close;
                if i < period {
                    sum += close;
                    if i == period - 1 {
                        values[i] = sum / period as f64;
                    }
                } else {
                    values[i] = close * k + values[i - 1] * (1.0 - k);
                }
            }
            IndicatorSeries { spec: spec.clone(), values, upper: None, lower: None }
        }
        IndicatorKind::Bollinger => {
            let mut upper = vec![f64::NAN; n];
            let mut lower = vec![f64::NAN; n];
            let mut sum = 0.0;
            let mut squares = 0.0;
            for i in 0..n {
                let close = candles[i].close;
                sum += close;
                squares += close * close;
                if i >= period {
                    let gone = candles[i - period].close;
                    sum -= gone;
                    squares -= gone * gone;
                }
                if i >= period - 1 {
                    let mean = sum / period as f64;
                    let variance = (squares / period as f64 - mean * mean).max(0.0);
                    let dev = variance.sqrt() * BOLL_MULT;
                    values[i] = mean;
                    upper[i] = mean + dev;
                    lower[i] = mean - dev;
                }
            }
            IndicatorSeries { spec: spec.clone(), values, upper: Some(upper), lower: Some(lower) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twelve closes, and candles around them.
    fn candles() -> Vec<Candle> {
        let closes =
            [44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.1, 45.42, 45.84, 46.08, 45.89, 46.03];
        closes
            .iter()
            .enumerate()
            .map(|(i, close)| Candle {
                time: 1000.0 + i as f64 * 60.0,
                open: close - 0.2,
                high: close + 0.5,
                low: close - 0.5,
                close: *close,
            })
            .collect()
    }

    fn spec(kind: IndicatorKind, period: usize) -> IndicatorSpec {
        IndicatorSpec { id: "i".to_owned(), kind, period, on: true }
    }

    /// Nan compares unequal to itself, so check it explicitly.
    fn assert_values(actual: &[f64], expected: &[f64]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            if expected.is_nan() {
                assert!(actual.is_nan(), "value {i}: {actual} is not NaN");
            } else {
                assert_eq!(actual, expected, "value {i}");
            }
        }
    }

    const SMA5: [f64; 12] = [
        f64::NAN,
        f64::NAN,
        f64::NAN,
        f64::NAN,
        44.104,
        44.20199999999999,
        44.40399999999999,
        44.65799999999999,
        45.103999999999985,
        45.453999999999986,
        45.66599999999998,
        45.85199999999998,
    ];

    #[test]
    fn simple_moving_average_warms_up_then_rolls() {
        let series = compute(&spec(IndicatorKind::Simple, 5), &candles());
        assert_values(&series.values, &SMA5);
        assert_eq!(series.upper, None);
        assert_eq!(series.lower, None);
    }

    #[test]
    fn exponential_moving_average_starts_from_the_simple_one() {
        let series = compute(&spec(IndicatorKind::Exponential, 5), &candles());
        assert_values(
            &series.values,
            &[
                f64::NAN,
                f64::NAN,
                f64::NAN,
                f64::NAN,
                44.104,
                44.346000000000004,
                44.59733333333334,
                44.87155555555556,
                45.19437037037038,
                45.48958024691359,
                45.623053497942394,
                45.7587023319616,
            ],
        );
    }

    #[test]
    fn a_period_as_long_as_the_history_has_one_value() {
        let series = compute(&spec(IndicatorKind::Exponential, 12), &candles());
        let mut expected = [f64::NAN; 12];
        expected[11] = 44.975833333333334;
        assert_values(&series.values, &expected);
    }

    #[test]
    fn a_period_longer_than_the_history_stays_empty() {
        let series = compute(&spec(IndicatorKind::Simple, 500), &candles());
        assert_values(&series.values, &[f64::NAN; 12]);
    }

    #[test]
    fn bollinger_bands_are_two_deviations_around_the_average() {
        let series = compute(&spec(IndicatorKind::Bollinger, 5), &candles());
        assert_values(&series.values, &SMA5);
        assert_values(
            series.upper.as_ref().unwrap(),
            &[
                f64::NAN,
                f64::NAN,
                f64::NAN,
                f64::NAN,
                44.63550352773976,
                44.990152269553505,
                45.44949318505742,
                45.92653616424705,
                46.1299512659009,
                46.37344330983739,
                46.377460469742495,
                46.3183732410909,
            ],
        );
        assert_values(
            series.lower.as_ref().unwrap(),
            &[
                f64::NAN,
                f64::NAN,
                f64::NAN,
                f64::NAN,
                43.57249647226024,
                43.41384773044648,
                43.35850681494256,
                43.38946383575293,
                44.07804873409907,
                44.53455669016258,
                44.95453953025747,
                45.38562675890906,
            ],
        );
    }

    #[test]
    fn periods_outside_the_range_are_clamped() {
        let short = compute(&spec(IndicatorKind::Simple, 0), &candles());
        let expected = compute(&spec(IndicatorKind::Simple, 2), &candles());
        assert_values(&short.values, &expected.values);
        let long = compute(&spec(IndicatorKind::Simple, 999), &candles());
        assert_values(&long.values, &[f64::NAN; 12]);
    }

    #[test]
    fn labels_name_the_window_and_the_bands() {
        assert_eq!(spec(IndicatorKind::Simple, 7).label(), "MA7");
        assert_eq!(spec(IndicatorKind::Exponential, 12).label(), "EMA12");
        assert_eq!(spec(IndicatorKind::Bollinger, 20).label(), "BOLL(20,2)");
    }

    #[test]
    fn the_default_set_is_the_common_one() {
        let defaults = default_indicators();
        let labels: Vec<String> = defaults.iter().map(IndicatorSpec::label).collect();
        assert_eq!(labels, ["MA7", "MA25", "MA99", "EMA12", "BOLL(20,2)"]);
        assert!(defaults.iter().all(|spec| !spec.on));
    }
}
