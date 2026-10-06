//! 里程碑聚合（2026-10-06）：把既有表的累计量一次算齐，报告页画成就阶梯。
//!
//! 零新表——六项全是 SUM/COUNT/DISTINCT，直接读 ot_records / act_hourly /
//! focus_sessions。阶梯阈值在前端常量里（后端只出原始量，调档不用动后端）；
//! 「赚满 ¥10 万」类需要逐日历日重算满勤薪，口径复杂，v1 不做（见 spec 非目标）。

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Milestones {
    /// 加班有效时长（小时，Σ valid_hours）
    pub ot_hours: f64,
    /// 加班费累计（元，Σ total，fee+meal 落盘口径）
    pub ot_fee: f64,
    /// 键盘敲击累计（次，Σ act_hourly.keys）
    pub keystrokes: i64,
    /// 键鼠移动累计（原始像素，96dpi 距离换算在前端 fmtDist）
    pub distance_px: i64,
    /// 专注累计（分钟，Σ focus_sessions.minutes）
    pub focus_minutes: i64,
    /// 有监控证据的天数（act_hourly 出现过的日期数）
    pub active_days: i64,
    /// 最早数据日期（YYYY-MM-DD，无记录为 None）
    pub first_date: Option<String>,
}

/// 单连接聚合内核（正式走 with_db，单测走 in-memory）
pub(crate) fn assemble(conn: &rusqlite::Connection) -> rusqlite::Result<Milestones> {
    let sum_f = |sql: &str| -> rusqlite::Result<f64> {
        Ok(conn
            .query_row(sql, [], |r| r.get::<_, Option<f64>>(0))?
            .unwrap_or(0.0))
    };
    let sum_i = |sql: &str| -> rusqlite::Result<i64> {
        Ok(conn
            .query_row(sql, [], |r| r.get::<_, Option<i64>>(0))?
            .unwrap_or(0))
    };
    let ot_hours = sum_f("SELECT SUM(valid_hours) FROM ot_records")?;
    let ot_fee = sum_f("SELECT SUM(total) FROM ot_records")?;
    let keystrokes = sum_i("SELECT SUM(keys) FROM act_hourly")?;
    let distance_px = sum_i("SELECT SUM(pixels) FROM act_hourly")?;
    let focus_minutes = sum_i("SELECT SUM(minutes) FROM focus_sessions")?;
    let active_days = conn.query_row("SELECT COUNT(DISTINCT date) FROM act_hourly", [], |r| {
        r.get::<_, i64>(0)
    })?;
    let first_date = conn.query_row("SELECT MIN(date) FROM act_hourly", [], |r| {
        r.get::<_, Option<String>>(0)
    })?;
    Ok(Milestones {
        ot_hours,
        ot_fee,
        keystrokes,
        distance_px,
        focus_minutes,
        active_days,
        first_date,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::CREATE_OT_RECORDS).unwrap();
        conn.execute_batch(crate::db::CREATE_ACT_HOURLY).unwrap();
        conn.execute_batch(crate::db::CREATE_FOCUS_SESSIONS)
            .unwrap();
        conn
    }

    #[test]
    fn aggregates_sum_distinct_and_first_date() {
        let conn = mem();
        conn.execute(
            "INSERT INTO ot_records (date, lock_time, ot_start, raw_hours, valid_hours, fee, meal, total) \
             VALUES ('2026-09-01', '21:00', '18:30', 2.0, 2.0, 50, 15, 65),
                    ('2026-09-03', '22:00', '18:30', 1.0, 1.0, 25, 0, 25)",
            [],
        )
        .unwrap();
        // 两天 act_hourly + 两天 focus（其中一天与 act 同日）→ active_days = 2
        conn.execute(
            "INSERT INTO act_hourly (date, hour, moves, pixels, `left`, dbl, `right`, wheel, wheel_ticks, mid, xbtn, keys) \
             VALUES ('2026-09-01', 9, 100, 960000, 10, 2, 3, 50, 5, 1, 0, 600000),
                    ('2026-09-02', 9, 100, 1920000, 10, 2, 3, 50, 5, 1, 0, 400000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO focus_sessions (date, start_hm, end_hm, minutes) \
             VALUES ('2026-09-01', '10:00', '10:25', 25), ('2026-09-02', '14:00', '14:35', 35)",
            [],
        )
        .unwrap();
        let m = assemble(&conn).unwrap();
        assert_eq!(m.ot_hours, 3.0);
        assert_eq!(m.ot_fee, 90.0);
        assert_eq!(m.keystrokes, 1_000_000);
        assert_eq!(m.distance_px, 2_880_000);
        assert_eq!(m.focus_minutes, 60);
        assert_eq!(m.active_days, 2);
        assert_eq!(m.first_date.as_deref(), Some("2026-09-01"));
    }

    #[test]
    fn empty_tables_yield_zeros_and_no_first_date() {
        let conn = mem();
        let m = assemble(&conn).unwrap();
        assert_eq!(m.ot_hours, 0.0);
        assert_eq!(m.keystrokes, 0);
        assert_eq!(m.active_days, 0);
        assert_eq!(m.first_date, None);
    }
}
