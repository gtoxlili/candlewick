use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    io::Read as _,
    path::Path,
};

fn main() {
    thsdk::fetch();
    text::compile();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = env::var("CARGO_PKG_VERSION").unwrap();
        let parts: Vec<_> = version.split('.').collect();
        let rc = format!(
            r#"1 24 "{}"
1 ICON "{}"
1 VERSIONINFO
 FILEVERSION {},{},{},0
 PRODUCTVERSION {},{},{},0
 FILEOS 0x40004
 FILETYPE 1
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904b0"
  BEGIN
   VALUE "FileDescription", "Candlewick\0"
   VALUE "ProductName", "Candlewick\0"
   VALUE "FileVersion", "{}\0"
   VALUE "ProductVersion", "{}\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
"#,
            Path::new("windows/app.manifest")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
            Path::new("icons/icon.ico")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
            parts[0],
            parts[1],
            parts[2],
            parts[0],
            parts[1],
            parts[2],
            version,
            version
        );
        let path = Path::new(&env::var("OUT_DIR").unwrap()).join("candlewick.rc");
        fs::write(&path, rc).unwrap();
        embed_resource::compile(path, embed_resource::NONE).manifest_required().unwrap();
        println!("cargo:rerun-if-changed=windows/app.manifest");
        println!("cargo:rerun-if-changed=icons/icon.ico");
    }
}

/// Fetches thsdk's runtime, which signs in to a Tonghuashun account and reads
/// its watchlist (`src/market/ths/thsdk.rs`), into `thsdk/`: the library for
/// the platform being built and its license. Both come from thsdk's release
/// on PyPI, pinned by hash; nothing is fetched once the files are there.
mod thsdk {
    use super::*;
    use flate2::read::GzDecoder;
    use sha2::{Digest as _, Sha256};

    const VERSION: &str = "2.0.2";
    const URL: &str = "https://files.pythonhosted.org/packages/e4/54/53227fca0b82e93e6292e0703ab412e517c87c632f744e82977738da23f3/thsdk-2.0.2.tar.gz";
    const SHA256: &str = "12b950a81df0ddc80f577dca3aed11660e8f0c5ef59b5ef4a21846ae3915f40a";

    /// The library inside the archive for each platform the app ships for,
    /// and the file it becomes in `thsdk/`.
    fn library() -> Option<(&'static str, &'static str)> {
        match (
            env::var("CARGO_CFG_TARGET_OS").as_deref(),
            env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
        ) {
            (Ok("macos"), Ok("aarch64")) => Some(("libs/darwin/arm64/hq.dylib", "hq.dylib")),
            (Ok("windows"), Ok("x86_64")) => Some(("libs/windows/amd64/hq.dll", "hq.dll")),
            _ => None,
        }
    }

    pub fn fetch() {
        println!("cargo:rerun-if-changed=build.rs");
        let Some((inside, name)) = library() else { return };
        let out = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("thsdk");
        let wanted = [
            (format!("thsdk-{VERSION}/thsdk/{inside}"), name),
            (format!("thsdk-{VERSION}/LICENSE"), "LICENSE"),
        ];
        // The stamp carries the hash: a new pin refetches, an interrupted
        // fetch (stamp written last) refetches too.
        let current = fs::read_to_string(out.join("VERSION")).is_ok_and(|s| s.trim() == SHA256)
            && wanted.iter().all(|(_, file)| out.join(file).exists());
        if current {
            return;
        }

        let archive = download();
        let hash = Sha256::digest(&archive);
        assert_eq!(format!("{hash:x}"), SHA256, "thsdk {VERSION}: sha256 of {URL}");

        fs::create_dir_all(&out).expect("create thsdk/");
        let tar = GzDecoder::new(&archive[..]);
        let mut tar = tar::Archive::new(tar);
        let mut found = 0;
        for entry in tar.entries().expect("thsdk: read the archive") {
            let mut entry = entry.expect("thsdk: read the archive");
            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let Some((_, file)) = wanted.iter().find(|(inside, _)| *inside == path) else {
                continue;
            };
            let mut data = Vec::new();
            entry.read_to_end(&mut data).expect("thsdk: read the archive");
            let file = out.join(file);
            fs::write(&file, data).expect("write the library");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = if file == out.join("LICENSE") { 0o644 } else { 0o755 };
                fs::set_permissions(&file, fs::Permissions::from_mode(mode))
                    .expect("set the library's permissions");
            }
            found += 1;
        }
        assert!(
            found == wanted.len(),
            "thsdk: {URL} lacks {}",
            wanted.iter().map(|(inside, _)| inside.as_str()).collect::<Vec<_>>().join(" or ")
        );
        fs::write(out.join("VERSION"), format!("{SHA256}\n")).expect("write thsdk/VERSION");
    }

    fn download() -> Vec<u8> {
        let agent = ureq::config::Config::builder().build().new_agent();
        let mut response =
            agent.get(URL).call().unwrap_or_else(|e| panic!("thsdk: {URL} answered {e}"));
        // The archive is bigger than the 10 MB the shorthand readers allow.
        response
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_vec()
            .unwrap_or_else(|e| panic!("thsdk: reading {URL}: {e}"))
    }
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
    /// Plural suffixes; a catalog with one in a key is an error, since `t!`
    /// picks no plural form.
    const PLURALS: [&str; 6] = ["_zero", "_one", "_two", "_few", "_many", "_other"];
    /// Key prefixes the every-message-is-said check leaves out, because no
    /// code says those messages.
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
