use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    path::Path,
};

fn main() {
    text::compile();
    // The Windows manifest; ignored when building for macOS.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}

/// Compiles the catalogs: the app's (`locales/<tag>.json`) into `t!` and a
/// function per message (`src/i18n.rs` includes both), and checks the pages'
/// (`../src/locales/<tag>.json`), which i18next reads as they are.
///
/// What fails the build, in either: a message one language lacks or only it
/// has, and placeholders that differ from English. What fails it where `t!`
/// is written: a key the catalogs don't have, a placeholder missing or
/// misnamed.
mod text {
    use super::*;

    /// The language every other is checked against.
    const SOURCE: &str = "en";
    /// A key's Windows wording, where the platforms differ ("menu bar",
    /// "taskbar"): `onePinned_windows` beside `onePinned`.
    const WINDOWS: &str = "_windows";
    /// The CLDR plural categories, as i18next suffixes them. English has
    /// `_one` and `_other`; a language without a plural only `_other`.
    const PLURALS: [&str; 6] = ["_zero", "_one", "_two", "_few", "_many", "_other"];

    /// A language's messages by dotted key.
    type Catalog = BTreeMap<String, String>;

    pub fn compile() {
        let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
        let app = Catalogs::read(&Path::new(&manifest).join("locales"), "locales");
        let pages = Catalogs::read(&Path::new(&manifest).join("../src/locales"), "src/locales");
        assert_eq!(app.tags(), pages.tags(), "the app and its pages speak different languages");
        app.check(false);
        pages.check(true);
        // The pages take `common` from the app's catalogs (src/lib/i18n.ts).
        let shadowed: Vec<&String> =
            pages.source().keys().filter(|key| key.starts_with("common.")).collect();
        assert!(shadowed.is_empty(), "src/locales: `common` is the app's, but has {shadowed:?}");

        let windows = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
        let app = app.on_platform(windows);
        let mut code = app.code();
        // What the code names to say each message, for the test that every
        // message is said somewhere.
        let named = |keys: BTreeSet<&str>| -> String {
            keys.into_iter().fold(String::new(), |mut list, key| {
                write!(list, "{key:?}, ").unwrap();
                list
            })
        };
        write!(
            code,
            "\n/// The app's messages, as `t!` names them.\n\
             #[cfg(test)]\n\
             const APP_MESSAGES: &[&str] = &[{}];\n\n\
             /// The pages' messages, as `t` names them: without the plural form,\n\
             /// platform or context that its options pick.\n\
             #[cfg(test)]\n\
             const PAGE_MESSAGES: &[&str] = &[{}];\n",
            named(app.source().keys().map(String::as_str).collect()),
            named(pages.source().keys().map(|key| key.split('_').next().unwrap_or(key)).collect()),
        )
        .unwrap();
        let out = Path::new(&env::var("OUT_DIR").unwrap()).join("text.rs");
        fs::write(out, code).expect("write the generated text");
    }

    /// One directory's catalogs, the source language first.
    struct Catalogs {
        /// The directory, as messages about it name it.
        name: &'static str,
        by_tag: Vec<(String, Catalog)>,
    }

    impl Catalogs {
        fn read(dir: &Path, name: &'static str) -> Self {
            // A directory reruns this when anything in it changes.
            println!("cargo:rerun-if-changed={}", dir.display());
            let mut by_tag: Vec<(String, Catalog)> = fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
                .map(|path| {
                    let tag = path.file_stem().unwrap().to_string_lossy().into_owned();
                    let json = fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    let json: serde_json::Value = serde_json::from_str(&json)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    let mut catalog = Catalog::new();
                    flatten("", &json, &mut catalog);
                    (tag, catalog)
                })
                .collect();
            by_tag.sort_by_key(|(tag, _)| (tag != SOURCE, tag.clone()));
            assert!(
                by_tag.first().is_some_and(|(tag, _)| tag == SOURCE),
                "{name}/{SOURCE}.json is missing"
            );
            Self { name, by_tag }
        }

        fn tags(&self) -> Vec<&str> {
            self.by_tag.iter().map(|(tag, _)| tag.as_str()).collect()
        }

        fn source(&self) -> &Catalog {
            &self.by_tag[0].1
        }

        /// Every language says every message, with the source's placeholders.
        /// With `plurals`, a message may come in the plural forms its
        /// language has (the pages': i18next picks one by `count`, which a
        /// form may leave out, "1 more"); without, in none (the app's: `t!`
        /// can't pick).
        fn check(&self, plurals: bool) {
            let name = self.name;
            let message = |key: &str| -> String {
                let form = PLURALS.iter().find_map(|suffix| key.strip_suffix(suffix));
                assert!(plurals || form.is_none(), "{name}: {key} is a plural form; `t!` has none");
                form.unwrap_or(key).to_owned()
            };
            let source = self.source();
            let messages: BTreeSet<String> = source.keys().map(|key| message(key)).collect();
            for (tag, catalog) in &self.by_tag {
                let theirs: BTreeSet<String> = catalog.keys().map(|key| message(key)).collect();
                let missing: Vec<&String> = messages.difference(&theirs).collect();
                let extra: Vec<&String> = theirs.difference(&messages).collect();
                assert!(missing.is_empty(), "{name}/{tag}.json lacks {missing:?}");
                assert!(extra.is_empty(), "{name}/{tag}.json has {extra:?}; {SOURCE}.json doesn't");
                for (key, text) in catalog {
                    let base = message(key);
                    let said = source
                        .get(key)
                        .or_else(|| source.get(&format!("{base}_other")))
                        .unwrap_or_else(|| panic!("{name}/{tag}.json: {key} has no {SOURCE} form"));
                    let named = |text: &str| -> BTreeSet<String> {
                        let mut names: BTreeSet<String> = placeholders(text).into_iter().collect();
                        if *key != base {
                            names.remove("count");
                        }
                        names
                    };
                    assert_eq!(named(text), named(said), "{name}/{tag}.json: {key}'s placeholders");
                }
            }
        }

        /// The catalogs as one platform says them: a message with a Windows
        /// wording takes it there and loses it elsewhere.
        fn on_platform(mut self, windows: bool) -> Self {
            let name = self.name;
            for (tag, catalog) in &mut self.by_tag {
                let variants: Vec<String> =
                    catalog.keys().filter(|key| key.ends_with(WINDOWS)).cloned().collect();
                for variant in variants {
                    let text = catalog.remove(&variant).unwrap();
                    let key = variant.strip_suffix(WINDOWS).unwrap();
                    let other = catalog
                        .get_mut(key)
                        .unwrap_or_else(|| panic!("{name}/{tag}.json: {variant} without {key}"));
                    assert_eq!(
                        placeholders(&text),
                        placeholders(other),
                        "{name}/{tag}.json: {variant}'s placeholders"
                    );
                    if windows {
                        *other = text;
                    }
                }
            }
            self
        }

        /// `t!`, a macro with a rule per message, and the function each rule
        /// calls: `&'static str` for a message without placeholders, a
        /// `String` formatted from them otherwise.
        fn code(&self) -> String {
            let mut rules = String::new();
            let mut functions = String::new();
            let mut names = BTreeMap::new();
            for (key, english) in self.source() {
                let name = key.replace('.', "_").to_ascii_lowercase();
                if let Some(other) = names.insert(name.clone(), key) {
                    panic!("{key} and {other} would both be text::{name}");
                }
                let params = placeholders(english);
                let said = |text: fn(&str) -> String| -> String {
                    self.by_tag.iter().fold(String::new(), |mut arms, (tag, catalog)| {
                        let (locale, text) = (variant(tag), text(&catalog[key]));
                        writeln!(arms, "            Locale::{locale} => {text},").unwrap();
                        arms
                    })
                };
                writeln!(functions, "    /// {}", english.replace('\n', " ")).unwrap();
                if params.is_empty() {
                    writeln!(rules, "    ({key:?}) => {{ $crate::i18n::text::{name}() }};")
                        .unwrap();
                    writeln!(
                        functions,
                        "    pub fn {name}() -> &'static str {{\n        match super::current() {{\n{}        }}\n    }}\n",
                        said(|text| format!("{text:?}")),
                    )
                    .unwrap();
                } else {
                    let list = |item: fn(&String) -> String| -> String {
                        params.iter().map(item).collect::<Vec<_>>().join(", ")
                    };
                    writeln!(
                        rules,
                        "    ({key:?}, {} $(,)?) => {{ $crate::i18n::text::{name}({}) }};",
                        list(|p| format!("{p} = ${p}:expr")),
                        list(|p| format!("&${p}")),
                    )
                    .unwrap();
                    writeln!(
                        functions,
                        "    pub fn {name}({}) -> String {{\n        match super::current() {{\n{}        }}\n    }}\n",
                        list(|p| format!("{p}: &dyn Display")),
                        said(|text| format!("format!({:?})", to_format(text))),
                    )
                    .unwrap();
                }
            }
            format!(
                "// Generated by build.rs from locales/*.json.\n\n\
                 /// A message in the app's language: `t!(\"tray.quit\")` is a\n\
                 /// `&'static str`; with placeholders, named in the order English\n\
                 /// has them, `t!(\"tray.update\", version = v)` is a `String`.\n\
                 macro_rules! t {{\n{rules}    \
                 ($key:literal $($rest:tt)*) => {{\n        \
                 compile_error!(concat!(\"locales/{SOURCE}.json has no message \\\"\", $key, \"\\\" with these placeholders\"))\n    \
                 }};\n}}\n\n\
                 /// What `t!` calls: a function per message.\n\
                 #[allow(dead_code)]\n\
                 pub mod text {{\n    \
                 use std::fmt::Display;\n\n    \
                 use super::Locale;\n\n{functions}}}\n"
            )
        }
    }

    /// `zh-CN` as `i18n::Locale` names it: `ZhCn`.
    fn variant(tag: &str) -> String {
        tag.split('-')
            .map(|part| {
                let (first, rest) = part.split_at(1);
                first.to_ascii_uppercase() + &rest.to_ascii_lowercase()
            })
            .collect()
    }

    fn flatten(prefix: &str, value: &serde_json::Value, out: &mut Catalog) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    let key =
                        if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                    flatten(&key, value, out);
                }
            }
            serde_json::Value::String(text) => {
                out.insert(prefix.to_owned(), text.clone());
            }
            other => panic!("{prefix} is {other}, not text"),
        }
    }

    /// `%{name}` placeholders, each once, in order.
    fn placeholders(text: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for rest in text.split("%{").skip(1) {
            let name = rest.split_once('}').map(|(name, _)| name).unwrap_or_default();
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "bad placeholder in {text:?}"
            );
            if !found.iter().any(|f| f == name) {
                found.push(name.to_owned());
            }
        }
        found
    }

    /// A message as a `format!` string: `%{name}` is `{name}`, other braces
    /// are literal.
    fn to_format(text: &str) -> String {
        let mut format = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("%{") {
            format.push_str(&rest[..at].replace('{', "{{").replace('}', "}}"));
            let (name, after) = rest[at + 2..].split_once('}').expect("a closed placeholder");
            format.push('{');
            format.push_str(name);
            format.push('}');
            rest = after;
        }
        format.push_str(&rest.replace('{', "{{").replace('}', "}}"));
        format
    }
}
