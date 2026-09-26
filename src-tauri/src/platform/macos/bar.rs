//! The menu bar status item and its native dropdown.
//!
//! Everything here runs on the main thread. Other threads call
//! [`request_render`], which collapses any burst of updates into one
//! main-thread pass; that pass only touches AppKit for strings that changed.

use std::{cell::RefCell, collections::HashMap, sync::atomic::Ordering};

use objc2::{AnyThread, MainThreadMarker, rc::Retained};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular,
    NSForegroundColorAttributeName, NSMenu, NSMutableParagraphStyle, NSParagraphStyleAttributeName,
    NSStatusItem, NSTextAlignment, NSTextTab,
};
use objc2_foundation::{NSArray, NSDictionary, NSMutableAttributedString, NSRange, NSString};
use tauri::{
    AppHandle, Manager, Wry,
    image::Image,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
};

use crate::{
    format::{self, Direction},
    macos,
    model::{ColorScheme, Instrument, Model, Quote, Session, Shared},
    ticker::{self, Ticker},
    window,
};

const ID_SETTINGS: &str = "settings";
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
    status_item: Option<Retained<NSStatusItem>>,
    fonts: Fonts,
    /// The watchlist the current menu was built for; a change rebuilds the
    /// menu. `None` until the first build, so even an empty list gets a menu.
    layout: Option<Vec<Instrument>>,
    /// Column widths in points. They only grow while the layout stays the
    /// same, so an open menu never shifts as prices tick.
    columns: Columns,
    tabs: Retained<NSMutableParagraphStyle>,
    scheme: ColorScheme,
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

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Columns {
    label: f64,
    price: f64,
    change: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Row {
    name: String,
    /// Shown dimmed after the name, e.g. `/USDT`.
    detail: String,
    price: String,
    /// Shown dimmed before the change, e.g. `盘后`.
    session: Option<&'static str>,
    change: Option<(String, Direction)>,
}

/// Creates the status item. Must run on the main thread (Tauri's `setup`).
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let tray = TrayIconBuilder::new()
        .icon(template_icon())
        .icon_as_template(true)
        .tooltip("Candlewick")
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;

    let menu_font_size = NSFont::menuFontOfSize(0.0).pointSize();
    // SAFETY: reading an immutable AppKit constant.
    let regular = unsafe { NSFontWeightRegular };
    UI.set(Some(Ui {
        tray: tray.clone(),
        status_item: None,
        fonts: Fonts {
            row: NSFont::monospacedDigitSystemFontOfSize_weight(menu_font_size, regular),
            caption: NSFont::menuFontOfSize(NSFont::smallSystemFontSize()),
        },
        layout: None,
        columns: Columns::default(),
        tabs: NSMutableParagraphStyle::new(),
        scheme: ColorScheme::default(),
        rows: Vec::new(),
        caption: None,
        ticker: None,
    }));
    // Runs inline: we are already on the main thread.
    tray.with_inner_tray_icon(|inner| {
        let item = inner.ns_status_item();
        if let Some(item) = &item {
            macos::use_tabular_digits(item);
        }
        UI.with_borrow_mut(|ui| {
            if let Some(ui) = ui {
                ui.status_item = item;
            }
        });
    })?;
    render(app);
    Ok(())
}

/// Schedules a redraw on the main thread unless one is already pending.
pub fn request_render(app: &AppHandle) {
    let shared = app.state::<Shared>();
    if shared.render_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let handle = app.clone();
    let scheduled = app.run_on_main_thread(move || {
        handle.state::<Shared>().render_pending.store(false, Ordering::Release);
        render(&handle);
    });
    if scheduled.is_err() {
        shared.render_pending.store(false, Ordering::Release);
    }
}

struct View {
    watchlist: Vec<Instrument>,
    rows: Vec<Row>,
    ticker: Option<Ticker>,
    caption: String,
    scheme: ColorScheme,
}

fn render(app: &AppHandle) {
    let view = {
        let shared = app.state::<Shared>();
        let model = shared.model();
        View {
            watchlist: model.settings.watchlist.clone(),
            rows: rows(&model.settings.watchlist, &model.quotes),
            ticker: ticker(&model),
            caption: caption(&model),
            scheme: model.settings.color_scheme,
        }
    };
    UI.with_borrow_mut(|ui| {
        if let Some(ui) = ui
            && let Err(e) = ui.apply(app, view)
        {
            log::error!("tray update failed: {e}");
        }
    });
}

impl Ui {
    fn apply(&mut self, app: &AppHandle, view: View) -> tauri::Result<()> {
        let mtm = MainThreadMarker::new().expect("tray renders on the main thread");
        if self.layout.as_ref() != Some(&view.watchlist) {
            log::debug!("menu rebuilt for {} instruments", view.watchlist.len());
            self.tray.set_menu(Some(build_menu(app, &view.watchlist)?))?;
            self.layout = Some(view.watchlist);
            self.columns = Columns::default();
            self.rows.clear();
            self.caption = None;
        }
        let Some(menu) = self.status_item.as_ref().and_then(|item| item.menu(mtm)) else {
            return Ok(());
        };

        let mut columns = self.columns;
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
        for (index, row) in view.rows.iter().enumerate() {
            if !restyle_all && self.rows.get(index) == Some(row) {
                continue;
            }
            if let Some(item) = menu.itemAtIndex(index as isize) {
                log::trace!("row {index}: {row:?}");
                item.setAttributedTitle(Some(&self.styled_row(row)));
            }
        }
        self.rows = view.rows;

        let count = self.layout.as_ref().map_or(0, Vec::len);
        if count > 0 && self.caption.as_ref() != Some(&view.caption) {
            log::debug!("caption: {:?}", view.caption);
            set_caption(&menu, count, &view.caption, &self.fonts.caption);
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
fn set_caption(menu: &NSMenu, index: usize, text: &str, font: &NSFont) {
    let Some(item) = menu.itemAtIndex(index as isize) else {
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

fn build_menu(app: &AppHandle, watchlist: &[Instrument]) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    if watchlist.is_empty() {
        menu.append(&MenuItem::with_id(app, "empty", "在设置中添加自选", false, None::<&str>)?)?;
    } else {
        for instrument in watchlist {
            let id = format!("{INSTRUMENT_PREFIX}{}", instrument.id());
            menu.append(&MenuItem::with_id(app, id, instrument.pair_label(), true, None::<&str>)?)?;
        }
        // Status caption, styled and shown/hidden natively by `set_caption`.
        menu.append(&MenuItem::with_id(app, "status", "", false, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    // No key equivalents: AppKit reserves a shortcut column on every row, which
    // would leave a wide empty band to the right of the prices.
    menu.append(&MenuItem::with_id(app, ID_SETTINGS, "设置…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, ID_QUIT, "退出 Candlewick", true, None::<&str>)?)?;
    Ok(menu)
}

/// The pinned entry as the menu bar shows it; `None` when nothing is pinned.
fn ticker(model: &Model) -> Option<Ticker> {
    let settings = &model.settings;
    let pinned = settings.pinned()?;
    let quote = model.quotes.get(&pinned.id());
    Some(Ticker {
        symbol: settings.show_symbol.then(|| pinned.short_label()),
        price: quote.map_or_else(
            || "—".to_owned(),
            |q| format::price(q.last, format::compact_decimals(q.last, pinned.decimals)),
        ),
        change: quote
            .filter(|_| settings.show_change)
            .and_then(|q| format::change_pct(q.last, q.open))
            .map(|pct| (format::change_signed(pct), format::direction(pct))),
        two_rows: settings.show_change,
        stale: model.stale(pinned.provider),
    })
}

/// The first feed that isn't live explains itself; empty while all are.
fn caption(model: &Model) -> String {
    model
        .settings
        .symbols()
        .into_keys()
        .map(|provider| {
            let status =
                model.feeds.get(&provider).map(|feed| feed.status.clone()).unwrap_or_default();
            status.caption(provider.name())
        })
        .find(|caption| !caption.is_empty())
        .unwrap_or_default()
}

fn rows(watchlist: &[Instrument], quotes: &HashMap<String, Quote>) -> Vec<Row> {
    watchlist
        .iter()
        .map(|instrument| {
            let quote = quotes.get(&instrument.id());
            let (name, detail) = instrument.row_label();
            Row {
                name,
                detail,
                session: quote.and_then(|q| q.session).map(Session::label),
                price: quote.map_or_else(
                    || "—".to_owned(),
                    |q| format::price(q.last, format::decimals(q.last, instrument.decimals)),
                ),
                change: quote
                    .and_then(|q| format::change_pct(q.last, q.open))
                    .map(|pct| (format::change_signed(pct), format::direction(pct))),
            }
        })
        .collect()
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let result = match event.id().as_ref() {
        ID_SETTINGS => window::open_settings(app),
        ID_QUIT => {
            app.exit(0);
            Ok(())
        }
        id => match id.strip_prefix(INSTRUMENT_PREFIX) {
            Some(instrument) => window::open_chart(app, instrument),
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        log::error!("menu action failed: {e}");
    }
}
