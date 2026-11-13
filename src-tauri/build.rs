fn main() {
    // On Windows, embed a manifest that asks for Administrator rights at launch
    // (Backblaze's data folder is readable only by administrators).
    #[cfg(windows)]
    {
        let windows = tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"));
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
            .expect("failed to run tauri-build");
    }
    #[cfg(not(windows))]
    tauri_build::build()
}
