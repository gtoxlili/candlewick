//! Candlewick colors and the GPUI Kit theme.

use gpui_kit::{App as GpuiApp, Global, Hsla, Rgba, component::Theme, px, rgb};

use crate::{app::App, model::ColorScheme};

pub const GREEN: ((u8, u8, u8), (u8, u8, u8)) = ((0x00, 0x80, 0x47), (0x30, 0xd7, 0x92));
pub const RED: ((u8, u8, u8), (u8, u8, u8)) = ((0xd4, 0x2e, 0x3d), (0xff, 0x6e, 0x74));

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub border: Hsla,
    pub input: Hsla,
    pub panel: Hsla,
    pub fill: Hsla,
    pub fill_strong: Hsla,
    pub popover: Hsla,
    pub glow: Hsla,
    pub mint: Hsla,
    pub violet: Hsla,
    pub amber: Hsla,
    pub green: Hsla,
    pub red: Hsla,
    pub up: Hsla,
    pub down: Hsla,
    pub series: [Hsla; 7],
    pub primary_foreground: Hsla,
}

impl Global for Palette {}

#[derive(Clone, Copy, PartialEq, Eq)]
struct AppliedTheme {
    dark: bool,
    scheme: ColorScheme,
}

impl Global for AppliedTheme {}

fn color(value: u32, alpha: f32) -> Hsla {
    let mut color: Rgba = rgb(value);
    color.a = alpha;
    color.into()
}

fn tuple(value: (u8, u8, u8)) -> Hsla {
    color((u32::from(value.0) << 16) | (u32::from(value.1) << 8) | u32::from(value.2), 1.)
}

impl Palette {
    pub fn new(dark: bool, scheme: ColorScheme) -> Self {
        let (ink, line, surface, glow, mint, violet, amber, fifth, popover, primary_foreground) =
            if dark {
                (
                    0xeef2f9, 0xffffff, 0xffffff, 0x49c8f3, 0x56ecb5, 0xaaa0fb, 0xf7b83d, 0xc1a9ee,
                    0x23262e, 0x07121e,
                )
            } else {
                (
                    0x1a1f2e, 0x161d37, 0xffffff, 0x0076af, 0x009365, 0x7456e0, 0xc5770f, 0x7b63a3,
                    0xfafcff, 0xffffff,
                )
            };
        let green = tuple(if dark { GREEN.1 } else { GREEN.0 });
        let red = tuple(if dark { RED.1 } else { RED.0 });
        let (up, down) = match scheme {
            ColorScheme::GreenUp => (green, red),
            ColorScheme::RedUp => (red, green),
        };
        let glow = color(glow, 1.);
        let mint = color(mint, 1.);
        let violet = color(violet, 1.);
        let amber = color(amber, 1.);
        Self {
            background: color(if dark { 0x16181f } else { 0xf8fafd }, 1.),
            foreground: color(ink, 0.92),
            muted: color(ink, 0.56),
            faint: color(ink, 0.36),
            border: color(line, 0.1),
            input: color(line, if dark { 0.13 } else { 0.12 }),
            panel: color(surface, if dark { 0.035 } else { 0.5 }),
            fill: color(line, if dark { 0.055 } else { 0.045 }),
            fill_strong: color(line, if dark { 0.09 } else { 0.075 }),
            popover: color(popover, if dark { 0.94 } else { 0.92 }),
            glow,
            mint,
            violet,
            amber,
            green,
            red,
            up,
            down,
            series: [
                glow,
                violet,
                mint,
                amber,
                color(fifth, 1.),
                color(ink, 0.3),
                color(ink, if dark { 0.16 } else { 0.18 }),
            ],
            primary_foreground: color(primary_foreground, 1.),
        }
    }

    pub fn current(app: &App, cx: &GpuiApp) -> Self {
        let dark = Theme::global(cx).is_dark();
        if let Some(applied) = cx.try_global::<AppliedTheme>()
            && applied.dark == dark
            && let Some(palette) = cx.try_global::<Self>()
        {
            return *palette;
        }
        Self::new(dark, app.shared().model().settings.color_scheme)
    }
}

pub fn apply(app: &App, dark: bool, cx: &mut GpuiApp) {
    let scheme = app.shared().model().settings.color_scheme;
    let target = AppliedTheme { dark, scheme };
    if cx.try_global::<AppliedTheme>().copied() == Some(target)
        && Theme::global(cx).is_dark() == dark
    {
        return;
    }
    let p = Palette::new(dark, scheme);
    cx.set_global(p);
    cx.set_global(target);
    Theme::change(
        if dark {
            gpui_kit::component::ThemeMode::Dark
        } else {
            gpui_kit::component::ThemeMode::Light
        },
        None,
        cx,
    );
    #[cfg(target_os = "windows")]
    let windows_font = {
        static FONT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
        *FONT.get_or_init(|| {
            if cx.text_system().all_font_names().iter().any(|name| name == "Segoe UI Variable Text")
            {
                "Segoe UI Variable Text"
            } else {
                "Segoe UI"
            }
        })
    };
    Theme::update(cx, |theme| {
        theme.font_size = px(if cfg!(target_os = "windows") { 14. } else { 13. });
        #[cfg(target_os = "windows")]
        {
            theme.font_family = windows_font.into();
        }
        theme.radius = px(10.);
        theme.radius_lg = px(18.);
        let c = &mut theme.colors;
        c.background = color(0xffffff, if dark { 0.16 } else { 1. });
        c.foreground = p.foreground;
        c.border = p.border;
        c.input = p.input;
        c.primary = p.glow;
        c.primary_hover = p.glow;
        c.primary_active = p.glow;
        c.primary_foreground = p.primary_foreground;
        c.button_primary = p.glow;
        c.button_primary_hover = p.glow;
        c.button_primary_active = p.glow;
        c.button_primary_foreground = p.primary_foreground;
        c.button = p.fill_strong;
        c.button_hover = p.fill;
        c.button_active = p.fill_strong;
        c.button_foreground = p.foreground;
        c.button_secondary = p.fill_strong;
        c.button_secondary_hover = p.fill;
        c.button_secondary_active = p.fill_strong;
        c.button_secondary_foreground = p.foreground;
        c.danger = p.red;
        c.danger_hover = p.red;
        c.danger_active = p.red;
        c.danger_foreground = p.primary_foreground;
        c.button_danger = p.red;
        c.button_danger_hover = p.red;
        c.button_danger_active = p.red;
        c.button_danger_foreground = p.primary_foreground;
        c.info = p.glow;
        c.info_hover = p.glow;
        c.info_active = p.glow;
        c.info_foreground = p.primary_foreground;
        c.success = p.green;
        c.success_hover = p.green;
        c.success_active = p.green;
        c.success_foreground = p.primary_foreground;
        c.warning = p.amber;
        c.warning_hover = p.amber;
        c.warning_active = p.amber;
        c.secondary = p.fill_strong;
        c.secondary_hover = p.fill;
        c.secondary_active = p.fill_strong;
        c.secondary_foreground = p.foreground;
        c.accent = p.fill_strong;
        c.accent_foreground = p.foreground;
        c.muted = p.fill_strong;
        c.muted_foreground = p.muted;
        c.popover = p.popover;
        c.popover_foreground = p.foreground;
        c.ring = p.glow.alpha(0.5);
        c.caret = p.glow;
        c.selection = p.glow.alpha(0.2);
        c.link = p.glow;
        c.link_hover = p.glow;
        c.link_active = p.glow;
        c.list = p.panel;
        c.list_hover = p.fill;
        c.list_active = p.fill_strong;
        c.list_active_border = p.glow;
        c.list_head = p.fill;
        c.list_even = p.fill;
        c.table = p.panel;
        c.table_hover = p.fill;
        c.table_active = p.fill_strong;
        c.table_active_border = p.glow;
        c.table_head = p.fill;
        c.table_row_border = p.border;
        c.switch = p.fill_strong;
        c.tab_bar_segmented = p.fill_strong;
        c.tab_foreground = p.muted;
        c.tab_active_foreground = p.foreground;
        c.chart_1 = p.series[0];
        c.chart_2 = p.series[1];
        c.chart_3 = p.series[2];
        c.chart_4 = p.series[3];
        c.chart_5 = p.series[4];
        c.chart_bullish = p.up;
        c.chart_bearish = p.down;
        c.chart_grid = p.border;
        c.title_bar = color(0, 0.);
        c.title_bar_border = color(0, 0.);
    });
}
