//! Persistent choices for native windows.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};

use super::chart::model::Drawing;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Line,
    #[default]
    Candle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideTab {
    #[default]
    Book,
    Trades,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorChoice {
    pub id: String,
    pub period: usize,
    pub on: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartPrefs {
    pub interval: u32,
    pub modes: BTreeMap<u32, Mode>,
    pub tab: SideTab,
    pub book_step: usize,
    pub bars: f64,
    pub indicators: Vec<IndicatorChoice>,
    pub drawings: BTreeMap<String, Vec<Drawing>>,
}

impl Default for ChartPrefs {
    fn default() -> Self {
        Self {
            interval: 60,
            modes: BTreeMap::new(),
            tab: SideTab::Book,
            book_step: 0,
            bars: 60.0,
            indicators: Vec::new(),
            drawings: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    pub chart: ChartPrefs,
}

impl UiPrefs {
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "missing parent directory")
        })?;
        fs::create_dir_all(parent)?;
        let temp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        fs::write(&temp, bytes)?;
        if let Err(error) = fs::rename(&temp, path) {
            let _ = fs::remove_file(temp);
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn round_trip_and_invalid_files() {
        let dir = std::env::temp_dir().join(format!(
            "candlewick-prefs-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let path = dir.join("ui.json");
        assert_eq!(UiPrefs::load(&path).chart.interval, 60);
        let mut prefs = UiPrefs::default();
        prefs.chart.interval = 300;
        prefs.chart.modes.insert(300, Mode::Line);
        prefs
            .chart
            .drawings
            .insert("pair".into(), vec![Drawing::Level { id: "a".into(), price: 42.0 }]);
        prefs.save(&path).unwrap();
        let loaded = UiPrefs::load(&path);
        assert_eq!(loaded.chart.interval, 300);
        assert_eq!(loaded.chart.modes.get(&300), Some(&Mode::Line));
        assert_eq!(loaded.chart.drawings["pair"], prefs.chart.drawings["pair"]);
        fs::write(&path, b"{").unwrap();
        assert_eq!(UiPrefs::load(&path).chart.interval, 60);
        fs::remove_dir_all(dir).unwrap();
    }
}
