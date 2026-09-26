fn main() {
    // The Windows manifest; ignored when building for macOS.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
