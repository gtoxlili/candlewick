//! The read-only HTTP API agents call (its contract is `SKILL.md`). It listens
//! on the loopback interface only and answers programs on this machine that
//! carry the token, never web pages (see [`admit`]).

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Query, Request, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use ring::{hmac, rand::SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{
    AppHandle, Manager,
    ipc::{Channel, InvokeResponseBody},
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, sleep, timeout},
};

use crate::{
    bar,
    market::{self, Candle, LiveEvent, ProviderId, Trade},
    model::{Instrument, Session, Shared},
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
    app: AppHandle,
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

pub fn router(app: AppHandle, port: u16, token: &str) -> Router {
    let key = hmac::Key::generate(hmac::HMAC_SHA256, &SystemRandom::new())
        .expect("the system's random number generator");
    let tag = hmac::sign(&key, token.as_bytes());
    let api = Api { app, port, key: Arc::new(key), token: Arc::new(tag) };
    Router::new()
        .route("/v1", get(index))
        .route("/v1/portfolio", get(portfolio))
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
    /// Stocks can be read: Longbridge credentials are set.
    stocks: bool,
    /// Candle intervals in seconds, by source.
    intervals: BTreeMap<ProviderId, Vec<u32>>,
}

async fn index(State(api): State<Api>) -> Json<Index> {
    let shared = api.app.state::<Shared>();
    let exchange = shared.model().settings.exchange;
    let (holdings, stocks) = {
        let credentials = shared.credentials();
        (credentials.exchanges.keys().copied().collect(), credentials.longbridge.is_some())
    };
    let intervals = ProviderId::ALL
        .into_iter()
        .map(|provider| {
            let spec = provider.provider().chart_spec(&bare(provider, ""));
            (provider, spec.intervals.iter().map(|i| i.secs).collect())
        })
        .collect();
    Json(Index {
        app: "Candlewick",
        version: api.app.package_info().version.to_string(),
        exchange,
        holdings,
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
    let shared = api.app.state::<Shared>();
    let ready = |shared: &Shared| shared.model().accounts.values().all(current);
    if !ready(&shared) {
        // As the dropdown opening would.
        bar::menu_opening(&api.app);
        let deadline = Instant::now() + REFRESH_WAIT;
        while !ready(&shared) && Instant::now() < deadline {
            sleep(Duration::from_millis(200)).await;
        }
    }
    Json(Portfolio::of(&shared.model().accounts))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistEntry {
    id: String,
    symbol: String,
    base: String,
    quote: String,
    name: Option<String>,
    pinned: bool,
    last: Option<f64>,
    /// 24 hours ago for crypto, the last regular close for stocks.
    reference: Option<f64>,
    change_pct: Option<f64>,
    session: Option<Session>,
}

async fn watchlist(State(api): State<Api>) -> Json<Vec<WatchlistEntry>> {
    let shared = api.app.state::<Shared>();
    let model = shared.model();
    let entries = model
        .settings
        .watchlist
        .iter()
        .map(|instrument| {
            let id = instrument.id();
            let quote = model.quotes.get(&id);
            WatchlistEntry {
                symbol: instrument.symbol.clone(),
                base: instrument.base.clone(),
                quote: instrument.quote.clone(),
                name: instrument.name.clone(),
                pinned: instrument.pinned,
                last: quote.map(|q| q.last),
                reference: quote.map(|q| q.open),
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
    base: String,
    quote: String,
    name: Option<String>,
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
    let exchange = api.app.state::<Shared>().model().settings.exchange;
    let searched = market::search(exchange, &q).await;
    let found = searched
        .candidates
        .into_iter()
        .map(|c| Found {
            id: c.instrument.id(),
            base: c.instrument.base,
            quote: c.instrument.quote,
            name: c.instrument.name,
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

/// Day statistics and the book, from the first data of a live connection.
async fn market_now(
    State(api): State<Api>,
    query: Result<Query<MarketQuery>, QueryRejection>,
) -> Answer<Value> {
    let Query(MarketQuery { id }) = query.map_err(bad_query)?;
    let instrument = resolve(&api.app, &id)?;
    let (events, mut received) = mpsc::unbounded_channel::<Value>();
    // The live stream speaks to the chart window's channel; here it is a
    // closure that keeps what it says.
    let channel = Channel::<LiveEvent>::new(move |body| {
        if let InvokeResponseBody::Json(text) = body
            && let Ok(event) = serde_json::from_str::<Value>(&text)
        {
            let _ = events.send(event);
        }
        Ok(())
    });
    let (stop, stopped) = oneshot::channel();
    let stream = instrument.provider.provider().stream(instrument, stopped, channel);
    tauri::async_runtime::spawn(stream);
    let (mut stats, mut book) = (Value::Null, Value::Null);
    let _ = timeout(MARKET_WAIT, async {
        while let Some(event) = received.recv().await {
            match event["kind"].as_str() {
                Some("stats") => stats = event["stats"].clone(),
                Some("book") => book = event["books"][0].clone(),
                _ => {}
            }
            if !stats.is_null() && !book.is_null() {
                break;
            }
        }
    })
    .await;
    drop(stop);
    Ok(Json(json!({ "stats": stats, "book": book })))
}

/// The instrument an id names: the watchlist's entry when it is one, else
/// just the source and symbol, which is all reading data takes.
fn resolve(app: &AppHandle, id: &str) -> Result<Instrument, Failure> {
    let invalid = || {
        failure(
            StatusCode::BAD_REQUEST,
            format!("invalid id {id}: expected source:symbol, such as okx:BTC-USDT"),
        )
    };
    let (source, symbol) = id.split_once(':').ok_or_else(invalid)?;
    let provider = ProviderId::ALL.into_iter().find(|p| p.key() == source).ok_or_else(invalid)?;
    let symbol = symbol.trim().to_uppercase();
    if symbol.is_empty() {
        return Err(invalid());
    }
    let shared = app.state::<Shared>();
    let model = shared.model();
    let known =
        model.settings.watchlist.iter().find(|i| i.provider == provider && i.symbol == symbol);
    Ok(known.cloned().unwrap_or_else(|| bare(provider, &symbol)))
}

fn bare(provider: ProviderId, symbol: &str) -> Instrument {
    Instrument {
        provider,
        symbol: symbol.to_owned(),
        base: String::new(),
        quote: String::new(),
        name: None,
        decimals: None,
        pinned: false,
    }
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
