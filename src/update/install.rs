//! Installing a downloaded update.
//!
//! macOS: the archive holds the new `.app`. It is unpacked into a staging
//! directory beside the installed bundle, checked for being a newer copy of
//! Candlewick, and then moved into place by two renames: the old bundle
//! aside, the new one where it was. When the second rename fails the first is
//! undone, so an install that cannot finish leaves the app the user had.
//!
//! Windows: the archive is the NSIS installer. It runs in passive mode,
//! installing for the current user, and starts the app again when it is done.

#[cfg(target_os = "macos")]
pub use self::macos::{install, installed_bundle};
#[cfg(windows)]
pub use self::windows::install;

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        fs, io,
        path::{Component, Path, PathBuf},
    };

    use cargo_packager_updater::semver::Version;

    use crate::app::IDENTIFIER;

    /// Replaces the bundle at `bundle` with the one in `bytes`.
    pub fn install(bundle: &Path, bytes: &[u8]) -> Result<(), InstallError> {
        let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("valid package version");
        install_with(&RealFs, bundle, bytes, &current)
    }

    /// The app bundle the running executable is in.
    pub fn installed_bundle() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        exe.ancestors()
            .find(|dir| dir.extension().is_some_and(|extension| extension == "app"))
            .map(Path::to_path_buf)
    }

    fn install_with(
        fs: &dyn Fs,
        bundle: &Path,
        bytes: &[u8],
        current: &Version,
    ) -> Result<(), InstallError> {
        // `installed_bundle` always has one; a path without a folder fails in
        // the staging step, with the path in the message.
        let parent = bundle.parent().unwrap_or(bundle);
        // Beside the bundle: the new app and the old one stay on one volume,
        // so the swap is two renames and never a copy. The directory deletes
        // itself, which is also what removes the old bundle afterwards.
        let staging = tempfile::Builder::new()
            .prefix(".candlewick-update-")
            .tempdir_in(parent)
            .map_err(|source| InstallError::Staging { path: parent.to_path_buf(), source })?;

        let staged = extract(bytes, staging.path())?;
        check_bundle(&staged, current)?;

        let backup = staging.path().join("previous.app");
        match swap(fs, bundle, &staged, &backup) {
            Ok(()) => Ok(()),
            Err(SwapFailure::Untouched(source)) | Err(SwapFailure::Restored(source)) => {
                Err(not_replaced(bundle, source))
            }
            Err(SwapFailure::Stranded { failed, restore }) => {
                // The staging directory holds the only copy of the app the
                // user had. It stays, and the error says where.
                let staging = staging.keep();
                Err(InstallError::Stranded {
                    path: bundle.to_path_buf(),
                    backup: staging.join("previous.app"),
                    failed,
                    restore,
                })
            }
        }
    }

    /// Unpacks the archive and returns the one app bundle in it.
    fn extract(bytes: &[u8], dir: &Path) -> Result<PathBuf, InstallError> {
        let decoder = flate2::read::GzDecoder::new(bytes);
        let mut tar_archive = tar::Archive::new(decoder);
        for entry in tar_archive.entries().map_err(archive)? {
            let mut entry = entry.map_err(archive)?;
            // `unpack_in` refuses anything that would land outside `dir`; an
            // archive that tries is not the one we signed, so it fails whole.
            let inside = {
                let path = entry.path().map_err(archive)?;
                path.components().all(|component| matches!(component, Component::Normal(_)))
            };
            if !inside || !entry.unpack_in(dir).map_err(archive)? {
                return Err(InstallError::NotABundle);
            }
        }
        one_bundle(dir)
    }

    fn archive(source: io::Error) -> InstallError {
        InstallError::Archive { source }
    }

    /// The app bundle among the entries the archive unpacked.
    fn one_bundle(dir: &Path) -> Result<PathBuf, InstallError> {
        let mut bundles = Vec::new();
        for entry in fs::read_dir(dir).map_err(archive)? {
            let path = entry.map_err(archive)?.path();
            if path.is_dir() && path.extension().is_some_and(|extension| extension == "app") {
                bundles.push(path);
            }
        }
        match bundles.len() {
            1 => Ok(bundles.remove(0)),
            _ => Err(InstallError::NotABundle),
        }
    }

    /// Reads the bundle's `Info.plist`: it has to be Candlewick, and newer
    /// than the copy it would replace.
    fn check_bundle(bundle: &Path, current: &Version) -> Result<(), InstallError> {
        let plist = bundle.join("Contents/Info.plist");
        let unreadable =
            |reason: String| InstallError::UnreadableBundle { path: plist.clone(), reason };
        let value =
            plist::Value::from_file(&plist).map_err(|error| unreadable(error.to_string()))?;
        let info = value
            .into_dictionary()
            .ok_or_else(|| unreadable("it is not a dictionary".to_owned()))?;
        let text = |key: &str| {
            info.get(key)
                .and_then(plist::Value::as_string)
                .map(str::to_owned)
                .ok_or_else(|| unreadable(format!("it has no {key}")))
        };

        let identifier = text("CFBundleIdentifier")?;
        if identifier != IDENTIFIER {
            return Err(InstallError::WrongIdentifier(identifier));
        }
        let version = text("CFBundleShortVersionString")?;
        let parsed = Version::parse(&version).map_err(|error| {
            unreadable(format!("its version {version:?} cannot be read: {error}"))
        })?;
        if parsed <= *current {
            return Err(InstallError::NotNewer { found: parsed, current: current.clone() });
        }
        let executable = bundle.join("Contents/MacOS").join(text("CFBundleExecutable")?);
        if !executable.is_file() {
            return Err(unreadable(format!("{} is missing", executable.display())));
        }
        Ok(())
    }

    /// The renames the swap performs. A test substitutes one that fails.
    trait Fs {
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    }

    struct RealFs;

    impl Fs for RealFs {
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            fs::rename(from, to)
        }
    }

    enum SwapFailure {
        /// The installed bundle could not be moved aside; it is untouched.
        Untouched(io::Error),
        /// The new bundle could not be moved in; the old one is back.
        Restored(io::Error),
        /// Neither worked: the old bundle stays at its backup path.
        Stranded { failed: io::Error, restore: io::Error },
    }

    /// Moves `staged` to `app`, with the bundle that was there kept at
    /// `backup` until the move succeeded.
    fn swap(fs: &dyn Fs, app: &Path, staged: &Path, backup: &Path) -> Result<(), SwapFailure> {
        fs.rename(app, backup).map_err(SwapFailure::Untouched)?;
        if let Err(failed) = fs.rename(staged, app) {
            return Err(match fs.rename(backup, app) {
                Ok(()) => SwapFailure::Restored(failed),
                Err(restore) => SwapFailure::Stranded { failed, restore },
            });
        }
        Ok(())
    }

    fn not_replaced(path: &Path, source: io::Error) -> InstallError {
        if source.kind() == io::ErrorKind::PermissionDenied {
            InstallError::NotWritable { path: path.to_path_buf() }
        } else {
            InstallError::NotReplaced { path: path.to_path_buf(), source }
        }
    }

    /// Why a bundle could not be installed. Every message names the folder or
    /// the value it is about: the log is all a report from a user carries.
    #[derive(Debug, thiserror::Error)]
    pub enum InstallError {
        /// No staging directory could be made: the folder is read-only, or
        /// full.
        #[error("cannot prepare the install in {path}: {source}")]
        Staging { path: PathBuf, source: io::Error },
        #[error("the update's archive cannot be unpacked: {source}")]
        Archive { source: io::Error },
        /// The archive does not hold exactly one `.app` folder.
        #[error("the update's archive does not hold an app")]
        NotABundle,
        #[error("the update's app cannot be read at {path}: {reason}")]
        UnreadableBundle { path: PathBuf, reason: String },
        #[error("the update is {0}, not com.influo.candlewick")]
        WrongIdentifier(String),
        #[error("the update is {found}, not newer than {current}")]
        NotNewer { found: Version, current: Version },
        #[error("cannot replace {path}: {source}")]
        NotReplaced { path: PathBuf, source: io::Error },
        #[error("cannot replace {path}: the folder it is in is not writable")]
        NotWritable { path: PathBuf },
        /// Both the swap and its rollback failed. The previous app is the only
        /// copy there is, so it stays at `backup`.
        #[error(
            "cannot replace {path}: {failed}; putting the previous app back also failed ({restore}), it is kept at {backup}"
        )]
        Stranded { path: PathBuf, backup: PathBuf, failed: io::Error, restore: io::Error },
    }

    #[cfg(test)]
    mod tests {
        use std::{cell::Cell, fs};

        use super::*;

        const CURRENT: &str = "1.0.0";

        /// A directory under the system temp directory, removed on drop.
        struct Scratch(PathBuf);

        impl Scratch {
            fn new(name: &str) -> Self {
                let path = std::env::temp_dir()
                    .join(format!("candlewick-update-{}-{name}", std::process::id()));
                fs::remove_dir_all(&path).ok();
                fs::create_dir_all(&path).unwrap();
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }

            /// The staging directories an install left here.
            fn leftovers(&self) -> Vec<PathBuf> {
                let entries = fs::read_dir(&self.0).unwrap();
                entries
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| {
                        path.file_name().is_some_and(|name| {
                            name.to_string_lossy().starts_with(".candlewick-update-")
                        })
                    })
                    .collect()
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).ok();
            }
        }

        /// An app bundle whose executable's content is its version, so a test
        /// can tell which copy is installed.
        fn bundle_in(dir: &Path, identifier: &str, version: &str) -> PathBuf {
            let bundle = dir.join("Candlewick.app");
            fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
            let info = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>{identifier}</string>
    <key>CFBundleExecutable</key>
    <string>Candlewick</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
</dict>
</plist>
"#
            );
            fs::write(bundle.join("Contents/Info.plist"), info).unwrap();
            fs::write(bundle.join("Contents/MacOS/Candlewick"), version).unwrap();
            bundle
        }

        fn archived(bundle: &Path) -> Vec<u8> {
            let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            tar.append_dir_all(bundle.file_name().unwrap(), bundle).unwrap();
            tar.into_inner().unwrap().finish().unwrap()
        }

        fn installed_version(bundle: &Path) -> String {
            fs::read_to_string(bundle.join("Contents/MacOS/Candlewick")).unwrap()
        }

        fn current() -> Version {
            Version::parse(CURRENT).unwrap()
        }

        /// A release built beside the installed one, as a download would be.
        fn update_in(scratch: &Scratch, identifier: &str, version: &str) -> Vec<u8> {
            let dir = scratch.path().join("update");
            fs::create_dir_all(&dir).unwrap();
            archived(&bundle_in(&dir, identifier, version))
        }

        /// Fails the renames whose call number is listed, passing the rest
        /// through.
        struct RenameFails {
            calls: Cell<usize>,
            fail: &'static [usize],
        }

        impl RenameFails {
            fn new(fail: &'static [usize]) -> Self {
                Self { calls: Cell::new(0), fail }
            }
        }

        impl Fs for RenameFails {
            fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
                let call = self.calls.get() + 1;
                self.calls.set(call);
                if self.fail.contains(&call) {
                    return Err(io::Error::other("rename failed"));
                }
                fs::rename(from, to)
            }
        }

        #[test]
        fn swaps_in_a_newer_bundle() {
            let scratch = Scratch::new("swap");
            let installed = bundle_in(scratch.path(), IDENTIFIER, CURRENT);
            let bytes = update_in(&scratch, IDENTIFIER, "1.1.0");

            install_with(&RealFs, &installed, &bytes, &current()).unwrap();

            assert_eq!(installed_version(&installed), "1.1.0");
            assert!(scratch.leftovers().is_empty());
        }

        #[test]
        fn puts_the_previous_bundle_back_when_the_new_one_cannot_move_in() {
            let scratch = Scratch::new("rollback");
            let installed = bundle_in(scratch.path(), IDENTIFIER, CURRENT);
            let bytes = update_in(&scratch, IDENTIFIER, "1.1.0");

            let error =
                install_with(&RenameFails::new(&[2]), &installed, &bytes, &current()).unwrap_err();

            assert!(matches!(error, InstallError::NotReplaced { .. }), "{error}");
            assert_eq!(installed_version(&installed), CURRENT);
            assert!(scratch.leftovers().is_empty());
        }

        #[test]
        fn keeps_the_previous_bundle_when_it_cannot_be_put_back() {
            let scratch = Scratch::new("stranded");
            let installed = bundle_in(scratch.path(), IDENTIFIER, CURRENT);
            let bytes = update_in(&scratch, IDENTIFIER, "1.1.0");

            let error = install_with(&RenameFails::new(&[2, 3]), &installed, &bytes, &current())
                .unwrap_err();

            let InstallError::Stranded { backup, .. } = &error else {
                panic!("{error}");
            };
            assert!(!installed.exists());
            assert_eq!(installed_version(backup), CURRENT);
            // The staging directory stays: inside it is the only copy.
            assert_eq!(scratch.leftovers().len(), 1);
        }

        #[test]
        fn rejects_a_bundle_from_another_app() {
            let scratch = Scratch::new("identifier");
            let installed = bundle_in(scratch.path(), IDENTIFIER, CURRENT);
            let bytes = update_in(&scratch, "com.example.other", "1.1.0");

            let error = install_with(&RealFs, &installed, &bytes, &current()).unwrap_err();

            let InstallError::WrongIdentifier(found) = &error else {
                panic!("{error}");
            };
            assert_eq!(found, "com.example.other");
            assert_eq!(installed_version(&installed), CURRENT);
            assert!(scratch.leftovers().is_empty());
        }

        #[test]
        fn rejects_a_bundle_that_is_not_newer() {
            let scratch = Scratch::new("version");
            let installed = bundle_in(scratch.path(), IDENTIFIER, CURRENT);
            let bytes = update_in(&scratch, IDENTIFIER, CURRENT);

            let error = install_with(&RealFs, &installed, &bytes, &current()).unwrap_err();

            assert!(matches!(error, InstallError::NotNewer { .. }), "{error}");
            assert_eq!(installed_version(&installed), CURRENT);
            assert!(scratch.leftovers().is_empty());
        }

        #[test]
        fn unpacks_the_bundle_from_the_archive() {
            let scratch = Scratch::new("extract");
            let bundle = bundle_in(scratch.path(), IDENTIFIER, "1.2.0");
            let out = scratch.path().join("out");
            fs::create_dir_all(&out).unwrap();

            let unpacked = extract(&archived(&bundle), &out).unwrap();

            assert_eq!(unpacked, out.join("Candlewick.app"));
            assert_eq!(installed_version(&unpacked), "1.2.0");
        }

        #[test]
        fn says_so_when_the_folder_is_not_writable() {
            let error = not_replaced(
                Path::new("/Applications/Candlewick.app"),
                io::Error::from(io::ErrorKind::PermissionDenied),
            );

            let InstallError::NotWritable { path } = &error else {
                panic!("{error}");
            };
            assert_eq!(path, Path::new("/Applications/Candlewick.app"));
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::{fs, io, path::PathBuf};

    /// Writes the downloaded installer under the temp directory and starts it.
    ///
    /// Passive mode installs for the current user without asking anything,
    /// `/NS` skips recreating shortcuts, and `/R` starts the app again when it
    /// is done. The caller has to exit: the installer cannot replace the files
    /// of a running app.
    pub fn install(bytes: &[u8]) -> Result<(), InstallError> {
        let path =
            std::env::temp_dir().join(format!("candlewick-{}-setup.exe", std::process::id()));
        fs::write(&path, bytes)
            .map_err(|source| InstallError::Staging { path: path.clone(), source })?;
        std::process::Command::new(&path)
            .args(["/P", "/R", "/NS"])
            .spawn()
            .map_err(|source| InstallError::Spawn { path, source })?;
        Ok(())
    }

    #[derive(Debug, thiserror::Error)]
    pub enum InstallError {
        #[error("cannot write the installer to {path}: {source}")]
        Staging { path: PathBuf, source: io::Error },
        #[error("cannot start the installer at {path}: {source}")]
        Spawn { path: PathBuf, source: io::Error },
    }
}
