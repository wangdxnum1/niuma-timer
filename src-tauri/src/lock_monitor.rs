//! Windows 锁屏检测：后台线程创建隐藏消息窗口，
//! 通过 WTSRegisterSessionNotification 接收 WM_WTSSESSION_CHANGE，
//! 收到 WTS_SESSION_LOCK 时记录时间戳。

use std::sync::Mutex;
use chrono::Local;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    RegisterClassW, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};

/// WM_WTSSESSION_CHANGE = 0x02B1，定义在 winuser.h
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
/// WTS_SESSION_LOCK = 0x7，定义在 wtsapi32.h
const WTS_SESSION_LOCK: u32 = 0x7;

/// 最近一次锁屏的 Unix 时间戳（秒）
static LAST_LOCK_TIME: Mutex<Option<i64>> = Mutex::new(None);

/// 查询最近一次锁屏时间戳
pub fn last_lock_timestamp() -> Option<i64> {
    *LAST_LOCK_TIME.lock().unwrap()
}

/// 启动锁屏监听线程（幂等，重复调用仅首次生效）
pub fn start() {
    std::thread::spawn(|| unsafe {
        let class_name = w!("NiumaLockMonitor");

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: class_name,
            ..Default::default()
        };

        let atom = RegisterClassW(&wc);
        if atom == 0 {
            return;
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!(""),
            WINDOW_STYLE::default(),
            0, 0, 0, 0,
            None, None, None, None,
        );

        let hwnd = match hwnd {
            Ok(h) => h,
            Err(_) => return,
        };

        let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);

        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if !r.as_bool() {
                break;
            }
            let _ = DispatchMessageW(&msg);
        }
    });
}

/// 窗口过程：接收 WM_WTSSESSION_CHANGE 消息
extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_WTSSESSION_CHANGE {
        if wparam.0 as u32 == WTS_SESSION_LOCK {
            let now = Local::now().timestamp();
            *LAST_LOCK_TIME.lock().unwrap() = Some(now);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
