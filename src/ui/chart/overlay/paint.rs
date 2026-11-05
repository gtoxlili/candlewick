use gpui_kit::{
    App, Bounds, ContentMask, Hsla, PathBuilder, Pixels, Point as GpPoint, ShapedLine, TextAlign,
    TextRun, Window, fill, point, px, quad, size,
};
use std::collections::HashMap;
use std::ops::Range;

use super::super::{
    analysis::Analysis,
    geometry::{self, Frame, Mapping, Point},
    model::{Anchor, Drawing, DrawingKind, FIB_LEVELS},
};
use crate::{format, ui::theme::Palette};

#[derive(Default)]
pub struct TextCache {
    lines: HashMap<(String, u32, u32), ShapedLine>,
}

impl TextCache {
    fn line(&mut self, text: String, color: Hsla, font_size: f32, window: &Window) -> ShapedLine {
        let key = (text, u32::from(color.to_rgb()), font_size.to_bits());
        if let Some(line) = self.lines.get(&key) {
            return line.clone();
        }
        if self.lines.len() > 512 {
            self.lines.clear();
        }
        let run = TextRun {
            len: key.0.len(),
            font: gpui_kit::Font {
                features: crate::ui::tabular_features(),
                ..window.text_style().font()
            },
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line =
            window.text_system().shape_line(key.0.clone().into(), px(font_size), &[run], None);
        self.lines.insert(key, line.clone());
        line
    }
}

struct Painter<'a> {
    window: &'a mut Window,
    cx: &'a mut App,
    cache: &'a mut TextCache,
    origin: GpPoint<Pixels>,
    palette: Palette,
    decimals: usize,
    interval: f64,
    day_offset: i64,
}

pub struct PaintOptions {
    pub palette: Palette,
    pub interval: f64,
    pub decimals: usize,
    pub day_offset: i64,
    pub panning: bool,
}

impl Painter<'_> {
    fn p(&self, x: f64, y: f64) -> GpPoint<Pixels> {
        point(self.origin.x + px(x as f32), self.origin.y + px(y as f32))
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: Hsla) {
        if w > 0. && h > 0. {
            self.window.paint_quad(fill(
                Bounds::new(self.p(x, y), size(px(w as f32), px(h as f32))),
                color,
            ));
        }
    }
    fn rounded(&mut self, x: f64, y: f64, w: f64, h: f64, radius: f32, color: Hsla) {
        if w > 0. && h > 0. {
            self.window.paint_quad(quad(
                Bounds::new(self.p(x, y), size(px(w as f32), px(h as f32))),
                px(radius),
                color,
                px(0.),
                color,
                Default::default(),
            ));
        }
    }
    fn line(&mut self, a: Point, b: Point, color: Hsla, width: f32, dashed: bool) {
        let mut path = PathBuilder::stroke(px(width));
        if dashed {
            path = path.dash_array(&[px(3.), px(3.)]);
        }
        path.move_to(self.p(a.x, a.y));
        path.line_to(self.p(b.x, b.y));
        if let Ok(path) = path.build() {
            self.window.paint_path(path, color);
        }
    }
    fn text(&mut self, text: String, x: f64, y: f64, color: Hsla, font_size: f32) -> f64 {
        let line = self.cache.line(text, color, font_size, self.window);
        let width = f32::from(line.width) as f64;
        let _ = line.paint(
            self.p(x, y),
            px(font_size + 3.),
            TextAlign::Left,
            None,
            self.window,
            self.cx,
        );
        width
    }
    fn tag(&mut self, text: String, x: f64, y: f64, color: Hsla) {
        let line = self.cache.line(text, self.palette.background, 11., self.window);
        let width = f32::from(line.width) as f64 + 12.;
        self.rounded(x, y - 9., width, 18., 5., color);
        let _ = line.paint(
            self.p(x + 6., y - 7.),
            px(14.),
            TextAlign::Left,
            None,
            self.window,
            self.cx,
        );
    }
    fn drawing(&mut self, d: &Drawing, map: &Mapping, selected: bool, lit: bool, draft: bool) {
        let color = if selected { self.palette.glow } else { self.palette.violet };
        let plot = map.plot();
        let pos = |a: Anchor| Point { x: map.x(a.time), y: map.y(a.price) };
        match d {
            Drawing::Level { price, .. } => self.line(
                Point { x: plot.x, y: map.y(*price) },
                Point { x: plot.right(), y: map.y(*price) },
                color,
                if lit { 1.5 } else { 1. },
                draft,
            ),
            Drawing::Trend { a, b, .. } => {
                self.line(pos(*a), pos(*b), color, if lit { 1.5 } else { 1. }, draft)
            }
            Drawing::Fib { a, b, .. } => {
                let (pa, pb) = (pos(*a), pos(*b));
                let (x0, x1) = geometry::fib_span(pa, pb, plot);
                let ys = geometry::fib_values(pa.y, pb.y);
                for i in 1..ys.len() {
                    let mut band = color;
                    band.a = 0.035 + 0.012 * i as f32;
                    self.rect(x0, ys[i - 1].min(ys[i]), x1 - x0, (ys[i] - ys[i - 1]).abs(), band);
                }
                let mut swing = color;
                swing.a = 0.35;
                self.line(pa, pb, swing, 1., true);
                for (i, level) in FIB_LEVELS.iter().enumerate() {
                    let mut c = color;
                    c.a = if i == 0 || i == 6 { 0.85 } else { 0.55 };
                    self.line(Point { x: x0, y: ys[i] }, Point { x: x1, y: ys[i] }, c, 1., false);
                    let label = format!(
                        "{}  {}",
                        level,
                        format::price(a.price + (b.price - a.price) * level, self.decimals)
                    );
                    self.text(label, x0 + 6., ys[i] - 15., color, 10.);
                }
            }
            Drawing::Measure { a, b, .. } => {
                let r = geometry::measure_box(pos(*a), pos(*b));
                let color = if b.price >= a.price { self.palette.up } else { self.palette.down };
                let mut bg = color;
                bg.a = if lit { 0.12 } else { 0.09 };
                self.rect(r.x, r.y, r.w, r.h, bg);
                let mut border = color;
                border.a = 0.6;
                for (p, q) in [
                    (Point { x: r.x, y: r.y }, Point { x: r.right(), y: r.y }),
                    (Point { x: r.right(), y: r.y }, Point { x: r.right(), y: r.bottom() }),
                    (Point { x: r.right(), y: r.bottom() }, Point { x: r.x, y: r.bottom() }),
                    (Point { x: r.x, y: r.bottom() }, Point { x: r.x, y: r.y }),
                ] {
                    self.line(p, q, border, 1., false);
                }
                let change = b.price - a.price;
                let pct = if a.price > 0. { change / a.price * 100. } else { 0. };
                let bars = ((b.time - a.time).abs() / self.interval).round();
                let one = format!("{pct:+.2}%  {}", format::signed(change, self.decimals));
                let count = bars as u64;
                let label = if count == 1 {
                    t!("chart.barsOne", count = count).to_string()
                } else {
                    t!("chart.bars", count = count).to_string()
                };
                let two = format!("{} · {}", label, format::duration((b.time - a.time).abs()));
                let one_width =
                    f32::from(self.cache.line(one.clone(), color, 11., self.window).width) as f64;
                let two_width = f32::from(
                    self.cache.line(two.clone(), self.palette.muted, 11., self.window).width,
                ) as f64;
                let w = (one_width.max(two_width) + 20.).min(plot.w);
                let x = (r.x + r.w / 2. - w / 2.).max(plot.x + 4.).min(plot.right() - w - 4.);
                let y = if r.y - 52. >= plot.y { r.y - 52. } else { r.bottom() + 8. };
                self.rounded(x, y, w, 44., 8., self.palette.popover);
                self.text(one, x + (w - one_width) / 2., y + 5., color, 11.);
                self.text(two, x + (w - two_width) / 2., y + 21., self.palette.muted, 11.);
            }
        }
    }
    fn series(
        &mut self,
        values: &[f64],
        candles: &[crate::market::Candle],
        range: Range<usize>,
        map: &Mapping,
        style: (Hsla, f32, bool),
    ) {
        let (color, width, dashed) = style;
        let mut path = PathBuilder::stroke(px(width));
        if dashed {
            path = path.dash_array(&[px(2.), px(4.)]);
        }
        let mut started = false;
        for i in range.start..range.end.min(values.len()) {
            if !values[i].is_finite() {
                continue;
            }
            let p = self.p(map.x(candles[i].time + self.interval / 2.), map.y(values[i]));
            if started {
                path.line_to(p);
            } else {
                path.move_to(p);
                started = true;
            }
        }
        if started && let Ok(path) = path.build() {
            self.window.paint_path(path, color);
        }
    }
}

pub fn paint(
    bounds: Bounds<Pixels>,
    frame: Frame,
    analysis: &Analysis,
    options: PaintOptions,
    cache: &mut TextCache,
    window: &mut Window,
    cx: &mut App,
) {
    let PaintOptions { palette, interval, decimals, day_offset, panning } = options;
    let map = geometry::mapping(&frame);
    let plot = map.plot();
    if plot.w <= 0. || plot.h <= 0. {
        return;
    }
    let mut p = Painter {
        window,
        cx,
        cache,
        origin: bounds.origin,
        palette,
        decimals,
        interval,
        day_offset,
    };
    let clip = Bounds::new(p.p(plot.x, plot.y), size(px(plot.w as f32), px(plot.h as f32)));
    p.window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
        let mut p = Painter {
            window,
            cx: p.cx,
            cache: p.cache,
            origin: p.origin,
            palette,
            decimals,
            interval,
            day_offset,
        };
        let candles = analysis.candles();
        let from = candles.partition_point(|c| c.time + interval < frame.left).saturating_sub(1);
        let to =
            (candles.partition_point(|c| c.time <= frame.right + interval) + 1).min(candles.len());
        for series in &analysis.series {
            let color = match series.spec.id.as_str() {
                "ma1" => palette.violet,
                "ma2" => palette.amber,
                "ma3" => palette.mint,
                "boll" => palette.glow,
                _ => palette.foreground,
            };
            if let (Some(upper), Some(lower)) = (&series.upper, &series.lower) {
                let mut path = PathBuilder::fill();
                let mut started = false;
                for i in from..to.min(upper.len()) {
                    if upper[i].is_finite() && lower[i].is_finite() {
                        let v = p.p(map.x(candles[i].time + interval / 2.), map.y(upper[i]));
                        if started {
                            path.line_to(v);
                        } else {
                            path.move_to(v);
                            started = true;
                        }
                    }
                }
                for i in (from..to.min(lower.len()).min(upper.len())).rev() {
                    if upper[i].is_finite() && lower[i].is_finite() {
                        path.line_to(p.p(map.x(candles[i].time + interval / 2.), map.y(lower[i])));
                    }
                }
                if started && let Ok(path) = path.build() {
                    let mut c = color;
                    c.a = 0.07;
                    p.window.paint_path(path, c);
                }
                let mut c = color;
                c.a = 0.55;
                p.series(upper, candles, from..to, &map, (c, 1., false));
                p.series(lower, candles, from..to, &map, (c, 1., false));
            }
            p.series(
                &series.values,
                candles,
                from..to,
                &map,
                (color, 1.25, series.upper.is_some()),
            );
        }
        for drawing in &analysis.drawings {
            let selected = analysis.selected.as_deref() == Some(drawing.id());
            let lit = selected || analysis.hovered.as_ref().is_some_and(|h| h.id == drawing.id());
            p.drawing(drawing, &map, selected, lit, false);
        }
        if let (Some(draft), Some(snapped)) = (&analysis.draft, analysis.snapped)
            && let Some(a) = draft.points.first().copied()
        {
            let b = snapped;
            let id = String::new();
            let d = match draft.kind {
                DrawingKind::Level => Drawing::Level { id, price: a.price },
                DrawingKind::Trend => Drawing::Trend { id, a, b },
                DrawingKind::Fib => Drawing::Fib { id, a, b },
                DrawingKind::Measure => Drawing::Measure { id, a, b },
            };
            p.drawing(&d, &map, true, true, true);
        }
    });
    for drawing in &analysis.drawings {
        if let Drawing::Level { price, .. } = drawing {
            let color = if analysis.selected.as_deref() == Some(drawing.id()) {
                palette.glow
            } else {
                palette.violet
            };
            let y = map.y(*price);
            if (plot.y..=plot.bottom()).contains(&y) {
                p.tag(format::price(*price, decimals), plot.right() + 3., y, color);
            }
        }
    }
    if let Some(selected) = &analysis.selected
        && let Some(d) = analysis.drawings.iter().find(|d| d.id() == selected)
        && let Drawing::Trend { a, b, .. }
        | Drawing::Fib { a, b, .. }
        | Drawing::Measure { a, b, .. } = d
    {
        for a in [a, b] {
            p.rounded(map.x(a.time) - 4.5, map.y(a.price) - 4.5, 9., 9., 4.5, palette.glow);
            p.rounded(map.x(a.time) - 3., map.y(a.price) - 3., 6., 6., 3., palette.background);
        }
    }
    if !panning
        && !analysis.dragging()
        && let (Some(pointer), Some(snapped)) = (analysis.pointer, analysis.snapped)
        && plot.contains(pointer)
    {
        let x = frame.hover.map_or(pointer.x, |c| map.x(c.time + interval / 2.));
        let y = map.y(snapped.price);
        let mut color = palette.foreground;
        color.a = 0.3;
        p.line(Point { x, y: plot.y }, Point { x, y: plot.bottom() }, color, 1., true);
        p.line(Point { x: plot.x, y }, Point { x: plot.right(), y }, color, 1., true);
        if geometry::snapped_to_candle(&frame, snapped) {
            p.rounded(x - 3., y - 3., 6., 6., 3., palette.foreground);
        }
        p.tag(format::price(snapped.price, decimals), plot.right() + 3., y, palette.foreground);
        if let Some(c) = frame.hover {
            let text = super::super::labels::candle_label(c.time, interval as u32, p.day_offset);
            let line = p.cache.line(text.clone(), palette.background, 11., p.window);
            let w = f32::from(line.width) as f64 + 12.;
            let left = (x - w / 2.).max(plot.x).min(plot.right() - w);
            p.tag(text, left, plot.bottom() + 14., palette.foreground);
        }
    }
    let index = frame.hover.and_then(|c| analysis.index_of(c.time));
    let mut x = plot.x + 8.;
    for series in &analysis.series {
        let color = match series.spec.id.as_str() {
            "ma1" => palette.violet,
            "ma2" => palette.amber,
            "ma3" => palette.mint,
            "boll" => palette.glow,
            _ => palette.foreground,
        };
        p.rect(x, plot.y + 8., 6., 6., color);
        x += 10.;
        let value = index
            .or_else(|| series.values.len().checked_sub(1))
            .and_then(|i| series.values.get(i))
            .filter(|v| v.is_finite())
            .map_or_else(|| "—".to_owned(), |v| format::price(*v, decimals));
        let text = format!("{} {}", series.spec.label(), value);
        x += p.text(text, x, plot.y + 3., color, 11.) + 12.;
    }
}
