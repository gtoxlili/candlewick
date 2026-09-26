//! Dark popup menus. Windows has no public switch for them: its own apps and
//! the major UI frameworks call uxtheme functions exported only by ordinal,
//! on the builds that have them (Windows 10 1809 and later), as this does.
//! tao has already opted the process in (`SetPreferredAppMode(AllowDark)`);
//! the menu's owner window must opt in too, and menus cache their theme
//! until flushed.

use std::sync::OnceLock;

use windows::{
    Win32::{
        Foundation::HWND,
        System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
    },
    core::{PCSTR, w},
};

type AllowDarkModeForWindow = unsafe extern "system" fn(HWND, bool) -> bool;
type Procedure = unsafe extern "system" fn();

struct Uxtheme {
    allow_dark_mode_for_window: AllowDarkModeForWindow,
    refresh_immersive_color_policy_state: Procedure,
    flush_menu_themes: Procedure,
}

fn uxtheme() -> Option<&'static Uxtheme> {
    static UXTHEME: OnceLock<Option<Uxtheme>> = OnceLock::new();
    UXTHEME
        .get_or_init(|| {
            if windows_version::OsVersion::current().build < 17763 {
                return None;
            }
            // SAFETY: uxtheme from System32, never unloaded; the ordinals
            // resolve to functions of exactly these signatures on these builds.
            unsafe {
                let module =
                    LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
                let ordinal = |n: usize| GetProcAddress(module, PCSTR(n as *const u8));
                Some(Uxtheme {
                    allow_dark_mode_for_window: std::mem::transmute::<_, AllowDarkModeForWindow>(
                        ordinal(133)?,
                    ),
                    refresh_immersive_color_policy_state: std::mem::transmute::<_, Procedure>(
                        ordinal(104)?,
                    ),
                    flush_menu_themes: std::mem::transmute::<_, Procedure>(ordinal(136)?),
                })
            }
        })
        .as_ref()
}

/// Lets menus owned by `window` follow the dark app mode.
pub fn allow(window: HWND) {
    if let Some(uxtheme) = uxtheme() {
        // SAFETY: a window of this process.
        unsafe { (uxtheme.allow_dark_mode_for_window)(window, true) };
    }
}

/// After the app mode changed: menus drop their cached theme.
pub fn refresh() {
    if let Some(uxtheme) = uxtheme() {
        // SAFETY: argument-free uxtheme calls.
        unsafe {
            (uxtheme.refresh_immersive_color_policy_state)();
            (uxtheme.flush_menu_themes)();
        }
    }
}
