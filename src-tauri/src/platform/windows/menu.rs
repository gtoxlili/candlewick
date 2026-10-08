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
        System::Threading::{GetCurrentProcessId, GetCurrentThreadId},
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, EVENT_SYSTEM_MENUPOPUPSTART,
                EnumThreadWindows, GetClassNameW, HMENU, InsertMenuItemW, IsWindowVisible,
                MENUITEMINFOW, MF_GRAYED, MF_SEPARATOR, MF_STRING, MFT_STRING, MIIM_BITMAP,
                MIIM_FTYPE, MIIM_ID, MIIM_STRING, MIIM_SUBMENU, SetMenuItemInfoW,
                SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WINEVENT_OUTOFCONTEXT,
            },
        },
    },
    core::{BOOL, HSTRING, PCWSTR, PWSTR, Result},
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
    i18n::Locale,
};

const SETTINGS: u32 = 1;
const QUIT: u32 = 2;
const STATUS: u32 = 3;
const UPDATE: u32 = 4;
const CHECK_UPDATE: u32 = 5;
/// The holdings row (whose submenu opens) and `View Holdings…` inside it.
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
    /// What its fixed items say.
    locale: Locale,
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
                locale: view.locale,
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
                let empty = HSTRING::from(escape(t!("tray.emptyWatchlist")));
                AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &empty)?;
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
                let title = escape(&t!("tray.update", version = version));
                AppendMenuW(menu, MF_STRING, UPDATE as usize, &HSTRING::from(title))?;
            } else {
                let title = HSTRING::from(escape(t!("tray.checkUpdate")));
                AppendMenuW(menu, MF_STRING, CHECK_UPDATE as usize, &title)?;
            }
            let settings = HSTRING::from(escape(t!("tray.settings")));
            AppendMenuW(menu, MF_STRING, SETTINGS as usize, &settings)?;
            AppendMenuW(menu, MF_STRING, QUIT as usize, &HSTRING::from(escape(t!("tray.quit"))))?;
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
                        let open = HSTRING::from(escape(t!("tray.viewHoldings")));
                        AppendMenuW(menu, MF_STRING, OPEN_HOLDINGS as usize, &open)?;
                    }
                    Slot::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?,
                    Slot::Header(section) => {
                        let title = HSTRING::from(escape(section.title()));
                        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &title)?;
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
    /// watchlist, holdings appearing, going or changing shape, or another
    /// language waits for the next time it opens.
    pub fn update(&mut self, view: &View, marks: &mut Marks) {
        let ids: Vec<String> = view.watchlist.iter().map(|instrument| instrument.id()).collect();
        if ids != self.ids
            || view.holdings.as_ref().map(bar::Holdings::shape) != self.shape
            || view.locale != self.locale
        {
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

/// Keeps the dropdown out of screen captures while it shows concealed
/// holdings (`View::conceal`). Windows creates a popup menu window (the
/// dropdown, then the submenu) each time one opens, so nothing can be marked
/// ahead of time: from `new` until dropped, each one this thread shows is
/// marked for the monitor only as the accessibility event announcing it
/// arrives, through the menu's own message loop.
///
/// The taskbar ticker can't be marked, being a child of Explorer's taskbar;
/// the bar masks the total instead (`bar::MASK`).
pub struct Shield(HWINEVENTHOOK);

impl Shield {
    pub fn new() -> Self {
        unsafe extern "system" fn shown(
            _: HWINEVENTHOOK,
            _: u32,
            window: HWND,
            _: i32,
            _: i32,
            _: u32,
            _: u32,
        ) {
            // SAFETY: a popup menu window of this thread, the hook's filter;
            // one already closed again just fails.
            if let Err(e) = unsafe { SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE) } {
                log::debug!("cannot keep a menu window out of captures: {e}");
            }
        }
        // SAFETY: an out-of-context hook on this thread's own events, called
        // back on this thread; `Drop` removes it.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_MENUPOPUPSTART,
                EVENT_SYSTEM_MENUPOPUPSTART,
                None,
                Some(shown),
                GetCurrentProcessId(),
                GetCurrentThreadId(),
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.is_invalid() {
            log::error!("cannot watch the dropdown open; it shows in captures");
        }
        Self(hook)
    }
}

impl Drop for Shield {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: our hook, set in `new`.
            unsafe {
                let _ = UnhookWinEvent(self.0);
            }
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
