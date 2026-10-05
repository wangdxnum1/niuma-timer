//! 日级休假标记（day_override 表）：用户手动标记「这天不上班」。
//!
//! 口径（spec 2026-10-05）：标记日一律按休息日处理——当日状态、历史账单的
//! 出勤/应赚/摸鱼率全部排除，月工作日分母（自动模式）剔除。kind 仅作记录
//! 展示，不影响口径；带薪假计薪是未来可选细化。
//!
//! 读路径分三层：
//! - 当日状态与月分母（`state::get_status` 每秒跑）：读进程内的**当月覆盖集**
//!   缓存 [`today_kind`] / [`month_off_workdays`]，启动/跨天时 [`reload_month`]
//!   装载、set 命令写穿——绝不每秒查库；
//! - 历史聚合（weekbill / focus / insights 起止范围一次）：[`range_overrides`]。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use rusqlite::{params, Connection};

/// 休假类型（记录展示用，不影响口径）
pub(crate) const KINDS: &[&str] = &["休假", "年假", "病假", "事假", "调休"];

/// 当月标记缓存存储类型：(YYYY-MM, date → kind)
type MonthCache = Option<(String, HashMap<String, String>)>;
static MONTH: OnceLock<Mutex<MonthCache>> = OnceLock::new();

fn month_cell() -> &'static Mutex<Option<(String, HashMap<String, String>)>> {
    MONTH.get_or_init(|| Mutex::new(None))
}

fn ym_of(date: &str) -> &str {
    // YYYY-MM-DD 截前 7 位；非法串交给查不到处理
    if date.len() >= 7 {
        &date[..7]
    } else {
        date
    }
}

/// 当日是否标记休假。缓存月份与 today 不符（跨月未及装载）返回 None，
/// 退化为按日历口径——宁可短暂失真，也不把上个月的标记算到这个月头上。
pub(crate) fn today_kind(today: &str) -> Option<String> {
    let guard = month_cell().lock().unwrap_or_else(|p| p.into_inner());
    let (ym, map) = guard.as_ref()?;
    if ym == ym_of(today) {
        map.get(today).cloned()
    } else {
        None
    }
}

/// 当月标记中、满足日历工作日口径的天数（供月分母剔除；休周末的误标记不扣）。
/// 谓词由调用方注入（节假日缓存在那边），缓存月份不符返回 0。
pub(crate) fn month_off_workdays<F: Fn(&str) -> bool>(ym: &str, is_calendar_workday: F) -> u32 {
    let guard = month_cell().lock().unwrap_or_else(|p| p.into_inner());
    let (_, map) = match guard.as_ref() {
        Some(v) if v.0 == ym => v,
        _ => return 0,
    };
    map.keys().filter(|d| is_calendar_workday(d)).count() as u32
}

/// 启动/跨月时装载当月标记（conn 由调用方给）
pub(crate) fn reload_month(conn: &Connection, year: i32, month: u32) -> rusqlite::Result<()> {
    let start = format!("{year:04}-{month:02}-01");
    let end = format!(
        "{year:04}-{month:02}-{day:02}",
        day = crate::holiday::days_in_month(year, month)
    );
    let map = range_overrides(conn, &start, &end)?;
    let ym = format!("{year:04}-{month:02}");
    *month_cell().lock().unwrap_or_else(|p| p.into_inner()) = Some((ym, map));
    Ok(())
}

/// set 命令写穿缓存。非当月标记不动缓存位；缓存未装载（None）时也直接落位，
/// 避免启动装载失败后标记永远不可见。
pub(crate) fn update_cache(date: &str, kind: Option<String>) {
    let ym = ym_of(date).to_string();
    let mut guard = month_cell().lock().unwrap_or_else(|p| p.into_inner());
    match guard.as_ref() {
        Some((cached_ym, _)) if *cached_ym != ym => {}
        _ => {
            let (_, map) = guard.get_or_insert_with(|| (ym.clone(), HashMap::new()));
            match kind {
                Some(k) => {
                    map.insert(date.to_string(), k);
                }
                None => {
                    map.remove(date);
                }
            }
        }
    }
}

/// upsert / 删除（kind = None 删除）。写穿当月缓存。
pub(crate) fn set(conn: &Connection, date: &str, kind: Option<&str>) -> rusqlite::Result<()> {
    match kind {
        Some(k) => {
            conn.execute(
                "INSERT INTO day_override (date, kind) VALUES (?1, ?2)
                 ON CONFLICT(date) DO UPDATE SET kind = excluded.kind",
                params![date, k],
            )?;
        }
        None => {
            conn.execute("DELETE FROM day_override WHERE date = ?1", params![date])?;
        }
    }
    update_cache(date, kind.map(|s| s.to_string()));
    Ok(())
}

/// 起止范围（含端点，YYYY-MM-DD 字典序）内的休假标记
pub(crate) fn range_overrides(
    conn: &Connection,
    start: &str,
    end: &str,
) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT date, kind FROM day_override WHERE date >= ?1 AND date <= ?2")?;
    let rows = stmt.query_map(params![start, end], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (d, k) = row?;
        map.insert(d, k);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// in-memory 库（DDL 取自 db::CREATE_DAY_OVERRIDE，勿手抄表结构）
    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::CREATE_DAY_OVERRIDE).unwrap();
        conn
    }

    // 缓存是进程级共享状态，cargo test 并行跑线程——以下场景合并在单个测试里
    // 顺序执行，避免并行互踩（纯 SQL 的 set/range 断言在另一个测试里，不碰缓存位）。
    #[test]
    fn cache_roundtrip_single_thread() {
        let conn = mem();
        // 未装载时标记当月 → 写穿可见
        set(&conn, "2026-10-05", Some("事假")).unwrap();
        assert_eq!(today_kind("2026-10-05").as_deref(), Some("事假"));
        // 非当月标记不污染当月缓存位
        set(&conn, "2025-01-01", Some("年假")).unwrap();
        assert_eq!(today_kind("2026-10-05").as_deref(), Some("事假"));
        assert_eq!(today_kind("2025-01-01"), None);
        // 当月删除 → None
        set(&conn, "2026-10-05", None).unwrap();
        assert_eq!(today_kind("2026-10-05"), None);
        // reload 装载整月：跨月数据不进缓存
        set(&conn, "2026-10-05", Some("年假")).unwrap();
        set(&conn, "2026-10-26", Some("调休")).unwrap();
        set(&conn, "2026-09-30", Some("年假")).unwrap();
        reload_month(&conn, 2026, 10).unwrap();
        assert_eq!(today_kind("2026-10-05").as_deref(), Some("年假"));
        assert_eq!(today_kind("2026-10-26").as_deref(), Some("调休"));
        assert_eq!(today_kind("2026-09-30"), None);
        // 跨月读取：缓存月份与新日期不符 → None（等 reload）
        assert_eq!(today_kind("2026-11-03"), None);
        // 月分母计数：谓词把 10-05 判为工作日、10-26 判为休息 → 只扣 1
        let n = month_off_workdays("2026-10", |d| d.ends_with("-05"));
        assert_eq!(n, 1);
        assert_eq!(month_off_workdays("2026-09", |_| true), 0, "月份不符返回 0");
    }

    // 纯 SQL 层：upsert/删除/范围含端点。日期特意用 2027-01：与缓存测试的
    // 2026-10 不同月，写穿互不干扰（update_cache 对非当月写入是 no-op）。
    #[test]
    fn set_upsert_delete_and_range() {
        let conn = mem();
        set(&conn, "2026-12-31", Some("年假")).unwrap();
        set(&conn, "2027-01-01", Some("年假")).unwrap();
        set(&conn, "2027-01-05", Some("病假")).unwrap();
        set(&conn, "2027-01-31", Some("调休")).unwrap();
        set(&conn, "2027-02-01", Some("年假")).unwrap();
        let map = range_overrides(&conn, "2027-01-01", "2027-01-31").unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(map.get("2027-01-05").unwrap(), "病假");
        // upsert 覆盖
        set(&conn, "2027-01-05", Some("事假")).unwrap();
        assert_eq!(
            range_overrides(&conn, "2027-01-01", "2027-01-31")
                .unwrap()
                .get("2027-01-05"),
            Some(&"事假".to_string())
        );
        // 删除
        set(&conn, "2027-01-05", None).unwrap();
        assert!(!range_overrides(&conn, "2027-01-01", "2027-01-31")
            .unwrap()
            .contains_key("2027-01-05"));
    }

    #[test]
    fn kinds_are_the_display_enum() {
        assert!(KINDS.contains(&"休假"));
        assert!(KINDS.contains(&"调休"));
    }
}
