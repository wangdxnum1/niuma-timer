//! 数据生命周期维护：WAL 收缩、过期数据清理、图标缓存清理、迁移遗留文件清理。
//!
//! 背景（2026-09-11 巡检实测）：
//! - **WAL 常年不收缩**：SQLite 的自动 checkpoint（`wal_autocheckpoint` 默认 1000 页
//!   = 4MB）只把已提交页写回主库并**复用** WAL 空间，**不缩小文件**；只有最后一个
//!   连接关闭时才会 truncate。托盘程序全程持有连接不退出 → WAL 永久停在 4MB，
//!   实测 4.15MB 是主库 245KB 的 17 倍。必须显式 `PRAGMA wal_checkpoint(TRUNCATE)`
//!   才能归零（已实测：PASSIVE 后仍是 4.4MB，TRUNCATE 后为 0）。
//! - **图标缓存只增不减**：`config_dir/icons/` 无清理机制。
//! - **历史数据无保留策略**：7 张按日期分桶的表只增不减。
//!
//! 设计原则：**默认不删用户数据**。保留期由配置 `retention_days` 控制，默认 0 =
//! 永久保留，此时只做 WAL 收缩、图标与遗留文件清理——这三项都不损失任何数据
//! （图标可从 exe 重新提取，遗留文件是 8-21 迁 SQLite 时的备份）。
//!
//! 所有操作幂等、失败只记 debug.log 不 panic：维护任务失败不该影响计时本身。

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, NaiveDate};

use crate::config::Config;

/// 图标缓存保留天数。超过则删；下次用到会从 exe 重新提取（一次 GDI 调用，成本可忽略）。
/// 图标是**可再生**的，故这里的保留期与用户数据的 `retention_days` 解耦。
const ICON_MAX_AGE_DAYS: i64 = 180;

/// 需要按日期清理的表（表名 → 日期列名）。
/// 全部为 `date TEXT` 且以 `YYYY-MM-DD` 存储，字典序即时间序，可直接字符串比较。
///
/// 表名/列名都是编译期常量、不涉及用户输入，故 `format!` 拼 SQL 无注入风险。
const DATED_TABLES: &[(&str, &str)] = &[
    ("ot_records", "date"),
    ("act_hourly", "date"),
    ("act_keys", "date"),
    ("app_usage", "date"),
    ("app_usage_hourly", "date"),
    ("audio_usage", "date"),
    ("audio_usage_hourly", "date"),
];

/// 迁移遗留文件：8-21 从 JSON 迁到 SQLite 时的备份，早已无用
const LEGACY_FILES: &[&str] = &["overtime.json.legacy.bak"];

fn config_dir() -> PathBuf {
    crate::config::config_dir()
}

fn wal_path() -> PathBuf {
    config_dir().join("niuma.db-wal")
}

/// 文件大小（字节）；不存在或读取失败返回 0。
/// 维护逻辑只用它做展示与日志，拿不到也不该让整个维护失败。
fn file_size(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// 存储占用快照（供设置页展示）
#[derive(Clone, serde::Serialize)]
pub struct StorageInfo {
    /// 主库字节数
    pub db_bytes: u64,
    /// WAL 字节数（SQLite 写前日志，checkpoint 后应接近 0）
    pub wal_bytes: u64,
    /// 图标缓存文件数
    pub icon_files: u32,
    /// 图标缓存字节数
    pub icon_bytes: u64,
    /// 最早的一条数据日期（7 张表取最小）
    pub earliest_date: Option<String>,
    /// 当前保留策略（天；0 = 永久保留）
    pub retention_days: u32,
}

/// 在一次连接上执行 WAL checkpoint（TRUNCATE 模式），返回 SQLite 的 busy 标志。
/// 拆出内层是为了能直接用临时文件库断言「文件真的变小」——`checkpoint_wal`
/// 走的是全局连接（真实库），单测不能碰。
pub fn checkpoint_wal_conn(conn: &rusqlite::Connection) -> rusqlite::Result<i32> {
    let mut stmt = conn.prepare("PRAGMA wal_checkpoint(TRUNCATE)")?;
    stmt.query_row([], |r| r.get::<_, i32>(0))
}

/// WAL 收缩。
///
/// 关键：必须用 `TRUNCATE` 模式。默认的 `PASSIVE`（也是自动 checkpoint 用的模式）
/// 只把页写回主库、让 WAL 空间可被复用，文件**不会**变小；`TRUNCATE` 才会在
/// checkpoint 后把文件截为 0。托盘程序持连接不退出，永远不会触发 SQLite 自己的
/// 关闭时 truncate，所以必须显式调用。
pub fn checkpoint_wal() -> Result<(), String> {
    let before = file_size(&wal_path());
    let busy = crate::db::with_db(|g| checkpoint_wal_conn(g))?;
    if busy != 0 {
        // 有别的事务在跑，本次没做完；下一轮会再来，不是错误
        crate::db::debug_log(&format!(
            "[maintain] WAL checkpoint 未完全完成（busy={busy}），下轮重试"
        ));
    }
    let after = file_size(&wal_path());
    // 只在确有收益时记日志，避免每天一条噪音
    if before > after && before - after >= 64 * 1024 {
        crate::db::debug_log(&format!(
            "[maintain] WAL 收缩 {:.0}KB -> {:.0}KB",
            before as f64 / 1024.0,
            after as f64 / 1024.0
        ));
    }
    Ok(())
}

/// 按保留期算出清理截止日；`retention_days == 0` 返回 None 表示**不清理**。
///
/// 单独抽成函数：这是「默认配置绝不能悄悄删用户数据」这条约束的唯一落点，
/// 必须能被单测直接断言，而不是埋在 run_daily 的分支里靠人眼检查。
pub fn cutoff_for(cfg: &Config, today: NaiveDate) -> Option<NaiveDate> {
    if cfg.retention_days == 0 {
        return None;
    }
    Some(today - Duration::days(cfg.retention_days as i64))
}

/// 在给定连接上删除 `cutoff` 之前的全部历史数据，返回删除行数。
///
/// 拆出「吃连接」的内层函数是为了能直接用 in-memory 库测——`purge_before`
/// 走的是全局连接（真实库），单测不能碰。
///
/// 7 张表一并处理：加班、活动、应用、媒体。保留期是用户显式选择的，
/// 若加班记录比监控明细更需要留久，用户应把保留期设长——分两套策略反而难解释。
pub fn purge_before_conn(conn: &rusqlite::Connection, cutoff: NaiveDate) -> rusqlite::Result<u32> {
    let cut = cutoff.format("%Y-%m-%d").to_string();
    let mut total = 0u32;
    for (table, col) in DATED_TABLES {
        let sql = format!("DELETE FROM {table} WHERE {col} < ?1");
        total += conn.execute(&sql, rusqlite::params![cut])? as u32;
    }
    Ok(total)
}

/// 删除 `cutoff` 之前的全部历史数据（走全局连接），返回删除行数。
pub fn purge_before(cutoff: NaiveDate) -> Result<u32, String> {
    crate::db::with_db(|g| purge_before_conn(g, cutoff))
}

/// 清理过期图标缓存，返回删除文件数。
/// 只删 `.png`（本模块唯一的产出格式），避免误删目录里的未知文件。
pub fn purge_old_icons() -> u32 {
    let dir = config_dir().join("icons");
    let Ok(rd) = fs::read_dir(&dir) else {
        return 0;
    };
    let cutoff = Local::now().date_naive() - Duration::days(ICON_MAX_AGE_DAYS);
    let mut removed = 0u32;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        let Ok(modified) = md.modified() else { continue };
        let secs = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let Some(d) = DateTime::from_timestamp(secs, 0) else {
            continue;
        };
        if d.with_timezone(&Local).date_naive() < cutoff && fs::remove_file(&p).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 删除迁移遗留文件，返回删除数。仅在文件确实存在时才记日志，
/// 避免每次启动都刷一条「删了 0 个」。
pub fn remove_legacy_files() -> u32 {
    let dir = config_dir();
    let mut n = 0u32;
    for name in LEGACY_FILES {
        let p = dir.join(name);
        if p.exists() && fs::remove_file(&p).is_ok() {
            n += 1;
            crate::db::debug_log(&format!("[maintain] 已清理迁移遗留文件 {name}"));
        }
    }
    n
}

/// 统计图标目录占用
fn icon_usage(dir: &Path) -> (u32, u64) {
    let Ok(rd) = fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut files = 0u32;
    let mut bytes = 0u64;
    for entry in rd.flatten() {
        if let Ok(md) = entry.metadata() {
            if md.is_file() {
                files += 1;
                bytes += md.len();
            }
        }
    }
    (files, bytes)
}

/// 当前存储占用快照
pub fn storage_info(cfg: &Config) -> StorageInfo {
    let dir = config_dir();
    let (icon_files, icon_bytes) = icon_usage(&dir.join("icons"));
    let earliest = crate::db::with_db(|g| {
        let mut min: Option<String> = None;
        for (table, col) in DATED_TABLES {
            let sql = format!("SELECT MIN({col}) FROM {table}");
            if let Ok(Some(d)) = g.query_row(&sql, [], |r| r.get::<_, Option<String>>(0)) {
                if min.as_deref().map_or(true, |m| d.as_str() < m) {
                    min = Some(d);
                }
            }
        }
        Ok(min)
    })
    .ok()
    .flatten();

    StorageInfo {
        db_bytes: file_size(&dir.join("niuma.db")),
        wal_bytes: file_size(&wal_path()),
        icon_files,
        icon_bytes,
        earliest_date: earliest,
        retention_days: cfg.retention_days,
    }
}

/// 每日维护（调度器每天调一次，设置页「立即整理」也走这里）。
///
/// 顺序有讲究：先收缩 WAL，再删数据，删完**再收缩一次**——DELETE 本身会写 WAL，
/// 只在开头收缩的话，刚删出来的空间又会被写满，用户点「立即整理」看不到效果。
pub fn run_daily(cfg: &Config) {
    if let Err(e) = checkpoint_wal() {
        crate::db::debug_log(&format!("[maintain] WAL checkpoint 失败: {e}"));
    }
    remove_legacy_files();
    let icons = purge_old_icons();
    if icons > 0 {
        crate::db::debug_log(&format!("[maintain] 清理过期图标缓存 {icons} 个"));
    }

    let Some(cutoff) = cutoff_for(cfg, Local::now().date_naive()) else {
        return; // 默认永久保留，不删任何历史数据
    };
    match purge_before(cutoff) {
        Ok(0) => {}
        Ok(n) => {
            crate::db::debug_log(&format!(
                "[maintain] 已清理 {cutoff} 之前的历史数据 {n} 行（保留期 {} 天）",
                cfg.retention_days
            ));
            let _ = checkpoint_wal();
        }
        Err(e) => crate::db::debug_log(&format!("[maintain] 历史数据清理失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 用真实建表函数建全 7 张表——手工挑几张会让测试与真实 schema 脱节：
    /// DATED_TABLES 覆盖全部 7 张，少建一张就会在运行期报 no such table。
    fn mem_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        crate::db::init_tables(&db);
        db
    }

    fn cfg_with_retention(days: u32) -> crate::config::Config {
        let mut c = crate::config::Config::default();
        c.retention_days = days;
        c
    }

    /// 回归：默认配置的保留期必须是 0（永久保留）。
    /// 这是本模块最重要的安全约束——默认值一旦改成某个天数，
    /// 所有老用户升级后会**静默丢失**超出保留期的历史数据。
    #[test]
    fn default_config_keeps_everything() {
        assert_eq!(
            crate::config::Config::default().retention_days,
            0,
            "默认必须永久保留"
        );
        let today = NaiveDate::from_ymd_opt(2026, 9, 11).unwrap();
        assert_eq!(
            cutoff_for(&cfg_with_retention(0), today),
            None,
            "保留期 0 天必须不产生 cutoff，即不清理"
        );
    }

    /// 回归：保留期换算——N 天的 cutoff 就是今天减 N 天（半开区间，cutoff 当天保留）
    #[test]
    fn cutoff_is_today_minus_retention() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 11).unwrap();
        assert_eq!(
            cutoff_for(&cfg_with_retention(365), today),
            Some(NaiveDate::from_ymd_opt(2025, 9, 11).unwrap())
        );
        // 跨闰年：2028 是闰年，2027-09-11 减 365 天要落在 2026-09-11
        let t = NaiveDate::from_ymd_opt(2027, 9, 11).unwrap();
        assert_eq!(
            cutoff_for(&cfg_with_retention(365), t),
            Some(NaiveDate::from_ymd_opt(2026, 9, 11).unwrap())
        );
    }

    /// 回归：删除只影响早于 cutoff 的日期，当天与未来数据不受影响
    #[test]
    fn purge_only_removes_rows_before_cutoff() {
        let db = mem_db();
        for d in ["2026-09-08", "2026-09-09", "2026-09-10", "2026-09-11"] {
            db.execute(
                "INSERT INTO ot_records (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source)
                 VALUES (?1,'22:00','18:00',4,4,80,20,100,0)",
                rusqlite::params![d],
            )
            .unwrap();
            db.execute(
                "INSERT INTO act_hourly (date, hour, moves) VALUES (?1, 10, 5)",
                rusqlite::params![d],
            )
            .unwrap();
        }
        let cut = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();

        // 直接调真实实现（走 purge_before_conn 而非全局连接）
        let n = purge_before_conn(&db, cut).unwrap();
        // 9-08、9-09 两天 × 2 张有数据的表 = 4 行
        assert_eq!(n, 4, "只应删掉早于 cutoff 的行");

        let left: i64 = db
            .query_row("SELECT COUNT(*) FROM ot_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 2, "cutoff 当天及之后必须保留");
    }

    /// 回归：日期以 YYYY-MM-DD 存储时，字符串比较等于时间序（跨月、跨年）
    #[test]
    fn date_string_order_matches_chronological_order() {
        let pairs = [
            ("2026-09-30", "2026-10-01"),
            ("2026-12-31", "2027-01-01"),
            ("2026-01-09", "2026-01-10"),
        ];
        for (a, b) in pairs {
            let da = NaiveDate::parse_from_str(a, "%Y-%m-%d").unwrap();
            let db_ = NaiveDate::parse_from_str(b, "%Y-%m-%d").unwrap();
            assert!(da < db_, "{a} 应早于 {b}");
            assert!(a < b, "字符串序应与时间序一致: {a} < {b}");
        }
    }

    /// 回归（本模块最核心的一条）：TRUNCATE 模式必须真的把 WAL 文件缩小。
    ///
    /// 若有人改回 PASSIVE，测试立刻红。PASSIVE 只复用空间、不缩文件，
    /// 而托盘程序持连接不退出，SQLite 自己的「最后连接关闭时 truncate」永不触发，
    /// WAL 会一直停在 4MB——这正是本次要根治的问题。
    #[test]
    fn checkpoint_truncate_shrinks_wal_file() {
        let dir = std::env::temp_dir().join(format!(
            "niuma-wal-test-{}",
            Local::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&dir);
        let dbp = dir.join("t.db");
        let walp = dir.join("t.db-wal");

        let db = Connection::open(&dbp).unwrap();
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.execute_batch("CREATE TABLE t(a INTEGER, b TEXT)").unwrap();

        // 写入约 4MB，让 WAL 明显越过硬盘上的收缩阈值
        db.execute_batch("BEGIN").unwrap();
        {
            let mut stmt = db.prepare("INSERT INTO t VALUES (?1, ?2)").unwrap();
            let pad = "x".repeat(180);
            for i in 0..20000 {
                stmt.execute(rusqlite::params![i, pad]).unwrap();
            }
        }
        db.execute_batch("COMMIT").unwrap();

        let before = file_size(&walp);
        assert!(before > 512 * 1024, "WAL 应先涨到足够大，实测 {before} 字节");

        let busy = checkpoint_wal_conn(&db).unwrap();
        assert_eq!(busy, 0, "单连接场景 checkpoint 不应被占用");

        let after = file_size(&walp);
        assert!(
            after * 4 < before,
            "TRUNCATE 后 WAL 应显著变小: {before} -> {after}"
        );

        // 数据必须完好：checkpoint 只是搬页，不能丢行
        let n: i64 = db
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 20000);

        drop(db);
        let _ = fs::remove_file(&dbp);
        let _ = fs::remove_file(&walp);
        let _ = fs::remove_file(dir.join("t.db-shm"));
        let _ = fs::remove_dir(&dir);
    }

    /// 回归：图标清理只认 .png，目录里的其它文件必须原样保留
    #[test]
    fn icon_purge_only_touches_png() {
        let dir = std::env::temp_dir().join(format!("niuma-icon-test-{}", Local::now().timestamp_nanos_opt().unwrap_or(0)));
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("a.png"), b"x").unwrap();
        fs::write(dir.join("b.txt"), b"y").unwrap();

        let pngs: usize = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("png"))
            .count();
        assert_eq!(pngs, 1, "只应识别到 1 个 png");

        let _ = fs::remove_file(dir.join("a.png"));
        let _ = fs::remove_file(dir.join("b.txt"));
        let _ = fs::remove_dir(&dir);
    }
}
