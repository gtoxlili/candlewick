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
    model::{Coin, ColorScheme, Quote, Settings, Shared},
    net, window,
};

const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";
const COIN_PREFIX: &str = "coin:";

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
    /// Coins the current menu was built for; a change rebuilds the menu.
    /// `None` until the first build, so even an empty list gets a menu.
    layout: Option<Vec<Coin>>,
    /// Column widths in points. They only grow while the layout stays the
    /// same, so an open menu never shifts as prices tick.
    columns: Columns,
    tabs: Retained<NSMutableParagraphStyle>,
    scheme: ColorScheme,
    rows: Vec<Row>,
    /// `None` until applied to the current menu.
    caption: Option<String>,
    /// `None` shows the template icon instead of text.
    title: Option<String>,
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
    base: String,
    /// Shown dimmed after the base, e.g. `/USDT`.
    quote: String,
    price: String,
    change: Option<(String, Direction)>,
}

/// Creates the status item. Must run on the main thread (Tauri's `setup`).
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let tray = TrayIconBuilder::new()
        .icon(template_icon())
        .icon_as_template(true)
        .tooltip("Coin Tray")
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
        title: None,
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
    coins: Vec<Coin>,
    rows: Vec<Row>,
    title: Option<String>,
    caption: String,
    scheme: ColorScheme,
}

fn render(app: &AppHandle) {
    let view = {
        let shared = app.state::<Shared>();
        let model = shared.model();
        View {
            coins: model.settings.coins.clone(),
            rows: rows(&model.settings.coins, &model.quotes),
            title: title(&model.settings, &model.quotes, model.stale),
            caption: model.status.caption(),
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
        if self.layout.as_ref() != Some(&view.coins) {
            log::debug!("menu rebuilt for {} coins", view.coins.len());
            self.tray.set_menu(Some(build_menu(app, &view.coins)?))?;
            self.layout = Some(view.coins);
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
        let mut restyle_all = self.scheme != view.scheme;
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

        let coin_count = self.layout.as_ref().map_or(0, Vec::len);
        if coin_count > 0 && self.caption.as_ref() != Some(&view.caption) {
            log::debug!("caption: {:?}", view.caption);
            set_caption(&menu, coin_count, &view.caption, &self.fonts.caption);
            self.caption = Some(view.caption);
        }

        if self.title != view.title {
            match &view.title {
                Some(text) => {
                    if self.title.is_none() {
                        self.tray.set_icon(None)?;
                    }
                    log::debug!("title: {text}");
                    self.tray.set_title(Some(text))?;
                }
                None => {
                    self.tray.set_title(Some(""))?;
                    self.tray.set_icon_with_as_template(Some(template_icon()), true)?;
                }
            }
            self.title = view.title;
        }
        Ok(())
    }

    /// `BTC/USDT ⇥ 84,002.01 ⇥ −0.24%` with right-aligned tab stops.
    fn styled_row(&self, row: &Row) -> Retained<NSMutableAttributedString> {
        let mut text = format!("{}{}\t{}", row.base, row.quote, row.price);
        if let Some((change, _)) = &row.change {
            text.push('\t');
            text.push_str(change);
        }
        let string = attributed(&text, &self.fonts.row);
        let base_len = utf16_len(&row.base);
        // SAFETY: each value matches its attribute key.
        unsafe {
            string.addAttribute_value_range(
                NSParagraphStyleAttributeName,
                &self.tabs,
                NSRange::new(0, utf16_len(&text)),
            );
            string.addAttribute_value_range(
                NSForegroundColorAttributeName,
                &NSColor::secondaryLabelColor(),
                NSRange::new(base_len, utf16_len(&row.quote)),
            );
        }
        if let Some((change, direction)) = &row.change {
            let color =
                match (direction, self.scheme) {
                    (Direction::Up, ColorScheme::GreenUp)
                    | (Direction::Down, ColorScheme::RedUp) => Some(NSColor::systemGreenColor()),
                    (Direction::Down, ColorScheme::GreenUp)
                    | (Direction::Up, ColorScheme::RedUp) => Some(NSColor::systemRedColor()),
                    (Direction::Flat, _) => None,
                };
            if let Some(color) = color {
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
        }
        string
    }
}

impl Columns {
    fn fit(self, row: &Row, font: &NSFont) -> Self {
        let change = row.change.as_ref().map_or(0.0, |(change, _)| text_width(change, font));
        Self {
            label: self.label.max(text_width(&format!("{}{}", row.base, row.quote), font)),
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

/// A small dimmed line under the coin rows, shown only when the feed is not
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

fn build_menu(app: &AppHandle, coins: &[Coin]) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    if coins.is_empty() {
        menu.append(&MenuItem::with_id(app, "empty", "在设置中添加币种", false, None::<&str>)?)?;
    } else {
        for coin in coins {
            let id = format!("{COIN_PREFIX}{}", coin.symbol);
            menu.append(&MenuItem::with_id(app, id, coin.pair_label(), true, None::<&str>)?)?;
        }
        // Status caption, styled and shown/hidden natively by `set_caption`.
        menu.append(&MenuItem::with_id(app, "status", "", false, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    // No key equivalents: AppKit reserves a shortcut column on every row, which
    // would leave a wide empty band to the right of the prices.
    menu.append(&MenuItem::with_id(app, ID_SETTINGS, "设置…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, ID_QUIT, "退出 Coin Tray", true, None::<&str>)?)?;
    Ok(menu)
}

/// Menu bar text, e.g. `BTC 84,050  ETH 3,412 ▲1.20%`; `None` when nothing is pinned.
fn title(settings: &Settings, quotes: &HashMap<String, Quote>, stale: bool) -> Option<String> {
    let parts: Vec<String> = settings
        .coins
        .iter()
        .filter(|coin| coin.pinned)
        .map(|coin| {
            let mut part = String::new();
            if settings.show_symbol {
                part.push_str(&coin.short_label());
                part.push(' ');
            }
            match quotes.get(&coin.symbol) {
                Some(q) => {
                    part.push_str(&format::price(
                        q.last,
                        format::compact_decimals(q.last, coin.decimals),
                    ));
                    if settings.show_change
                        && let Some(pct) = format::change_pct(q.last, q.open)
                    {
                        part.push(' ');
                        part.push_str(&format::change_arrow(pct));
                    }
                }
                None => part.push('—'),
            }
            part
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join("  ");
    Some(if stale { format!("⚠︎ {joined}") } else { joined })
}

fn rows(coins: &[Coin], quotes: &HashMap<String, Quote>) -> Vec<Row> {
    coins
        .iter()
        .map(|coin| {
            let quote = quotes.get(&coin.symbol);
            Row {
                base: coin.base.clone(),
                quote: format!("/{}", coin.quote),
                price: quote.map_or_else(
                    || "—".to_owned(),
                    |q| format::price(q.last, format::decimals(q.last, coin.decimals)),
                ),
                change: quote
                    .and_then(|q| format::change_pct(q.last, q.open))
                    .map(|pct| (format::change_signed(pct), format::direction(pct))),
            }
        })
        .collect()
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        ID_SETTINGS => {
            if let Err(e) = window::open(app) {
                log::error!("cannot open settings: {e}");
            }
        }
        ID_QUIT => app.exit(0),
        id => {
            if let Some(symbol) = id.strip_prefix(COIN_PREFIX) {
                open_trade_page(app, symbol);
            }
        }
    }
}

fn open_trade_page(app: &AppHandle, symbol: &str) {
    let url = {
        let shared = app.state::<Shared>();
        let model = shared.model();
        let Some(coin) = model.settings.coins.iter().find(|c| c.symbol == symbol) else {
            return;
        };
        format!(
            "https://www.binance.com/zh-CN/trade/{}_{}?type=spot",
            net::percent_encode(&coin.base),
            net::percent_encode(&coin.quote)
        )
    };
    macos::open_url(&url);
}
