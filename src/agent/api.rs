//! The local HTTP API agents call (its contract is `SKILL.md`). It listens
//! on the loopback interface only and answers programs on this machine that
//! carry the token, never web pages (see [`admit`]).

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use crate::app::App;
use axum::{
    Json, Router,
    extract::{
        Path, Query, Request, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use ring::{hmac, rand::SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, sleep, timeout},
};

use crate::{
    bar,
    market::{self, Candle, LiveEvent, ProviderId, Trade},
    model::{Instrument, Session, Shared, Venue},
    portfolio::{self, Portfolio, now_ms},
};

/// Holdings older than this refresh before `/v1/portfolio` answers.
const FRESH_MS: f64 = 15_000.0;
/// The longest `/v1/portfolio` waits for those refreshes.
const REFRESH_WAIT: Duration = Duration::from_secs(10);
/// The longest `/v1/market` waits for a live connection's first data.
const MARKET_WAIT: Duration = Duration::from_secs(8);
const MAX_TRADES: usize = 1000;
const MAX_CANDLES: usize = 1000;

#[derive(Clone)]
struct Api {
    app: App,
    port: u16,
    /// The token as a MAC under a key of this run's own: comparing MACs takes
    /// the same time however much of a guess is right.
    key: Arc<hmac::Key>,
    token: Arc<hmac::Tag>,
}

impl Api {
    fn knows(&self, presented: &str) -> bool {
        hmac::verify(&self.key, presented.as_bytes(), (*self.token).as_ref()).is_ok()
    }
}

pub fn router(app: App, port: u16, token: &str) -> Router {
    let key = hmac::Key::generate(hmac::HMAC_SHA256, &SystemRandom::new())
        .expect("the system's random number generator");
    let tag = hmac::sign(&key, token.as_bytes());
    let api = Api { app, port, key: Arc::new(key), token: Arc::new(tag) };
    Router::new()
        .route("/v1", get(index))
        .route("/v1/portfolio", get(portfolio))
        .route("/v1/manual-holdings", get(manual_holdings).post(create_manual_holding))
        .route(
            "/v1/manual-holdings/{id}",
            get(manual_holding).put(replace_manual_holding).delete(remove_manual_holding),
        )
        .route("/v1/watchlist", get(watchlist))
        .route("/v1/search", get(search))
        .route("/v1/candles", get(candles))
        .route("/v1/trades", get(trades))
        .route("/v1/market", get(market_now))
        .fallback(|| async { failure(StatusCode::NOT_FOUND, "no such endpoint; see GET /v1") })
        .layer(middleware::from_fn_with_state(api.clone(), guard))
        .with_state(api)
}

async fn guard(State(api): State<Api>, request: Request, next: Next) -> Response {
    match admit(request.headers(), api.port, |token| api.knows(token)) {
        Ok(()) => next.run(request).await,
        Err(refused) => refused.into_response(),
    }
}

/// Lets in a request from a program on this machine that carries the token.
/// Browsers name the page a request comes from (`Origin`), and a page that
/// rebinds its own name to this address still sends that name as `Host`.
fn admit(headers: &HeaderMap, port: u16, knows: impl Fn(&str) -> bool) -> Result<(), Failure> {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    let local = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if headers.contains_key(header::ORIGIN) || !local.iter().any(|l| l == host) {
        return Err(failure(StatusCode::FORBIDDEN, "only programs on this machine may call"));
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    if !bearer.is_some_and(knows) {
        return Err(failure(StatusCode::UNAUTHORIZED, "missing or wrong token"));
    }
    Ok(())
}

/// A refused or failed request: `{"error": "…"}` with its status.
struct Failure(StatusCode, String);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn failure(status: StatusCode, message: impl Into<String>) -> Failure {
    Failure(status, message.into())
}

type Answer<T> = Result<Json<T>, Failure>;

/// What the API sees and offers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    app: &'static str,
    version: String,
    /// The crypto exchange the watchlist's pairs and searches come from.
    exchange: ProviderId,
    /// Exchanges whose holdings `/v1/portfolio` reads.
    holdings: Vec<ProviderId>,
    manual_holdings: bool,
    /// Stock sources whose credentials are set.
    stocks: Vec<ProviderId>,
    /// Candle intervals in seconds, by source.
    intervals: BTreeMap<ProviderId, Vec<u32>>,
}

async fn index(State(api): State<Api>) -> Json<Index> {
    let shared = api.app.shared();
    let exchange = shared.model().settings.exchange;
    let (holdings, stocks) = {
        let credentials = shared.credentials();
        let stocks = [
            (ProviderId::Longbridge, credentials.longbridge.is_some()),
            (ProviderId::Ths, credentials.ths.is_some()),
        ];
        let stocks = stocks.into_iter().filter_map(|(source, set)| set.then_some(source)).collect();
        (credentials.exchanges.keys().copied().collect(), stocks)
    };
    let intervals = ProviderId::ALL
        .into_iter()
        .map(|provider| {
            let spec = provider.provider().chart_spec(&Instrument::bare(provider, ""));
            (provider, spec.intervals.iter().map(|i| i.secs).collect())
        })
        .collect();
    Json(Index {
        app: "Candlewick",
        version: env!("CARGO_PKG_VERSION").to_owned(),
        exchange,
        holdings,
        manual_holdings: true,
        stocks,
        intervals,
    })
}

/// The holdings, first refreshing accounts older than [`FRESH_MS`].
async fn portfolio(State(api): State<Api>) -> Json<Portfolio> {
    let asked = now_ms();
    let current = |account: &portfolio::Account| {
        account.updated.is_some_and(|at| asked - at < FRESH_MS)
            || account.checked.is_some_and(|at| at >= asked)
    };
    let shared = api.app.shared();
    let ready = |shared: &Shared| shared.model().accounts.values().all(current);
    if !ready(shared) {
        // As the dropdown opening would.
        bar::menu_opening(&api.app);
        let deadline = Instant::now() + REFRESH_WAIT;
        while !ready(shared) && Instant::now() < deadline {
            sleep(Duration::from_millis(200)).await;
        }
    }
    Json(shared.model().portfolio())
}

fn manual_failure(error: crate::manual::Error) -> Failure {
    let status = match error {
        crate::manual::Error::Invalid(_) => StatusCode::BAD_REQUEST,
        crate::manual::Error::NotFound => StatusCode::NOT_FOUND,
        crate::manual::Error::Io(_) | crate::manual::Error::Unavailable(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    failure(status, error.to_string())
}

async fn manual_holdings(State(api): State<Api>) -> Answer<Vec<crate::manual::Holding>> {
    crate::api::get_manual_holdings(&api.app).map(Json).map_err(manual_failure)
}

async fn manual_holding(
    State(api): State<Api>,
    Path(id): Path<String>,
) -> Answer<crate::manual::Holding> {
    let entries = crate::api::get_manual_holdings(&api.app).map_err(manual_failure)?;
    entries
        .into_iter()
        .find(|entry| entry.id == id)
        .map(Json)
        .ok_or_else(|| manual_failure(crate::manual::Error::NotFound))
}

async fn create_manual_holding(
    State(api): State<Api>,
    input: Result<Json<crate::manual::Input>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::manual::Holding>), Failure> {
    let Json(input) = input.map_err(|error| failure(StatusCode::BAD_REQUEST, error.body_text()))?;
    crate::api::create_manual_holding(&api.app, input)
        .map(|entry| (StatusCode::CREATED, Json(entry)))
        .map_err(manual_failure)
}

async fn replace_manual_holding(
    State(api): State<Api>,
    Path(id): Path<String>,
    input: Result<Json<crate::manual::Input>, JsonRejection>,
) -> Answer<crate::manual::Holding> {
    let Json(input) = input.map_err(|error| failure(StatusCode::BAD_REQUEST, error.body_text()))?;
    crate::api::replace_manual_holding(&api.app, &id, input).map(Json).map_err(manual_failure)
}

async fn remove_manual_holding(
    State(api): State<Api>,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    crate::api::remove_manual_holding(&api.app, &id).map_err(manual_failure)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistEntry {
    id: String,
    metadata: market::InstrumentMeta,
    symbol: String,
    base: String,
    quote: String,
    name: Option<String>,
    /// A pool's chain and DEX.
    venue: Option<Venue>,
    pinned: bool,
    last: Option<f64>,
    /// 24 hours ago for crypto, the last regular close for stocks.
    reference: Option<f64>,
    price_updated: Option<f64>,
    received: Option<f64>,
    change_pct: Option<f64>,
    session: Option<Session>,
}

async fn watchlist(State(api): State<Api>) -> Json<Vec<WatchlistEntry>> {
    let shared = api.app.shared();
    let model = shared.model();
    let entries = model
        .settings
        .watchlist
        .iter()
        .map(|instrument| {
            let id = instrument.id();
            let quote = model.quotes.get(&id);
            WatchlistEntry {
                metadata: instrument.metadata(),
                symbol: instrument.symbol.clone(),
                base: instrument.base.clone(),
                quote: instrument.quote.clone(),
                name: instrument.name.clone(),
                venue: instrument.venue.clone(),
                pinned: instrument.pinned,
                last: quote.map(|q| q.last),
                reference: quote.map(|q| q.open).filter(|price| price.is_finite() && *price > 0.),
                price_updated: quote.and_then(|quote| quote.time.source),
                received: quote.map(|quote| quote.time.received).filter(|time| *time > 0.),
                change_pct: quote.and_then(|q| portfolio::pct(q.last, q.open)),
                session: quote.and_then(|q| q.session),
                id,
            }
        })
        .collect();
    Json(entries)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Serialize)]
struct Found {
    id: String,
    metadata: market::InstrumentMeta,
    instrument: Instrument,
    /// A pool's liquidity in USD.
    liquidity: Option<f64>,
}

#[derive(Serialize)]
struct Searched {
    found: Vec<Found>,
    /// Why results may be missing.
    notes: Vec<String>,
}

async fn search(
    State(api): State<Api>,
    query: Result<Query<SearchQuery>, QueryRejection>,
) -> Answer<Searched> {
    let Query(SearchQuery { q }) = query.map_err(bad_query)?;
    let exchange = api.app.shared().model().settings.exchange;
    let searched = market::search(exchange, &q).await;
    let found = searched
        .candidates
        .into_iter()
        .map(|c| Found {
            id: c.instrument.id(),
            metadata: c.instrument.metadata(),
            instrument: c.instrument,
            liquidity: c.liquidity,
        })
        .collect();
    Ok(Json(Searched { found, notes: searched.notes }))
}

#[derive(Deserialize)]
struct CandlesQuery {
    id: String,
    interval: u32,
    limit: Option<usize>,
    /// Epoch seconds: candles that opened before it.
    end: Option<f64>,
}

async fn candles(
    State(api): State<Api>,
    query: Result<Query<CandlesQuery>, QueryRejection>,
) -> Answer<Vec<Candle>> {
    let Query(query) = query.map_err(bad_query)?;
    let instrument = resolve(&api.app, &query.id)?;
    let limit = query.limit.unwrap_or(500).clamp(1, MAX_CANDLES);
    let provider = instrument.provider.provider();
    let found = provider.history(&instrument, query.interval, query.end, limit).await;
    found.map(Json).map_err(|e| failure(StatusCode::BAD_GATEWAY, e.to_string()))
}

#[derive(Deserialize)]
struct TradesQuery {
    id: String,
    limit: Option<usize>,
}

async fn trades(
    State(api): State<Api>,
    query: Result<Query<TradesQuery>, QueryRejection>,
) -> Answer<Vec<Trade>> {
    let Query(query) = query.map_err(bad_query)?;
    let instrument = resolve(&api.app, &query.id)?;
    let limit = query.limit.unwrap_or(100).clamp(1, MAX_TRADES);
    let found = instrument.provider.provider().recent_trades(&instrument, limit).await;
    found.map(Json).map_err(|e| failure(StatusCode::BAD_GATEWAY, e.to_string()))
}

#[derive(Deserialize)]
struct MarketQuery {
    id: String,
}

/// Day statistics and the book (where the source has one), from the first
/// data of a live connection.
async fn market_now(
    State(api): State<Api>,
    query: Result<Query<MarketQuery>, QueryRejection>,
) -> Answer<Value> {
    let Query(MarketQuery { id }) = query.map_err(bad_query)?;
    let instrument = resolve(&api.app, &id)?;
    let metadata = instrument.metadata();
    let provider = instrument.provider.provider();
    let wants_book = provider.chart_spec(&instrument).book;
    let (events, mut received) = mpsc::unbounded_channel::<LiveEvent>();
    let (stop, stopped) = oneshot::channel();
    let stream = provider.stream(instrument, stopped, events);
    api.app.spawn(stream);
    let (mut stats, mut book) = (Value::Null, Value::Null);
    let _ = timeout(MARKET_WAIT, async {
        while let Some(event) = received.recv().await {
            match event {
                LiveEvent::Stats { stats: value } => stats = json!(value),
                LiveEvent::Book { books } => book = json!(books.first()),
                _ => {}
            }
            if !stats.is_null() && (!wants_book || !book.is_null()) {
                break;
            }
        }
    })
    .await;
    drop(stop);
    Ok(Json(json!({ "metadata": metadata, "stats": stats, "book": book })))
}

/// The instrument an id names: the watchlist's entry when it is one, else
/// just the source and symbol, which is all reading data takes.
fn resolve(app: &App, id: &str) -> Result<Instrument, Failure> {
    let invalid = || {
        failure(
            StatusCode::BAD_REQUEST,
            format!("invalid id {id}: expected source:symbol, such as okx:BTC-USDT"),
        )
    };
    let (source, symbol) = id.split_once(':').ok_or_else(invalid)?;
    let provider = ProviderId::ALL.into_iter().find(|p| p.key() == source).ok_or_else(invalid)?;
    let bare = provider.provider().bare(symbol).ok_or_else(invalid)?;
    let shared = app.shared();
    let model = shared.model();
    let known =
        model.settings.watchlist.iter().find(|i| i.provider == provider && i.symbol == bare.symbol);
    Ok(known.cloned().unwrap_or(bare))
}

fn bad_query(rejection: QueryRejection) -> Failure {
    failure(StatusCode::BAD_REQUEST, rejection.body_text())
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderName, HeaderValue};

    use super::*;

    fn status(pairs: &[(HeaderName, &str)]) -> Option<StatusCode> {
        let headers: HeaderMap = pairs
            .iter()
            .map(|(name, value)| (name.clone(), HeaderValue::from_str(value).unwrap()))
            .collect();
        admit(&headers, 52733, |token| token == "t0ken").err().map(|f| f.0)
    }

    // contract: Eastmoney is a program-wide source. A normal watchlist entry
    // uses its quote and public market routes without any registered holdings.
    #[test]
    fn eastmoney_serves_the_watchlist_and_market_api_without_registered_holdings() {
        use crate::model::{Quote, Settings};
        use axum::body::{Body, to_bytes};
        use gpui_kit::TestAppContext;
        use tower::ServiceExt as _;

        let dir =
            tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
                .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cx = TestAppContext::single();
        let provider = ProviderId::Eastmoney.provider();
        let mut fund = provider.bare("150.000216").unwrap();
        fund.pinned = true;
        let gold = provider.bare("118.AU9999").unwrap();
        let settings =
            Settings { watchlist: vec![fund, gold], ..Default::default() }.validated().unwrap();
        let (shared, _control) = Shared::new(
            settings,
            dir.path().join("settings.json"),
            Default::default(),
            dir.path().join("credentials.json"),
        );
        assert_eq!(
            shared.control.borrow().symbols[&ProviderId::Eastmoney],
            ["118.AU9999", "150.000216"]
        );
        let app = cx.update(|cx| App::new(cx, runtime.handle().clone(), shared));
        {
            let mut model = app.shared().model();
            assert!(model.manual.entries.is_empty());
            assert!(market::record_quote(
                &mut model,
                "eastmoney:150.000216".into(),
                Quote { last: 3.0022, open: 3.0545, session: None, ..Default::default() },
                Some(1_793_894_400_000.)
            ));
            assert!(market::record_quote(
                &mut model,
                "eastmoney:118.AU9999".into(),
                Quote { last: 874., open: 868.57, session: None, ..Default::default() },
                Some(1_793_952_000_000.)
            ));
        }
        let view = bar::view(&app);
        assert_eq!(view.rows[0].value.0, "3.0022");
        assert_eq!(view.rows[1].value.0, "874.00");
        assert!(view.ticker.is_some());
        assert!(view.holdings.is_none(), "market data does not require a portfolio");
        assert_eq!(
            crate::api::chart_instrument(&app, "eastmoney:150.000216".into()).unwrap().quote,
            "CNY"
        );
        assert!(!crate::api::chart_spec(&app, "eastmoney:118.AU9999".into()).unwrap().book);
        let router = router(app.clone(), 52733, "test-token");
        let get = |uri: &str| {
            runtime.block_on(async {
                let request = axum::http::Request::builder()
                    .uri(uri)
                    .header("Host", "localhost:52733")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap();
                let response = router.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
                serde_json::from_slice::<Value>(&bytes).unwrap()
            })
        };
        assert_eq!(get("/v1")["intervals"]["eastmoney"], json!([86400]));
        let watchlist = get("/v1/watchlist");
        assert_eq!(watchlist[0]["id"], "eastmoney:150.000216");
        assert_eq!(watchlist[0]["quote"], "CNY");
        assert_eq!(watchlist[0]["metadata"]["quantityUnit"], "fundUnits");
        assert_eq!(watchlist[0]["metadata"]["priceKind"]["kind"], "nav");
        assert_eq!(watchlist[1]["metadata"]["quantityUnit"], "grams");
        assert_eq!(watchlist[0]["last"], 3.0022);
        assert_eq!(watchlist[1]["last"], 874.);
        // These routes resolve the source even for a symbol outside the watchlist.
        assert_eq!(get("/v1/candles?id=eastmoney:150.002611&interval=86400"), json!([]));
        assert_eq!(get("/v1/trades?id=eastmoney:150.002611"), json!([]));
        assert!(app.shared().model().manual.entries.is_empty());
    }

    // contract: authenticated local clients can maintain registered quantities;
    // the router enforces the same origin/token boundary for writes as reads.
    #[test]
    fn registered_holdings_crud_is_authenticated_persistent_and_live() {
        use crate::model::{Quote, Settings};
        use axum::body::{Body, to_bytes};
        use gpui_kit::TestAppContext;
        use tower::ServiceExt as _;
        let dir =
            tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
                .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cx = TestAppContext::single();
        let (shared, _control) = Shared::new(
            Settings::default(),
            dir.path().join("settings.json"),
            Default::default(),
            dir.path().join("credentials.json"),
        );
        let app = cx.update(|cx| App::new(cx, runtime.handle().clone(), shared));
        let router = router(app.clone(), 52733, "test-token");
        let input = json!({"platform":"Broker", "instrument":{"provider":"ths", "symbol":"510300.SH", "base":"510300", "quote":"CNY"}, "amount":100, "cost":3.5});
        let call = |method: &str, uri: &str, body: Value, authorized: bool, origin: bool| {
            let mut request = axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("Host", "localhost:52733")
                .header("Content-Type", "application/json");
            if authorized {
                request = request.header("Authorization", "Bearer test-token");
            }
            if origin {
                request = request.header("Origin", "https://example.test");
            }
            runtime.block_on(async {
                let response = router
                    .clone()
                    .oneshot(request.body(Body::from(body.to_string())).unwrap())
                    .await
                    .unwrap();
                let status = response.status();
                let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
                (status, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
            })
        };
        assert_eq!(
            call("POST", "/v1/manual-holdings", input.clone(), false, false).0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call("POST", "/v1/manual-holdings", input.clone(), true, true).0,
            StatusCode::FORBIDDEN
        );
        assert!(!dir.path().join("manual-holdings.json").exists());
        let (status, created) = call("POST", "/v1/manual-holdings", input.clone(), true, false);
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().unwrap();
        let uri = format!("/v1/manual-holdings/{id}");
        assert_eq!(call("GET", &uri, Value::Null, true, false).1, created);
        assert_eq!(
            call("GET", "/v1/manual-holdings", Value::Null, true, false)
                .1
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            crate::manual::Store::load(&dir.path().join("manual-holdings.json")).entries.len(),
            1
        );
        assert_eq!(app.shared().control.borrow().symbols[&ProviderId::Ths], ["510300.SH"]);
        // The instrument is subscribed even though it was never on the watchlist.
        {
            let mut model = app.shared().model();
            model.exchange_rates.snapshot = Some(crate::currency::Snapshot {
                date: "2026-11-06".into(),
                usd: crate::currency::Rates { cny: 6.25, jpy: 150., hkd: 7.8 },
            });
            assert!(market::record_quote(
                &mut model,
                "ths:510300.SH".into(),
                Quote { last: 5., open: 4., session: None, ..Default::default() },
                None
            ));
        }
        let portfolio = call("GET", "/v1/portfolio", Value::Null, true, false).1;
        assert_eq!(portfolio["total"], 80.);
        assert_eq!(portfolio["manualHoldings"][0]["price"], 5.);
        assert_eq!(portfolio["assets"][0]["held"][0]["manualId"], id);
        let mut bad = input.clone();
        bad["amount"] = json!(-1);
        assert_eq!(call("PUT", &uri, bad, true, false).0, StatusCode::BAD_REQUEST);
        let mut bad = input.clone();
        bad["price"] = json!(5);
        assert_eq!(
            call("POST", "/v1/manual-holdings", bad, true, false).0,
            StatusCode::BAD_REQUEST,
            "user-supplied prices are not a supported field"
        );
        let mut changed = input.clone();
        changed["amount"] = json!(200);
        assert_eq!(call("PUT", &uri, changed, true, false).1["amount"], 200.);
        assert_eq!(call("GET", "/v1/portfolio", Value::Null, true, false).1["total"], 160.);
        let second = call("POST", "/v1/manual-holdings", input, true, false).1;
        let second_uri = format!("/v1/manual-holdings/{}", second["id"].as_str().unwrap());
        assert_eq!(call("DELETE", &uri, Value::Null, true, false).0, StatusCode::NO_CONTENT);
        assert_eq!(
            app.shared().control.borrow().symbols[&ProviderId::Ths],
            ["510300.SH"],
            "the other record still needs this quote"
        );
        assert_eq!(call("GET", "/v1/portfolio", Value::Null, true, false).1["total"], 80.);
        assert_eq!(call("DELETE", &second_uri, Value::Null, true, false).0, StatusCode::NO_CONTENT);
        assert_eq!(call("GET", &uri, Value::Null, true, false).0, StatusCode::NOT_FOUND);
        assert!(
            crate::manual::Store::load(&dir.path().join("manual-holdings.json")).entries.is_empty()
        );
        assert!(!app.shared().control.borrow().symbols.contains_key(&ProviderId::Ths));
        assert!(!app.shared().model().quotes.contains_key("ths:510300.SH"));
    }

    #[test]
    fn only_local_programs_with_the_token_get_in() {
        let host = (header::HOST, "127.0.0.1:52733");
        let auth = (header::AUTHORIZATION, "Bearer t0ken");
        assert_eq!(status(&[host.clone(), auth.clone()]), None);
        assert_eq!(status(&[(header::HOST, "localhost:52733"), auth.clone()]), None);
        // A page, even one served from a name rebound to this address.
        let origin = (header::ORIGIN, "https://evil.example");
        assert_eq!(status(&[host.clone(), auth.clone(), origin]), Some(StatusCode::FORBIDDEN));
        assert_eq!(
            status(&[(header::HOST, "evil.example:52733"), auth]),
            Some(StatusCode::FORBIDDEN)
        );
        assert_eq!(status(std::slice::from_ref(&host)), Some(StatusCode::UNAUTHORIZED));
        assert_eq!(
            status(&[host, (header::AUTHORIZATION, "Bearer guess")]),
            Some(StatusCode::UNAUTHORIZED)
        );
    }
}
