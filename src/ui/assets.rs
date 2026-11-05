//! The icons the app draws.
//!
//! GPUI resolves every SVG through the application's asset source. This source
//! embeds the icons named by app code and the icons GPUI Kit's components use
//! by default, so the binary carries no part of the full Lucide catalog. The
//! SVG files and their license live in `assets/icons`.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

#[cfg(test)]
use gpui_kit::assets::IconName;

struct Icon {
    path: &'static str,
    bytes: &'static [u8],
}

macro_rules! icons {
    ($($name:ident => $file:literal),* $(,)?) => {
        const ICONS: &[Icon] = &[
            $(Icon {
                path: concat!("icons/", $file, ".svg"),
                bytes: include_bytes!(concat!("../../assets/icons/", $file, ".svg")),
            }),*
        ];

        /// The source name of an embedded icon, for tests.
        #[cfg(test)]
        fn icon_name_of(path: &str) -> Option<&'static str> {
            match path {
                $(concat!("icons/", $file, ".svg") => Some(stringify!($name)),)*
                _ => None,
            }
        }

        /// The `IconName` a source name stands for, for tests.
        #[cfg(test)]
        fn icon_name(name: &str) -> Option<IconName> {
            match name {
                $(stringify!($name) => Some(IconName::$name),)*
                _ => None,
            }
        }
    };
}

icons! {
    // Named by the chart view.
    ChartCandlestick => "chart-candlestick",
    ChartLine => "chart-line",
    ChartNoAxesGantt => "chart-no-axes-gantt",
    ChevronsUpDown => "chevrons-up-down",
    Eraser => "eraser",
    ExternalLink => "external-link",
    Minus => "minus",
    MousePointer2 => "mouse-pointer-2",
    Ruler => "ruler",
    Sigma => "sigma",
    TrendingUp => "trending-up",
    Undo2 => "undo-2",

    // Drawn by GPUI Kit components the windows use: the select, text inputs,
    // title bar controls, alerts, buttons and spinner, plus the searchable
    // list and menu rows inside them.
    Asterisk => "asterisk",
    CaseSensitive => "case-sensitive",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    CircleCheck => "circle-check",
    CircleX => "circle-x",
    Close => "close",
    Eye => "eye",
    EyeOff => "eye-off",
    Inbox => "inbox",
    Info => "info",
    Loader => "loader",
    Plus => "plus",
    Replace => "replace",
    Search => "search",
    TriangleAlert => "triangle-alert",
    WindowClose => "window-close",
    WindowMaximize => "window-maximize",
    WindowMinimize => "window-minimize",
    WindowRestore => "window-restore",
}

/// Embeds [`ICONS`] and nothing else.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|icon| icon.path == path).map(|icon| Cow::Borrowed(icon.bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|icon| icon.path.starts_with(path))
            .map(|icon| icon.path.into())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
    };

    use super::*;

    #[test]
    fn each_embedded_icon_matches_its_icon_name() {
        for icon in ICONS {
            let name = icon_name_of(icon.path).expect("a name for every embedded path");
            let name = icon_name(name).expect("the table names a real IconName");
            assert_eq!(name.path().as_ref(), icon.path);
            assert!(!icon.bytes.is_empty(), "{}", icon.path);
        }
    }

    #[test]
    fn each_embedded_path_appears_once() {
        let mut paths = BTreeSet::new();
        for icon in ICONS {
            assert!(paths.insert(icon.path), "duplicate path {}", icon.path);
        }
    }

    #[test]
    fn icons_named_in_source_are_embedded() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for dir in ["src", "tests"] {
            collect(&root.join(dir), &mut files);
        }
        let mut missing = Vec::new();
        for file in files {
            let source = fs::read_to_string(&file).expect("source file");
            for name in icon_names(&source) {
                let embedded = ICONS
                    .iter()
                    .any(|icon| icon_name_of(icon.path).is_some_and(|embedded| embedded == name));
                if !embedded {
                    missing.push(format!("{}: IconName::{name}", file.display()));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "icons named in source but not embedded in assets/icons:\n{}",
            missing.join("\n")
        );
    }

    fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                collect(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }

    fn icon_names(source: &str) -> BTreeSet<&str> {
        const PREFIX: &str = "IconName::";
        let mut names = BTreeSet::new();
        let mut rest = source;
        while let Some(index) = rest.find(PREFIX) {
            rest = &rest[index + PREFIX.len()..];
            let len =
                rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
            if len > 0 {
                names.insert(&rest[..len]);
            }
        }
        names
    }
}
