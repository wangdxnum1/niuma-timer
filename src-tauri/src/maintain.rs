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

/// 表 → 业务分类（表名, 分类 key, 展示名）。
///
/// 一份业务数据通常落在两张表上（日汇总 + 逐小时明细），用户关心的是
/// 「键鼠活动占多少」而不是「act_hourly 占多少」，故按业务合并展示。
/// 新增表时这里与 `DATED_TABLES` 一起改。
const TABLE_GROUPS: &[(&str, &str, &str)] = &[
    ("ot_records", "overtime", "加班记录"),
    ("act_hourly", "activity", "键鼠活动"),
    ("act_keys", "activity", "键鼠活动"),
    ("app_usage", "app", "应用使用"),
    ("app_usage_hourly", "app", "应用使用"),
    ("audio_usage", "audio", "媒体播放"),
    ("audio_usage_hourly", "audio", "媒体播放"),
];

/// 分类展示顺序（key, 展示名）。key 与 `TABLE_GROUPS` 一一对应。
const GROUP_ORDER: &[(&str, &str)] = &[
    ("overtime", "加班记录"),
    ("activity", "键鼠活动"),
    ("app", "应用使用"),
    ("audio", "媒体播放"),
];

/// 表结构之外、但同在数据目录里的文件（配置 / 节假日缓存 / 日志），
/// 单独统计而非混进「其它」，因为它们是用户可以直观理解的东西。
fn other_files(dir: &Path) -> (u64, u64) {
    let mut cfg = 0u64;
    let mut logs = 0u64;
    let Ok(rd) = fs::read_dir(dir) else {
        return (0, 0);
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let size = file_size(&p);
        if name == "config.json" || (name.starts_with("holiday_") && name.ends_with(".json")) {
            cfg += size;
        } else if name == "debug.log" || name == "panic.log" {
            logs += size;
        }
    }
    (cfg, logs)
}

fn group_key(table: &str) -> Option<&'static str> {
    TABLE_GROUPS
        .iter()
        .find(|(t, _, _)| *t == table)
        .map(|(_, k, _)| *k)
}

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

/// 一个存储分类的占用（设置页细分列表的一行）
#[derive(Clone, serde::Serialize)]
pub struct StorageSlice {
    /// 稳定标识，前端据此上色；也是测试断言用的键
    pub key: String,
    /// 展示名
    pub label: String,
    pub bytes: u64,
    /// 行数；文件类分类这里是文件数，其余为 0
    pub rows: u64,
    /// 计数单位（"行" / "个文件"）；为空表示不计
    pub unit: String,
}

/// 单表占用（内部统计用，组装时才按业务合并）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableUsage {
    pub table: String,
    pub bytes: u64,
    pub rows: u64,
}

/// 组装快照所需的各部分字节数。
///
/// 抽成结构是因为 `build_storage_info` 的参数已多到容易传错顺序；
/// 同时它不含任何 DB / 文件系统依赖，可以在单测里直接构造。
pub(crate) struct StorageParts {
    pub db_bytes: u64,
    pub wal_bytes: u64,
    pub shm_bytes: u64,
    pub icon_files: u32,
    pub icon_bytes: u64,
    pub config_bytes: u64,
    pub log_bytes: u64,
    pub tables: Vec<TableUsage>,
    /// 是否为估算值（dbstat 不可用时按行数比例分摊）
    pub approx: bool,
    pub earliest: Option<String>,
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
    /// 全部占用之和（主库 + 写前日志 + 图标 + 配置与日志）
    pub total_bytes: u64,
    /// 细分项，按占用降序
    pub slices: Vec<StorageSlice>,
    /// true 表示表级占用是按行数比例估算的（dbstat 虚拟表不可用）
    pub approx: bool,
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

/// 用 dbstat 虚拟表精确统计每张表的占用（含索引与溢出页）。
///
/// dbstat 需要 SQLite 编译时开 `SQLITE_ENABLE_DBSTAT_VTAB`；rusqlite 的
/// `bundled` feature 默认开了（0.30.1 的 build.rs 里有该 flag）。不可用时返回
/// Err，由调用方退化到按行数估算——宁可给个近似值也不要整块统计失败。
pub fn table_usage_conn(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<TableUsage>> {
    let mut sizes: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    {
        let mut st = conn.prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for r in rows {
            let (name, bytes) = r?;
            if bytes > 0 {
                *sizes.entry(name).or_default() += bytes as u64;
            }
        }
    }
    let mut out = Vec::with_capacity(TABLE_GROUPS.len());
    for (table, _, _) in TABLE_GROUPS {
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
        out.push(TableUsage {
            table: (*table).to_string(),
            bytes: sizes.get(*table).copied().unwrap_or(0),
            rows: n.max(0) as u64,
        });
    }
    Ok(out)
}

/// dbstat 不可用时的兜底：按行数占比把整库有效页分摊到各表。
/// 只用于展示，宁可粗略也不要让「存储占用」整块消失。
pub fn estimate_usage_conn(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<TableUsage>> {
    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let free: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    let used = ((pages - free).max(0) as u64).saturating_mul(page_size.max(1) as u64);

    let mut counts: Vec<u64> = Vec::with_capacity(TABLE_GROUPS.len());
    let mut total_rows = 0u64;
    for (table, _, _) in TABLE_GROUPS {
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
        let n = n.max(0) as u64;
        total_rows += n;
        counts.push(n);
    }
    let out = TABLE_GROUPS
        .iter()
        .zip(counts)
        .map(|((table, _, _), n)| TableUsage {
            table: (*table).to_string(),
            bytes: if total_rows == 0 {
                0
            } else {
                used.saturating_mul(n) / total_rows
            },
            rows: n,
        })
        .collect();
    Ok(out)
}

/// 把各表占用按业务合并，再加上文件类分类，组装成展示用的快照。
///
/// 纯函数：不读文件系统也不碰 DB，便于单测覆盖「分组 / 排序 / 兜底」这些
/// 容易出错的分支（真实库上的效果留给手工验证）。
pub(crate) fn build_storage_info(cfg: &Config, p: StorageParts) -> StorageInfo {
    let mut slices: Vec<StorageSlice> = Vec::new();
    for (key, label) in GROUP_ORDER {
        let bytes = p
            .tables
            .iter()
            .filter(|t| group_key(&t.table) == Some(*key))
            .map(|t| t.bytes)
            .sum();
        let rows = p
            .tables
            .iter()
            .filter(|t| group_key(&t.table) == Some(*key))
            .map(|t| t.rows)
            .sum();
        slices.push(StorageSlice {
            key: (*key).to_string(),
            label: (*label).to_string(),
            bytes,
            rows,
            unit: "行".to_string(),
        });
    }

    // 主库里不属于这 7 张表的部分：索引、空闲页、表结构开销。
    // saturating_sub：dbstat 精确值理论上不会超，但估算路径可能，不能让它下溢成天文数字
    let counted: u64 = slices.iter().map(|s| s.bytes).sum();
    slices.push(StorageSlice {
        key: "other".to_string(),
        label: "索引与空闲页".to_string(),
        bytes: p.db_bytes.saturating_sub(counted),
        rows: 0,
        unit: String::new(),
    });
    slices.push(StorageSlice {
        key: "wal".to_string(),
        label: "写前日志".to_string(),
        bytes: p.wal_bytes.saturating_add(p.shm_bytes),
        rows: 0,
        unit: String::new(),
    });
    slices.push(StorageSlice {
        key: "icons".to_string(),
        label: "图标缓存".to_string(),
        bytes: p.icon_bytes,
        rows: p.icon_files as u64,
        unit: "个文件".to_string(),
    });
    slices.push(StorageSlice {
        key: "config".to_string(),
        label: "配置与节假日".to_string(),
        bytes: p.config_bytes,
        rows: 0,
        unit: String::new(),
    });
    slices.push(StorageSlice {
        key: "logs".to_string(),
        label: "运行日志".to_string(),
        bytes: p.log_bytes,
        rows: 0,
        unit: String::new(),
    });

    let total = p
        .db_bytes
        .saturating_add(p.wal_bytes)
        .saturating_add(p.shm_bytes)
        .saturating_add(p.icon_bytes)
        .saturating_add(p.config_bytes)
        .saturating_add(p.log_bytes);

    // 降序：用户第一眼看的是「谁占得最多」
    slices.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    StorageInfo {
        db_bytes: p.db_bytes,
        wal_bytes: p.wal_bytes,
        icon_files: p.icon_files,
        icon_bytes: p.icon_bytes,
        earliest_date: p.earliest,
        retention_days: cfg.retention_days,
        total_bytes: total,
        slices,
        approx: p.approx,
    }
}

/// 当前存储占用快照
pub fn storage_info(cfg: &Config) -> StorageInfo {
    let dir = config_dir();
    let (icon_files, icon_bytes) = icon_usage(&dir.join("icons"));
    let (config_bytes, log_bytes) = other_files(&dir);

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

    // 精确统计失败时退化到估算，绝不因为统计出错就让整块「存储占用」消失
    // 闭包返回的是 rusqlite::Result，故兜底分支也要回 rusqlite::Error（with_db 负责转 String）
    let (tables, approx) = crate::db::with_db(|g| match table_usage_conn(g) {
        Ok(v) => Ok((v, false)),
        Err(first) => match estimate_usage_conn(g) {
            Ok(v) => {
                crate::db::debug_log(&format!(
                    "[maintain] dbstat 不可用，改用行数估算: {first}"
                ));
                Ok((v, true))
            }
            Err(_) => Err(first),
        },
    })
    .unwrap_or_else(|e| {
        crate::db::debug_log(&format!("[maintain] 存储占用统计失败: {e}"));
        (Vec::new(), true)
    });

    build_storage_info(
        cfg,
        StorageParts {
            db_bytes: file_size(&dir.join("niuma.db")),
            wal_bytes: file_size(&wal_path()),
            shm_bytes: file_size(&dir.join("niuma.db-shm")),
            icon_files,
            icon_bytes,
            config_bytes,
            log_bytes,
            tables,
            approx,
            earliest,
        },
    )
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

    fn slice_of(info: &StorageInfo, key: &str) -> StorageSlice {
        info.slices
            .iter()
            .find(|s| s.key == key)
            .unwrap_or_else(|| panic!("缺少分类 {key}"))
            .clone()
    }

    fn parts_with(tables: Vec<TableUsage>, db_bytes: u64) -> StorageParts {
        StorageParts {
            db_bytes,
            wal_bytes: 1000,
            shm_bytes: 24,
            icon_files: 3,
            icon_bytes: 300,
            config_bytes: 40,
            log_bytes: 60,
            tables,
            approx: false,
            earliest: None,
        }
    }

    fn tu(table: &str, bytes: u64, rows: u64) -> TableUsage {
        TableUsage {
            table: table.to_string(),
            bytes,
            rows,
        }
    }

    /// 回归：同一业务的两张表（日汇总 + 逐小时明细）必须合并成一行，
    /// 字节数与行数都相加——用户不认识 act_hourly / act_keys 这种表名
    #[test]
    fn build_groups_tables_by_business() {
        let cfg = crate::config::Config::default();
        let tables = vec![
            tu("ot_records", 100, 10),
            tu("act_hourly", 200, 24),
            tu("act_keys", 50, 300),
            tu("app_usage", 10, 5),
            tu("app_usage_hourly", 20, 40),
            tu("audio_usage", 5, 2),
            tu("audio_usage_hourly", 5, 8),
        ];
        let info = build_storage_info(&cfg, parts_with(tables, 1000));

        let act = slice_of(&info, "activity");
        assert_eq!(act.bytes, 250, "键鼠活动 = act_hourly + act_keys");
        assert_eq!(act.rows, 324, "行数同样要合并");
        assert_eq!(slice_of(&info, "overtime").bytes, 100);
        assert_eq!(slice_of(&info, "app").bytes, 30);
        assert_eq!(slice_of(&info, "audio").bytes, 10);
    }

    /// 回归：主库里不属于 7 张表的部分（索引、空闲页）归入「索引与空闲页」，
    /// 且用 saturating_sub——估算路径下可能出现表总和 > 主库，不能下溢
    #[test]
    fn build_other_is_db_minus_tables_and_never_negative() {
        let cfg = crate::config::Config::default();
        let tables = vec![tu("ot_records", 300, 1)];
        let info = build_storage_info(&cfg, parts_with(tables, 1000));
        assert_eq!(slice_of(&info, "other").bytes, 700);

        // 表总和 > 主库（估算路径可能如此）：必须为 0，不能绕回巨大值
        let tables = vec![tu("ot_records", 5000, 1)];
        let info = build_storage_info(&cfg, parts_with(tables, 1000));
        assert_eq!(
            slice_of(&info, "other").bytes,
            0,
            "下溢会显示成几百 PB，必须夹到 0"
        );
    }

    /// 回归：总计 = 主库 + 写前日志 + shm + 图标 + 配置 + 日志
    #[test]
    fn build_total_is_sum_of_all_parts() {
        let cfg = crate::config::Config::default();
        let info = build_storage_info(&cfg, parts_with(vec![tu("ot_records", 300, 1)], 1000));
        assert_eq!(info.total_bytes, 1000 + 1000 + 24 + 300 + 40 + 60);
    }

    /// 回归：细分项按占用降序，占用最多的排第一（用户第一眼看的就是它）
    #[test]
    fn build_slices_sorted_by_bytes_desc() {
        let cfg = crate::config::Config::default();
        let info = build_storage_info(&cfg, parts_with(vec![tu("ot_records", 900, 1)], 1000));
        let bytes: Vec<u64> = info.slices.iter().map(|s| s.bytes).collect();
        let mut sorted = bytes.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(bytes, sorted, "细分项必须降序: {bytes:?}");
        assert_eq!(info.slices[0].key, "wal", "写前日志 1024 应排第一");
    }

    /// 回归：分类 key 稳定（前端据此上色），且覆盖全部业务 + 4 个文件类
    #[test]
    fn build_slices_cover_every_category() {
        let cfg = crate::config::Config::default();
        let info = build_storage_info(&cfg, parts_with(Vec::new(), 100));
        let mut keys: Vec<&str> = info.slices.iter().map(|s| s.key.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["activity", "app", "audio", "config", "icons", "logs", "other", "overtime", "wal"]
        );
    }

    /// 回归：StorageInfo 的 JSON 键必须就是字段名本身（snake_case）。
    ///
    /// 前端按 `info.total_bytes` / `info.earliest_date` 取值。若谁给结构体加上
    /// `#[serde(rename_all = "camelCase")]`，键会变成 totalBytes，前端读不到又
    /// 不会报错——只会静默显示「共占用 0 B、每行占比 0%」，正是本次修掉的 bug。
    #[test]
    fn storage_info_json_keys_match_field_names() {
        let cfg = crate::config::Config::default();
        let info = build_storage_info(&cfg, parts_with(vec![tu("ot_records", 300, 1)], 1000));
        let v: serde_json::Value = serde_json::to_value(&info).expect("StorageInfo 应可序列化");

        let mut keys: Vec<&str> = v.as_object().expect("应为 JSON 对象").keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "approx",
                "db_bytes",
                "earliest_date",
                "icon_bytes",
                "icon_files",
                "retention_days",
                "slices",
                "total_bytes",
                "wal_bytes",
            ],
            "字段名变了就必须同步 frontend/app.js"
        );

        // 细分项字段同样不能被改名：前端读 s.key / s.bytes / s.rows / s.unit
        let slice = v["slices"][0].as_object().expect("细分项应为 JSON 对象");
        let mut skeys: Vec<&str> = slice.keys().map(String::as_str).collect();
        skeys.sort();
        assert_eq!(skeys, vec!["bytes", "key", "label", "rows", "unit"]);
    }

    /// 回归：dbstat 精确统计出来的字节数必须 > 0（否则说明虚拟表没编译进去，
    /// 用户看到的每个分类都是 0 而总计却有几 MB，自相矛盾）
    #[test]
    fn table_usage_reads_real_bytes_from_dbstat() {
        let db = mem_db();
        db.execute(
            "INSERT INTO ot_records (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source)
             VALUES ('2026-09-11','22:00','18:00',4,4,80,20,100,0)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO act_hourly (date, hour, moves) VALUES ('2026-09-11', 10, 5)",
            [],
        )
        .unwrap();

        let usage = table_usage_conn(&db).expect("dbstat 应可用（rusqlite bundled）");
        let ot = usage.iter().find(|t| t.table == "ot_records").unwrap();
        assert_eq!(ot.rows, 1);
        assert!(ot.bytes > 0, "dbstat 应给出真实字节数，实测 {}", ot.bytes);

        let act = usage.iter().find(|t| t.table == "act_hourly").unwrap();
        assert_eq!(act.rows, 1);
        assert!(act.bytes > 0);
        // 没插数据的表行数为 0
        let app = usage.iter().find(|t| t.table == "app_usage").unwrap();
        assert_eq!(app.rows, 0);
    }

    /// 回归：dbstat 不可用时的兜底按行数比例分摊，且行数必须准确
    #[test]
    fn estimate_splits_by_row_count_when_dbstat_missing() {
        let db = mem_db();
        for i in 0..3 {
            db.execute(
                "INSERT INTO act_hourly (date, hour, moves) VALUES ('2026-09-11', ?1, 5)",
                rusqlite::params![i],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO ot_records (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total, source)
             VALUES ('2026-09-11','22:00','18:00',4,4,80,20,100,0)",
            [],
        )
        .unwrap();

        let usage = estimate_usage_conn(&db).unwrap();
        let act = usage.iter().find(|t| t.table == "act_hourly").unwrap();
        let ot = usage.iter().find(|t| t.table == "ot_records").unwrap();
        assert_eq!(act.rows, 3);
        assert_eq!(ot.rows, 1);
        assert!(
            act.bytes >= ot.bytes,
            "3 行应不少于 1 行: act={} ot={}",
            act.bytes,
            ot.bytes
        );
        let empty = usage.iter().find(|t| t.table == "app_usage").unwrap();
        assert_eq!(empty.bytes, 0, "0 行的表不该分到字节");
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
