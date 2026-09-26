//! How Windows is set to look: light or dark for the taskbar (the Windows
//! mode) and for apps, high contrast, and the accent color.

use serde::Serialize;
use windows::{
    UI::ViewManagement::{UIColorType, UISettings},
    Win32::{
        Graphics::Gdi::{COLOR_HIGHLIGHT, COLOR_WINDOWTEXT, GetSysColor},
        System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            WindowsAndMessaging::{
                SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
            },
        },
    },
    core::{PCWSTR, w},
};

use super::portable::color::{Palette, Rgba, Tone};

const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
const EXPLORER_ADVANCED: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced");

/// Read again whenever Windows reports a settings change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub taskbar: Tone,
    /// Windows, menus and pages.
    pub apps: Tone,
    /// A high-contrast theme's text and highlight colors.
    pub contrast: Option<(Rgba, Rgba)>,
    /// Windows 11 moves the Widgets button (with the weather) beside the
    /// notification area when the taskbar is aligned left, where the ticker
    /// sits.
    pub widgets_beside_tray: bool,
}

impl Look {
    pub fn current() -> Self {
        Self {
            // The defaults of a Windows 10 install, where these can be missing.
            taskbar: tone(w!("SystemUsesLightTheme"), Tone::Dark),
            apps: tone(w!("AppsUseLightTheme"), Tone::Light),
            contrast: high_contrast(),
            widgets_beside_tray: windows_version::OsVersion::current().build >= 22000
                && dword(EXPLORER_ADVANCED, w!("TaskbarAl")) == Some(0)
                && dword(EXPLORER_ADVANCED, w!("TaskbarDa")) != Some(0),
        }
    }

    /// What the ticker and tray icon draw with.
    pub fn taskbar_palette(&self) -> Palette {
        self.palette(self.taskbar)
    }

    /// What the dropdown's marks draw with.
    pub fn menu_palette(&self) -> Palette {
        self.palette(self.apps)
    }

    fn palette(&self, tone: Tone) -> Palette {
        match self.contrast {
            Some((text, highlight)) => Palette::high_contrast(text, highlight),
            None => Palette::taskbar(tone),
        }
    }
}

fn tone(value: PCWSTR, missing: Tone) -> Tone {
    match dword(PERSONALIZE, value) {
        Some(0) => Tone::Dark,
        Some(_) => Tone::Light,
        None => missing,
    }
}

fn dword(key: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `data` holds `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data)
}

fn high_contrast() -> Option<(Rgba, Rgba)> {
    let mut settings =
        HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    // SAFETY: `settings` is the struct SPI_GETHIGHCONTRAST fills, with its size set.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            settings.cbSize,
            Some((&raw mut settings).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if read.is_err() || (settings.dwFlags.0 & HCF_HIGHCONTRASTON.0) == 0 {
        return None;
    }
    // SAFETY: plain reads of the theme's system colors.
    let (text, highlight) =
        unsafe { (GetSysColor(COLOR_WINDOWTEXT), GetSysColor(COLOR_HIGHLIGHT)) };
    Some((Rgba::from_colorref(text), Rgba::from_colorref(highlight)))
}

/// The accent color the pages use for controls: Windows 11's
/// AccentFillColorDefault is the accent's first darker shade on light
/// surfaces and its second lighter shade on dark ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Accent {
    pub on_light: String,
    pub on_dark: String,
}

impl Default for Accent {
    /// Windows' default blue.
    fn default() -> Self {
        Self { on_light: "#005fb8".to_owned(), on_dark: "#60cdff".to_owned() }
    }
}

impl Accent {
    pub fn current() -> Self {
        let read = || -> windows::core::Result<Self> {
            let settings = UISettings::new()?;
            let hex = |kind: UIColorType| {
                settings.GetColorValue(kind).map(|c| format!("#{:02x}{:02x}{:02x}", c.R, c.G, c.B))
            };
            Ok(Self {
                on_light: hex(UIColorType::AccentDark1)?,
                on_dark: hex(UIColorType::AccentLight2)?,
            })
        };
        read().unwrap_or_else(|e| {
            log::debug!("no accent color: {e}");
            Self::default()
        })
    }
}
