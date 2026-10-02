//! The language the app speaks: the one the settings pick, or the first of
//! the system's preferred languages it has text in, else English.
//!
//! Rust's own text (the bar and its menus, window titles, errors) is
//! `t!("tray.quit")`, compiled by `build.rs` from `locales/<tag>.json`: a key
//! or placeholder that isn't there doesn't compile, and every language has
//! every message. The pages translate their own text with the same
//! placeholders (`src/lib/i18n.ts`) and share `common` from these catalogs;
//! they ask for the locale as they load and hear of changes (`window.rs`).
//! What travels to a page as data (wallets, contract kinds, intervals) goes
//! as a code, for the page to name in its own language.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/text.rs"));

/// What the settings ask for: the system's language, or one of the app's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(untagged)]
    Only(Locale),
}

impl Language {
    pub fn locale(self) -> Locale {
        match self {
            Self::System => Locale::of_system(),
            Self::Only(locale) => locale,
        }
    }
}

/// A language the app has text in, named by the BCP 47 tag of its catalogs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Locale {
    #[default]
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "ja")]
    Ja,
}

impl Locale {
    /// In declaration order: a locale is at its discriminant.
    const ALL: [Self; 3] = [Self::En, Self::ZhCn, Self::Ja];

    /// The first of the system's preferred languages that the app speaks.
    fn of_system() -> Self {
        sys_locale::get_locales().find_map(|tag| Self::matching(&tag)).unwrap_or_default()
    }

    /// The locale that speaks a system language tag (`en-GB`, `ja-JP`,
    /// `zh-Hans-CN`, `zh_CN`), if any. Traditional Chinese isn't Simplified:
    /// as when the system matches a bundle's localizations, the next
    /// preferred language decides then.
    fn matching(tag: &str) -> Option<Self> {
        let tag = tag.to_ascii_lowercase();
        let mut subtags = tag.split(['-', '_']);
        match subtags.next()? {
            "en" => Some(Self::En),
            "ja" => Some(Self::Ja),
            "zh" => {
                let traditional =
                    subtags.any(|subtag| matches!(subtag, "hant" | "tw" | "hk" | "mo"));
                (!traditional || tag.contains("hans")).then_some(Self::ZhCn)
            }
            _ => None,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(Locale::En as u8);

/// Speaks `locale` from now on, on every thread.
pub fn set(locale: Locale) {
    CURRENT.store(locale as u8, Ordering::Relaxed);
}

/// The locale the app speaks now.
pub fn current() -> Locale {
    Locale::ALL[usize::from(CURRENT.load(Ordering::Relaxed))]
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    #[test]
    fn system_tags_match_the_locales() {
        for (tag, locale) in [
            ("en-US", Some(Locale::En)),
            ("en_GB", Some(Locale::En)),
            ("ja-JP", Some(Locale::Ja)),
            ("zh-Hans-CN", Some(Locale::ZhCn)),
            ("zh-CN", Some(Locale::ZhCn)),
            ("zh-SG", Some(Locale::ZhCn)),
            ("zh", Some(Locale::ZhCn)),
            ("zh-Hans-HK", Some(Locale::ZhCn)),
            ("zh-Hant-TW", None),
            ("zh-TW", None),
            ("zh-HK", None),
            ("fr-FR", None),
        ] {
            assert_eq!(Locale::matching(tag), locale, "{tag}");
        }
    }

    // The settings file and the pages name a language by its tag, or `system`.
    #[test]
    fn languages_are_written_as_tags() {
        for (language, json) in [
            (Language::System, r#""system""#),
            (Language::Only(Locale::En), r#""en""#),
            (Language::Only(Locale::ZhCn), r#""zh-CN""#),
            (Language::Only(Locale::Ja), r#""ja""#),
        ] {
            assert_eq!(serde_json::to_string(&language).unwrap(), json);
            assert_eq!(serde_json::from_str::<Language>(json).unwrap(), language);
        }
        assert!(serde_json::from_str::<Language>(r#""fr""#).is_err());
    }

    // `current` reads a locale back by its discriminant.
    #[test]
    fn locales_are_listed_in_order() {
        for (index, locale) in Locale::ALL.into_iter().enumerate() {
            assert_eq!(locale as usize, index);
        }
    }

    fn sources(dir: &Path, extensions: &[&str], out: &mut String) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "locales") {
                    sources(&path, extensions, out);
                }
            } else if path.extension().is_some_and(|e| extensions.iter().any(|x| e == *x)) {
                out.push_str(&fs::read_to_string(path).unwrap());
                out.push('\n');
            }
        }
    }

    // A message nothing says is dead weight in every language. The app's are
    // said through `t!`; `common` is the pages' too; the pages name theirs as
    // string literals, or a family of them by a template's prefix
    // (`holdings.wallet.${…}`).
    #[test]
    fn every_message_is_said_somewhere() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut rust = String::new();
        sources(&root.join("src"), &["rs"], &mut rust);
        // Without whitespace, so a `t!(` that rustfmt broke over lines reads whole.
        let rust: String = rust.split_whitespace().collect();
        let mut pages = String::new();
        sources(&root.join("../src"), &["ts", "tsx"], &mut pages);
        let prefixes: Vec<&str> = pages
            .split('`')
            .skip(1)
            .step_by(2)
            .filter_map(|template| template.split_once("${").map(|(head, _)| head))
            .filter(|head| head.ends_with('.') && !head.contains(' '))
            .collect();
        let in_pages = |key: &str| {
            pages.contains(&format!("\"{key}\""))
                || prefixes.iter().any(|prefix| key.starts_with(prefix))
        };

        let unsaid: Vec<&&str> = APP_MESSAGES
            .iter()
            .filter(|key| {
                !rust.contains(&format!("t!(\"{key}\""))
                    && !(key.starts_with("common.") && in_pages(key))
            })
            .collect();
        assert!(unsaid.is_empty(), "locales: nothing says {unsaid:?}");
        let unsaid: Vec<&&str> = PAGE_MESSAGES.iter().filter(|key| !in_pages(key)).collect();
        assert!(unsaid.is_empty(), "src/locales: nothing says {unsaid:?}");
    }
}
