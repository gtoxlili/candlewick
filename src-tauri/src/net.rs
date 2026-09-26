//! Opens the market-data websockets, following the macOS system proxy the way
//! Safari would (HTTPS proxy via CONNECT, else SOCKS5, honoring exceptions),
//! and gives the HTTP client (`http.rs`) the same route and TLS setup.

use std::{
    fmt, io,
    sync::{Arc, OnceLock},
};

use objc2_core_foundation::{CFArray, CFNumber, CFString, CFType};
use objc2_system_configuration::{
    SCDynamicStore, kSCPropNetProxiesExceptionsList, kSCPropNetProxiesHTTPSEnable,
    kSCPropNetProxiesHTTPSPort, kSCPropNetProxiesHTTPSProxy, kSCPropNetProxiesSOCKSEnable,
    kSCPropNetProxiesSOCKSPort, kSCPropNetProxiesSOCKSProxy,
};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, client_async_tls_with_config,
    tungstenite::protocol::WebSocketConfig,
};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Direct,
    Http { host: String, port: u16 },
    Socks5 { host: String, port: u16 },
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct => f.write_str("直连"),
            Self::Http { host, port } => write!(f, "HTTP 代理 {host}:{port}"),
            Self::Socks5 { host, port } => write!(f, "SOCKS5 代理 {host}:{port}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("代理拒绝了连接（{0}）")]
    Proxy(String),
    #[error("{0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
}

/// Connects to `wss://{host}{path}` through whatever route the system proxy
/// settings prescribe. A configured proxy that is not listening (a proxy app
/// that quit without restoring settings) falls back to a direct connection.
pub async fn connect(host: &str, path_and_query: &str) -> Result<(Socket, Route), Error> {
    const PORT: u16 = 443;
    let mut route = system_route(host);
    let proxy = match &route {
        Route::Direct => None,
        Route::Http { host, port } | Route::Socks5 { host, port } => Some((host.clone(), *port)),
    };
    let tcp = match proxy {
        None => TcpStream::connect((host, PORT)).await?,
        Some((proxy_host, proxy_port)) => {
            match TcpStream::connect((proxy_host.as_str(), proxy_port)).await {
                Ok(mut tcp) => {
                    if matches!(route, Route::Http { .. }) {
                        http_connect(&mut tcp, host, PORT).await?;
                    } else {
                        socks5_connect(&mut tcp, host, PORT).await?;
                    }
                    tcp
                }
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    log::warn!("proxy {proxy_host}:{proxy_port} refused, connecting directly");
                    route = Route::Direct;
                    TcpStream::connect((host, PORT)).await?
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
    let url = format!("wss://{host}{path_and_query}");
    let (socket, _) =
        client_async_tls_with_config(url, tcp, Some(config), Some(Connector::Rustls(tls_config())))
            .await?;
    Ok((socket, route))
}

/// The system proxy for `host` as a proxy URL, or `None` to go direct.
pub fn proxy_url(host: &str) -> Option<String> {
    match system_route(host) {
        Route::Direct => None,
        Route::Http { host, port } => Some(format!("http://{host}:{port}")),
        // `socks5h`: the proxy resolves the name, as `socks5_connect` has it do.
        Route::Socks5 { host, port } => Some(format!("socks5h://{host}:{port}")),
    }
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
                // Only fails on Android (JVM not initialized); unreachable on macOS.
                .expect("ring + platform verifier");
            Arc::new(config.with_no_client_auth())
        })
        .clone()
}

/// Reads the current system proxy settings (they can change at any time, so
/// this runs on every connection attempt).
fn system_route(target: &str) -> Route {
    let Some(dict) = SCDynamicStore::proxies(None) else {
        return Route::Direct;
    };
    // SAFETY: SCDynamicStoreCopyProxies returns a dictionary keyed by CFString.
    let dict = unsafe { dict.cast_unchecked::<CFString, CFType>() };
    let number = |key: &CFString| {
        dict.get(key)
            .and_then(|v| v.downcast::<CFNumber>().ok())
            .and_then(|n| n.as_i64())
            .unwrap_or(0)
    };
    let string = |key: &CFString| {
        dict.get(key)
            .and_then(|v| v.downcast::<CFString>().ok())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    };

    // SAFETY: the kSCPropNetProxies* statics are immutable CFString constants.
    let (exceptions_key, https, socks) = unsafe {
        (
            kSCPropNetProxiesExceptionsList,
            (kSCPropNetProxiesHTTPSEnable, kSCPropNetProxiesHTTPSProxy, kSCPropNetProxiesHTTPSPort),
            (kSCPropNetProxiesSOCKSEnable, kSCPropNetProxiesSOCKSProxy, kSCPropNetProxiesSOCKSPort),
        )
    };

    let excepted =
        dict.get(exceptions_key).and_then(|v| v.downcast::<CFArray>().ok()).is_some_and(|list| {
            // SAFETY: the exceptions list holds CFStrings.
            let list = unsafe { list.cast_unchecked::<CFString>() };
            list.iter().any(|pattern| host_matches(target, &pattern.to_string()))
        });
    if excepted {
        return Route::Direct;
    }

    let port = |key| u16::try_from(number(key)).ok().filter(|p| *p != 0);
    if number(https.0) == 1
        && let (Some(host), Some(port)) = (string(https.1), port(https.2))
    {
        return Route::Http { host, port };
    }
    if number(socks.0) == 1
        && let (Some(host), Some(port)) = (string(socks.1), port(socks.2))
    {
        return Route::Socks5 { host, port };
    }
    Route::Direct
}

/// Proxy exception patterns: exact host names and `*.suffix` wildcards.
/// CIDR entries only ever match IP literals, which we never dial.
fn host_matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    match pattern.strip_prefix("*.") {
        Some(suffix) => {
            host.eq_ignore_ascii_case(suffix)
                || host.len() > suffix.len()
                    && host[host.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
                    && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
        }
        None => host.eq_ignore_ascii_case(pattern),
    }
}

async fn http_connect(tcp: &mut TcpStream, host: &str, port: u16) -> Result<(), Error> {
    let request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n");
    tcp.write_all(request.as_bytes()).await?;
    // The proxy sends nothing after its header until we start TLS, so reading
    // in chunks cannot swallow tunnel bytes.
    let mut head = Vec::with_capacity(128);
    let mut chunk = [0u8; 256];
    loop {
        let n = tcp.read(&mut chunk).await?;
        if n == 0 {
            return Err(Error::Proxy("连接被关闭".to_owned()));
        }
        head.extend_from_slice(&chunk[..n]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if head.len() > 8 * 1024 {
            return Err(Error::Proxy("响应头过长".to_owned()));
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
    let fail = |what: &str| Error::Proxy(format!("SOCKS5 {what}"));

    tcp.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut reply = [0u8; 2];
    tcp.read_exact(&mut reply).await?;
    if reply != [0x05, 0x00] {
        return Err(fail("需要认证"));
    }

    let name = host.as_bytes();
    let len = u8::try_from(name.len()).map_err(|_| fail("主机名过长"))?;
    let mut request = Vec::with_capacity(7 + name.len());
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, len]);
    request.extend_from_slice(name);
    request.extend_from_slice(&port.to_be_bytes());
    tcp.write_all(&request).await?;

    let mut head = [0u8; 4];
    tcp.read_exact(&mut head).await?;
    if head[1] != 0x00 {
        return Err(fail(&format!("错误码 {}", head[1])));
    }
    // Skip the bound address: IPv4, domain (length-prefixed) or IPv6, then port.
    let addr_len = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => usize::from(tcp.read_u8().await?),
        _ => return Err(fail("地址类型未知")),
    };
    let mut rest = vec![0u8; addr_len + 2];
    tcp.read_exact(&mut rest).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::host_matches;

    // macOS proxy exception semantics: `*.x` covers x and its subdomains only.
    #[test]
    fn exception_patterns() {
        assert!(host_matches("stream.binance.com", "*.binance.com"));
        assert!(host_matches("binance.com", "*.binance.com"));
        assert!(!host_matches("notbinance.com", "*.binance.com"));
        assert!(host_matches("localhost", "localhost"));
        assert!(!host_matches("stream.binance.com", "127.0.0.0/8"));
    }
}
