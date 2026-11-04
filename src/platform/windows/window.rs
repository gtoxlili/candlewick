use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, ShowWindow},
};

use crate::app::App;

pub const ACTIVATION_RAISES_WINDOWS: bool = false;

pub fn init(_app: &App, _cx: &mut gpui_kit::App) {}
pub fn relabel(_cx: &mut gpui_kit::App) {}

pub fn will_show(_app: &App, _cx: &mut gpui_kit::App, _label: &'static str) {
    super::set_efficiency_mode(false);
}

fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return None };
    Some(HWND(handle.hwnd.get() as *mut _))
}

pub fn is_unminimized(window: &Window) -> bool {
    hwnd(window).is_none_or(|hwnd| {
        // SAFETY: the handle belongs to the live GPUI window.
        !unsafe { IsIconic(hwnd) }.as_bool()
    })
}

pub fn bring_to_front(window: &Window) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle belongs to the live GPUI window.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
    }
    window.activate_window();
}

pub fn did_close_all(_cx: &mut gpui_kit::App) {
    super::set_efficiency_mode(true);
}
