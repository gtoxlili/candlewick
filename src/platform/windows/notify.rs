//! The tray icon: the app's price-line glyph drawn at the exact size the
//! notification area asks for, in the taskbar's ink, its dot taking the
//! pinned entry's trend color. Registered with version 4 of the protocol, so
//! keyboard users (Win+B, arrows, Enter) can open the dropdown too.

use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::{
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
            NIM_SETVERSION, NOTIFY_ICON_DATA_FLAGS, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
            NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
        },
        WindowsAndMessaging::{DestroyIcon, HICON},
    },
};

use super::{
    draw,
    portable::{color::Rgba, glyph},
    shell,
};

const ID: u32 = 1;

/// What the icon looks like; it is redrawn only when this changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub size: u32,
    pub ink: Rgba,
    pub dot: Rgba,
}

pub struct NotifyIcon {
    owner: HWND,
    added: bool,
    icon: Option<HICON>,
    look: Option<Look>,
    tip: String,
}

impl NotifyIcon {
    pub fn new(owner: HWND) -> Self {
        Self { owner, added: false, icon: None, look: None, tip: "Candlewick".to_owned() }
    }

    /// Adds the icon, again after Explorer restarts. Before the taskbar
    /// exists (an early start at login) this fails until TaskbarCreated.
    pub fn add(&mut self) {
        let mut data = self.data(NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        // SAFETY: a fully initialized NOTIFYICONDATAW for our window.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            self.added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
            if self.added {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
    }

    pub fn added(&self) -> bool {
        self.added
    }

    pub fn update(&mut self, look: Look, tip: &str) {
        let mut flags = NOTIFY_ICON_DATA_FLAGS(0);
        if self.look != Some(look) {
            match draw::icon(&glyph::tray(look.size, look.ink, look.dot)) {
                Ok(icon) => {
                    if let Some(old) = self.icon.replace(icon) {
                        // SAFETY: an icon we created; the shell copied it.
                        let _ = unsafe { DestroyIcon(old) };
                    }
                    self.look = Some(look);
                    flags |= NIF_ICON;
                }
                Err(e) => log::warn!("cannot draw the tray icon: {e}"),
            }
        }
        if self.tip != tip {
            self.tip = tip.to_owned();
            flags |= NIF_TIP | NIF_SHOWTIP;
        }
        if flags.0 == 0 || !self.added {
            return;
        }
        // The tooltip goes along every time: under version 4 a modify without
        // NIF_SHOWTIP may fall back to no standard tooltip.
        let data = self.data(flags | NIF_TIP | NIF_SHOWTIP);
        // SAFETY: as in `add`.
        if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            // Explorer went away without saying; TaskbarCreated brings it back.
            self.added = false;
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            let data = self.data(NOTIFY_ICON_DATA_FLAGS(0));
            // SAFETY: as in `add`.
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            self.added = false;
        }
        if let Some(icon) = self.icon.take() {
            // SAFETY: an icon we created.
            let _ = unsafe { DestroyIcon(icon) };
        }
        self.look = None;
    }

    /// The icon's bounds on screen, to open the dropdown beside it.
    pub fn rect(&self) -> Option<RECT> {
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.owner,
            uID: ID,
            ..Default::default()
        };
        // SAFETY: identifies our own icon.
        unsafe { Shell_NotifyIconGetRect(&identifier) }.ok()
    }

    fn data(&self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        // SAFETY: an all-zero NOTIFYICONDATAW is valid (null handles, empty
        // strings); the fields that matter are set below.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.owner;
        data.uID = ID;
        data.uFlags = flags;
        data.uCallbackMessage = shell::WM_ICON;
        data.hIcon = self.icon.unwrap_or_default();
        // At most 127 units and a terminating null, never half a pair; built
        // apart and stored whole, as the struct is packed on 32-bit x86.
        let mut tip = [0u16; 128];
        let mut length = 0;
        for (slot, unit) in tip.iter_mut().zip(self.tip.encode_utf16().take(127)) {
            *slot = unit;
            length += 1;
        }
        if length > 0 && (0xD800..0xDC00).contains(&tip[length - 1]) {
            tip[length - 1] = 0;
        }
        data.szTip = tip;
        data
    }
}
