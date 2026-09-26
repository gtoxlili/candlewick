//! HTTP requests to market-data services, through reqwest. They take the same
//! route as the websockets: `net` picks the system proxy per host, and a proxy
//! that is configured but not running is skipped, as `net::connect` skips it.

use std::{error::Error as _, io, sync::LazyLock, time::Duration};

use reqwest::{Client, Proxy, RequestBuilder, Response};
use serde::de::DeserializeOwned;

use crate::net;

struct Clients {
    /// Follows the system proxy settings, looked up per request.
    routed: Client,
    /// For when the configured proxy refuses connections.
    direct: Client,
}

static CLIENTS: LazyLock<Clients> = LazyLock::new(|| Clients {
    routed: builder()
        .proxy(Proxy::custom(|url| url.host_str().and_then(net::proxy_url)))
        .build()
        .expect("HTTP client with a preconfigured TLS setup"),
    direct: builder().no_proxy().build().expect("HTTP client with a preconfigured TLS setup"),
});

fn builder() -> reqwest::ClientBuilder {
    Client::builder()
        .use_preconfigured_tls(net::tls_client_config())
        .user_agent(concat!("Candlewick/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        // Paging back through candles makes runs of requests to one host.
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(1)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("请求超时")]
    Timeout,
    #[error("无法连接服务器")]
    Connect,
    #[error("服务器返回 HTTP {0}")]
    Status(u16),
    #[error("数据格式有误")]
    Format,
    #[error("网络错误")]
    Other,
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        log::debug!("http: {e:?}");
        if e.is_timeout() {
            Self::Timeout
        } else if e.is_connect() {
            Self::Connect
        } else if e.is_decode() {
            Self::Format
        } else if let Some(status) = e.status() {
            Self::Status(status.as_u16())
        } else {
            Self::Other
        }
    }
}

/// Sends the request `build` makes, again without the proxy if the configured
/// one refuses the connection.
pub async fn send(build: impl Fn(&Client) -> RequestBuilder) -> Result<Response, Error> {
    let clients = &*CLIENTS;
    match build(&clients.routed).send().await {
        Err(e) if proxy_refused(&e) => {
            log::warn!("proxy refused the connection, retrying directly");
            Ok(build(&clients.direct).send().await?)
        }
        result => Ok(result?),
    }
}

/// GETs `url` and parses its JSON body; other statuses than 2xx fail.
pub async fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, Error> {
    let response = send(|client| client.get(url)).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    Ok(response.json().await?)
}

/// The connection was refused while a proxy was configured for the host.
fn proxy_refused(error: &reqwest::Error) -> bool {
    let proxied = error.url().and_then(|url| url.host_str()).and_then(net::proxy_url).is_some();
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<io::Error>() {
            return proxied && io.kind() == io::ErrorKind::ConnectionRefused;
        }
        source = cause.source();
    }
    false
}
