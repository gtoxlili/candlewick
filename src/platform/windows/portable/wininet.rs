//! The Windows proxy settings (Settings → Network → Proxy, which is what
//! Clash Verge, v2rayN and similar tools set), read the way browsers read
//! them for an `https://` or `wss://` address.
//!
//! The server string is either one `host:port` for every protocol, or
//! `scheme=host:port` entries separated by `;`. The bypass list holds
//! `;`-separated patterns with `*` wildcards, plus `<local>` for host names
//! without a dot.

use crate::net::Route;

/// The route to `target` (a host dialed on port 443) under the manual proxy
/// `server` and its `bypass` list: the HTTPS proxy, else the one for every
/// protocol, else SOCKS.
pub fn route(target: &str, server: &str, bypass: Option<&str>) -> Route {
    if bypass.is_some_and(|list| bypassed(target, list)) {
        return Route::Direct;
    }
    let mut https = None;
    let mut every = None;
    let mut socks = None;
    for entry in server.split([';', ' ', '\t', '\n', '\r']).filter(|e| !e.is_empty()) {
        match entry.split_once('=') {
            Some((scheme, address)) => match scheme.trim().to_ascii_lowercase().as_str() {
                "https" => https = https.or(Some(address)),
                "socks" | "socks5" => socks = socks.or(Some(address)),
                _ => {}
            },
            None => every = every.or(Some(entry)),
        }
    }
    if let Some(route) = https.and_then(|address| proxy(address, false)) {
        return route;
    }
    if let Some(route) = every.and_then(|address| proxy(address, false)) {
        return route;
    }
    socks.and_then(|address| proxy(address, true)).unwrap_or(Route::Direct)
}

/// `host:port`, maybe with a scheme that says which kind of proxy it is.
fn proxy(address: &str, socks: bool) -> Option<Route> {
    let address = address.trim();
    let (socks, address) = match address.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase().starts_with("socks"), rest),
        None => (socks, address),
    };
    let address = address.trim_end_matches('/');
    let (host, port) = split_port(address);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return None;
    }
    let port = match port {
        Some(port) => port.parse().ok().filter(|p| *p != 0)?,
        None if socks => 1080,
        None => 80,
    };
    let host = host.to_owned();
    Some(if socks { Route::Socks5 { host, port } } else { Route::Http { host, port } })
}

/// Splits `host:port`, leaving an IPv6 literal's colons alone.
fn split_port(address: &str) -> (&str, Option<&str>) {
    if let Some(rest) = address.strip_prefix('[') {
        return match rest.split_once(']') {
            Some((host, tail)) => (host, tail.strip_prefix(':')),
            None => (address, None),
        };
    }
    match address.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, Some(port)),
        _ => (address, None),
    }
}

/// Whether the bypass `list` exempts `host` (dialed on port 443).
fn bypassed(host: &str, list: &str) -> bool {
    list.split([';', ',', ' ', '\t', '\n', '\r']).filter(|p| !p.is_empty()).any(|pattern| {
        if pattern.eq_ignore_ascii_case("<local>") {
            return !host.contains('.');
        }
        let pattern = pattern.split_once("://").map_or(pattern, |(_, rest)| rest);
        let pattern = match split_port(pattern) {
            (host_part, Some(port)) if port.chars().all(|c| c.is_ascii_digit()) => {
                if port != "443" {
                    return false;
                }
                host_part
            }
            _ => pattern,
        };
        glob(&pattern.to_ascii_lowercase(), &host.to_ascii_lowercase())
    })
}

/// `*` matches any run of characters, dots included.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((star_p, star_t)) = star {
            pi = star_p + 1;
            ti = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(host: &str, port: u16) -> Route {
        Route::Http { host: host.to_owned(), port }
    }

    // What proxy tools write: Clash Verge and v2rayN one address for every
    // protocol; others per scheme. An https= entry wins, as in WinINet.
    #[test]
    fn server_strings() {
        assert_eq!(route("api.binance.com", "127.0.0.1:7897", None), http("127.0.0.1", 7897));
        assert_eq!(
            route(
                "api.binance.com",
                "http=127.0.0.1:1;https=127.0.0.1:7890;socks=127.0.0.1:7891",
                None
            ),
            http("127.0.0.1", 7890)
        );
        assert_eq!(
            route("api.binance.com", "socks=127.0.0.1:1080", None),
            Route::Socks5 { host: "127.0.0.1".to_owned(), port: 1080 }
        );
        assert_eq!(
            route("api.binance.com", "http://proxy.lan:8080", None),
            http("proxy.lan", 8080)
        );
        assert_eq!(route("api.binance.com", "[::1]:7890", None), http("::1", 7890));
        // Only a plain-http proxy: WinINet goes direct for https.
        assert_eq!(route("api.binance.com", "http=127.0.0.1:7890", None), Route::Direct);
    }

    // ProxyOverride semantics: `*` wildcards anywhere, `<local>` for dotless
    // names, case-insensitive; entries for other ports don't apply to 443.
    #[test]
    fn bypass_lists() {
        let list = "localhost;127.*;10.*;*.binance.vision;<local>";
        assert!(bypassed("data-api.binance.vision", list));
        assert!(!bypassed("api.binance.com", list));
        assert!(bypassed("intranet", list));
        assert!(bypassed("LOCALHOST", list));
        assert!(bypassed("openapi.longbridge.cn", "*longbridge*"));
        assert!(!bypassed("api.binance.com", "api.binance.com:80"));
        assert!(bypassed("api.binance.com", "https://api.binance.com:443"));
        assert_eq!(route("api.binance.com", "127.0.0.1:7890", Some("*.com")), Route::Direct);
    }
}
