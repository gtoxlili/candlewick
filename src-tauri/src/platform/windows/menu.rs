//! The dropdown: a native popup menu of the watchlist, like the macOS one. A
//! row per entry (its price and change right-aligned in a column of their
//! own, a small colored triangle for the direction), a status line while a
//! feed isn't live, then settings and quit. It stays live while open: rows
//! update in place as prices tick.

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        Graphics::Gdi::{DeleteObject, HBITMAP, InvalidateRect},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, DestroyMenu, EnumThreadWindows, GetClassNameW, HMENU,
            InsertMenuItemW, IsWindowVisible, MENUITEMINFOW, MF_GRAYED, MF_SEPARATOR, MF_STRING,
            MFT_STRING, MIIM_BITMAP, MIIM_FTYPE, MIIM_ID, MIIM_STRING, SetMenuItemInfoW,
        },
    },
    core::{BOOL, HSTRING, PCWSTR, PWSTR, Result, w},
};

use super::{
    draw,
    portable::{
        color::{Palette, Tone},
        glyph,
        menu_text::{self, escape},
    },
};
use crate::{
    bar::{self, Action, Hue, Row, View},
    format::Direction,
};

const SETTINGS: u32 = 1;
const QUIT: u32 = 2;
const STATUS: u32 = 3;
/// Row n has this id plus n.
const FIRST_ROW: u32 = 100;

/// An open dropdown.
pub struct Open {
    pub menu: HMENU,
    /// The instrument of each row.
    ids: Vec<String>,
    /// The text and mark each row shows.
    shown: Vec<(String, Option<HBITMAP>)>,
}

impl Open {
    pub fn build(view: &View, marks: &mut Marks) -> Result<Self> {
        // SAFETY: a fresh menu, filled with our own items; `Drop` destroys it.
        unsafe {
            let menu = CreatePopupMenu()?;
            let mut open = Self { menu, ids: Vec::new(), shown: Vec::new() };
            if view.watchlist.is_empty() {
                AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("在设置中添加自选"))?;
            } else {
                for (index, (text, row)) in
                    menu_text::rows(&view.rows).into_iter().zip(&view.rows).enumerate()
                {
                    let mark = marks.for_row(row, view);
                    let mut wide = wide(&text);
                    let item = MENUITEMINFOW {
                        cbSize: size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_ID | MIIM_FTYPE | MIIM_STRING | MIIM_BITMAP,
                        fType: MFT_STRING,
                        wID: FIRST_ROW + index as u32,
                        dwTypeData: PWSTR(wide.as_mut_ptr()),
                        hbmpItem: mark.unwrap_or_default(),
                        ..Default::default()
                    };
                    InsertMenuItemW(menu, index as u32, true, &item)?;
                    open.shown.push((text, mark));
                }
                open.ids = view.watchlist.iter().map(|instrument| instrument.id()).collect();
                if !view.caption.is_empty() {
                    AppendMenuW(
                        menu,
                        MF_STRING | MF_GRAYED,
                        STATUS as usize,
                        &HSTRING::from(escape(&view.caption)),
                    )?;
                }
            }
            AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
            AppendMenuW(menu, MF_STRING, SETTINGS as usize, w!("设置…"))?;
            AppendMenuW(menu, MF_STRING, QUIT as usize, w!("退出 Candlewick"))?;
            Ok(open)
        }
    }

    /// Brings the open menu's rows up to date and repaints it. A changed
    /// watchlist waits for the next time it opens.
    pub fn update(&mut self, view: &View, marks: &mut Marks) {
        let ids: Vec<String> = view.watchlist.iter().map(|instrument| instrument.id()).collect();
        if ids != self.ids {
            return;
        }
        let mut changed = false;
        for (index, (text, row)) in
            menu_text::rows(&view.rows).into_iter().zip(&view.rows).enumerate()
        {
            let mark = marks.for_row(row, view);
            if self.shown.get(index) == Some(&(text.clone(), mark)) {
                continue;
            }
            let mut wide = wide(&text);
            let item = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_STRING | MIIM_BITMAP,
                dwTypeData: PWSTR(wide.as_mut_ptr()),
                hbmpItem: mark.unwrap_or_default(),
                ..Default::default()
            };
            // SAFETY: an item of our open menu; the string outlives the call.
            if unsafe { SetMenuItemInfoW(self.menu, FIRST_ROW + index as u32, false, &item) }
                .is_ok()
            {
                self.shown[index] = (text, mark);
                changed = true;
            }
        }
        if changed && let Some(window) = menu_window() {
            // SAFETY: repaints the menu window of this thread.
            unsafe {
                let _ = InvalidateRect(Some(window), None, false);
            }
        }
    }

    /// What the chosen item asks for; 0 is "nothing chosen".
    pub fn action(&self, command: u32) -> Option<Action> {
        match command {
            SETTINGS => Some(Action::Settings),
            QUIT => Some(Action::Quit),
            id if id >= FIRST_ROW => {
                self.ids.get((id - FIRST_ROW) as usize).cloned().map(Action::Chart)
            }
            _ => None,
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: our menu, closed by now. Its bitmaps belong to `Marks`.
        unsafe {
            let _ = DestroyMenu(self.menu);
        }
    }
}

/// The popup menu window (class `#32768`) this thread is showing.
fn menu_window() -> Option<HWND> {
    unsafe extern "system" fn find(window: HWND, found: LPARAM) -> BOOL {
        let mut class = [0u16; 16];
        // SAFETY: fills our buffer with the window's class name.
        let length = unsafe { GetClassNameW(window, &mut class) } as usize;
        // SAFETY: `found` points at the caller's `Option<HWND>`.
        if &class[..length] == "#32768".encode_utf16().collect::<Vec<_>>().as_slice()
            && unsafe { IsWindowVisible(window) }.as_bool()
        {
            unsafe { *(found.0 as *mut Option<HWND>) = Some(window) };
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut found: Option<HWND> = None;
    // SAFETY: the callback writes only to `found`, which outlives the call.
    unsafe {
        let _ =
            EnumThreadWindows(GetCurrentThreadId(), Some(find), LPARAM((&raw mut found) as isize));
    }
    found
}

/// The direction marks, drawn once per size and mode and shared by rows.
#[derive(Default)]
pub struct Marks {
    key: Option<(u32, Tone, Palette)>,
    bitmaps: Vec<((bool, Hue), HBITMAP)>,
}

impl Marks {
    /// Starts drawing at `size` pixels in `tone`'s colors, dropping the
    /// bitmaps of another size or mode. Only call while no menu is open.
    pub fn prepare(&mut self, size: u32, tone: Tone, palette: Palette) {
        if self.key != Some((size, tone, palette)) {
            self.clear();
            self.key = Some((size, tone, palette));
        }
    }

    fn for_row(&mut self, row: &Row, view: &View) -> Option<HBITMAP> {
        let (_, direction) = row.change.as_ref()?;
        let hue = bar::hue(*direction, view.scheme)?;
        let up = *direction == Direction::Up;
        if let Some((_, bitmap)) = self.bitmaps.iter().find(|(key, _)| *key == (up, hue)) {
            return Some(*bitmap);
        }
        let (size, _, palette) = self.key?;
        match draw::menu_bitmap(&glyph::trend_mark(size, up, palette.hue(hue))) {
            Ok(bitmap) => {
                self.bitmaps.push(((up, hue), bitmap));
                Some(bitmap)
            }
            Err(e) => {
                log::warn!("cannot draw a menu mark: {e}");
                None
            }
        }
    }

    pub fn clear(&mut self) {
        for (_, bitmap) in self.bitmaps.drain(..) {
            // SAFETY: bitmaps we created, in no open menu.
            unsafe {
                let _ = DeleteObject(bitmap.into());
            }
        }
    }
}

impl Drop for Marks {
    fn drop(&mut self) {
        self.clear();
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
