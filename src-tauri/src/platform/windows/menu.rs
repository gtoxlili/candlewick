//! The dropdown: a native popup menu of the watchlist, like the macOS one. The
//! total holdings once an exchange has an API key, with a submenu of what
//! they are made of; a row per entry (its price and change right-aligned in
//! a column of their own, a small colored triangle for the direction), a
//! status line while a feed isn't live, then check for updates (or restart
//! into one), settings and quit. It stays live while open: rows update in
//! place as prices tick.

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        Graphics::Gdi::{DeleteObject, HBITMAP, InvalidateRect},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, DestroyMenu, EnumThreadWindows, GetClassNameW, HMENU,
            InsertMenuItemW, IsWindowVisible, MENUITEMINFOW, MF_GRAYED, MF_SEPARATOR, MF_STRING,
            MFT_STRING, MIIM_BITMAP, MIIM_FTYPE, MIIM_ID, MIIM_STRING, MIIM_SUBMENU,
            SetMenuItemInfoW,
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
    bar::{self, Action, Hue, Row, Shape, Slot, View},
    format::Direction,
};

const SETTINGS: u32 = 1;
const QUIT: u32 = 2;
const STATUS: u32 = 3;
const UPDATE: u32 = 4;
const CHECK_UPDATE: u32 = 5;
/// The holdings row (whose submenu opens) and `查看持仓…` inside it.
const HOLDINGS: u32 = 6;
const OPEN_HOLDINGS: u32 = 7;
/// Watchlist row n has this id plus n.
const FIRST_ROW: u32 = 100;
/// Row n of the holdings submenu has this id plus n; every one opens the
/// holdings window.
const FIRST_HOLDINGS_ROW: u32 = 200;

/// An open dropdown.
pub struct Open {
    pub menu: HMENU,
    /// The instrument of each watchlist row.
    ids: Vec<String>,
    /// The submenu it was built with, if any.
    shape: Option<Shape>,
    /// The command id, text and mark of each row shown, holdings first.
    shown: Vec<(u32, String, Option<HBITMAP>)>,
}

/// A row as the menu shows it.
struct Line<'a> {
    id: u32,
    row: &'a Row,
    text: String,
}

/// The rows laid out together, so their columns line up: the holdings
/// total with the watchlist (`priced`), and the submenu's rows (`sub`).
fn lines<'a>(ids: impl Iterator<Item = u32>, rows: Vec<&'a Row>) -> Vec<Line<'a>> {
    let texts = menu_text::rows(&rows.iter().map(|row| (*row).clone()).collect::<Vec<_>>());
    ids.zip(rows).zip(texts).map(|((id, row), text)| Line { id, row, text }).collect()
}

fn priced(view: &View) -> Vec<Line<'_>> {
    let rows: Vec<&Row> = view.holdings.iter().map(|h| &h.total).chain(view.rows.iter()).collect();
    let ids = view
        .holdings
        .iter()
        .map(|_| HOLDINGS)
        .chain((0..view.rows.len()).map(|index| FIRST_ROW + index as u32));
    lines(ids, rows)
}

fn sub(view: &View) -> Vec<Line<'_>> {
    let rows: Vec<&Row> = view
        .holdings
        .iter()
        .flat_map(|h| h.slots())
        .filter_map(|slot| match slot {
            Slot::Row(row) => Some(row),
            _ => None,
        })
        .collect();
    lines((0..rows.len()).map(|index| FIRST_HOLDINGS_ROW + index as u32), rows)
}

impl Open {
    pub fn build(view: &View, marks: &mut Marks) -> Result<Self> {
        // SAFETY: a fresh menu, filled with our own items; `Drop` destroys
        // it, submenu included.
        unsafe {
            let menu = CreatePopupMenu()?;
            let mut open = Self {
                menu,
                ids: Vec::new(),
                shape: view.holdings.as_ref().map(bar::Holdings::shape),
                shown: Vec::new(),
            };
            let mut position = 0;
            for line in priced(view) {
                let mark = marks.for_row(line.row, view);
                let submenu =
                    (line.id == HOLDINGS).then(|| open.submenu(view, marks)).transpose()?;
                let mut wide = wide(&line.text);
                let item = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_ID
                        | MIIM_FTYPE
                        | MIIM_STRING
                        | MIIM_BITMAP
                        | if submenu.is_some() { MIIM_SUBMENU } else { Default::default() },
                    fType: MFT_STRING,
                    wID: line.id,
                    hSubMenu: submenu.unwrap_or_default(),
                    dwTypeData: PWSTR(wide.as_mut_ptr()),
                    hbmpItem: mark.unwrap_or_default(),
                    ..Default::default()
                };
                InsertMenuItemW(menu, position, true, &item)?;
                position += 1;
                open.shown.push((line.id, line.text, mark));
                if line.id == HOLDINGS {
                    AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                    position += 1;
                }
            }
            if view.watchlist.is_empty() {
                AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("在设置中添加自选"))?;
            } else {
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
            if let Some(version) = &view.update {
                let title = escape(&format!("更新到 {version} 并重新启动"));
                AppendMenuW(menu, MF_STRING, UPDATE as usize, &HSTRING::from(title))?;
            } else {
                AppendMenuW(menu, MF_STRING, CHECK_UPDATE as usize, w!("检查更新…"))?;
            }
            AppendMenuW(menu, MF_STRING, SETTINGS as usize, w!("设置…"))?;
            AppendMenuW(menu, MF_STRING, QUIT as usize, w!("退出 Candlewick"))?;
            Ok(open)
        }
    }

    /// The holdings submenu: what [`bar::Holdings::slots`] lists, section
    /// titles as dimmed rows (a menu has no headers of its own).
    unsafe fn submenu(&mut self, view: &View, marks: &mut Marks) -> Result<HMENU> {
        // SAFETY: a fresh menu, filled with our own items; it goes with
        // its parent.
        unsafe {
            let menu = CreatePopupMenu()?;
            let mut rows = sub(view).into_iter();
            for slot in view.holdings.iter().flat_map(|h| h.slots()) {
                match slot {
                    Slot::Open => {
                        AppendMenuW(menu, MF_STRING, OPEN_HOLDINGS as usize, w!("查看持仓…"))?;
                    }
                    Slot::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?,
                    Slot::Header(title) => {
                        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(title))?;
                    }
                    Slot::Caption(text) => {
                        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(escape(text)))?;
                    }
                    Slot::Row(_) => {
                        let line = rows.next().expect("a line per row slot");
                        let mark = marks.for_row(line.row, view);
                        let mut wide = wide(&line.text);
                        let item = MENUITEMINFOW {
                            cbSize: size_of::<MENUITEMINFOW>() as u32,
                            fMask: MIIM_ID | MIIM_FTYPE | MIIM_STRING | MIIM_BITMAP,
                            fType: MFT_STRING,
                            wID: line.id,
                            dwTypeData: PWSTR(wide.as_mut_ptr()),
                            hbmpItem: mark.unwrap_or_default(),
                            ..Default::default()
                        };
                        InsertMenuItemW(menu, u32::MAX, true, &item)?;
                        self.shown.push((line.id, line.text, mark));
                    }
                }
            }
            Ok(menu)
        }
    }

    /// Brings the open menu's rows up to date and repaints it. A changed
    /// watchlist, or holdings appearing, going or changing shape, waits
    /// for the next time it opens.
    pub fn update(&mut self, view: &View, marks: &mut Marks) {
        let ids: Vec<String> = view.watchlist.iter().map(|instrument| instrument.id()).collect();
        if ids != self.ids || view.holdings.as_ref().map(bar::Holdings::shape) != self.shape {
            return;
        }
        let lines: Vec<Line> = priced(view).into_iter().chain(sub(view)).collect();
        if !lines.iter().map(|line| line.id).eq(self.shown.iter().map(|(id, ..)| *id)) {
            return;
        }
        let mut changed = false;
        for (index, line) in lines.into_iter().enumerate() {
            let mark = marks.for_row(line.row, view);
            if self.shown[index] == (line.id, line.text.clone(), mark) {
                continue;
            }
            let mut wide = wide(&line.text);
            let item = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_STRING | MIIM_BITMAP,
                dwTypeData: PWSTR(wide.as_mut_ptr()),
                hbmpItem: mark.unwrap_or_default(),
                ..Default::default()
            };
            // SAFETY: an item of our open menu (or its submenu, which a
            // lookup by id reaches); the string outlives the call.
            if unsafe { SetMenuItemInfoW(self.menu, line.id, false, &item) }.is_ok() {
                self.shown[index] = (line.id, line.text, mark);
                changed = true;
            }
        }
        if changed {
            for window in menu_windows() {
                // SAFETY: repaints a menu window of this thread.
                unsafe {
                    let _ = InvalidateRect(Some(window), None, false);
                }
            }
        }
    }

    /// What the chosen item asks for; 0 is "nothing chosen".
    pub fn action(&self, command: u32) -> Option<Action> {
        match command {
            HOLDINGS | OPEN_HOLDINGS => Some(Action::Holdings),
            SETTINGS => Some(Action::Settings),
            UPDATE => Some(Action::Update),
            CHECK_UPDATE => Some(Action::CheckUpdate),
            QUIT => Some(Action::Quit),
            id if id >= FIRST_HOLDINGS_ROW => Some(Action::Holdings),
            id if id >= FIRST_ROW => {
                self.ids.get((id - FIRST_ROW) as usize).cloned().map(Action::Chart)
            }
            _ => None,
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: our menu, closed by now; destroying it destroys its
        // submenu. Its bitmaps belong to `Marks`.
        unsafe {
            let _ = DestroyMenu(self.menu);
        }
    }
}

/// The popup menu windows (class `#32768`) this thread is showing: the
/// dropdown and, while it is open, the submenu.
fn menu_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(window: HWND, found: LPARAM) -> BOOL {
        let mut class = [0u16; 16];
        // SAFETY: fills our buffer with the window's class name.
        let length = unsafe { GetClassNameW(window, &mut class) } as usize;
        // SAFETY: `found` points at the caller's `Vec<HWND>`.
        if &class[..length] == "#32768".encode_utf16().collect::<Vec<_>>().as_slice()
            && unsafe { IsWindowVisible(window) }.as_bool()
        {
            unsafe { (*(found.0 as *mut Vec<HWND>)).push(window) };
        }
        BOOL(1)
    }
    let mut found: Vec<HWND> = Vec::new();
    // SAFETY: the callback writes only to `found`, which outlives the call.
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(collect),
            LPARAM((&raw mut found) as isize),
        );
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

    /// The mark for a row's move: its change, or else its signed value.
    fn for_row(&mut self, row: &Row, view: &View) -> Option<HBITMAP> {
        let direction = match &row.change {
            Some((_, direction)) => *direction,
            None => row.value.1,
        };
        let hue = bar::hue(direction, view.scheme)?;
        let up = direction == Direction::Up;
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
