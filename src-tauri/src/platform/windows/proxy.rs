//! The Windows system proxy: the manual proxy in the user's Internet settings,
//! read through WinHTTP on every connection attempt (it can change at any
//! time). Automatic configuration scripts are not evaluated, as on macOS.

use windows::{
    Win32::{
        Foundation::{GlobalFree, HGLOBAL},
        Networking::WinHttp::{
            WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WinHttpGetIEProxyConfigForCurrentUser,
        },
    },
    core::PWSTR,
};

use super::portable::wininet;
use crate::net::Route;

pub fn route(target: &str) -> Route {
    let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    // SAFETY: WinHTTP fills the struct with strings it allocated, which `take`
    // copies and frees.
    let (server, bypass) = unsafe {
        if WinHttpGetIEProxyConfigForCurrentUser(&mut config).is_err() {
            return Route::Direct;
        }
        let _ = take(config.lpszAutoConfigUrl);
        (take(config.lpszProxy), take(config.lpszProxyBypass))
    };
    // No server string: the manual proxy is off.
    match server {
        Some(server) => wininet::route(target, &server, bypass.as_deref()),
        None => Route::Direct,
    }
}

/// # Safety
/// `text` is null or a string WinHTTP allocated with GlobalAlloc.
unsafe fn take(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: a live, null-terminated string owned by us from here on.
    unsafe {
        let value = text.to_string().ok();
        let _ = GlobalFree(Some(HGLOBAL(text.0.cast())));
        value.filter(|value| !value.trim().is_empty())
    }
}
