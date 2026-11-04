//! Everything that differs between macOS and Windows, behind one set of
//! names: the system bar (`bar`), window chrome (`window`), the system proxy
//! (`proxy`), login items, pauses and opening links. Each system's module
//! provides the same items; callers never branch on the platform.

#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("Candlewick runs on macOS and Windows.");

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

// The Windows code that makes no system calls, built for tests everywhere.
#[cfg(all(test, not(windows)))]
#[allow(dead_code)]
#[path = "windows/portable/mod.rs"]
mod windows_portable;

/// Why streaming is paused: in each case nobody can see the bar.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum Pause {
    SystemSleep = 1 << 0,
    DisplaySleep = 1 << 1,
    /// Another user's session is on screen (fast user switching), or, on
    /// Windows, the session is locked.
    SessionInactive = 1 << 2,
}
