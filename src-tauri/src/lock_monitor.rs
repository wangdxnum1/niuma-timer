//! Windows 锁屏检测：后台线程创建隐藏消息窗口，
//! 通过 WTSRegisterSessionNotification 接收 WM_WTSSESSION_CHANGE，
//! 收到 WTS_SESSION_LOCK 时记录时间戳，并维护全局「离开(AWAY)」状态。
//!
//! AWAY 状态供「应用使用」「键鼠活动」等时间类统计在锁屏后暂停，
//! 避免把锁屏界面 / 解锁输入误记为工作活动。媒体播放（听歌）不受影响。

use crate::sync;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use chrono::Local;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};

/// WM_WTSSESSION_CHANGE = 0x02B1，定义在 winuser.h
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
/// WTS_SESSION_LOCK = 0x7，定义在 wtsapi32.h
const WTS_SESSION_LOCK: u32 = 0x7;
/// WTS_SESSION_UNLOCK = 0x8
const WTS_SESSION_UNLOCK: u32 = 0x8;

/// 最近一次锁屏的 Unix 时间戳（秒）
static LAST_LOCK_TIME: Mutex<Option<i64>> = Mutex::new(None);

/// 全局「离开」状态：锁屏时为 true，解锁为 false。
/// 供 app_usage / activity 在锁屏后暂停时间类统计。
static AWAY: AtomicBool = AtomicBool::new(false);

/// 查询当前是否处于「离开」（锁屏）状态
pub fn is_away() -> bool {
    AWAY.load(Ordering::Relaxed)
}

/// 查询最近一次锁屏时间戳
pub fn last_lock_timestamp() -> Option<i64> {
    *sync::lock(&LAST_LOCK_TIME, "lock_monitor::LAST_LOCK_TIME")
}

/// 启动锁屏监听线程（幂等，重复调用仅首次生效）
pub fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut delay = Duration::from_secs(1);
        loop {
            match try_run() {
                Ok(()) => return,
                Err(e) => {
                    crate::db::debug_log(&format!(
                        "[lock_monitor] {e}，{}s 后重试",
                        delay.as_secs()
                    ));
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_secs(60));
                }
            }
        }
    });
}

fn try_run() -> Result<(), String> {
    unsafe {
        let class_name = w!("NiumaLockMonitor");

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: class_name,
            ..Default::default()
        };

        let atom = RegisterClassW(&wc);
        if atom == 0 {
            let err = GetLastError();
            if err != ERROR_CLASS_ALREADY_EXISTS {
                return Err(format!("RegisterClassW 失败: {err:?}"));
            }
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!(""),
            WINDOW_STYLE::default(),
            0, 0, 0, 0,
            None, None, None, None,
        )
        .map_err(|e| format!("CreateWindowExW 失败: {e}"))?;

        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)
            .map_err(|e| format!("WTSRegisterSessionNotification 失败: {e}"))?;

        crate::win::run_message_loop();
        Ok(())
    }
}

/// 窗口过程：接收 WM_WTSSESSION_CHANGE 消息
extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_WTSSESSION_CHANGE {
        match wparam.0 as u32 {
            WTS_SESSION_LOCK => {
                AWAY.store(true, Ordering::Relaxed);
                let now = Local::now().timestamp();
                *sync::lock(&LAST_LOCK_TIME, "lock_monitor::LAST_LOCK_TIME") = Some(now);
            }
            WTS_SESSION_UNLOCK => {
                AWAY.store(false, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
