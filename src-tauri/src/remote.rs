//! 远程会话识别：判定「此刻的键鼠操作是否来自远程控制」。
//!
//! 用途：远程连接（RDP / 向日葵 / ToDesk / UU 等）会制造假的锁屏 / 会话事件，
//! 把「连上/断开远程」误当成「下班离开」，导致自动加班记录被刷新、产生误差。
//! 本模块提供实时判定，供 `main::maybe_record_overtime_lock` 在远程期间跳过自动记录。
//!
//! 两个信号取或：
//! 1. RDP：`GetSystemMetrics(SM_REMOTESESSION)` —— 系统权威、实时、零成本。
//! 2. 第三方虚拟设备：复用 Raw Input 钩子，每次 `WM_INPUT` 带 `hDevice`，
//!    解析设备名命中远程特征串即记「最近一次远程输入时刻」，配合衰减窗口覆盖
//!    「连着但没动鼠标」的 idle 期。

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};

/// 第三方远程虚拟设备的设备名特征串（大小写不敏感）。
///
/// - `RDP` / `RDP_KBD` / `RDP_MOU`：微软远程桌面虚拟键鼠，最稳。
/// - `SUNLOGIN`：向日葵；`TODESK`：ToDesk；`UU`：UU 远程。
///
/// 注意**不包含**裸 `VIRTUAL` / `MIRROR` —— 物理设备也可能报这类词，误伤会反噬。
/// 名单已覆盖主流远程工具，必要时可继续扩充。
pub const REMOTE_WINDOW: Duration = Duration::from_secs(60);

/// 最近一次「确认来自远程设备」的输入时刻；`None` = 从未见过远程输入。
static LAST_REMOTE_INPUT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// 设备句柄 → 是否已判定为远程（首次见到才解析设备名，之后直接查表，零成本）。
static DEVICE_REMOTE: OnceLock<Mutex<HashMap<usize, bool>>> = OnceLock::new();

/// 当前会话是否为 RDP 远程会话（系统权威）。
pub fn is_rdp_session() -> bool {
    // SM_REMOTESESSION 返回非零表示当前会话是远程会话。
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

/// 设备名是否命中远程特征串（大小写不敏感）。
pub fn remote_device_signature(name: &str) -> bool {
    let up = name.to_uppercase();
    const SIG: &[&str] = &["RDP", "RDP_KBD", "RDP_MOU", "SUNLOGIN", "TODESK", "UU"];
    SIG.iter().any(|s| up.contains(s))
}

/// 观测一次输入的设备句柄：首次见到才解析设备名（开销大，故缓存），
/// 由 `activity::raw_wndproc` 的 `WM_INPUT` 回调每事件调用——已判定为远程的设备
/// 只刷新时间戳、不重复解析；未判定过的设备才解析一次，故对高频鼠标事件零额外开销。
pub fn observe_input_device(h: usize) {
    // 已分类：远程设备刷新时间戳后立即返回；非远程直接返回（不解析名字）。
    {
        let tbl = DEVICE_REMOTE.get_or_init(|| Mutex::new(HashMap::new()));
        let g = tbl.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&is_rem) = g.get(&h) {
            if is_rem {
                if let Some(slot) = LAST_REMOTE_INPUT.get() {
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                }
            }
            return;
        }
    }
    // 首次见到：解析设备名（仅在此时付出 GetRawInputDeviceInfoW 开销）
    let name = match crate::win::raw_input_device_name(HANDLE(h as *mut c_void)) {
        Some(n) => n,
        None => {
            DEVICE_REMOTE
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(h, false);
            return;
        }
    };
    let is_rem = remote_device_signature(&name);
    if is_rem {
        if let Some(slot) = LAST_REMOTE_INPUT.get() {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        }
    }
    DEVICE_REMOTE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(h, is_rem);
}

/// 聚合判定：当前是否处于远程会话（RDP 或近期有远程设备输入）。
pub fn is_remote_active(window: Duration) -> bool {
    if is_rdp_session() {
        return true;
    }
    match LAST_REMOTE_INPUT.get().and_then(|m| *m.lock().unwrap_or_else(|e| e.into_inner())) {
        Some(t) => t.elapsed() < window,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_hits_known_remotes() {
        assert!(remote_device_signature("ROOT\\RDP_KBD\\0000"));
        assert!(remote_device_signature("ACPI\\SunloginRemote\\1"));
        assert!(remote_device_signature("HID\\ToDesk_Virtual_KB\\2"));
        assert!(remote_device_signature("USB\\VID_1234&PID_UU\\3"));
    }

    #[test]
    fn signature_skips_physical() {
        assert!(!remote_device_signature("HID\\VID_046D&PID_C52B\\4")); // Logitech 物理键鼠
        assert!(!remote_device_signature("ACPI\\Virtual_Keyboard\\5")); // 裸 VIRTUAL 不误伤
        assert!(!remote_device_signature(""));
    }

    /// 纯函数版聚合判定（便于单测，不依赖真实时钟与系统调用）。
    fn active_given(last: Option<Instant>, now: Instant, window: Duration, rdp: bool) -> bool {
        if rdp {
            return true;
        }
        match last {
            Some(t) => now.saturating_duration_since(t) < window,
            None => false,
        }
    }

    #[test]
    fn active_window_boundary() {
        let now = Instant::now();
        // 59s 内 = 仍远程；61s 外 = 已断开
        assert!(active_given(
            Some(now - Duration::from_secs(59)),
            now,
            REMOTE_WINDOW,
            false
        ));
        assert!(!active_given(
            Some(now - Duration::from_secs(61)),
            now,
            REMOTE_WINDOW,
            false
        ));
        // 无远程输入 + 非 RDP = 非远程
        assert!(!active_given(None, now, REMOTE_WINDOW, false));
        // 任意时刻 RDP 都算远程
        assert!(active_given(None, now, REMOTE_WINDOW, true));
    }
}
