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

/// Compiles the catalogs into `t!` and checks keys and placeholders across
/// languages.
mod text {
    use super::*;

    /// The language every other is checked against.
    const SOURCE: &str = "en";
    /// The surfaces a language's catalog is split into, one file each: `app`
    /// is the bar and its menus, `common` what the windows share, the rest
    /// one window each.
    const SURFACES: [&str; 5] = ["app", "common", "settings", "chart", "holdings"];
    /// A key's Windows wording, where the platforms differ ("menu bar",
    /// "taskbar"): `onePinned_windows` beside `onePinned`.
    const WINDOWS: &str = "_windows";
    /// i18next-style plural suffixes; a catalog with one in a key is an
    /// error, since `t!` picks no plural form.
    const PLURALS: [&str; 6] = ["_zero", "_one", "_two", "_few", "_many", "_other"];
    /// Key prefixes of messages nothing says yet: the windows that will say
    /// them don't exist. Each window's task shortens this list; when it is
    /// empty, every message is said somewhere.
    const UNSAID: [&str; 8] = [
        "common.",
        "loading",
        "market.",
        "interval.",
        "windowControls.",
        "settings.",
        "chart.",
        "holdings.",
    ];

    /// A language's messages by dotted key.
    type Catalog = BTreeMap<String, String>;

    pub fn compile() {
        let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
        let app = Catalogs::read(&Path::new(&manifest).join("locales"));
        app.check();

        let windows = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
        let app = app.on_platform(windows);
        let mut code = app.code();
        // What the code names to say each message, for the test that every
        // message is said somewhere.
        let said: BTreeSet<&str> = app
            .source()
            .keys()
            .map(String::as_str)
            .filter(|key| !UNSAID.iter().any(|prefix| key.starts_with(prefix)))
            .collect();
        let named = said.into_iter().fold(String::new(), |mut list, key| {
            write!(list, "{key:?}, ").unwrap();
            list
        });
        write!(
            code,
            "\n/// The app's messages, as `t!` names them.\n\
             #[cfg(test)]\n\
             const APP_MESSAGES: &[&str] = &[{}];\n",
            named,
        )
        .unwrap();
        let out = Path::new(&env::var("OUT_DIR").unwrap()).join("text.rs");
        fs::write(out, code).expect("write the generated text");
    }

    /// The merged catalogs, the source language first.
    struct Catalogs {
        /// The directory, as messages about it name it.
        name: &'static str,
        by_tag: Vec<(String, Catalog)>,
    }

    impl Catalogs {
        fn read(dir: &Path) -> Self {
            // A directory reruns this when anything in it changes.
            println!("cargo:rerun-if-changed={}", dir.display());
            let mut by_tag: Vec<(String, Catalog)> = fs::read_dir(dir)
                .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.is_dir())
                .map(|dir| {
                    let tag = dir.file_name().unwrap().to_string_lossy().into_owned();
                    let catalog = catalog(&dir, &tag);
                    (tag, catalog)
                })
                .collect();
            by_tag.sort_by_key(|(tag, _)| (tag != SOURCE, tag.clone()));
            assert!(
                by_tag.first().is_some_and(|(tag, _)| tag == SOURCE),
                "{}/{SOURCE} is missing",
                dir.display()
            );
            Self { name: "locales", by_tag }
        }

        fn source(&self) -> &Catalog {
            &self.by_tag[0].1
        }

        /// Every language says every message, with the source's placeholders.
        fn check(&self) {
            let name = self.name;
            let source = self.source();
            for (tag, catalog) in &self.by_tag {
                for key in catalog.keys() {
                    assert!(
                        !PLURALS.iter().any(|suffix| key.ends_with(suffix)),
                        "{name}/{tag}: {key} is a plural form; `t!` picks none"
                    );
                }
                let missing: Vec<&String> =
                    source.keys().filter(|k| !catalog.contains_key(*k)).collect();
                let extra: Vec<&String> =
                    catalog.keys().filter(|k| !source.contains_key(*k)).collect();
                assert!(missing.is_empty(), "{name}/{tag} lacks {missing:?}");
                assert!(extra.is_empty(), "{name}/{tag} has {extra:?}; {SOURCE} doesn't");
                for (key, text) in catalog {
                    let said = &source[key];
                    let named = |text: &str| -> BTreeSet<String> {
                        placeholders(text).into_iter().collect()
                    };
                    assert_eq!(named(text), named(said), "{name}/{tag}: {key}'s placeholders");
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
                        .unwrap_or_else(|| panic!("{name}/{tag}: {variant} without {key}"));
                    assert_eq!(
                        placeholders(&text),
                        placeholders(other),
                        "{name}/{tag}: {variant}'s placeholders"
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
                "// Generated by build.rs from locales/.\n\n\
                 /// A message in the app's language: `t!(\"tray.quit\")` is a\n\
                 /// `&'static str`; with placeholders, named in the order English\n\
                 /// has them, `t!(\"tray.update\", version = v)` is a `String`.\n\
                 macro_rules! t {{\n{rules}    \
                 ($key:literal $($rest:tt)*) => {{\n        \
                 compile_error!(concat!(\"locales/{SOURCE} has no message \\\"\", $key, \"\\\" with these placeholders\"))\n    \
                 }};\n}}\n\n\
                 /// What `t!` calls: a function per message.\n\
                 #[allow(dead_code)]\n\
                 pub mod text {{\n    \
                 use std::fmt::Display;\n\n    \
                 use super::Locale;\n\n{functions}}}\n"
            )
        }
    }

    /// One language's catalog: every surface file it has, merged. A key in
    /// two surfaces is a mistake — nothing resolves by priority.
    fn catalog(dir: &Path, tag: &str) -> Catalog {
        let mut merged = Catalog::new();
        for surface in SURFACES {
            let path = dir.join(format!("{surface}.json"));
            if !path.exists() {
                // Only the app's own surface has to be there; a window's
                // surface may still be empty.
                assert!(surface == "app", "locales/{tag}: {} is missing", path.display());
                continue;
            }
            let json =
                fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let json: serde_json::Value =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut one = Catalog::new();
            flatten("", &json, &mut one);
            for (key, text) in one {
                let repeat = merged.insert(key.clone(), text).is_none();
                assert!(
                    repeat,
                    "locales/{tag}: {surface}.json repeats a key another surface has: {key}"
                );
            }
        }
        merged
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
