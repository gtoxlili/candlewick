//! Opens the market-data websockets through the route the system proxy
//! settings prescribe (`platform::proxy`: an HTTPS proxy via CONNECT, else
//! SOCKS5, honoring exceptions), and gives the HTTP client (`http.rs`) the
//! same route and TLS setup.

use std::{
    fmt, io,
    sync::{Arc, OnceLock},
};

use rustls_platform_verifier::BuilderVerifierExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, client_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig},
};

use crate::platform;

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How a connection reaches its host. A proxy `host` is a name or an IP
/// literal, IPv6 without brackets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Direct,
    Http { host: String, port: u16 },
    Socks5 { host: String, port: u16 },
}

/// For the log.
impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct => f.write_str("direct"),
            Self::Http { host, port } => write!(f, "HTTP proxy {}", Authority(host, *port)),
            Self::Socks5 { host, port } => write!(f, "SOCKS5 proxy {}", Authority(host, *port)),
        }
    }
}

impl Route {
    /// The proxy as a URL, or `None` to go direct.
    fn url(&self) -> Option<String> {
        match self {
            Self::Direct => None,
            Self::Http { host, port } => Some(format!("http://{}", Authority(host, *port))),
            // `socks5h`: the proxy resolves the name, as `socks5_connect` has it do.
            Self::Socks5 { host, port } => Some(format!("socks5h://{}", Authority(host, *port))),
        }
    }
}

/// `host:port` as URLs and CONNECT requests write it: an IPv6 literal goes in
/// brackets, or its colons would run into the port's.
struct Authority<'a>(&'a str, u16);

impl fmt::Display for Authority<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self(host, port) = self;
        if host.contains(':') { write!(f, "[{host}]:{port}") } else { write!(f, "{host}:{port}") }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] io::Error),
    /// Why the proxy refused, said in the app's language.
    #[error("{}", refused(.0))]
    Proxy(String),
    #[error("{0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
}

fn refused(reason: &str) -> String {
    t!("error.proxyRefused", reason = reason)
}

/// Connects to `wss://{host}:{port}{path}` through whatever route the system
/// proxy settings prescribe. A configured proxy that is not listening (a proxy
/// app that quit without restoring settings) falls back to a direct connection.
pub async fn connect(
    host: &str,
    port: u16,
    path_and_query: &str,
) -> Result<(Socket, Route), Error> {
    connect_with_headers(host, port, path_and_query, &[]).await
}

/// [`connect`], with extra headers on the upgrade request.
pub async fn connect_with_headers(
    host: &str,
    port: u16,
    path_and_query: &str,
    headers: &[(&'static str, &str)],
) -> Result<(Socket, Route), Error> {
    let mut route = platform::proxy::route(host);
    let proxy = match &route {
        Route::Direct => None,
        Route::Http { host, port } | Route::Socks5 { host, port } => Some((host.clone(), *port)),
    };
    let tcp = match proxy {
        None => TcpStream::connect((host, port)).await?,
        Some((proxy_host, proxy_port)) => {
            match TcpStream::connect((proxy_host.as_str(), proxy_port)).await {
                Ok(mut tcp) => {
                    if matches!(route, Route::Http { .. }) {
                        http_connect(&mut tcp, host, port).await?;
                    } else {
                        socks5_connect(&mut tcp, host, port).await?;
                    }
                    tcp
                }
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    let proxy = Authority(&proxy_host, proxy_port);
                    log::warn!("proxy {proxy} refused, connecting directly");
                    route = Route::Direct;
                    TcpStream::connect((host, port)).await?
                }
                Err(e) => return Err(e.into()),
            }
        }
    };
    tcp.set_nodelay(true)?;

    // Frames are ~250 bytes; the 128 KiB defaults would only waste memory.
    let config = WebSocketConfig::default()
        .read_buffer_size(4 * 1024)
        .write_buffer_size(0)
        .max_message_size(Some(256 * 1024))
        .max_frame_size(Some(256 * 1024));
    let authority = if port == 443 { host.to_owned() } else { Authority(host, port).to_string() };
    let mut request = format!("wss://{authority}{path_and_query}").into_client_request()?;
    for (name, value) in headers {
        let value = HeaderValue::from_str(value).map_err(|e| Error::Proxy(e.to_string()))?;
        request.headers_mut().insert(*name, value);
    }
    let (socket, _) = client_async_tls_with_config(
        request,
        tcp,
        Some(config),
        Some(Connector::Rustls(tls_config())),
    )
    .await?;
    Ok((socket, route))
}

/// The system proxy for `host` as a proxy URL, or `None` to go direct.
pub fn proxy_url(host: &str) -> Option<String> {
    platform::proxy::route(host).url()
}

/// The websockets' TLS setup (ring, platform certificate verifier).
pub fn tls_client_config() -> rustls::ClientConfig {
    (*tls_config()).clone()
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .and_then(|builder| builder.with_platform_verifier())
                // Only fails on Android (JVM not initialized); unreachable here.
                .expect("ring + platform verifier");
            Arc::new(config.with_no_client_auth())
        })
        .clone()
}

async fn http_connect(tcp: &mut TcpStream, host: &str, port: u16) -> Result<(), Error> {
    let target = Authority(host, port);
    let request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    tcp.write_all(request.as_bytes()).await?;
    // The proxy sends nothing after its header until we start TLS, so reading
    // in chunks cannot swallow tunnel bytes.
    let mut head = Vec::with_capacity(128);
    let mut chunk = [0u8; 256];
    loop {
        let n = tcp.read(&mut chunk).await?;
        if n == 0 {
            return Err(Error::Proxy(t!("error.proxyClosed").to_owned()));
        }
        head.extend_from_slice(&chunk[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if head.len() > 8 * 1024 {
            return Err(Error::Proxy(t!("error.proxyHeaderTooLong").to_owned()));
        }
    }
    let status_line = head.split(|&b| b == b'\r').next().unwrap_or_default();
    let status_line = String::from_utf8_lossy(status_line);
    match status_line.split_whitespace().nth(1) {
        Some(code) if code.starts_with('2') => Ok(()),
        _ => Err(Error::Proxy(status_line.into_owned())),
    }
}

/// RFC 1928, no authentication, remote DNS resolution.
async fn socks5_connect(tcp: &mut TcpStream, host: &str, port: u16) -> Result<(), Error> {
    tcp.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut reply = [0u8; 2];
    tcp.read_exact(&mut reply).await?;
    if reply != [0x05, 0x00] {
        return Err(Error::Proxy(t!("error.proxyAuth").to_owned()));
    }

    let name = host.as_bytes();
    let len = u8::try_from(name.len())
        .map_err(|_| Error::Proxy(t!("error.proxyHostTooLong").to_owned()))?;
    let mut request = Vec::with_capacity(7 + name.len());
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, len]);
    request.extend_from_slice(name);
    request.extend_from_slice(&port.to_be_bytes());
    tcp.write_all(&request).await?;

    let mut head = [0u8; 4];
    tcp.read_exact(&mut head).await?;
    if head[1] != 0x00 {
        return Err(Error::Proxy(t!("error.proxyCode", code = head[1])));
    }
    // Skip the bound address: IPv4, domain (length-prefixed) or IPv6, then port.
    let addr_len = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => usize::from(tcp.read_u8().await?),
        _ => return Err(Error::Proxy(t!("error.proxyAddress").to_owned())),
    };
    let mut rest = vec![0u8; addr_len + 2];
    tcp.read_exact(&mut rest).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(host: &str, port: u16) -> Route {
        Route::Http { host: host.to_owned(), port }
    }

    fn socks5(host: &str, port: u16) -> Route {
        Route::Socks5 { host: host.to_owned(), port }
    }

    // reqwest goes direct without a word when it cannot parse a proxy URL, so
    // each route must give one that parses back to the same host and port.
    #[test]
    fn proxy_urls() {
        let cases = [
            (http("127.0.0.1", 7890), "http://127.0.0.1:7890", "127.0.0.1", 7890),
            (http("proxy.lan", 8080), "http://proxy.lan:8080", "proxy.lan", 8080),
            (http("::1", 7890), "http://[::1]:7890", "[::1]", 7890),
            (socks5("127.0.0.1", 1080), "socks5h://127.0.0.1:1080", "127.0.0.1", 1080),
            (socks5("::1", 1080), "socks5h://[::1]:1080", "[::1]", 1080),
        ];
        for (route, expected, host, port) in cases {
            let url = route.url().expect("a proxy route");
            assert_eq!(url, expected);
            let parsed = reqwest::Url::parse(&url).expect("a valid URL");
            assert_eq!((parsed.host_str(), parsed.port()), (Some(host), Some(port)));
            assert!(reqwest::Proxy::all(&url).is_ok(), "reqwest rejects {url}");
        }
        assert_eq!(Route::Direct.url(), None);
    }

    #[test]
    fn route_labels() {
        assert_eq!(http("::1", 7890).to_string(), "HTTP proxy [::1]:7890");
        assert_eq!(socks5("127.0.0.1", 1080).to_string(), "SOCKS5 proxy 127.0.0.1:1080");
    }
}
