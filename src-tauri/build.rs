fn main() {
    // 前端资源目录变化时强制重跑 tauri-build（把 HTML/CSS/JS 重新嵌入 exe）。
    // tauri-build 默认不监控 frontend/，不写这条会导致改前端文件后 exe 里仍是旧资源。
    println!("cargo:rerun-if-changed=../frontend");
    println!("cargo:rerun-if-changed=../frontend/hover_card.html");
    println!("cargo:rerun-if-changed=../frontend/index.html");
    println!("cargo:rerun-if-changed=../frontend/app.js");
    println!("cargo:rerun-if-changed=../frontend/styles.css");
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
            "get_overtime_records",
            "save_overtime_record",
            "delete_overtime_record",
        ]),
    );
    tauri_build::try_build(attrs).expect("tauri-build failed");
}
