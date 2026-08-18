fn main() {
    // 显式登记主程序命令，让 tauri-build 为它们生成 allow-*/deny-* ACL 权限，
    // 否则前端 invoke 会被 "not allowed by ACL" 拒绝。
    let attrs = tauri_build::Attributes::default().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "load_config",
            "save_config",
            "refresh_holidays",
            "get_status_cmd",
            "hide_window",
            "show_window",
            "focus_window",
        ]),
    );
    tauri_build::try_build(attrs).expect("tauri-build failed");
}
