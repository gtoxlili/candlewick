//! The colors the taskbar ticker, tray icon and dropdown marks draw with:
//! Windows 11's own text and fill tokens for each taskbar mode, and the same
//! trend colors the macOS menu bar uses.

use crate::bar::Hue;

/// sRGB with straight (not premultiplied) alpha, each channel 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    pub const fn alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// A GDI `COLORREF` (`0x00BBGGRR`), opaque.
    pub fn from_colorref(color: u32) -> Self {
        let [r, g, b, _] = color.to_le_bytes();
        Self::rgb(r, g, b)
    }

    /// This color with `amount` of `other` mixed in; the alpha stays.
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let lerp = |from: f32, to: f32| from + (to - from) * amount;
        Self {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: self.a,
        }
    }
}

/// Light or dark. The taskbar follows the Windows mode, menus and windows
/// the app mode; both can differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Light,
    Dark,
}

/// How much trend color goes into the ticker's change, as in the macOS menu
/// bar: full red and green glare among the monochrome taskbar items, a tint
/// of the text color still reads as up or down at a glance.
pub const TREND_TINT: f32 = 0.45;

/// What the ticker and tray icon draw with on one taskbar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub text: Rgba,
    /// The stale marker.
    pub secondary: Rgba,
    /// Behind the ticker under the pointer, and while its dropdown is open.
    pub hover: Rgba,
    pub pressed: Rgba,
    pub green: Rgba,
    pub red: Rgba,
}

impl Palette {
    /// Windows 11's TextFillColorPrimary/Secondary and SubtleFillColor
    /// Secondary/Tertiary for the mode, with the system green and red the
    /// windows use.
    pub const fn taskbar(tone: Tone) -> Self {
        match tone {
            Tone::Dark => Self {
                text: Rgba::rgb(255, 255, 255),
                secondary: Rgba::rgb(255, 255, 255).alpha(0.786),
                hover: Rgba::rgb(255, 255, 255).alpha(0.0605),
                pressed: Rgba::rgb(255, 255, 255).alpha(0.0419),
                green: Rgba::rgb(48, 209, 88),
                red: Rgba::rgb(255, 66, 69),
            },
            Tone::Light => Self {
                text: Rgba::rgb(0, 0, 0).alpha(0.894),
                secondary: Rgba::rgb(0, 0, 0).alpha(0.62),
                hover: Rgba::rgb(0, 0, 0).alpha(0.0373),
                pressed: Rgba::rgb(0, 0, 0).alpha(0.0241),
                green: Rgba::rgb(52, 199, 89),
                red: Rgba::rgb(255, 56, 60),
            },
        }
    }

    /// A high-contrast theme: its text and highlight colors, and no trend
    /// colors (the theme decides every color).
    pub fn high_contrast(text: Rgba, highlight: Rgba) -> Self {
        Self {
            text,
            secondary: text,
            hover: highlight.alpha(0.35),
            pressed: highlight.alpha(0.5),
            green: text,
            red: text,
        }
    }

    pub fn hue(&self, hue: Hue) -> Rgba {
        match hue {
            Hue::Green => self.green,
            Hue::Red => self.red,
        }
    }

    /// Text that moved: its color tinted toward the move's.
    pub fn trend_text(&self, hue: Option<Hue>) -> Rgba {
        hue.map_or(self.text, |hue| self.text.mix(self.hue(hue), TREND_TINT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    // COLORREF stores red in the low byte (GDI's RGB macro).
    #[test]
    fn colorref_is_bgr() {
        let c = Rgba::from_colorref(0x0000_80FF);
        assert!(close(c.r, 1.0) && close(c.g, 128.0 / 255.0) && close(c.b, 0.0));
    }

    // The tint keeps the text's alpha, so a tinted change is exactly as strong
    // as the plain text beside it.
    #[test]
    fn tint_keeps_text_alpha() {
        let light = Palette::taskbar(Tone::Light);
        let tinted = light.trend_text(Some(Hue::Green));
        assert!(close(tinted.a, light.text.a));
        assert!(close(tinted.g, light.green.g * TREND_TINT));
        assert_eq!(light.trend_text(None), light.text);
    }
}
