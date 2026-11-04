//! The macOS system proxy, the way Safari follows it: the HTTPS proxy, else
//! SOCKS, honoring the exceptions list.

use objc2_core_foundation::{CFArray, CFNumber, CFString, CFType};
use objc2_system_configuration::{
    SCDynamicStore, kSCPropNetProxiesExceptionsList, kSCPropNetProxiesHTTPSEnable,
    kSCPropNetProxiesHTTPSPort, kSCPropNetProxiesHTTPSProxy, kSCPropNetProxiesSOCKSEnable,
    kSCPropNetProxiesSOCKSPort, kSCPropNetProxiesSOCKSProxy,
};

use crate::net::Route;

/// Reads the current system proxy settings (they can change at any time, so
/// this runs on every connection attempt).
pub fn route(target: &str) -> Route {
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
