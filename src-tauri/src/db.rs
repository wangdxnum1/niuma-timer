//! SQLite 持久化层：统一管理「加班记录」与「活动统计」的落盘。
//!
//! - 单库单连接（`Mutex<Connection>` 串行化，SQLite 单写者模型足够）；
//! - WAL 模式：崩溃/断电不损坏数据，读写不互斥；
//! - 数据库文件：`%APPDATA%/niuma-timer/niuma.db`；
//! - config.json / holiday_cache.json 保持 JSON（固定体量、一次性读写，迁移无收益）。
//!
//! 表结构：
//! - `ot_records`   加班记录，date 主键（跨月归档自然消失——按年月过滤即可）
//! - `act_hourly`   活动统计小时桶，(date, hour) 主键
//! - `act_keys`     活动统计按键明细，(date, vk) 主键

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::activity::DayState;
use crate::overtime::{MonthlyOvertime, OvertimeRecord};

/// 建表语句（幂等，重复执行无副作用）
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ot_records (
    date        TEXT PRIMARY KEY,
    lock_time   TEXT NOT NULL,
    ot_start    TEXT NOT NULL,
    raw_hours   REAL NOT NULL,
    valid_hours REAL NOT NULL,
    fee         REAL NOT NULL,
    meal        REAL NOT NULL,
    total       REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS act_hourly (
    date        TEXT NOT NULL,
    hour        INTEGER NOT NULL,
    moves       INTEGER NOT NULL DEFAULT 0,
    pixels      INTEGER NOT NULL DEFAULT 0,
    left        INTEGER NOT NULL DEFAULT 0,
    dbl         INTEGER NOT NULL DEFAULT 0,
    right       INTEGER NOT NULL DEFAULT 0,
    wheel       INTEGER NOT NULL DEFAULT 0,
    wheel_ticks INTEGER NOT NULL DEFAULT 0,
    mid         INTEGER NOT NULL DEFAULT 0,
    xbtn        INTEGER NOT NULL DEFAULT 0,
    keys        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, hour)
);

CREATE TABLE IF NOT EXISTS act_keys (
    date  TEXT NOT NULL,
    vk    INTEGER NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, vk)
);

CREATE TABLE IF NOT EXISTS app_usage (
    date    TEXT NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, app)
);

CREATE TABLE IF NOT EXISTS app_usage_hourly (
    date    TEXT NOT NULL,
    hour    INTEGER NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, hour, app)
);

CREATE TABLE IF NOT EXISTS audio_usage (
    date    TEXT NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, app)
);

CREATE TABLE IF NOT EXISTS audio_usage_hourly (
    date    TEXT NOT NULL,
    hour    INTEGER NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, hour, app)
);
"#;

/// 当前数据库 schema 版本。每次 schema 变更递增，旧库启动时会按版本逐步迁移。
const CURRENT_DB_VERSION: i32 = 1;

/// 按 `user_version` 执行数据库迁移（幂等、可重入）。
///
/// - 新库（user_version=0）：建全部表并置版本为 1；
/// - 旧库（user_version<当前）：按版本差逐步执行迁移语句，最后更新版本号；
/// - 已是最新：直接返回，不碰表。
///
/// 未来新增列/改表只需在下方 `if version < N` 分支追加对应 ALTER/CREATE，
/// 旧用户升级时即可平滑迁移，无需 `DROP TABLE` 重建丢数据。
fn migrate_db(db: &Connection) {
    let version: i32 = db
        .query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
        .unwrap_or(0);
    if version >= CURRENT_DB_VERSION {
        return;
    }
    eprintln!("[db] 数据库迁移：当前版本 {version} → 目标 {CURRENT_DB_VERSION}");

    // v0 → v1：建立初始全部数据表（IF NOT EXISTS 保证旧库表已存在时幂等）
    if version < 1 {
        db.execute_batch(SCHEMA)
            .expect("初始化数据库表失败");
    }
    // 未来迁移示例（取消注释并改版本号即可扩展）：
    // if version < 2 {
    //     db.execute_batch("ALTER TABLE ot_records ADD COLUMN xxx REAL")
    //         .expect("迁移 ot_records 失败");
    // }

    db.execute_batch(&format!("PRAGMA user_version = {CURRENT_DB_VERSION}"))
        .expect("设置数据库版本失败");
}

fn db_path() -> PathBuf {
    crate::config::config_dir().join("niuma.db")
}

/// 前端/命令层调试日志：追加写入 `%APPDATA%/niuma-timer/debug.log`。
/// 专供排查用户桌面环境问题（沙箱无法复现 WebView2 现场时以该文件为准）。
/// 超过 256KB 自动截断重开，避免无限增长。
pub fn debug_log(msg: &str) {
    use std::io::Write;
    let path = crate::config::config_dir().join("debug.log");
    if let Ok(md) = fs::metadata(&path) {
        if md.len() > 256 * 1024 {
            let _ = fs::write(&path, "");
        }
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let now = chrono::Local::now().format("%m-%d %H:%M:%S");
        let _ = writeln!(f, "[{}] {}", now, msg);
    }
}

/// 全局连接（懒初始化：首次调用时建目录 + 开库 + 建表 + 开 WAL）
pub fn conn() -> &'static Mutex<Connection> {
    static C: OnceLock<Mutex<Connection>> = OnceLock::new();
    C.get_or_init(|| {
        let dir = crate::config::config_dir();
        let _ = fs::create_dir_all(&dir);
        eprintln!("[diag] conn: dir={:?} db={:?}", dir, db_path());
        let db = Connection::open(db_path()).expect("无法打开 niuma.db");
        eprintln!("[diag] conn: opened");
        let _ = db.busy_timeout(Duration::from_secs(5));
        let _ = db.pragma_update(None, "journal_mode", "WAL");
        eprintln!("[diag] conn: wal set");
        migrate_db(&db);
        eprintln!("[diag] conn: schema ok");
        Mutex::new(db)
    })
}

// ---------------------------------------------------------------------------
// 旧 JSON 数据一次性迁移
// ---------------------------------------------------------------------------

/// 把历史 JSON 数据导入 SQLite（幂等：目标表非空则跳过），导入后旧文件改名 `.legacy.bak` 备份。
/// 必须在任何业务读写之前调用（程序启动 setup 阶段）。
pub fn migrate_legacy() {
    migrate_ot_json();
    migrate_activity_json();
}

/// 迁移加班数据：overtime.json（主文件，BTreeMap<月, MonthlyOvertime>）
/// + overtime-YYYY-MM.json（历史归档文件）
fn migrate_ot_json() {
    let dir = crate::config::config_dir();
    let g = conn().lock().unwrap();

    // 幂等：表里已有数据则不再导入
    let count: i64 = g
        .query_row("SELECT COUNT(*) FROM ot_records", [], |r| r.get(0))
        .unwrap_or(0);
    if count > 0 {
        return;
    }

    let mut imported = 0usize;

    // 主文件 overtime.json
    let main = dir.join("overtime.json");
    if let Ok(s) = fs::read_to_string(&main) {
        if let Ok(map) = serde_json::from_str::<BTreeMap<String, MonthlyOvertime>>(&s) {
            for (_, monthly) in map {
                for rec in monthly.records {
                    if insert_ot(&g, &rec) {
                        imported += 1;
                    }
                }
            }
            // 解析成功即视为旧格式存在，导入后备份
            let _ = fs::rename(&main, dir.join("overtime.json.legacy.bak"));
        }
    }

    // 历史归档文件 overtime-YYYY-MM.json
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name.starts_with("overtime-") && name.ends_with(".json") {
                if let Ok(s) = fs::read_to_string(&p) {
                    if let Ok(monthly) = serde_json::from_str::<MonthlyOvertime>(&s) {
                        for rec in monthly.records {
                            if insert_ot(&g, &rec) {
                                imported += 1;
                            }
                        }
                        let _ = fs::rename(&p, dir.join(format!("{name}.legacy.bak")));
                    }
                }
            }
        }
    }

    if imported > 0 {
        eprintln!("[db] 已从 JSON 迁移 {imported} 条加班记录到 SQLite");
    }
}

/// 迁移活动统计：activity-YYYY-MM-DD.json（每天一个文件，含 24 桶 + 按键明细）
fn migrate_activity_json() {
    let dir = crate::config::config_dir();
    let g = conn().lock().unwrap();

    // 幂等：act_hourly 表非空则不再导入
    let count: i64 = g
        .query_row("SELECT COUNT(*) FROM act_hourly", [], |r| r.get(0))
        .unwrap_or(0);
    if count > 0 {
        return;
    }

    let mut imported = 0usize;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name.starts_with("activity-") && name.ends_with(".json") {
                if let Ok(s) = fs::read_to_string(&p) {
                    if let Ok(d) = serde_json::from_str::<DayState>(&s) {
                        if d.hourly.len() == 24 {
                            insert_day_activity(&g, &d);
                            imported += 1;
                            let _ = fs::rename(&p, dir.join(format!("{name}.legacy.bak")));
                        }
                    }
                }
            }
        }
    }

    if imported > 0 {
        eprintln!("[db] 已从 JSON 迁移 {imported} 天活动统计到 SQLite");
    }
}

/// 把库里已存的旧英文显示名（如 "NetEase Cloud Music"）统一迁移成中文常用名。
///
/// 背景：早期版本展示名直接取 exe 版本资源的英文 FileDescription，映射表（按 exe
/// 文件名匹配）无法反查这些旧名。本函数遍历四张统计表里所有 DISTINCT app 名，
/// 命中 LEGACY 别名表则聚合迁移（同日/同时段同名冲突时秒数累加），幂等安全。
/// 必须在 app_usage::start() / audio_usage::start() 写入新数据之前调用。
pub fn normalize_app_names() {
    let g = conn().lock().unwrap();
    // (天级表, 小时表)：结构分别为 (date,app,seconds) / (date,hour,app,seconds)
    let pairs = [
        ("app_usage", "app_usage_hourly"),
        ("audio_usage", "audio_usage_hourly"),
    ];
    for (daily, hourly) in pairs {
        let names: Vec<String> = {
            let sql = format!("SELECT DISTINCT app FROM {daily}");
            let mut stmt = match g.prepare(&sql) {
                Ok(s) => s,
                Err(_) => continue,
            };
            stmt.query_map([], |r| r.get::<_, String>(0))
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        };
        for old in names {
            let Some(new) = crate::app_usage::map_legacy_display(&old) else {
                continue;
            };
            if new == old {
                continue;
            }
            // 天级表：按 date 聚合旧名秒数并入新名
            let _ = g.execute(
                &format!(
                    "INSERT INTO {daily} (date, app, seconds) \
                     SELECT date, ?1, SUM(seconds) FROM {daily} WHERE app = ?2 GROUP BY date \
                     ON CONFLICT(date, app) DO UPDATE SET seconds = seconds + excluded.seconds",
                ),
                params![new, old],
            );
            let _ = g.execute(
                &format!("DELETE FROM {daily} WHERE app = ?1"),
                params![old],
            );
            // 小时表：按 (date, hour) 聚合
            let _ = g.execute(
                &format!(
                    "INSERT INTO {hourly} (date, hour, app, seconds) \
                     SELECT date, hour, ?1, SUM(seconds) FROM {hourly} WHERE app = ?2 GROUP BY date, hour \
                     ON CONFLICT(date, hour, app) DO UPDATE SET seconds = seconds + excluded.seconds",
                ),
                params![new, old],
            );
            let _ = g.execute(
                &format!("DELETE FROM {hourly} WHERE app = ?1"),
                params![old],
            );
            eprintln!("[db] 应用名归一化: {old} → {new}");
        }
    }
}

/// 插入单条加班记录（幂等，同日覆盖）
fn insert_ot(g: &Connection, rec: &OvertimeRecord) -> bool {
    g.execute(
        "INSERT OR REPLACE INTO ot_records \
         (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            rec.date,
            rec.lock_time,
            rec.ot_start,
            rec.raw_hours,
            rec.valid_hours,
            rec.fee,
            rec.meal,
            rec.total
        ],
    )
    .is_ok()
}

/// 插入某天的完整活动统计（24 小时桶 + 按键明细）
fn insert_day_activity(g: &Connection, d: &DayState) {
    for (i, b) in d.hourly.iter().enumerate() {
        let _ = g.execute(
            "INSERT OR REPLACE INTO act_hourly \
             (date, hour, moves, pixels, left, dbl, right, wheel, wheel_ticks, mid, xbtn, keys) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                d.date,
                i as i64,
                b.moves as i64,
                b.pixels as i64,
                b.left as i64,
                b.dbl as i64,
                b.right as i64,
                b.wheel as i64,
                b.wheel_ticks as i64,
                b.mid as i64,
                b.xbtn as i64,
                b.keys as i64
            ],
        );
    }
    for (vk, cnt) in &d.key_detail {
        let _ = g.execute(
            "INSERT OR REPLACE INTO act_keys (date, vk, count) VALUES (?1, ?2, ?3)",
            params![d.date, *vk as i64, *cnt as i64],
        );
    }
}
