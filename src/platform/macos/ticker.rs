//! The two-row ticker: with the change shown, the menu bar title is the
//! symbol beside a block of price over change that takes about half the
//! width. A status item title is a single line, so that layout is drawn into
//! an image.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::{AnyThread, rc::Retained, runtime::Bool};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSAttributedStringNSStringDrawing, NSColor, NSFont, NSFontAttributeName, NSFontWeightMedium,
    NSForegroundColorAttributeName, NSImage, NSImageCacheMode, NSStatusBar,
};
use objc2_foundation::{
    NSArray, NSMutableAttributedString, NSPoint, NSRange, NSRect, NSSize, NSString,
};

use crate::{
    bar::{self, Hue, Ticker},
    format::Direction,
    model::ColorScheme,
};

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

/// The app's trend colors (src/index.css `--up`/`--down`), as sRGB bytes
/// for the light and the dark appearance.
const GREEN: ((u8, u8, u8), (u8, u8, u8)) = ((0x00, 0x80, 0x47), (0x30, 0xd7, 0x92));
const RED: ((u8, u8, u8), (u8, u8, u8)) = ((0xd4, 0x2e, 0x3d), (0xff, 0x6e, 0x74));

/// A color that resolves to its light or dark value for whatever appearance
/// it is drawn in, as the system's own semantic colors do.
fn dynamic(name: &str, (light, dark): ((u8, u8, u8), (u8, u8, u8))) -> Retained<NSColor> {
    let srgb = |(r, g, b): (u8, u8, u8)| {
        NSColor::colorWithSRGBRed_green_blue_alpha(
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
            1.0,
        )
    };
    let provider = RcBlock::new(move |appearance: NonNull<NSAppearance>| -> NonNull<NSColor> {
        // SAFETY: AppKit hands a live appearance to the provider.
        let appearance = unsafe { appearance.as_ref() };
        // SAFETY: reading immutable AppKit constants.
        let (aqua, dark_aqua) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
        let is_dark = appearance
            .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[aqua, dark_aqua]))
            .is_some_and(|best| &*best == dark_aqua);
        // The provider returns an autoreleased object, as a method would.
        let color = Retained::autorelease_return(srgb(if is_dark { dark } else { light }));
        NonNull::new(color).expect("a color")
    });
    // SAFETY: the block takes and returns the documented types.
    unsafe { NSColor::colorWithName_dynamicProvider(Some(&NSString::from_str(name)), &provider) }
}

/// The color of a rising or falling number under the user's convention.
pub fn trend_color(direction: Direction, scheme: ColorScheme) -> Option<Retained<NSColor>> {
    bar::hue(direction, scheme).map(|hue| match hue {
        Hue::Green => dynamic("candlewick.green", GREEN),
        Hue::Red => dynamic("candlewick.red", RED),
    })
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
