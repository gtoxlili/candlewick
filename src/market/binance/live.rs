//! The chart's live feed for one pair, all on one combined stream: every
//! aggregate trade, the 24h ticker, and the whole book (a REST snapshot kept
//! current by the diff stream). A REST ticker fills the window before the
//! first pushes.

use std::{borrow::Cow, time::Duration};

use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::time::sleep;

use super::{
    RawTrade, RestTicker, WsTicker,
    depth::{self, Depth, Diff, Snapshot, Sync},
    get,
};
use crate::{
    market::{
        Error, LiveEvent, Stats,
        crypto::{self, Out, Session, Units, backoff},
    },
    model::Instrument,
    net,
};

pub struct Live {
    symbol: String,
    steps: Vec<Units>,
    depth: Depth,
    /// Snapshots taken since a pushed diff last applied: a book that keeps
    /// losing sync backs off instead of spending the IP's request weight.
    resyncs: u32,
    /// A snapshot is on its way.
    loading: bool,
}

pub enum Fetched {
    Ticker(Option<Stats>),
    Snapshot(Result<Snapshot, Error>),
}

impl Live {
    pub(super) fn new(instrument: &Instrument) -> Self {
        Self {
            symbol: instrument.symbol.clone(),
            steps: crypto::book_steps(instrument),
            depth: Depth::default(),
            resyncs: 0,
            loading: false,
        }
    }

    /// The whole book as it is after `delay`.
    fn load(&mut self, out: &mut Out<Fetched>, delay: Duration) {
        self.loading = true;
        let path = format!(
            "/api/v3/depth?symbol={}&limit={}",
            net::percent_encode(&self.symbol),
            depth::SNAPSHOT_LEVELS
        );
        out.fetch(async move {
            sleep(delay).await;
            Fetched::Snapshot(get::<Snapshot>(&path).await)
        });
    }

    /// Takes another snapshot after a backoff.
    fn resync(&mut self, out: &mut Out<Fetched>) {
        self.load(out, backoff(self.resyncs));
        self.resyncs += 1;
    }
}

impl Session for Live {
    type Fetched = Fetched;

    fn path(&self) -> String {
        let lower = net::percent_encode(&self.symbol.to_lowercase());
        format!("/stream?streams={lower}@aggTrade/{lower}@depth/{lower}@ticker")
    }

    fn connected(&mut self, out: &mut Out<Fetched>) {
        // Diffs wait from the start for the snapshot they apply to.
        self.depth = Depth::default();
        self.resyncs = 1;
        self.load(out, Duration::ZERO);
        // The ticker as it is now; if this fails, the pushes bring it.
        let path = format!("/api/v3/ticker/24hr?symbol={}", net::percent_encode(&self.symbol));
        out.fetch(async move {
            Fetched::Ticker(get::<RestTicker>(&path).await.ok().map(|ticker| ticker.stats()))
        });
    }

    fn frame(&mut self, text: &str, out: &mut Out<Fetched>) {
        let Ok(envelope) = serde_json::from_str::<Envelope>(text) else { return };
        let data = envelope.data.get();
        match envelope.stream.rsplit_once('@').map(|(_, kind)| kind) {
            Some("aggTrade") => {
                if let Ok(trade) = serde_json::from_str::<RawTrade>(data) {
                    out.trade(trade.trade());
                }
            }
            Some("depth") => {
                let Ok(diff) = serde_json::from_str::<Diff>(data) else { return };
                match self.depth.diff(diff) {
                    Sync::Changed => {
                        self.resyncs = 0;
                        out.event(LiveEvent::Book { books: self.depth.books(&self.steps) });
                    }
                    Sync::Unchanged => {}
                    Sync::Lost => {
                        log::info!("order book for {} lost sync", self.symbol);
                        if !self.loading {
                            self.resync(out);
                        }
                    }
                }
            }
            Some("ticker") => {
                if let Ok(ticker) = serde_json::from_str::<WsTicker>(data) {
                    out.event(LiveEvent::Stats { stats: ticker.stats() });
                }
            }
            _ => {}
        }
    }

    fn fetched(&mut self, fetched: Fetched, out: &mut Out<Fetched>) {
        match fetched {
            Fetched::Ticker(stats) => {
                if let Some(stats) = stats {
                    out.event(LiveEvent::Stats { stats });
                }
            }
            Fetched::Snapshot(result) => {
                self.loading = false;
                let synced = match result {
                    Ok(snapshot) => self.depth.snapshot(snapshot),
                    Err(e) => {
                        log::info!("order book snapshot for {} failed: {e}", self.symbol);
                        Sync::Lost
                    }
                };
                if let Sync::Lost = synced {
                    self.resync(out);
                } else {
                    out.event(LiveEvent::Book { books: self.depth.books(&self.steps) });
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    stream: Cow<'a, str>,
    #[serde(borrow)]
    data: &'a RawValue,
}
