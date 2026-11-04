//! Pixels for the shell: the taskbar ticker, drawn with Direct2D and
//! DirectWrite into a 32-bit bitmap the layered ticker window shows as it is
//! (text anti-aliased in grayscale straight onto the taskbar's material, in
//! the taskbar clock's font and sizes), and the icon and menu bitmaps made
//! from `portable::glyph`.

use std::cell::RefCell;

use windows::{
    Win32::{
        Foundation::{E_FAIL, RECT},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
                },
                D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
                D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
                D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
                D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1DCRenderTarget,
                ID2D1Factory,
            },
            DirectWrite::{
                DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_FEATURE,
                DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, DWRITE_FONT_METRICS,
                DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
                DWRITE_FONT_WEIGHT_REGULAR, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_LINE_METRICS,
                DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP,
                DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
                IDWriteTextLayout, IDWriteTypography,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateCompatibleDC,
                CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ,
                RGBQUAD, SelectObject,
            },
        },
        UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO},
    },
    core::{BOOL, Error, HSTRING, Result, w},
};
use windows_numerics::Vector2;

use super::portable::{
    color::{Palette, Rgba},
    glyph::Bitmap,
};
use crate::{
    bar::{self, Ticker},
    i18n::{self, Locale},
    model::ColorScheme,
};

/// A text format's size (its bits), weight, and the language its fallback
/// fonts suit.
type FormatKey = (u32, i32, Locale);

/// Sizes in device-independent pixels (1/96 inch). Two rows at the taskbar
/// clock's size and line pitch; the symbol and one-line text a step larger.
const PADDING: f32 = 8.0;
const ROW_SIZE: f32 = 12.0;
const ROW_PITCH: f32 = 16.0;
const LINE_SIZE: f32 = 14.0;
const SYMBOL_GAP: f32 = 6.0;
const MARKER_GAP: f32 = 4.0;
const WORD_GAP: f32 = 5.0;
/// The hover plate, as Windows 11 draws it behind taskbar buttons.
const PLATE_INSET: f32 = 4.0;
const PLATE_RADIUS: f32 = 4.0;
/// Below this taskbar height (small taskbar buttons) two rows don't fit.
const MIN_TWO_ROW_HEIGHT: f32 = 38.0;

/// What the ticker window shows behind the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plate {
    None,
    Hover,
    Pressed,
    /// Its dropdown is open.
    Open,
}

/// Everything that decides the ticker's pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub ticker: Ticker,
    pub scheme: ColorScheme,
    pub palette: Palette,
    pub dpi: u32,
    pub height: i32,
    pub plate: Plate,
}

/// A laid-out run of text.
struct Text {
    layout: IDWriteTextLayout,
    width: f32,
    /// From the top of the layout box.
    baseline: f32,
}

pub struct Painter {
    dwrite: IDWriteFactory,
    target: ID2D1DCRenderTarget,
    typography: IDWriteTypography,
    family: HSTRING,
    /// Cap height as a share of the font size: text is centered by its
    /// capitals, as the clock's is.
    cap: f32,
    formats: RefCell<Vec<(FormatKey, IDWriteTextFormat)>>,
    // Keeps the factory alive as long as its target.
    _d2d: ID2D1Factory,
}

impl Painter {
    pub fn new() -> Result<Self> {
        // SAFETY: plain factory and target creation on this thread.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            // Software: a small bitmap redrawn a few times a second is
            // cheaper on the CPU than a round trip through the GPU.
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                // Everything is laid out in device pixels.
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = d2d.CreateDCRenderTarget(&properties)?;
            // ClearType needs an opaque background; this one is see-through.
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let typography = dwrite.CreateTypography()?;
            typography.AddFontFeature(DWRITE_FONT_FEATURE {
                nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
                parameter: 1,
            })?;
            let (family, cap) = font(&dwrite)?;
            Ok(Self {
                dwrite,
                target,
                typography,
                family,
                cap,
                formats: RefCell::default(),
                _d2d: d2d,
            })
        }
    }

    /// Draws `frame` into `surface`, which it replaces when the size changes.
    pub fn ticker(&self, frame: &Frame, surface: &mut Option<Surface>) -> Result<()> {
        let scale = frame.dpi as f32 / 96.0;
        let height = frame.height as f32;
        let ticker = &frame.ticker;
        let palette = &frame.palette;
        let change_color = ticker
            .change
            .as_ref()
            .map(|(_, direction)| palette.trend_text(bar::hue(*direction, frame.scheme)));
        let stacked = ticker.two_rows && height / scale >= MIN_TWO_ROW_HEIGHT;
        let head_size = LINE_SIZE * scale;
        let marker = ticker
            .stale
            .then(|| self.text("\u{26A0}\u{FE0E}", head_size, DWRITE_FONT_WEIGHT_REGULAR))
            .transpose()?;
        let symbol = ticker
            .symbol
            .as_deref()
            .map(|symbol| self.text(symbol, head_size, DWRITE_FONT_WEIGHT_SEMI_BOLD))
            .transpose()?;
        let head_baseline = ((height + self.cap * head_size) / 2.0).round();

        // What goes where: (text, x, baseline, color).
        let mut runs: Vec<(Text, f32, f32, Rgba)> = Vec::new();
        let mut x = PADDING * scale;
        if let Some(marker) = marker {
            let width = marker.width;
            runs.push((marker, x, head_baseline, palette.secondary));
            x += width + MARKER_GAP * scale;
        }
        if let Some(symbol) = symbol {
            let width = symbol.width;
            runs.push((symbol, x, head_baseline, palette.text));
            x += width + SYMBOL_GAP * scale;
        }
        if stacked {
            let size = ROW_SIZE * scale;
            let price = self.text(&ticker.price, size, DWRITE_FONT_WEIGHT_REGULAR)?;
            let change = match &ticker.change {
                Some((change, _)) => Some(self.text(change, size, DWRITE_FONT_WEIGHT_REGULAR)?),
                None => None,
            };
            let right = x + price.width.max(change.as_ref().map_or(0.0, |c| c.width));
            let cap = self.cap * size;
            match change {
                Some(change) => {
                    let pitch = ROW_PITCH * scale;
                    let upper = ((height - (cap + pitch)) / 2.0 + cap).round();
                    let lower = upper + pitch.round();
                    let price_x = right - price.width;
                    runs.push((price, price_x, upper, palette.text));
                    let change_x = right - change.width;
                    runs.push((change, change_x, lower, change_color.unwrap_or(palette.text)));
                }
                // No quote yet: the placeholder price sits alone, centered.
                None => {
                    let middle = ((height + cap) / 2.0).round();
                    let price_x = right - price.width;
                    runs.push((price, price_x, middle, palette.text));
                }
            }
            x = right;
        } else {
            let price = self.text(&ticker.price, head_size, DWRITE_FONT_WEIGHT_REGULAR)?;
            let width = price.width;
            runs.push((price, x, head_baseline, palette.text));
            x += width;
            // Too short for two rows: the change follows on the same line.
            if ticker.two_rows
                && let Some((change, _)) = &ticker.change
            {
                x += WORD_GAP * scale;
                let change = self.text(change, head_size, DWRITE_FONT_WEIGHT_REGULAR)?;
                let width = change.width;
                runs.push((change, x, head_baseline, change_color.unwrap_or(palette.text)));
                x += width;
            }
        }
        let width = (x + PADDING * scale).ceil() as i32;

        if surface.as_ref().is_none_or(|s| s.width != width || s.height != frame.height) {
            *surface = None;
            *surface = Some(Surface::new(width, frame.height)?);
        }
        let Some(surface) = surface.as_ref() else {
            return Err(Error::from(E_FAIL));
        };
        let bounds = RECT { left: 0, top: 0, right: width, bottom: frame.height };
        let plate = match frame.plate {
            Plate::None => None,
            Plate::Hover | Plate::Open => Some(palette.hover),
            Plate::Pressed => Some(palette.pressed),
        };
        // SAFETY: draws into the surface's bitmap, selected into its DC.
        unsafe {
            self.target.BindDC(surface.dc, &bounds)?;
            // Everything that can fail comes first: between BeginDraw and
            // EndDraw nothing may return early, or the target stays mid-frame.
            let plate = plate
                .map(|plate| self.target.CreateSolidColorBrush(&color(plate), None))
                .transpose()?;
            let brushes = runs
                .iter()
                .map(|(_, _, _, fill)| self.target.CreateSolidColorBrush(&color(*fill), None))
                .collect::<Result<Vec<_>>>()?;
            self.target.BeginDraw();
            // Nearly transparent, not transparent: a layered window takes
            // clicks only where its alpha isn't zero.
            let clear = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 / 255.0 };
            self.target.Clear(Some(&raw const clear));
            if let Some(brush) = &plate {
                let inset = PLATE_INSET * scale;
                let radius = PLATE_RADIUS * scale;
                let rounded = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 0.5,
                        top: inset,
                        right: width as f32 - 0.5,
                        bottom: height - inset,
                    },
                    radiusX: radius,
                    radiusY: radius,
                };
                self.target.FillRoundedRectangle(&rounded, brush);
            }
            for ((text, x, baseline, _), brush) in runs.iter().zip(&brushes) {
                let origin = Vector2 { X: x.round(), Y: baseline - text.baseline };
                self.target.DrawTextLayout(
                    origin,
                    &text.layout,
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            self.target.EndDraw(None, None)?;
        }
        Ok(())
    }

    fn text(&self, text: &str, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<Text> {
        let format = self.format(size, weight)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: plain DirectWrite calls on objects this painter owns.
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, &format, 4096.0, 4096.0)?;
            layout.SetTypography(
                &self.typography,
                DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 },
            )?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let mut lines = [DWRITE_LINE_METRICS::default()];
            let mut count = 0;
            layout.GetLineMetrics(Some(&mut lines), &mut count)?;
            Ok(Text { layout, width: metrics.width, baseline: lines[0].baseline })
        }
    }

    fn format(&self, size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
        let locale = i18n::current();
        let key = (size.to_bits(), weight.0, locale);
        if let Some((_, format)) = self.formats.borrow().iter().find(|(k, _)| *k == key) {
            return Ok(format.clone());
        }
        // The app's language picks the font for what Segoe UI lacks: Chinese
        // names in Microsoft YaHei, Japanese in Yu Gothic.
        let locale_name = match locale {
            Locale::En => w!("en-US"),
            Locale::ZhCn => w!("zh-CN"),
            Locale::Ja => w!("ja-JP"),
        };
        // SAFETY: plain DirectWrite calls.
        let format = unsafe {
            let format = self.dwrite.CreateTextFormat(
                &self.family,
                None::<&IDWriteFontCollection>,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                locale_name,
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format
        };
        let mut formats = self.formats.borrow_mut();
        // Sizes change with the DPI only; a handful covers every display.
        if formats.len() > 12 {
            formats.clear();
        }
        formats.push((key, format.clone()));
        Ok(format)
    }
}

/// The taskbar's own font on Windows 11, else Windows 10's, and its cap height.
fn font(dwrite: &IDWriteFactory) -> Result<(HSTRING, f32)> {
    let mut collection = None;
    // SAFETY: plain DirectWrite queries.
    unsafe {
        dwrite.GetSystemFontCollection(&mut collection, false)?;
        let collection = collection.ok_or_else(|| Error::from(E_FAIL))?;
        for name in ["Segoe UI Variable Text", "Segoe UI"] {
            let name = HSTRING::from(name);
            let (mut index, mut exists) = (0u32, BOOL(0));
            collection.FindFamilyName(&name, &mut index, &mut exists)?;
            if !exists.as_bool() {
                continue;
            }
            let font = collection.GetFontFamily(index)?.GetFirstMatchingFont(
                DWRITE_FONT_WEIGHT_REGULAR,
                DWRITE_FONT_STRETCH_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
            )?;
            let mut metrics = DWRITE_FONT_METRICS::default();
            font.GetMetrics(&mut metrics);
            let cap = f32::from(metrics.capHeight) / f32::from(metrics.designUnitsPerEm.max(1));
            return Ok((name, cap));
        }
    }
    Ok((HSTRING::from("Segoe UI"), 0.7))
}

fn color(rgba: Rgba) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: rgba.r, g: rgba.g, b: rgba.b, a: rgba.a }
}

/// A 32-bit top-down DIB selected into a memory DC.
pub struct Surface {
    pub dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pub width: i32,
    pub height: i32,
}

impl Surface {
    fn new(width: i32, height: i32) -> Result<Self> {
        // SAFETY: a fresh DC and DIB, released in `drop`.
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return Err(Error::from_thread());
            }
            let (bitmap, _) = match dib(width, height, dc) {
                Ok(dib) => dib,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e);
                }
            };
            let previous = SelectObject(dc, bitmap.into());
            Ok(Self { dc, bitmap, previous, width, height })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the DC and bitmap this surface created.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// A 32-bit top-down DIB section and its pixels.
///
/// # Safety
/// `dc` is null or a live device context.
unsafe fn dib(width: i32, height: i32, dc: HDC) -> Result<(HBITMAP, *mut u8)> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        bmiColors: [RGBQUAD::default()],
    };
    let mut bits = std::ptr::null_mut();
    let dc = (!dc.is_invalid()).then_some(dc);
    // SAFETY: `info` describes the bitmap; `bits` receives its memory.
    let bitmap = unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };
    if bits.is_null() {
        return Err(Error::from(E_FAIL));
    }
    Ok((bitmap, bits.cast()))
}

/// A bitmap of `bytes` (BGRA, top-down) for a square `size` pixels wide.
fn bitmap_from(bytes: &[u8], size: u32) -> Result<HBITMAP> {
    // SAFETY: the DIB holds size × size × 4 bytes, which `bytes` has.
    unsafe {
        let (bitmap, bits) = dib(size as i32, size as i32, HDC::default())?;
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            bits,
            bytes.len().min((size * size * 4) as usize),
        );
        Ok(bitmap)
    }
}

/// An icon (straight alpha, as icons take it) from a glyph bitmap.
pub fn icon(glyph: &Bitmap) -> Result<HICON> {
    let color = bitmap_from(&glyph.straight_bgra(), glyph.size)?;
    // The mask is ignored where the color bitmap has alpha; all zeros keeps
    // it from adding anything. Rows of a 1-bit bitmap are word-aligned.
    let stride = (glyph.size as usize).div_ceil(16) * 2;
    let zeros = vec![0u8; stride * glyph.size as usize];
    // SAFETY: both bitmaps are ours and deleted once the icon has copied them.
    unsafe {
        let mask =
            CreateBitmap(glyph.size as i32, glyph.size as i32, 1, 1, Some(zeros.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// A menu item bitmap (premultiplied alpha, as menus take it).
pub fn menu_bitmap(glyph: &Bitmap) -> Result<HBITMAP> {
    bitmap_from(&glyph.premultiplied_bgra(), glyph.size)
}
