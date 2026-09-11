//! 构建脚本。
//!
//! 负责两件事：
//!
//! 1. 让 `frontend/` 的内容变化真正反映到 exe 里（tauri-build 默认不监控该目录）；
//! 2. **自动同步缓存戳**——改完前端直接编译即可，不再需要手工改四处版本号。
//!
//! # 为什么必须有缓存戳
//! WebView2 会缓存 `tauri.localhost` 下的 HTML/CSS/JS。即使 exe 里嵌入的是新资源，
//! 运行中的 WebView 仍可能拿旧的，必须让资源 **URL** 变化才能穿透缓存。因此
//! `index.html` / `styles.css` / `app.js` 三处引用都带 `?v=<指纹>`。
//!
//! # 关键：哈希前必须先归一化
//! 本脚本会把算出的指纹**回写**到源码里。若直接哈希原始内容，就会形成
//! 「回写版本号 → 文件内容变化 → 指纹变化 → 再次回写」的死循环，
//! 结果是每次编译都改版本号、每次都强制重编整个 crate。
//! 故哈希前先剔除缓存戳本身（见 [`normalize`]），使「仅改版本号」不影响指纹。

use std::path::{Path, PathBuf};

const FRONTEND: &str = "../frontend";
const FE_VER_PREFIX: &str = "const FE_VER = \"";
/// 需要同步缓存戳的文件（路径相对 build.rs 的工作目录，即 src-tauri/）
const TARGETS: &[&str] = &["../frontend/index.html", "../frontend/app.js", "tauri.conf.json"];

/// 剔除内容里的缓存戳，保证「仅改版本号」不改变指纹。
///
/// 覆盖两种写法：资源引用的 `?v=<hex>` 与日志标记的 `const FE_VER = "v<hex>"`。
fn normalize(s: &str) -> String {
    let mut s = strip_query_ver(s);
    if let Some(p) = s.find(FE_VER_PREFIX) {
        let start = p + FE_VER_PREFIX.len();
        if let Some(off) = s[start..].find('"') {
            let end = start + off;
            s = format!("{}{}\"{}", &s[..p], FE_VER_PREFIX, &s[end..]);
        }
    }
    s
}

/// 把 `?v=` 后跟随的十六进制数字串抹掉，只留 `?v=`。
fn strip_query_ver(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find("?v=") {
        out.push_str(&rest[..p]);
        out.push_str("?v=");
        let b = rest.as_bytes();
        let mut i = p + 3;
        while i < b.len() && (b[i] as char).is_ascii_hexdigit() {
            i += 1;
        }
        rest = &rest[i..];
    }
    out.push_str(rest);
    out
}

/// 把缓存戳写入内容：`?v=` 后的数字换成 `ver`，FE_VER 的值换成 `v{ver}`。
fn apply_ver(s: &str, ver: &str) -> String {
    let mut out = String::with_capacity(s.len() + 128);
    let mut rest = s;
    loop {
        let q = rest.find("?v=");
        let f = rest.find(FE_VER_PREFIX);
        match (q, f) {
            (Some(q), Some(f)) if f < q => rest = apply_fe(rest, ver, &mut out),
            (Some(_), _) => rest = apply_query(rest, ver, &mut out),
            (None, Some(_)) => rest = apply_fe(rest, ver, &mut out),
            _ => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// 处理一处 `?v=`，返回剩余未处理部分。
fn apply_query<'a>(rest: &'a str, ver: &str, out: &mut String) -> &'a str {
    let p = rest.find("?v=").unwrap();
    out.push_str(&rest[..p]);
    out.push_str("?v=");
    out.push_str(ver);
    let b = rest.as_bytes();
    let mut i = p + 3;
    while i < b.len() && (b[i] as char).is_ascii_hexdigit() {
        i += 1;
    }
    &rest[i..]
}

/// 处理一处 `const FE_VER = "..."`，返回剩余未处理部分。
fn apply_fe<'a>(rest: &'a str, ver: &str, out: &mut String) -> &'a str {
    let p = rest.find(FE_VER_PREFIX).unwrap();
    out.push_str(&rest[..p]);
    out.push_str(FE_VER_PREFIX);
    out.push('v');
    out.push_str(ver);
    out.push('"');
    let start = p + FE_VER_PREFIX.len();
    let off = rest[start..].find('"').unwrap();
    &rest[start + off + 1..]
}

/// 计算 frontend 目录的内容指纹（FNV-1a，对归一化后的文本求哈希）。
fn frontend_fingerprint(dir: &Path) -> u64 {
    fn walk(dir: &Path, h: &mut u64, count: &mut u64) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        // 排序：read_dir 的返回顺序不保证，不排序会导致同一份内容算出不同指纹
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, h, count);
            } else if let Ok(data) = std::fs::read(&p) {
                let norm = normalize(&String::from_utf8_lossy(&data));
                for &b in norm.as_bytes() {
                    *h ^= b as u64;
                    *h = h.wrapping_mul(0x1_0000_01b3);
                }
                *h ^= norm.len() as u64;
                *count += 1;
            }
        }
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut count: u64 = 0;
    walk(dir, &mut h, &mut count);
    h ^ count
}

/// 把缓存戳同步到源文件。只在内容真的变化时写盘（避免无谓的 mtime 变动与 git 脏）。
fn sync_cache_buster(ver: &str) -> usize {
    let mut written = 0;
    for rel in TARGETS {
        let Ok(orig) = std::fs::read_to_string(rel) else {
            println!("cargo:warning=缓存戳同步跳过：读不到 {rel}");
            continue;
        };
        let next = apply_ver(&orig, ver);
        if next == orig {
            continue;
        }
        match std::fs::write(rel, next.as_bytes()) {
            Ok(()) => written += 1,
            Err(e) => println!("cargo:warning=缓存戳同步失败（{rel}）：{e}"),
        }
    }
    written
}

fn main() {
    // 前端资源目录变化时强制重跑 tauri-build（把 HTML/CSS/JS 重新嵌入 exe）。
    // tauri-build 默认不监控 frontend/，不写这条会导致改前端文件后 exe 里仍是旧资源。
    println!("cargo:rerun-if-changed=../frontend");
    println!("cargo:rerun-if-changed=../frontend/hover_card.html");
    println!("cargo:rerun-if-changed=../frontend/index.html");
    println!("cargo:rerun-if-changed=../frontend/app.js");
    println!("cargo:rerun-if-changed=../frontend/styles.css");

    let fp = frontend_fingerprint(Path::new(FRONTEND));
    // 折叠成 32 位：高低位异或，避免只用低位（FNV-1a 的低位散列质量相对差）
    let ver = format!("{:08x}", (fp >> 32) as u32 ^ fp as u32);

    // 必须在 tauri_build 之前回写：它是读 tauri.conf.json 生成 context 的，
    // 晚一步则本次嵌入 exe 的仍是带旧戳的资源，改动要编译两次才生效。
    let n = sync_cache_buster(&ver);
    if n > 0 {
        println!("cargo:warning=已自动同步 {n} 处缓存戳 -> v{ver}（提交时请一并带上这些文件）");
    }

    // 指纹注入：frontend 内容一变 → rustc-env 值变化 → cargo 自动重编 crate → 宏重跑嵌入新资源
    println!("cargo:rustc-env=TAURI_FRONTEND_FP={fp}");

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
