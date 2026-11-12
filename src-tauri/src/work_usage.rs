//! Precisely captured working-window usage. Legacy totals carry no timing evidence.
use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::config::Config;

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct WorkUsage {
    pub front_seconds: i64,
    pub slack_seconds: i64,
    pub legacy_seconds: i64,
}

fn minute(hm: &str) -> i64 {
    let Some((h, m)) = hm.split_once(':') else {
        return -1;
    };
    match (h.parse::<i64>(), m.parse::<i64>()) {
        (Ok(h), Ok(m)) if (0..24).contains(&h) && (0..60).contains(&m) => h * 60 + m,
        _ => -1,
    }
}

/// Calendar/leave eligibility is the caller's responsibility. Each minute is counted
/// once even when AM and PM windows overlap. Never reconstruct old hourly records.
pub(crate) fn range(
    conn: &Connection,
    start: &str,
    end_exclusive: &str,
    cfg: &Config,
) -> rusqlite::Result<HashMap<String, WorkUsage>> {
    let mut out: HashMap<String, WorkUsage> = HashMap::new();
    let mut captured: HashMap<String, i64> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT date, app, SUM(seconds),
         SUM(CASE WHEN (minute >= ?3 AND minute < ?4 AND ?3 >= 0)
                   OR (minute >= ?5 AND minute < ?6 AND ?5 >= 0)
                  THEN seconds ELSE 0 END)
         FROM app_usage_minutes WHERE date >= ?1 AND date < ?2 GROUP BY date, app",
    )?;
    let rows = stmt.query_map(
        params![
            start,
            end_exclusive,
            minute(&cfg.am_start),
            minute(&cfg.am_end),
            minute(&cfg.pm_start),
            minute(&cfg.pm_end)
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    for row in rows {
        let (date, app, all, work) = row?;
        *captured.entry(date.clone()).or_default() += all;
        let usage = out.entry(date).or_default();
        usage.front_seconds += work;
        if crate::app_usage::category_of(&app, cfg) == crate::app_usage::CAT_SLACK {
            usage.slack_seconds += work;
        }
    }
    let mut stmt = conn.prepare(
        "SELECT date, SUM(seconds) FROM app_usage WHERE date >= ?1 AND date < ?2 GROUP BY date",
    )?;
    let rows = stmt.query_map(params![start, end_exclusive], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (date, raw) = row?;
        out.entry(date.clone()).or_default().legacy_seconds =
            (raw - captured.get(&date).copied().unwrap_or_default()).max(0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::CREATE_APP_USAGE).unwrap();
        conn.execute_batch(crate::db::CREATE_APP_USAGE_MINUTES)
            .unwrap();
        conn
    }

    #[test]
    fn precise_windows_boundaries_lunch_overlap_and_classification() {
        let conn = connection();
        // Include exact starts, exclude exact ends and lunch. Four app categories
        // all participate in front_seconds, only the configured slack category does.
        for (minute, app) in [
            (569, "网易云音乐"),
            (570, "网易云音乐"),
            (719, "VS Code"),
            (720, "网易云音乐"),
            (779, "网易云音乐"),
            (780, "微信"),
            (1079, "Chrome"),
            (1080, "网易云音乐"),
        ] {
            conn.execute(
                "INSERT INTO app_usage_minutes VALUES ('2026-11-12', ?1, ?2, 10)",
                params![minute, app],
            )
            .unwrap();
        }
        let mut cfg = Config {
            am_start: "09:30".into(),
            ..Config::default()
        };
        let usage = range(&conn, "2026-11-12", "2026-11-13", &cfg).unwrap()["2026-11-12"];
        assert_eq!((usage.front_seconds, usage.slack_seconds), (40, 10));
        cfg.pm_start = "11:00".into();
        cfg.app_categories
            .insert("微信".into(), crate::app_usage::CAT_SLACK.into());
        let usage = range(&conn, "2026-11-12", "2026-11-13", &cfg).unwrap()["2026-11-12"];
        assert_eq!((usage.front_seconds, usage.slack_seconds), (60, 40));
        assert!(range(&conn, "2026-11-13", "2026-11-14", &cfg)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn legacy_is_raw_less_all_captured_even_outside_windows() {
        let conn = connection();
        conn.execute_batch(
            "INSERT INTO app_usage VALUES ('2026-11-10','Chrome',100),
            ('2026-11-11','Chrome',150), ('2026-11-12','Chrome',5);
            INSERT INTO app_usage_minutes VALUES ('2026-11-11',540,'Chrome',30),
            ('2026-11-11',1200,'Chrome',70), ('2026-11-12',540,'Chrome',10);",
        )
        .unwrap();
        let rows = range(&conn, "2026-11-10", "2026-11-13", &Config::default()).unwrap();
        assert_eq!(
            (
                rows["2026-11-10"].front_seconds,
                rows["2026-11-10"].legacy_seconds
            ),
            (0, 100)
        );
        assert_eq!(
            (
                rows["2026-11-11"].front_seconds,
                rows["2026-11-11"].legacy_seconds
            ),
            (30, 50)
        );
        assert_eq!(rows["2026-11-12"].legacy_seconds, 0);
    }

    #[test]
    fn missing_precise_table_propagates_unavailability() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::CREATE_APP_USAGE).unwrap();
        assert!(range(&conn, "2026-11-12", "2026-11-13", &Config::default()).is_err());
    }
}
