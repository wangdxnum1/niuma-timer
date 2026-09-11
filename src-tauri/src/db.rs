//! SQLite 持久化层：统一管理「加班记录」与「活动统计」的落盘。
//!
//! - 单库单连接（`Mutex<Connection>` 串行化，SQLite 单写者模型足够）；
//!   访问一律走 [`with_db`]，不要自己 `conn().lock().unwrap()`；
//! - WAL 模式：崩溃/断电不损坏数据，读写不互斥；
//! - 数据库文件：`%APPDATA%/niuma-timer/niuma.db`；
//! - config.json / holiday_cache.json 保持 JSON（固定体量、一次性读写，迁移无收益）。
//!
//! 表结构：
//! - `ot_records`   加班记录，date 主键（跨月归档自然消失——按年月过滤即可），
//!                  另有 `source` 列区分自动(0)/手动(1)，手动记录不会被自动 upsert 覆盖
//! - `act_hourly`   活动统计小时桶，(date, hour) 主键
//! - `act_keys`     活动统计按键明细，(date, vk) 主键

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{params, Connection};

/// `ot_records` 建表语句。单独拆成常量：overtime 模块的单测要在 in-memory 库上
/// 按它建表，以免测试碰到真实库 `%APPDATA%/niuma-timer/niuma.db`。
///
/// `source`：记录来源，0 = 自动（锁屏生成）、1 = 手动录入。自动 upsert 只覆盖
/// 自动记录，手改过的记录不会被当晚的锁屏数据顶掉（详见 overtime::upsert_auto）。
pub(crate) const CREATE_OT_RECORDS: &str = r#"
CREATE TABLE IF NOT EXISTS ot_records (
    date        TEXT PRIMARY KEY,
    lock_time   TEXT NOT NULL,
    ot_start    TEXT NOT NULL,
    raw_hours   REAL NOT NULL,
    valid_hours REAL NOT NULL,
    fee         REAL NOT NULL,
    meal        REAL NOT NULL,
    total       REAL NOT NULL,
    source      INTEGER NOT NULL DEFAULT 0,
    cross_midnight INTEGER NOT NULL DEFAULT 0
);
"#;

pub(crate) const CREATE_ACT_HOURLY: &str = r#"
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
"#;

pub(crate) const CREATE_ACT_KEYS: &str = r#"
CREATE TABLE IF NOT EXISTS act_keys (
    date  TEXT NOT NULL,
    vk    INTEGER NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, vk)
);
"#;

const CREATE_APP_USAGE: &str = r#"
CREATE TABLE IF NOT EXISTS app_usage (
    date    TEXT NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, app)
);
"#;

const CREATE_APP_USAGE_HOURLY: &str = r#"
CREATE TABLE IF NOT EXISTS app_usage_hourly (
    date    TEXT NOT NULL,
    hour    INTEGER NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, hour, app)
);
"#;

const CREATE_AUDIO_USAGE: &str = r#"
CREATE TABLE IF NOT EXISTS audio_usage (
    date    TEXT NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, app)
);
"#;

const CREATE_AUDIO_USAGE_HOURLY: &str = r#"
CREATE TABLE IF NOT EXISTS audio_usage_hourly (
    date    TEXT NOT NULL,
    hour    INTEGER NOT NULL,
    app     TEXT NOT NULL,
    seconds INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, hour, app)
);
"#;

/// 全部建表语句（幂等，重复执行无副作用）。
///
/// 按表拆开而非常量拼接：`concat!` 只接受字面量、无法组合 `const &str`；
/// 拆开后单表 DDL（如 `CREATE_OT_RECORDS`）还能被单测复用来建 in-memory 表。
const TABLE_DDL: &[&str] = &[
    CREATE_OT_RECORDS,
    CREATE_ACT_HOURLY,
    CREATE_ACT_KEYS,
    CREATE_APP_USAGE,
    CREATE_APP_USAGE_HOURLY,
    CREATE_AUDIO_USAGE,
    CREATE_AUDIO_USAGE_HOURLY,
];

/// 建全部数据表（幂等，重复执行无副作用）。
///
/// v1.0.0 起已有真实用户库，schema 变更**不能**再靠删库重建：建表后再按
/// [`EXTRA_COLUMNS`] 幂等补列（见 [`ensure_column`]）。
pub(crate) fn init_tables(db: &Connection) {
    for ddl in TABLE_DDL {
        db.execute_batch(ddl).expect("初始化数据库表失败");
    }
    for (table, col, decl) in EXTRA_COLUMNS {
        ensure_column(db, table, col, decl);
    }
}

/// 历史库需要补上的列：`(表名, 列名, 列声明)`。
///
/// 全新库的建表 DDL 里已含这些列，只有 v1.0.0 及更早的库才需要 ALTER。
/// 列一旦长期稳定可把声明并进 DDL、从这里移除。
const EXTRA_COLUMNS: &[(&str, &str, &str)] = &[(
    "ot_records",
    "cross_midnight",
    "INTEGER NOT NULL DEFAULT 0",
)];

/// 幂等补列：仅当列不存在时 `ALTER TABLE ... ADD COLUMN`。
///
/// **必须先查 `pragma_table_info` 再 ALTER**。直接无条件 ALTER 会让全新库报
/// 「duplicate column name」（它的建表 DDL 里已有该列）——历史上正是这么翻的车，
/// 当时的结论是「不做迁移」，代价是老库永远升不了级。
fn ensure_column(db: &Connection, table: &str, col: &str, decl: &str) {
    let exists: i32 = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            params![table, col],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if exists > 0 {
        return;
    }
    if let Err(e) = db.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"), []) {
        // 补列失败不阻断启动：缺的只是「跨午夜」标记，已有记录照常读写，
        // 仅写入新记录时会因列缺失报错——把原因记下来便于排查。
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            debug_log(&format!("补列 {table}.{col} 失败: {e}"))
        }));
    }
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
        let db = Connection::open(db_path()).expect("无法打开 niuma.db");
        let _ = db.busy_timeout(Duration::from_secs(5));
        let _ = db.pragma_update(None, "journal_mode", "WAL");
        init_tables(&db);
        Mutex::new(db)
    })
}

/// 在全局连接上执行一段数据库操作，**统一**处理锁中毒与错误上报。
///
/// 背景：此前 4 个模块各自 `conn().lock().unwrap()`（共 10 处），有两个隐患——
/// 1. **故障连锁**：任一处 DB 操作 panic 都会让这把全局 Mutex 中毒，此后
///    加班 / 活动 / 应用 / 媒体每一次 `unwrap()` 全部连锁 panic，
///    一个点出错即整个程序持续崩溃，而连接本身其实完好可用；
/// 2. **错误静默**：`let _ = g.execute(...)` 的写法让失败彻底消失，无人知晓。
///
/// 这里统一收口：
/// - 锁中毒时**自愈**——取回内层连接继续服务（中毒只说明曾有线程在持锁期间
///   panic，不代表 SQLite 连接损坏），并记一条 debug.log；
/// - 每次失败都写 `debug_log`，日志带具体 SQL 错误；
/// - 返回 `Result`，由调用方决定降级（忽略 / 返回空 / 上报前端），不再 panic。
///
/// 闭包拿到 `&mut Connection`：既能执行写操作，也能开事务；
/// 只读场景传进去会自动 reborrow 成 `&Connection`。
pub fn with_db<T>(f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let mut guard = match conn().lock() {
        Ok(g) => g,
        Err(poisoned) => {
            debug_log("[db] 检测到连接锁中毒，已自愈（此前有线程在持锁期间 panic）");
            poisoned.into_inner()
        }
    };
    f(&mut guard).map_err(|e| {
        let msg = e.to_string();
        debug_log(&format!("[db] SQL 执行失败: {msg}"));
        msg
    })
}

#[cfg(test)]
mod tests {
    use super::init_tables;
    use rusqlite::Connection;

    /// 回归：建表幂等——重复执行不报错、source 列存在。
    /// （历史教训：版本化迁移曾在全新库上重复加列 source，首跑即 panic。）
    #[test]
    fn init_tables_is_idempotent() {
        let db = Connection::open_in_memory().unwrap();
        init_tables(&db);
        init_tables(&db); // 第二次不应报错
        let has_source: i32 = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('ot_records') WHERE name = 'source'",
                [],
                |r| r.get::<_, i32>(0),
            )
            .unwrap_or(0);
        assert_eq!(has_source, 1);
    }

    /// 老库补列：v1.0.0 建的库没有 cross_midnight，init_tables 必须补上而不是崩。
    /// 这正是历史上「duplicate column name」翻车的场景——所以 ensure_column
    /// 必须先查 pragma_table_info 再 ALTER，不能无条件加。
    #[test]
    fn init_tables_adds_missing_column_to_legacy_db() {
        let db = Connection::open_in_memory().unwrap();
        // 模拟 v1.0.0 的旧 schema：少一列 cross_midnight
        db.execute_batch(
            "CREATE TABLE ot_records (
                date        TEXT PRIMARY KEY,
                lock_time   TEXT NOT NULL,
                ot_start    TEXT NOT NULL,
                raw_hours   REAL NOT NULL,
                valid_hours REAL NOT NULL,
                fee         REAL NOT NULL,
                meal        REAL NOT NULL,
                total       REAL NOT NULL,
                source      INTEGER NOT NULL DEFAULT 0
            )",
        )
        .unwrap();
        init_tables(&db);
        let n: i32 = db
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('ot_records') WHERE name = 'cross_midnight'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "老库应补上 cross_midnight 列");
    }
}
