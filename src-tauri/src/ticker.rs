//! The menu bar title for the pinned pair: plain text on one line, or, with
//! the 24h change shown, the symbol beside a two-row block (price over
//! change) that takes about half the width. A status item title is a single
//! line, so that layout is drawn into an image.

use block2::RcBlock;
use objc2::{AnyThread, rc::Retained, runtime::Bool};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSColor, NSFont, NSFontAttributeName, NSFontWeightMedium,
    NSForegroundColorAttributeName, NSImage, NSImageCacheMode, NSStatusBar,
};
use objc2_foundation::{NSMutableAttributedString, NSPoint, NSRange, NSRect, NSSize, NSString};

use crate::{format::Direction, model::ColorScheme};

/// The symbol is set like other menu bar text. The rows are sized against
/// other two-row menu bar items (network and CPU meters): one small size, with
/// clear space between and around them in a 22 pt bar.
const ROW_SIZE: f64 = 9.0;
const ROW_GAP: f64 = 3.0;
const SYMBOL_GAP: f64 = 5.0;
const MARKER_GAP: f64 = 3.0;
/// How much trend color goes into the change. Full red and green glare among
/// the monochrome menu bar items; a tint of the text color still reads as up
/// or down at a glance.
const TREND_TINT: f64 = 0.45;

#[derive(Debug, Clone, PartialEq)]
pub struct Ticker {
    /// `BTC`, or `ETH/BTC` for a non-USD quote; `None` when symbols are hidden.
    pub symbol: Option<String>,
    /// `—` until the first quote arrives.
    pub price: String,
    /// The 24h change; set once there is a quote, if the change is shown.
    pub change: Option<(String, Direction)>,
    pub two_rows: bool,
    /// Prices may be old: the feed is reconnecting.
    pub stale: bool,
}

impl Ticker {
    /// The one-line title, e.g. `BTC 84,050`.
    pub fn line(&self) -> String {
        let mut line = String::new();
        if self.stale {
            line.push_str("⚠︎ ");
        }
        if let Some(symbol) = &self.symbol {
            line.push_str(symbol);
            line.push(' ');
        }
        line.push_str(&self.price);
        line
    }

    /// What VoiceOver reads for the drawn layout.
    fn spoken(&self) -> String {
        match &self.change {
            Some((change, _)) => format!("{} {change}", self.line()),
            None => self.line(),
        }
    }
}

/// The color of a rising or falling number under the user's convention.
pub fn trend_color(direction: Direction, scheme: ColorScheme) -> Option<Retained<NSColor>> {
    match (direction, scheme) {
        (Direction::Up, ColorScheme::GreenUp) | (Direction::Down, ColorScheme::RedUp) => {
            Some(NSColor::systemGreenColor())
        }
        (Direction::Down, ColorScheme::GreenUp) | (Direction::Up, ColorScheme::RedUp) => {
            Some(NSColor::systemRedColor())
        }
        (Direction::Flat, _) => None,
    }
}

/// The symbol, vertically centered, then the price over the change, both
/// right-aligned. Colors are resolved each time the image is drawn, so they
/// follow the menu bar's own light or dark appearance.
pub fn image(ticker: &Ticker, scheme: ColorScheme) -> Retained<NSImage> {
    // SAFETY: reading an immutable AppKit constant.
    let medium = unsafe { NSFontWeightMedium };
    let symbol_font =
        NSFont::systemFontOfSize_weight(NSFont::menuBarFontOfSize(0.0).pointSize(), medium);
    let row_font = NSFont::monospacedDigitSystemFontOfSize_weight(ROW_SIZE, medium);

    let marker = ticker.stale.then(|| styled("⚠︎", &symbol_font, &NSColor::secondaryLabelColor()));
    let symbol = ticker.symbol.as_ref().map(|s| styled(s, &symbol_font, &NSColor::labelColor()));
    let price = styled(&ticker.price, &row_font, &NSColor::labelColor());
    // Styled at draw time: the tint mixes into the text color of whichever
    // appearance the menu bar is drawing in.
    let change = ticker.change.as_ref().map(|(text, direction)| {
        let row_font = row_font.clone();
        let (text, trend) = (text.clone(), trend_color(*direction, scheme));
        move || {
            let text_color = NSColor::labelColor();
            let color = trend
                .as_ref()
                .and_then(|trend| text_color.blendedColorWithFraction_ofColor(TREND_TINT, trend))
                .unwrap_or(text_color);
            styled(&text, &row_font, &color)
        }
    });

    let height = NSStatusBar::systemStatusBar().thickness();
    let marker_width = marker.as_ref().map_or(0.0, |m| (m.size().width + MARKER_GAP).ceil());
    let symbol_width = symbol.as_ref().map_or(0.0, |s| (s.size().width + SYMBOL_GAP).ceil());
    let price_width = price.size().width;
    let change_width = change.as_ref().map_or(0.0, |c| c().size().width);
    let column = price_width.max(change_width).ceil();
    let right = marker_width + symbol_width + column;

    // `drawAtPoint` puts the line's bottom (baseline plus descender) at y.
    let (symbol_cap, symbol_descender) = (symbol_font.capHeight(), symbol_font.descender());
    let (row_cap, row_descender) = (row_font.capHeight(), row_font.descender());
    let draw = RcBlock::new(move |_: NSRect| -> Bool {
        let symbol_y = ((height - symbol_cap) / 2.0).round() + symbol_descender;
        if let Some(marker) = &marker {
            marker.drawAtPoint(NSPoint::new(0.0, symbol_y));
        }
        if let Some(symbol) = &symbol {
            symbol.drawAtPoint(NSPoint::new(marker_width, symbol_y));
        }
        match &change {
            Some(change) => {
                let lower = ((height - (2.0 * row_cap + ROW_GAP)) / 2.0).round();
                change().drawAtPoint(NSPoint::new(right - change_width, lower + row_descender));
                let upper = lower + row_cap + ROW_GAP;
                price.drawAtPoint(NSPoint::new(right - price_width, upper + row_descender));
            }
            // No quote yet: the placeholder price sits alone, centered.
            None => {
                let middle = ((height - row_cap) / 2.0).round();
                price.drawAtPoint(NSPoint::new(right - price_width, middle + row_descender));
            }
        }
        Bool::YES
    });
    let image =
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(right, height), false, &draw);
    image.setCacheMode(NSImageCacheMode::Never);
    image.setAccessibilityDescription(Some(&NSString::from_str(&ticker.spoken())));
    image
}

fn styled(text: &str, font: &NSFont, color: &NSColor) -> Retained<NSMutableAttributedString> {
    let string = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &NSString::from_str(text),
    );
    let all = NSRange::new(0, text.encode_utf16().count());
    // SAFETY: each value matches its attribute key.
    unsafe {
        string.addAttribute_value_range(NSFontAttributeName, font, all);
        string.addAttribute_value_range(NSForegroundColorAttributeName, color, all);
    }
    string
}
