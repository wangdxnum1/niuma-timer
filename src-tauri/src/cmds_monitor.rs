//! 监控查询命令：键鼠活动、应用使用、媒体播放三个 summary（前端每 2 秒轮询）。
//!
//! 二期自 main.rs 切出（纯移动）。

use tauri::State;

use crate::activity;
use crate::app_usage;
use crate::audio_usage;
use crate::state::AppState;
use crate::sync;

/// 获取今日鼠标/键盘活动统计（逐小时 + 汇总 + 高频按键）
/// `(async)`：前端每 2 秒轮询的三个 summary 都不该占主线程，理由同 get_storage_info
#[tauri::command(async)]
pub(crate) fn get_activity_summary(date: Option<String>) -> activity::ActivitySummary {
    activity::summary_for(date.as_deref())
}

/// 获取今日应用使用时长统计（各应用累计 + 24 小时分布）
///
/// `known_icons`：前端已缓存图标的应用名，命中者不再回传 base64（图标是几 KB~几十 KB
/// 的 data URL，每 2 秒轮询整批搬运纯属浪费，只有新应用才需要传一次）。
#[tauri::command(async)]
pub(crate) fn get_app_usage_summary(
    state: State<'_, AppState>,
    known_icons: Vec<String>,
    date: Option<String>,
) -> app_usage::AppUsageSummary {
    // 摸鱼统计需要用户分类（config.app_categories）参与归类，取配置副本传入
    let cfg = sync::lock(&state.config, "state.config").clone();
    app_usage::summary(&known_icons, date.as_deref(), &cfg)
}

/// 获取今日媒体播放时长统计（各应用累计 + 24 小时分布）
#[tauri::command(async)]
pub(crate) fn get_audio_usage_summary(
    known_icons: Vec<String>,
    date: Option<String>,
) -> audio_usage::AudioUsageSummary {
    audio_usage::summary(&known_icons, date.as_deref())
}
