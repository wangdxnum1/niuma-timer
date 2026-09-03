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
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS,
    RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE, RegisterClassW,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW, WNDCLASS_STYLES, WNDPROC, MSG,
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
            Ok(h) => h,
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
        Some(GetModuleHandleW(None)?),
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
    let mut buf = vec![0u8; cb as usize];
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
    Some(buf)
}
