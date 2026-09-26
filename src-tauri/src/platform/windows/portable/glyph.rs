//! Small pictures drawn pixel-exact at whatever size Windows asks for: the
//! tray icon (the app icon's price line ending in its dot, in the taskbar's
//! ink) and the trend marks beside dropdown rows. Shapes are signed distance
//! fields sampled at pixel centers, which anti-aliases their edges over one
//! pixel however small the size.

use super::color::Rgba;

/// Premultiplied RGBA, rows top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitmap {
    pub size: u32,
    pixels: Vec<[f32; 4]>,
}

impl Bitmap {
    fn new(size: u32) -> Self {
        Self { size, pixels: vec![[0.0; 4]; (size * size) as usize] }
    }

    /// Paints `color` over the bitmap where `coverage(x, y)` (at the pixel
    /// center, in pixels) says the shape is.
    fn paint(&mut self, color: Rgba, coverage: impl Fn(f32, f32) -> f32) {
        let size = self.size as usize;
        for (index, pixel) in self.pixels.iter_mut().enumerate() {
            let (x, y) = ((index % size) as f32 + 0.5, (index / size) as f32 + 0.5);
            let a = coverage(x, y).clamp(0.0, 1.0) * color.a;
            if a <= 0.0 {
                continue;
            }
            let src = [color.r * a, color.g * a, color.b * a, a];
            for channel in 0..4 {
                pixel[channel] = src[channel] + pixel[channel] * (1.0 - a);
            }
        }
    }

    /// Premultiplied BGRA bytes, as layered windows and menu bitmaps take them.
    pub fn premultiplied_bgra(&self) -> Vec<u8> {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels
            .iter()
            .flat_map(|[r, g, b, a]| [byte(*b), byte(*g), byte(*r), byte(*a)])
            .collect()
    }

    /// Straight-alpha BGRA bytes, as icons take them.
    pub fn straight_bgra(&self) -> Vec<u8> {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels
            .iter()
            .flat_map(|[r, g, b, a]| {
                let un = |v: f32| if *a > 0.0 { v / a } else { 0.0 };
                [byte(un(*b)), byte(un(*g)), byte(un(*r)), byte(*a)]
            })
            .collect()
    }

    #[cfg(test)]
    fn alpha_at(&self, x: u32, y: u32) -> f32 {
        self.pixels[(y * self.size + x) as usize][3]
    }
}

/// The price line of `icons/tray-template.svg`, in its 36-unit square.
const LINE: [[(f32, f32); 4]; 3] = [
    [(4.0, 23.5), (5.2, 22.5), (9.0, 17.8), (11.0, 17.5)],
    [(11.0, 17.5), (13.0, 17.2), (13.3, 23.1), (16.0, 22.0)],
    [(16.0, 22.0), (18.7, 20.9), (25.2, 12.8), (27.0, 11.0)],
];
const LINE_WIDTH: f32 = 2.8;
const DOT: (f32, f32) = (27.0, 11.0);
const DOT_RADIUS: f32 = 3.8;
/// The glyph is wide and short; drawn this much larger around its own center
/// (`GLYPH_CENTER`), it spans 90% of the square the notification area gives it.
const ZOOM: f32 = 1.15;
const GLYPH_CENTER: (f32, f32) = (16.7, 16.05);
/// A hairline blurs into the taskbar at 16 px; the line never gets thinner.
const MIN_LINE_PX: f32 = 1.25;

/// The tray icon at `size` pixels square: the line in `ink`, its dot in `dot`.
pub fn tray(size: u32, ink: Rgba, dot: Rgba) -> Bitmap {
    let scale = size as f32 / 36.0;
    let place = |(x, y): (f32, f32)| {
        (((x - GLYPH_CENTER.0) * ZOOM + 18.0) * scale, ((y - GLYPH_CENTER.1) * ZOOM + 18.0) * scale)
    };
    let points: Vec<(f32, f32)> = LINE
        .iter()
        .enumerate()
        .flat_map(|(index, curve)| {
            // Each curve's first point is the previous one's last.
            let first = if index == 0 { 0 } else { 1 };
            (first..=12).map(move |step| cubic(curve, step as f32 / 12.0))
        })
        .map(place)
        .collect();
    let half = (LINE_WIDTH * ZOOM * scale).max(MIN_LINE_PX) / 2.0;
    let (cx, cy) = place(DOT);
    let radius = DOT_RADIUS * ZOOM * scale;

    let mut bitmap = Bitmap::new(size);
    bitmap.paint(ink, |x, y| {
        let distance = points
            .windows(2)
            .map(|segment| segment_distance((x, y), segment[0], segment[1]))
            .fold(f32::INFINITY, f32::min);
        half + 0.5 - distance
    });
    // The line ends under the dot's center, as in the template.
    bitmap.paint(dot, |x, y| radius + 0.5 - (x - cx).hypot(y - cy));
    bitmap
}

/// A small triangle pointing up or down, centered, for a dropdown row.
pub fn trend_mark(size: u32, up: bool, color: Rgba) -> Bitmap {
    let s = size as f32;
    let (half_width, height) = (s * 0.3, s * 0.42);
    let (top, bottom) = ((s - height) / 2.0, (s + height) / 2.0);
    let (apex, base) = if up { (top, bottom) } else { (bottom, top) };
    let triangle = [(s / 2.0, apex), (s / 2.0 + half_width, base), (s / 2.0 - half_width, base)];
    let mut bitmap = Bitmap::new(size);
    bitmap.paint(color, |x, y| 0.5 - convex_distance((x, y), &triangle));
    bitmap
}

fn cubic(curve: &[(f32, f32); 4], t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let [a, b, c, d] = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    let [p0, p1, p2, p3] = curve;
    (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Signed distance to a convex polygon: negative inside. Exact inside and
/// along the edges, which is all a one-pixel edge ramp needs.
fn convex_distance(p: (f32, f32), polygon: &[(f32, f32)]) -> f32 {
    let area: f32 = (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let orientation = area.signum();
    (0..polygon.len())
        .map(|i| {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            let (ex, ey) = (b.0 - a.0, b.1 - a.1);
            let length = ex.hypot(ey);
            // Outward normal for the winding the area gave.
            orientation * ((p.0 - a.0) * ey - (p.1 - a.1) * ex) / length
        })
        .fold(f32::NEG_INFINITY, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Rgba = Rgba::rgb(255, 255, 255);
    const GREEN: Rgba = Rgba::rgb(48, 209, 88);

    // The notification area asks for SM_CXSMICON at the taskbar's DPI: 16, 20,
    // 24 and 32 px at 100–200% scaling. Every size gets its own square bitmap.
    #[test]
    fn sizes_are_exact() {
        for size in [16, 20, 24, 32] {
            let icon = tray(size, INK, GREEN);
            assert_eq!(icon.size, size);
            assert_eq!(icon.straight_bgra().len(), (size * size * 4) as usize);
        }
    }

    // tray-template.svg's dot (27, 11 of 36) and line start (4, 23.5), moved
    // by the 1.15 zoom around the glyph's center (16.7, 16.05): (29.8, 12.2)
    // and (3.4, 26.6). The dot takes its own color, the line the ink.
    #[test]
    fn dot_and_line_follow_the_template() {
        let icon = tray(36, INK, GREEN);
        let dot = &icon.straight_bgra()[((12 * 36 + 29) * 4) as usize..][..4];
        assert_eq!(dot, [88, 209, 48, 255]);
        assert!(icon.alpha_at(3, 26) > 0.9);
        // Corners stay clear.
        assert_eq!(icon.alpha_at(0, 0), 0.0);
        assert_eq!(icon.alpha_at(35, 35), 0.0);
    }

    // Premultiplied bytes never exceed alpha, or AlphaBlend reads garbage.
    #[test]
    fn premultiplied_channels_stay_under_alpha() {
        let mark = trend_mark(16, true, GREEN.alpha(0.8));
        for pixel in mark.premultiplied_bgra().chunks(4) {
            assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
        }
    }

    // An up mark is heavier at the bottom (its base), a down mark at the top.
    #[test]
    fn marks_point_their_way() {
        let weight = |mark: &Bitmap, rows: std::ops::Range<u32>| -> f32 {
            rows.flat_map(|y| (0..mark.size).map(move |x| (x, y)))
                .map(|(x, y)| mark.alpha_at(x, y))
                .sum()
        };
        let up = trend_mark(16, true, GREEN);
        let down = trend_mark(16, false, GREEN);
        assert!(weight(&up, 8..16) > weight(&up, 0..8));
        assert!(weight(&down, 0..8) > weight(&down, 8..16));
    }
}
