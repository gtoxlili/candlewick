//! The menu bar status item and its native dropdown.
//!
//! Everything here runs on the main thread: `bar::request_render` schedules
//! one pass there per burst of updates, and that pass only touches AppKit for
//! strings that changed.

use std::cell::RefCell;

use objc2::{AnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular,
    NSForegroundColorAttributeName, NSMenu, NSMutableParagraphStyle, NSParagraphStyleAttributeName,
    NSStatusItem, NSTextAlignment, NSTextTab,
};
use objc2_foundation::{NSArray, NSDictionary, NSMutableAttributedString, NSRange, NSString};
use tauri::{
    ActivationPolicy, AppHandle, Wry,
    image::Image,
    menu::MenuEvent,
    tray::{MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};
use tray_icon::menu::{ContextMenu, Menu, MenuItem, PredefinedMenuItem};

use super::ticker;
use crate::{
    bar::{self, Action, Row, Ticker, View},
    model::{ColorScheme, Instrument},
};

const ID_HOLDINGS: &str = "holdings";
const ID_SETTINGS: &str = "settings";
const ID_UPDATE: &str = "update";
const ID_CHECK_UPDATE: &str = "check-update";
const ID_QUIT: &str = "quit";
const INSTRUMENT_PREFIX: &str = "instrument:";

fn template_icon() -> Image<'static> {
    tauri::include_image!("icons/tray-template.png")
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

struct Ui {
    tray: TrayIcon<Wry>,
    native_tray: tray_icon::TrayIcon,
    /// The menu exists even when tray-icon detaches it from the status item.
    menu: Option<Retained<NSMenu>>,
    status_item: Option<Retained<NSStatusItem>>,
    fonts: Fonts,
    /// What the current menu was built for; a change rebuilds it. `None`
    /// until the first build, so even an empty list gets a menu.
    layout: Option<Layout>,
    /// Column widths in points. They only grow while the layout stays the
    /// same, so an open menu never shifts as prices tick.
    columns: Columns,
    tabs: Retained<NSMutableParagraphStyle>,
    scheme: ColorScheme,
    /// As applied to the menu: the holdings row, then the watchlist's.
    holdings: Option<Row>,
    rows: Vec<Row>,
    /// `None` until applied to the current menu.
    caption: Option<String>,
    /// What the status item shows: `None` is the template icon. `Some(None)`
    /// only before the first render, so that one always applies.
    ticker: Option<Option<Ticker>>,
}

struct Fonts {
    /// The menu font with tabular digits, so prices line up digit by digit.
    row: Retained<NSFont>,
    caption: Retained<NSFont>,
}

#[derive(Debug, Clone, PartialEq)]
struct Layout {
    watchlist: Vec<Instrument>,
    /// The offered update.
    update: Option<String>,
    /// A holdings row leads the menu, with a separator after it.
    holdings: bool,
}

impl Layout {
    /// The menu index of watchlist row `index`; the status caption follows
    /// the last row.
    fn row(&self, index: usize) -> isize {
        (index + if self.holdings { 2 } else { 0 }) as isize
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Columns {
    label: f64,
    price: f64,
    change: f64,
}

/// Creates the status item and makes this a menu bar app: no Dock icon or app
/// menu until a window opens. Must run on the main thread (Tauri's `setup`).
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    app.set_activation_policy(ActivationPolicy::Accessory)?;
    let tray = TrayIconBuilder::new()
        .icon(template_icon())
        .icon_as_template(true)
        .tooltip("Candlewick")
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        // The dropdown opens on the press.
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button_state: MouseButtonState::Down, .. } = event {
                bar::menu_opening(tray.app_handle());
            }
        })
        .build(app)?;

    // This callback runs inline on the main thread. Both handles refer to
    // the same tray; native menu ownership stays on this thread.
    let handle = tray.clone();
    tray.with_inner_tray_icon(move |inner| {
        let menu_font_size = NSFont::menuFontOfSize(0.0).pointSize();
        // SAFETY: reading an immutable AppKit constant.
        let regular = unsafe { NSFontWeightRegular };
        UI.set(Some(Ui {
            tray: handle,
            native_tray: inner.clone(),
            menu: None,
            status_item: inner.ns_status_item().inspect(|item| use_tabular_digits(item)),
            fonts: Fonts {
                row: NSFont::monospacedDigitSystemFontOfSize_weight(menu_font_size, regular),
                caption: NSFont::menuFontOfSize(NSFont::smallSystemFontSize()),
            },
            layout: None,
            columns: Columns::default(),
            tabs: NSMutableParagraphStyle::new(),
            scheme: ColorScheme::default(),
            holdings: None,
            rows: Vec::new(),
            caption: None,
            ticker: None,
        }));
    })?;
    render(app);
    Ok(())
}

/// Queues a render on the main thread; false if the event loop is gone.
pub fn schedule_render(app: &AppHandle) -> bool {
    let handle = app.clone();
    app.run_on_main_thread(move || render(&handle)).is_ok()
}

/// Nothing to tear down: the status item goes with the process.
pub fn shutdown() {}

fn render(app: &AppHandle) {
    let view = bar::view(app);
    UI.with_borrow_mut(|ui| {
        if let Some(ui) = ui
            && let Err(e) = ui.apply(view)
        {
            log::error!("tray update failed: {e}");
        }
    });
}

impl Ui {
    fn apply(&mut self, view: View) -> tauri::Result<()> {
        let mtm = MainThreadMarker::new().expect("tray renders on the main thread");
        let layout = Layout {
            watchlist: view.watchlist,
            update: view.update,
            holdings: view.holdings.is_some(),
        };
        if self.layout.as_ref() != Some(&layout) {
            log::debug!("menu rebuilt for {} instruments", layout.watchlist.len());
            let menu = build_menu(&layout)?;
            // SAFETY: muda returns its live NSMenu on macOS. Retain it on
            // the main thread before transferring the menu to tray-icon.
            self.menu = unsafe { Retained::retain(menu.ns_menu().cast::<NSMenu>()) };
            self.native_tray.set_menu(Some(Box::new(menu)));
            self.layout = Some(layout);
            self.columns = Columns::default();
            self.holdings = None;
            self.rows.clear();
            self.caption = None;
        }
        let menu = self.menu.clone().expect("the tray menu has been built");
        let layout = self.layout.clone().expect("the tray menu has been built");

        let mut columns = self.columns;
        if let Some(row) = &view.holdings
            && self.holdings.as_ref() != Some(row)
        {
            columns = columns.fit(row, &self.fonts.row);
        }
        for (index, row) in view.rows.iter().enumerate() {
            if self.rows.get(index) != Some(row) {
                columns = columns.fit(row, &self.fonts.row);
            }
        }
        let scheme_changed = self.scheme != view.scheme;
        let mut restyle_all = scheme_changed;
        if columns != self.columns {
            self.columns = columns;
            self.tabs = tab_stops(columns, self.fonts.row.pointSize());
            restyle_all = true;
        }
        self.scheme = view.scheme;
        if let Some(row) = &view.holdings
            && (restyle_all || self.holdings.as_ref() != Some(row))
            && let Some(item) = menu.itemAtIndex(0)
        {
            item.setAttributedTitle(Some(&self.styled_row(row)));
        }
        self.holdings = view.holdings;
        for (index, row) in view.rows.iter().enumerate() {
            if !restyle_all && self.rows.get(index) == Some(row) {
                continue;
            }
            if let Some(item) = menu.itemAtIndex(layout.row(index)) {
                log::trace!("row {index}: {row:?}");
                item.setAttributedTitle(Some(&self.styled_row(row)));
            }
        }
        self.rows = view.rows;

        let count = layout.watchlist.len();
        if count > 0 && self.caption.as_ref() != Some(&view.caption) {
            log::debug!("caption: {:?}", view.caption);
            set_caption(&menu, layout.row(count), &view.caption, &self.fonts.caption);
            self.caption = Some(view.caption);
        }

        // The drawn block also carries the trend colors.
        let redraw = scheme_changed && view.ticker.as_ref().is_some_and(|t| t.two_rows);
        if redraw || self.ticker.as_ref() != Some(&view.ticker) {
            self.show(view.ticker.as_ref(), mtm)?;
            self.ticker = Some(view.ticker);
        }
        Ok(())
    }

    fn show(&self, ticker: Option<&Ticker>, mtm: MainThreadMarker) -> tauri::Result<()> {
        match ticker {
            None => {
                self.tray.set_title(Some(""))?;
                self.tray.set_icon_with_as_template(Some(template_icon()), true)?;
            }
            Some(ticker) if ticker.two_rows => {
                log::trace!("ticker: {ticker:?}");
                if let Some(button) = self.status_item.as_ref().and_then(|item| item.button(mtm)) {
                    button.setImage(Some(&ticker::image(ticker, self.scheme)));
                }
                // After the image: Tauri's setters also fit its click target
                // to the button, whose width the image just changed.
                self.tray.set_title(Some(""))?;
            }
            Some(ticker) => {
                log::trace!("ticker: {ticker:?}");
                // Coming from the icon or the drawn block: clear the image
                // (Tauri's call clears whatever the button holds).
                let was_text = matches!(&self.ticker, Some(Some(shown)) if !shown.two_rows);
                if !was_text {
                    self.tray.set_icon(None)?;
                }
                self.tray.set_title(Some(&ticker.line()))?;
            }
        }
        Ok(())
    }

    /// `BTC/USDT ⇥ 84,002.01 ⇥ −0.24%` with right-aligned tab stops.
    fn styled_row(&self, row: &Row) -> Retained<NSMutableAttributedString> {
        let mut text = format!("{}{}\t{}", row.name, row.detail, row.price);
        let mut session_at = None;
        if let Some((change, _)) = &row.change {
            text.push('\t');
            if let Some(session) = row.session {
                session_at = Some(utf16_len(&text));
                text.push_str(session);
                text.push(' ');
            }
            text.push_str(change);
        }
        let string = attributed(&text, &self.fonts.row);
        let dimmed = |at: usize, len: usize| {
            // SAFETY: NSColor for the foreground color key.
            unsafe {
                string.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    &NSColor::secondaryLabelColor(),
                    NSRange::new(at, len),
                );
            }
        };
        dimmed(utf16_len(&row.name), utf16_len(&row.detail));
        if let (Some(at), Some(session)) = (session_at, row.session) {
            dimmed(at, utf16_len(session));
        }
        // SAFETY: an NSParagraphStyle for the paragraph style key.
        unsafe {
            string.addAttribute_value_range(
                NSParagraphStyleAttributeName,
                &self.tabs,
                NSRange::new(0, utf16_len(&text)),
            );
        }
        if let Some((change, direction)) = &row.change
            && let Some(color) = ticker::trend_color(*direction, self.scheme)
        {
            let len = utf16_len(change);
            // SAFETY: NSColor for the foreground color key.
            unsafe {
                string.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    &color,
                    NSRange::new(utf16_len(&text) - len, len),
                );
            }
        }
        string
    }
}

impl Columns {
    fn fit(self, row: &Row, font: &NSFont) -> Self {
        let change = row.change.as_ref().map_or(0.0, |(change, _)| {
            let session = row.session.map(|s| format!("{s} ")).unwrap_or_default();
            text_width(&format!("{session}{change}"), font)
        });
        Self {
            label: self.label.max(text_width(&format!("{}{}", row.name, row.detail), font)),
            price: self.price.max(text_width(&row.price, font)),
            change: self.change.max(change),
        }
    }
}

/// Tabular digits keep the status item from changing width every tick.
fn use_tabular_digits(item: &NSStatusItem) {
    let Some(button) = MainThreadMarker::new().and_then(|mtm| item.button(mtm)) else {
        return;
    };
    let size = NSFont::menuBarFontOfSize(0.0).pointSize();
    // SAFETY: reading an immutable AppKit constant.
    let font = NSFont::monospacedDigitSystemFontOfSize_weight(size, unsafe { NSFontWeightRegular });
    button.setFont(Some(&font));
}

/// Right-aligned stops at the end of the price and change columns.
fn tab_stops(columns: Columns, font_size: f64) -> Retained<NSMutableParagraphStyle> {
    let price_end = columns.label + font_size * 2.0 + columns.price;
    let change_end = price_end + font_size * 1.2 + columns.change;
    let stops: Vec<Retained<NSTextTab>> = [price_end, change_end]
        .into_iter()
        .map(|location| {
            // SAFETY: an empty options dictionary is valid.
            unsafe {
                NSTextTab::initWithTextAlignment_location_options(
                    NSTextTab::alloc(),
                    NSTextAlignment::Right,
                    location.ceil(),
                    &NSDictionary::new(),
                )
            }
        })
        .collect();
    let style = NSMutableParagraphStyle::new();
    style.setTabStops(Some(&NSArray::from_retained_slice(&stops)));
    style
}

/// A small dimmed line under the watchlist rows, shown only when a feed is not
/// live (connecting, retrying, paused); hidden items take no space.
fn set_caption(menu: &NSMenu, index: isize, text: &str, font: &NSFont) {
    let Some(item) = menu.itemAtIndex(index) else {
        return;
    };
    item.setHidden(text.is_empty());
    if text.is_empty() {
        return;
    }
    let string = attributed(text, font);
    // SAFETY: NSColor for the foreground color key.
    unsafe {
        string.addAttribute_value_range(
            NSForegroundColorAttributeName,
            &NSColor::secondaryLabelColor(),
            NSRange::new(0, utf16_len(text)),
        );
    }
    item.setAttributedTitle(Some(&string));
}

fn attributed(text: &str, font: &NSFont) -> Retained<NSMutableAttributedString> {
    let string = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &NSString::from_str(text),
    );
    // SAFETY: NSFont for the font key.
    unsafe {
        string.addAttribute_value_range(
            NSFontAttributeName,
            font,
            NSRange::new(0, utf16_len(text)),
        );
    }
    string
}

fn text_width(text: &str, font: &NSFont) -> f64 {
    attributed(text, font).size().width
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn build_menu(layout: &Layout) -> tauri::Result<Menu> {
    let Layout { watchlist, update, holdings } = layout;
    let menu = Menu::new();
    if *holdings {
        // Styled by `apply`, like the rows.
        menu.append(&MenuItem::with_id(ID_HOLDINGS, "总资产", true, None))?;
        menu.append(&PredefinedMenuItem::separator())?;
    }
    if watchlist.is_empty() {
        menu.append(&MenuItem::with_id("empty", "在设置中添加自选", false, None))?;
    } else {
        for instrument in watchlist {
            let id = format!("{INSTRUMENT_PREFIX}{}", instrument.id());
            menu.append(&MenuItem::with_id(id, instrument.pair_label(), true, None))?;
        }
        // Status caption, styled and shown/hidden natively by `set_caption`.
        menu.append(&MenuItem::with_id("status", "", false, None))?;
    }
    menu.append(&PredefinedMenuItem::separator())?;
    if let Some(version) = update {
        let title = format!("更新到 {version} 并重新启动");
        menu.append(&MenuItem::with_id(ID_UPDATE, title, true, None))?;
    } else {
        menu.append(&MenuItem::with_id(ID_CHECK_UPDATE, "检查更新…", true, None))?;
    }
    // No key equivalents: AppKit reserves a shortcut column on every row, which
    // would leave a wide empty band to the right of the prices.
    menu.append(&MenuItem::with_id(ID_SETTINGS, "设置…", true, None))?;
    menu.append(&MenuItem::with_id(ID_QUIT, "退出 Candlewick", true, None))?;
    Ok(menu)
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let action = match event.id().as_ref() {
        ID_HOLDINGS => Action::Holdings,
        ID_SETTINGS => Action::Settings,
        ID_UPDATE => Action::Update,
        ID_CHECK_UPDATE => Action::CheckUpdate,
        ID_QUIT => Action::Quit,
        id => match id.strip_prefix(INSTRUMENT_PREFIX) {
            Some(instrument) => Action::Chart(instrument.to_owned()),
            None => return,
        },
    };
    bar::perform(app, action);
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "bar_tests.rs"]
pub(crate) mod tests;
