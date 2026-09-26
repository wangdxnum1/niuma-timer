//! 数据备份与一键还原（v1.4.0 包 D）。
//!
//! # 为什么库快照用 `VACUUM INTO` 而不是直接拷文件
//! 库开 WAL，最近的写可能还在 `-wal` 里没并进主库文件；直接 `fs::copy`
//! 拷 `niuma.db` 会静默丢掉最近的记录。`VACUUM INTO` 由 SQLite 自己产出一份
//! 「已合并 WAL、页级一致」的新库文件，是唯一可靠的文件级快照手段。
//!
//! # 为什么还原是两段式（pending → 重启交换）
//! [`crate::db::conn`] 是进程级 `OnceLock<Mutex<Connection>>`，连接永不释放。
//! 持连接时用 `fs::copy` 覆盖 `niuma.db`，Windows 层面可能成功，但已打开的
//! 连接持有旧页缓存，后续读写会把新旧数据混在一起——最难排查的一类损坏。
//! 因此还原只做「铺好 `.pending` 文件」，真正替换发生在下一个进程的最开头
//! （[`apply_pending_on_startup`]，先于一切 `db::conn()` 调用）。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
static BACKUP_OP: Mutex<()> = Mutex::new(());

use serde::{Deserialize, Serialize};

/// zip 内三个固定条目名（不带目录前缀，任何解压工具打开都一眼可读）。
const ENTRY_MANIFEST: &str = "manifest.json";
const ENTRY_DB: &str = "niuma.db";
const ENTRY_CONFIG: &str = "config.json";

/// 还原后旧库 / 旧配置的留存名（单份，覆盖式，给最后一层手工救援通道）。
const DB_BAK: &str = "niuma.db.pre-restore.bak";
const CONFIG_BAK: &str = "config.json.pre-restore.bak";

/// 一份备份在界面上的呈现（列表项），字段全部来自 manifest.json + 文件自身。
#[derive(Debug, Clone, Serialize)]
pub struct BackupEntry {
    /// zip 文件名（还原时作为定位键；不含路径，避免前端拼路径）
    pub name: String,
    /// 完整绝对路径（界面提示用）
    pub path: String,
    /// 文件字节数
    pub bytes: u64,
    /// 备份创建时刻，本地时区 `YYYY-MM-DD HH:MM:SS`
    pub created_at: String,
    /// 写入该备份时的应用版本（可能低于当前版本）
    pub app_version: String,
    /// 若 zip 读不出清单（被截断 / 非本应用产物），置 true，界面标「不可用」
    pub broken: bool,
}

/// 打进 zip 的 manifest.json，同时是还原时的第一道校验依据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// 固定 "niuma-timer"，用于识别「这不是本应用的备份」
    pub app: String,
    pub app_version: String,
    pub created_at: String,
    /// 快照库文件的字节数（还原时与解出文件比对，防条目被换掉）
    pub db_bytes: u64,
    /// config.json 是否存在（用户可能删过配置，允许缺失）
    pub has_config: bool,
}

/// 备份目录：文档\niuma-timer-backup；取不到文档目录时退回 %APPDATA%\niuma-timer\backups。
pub fn backup_dir() -> PathBuf {
    dirs::document_dir().map(|d| d.join("niuma-timer-backup"))
        .unwrap_or_else(|| crate::config::config_dir().join("backups"))
}

/// 组 `VACUUM INTO '<path>'` SQL；路径里的单引号在 SQL 字面量中必须双写转义，
/// 否则 VACUUM 直接语法错（文档目录被 OneDrive 重定向后路径可能含特殊字符）。
pub fn vacuum_into_sql(path: &Path) -> String {
    format!("VACUUM INTO '{}'", path.to_string_lossy().replace('\'', "''"))
}

use std::fs::File;
use std::io::{Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// 读 zip 内的 manifest.json（只读中央目录 + 一个条目，不整包解压）。
/// 任何环节失败都收敛成中文错误；缺 manifest 的包按「不是本应用的备份」拒绝。
pub fn read_manifest(zip_path: &Path) -> Result<Manifest, String> {
    let file = File::open(zip_path).map_err(|e| format!("打开备份失败：{e}"))?;
    let mut arc = ZipArchive::new(file).map_err(|e| format!("备份已损坏，无法打开：{e}"))?;
    let mut entry = arc
        .by_name(ENTRY_MANIFEST)
        .map_err(|_| "不是本应用的备份（缺少 manifest.json）".to_string())?;
    if entry.size() > 65536 { return Err("备份清单过大".to_string()); }
    let mut json = String::new();
    entry
        .read_to_string(&mut json)
        .map_err(|e| format!("读取清单失败：{e}"))?;
    let m: Manifest =
        serde_json::from_str(&json).map_err(|e| format!("清单内容无法解析：{e}"))?;
    if m.app != "niuma-timer" {
        return Err("不是本应用的备份（app 标识不匹配）".to_string());
    }
    Ok(m)
}

/// 生成一份备份。库开不出来 / 写 zip 失败 → Err(中文原因)。
/// 临时快照即使中途出错也必删（RAII 守卫），绝不在 AppData 留等大垃圾文件。
pub fn create_backup(cfg_version: &str) -> Result<BackupEntry, String> {
    let _op = crate::sync::lock(&BACKUP_OP, "backup::operation");
    create_backup_inner(cfg_version)
}

fn create_backup_inner(cfg_version: &str) -> Result<BackupEntry, String> {
    let dir = backup_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建备份目录失败：{e}"))?;

    let now = chrono::Local::now();
    let snap = crate::config::config_dir().join("backup-snapshot.db");
    let _guard = SnapshotGuard(snap.clone()); // 作用域结束必删临时快照
    let _ = std::fs::remove_file(&snap);

    // 必须走 with_db：统一处理锁中毒与错误上报，不裸连。
    // with_db 返回 Result<usize, String>，闭包内已 map_err 成 String，单个 ? 即可
    crate::db::with_db(|conn| {
        conn.execute(&vacuum_into_sql(&snap), [])
    })?;

    let db_bytes = std::fs::metadata(&snap)
        .map_err(|e| format!("读取快照大小失败：{e}"))?
        .len();
    let cfg_path = crate::config::config_dir().join("config.json");
    let has_config = cfg_path.exists();

    let manifest = Manifest {
        app: "niuma-timer".to_string(),
        app_version: cfg_version.to_string(),
        created_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        db_bytes,
        has_config,
    };

    let name = format!(
        "niuma-backup-v{}-{}.zip",
        cfg_version,
        now.format("%Y%m%d-%H%M%S-%f")
    );
    let out = dir.join(&name);
    let partial = out.with_extension("zip.partial");
    let _partial_guard = SnapshotGuard(partial.clone());
    let file = File::options().write(true).create_new(true).open(&partial).map_err(|e| format!("创建备份文件失败：{e}"))?;
    let mut zw = ZipWriter::new(file);
    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zw.start_file(ENTRY_MANIFEST, deflate)
        .map_err(|e| format!("写入清单失败：{e}"))?;
    zw.write_all(serde_json::to_vec_pretty(&manifest).unwrap().as_slice())
        .map_err(|e| format!("写入清单失败：{e}"))?;

    zw.start_file(ENTRY_DB, deflate)
        .map_err(|e| format!("写入数据库条目失败：{e}"))?;
    let snap_bytes = std::fs::read(&snap).map_err(|e| format!("读回快照失败：{e}"))?;
    zw.write_all(&snap_bytes)
        .map_err(|e| format!("写入数据库条目失败：{e}"))?;

    if has_config {
        zw.start_file(ENTRY_CONFIG, deflate)
            .map_err(|e| format!("写入配置条目失败：{e}"))?;
        let cfg_bytes = std::fs::read(&cfg_path).map_err(|e| format!("读配置失败：{e}"))?;
        zw.write_all(&cfg_bytes)
            .map_err(|e| format!("写入配置条目失败：{e}"))?;
    }
    let file = zw.finish().map_err(|e| format!("完成 zip 失败：{e}"))?;
    file.sync_all().map_err(|e| format!("备份写盘失败：{e}"))?;
    drop(file);
    std::fs::rename(&partial, &out).map_err(|e| format!("备份就位失败：{e}"))?;

    Ok(BackupEntry {
        name,
        path: out.to_string_lossy().into_owned(),
        bytes: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
        created_at: manifest.created_at,
        app_version: manifest.app_version,
        broken: false,
    })
}

/// 临时快照守卫：VACUUM INTO 的目标文件即使中途 Err 也必须删掉。
struct SnapshotGuard(PathBuf);
impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 列出备份目录下全部 *.zip：好包按 created_at 倒序（新→旧），坏包垫底。
/// 坏包仍然列出——用户需要看到「这里有个坏文件」并自行删掉。
pub fn list_backups() -> Vec<BackupEntry> {
    let dir = backup_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut good = Vec::new();
    let mut broken = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("zip") {
            continue; // 随便.txt 不出现
        }
        let Some(fname) = path.file_name().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let bytes = ent.metadata().map(|m| m.len()).unwrap_or(0);
        match read_manifest(&path) {
            Ok(m) => good.push(BackupEntry {
                name: fname,
                path: path.to_string_lossy().into_owned(),
                bytes,
                created_at: m.created_at,
                app_version: m.app_version,
                broken: false,
            }),
            Err(_) => broken.push(BackupEntry {
                name: fname,
                path: path.to_string_lossy().into_owned(),
                bytes,
                created_at: String::new(),
                app_version: String::new(),
                broken: true,
            }),
        }
    }
    good.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    broken.sort_by(|a, b| b.name.cmp(&a.name));
    good.extend(broken);
    good
}

use rusqlite::Connection;

/// 7 张业务表：解出的库必须齐全，少一张就不是完整备份。
const REQUIRED_TABLES: &[&str] = &[
    "ot_records",
    "act_hourly",
    "act_keys",
    "app_usage",
    "app_usage_hourly",
    "audio_usage",
    "audio_usage_hourly",
];

/// 还原第一步：校验 + 铺 pending。校验任一环失败即 Err 且零磁盘副作用。
pub fn stage_restore(name: &str, current_version: &str) -> Result<String, String> {
    let _op = crate::sync::lock(&BACKUP_OP, "backup::operation");
    if name.is_empty() || Path::new(name).components().count() != 1
        || name.contains(['/', '\\', ':']) || name.contains("..") || !name.ends_with(".zip") {
        return Err("备份名不合法".to_string());
    }
    let dir = crate::config::config_dir();
    if dir.join("niuma.db.pending").exists() {
        return Err("已有还原等待重启，请先重启程序".to_string());
    }
    let zip_path = backup_dir().join(name);
    let check_db = std::env::temp_dir().join(format!("niuma-restore-check-{}-{}.db",
        std::process::id(), chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()));
    let _guard = TempGuard(check_db.clone());
    let cfg_bytes = validate_archive(&zip_path, &check_db)?;
    let guard_entry = create_backup_inner(current_version)?;
    stage_validated(&check_db, cfg_bytes.as_deref(), &dir)?;
    Ok(format!("还原已就绪；当前数据已自动备份为 {}，重启后生效", guard_entry.name))
}

/// 校验只操作独立临时文件，不读写用户当前数据库。
fn validate_archive(zip_path: &Path, check_db: &Path) -> Result<Option<Vec<u8>>, String> {
    let manifest = read_manifest(zip_path)?;
    let mut arc = ZipArchive::new(File::open(zip_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if arc.len() != if manifest.has_config { 3 } else { 2 } {
        return Err("备份条目数量与清单不符".to_string());
    }
    {
        let mut entry = arc.by_name(ENTRY_DB).map_err(|_| "备份缺少 niuma.db".to_string())?;
        if entry.size() != manifest.db_bytes || entry.size() > 4 * 1024 * 1024 * 1024 {
            return Err("数据库大小与清单不符或超过 4GB".to_string());
        }
        let mut out = File::options().write(true).create_new(true).open(check_db)
            .map_err(|e| format!("创建校验文件失败：{e}"))?;
        let copied = std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        if copied != manifest.db_bytes { return Err("数据库解压大小不符".to_string()); }
    }
    let cfg_bytes = if manifest.has_config {
        let mut entry = arc.by_name(ENTRY_CONFIG).map_err(|_| "备份缺少 config.json".to_string())?;
        if entry.size() > 4 * 1024 * 1024 { return Err("配置文件过大".to_string()); }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        serde_json::from_slice::<crate::config::Config>(&bytes)
            .map_err(|e| format!("备份配置无效：{e}"))?;
        Some(bytes)
    } else {
        if arc.index_for_name(ENTRY_CONFIG).is_some() { return Err("配置与清单不符".to_string()); }
        None
    };
    let conn = Connection::open_with_flags(check_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("备份数据库无法打开：{e}"))?;
    let check: String = conn.pragma_query_value(None, "integrity_check", |row| row.get(0))
        .map_err(|e| format!("完整性检查失败：{e}"))?;
    if check != "ok" { return Err(format!("完整性检查未通过：{check}")); }
    for table in REQUIRED_TABLES {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table], |row| row.get(0)).map_err(|e| e.to_string())?;
        if !exists { return Err(format!("备份缺少业务表：{table}")); }
    }
    Ok(cfg_bytes)
}

/// 数据库 pending 是提交标记，配置与数据库先写完，再发布此标记。
fn stage_validated(check_db: &Path, cfg: Option<&[u8]>, dir: &Path) -> Result<(), String> {
    let db_temp = dir.join("niuma.db.restore-tmp");
    let cfg_pending = dir.join("config.json.pending");
    let _db_guard = TempGuard(db_temp.clone());
    let result = (|| -> Result<(), String> {
        std::fs::copy(check_db, &db_temp).map_err(|e| e.to_string())?;
        File::options().write(true).open(&db_temp).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        match cfg {
            Some(bytes) => {
                std::fs::write(&cfg_pending, bytes).map_err(|e| e.to_string())?;
                File::options().write(true).open(&cfg_pending).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
            }
            None => {
                if cfg_pending.exists() {
                    std::fs::remove_file(&cfg_pending).map_err(|e| e.to_string())?;
                }
            }
        }
        std::fs::rename(&db_temp, dir.join("niuma.db.pending")).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(cfg_pending); }
    result.map_err(|e| format!("准备还原失败：{e}"))
}

/// temp 文件守卫（校验库）。
struct TempGuard(PathBuf);
impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 启动期交换。无 pending → None（零开销）；有 → 交换并返回摘要。
/// 失败尽力回滚并返回 None——还原失败绝不能让程序起不来。
pub fn apply_pending_on_startup() -> Option<String> {
    swap_pending_in(&crate::config::config_dir())
}

/// 交换内核（抽出独立函数以便用临时目录单测，绝不碰真实数据目录）。
pub(crate) fn swap_pending_in(dir: &Path) -> Option<String> {
    let db_pending = dir.join("niuma.db.pending");
    let marker = dir.join("restore-in-progress");
    if !db_pending.exists() && !marker.exists() {
        return None; // 1. 无 pending：零开销返回，目录不做任何改动
    }
    // Durable intent survives consuming the DB pending file before config promotion.
    if !marker.exists() {
        let f = std::fs::File::create(&marker).ok()?;
        f.sync_all().ok()?;
    }

    if db_pending.exists() {
    let db = dir.join("niuma.db");
    let db_bak = dir.join(DB_BAK);
    let had_old_db = db.exists();
    if had_old_db {
        if db_bak.exists() && std::fs::remove_file(&db_bak).is_err() { return None; }
        if std::fs::rename(&db, &db_bak).is_err() { return None; }
    }
    // 保留旧 WAL/SHM；失败时一并恢复，不能先删除未合并的历史数据。
    let mut sidecars = Vec::new();
    for suffix in ["-wal", "-shm"] {
        let from = dir.join(format!("niuma.db{suffix}"));
        let to = dir.join(format!("{DB_BAK}{suffix}"));
        if from.exists() {
            if to.exists() && std::fs::remove_file(&to).is_err()
                || std::fs::rename(&from, &to).is_err() {
                for (f, t) in sidecars.iter().rev() { let _ = std::fs::rename(t, f); }
                if had_old_db { let _ = std::fs::rename(&db_bak, &db); }
                return None;
            }
            sidecars.push((from, to));
        }
    }
    if let Err(e) = std::fs::rename(&db_pending, &db) {
        crate::db::debug_log(&format!("[backup] 新库就位失败，尝试回滚：{e}"));
        for (f, t) in sidecars.iter().rev() { let _ = std::fs::rename(t, f); }
        if had_old_db { let _ = std::fs::rename(&db_bak, &db); }
        return None;
    }
    }

    // 5. 配置非对称处置：库已就位后配置失败**不回退库**（回退会让用户白等一次重启），
    //    仅记录日志，下次启动用默认配置，用户重设即可。
    let cfg_pending = dir.join("config.json.pending");
    let mut cfg_part = "配置未变".to_string();
    if cfg_pending.exists() {
        let cfg = dir.join("config.json");
        let cfg_bak = dir.join(CONFIG_BAK);
        let had_cfg = cfg.exists();
        if had_cfg {
            let _ = std::fs::remove_file(&cfg_bak);
            if std::fs::rename(&cfg, &cfg_bak).is_err() {
                crate::db::debug_log("[backup] 旧配置改名失败，保留旧配置继续启动");
            }
        }
        if let Err(e) = std::fs::rename(&cfg_pending, &cfg) {
            // 旧配置此时已在 .pre-restore.bak、config.json 缺位 → 程序走默认配置；
            // 与上方注释一致：不回退库，用户在界面重设即可
            crate::db::debug_log(&format!(
                "[backup] 新配置就位失败（不回退已还原的库）：{e}"
            ));
            if had_cfg && !cfg.exists() { let _ = std::fs::rename(&cfg_bak, &cfg); }
            cfg_part = "配置还原失败，已尽力保留旧配置".to_string();
        } else {
            cfg_part = format!("配置已还原（旧配置留存 {CONFIG_BAK}）");
        }
    }
    // Leave the marker on retryable config errors; next startup resumes the pair.
    if !cfg_pending.exists() { let _ = std::fs::remove_file(&marker); }

    // 6+7.
    let msg = format!(
        "[backup] 已应用启动期还原：新库 niuma.db 已就位（旧库留存 {DB_BAK}）；{cfg_part}"
    );
    crate::db::debug_log(&msg);
    Some(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 用 STORED（不压缩）在内存里造一个 zip，返回字节流。
    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            let opts = SimpleFileOptions::default();
            for (name, data) in entries {
                zw.start_file(*name, opts).unwrap();
                zw.write_all(data).unwrap();
            }
            zw.finish().unwrap();
        }
        buf.into_inner()
    }

    /// 造包写到临时文件，返回路径；测试结束删除。
    fn write_tmp_zip(tag: &str, bytes: &[u8]) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "niuma-backup-test-{tag}-{}.zip",
            std::process::id()
        ));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn manifest_identifies_foreign_zip() {
        // 一个结构合法、但不含 manifest.json 的 zip（随便哪个别的软件的备份）
        let bytes = make_zip(&[(ENTRY_DB, b"not a real database")]);
        let p = write_tmp_zip("foreign", &bytes);
        let err = read_manifest(&p).unwrap_err();
        assert!(err.contains("不是本应用的备份"), "实际: {err}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn manifest_rejects_truncated_zip() {
        let bytes = make_zip(&[(ENTRY_MANIFEST, br#"{"app":"niuma-timer"}"#)]);
        // 截断一半：中央目录损坏，ZipArchive 必须返回 Err 而不是 panic
        let cut = bytes.len() / 2;
        let p = write_tmp_zip("truncated", &bytes[..cut]);
        assert!(read_manifest(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }
    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "niuma-restore-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_file(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    fn snapshot(dir: &Path) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        for ent in std::fs::read_dir(dir).unwrap().flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            let body = std::fs::read_to_string(ent.path()).ok();
            out.push((name, body));
        }
        out.sort();
        out
    }

    #[test]
    fn apply_pending_noop_without_files() {
        let dir = tmp_dir("noop");
        write_file(&dir, "keep.txt", "x");
        let before = snapshot(&dir);
        assert!(swap_pending_in(&dir).is_none());
        assert_eq!(before, snapshot(&dir), "无 pending 时目录必须完全不变");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_resumes_after_db_or_config_rename_crash() {
        for old_cfg_moved in [false, true] {
            let dir = tmp_dir(if old_cfg_moved { "resume-config" } else { "resume-db" });
            write_file(&dir, "restore-in-progress", "");
            write_file(&dir, "niuma.db", "NEW-DB");
            write_file(&dir, DB_BAK, "OLD-DB");
            write_file(&dir, if old_cfg_moved { CONFIG_BAK } else { "config.json" }, "OLD-CFG");
            write_file(&dir, "config.json.pending", "NEW-CFG");
            assert!(swap_pending_in(&dir).is_some());
            assert_eq!(std::fs::read_to_string(dir.join("config.json")).unwrap(), "NEW-CFG");
            assert_eq!(std::fs::read_to_string(dir.join(DB_BAK)).unwrap(), "OLD-DB");
            assert!(!dir.join("restore-in-progress").exists());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn apply_pending_swaps_db() {
        let dir = tmp_dir("swapdb");
        write_file(&dir, "niuma.db", "OLD-DB");
        write_file(&dir, "niuma.db.pending", "NEW-DB");
        let msg = swap_pending_in(&dir).expect("应完成交换");
        assert!(msg.contains("niuma.db"), "实际: {msg}");
        assert_eq!(std::fs::read_to_string(dir.join("niuma.db")).unwrap(), "NEW-DB");
        assert_eq!(
            std::fs::read_to_string(dir.join(DB_BAK)).unwrap(),
            "OLD-DB",
            "旧库必须留存为 .pre-restore.bak"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_pending_removes_stale_wal() {
        let dir = tmp_dir("wal");
        write_file(&dir, "niuma.db", "OLD");
        write_file(&dir, "niuma.db-wal", "WAL-JUNK");
        write_file(&dir, "niuma.db-shm", "SHM-JUNK");
        write_file(&dir, "niuma.db.pending", "NEW");
        assert!(swap_pending_in(&dir).is_some());
        assert!(!dir.join("niuma.db-wal").exists(), "旧 WAL 必须删除");
        assert!(!dir.join("niuma.db-shm").exists(), "旧 SHM 必须删除");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_pending_swaps_config_too() {
        let dir = tmp_dir("cfgpair");
        write_file(&dir, "niuma.db", "OLD-DB");
        write_file(&dir, "niuma.db.pending", "NEW-DB");
        write_file(&dir, "config.json", "OLD-CFG");
        write_file(&dir, "config.json.pending", "NEW-CFG");
        assert!(swap_pending_in(&dir).is_some());
        assert_eq!(std::fs::read_to_string(dir.join("config.json")).unwrap(), "NEW-CFG");
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_BAK)).unwrap(),
            "OLD-CFG"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_pending_keeps_db_when_config_missing() {
        let dir = tmp_dir("nocfg");
        write_file(&dir, "niuma.db", "OLD-DB");
        write_file(&dir, "niuma.db.pending", "NEW-DB");
        // 无 config.json 也无 config.json.pending：只换库，且不留任何 config 空文件
        assert!(swap_pending_in(&dir).is_some());
        assert_eq!(std::fs::read_to_string(dir.join("niuma.db")).unwrap(), "NEW-DB");
        assert!(!dir.join("config.json.pending").exists());
        assert!(!dir.join(CONFIG_BAK).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_pending_is_idempotent() {
        let dir = tmp_dir("idem");
        write_file(&dir, "niuma.db", "OLD-DB");
        write_file(&dir, "niuma.db.pending", "NEW-DB");
        assert!(swap_pending_in(&dir).is_some());
        // 第二次：pending 已被 rename 吃掉，必须 None 且不破坏结果
        assert!(swap_pending_in(&dir).is_none());
        assert_eq!(std::fs::read_to_string(dir.join("niuma.db")).unwrap(), "NEW-DB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_roundtrip() {
        let m = Manifest {
            app: "niuma-timer".to_string(),
            app_version: "1.4.0".to_string(),
            created_at: "2026-09-25 10:20:30".to_string(),
            db_bytes: 248_123,
            has_config: true,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.app, m.app);
        assert_eq!(back.app_version, m.app_version);
        assert_eq!(back.created_at, m.created_at);
        assert_eq!(back.db_bytes, m.db_bytes);
        assert_eq!(back.has_config, m.has_config);
    }

    #[test]
    fn backup_dir_is_document_subdir() {
        // 不硬编码盘符 / 用户名：只断言末段目录名，兼容 OneDrive 重定向
        let last = backup_dir()
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert_eq!(last, "niuma-timer-backup");
    }

    #[test]
    fn vacuum_into_sql_escapes_quote() {
        let sql = vacuum_into_sql(Path::new(r"C:\Users\O'Neil\docs\snap.db"));
        assert!(sql.contains("VACUUM INTO '"), "实际: {sql}");
        assert!(sql.contains("O''Neil"), "单引号必须双写: {sql}");
        assert!(sql.ends_with("'"));
    }
}
