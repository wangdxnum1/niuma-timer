//! Windows 平台 Win32 原语集中地。
//!
//! 监控套件（锁屏监听、Raw Input 键鼠采集、前台窗口事件）都依赖 Windows 专属 API，
//! 这些 `unsafe` 样板此前散落在各模块。这里把它们收敛到一处，给未来新增的
//! Win32 代码一个明确的归属，既避免 unsafe 样板继续扩散，也便于统一维护
//! 关键安全不变量。
//!
//! 关键不变量（违反会导致输入中断 / 事件丢失 / 窗口失败）：
//! - Raw Input 的 `WM_INPUT` 与 `SetWinEventHook` 的回调线程必须跑消息循环，
//!   否则收不到输入 / 事件消息；
//! - message-only 窗口类只需注册一次，重复注册会被系统拒绝（类名进程内唯一）；
//! - Raw Input 设备注册（`RIDEV_INPUTSINK`）须在窗口创建之后、消息循环之前完成，
//!   停用 / 退出时必须 `RIDEV_REMOVE` 注销，否则系统仍会把输入旁路投递到本窗口；
//! - 注册时**绝不加** `RIDEV_NOLEGACY`——那会禁用别的程序的 `WM_KEYDOWN`，等于劫持键盘。
//!
//! 注意：本项目监控仅支持 Windows，跨平台不在范围内。

use core::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent, WINEVENTPROC,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RAWKEYBOARD, RAWMOUSE, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD,
    RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HWND_MESSAGE, RegisterClassW,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW, WNDCLASS_STYLES, WNDPROC,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, MSG,
};

/// 在调用线程上运行标准 Windows 消息循环，直到收到 `WM_QUIT`。
///
/// Raw Input 窗口线程、前台窗口事件线程都依赖它——此前两处各自内联了同一段循环，
/// 现统一为单一实现，消除漂移风险。
///
/// # Safety
/// 调用线程必须是可以安全处理消息的线程；本函数只转发消息，不处理任何窗口过程。
pub unsafe fn run_message_loop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

/// message-only 窗口类是否已注册（进程内只注册一次）。
static RAW_CLASS_READY: AtomicBool = AtomicBool::new(false);

/// 注册（仅一次）一个 message-only 窗口类，随后每次「启用」都基于它创建窗口。
///
/// # Safety
/// `wndproc` 必须能在创建出的窗口的消息循环上安全调用。
unsafe fn ensure_raw_class(class_name: &[u16], wndproc: WNDPROC) {
    if RAW_CLASS_READY.swap(true, Ordering::SeqCst) {
        return;
    }
    let wc = WNDCLASSW {
        style: WNDCLASS_STYLES(0),
        lpfnWndProc: wndproc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: match GetModuleHandleW(None) {
            Ok(h) => h.into(),
            Err(_) => return,
        },
        hIcon: Default::default(),
        hCursor: Default::default(),
        hbrBackground: Default::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
    };
    // RegisterClassW 返回类原子（0 表示失败）。本进程类名唯一，仅「已注册」这一种
    // 失败需要忽略；真实失败会在后续 CreateWindowExW 处暴露。不区分 GetLastError。
    let _ = RegisterClassW(&wc);
}

/// 创建 message-only 窗口（注册类 + 建窗口）。窗口无可见界面，仅用于接收 `WM_INPUT`。
///
/// # Safety
/// `class_name` 必须是 null 结尾的 UTF-16；`wndproc` 必须安全。
pub unsafe fn create_message_window(class_name: &[u16], wndproc: WNDPROC) -> windows::core::Result<HWND> {
    ensure_raw_class(class_name, wndproc);
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR(class_name.as_ptr()),
        PCWSTR::null(),
        WINDOW_STYLE(0),
        0,
        0,
        0,
        0,
        Some(HWND_MESSAGE),
        None,
        Some(GetModuleHandleW(None)?.into()),
        None,
    )?;
    Ok(hwnd)
}

/// 销毁 message-only 窗口（停用 / 退出时调用，与 create_message_window 配对）。
///
/// # Safety
/// `hwnd` 必须是由 create_message_window 创建的合法窗口。
pub unsafe fn destroy_message_window(hwnd: HWND) -> windows::core::Result<()> {
    DestroyWindow(hwnd)
}

/// 注册 Raw Input 设备（键盘 + 鼠标），把输入旁路投递到 `hwnd`。
///
/// 用 `RIDEV_INPUTSINK`：即使本窗口不是前台窗口（托盘程序常如此）也能收到全局输入。
/// **绝不**加 `RIDEV_NOLEGACY`——那会禁用别的程序的 `WM_KEYDOWN`，等于劫持键盘。
///
/// # Safety
/// `hwnd` 必须有效；调用线程需运行消息循环以接收 `WM_INPUT`。
pub unsafe fn register_raw_input(hwnd: HWND) -> windows::core::Result<()> {
    let devices = [
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x06, // 键盘 (Generic Desktop / Keyboard)
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02, // 鼠标 (Generic Desktop / Mouse)
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
    ];
    RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32)
}

/// 注销 Raw Input 设备（停用 / 退出时调用，与 register_raw_input 配对）。
///
/// `RIDEV_REMOVE` 下 `hwndTarget` 被系统忽略，仍传 hwnd 无害。
///
/// # Safety
/// `hwnd` 必须有效。
pub unsafe fn unregister_raw_input(hwnd: HWND) -> windows::core::Result<()> {
    let devices = [
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x06,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: hwnd,
        },
    ];
    RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32)
}

/// 读取 `WM_INPUT` 的 `lParam` 携带的原始输入数据（HRAWINPUT）。
///
/// `GetRawInputData` 需要两趟：先传 `None` 取所需缓冲区大小，再分配后取数据。
/// 返回的字节缓冲可按 `RAWINPUT` 解释（`header.dwType` 分流键盘 / 鼠标）。
///
/// # Safety
/// `lparam` 必须来自一个真实的 `WM_INPUT` 消息。
pub unsafe fn read_raw_input(lparam: LPARAM) -> Option<Vec<u8>> {
    let hraw = HRAWINPUT(lparam.0 as *mut c_void);
    let mut cb = 0u32;
    // 第一趟：pdata=None，仅取大小（cbsizeheader 传 RAWINPUTHEADER 的大小）
    let _ = GetRawInputData(
        hraw,
        RID_INPUT,
        None,
        &mut cb,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if cb == 0 {
        return None;
    }
    // 按 cb 与完整 RAWINPUT 的较大者分配、零填充：
    // 键盘事件系统只填 40 字节（header 24 + RAWKEYBOARD 16），而按 RAWINPUT
    // （union 取鼠标分支，48 字节）解引用会读到尾部的 8 字节。多分配并置零可保证
    // 后续 `&*(buf.as_ptr() as *const RAWINPUT)` 无论哪种设备都在合法范围内。
    let mut buf = vec![0u8; (cb as usize).max(size_of::<RAWINPUT>())];
    // 第二趟：写入缓冲。返回 u32::MAX(-1) 表示失败。
    let ret = GetRawInputData(
        hraw,
        RID_INPUT,
        Some(buf.as_mut_ptr() as *mut c_void),
        &mut cb,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if ret == u32::MAX {
        return None;
    }
    // 注意：Windows 会把实际拷入的字节数写回 cb（键盘 40 / 鼠标 48），
    // 与 buf 长度（≥48）不一定相等。实际长度以 RAWINPUTHEADER.dwSize 为准，
    // 由 raw_input_len_ok 校验，不要拿 buf.len() 当数据长度用。
    Some(buf)
}

/// 校验 `WM_INPUT` 载荷长度是否够按 `dwType` 读取对应的设备结构体。
///
/// **这是个踩过的坑，务必保留**：`GetRawInputData` 返回的字节数随设备类型浮动——
/// 键盘 = header + `RAWKEYBOARD`（x64: 24+16=40），鼠标 = header + `RAWMOUSE`（24+24=48），
/// 而 `size_of::<RAWINPUT>()` 的 union 取最大分支、恒为 48。
/// 若拿 `size_of::<RAWINPUT>()` 当门槛，键盘事件（40 < 48）会被**全部静默丢弃**，
/// 表现为「鼠标统计正常、按键次数恒为 0」——注册是成功的，数据是被门槛吃掉的。
/// 因此必须按 dwType 分别比对，长度取自系统填好的 `RAWINPUTHEADER.dwSize`。
pub fn raw_input_len_ok(dw_type: u32, dw_size: u32) -> bool {
    let need = match dw_type {
        t if t == RIM_TYPEKEYBOARD.0 => size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>(),
        t if t == RIM_TYPEMOUSE.0 => size_of::<RAWINPUTHEADER>() + size_of::<RAWMOUSE>(),
        _ => return false, // HID 等未处理的设备类型：一律不读，避免按错结构体解释
    };
    (dw_size as usize) >= need
}

// ---------------------------------------------------------------------------
// 前台窗口事件钩子（SetWinEventHook）
// ---------------------------------------------------------------------------

/// `SetWinEventHook` 句柄的 RAII 守卫：Drop 时自动 `UnhookWinEvent`。
///
/// 以前调用方手工「安装 → 消息循环 → 卸载」三段配对，任何提前 return / panic
/// 都会漏掉卸载；守卫把配对交给类型系统——钩子生命周期 == 守卫生命周期。
pub struct WinEventHookGuard(HWINEVENTHOOK);

impl WinEventHookGuard {
    /// 安装前台窗口切换事件钩子（`EVENT_SYSTEM_FOREGROUND`）。
    ///
    /// 标志位固定为 `WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS`：
    /// - `OUTOFCONTEXT`：回调投递到安装线程（**该线程随后必须进入
    ///   `run_message_loop()`**，否则一个事件都收不到——见本文件顶部不变量）；
    /// - `SKIPOWNPROCESS`：不收自己进程的事件（本程序切自己的窗口不该记账）。
    ///
    /// 安装失败（系统返回无效句柄）返回 `None`。
    pub fn install_foreground_hook(callback: WINEVENTPROC) -> Option<Self> {
        // SAFETY：SetWinEventHook 本身线程安全；OUTOFCONTEXT 模式下回调跑在
        // 安装线程的消息循环上，「安装线程随后进入消息循环」是本函数的调用契约。
        let h = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                callback,
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if h.is_invalid() {
            None
        } else {
            Some(Self(h))
        }
    }
}

impl Drop for WinEventHookGuard {
    fn drop(&mut self) {
        // SAFETY：句柄来自 SetWinEventHook，且只在这里卸载一次。
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
    }
}

/// 当前前台窗口（安全封装）。无前台窗口 / 句柄无效时返回 `None`。
pub fn foreground_window() -> Option<HWND> {
    // SAFETY：纯查询，无副作用。
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        None
    } else {
        Some(fg)
    }
}

/// 窗口句柄 → 所属进程 PID。窗口无效 / 查询失败返回 `None`。
pub fn window_pid(hwnd: HWND) -> Option<u32> {
    // SAFETY：纯查询；pid 先初始化为 0，失败时保持 0，据此判定失败。
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

/// PID → 进程主模块的完整路径。进程已退出 / 权限不足返回 `None`。
///
/// 用 `PROCESS_QUERY_LIMITED_INFORMATION`（而非 `PROCESS_ALL_ACCESS`）：
/// 查询镜像路径不需要完整权限，这样对提权进程（如系统服务）也能拿到路径，
/// 不会因为拒绝访问而漏统计。
pub fn process_exe_path(pid: u32) -> Option<String> {
    // SAFETY：句柄在本函数内成对 CloseHandle，绝不外泄；缓冲区长度由 size 传入并
    // 以返回值写回，读取区间不会越界。
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(h);
        if ok.is_ok() && size > 0 && (size as usize) <= buf.len() {
            return Some(String::from_utf16_lossy(&buf[..size as usize]));
        }
    }
    None
}

/// 窗口 → `(exe 完整路径, 进程名小写)`。
///
/// 进程名用于「已知软件映射表」匹配与自身进程排除，故统一小写。
pub fn window_process_info(hwnd: HWND) -> Option<(String, String)> {
    let path = process_exe_path(window_pid(hwnd)?)?;
    let name = path.rsplit('\\').next().unwrap_or("").to_lowercase();
    Some((path, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    // 仅测试用到：正式代码里 HID 走 raw_input_len_ok 的 `_` 分支，故不占顶层 import
    use windows::Win32::UI::Input::RIM_TYPEHID;

    /// 核心不变量：键盘载荷严格小于完整 RAWINPUT，
    /// 故「用 size_of::<RAWINPUT>() 当门槛」在结构上就是错的（会丢光键盘事件）。
    #[test]
    fn keyboard_payload_is_shorter_than_rawinput() {
        let kb = size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>();
        assert!(
            kb < size_of::<RAWINPUT>(),
            "键盘载荷 {kb} 应小于 RAWINPUT {}，否则说明 union 布局变了、本文件的门槛也要重估",
            size_of::<RAWINPUT>()
        );
    }

    /// 进程路径解析：拿**自身进程**实测（不依赖任何全局状态或外部窗口）。
    /// 这是 app_usage（前台窗口）与 audio_usage（音频会话）共用的唯一实现，
    /// 若它退化成返回 None，两个模块的统计会一起静默清零。
    #[test]
    fn process_exe_path_of_self_is_absolute_exe() {
        let p = process_exe_path(std::process::id()).expect("自身进程路径必须能查到");
        assert!(p.to_lowercase().ends_with(".exe"), "应指向 exe: {p}");
        assert!(p.contains('\\'), "应是完整路径而非进程名: {p}");
    }

    /// 不存在的 PID：必须优雅返回 None（进程已退出是常态，不能 panic）
    #[test]
    fn process_exe_path_of_dead_pid_is_none() {
        assert!(process_exe_path(u32::MAX).is_none());
    }

    /// 空窗口句柄：PID 查询失败返回 None
    #[test]
    fn window_pid_of_invalid_window_is_none() {
        assert!(window_pid(HWND::default()).is_none());
        assert!(window_process_info(HWND::default()).is_none());
    }

    /// 每种设备类型：刚好够长则通过，短一个字节则拒绝，HID 一律拒绝。
    /// 用相对 size_of 表达，x64 / x86 都可跑。
    #[test]
    fn raw_input_len_ok_checks_per_device_type() {
        let kb = (size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>()) as u32;
        assert!(raw_input_len_ok(RIM_TYPEKEYBOARD.0, kb));
        assert!(raw_input_len_ok(RIM_TYPEKEYBOARD.0, kb + 8));
        assert!(!raw_input_len_ok(RIM_TYPEKEYBOARD.0, kb - 1));

        let ms = (size_of::<RAWINPUTHEADER>() + size_of::<RAWMOUSE>()) as u32;
        assert!(raw_input_len_ok(RIM_TYPEMOUSE.0, ms));
        assert!(!raw_input_len_ok(RIM_TYPEMOUSE.0, ms - 1));

        // 键盘的真实长度（40）拿去当鼠标载荷判：必须拒绝
        if kb < ms {
            assert!(!raw_input_len_ok(RIM_TYPEMOUSE.0, kb));
        }
        assert!(!raw_input_len_ok(RIM_TYPEHID.0, ms));
    }

    unsafe extern "system" fn noop_win_event_proc(
        _: HWINEVENTHOOK,
        _: u32,
        _: HWND,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
    ) {
    }

    /// RAII 配对回归：安装成功 → drop（内部 Unhook）→ 立即重装仍成功。
    /// 若 Drop 漏卸载或重复卸载，第二次安装大概率失败 / 行为未定义。
    #[test]
    fn foreground_hook_guard_installs_drops_and_reinstalls() {
        let guard = WinEventHookGuard::install_foreground_hook(Some(noop_win_event_proc));
        assert!(guard.is_some(), "SetWinEventHook 安装失败（OUTOFCONTEXT 钩子无需窗口）");
        drop(guard);
        assert!(
            WinEventHookGuard::install_foreground_hook(Some(noop_win_event_proc)).is_some(),
            "卸载后重装失败：WinEventHookGuard 的 Drop 没有正确 UnhookWinEvent"
        );
    }
}
