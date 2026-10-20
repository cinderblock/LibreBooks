fn main() {
    // Declaring the app's commands generates an `allow-<command>` permission for
    // each, so the capabilities in `capabilities/` decide which window (and
    // which origin) may call which command.
    // Static VC runtime: the installer needs no separate redistributable.
    let windows = tauri_build::WindowsAttributes::new().static_vc_runtime(true);
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(windows)
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "get_shell",
                "save_settings",
                "pick_folder",
                "open_data_folder",
                "test_server",
                "enroll_device",
                "forget_device",
                "check_for_updates",
                "install_update",
                "open_settings",
            ])),
    )
    .expect("failed to run tauri-build");
}
