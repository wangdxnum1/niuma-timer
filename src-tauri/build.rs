use std::path::Path;

/// 计算 frontend 目录的内容指纹（FNV-1a）。
/// 原理：cargo 会监听 build.rs 输出的 `cargo:rustc-env` 值，值一旦变化就强制重编本 crate，
/// 从而让 generate_context! 宏重新读取 frontend/ 并把最新 HTML/CSS/JS 嵌入 exe。
/// 没有这行指纹时，只改前端文件只会重跑 build.rs、不会重编 crate → 资源永远是旧的。
fn frontend_fingerprint() -> u64 {
    fn walk(dir: &Path, h: &mut u64, count: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, h, count);
                } else if let Ok(data) = std::fs::read(&p) {
                    for &b in &data {
                        *h ^= b as u64;
                        *h = h.wrapping_mul(0x100000001b3);
                    }
                    *h ^= data.len() as u64;
                    *count += 1;
                }
            }
        }
    }
    let mut h: u64 = 0xcbf29ce484222325;
    let mut count: u64 = 0;
    walk(Path::new("../frontend"), &mut h, &mut count);
    h ^ count
}

fn main() {
    // 前端资源目录变化时强制重跑 tauri-build（把 HTML/CSS/JS 重新嵌入 exe）。
    // tauri-build 默认不监控 frontend/，不写这条会导致改前端文件后 exe 里仍是旧资源。
    println!("cargo:rerun-if-changed=../frontend");
    // 指纹注入：frontend 内容一变 → rustc-env 值变化 → cargo 自动重编 crate → 宏重跑嵌入新资源
    println!("cargo:rustc-env=TAURI_FRONTEND_FP={}", frontend_fingerprint());
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
            "get_activity_summary",
            "get_app_usage_summary",
            "get_audio_usage_summary",
            "write_debug_log",
        ]),
    );
    tauri_build::try_build(attrs).expect("tauri-build failed");
}
