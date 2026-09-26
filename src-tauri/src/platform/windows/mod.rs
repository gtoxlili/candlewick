//! Win32 glue. The taskbar presence (tray icon, ticker, dropdown) and the
//! system notifications it listens for live on one thread of their own
//! (`shell`); the rest are plain calls: login items, links, the proxy and
//! window chrome.

pub mod bar;
mod dark;
mod draw;
mod menu;
mod notify;
mod portable;
pub mod proxy;
mod shell;
mod theme;
mod ticker;
pub mod window;

use std::time::Duration;

use tauri::{Builder, Wry};
use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, LPARAM, WPARAM},
        System::{
            Memory::{
                GetProcessHeap, HEAP_FLAGS, HeapCompact, SETPROCESSWORKINGSETSIZEEX_FLAGS,
                SetProcessWorkingSetSizeEx,
            },
            Registry::{
                HKEY_CURRENT_USER, REG_BINARY, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
                RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
            },
            Threading::{
                CreateMutexW, GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
                ProcessPowerThrottling, SetProcessInformation,
            },
        },
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
                SW_SHOWNORMAL,
            },
        },
    },
    core::{HSTRING, PCWSTR, w},
};

use super::Pause;

/// Whether the first launch from an install location opens the settings
/// window. Windows 11 puts a new app's tray icon in the overflow, so that
/// launch shows the app is running.
pub const SETTINGS_ON_FIRST_RUN: bool = true;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
/// Where Task Manager and Settings → Apps → Startup record a user's choice.
const APPROVED_KEY: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
const RUN_VALUE: PCWSTR = w!("Candlewick");

/// One instance per session: a second launch asks the first to reopen (as
/// macOS does on a Dock click) and exits. Call before anything else.
pub fn claim_single_instance() {
    // SAFETY: a named mutex, kept for the life of the process.
    let claimed = unsafe { CreateMutexW(None, false, w!("Local\\com.influo.candlewick")) };
    // SAFETY: reads the calling thread's last error, set by CreateMutexW.
    let running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    match claimed {
        // Never closed: the mutex marks this instance for the life of the process.
        Ok(_) if !running => return,
        Ok(_) => {}
        // No mutex, no way to tell: run.
        Err(_) => return,
    }
    // The first instance may still be starting; its window shows up soon.
    for _ in 0..30 {
        // SAFETY: plain window lookups and a posted message.
        unsafe {
            if let Ok(window) = FindWindowW(shell::CLASS, PCWSTR::null()) {
                let mut pid = 0;
                GetWindowThreadProcessId(window, Some(&mut pid));
                // Lets the first instance bring its window forward: this
                // process has the user's launch as its foreground right.
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(window), shell::WM_REOPEN, WPARAM(0), LPARAM(0));
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::process::exit(0);
}

/// No app menu: Windows windows carry their own controls.
pub fn configure(builder: Builder<Wry>) -> Builder<Wry> {
    builder
}

/// Calls `on_change(reason, paused)` on sleep and resume, display off and on,
/// and session lock, unlock and switching; the shell thread watches for them.
pub fn observe_pauses(on_change: impl Fn(Pause, bool) + Send + Sync + 'static) {
    shell::on_pause(Box::new(on_change));
}

pub fn open_url(url: &str) {
    // SAFETY: a plain shell call; the URL is an https address checked by the caller.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success as a value above 32.
    if result.0 as usize <= 32 {
        log::warn!("cannot open {url}: ShellExecute returned {}", result.0 as usize);
    }
}

/// Registered under the Run key, and not turned off in Task Manager.
pub fn login_item_enabled() -> bool {
    // SAFETY: registry reads into local buffers.
    let registered = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None)
    }
    .is_ok();
    registered && startup_approved()
}

/// Task Manager keeps its own switch: an even first byte is on, an odd one
/// off (with the time it was turned off after it). No value: on.
fn startup_approved() -> bool {
    let mut data = [0u8; 64];
    let mut size = data.len() as u32;
    // SAFETY: `data` holds `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED_KEY,
            RUN_VALUE,
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    !status.is_ok() || size == 0 || data[0] & 1 == 0
}

/// Starts the app at login (the Run key), or stops it. Turning it on also
/// clears a "disabled" left in Task Manager, which would otherwise win.
pub fn set_login_item(enabled: bool) -> Result<(), String> {
    let fail = |e: windows::core::Error| format!("无法修改开机启动：{}", e.message());
    if enabled {
        let exe = std::env::current_exe().map_err(|e| format!("无法修改开机启动：{e}"))?;
        let command: Vec<u16> =
            format!("\"{}\"", exe.display()).encode_utf16().chain(std::iter::once(0)).collect();
        let approved = [2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        // SAFETY: each buffer holds the byte count passed with it.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                (command.len() * 2) as u32,
            )
            .ok()
            .map_err(fail)?;
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                APPROVED_KEY,
                RUN_VALUE,
                REG_BINARY.0,
                Some(approved.as_ptr().cast()),
                approved.len() as u32,
            )
            .ok()
            .map_err(fail)?;
        }
    } else {
        for key in [RUN_KEY, APPROVED_KEY] {
            // SAFETY: deletes a value of the app's own name.
            let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key, RUN_VALUE) };
            if status != ERROR_FILE_NOT_FOUND {
                status.ok().map_err(fail)?;
            }
        }
    }
    Ok(())
}

/// Installed by the setup program, which leaves its uninstaller beside the
/// app. A copy run from Downloads or a zip would register a path that goes
/// away, so first-launch registration waits for an install.
pub fn is_installed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("uninstall.exe")))
        .is_some_and(|uninstaller| uninstaller.is_file())
}

/// Efficiency mode (EcoQoS) while no window is open: the ticker and its feeds
/// run on efficient cores at low clock speeds, as background work should.
fn set_efficiency_mode(on: bool) {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: if on { PROCESS_POWER_THROTTLING_EXECUTION_SPEED } else { 0 },
    };
    // SAFETY: the struct and its size, for the current process.
    let result = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    if let Err(e) = result {
        log::debug!("efficiency mode unavailable: {e}");
    }
}

/// After the last window: WebView2's processes exit on their own; hand the
/// pages this process freed back, so its footprint returns to its baseline.
fn release_free_memory() {
    // SAFETY: compacts this process's default heap and trims its working set
    // (-1, -1 means "as much as possible"); both are advisory.
    unsafe {
        if let Ok(heap) = GetProcessHeap() {
            let _ = HeapCompact(heap, HEAP_FLAGS(0));
        }
        let _ = SetProcessWorkingSetSizeEx(
            GetCurrentProcess(),
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        );
    }
}
