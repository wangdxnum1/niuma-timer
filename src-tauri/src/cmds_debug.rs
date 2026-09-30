//! 调试卡命令（v1.3.0 彩蛋）：验证通知通道、重置提醒状态、立即调度、模拟久坐。
//!
//! 二期自 main.rs 切出（纯移动）。

use tauri::Manager;

use crate::remind;
use crate::state::get_status;
use crate::AppState;

/// 调试卡「下班提醒」：直发一条真实文案的系统通知，点一下即可验证通知通道
#[tauri::command]
pub(crate) fn test_offwork_notify(app: tauri::AppHandle) {
    let st = get_status(app.state::<AppState>().inner());
    let (title, body) = remind::offwork_texts(st.earned);
    remind::notify(&app, &title, &body);
}

/// 调试卡「休息提醒」：直发久坐提醒文案（分钟数取当前配置阈值），验证通知通道
#[tauri::command]
pub(crate) fn test_sedentary_notify(app: tauri::AppHandle) {
    let cfg = crate::cmds_core::load_config(app.state::<AppState>());
    let (title, body) = remind::sedentary_texts(cfg.remind_sedentary_minutes as i64);
    remind::notify(&app, &title, &body);
}

/// 调试卡「重置提醒状态」：清空下班「今天已提醒」标记与久坐冷却/连续计时。
/// 下班提醒每天只触发一次，不重置当天就没法再复测真实触发链路。
#[tauri::command]
pub(crate) fn reset_remind_state() {
    remind::reset_state();
}

/// 调试卡「立即调度」：按真实规则跑一次提醒判定（久坐 + 下班），与后台每 60 秒
/// 的自动调度走同一函数，用来验证触发条件而不必干等下一拍。
#[tauri::command]
pub(crate) fn run_remind_tick(app: tauri::AppHandle) {
    remind::tick(&app);
}

/// 调试卡「模拟久坐」：把连续活跃起点前拨到阈值之前，再跑一次真实调度，
/// 秒级复现「久坐满阈值」的触发链路（判定 / 文案 / 通知 / 状态重置全走真实代码）。
/// 真实链路阈值最小 1 分钟、tick 又是 60 秒一拍，手工复测至少要等一拍。
#[tauri::command]
pub(crate) fn test_sedentary_trigger(app: tauri::AppHandle) {
    let cfg = crate::cmds_core::load_config(app.state::<AppState>());
    remind::force_sedentary_since(cfg.remind_sedentary_minutes as i64);
    remind::tick(&app);
}
