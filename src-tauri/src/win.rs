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

use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CLASS_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetObjectW, SelectObject, BI_RGB,
    BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HDC, HGDIOBJ,
};
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::Accessibility::{
    HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent, WINEVENTPROC,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RAWKEYBOARD, RAWMOUSE, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD,
    RIM_TYPEMOUSE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetDoubleClickTime, GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyIcon, DestroyWindow, DispatchMessageW, DrawIconEx,
    EVENT_SYSTEM_FOREGROUND, GetCursorPos, GetForegroundWindow, GetIconInfo, GetMessageTime,
    GetMessageW, GetWindowThreadProcessId, HWND_MESSAGE, PostThreadMessageW, RegisterClassW,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW, WNDCLASS_STYLES, WNDPROC,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, DI_NORMAL, HICON, ICONINFO, MSG, RI_KEY_BREAK,
    ICON_BIG, ICON_SMALL, IMAGE_ICON, LoadImageW, LR_DEFAULTSIZE,
    MessageBoxW, MB_ICONERROR, MB_OK, SendMessageW, SM_CXICON, SM_CXSMICON, SM_CYICON,
    SM_CYSMICON, WM_SETICON, WM_QUIT,
};

/// 在调用线程上运行标准 Windows 消息循环，直到收到 `WM_QUIT`。
///
/// Raw Input 窗口线程、前台窗口事件线程都依赖它——此前两处各自内联了同一段循环，
/// 现统一为单一实现，消除漂移风险。
///
pub fn run_message_loop() {
    let mut msg = MSG::default();
    // SAFETY：本函数只转发消息（`GetMessageW` / `DispatchMessageW`），窗口过程由
    // 注册方提供。窗口过程里可能回跑到业务代码，但那是调用方自己的契约。
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        let _ = unsafe { TranslateMessage(&msg) };
        unsafe { DispatchMessageW(&msg) };
    }
}

/// message-only 窗口类是否已注册（进程内只注册一次）。
static RAW_CLASS_READY: AtomicBool = AtomicBool::new(false);

/// 注册（仅一次）一个 message-only 窗口类，随后每次「启用」都基于它创建窗口。
///
fn ensure_raw_class(class_name: &[u16], wndproc: WNDPROC) {
    if RAW_CLASS_READY.load(Ordering::SeqCst) {
        return;
    }
    let wc = WNDCLASSW {
        style: WNDCLASS_STYLES(0),
        lpfnWndProc: wndproc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: match unsafe { GetModuleHandleW(None) } {
            Ok(h) => h.into(),
            Err(e) => {
                crate::db::debug_log(&format!("[win] GetModuleHandleW 失败，Raw Input 窗口类未注册: {e}"));
                return;
            }
        },
        hIcon: Default::default(),
        hCursor: Default::default(),
        hbrBackground: Default::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
    };
    // RegisterClassW 返回类原子（0 表示失败）。类名已被本进程注册过算成功。
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        let err = unsafe { GetLastError() };
        if err != ERROR_CLASS_ALREADY_EXISTS {
            crate::db::debug_log(&format!("[win] RegisterClassW 失败: {err:?}"));
            return;
        }
    }
    RAW_CLASS_READY.store(true, Ordering::SeqCst);
}

/// message-only 窗口的 RAII 守卫：Drop 时自动注销 Raw Input 并销毁窗口。
///
/// 与 `WinEventHookGuard` 同一思路——此前调用方要手工「建窗口 → 注册设备 →
/// 消息循环 → 注销设备 → 销毁窗口」五段配对，任一提前 return / panic 都会漏掉
/// 后半段，后果是系统仍把输入旁路投递给一个本已「停用」的窗口。守卫把配对
/// 交给类型系统：窗口生命周期 == 守卫生命周期。
pub struct MessageWindow {
    hwnd: HWND,
    raw_registered: bool,
}

impl MessageWindow {
    /// 创建 message-only 窗口（注册类 + 建窗口）。窗口无可见界面，仅用于收消息。
    ///
    /// `class_name` 传普通 UTF-8 字符串，null 结尾由本函数保证——此前要求调用方
    /// 自己维护 null 结尾的 UTF-16，是个容易被忽视的越界读隐患。
    ///
    /// 注意：窗口类进程内只注册一次（`ensure_raw_class`），类名相同的第二次创建
    /// 会沿用首次注册的类（含其 wndproc）。
    pub fn create(class_name: &str, wndproc: WNDPROC) -> windows::core::Result<Self> {
        let wide: Vec<u16> = class_name.encode_utf16().chain(Some(0)).collect();
        ensure_raw_class(&wide, wndproc);
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(wide.as_ptr()),
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
            )
        }?;
        Ok(Self {
            hwnd,
            raw_registered: false,
        })
    }

    /// 注册 Raw Input 设备（键盘 + 鼠标），把输入旁路投递到本窗口。
    ///
    /// 用 `RIDEV_INPUTSINK`：即使本窗口不是前台窗口（托盘程序常如此）也能收到全局输入。
    /// **绝不**加 `RIDEV_NOLEGACY`——那会禁用别的程序的 `WM_KEYDOWN`，等于劫持键盘。
    pub fn register_raw_input(&mut self) -> windows::core::Result<()> {
        let devices = [
            RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x06, // 键盘 (Generic Desktop / Keyboard)
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: self.hwnd,
            },
            RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x02, // 鼠标 (Generic Desktop / Mouse)
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: self.hwnd,
            },
        ];
        unsafe { RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32) }?;
        self.raw_registered = true;
        Ok(())
    }

    /// 注销 Raw Input 设备。`RIDEV_REMOVE` 下 `hwndTarget` 被系统忽略，仍传 hwnd 无害。
    /// 只在实际注册过时才调用，避免未注册时的多余系统调用。
    pub fn unregister_raw_input(&mut self) {
        if !self.raw_registered {
            return;
        }
        self.raw_registered = false;
        let devices = [
            RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x06,
                dwFlags: RIDEV_REMOVE,
                hwndTarget: self.hwnd,
            },
            RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x02,
                dwFlags: RIDEV_REMOVE,
                hwndTarget: self.hwnd,
            },
        ];
        let _ = unsafe { RegisterRawInputDevices(&devices, size_of::<RAWINPUTDEVICE>() as u32) };
    }

    /// 在调用线程上跑消息循环，直到本线程收到 `WM_QUIT`。
    pub fn run_message_loop(&self) {
        run_message_loop()
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        self.unregister_raw_input();
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
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

/// `RAWMOUSE` 的业务子集：把 union 里真正用到的字段摊平成普通 Rust 数据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawMouse {
    /// `usFlags`：0=相对移动 1=绝对移动（触控板/笔）
    pub flags: u16,
    /// `usButtonFlags`：各按键的按下/抬起位，可同时出现
    pub button_flags: u16,
    /// `usButtonData`：滚轮增量（i16，符号表示方向，每格通常 120）
    pub button_data: i16,
    pub last_x: i32,
    pub last_y: i32,
}

/// `WM_INPUT` 载荷的安全解析结果。
///
/// `RAWINPUT.data` 是个 union——按错分支读就是 UB，且键盘/鼠标载荷长度不同
/// （见 `raw_input_len_ok` 的教训）。解析收口在这里，业务模块只见普通 Rust 数据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawEvent {
    /// 虚拟键码 + 是否为「抬起」
    Keyboard { vk: u16, up: bool },
    Mouse(RawMouse),
}

/// 把 `read_raw_input` 取回的字节缓冲解析成 `RawEvent`。
///
/// 三重防线：缓冲够放 header → `dwType` 对应的载荷长度达标（按 `dwSize`）→
/// 缓冲本身也够长。任一不满足就返回 None，**不碰 union**。
pub fn parse_raw_input(buf: &[u8]) -> Option<RawEvent> {
    if buf.len() < size_of::<RAWINPUTHEADER>() {
        return None;
    }
    let header = unsafe { &*(buf.as_ptr() as *const RAWINPUTHEADER) };
    if !raw_input_len_ok(header.dwType, header.dwSize) {
        return None;
    }
    // buf 实际长度同样要够（read_raw_input 恒给 ≥48；单测里可能给短缓冲）
    let need = match header.dwType {
        t if t == RIM_TYPEKEYBOARD.0 => size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>(),
        t if t == RIM_TYPEMOUSE.0 => size_of::<RAWINPUTHEADER>() + size_of::<RAWMOUSE>(),
        _ => return None,
    };
    if buf.len() < need {
        return None;
    }
    let raw = unsafe { &*(buf.as_ptr() as *const RAWINPUT) };
    match header.dwType {
        t if t == RIM_TYPEMOUSE.0 => {
            let m = unsafe { raw.data.mouse };
            Some(RawEvent::Mouse(RawMouse {
                flags: m.usFlags.0,
                button_flags: unsafe { m.Anonymous.Anonymous.usButtonFlags },
                button_data: unsafe { m.Anonymous.Anonymous.usButtonData } as i16,
                last_x: m.lLastX,
                last_y: m.lLastY,
            }))
        }
        t if t == RIM_TYPEKEYBOARD.0 => {
            let k = unsafe { raw.data.keyboard };
            Some(RawEvent::Keyboard {
                vk: k.VKey,
                up: k.Flags & RI_KEY_BREAK as u16 != 0,
            })
        }
        _ => None,
    }
}

/// 光标当前屏幕坐标。
pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt) }.ok().map(|()| (pt.x, pt.y))
}

/// 当前消息的时间戳（`GetMessageTime`），与 `GetTickCount` 同域的毫秒计数。
pub fn message_time() -> u32 {
    // 负值（出错）按 u32 解释，与旧实现一致
    (unsafe { GetMessageTime() }) as u32
}

/// 当前线程 ID（`GetCurrentThreadId`）。用于向特定线程投递 `WM_QUIT`。
pub fn current_thread_id() -> u32 {
    unsafe { GetCurrentThreadId() }
}

/// 系统双击时间阈值（毫秒）。
pub fn double_click_time() -> u32 {
    unsafe { GetDoubleClickTime() }
}

/// 距上次「真实输入」的空闲毫秒数。
///
/// 由内核为每个登录会话维护，覆盖面优于自造打点（含钩子看不到的部分全屏程序
/// 原始输入）。失败返回 None，由调用方决定降级。
pub fn idle_ms() -> Option<u64> {
    let mut lii = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut lii) }.as_bool() {
        // dwTime 与 GetTickCount 同为 u32 毫秒，约 49.7 天回绕；
        // wrapping_sub 保证跨回绕点仍算出正确差值
        Some(unsafe { GetTickCount() }.wrapping_sub(lii.dwTime) as u64)
    } else {
        None
    }
}

/// 给指定线程投递 `WM_QUIT` 唤醒其消息循环。
/// 失败（线程已退出 / 无消息队列）返回 false，可安全忽略。
pub fn post_thread_quit(tid: u32) -> bool {
    unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) }.is_ok()
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

// ---------------------------------------------------------------------------
// COM / Core Audio（媒体播放监控用）
// ---------------------------------------------------------------------------

/// 本线程 COM 初始化的 RAII 守卫。
///
/// 关键不变量：COM 必须「谁初始化谁反初始化」，而 `CoUninitialize` 会**连本线程的
/// COM 对象一起带走**。这正是音频峰值计必须跨整个采样周期持有、绝不能在枚举函数
/// 内部 init/uninit 成对调用的原因。守卫把生命周期绑到作用域，退出时自动收尾。
pub struct ComGuard {
    /// 私有字段：强制走 `init_multithreaded()` 构造，保证「构造成功 ⟺ 已初始化」
    _private: (),
}

impl ComGuard {
    /// 以多线程套间（MTA）初始化本线程 COM。
    ///
    /// 返回 `None` 表示初始化失败——典型是 `RPC_E_CHANGED_MODE`（本线程已被别人
    /// 以别的套间初始化）。此时**绝不能** `CoUninitialize`，那会拆掉别人的初始化。
    pub fn init_multithreaded() -> Option<Self> {
        // SAFETY：`pvReserved` 必须为 NULL；配对的反初始化由 Drop 保证，不会重漏。
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // S_OK / S_FALSE 都算成功（S_FALSE = 本线程此前已初始化过，仍需配对反初始化）。
        // 注意 HRESULT::ok() 得到的是 Result<()>，再 .ok() 才是 Option。
        hr.ok().ok().map(|()| Self { _private: () })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY：与构造时那次成功的 CoInitializeEx 严格一一配对。
        unsafe { CoUninitialize() }
    }
}

/// 一个音频会话的峰值计及其所属进程 PID。
pub struct AudioMeter {
    /// 会话所属进程 PID；`0` = 系统声音会话（没有对应的用户进程）
    pub pid: u32,
    meter: IAudioMeterInformation,
}

impl AudioMeter {
    /// 读取本会话当前峰值（0.0~1.0 归一化）；会话已消失（应用关闭）返回 `None`。
    ///
    /// 注意：返回的是**上一个设备周期**（典型 10ms 级）的峰值快照，不是自上次调用
    /// 以来的累积值（MSDN：peak value is recorded over one device period）。
    /// 因此调用方必须密采样，不能一个统计周期只读一次——否则提示音会被放大成整周期。
    pub fn peak(&self) -> Option<f32> {
        // SAFETY：meter 由本模块的 `audio_meters` 创建，接口指针在本结构体存活期内有效。
        unsafe { self.meter.GetPeakValue() }.ok()
    }
}

// ---------------------------------------------------------------------------
// exe 资源读取（版本信息 / 图标）
// ---------------------------------------------------------------------------

/// 图标尺寸上限（像素）。超过则放弃提取——托盘卡片只需要小图，
/// 大图纯属浪费内存与 PNG 编码时间。
const MAX_ICON_PX: u32 = 128;

/// 从 exe 提取出的图标原始像素（RGBA，自上而下）。
pub struct IconRgba {
    pub width: u32,
    pub height: u32,
    /// 长度恒为 `width * height * 4`，RGBA 顺序（已由 BGRA 转换而来）
    pub rgba: Vec<u8>,
}

/// GDI 图标句柄守卫：Drop 自动 `DestroyIcon`。
struct IconGuard(HICON);

impl Drop for IconGuard {
    fn drop(&mut self) {
        // SAFETY：句柄由本模块的 ExtractIconExW 产出，且只在这里释放一次。
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

/// GDI 对象（位图 / 画笔等）守卫：Drop 自动 `DeleteObject`。
struct ObjGuard(HGDIOBJ);

impl Drop for ObjGuard {
    fn drop(&mut self) {
        // SAFETY：句柄来自 GetIconInfo / CreateDIBSection，且只在这里释放一次。
        unsafe {
            let _ = DeleteObject(self.0);
        }
    }
}

/// 内存 DC 守卫：Drop 自动 `DeleteDC`（内存 DC 必须用 DeleteDC，不是 ReleaseDC）。
struct DcGuard(HDC);

impl Drop for DcGuard {
    fn drop(&mut self) {
        // SAFETY：DC 由本模块 CreateCompatibleDC 产出，且只在这里释放一次。
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// 提取 exe 的第一个图标 → RGBA 像素。无图标资源 / 失败返回 `None`。
///
/// 这段逻辑原本内联在 app_usage 里，每个失败分支都要手工重复
/// `DestroyIcon×2 + DeleteDC + DeleteObject` 四句清理——本次改为 RAII 守卫后，
/// 任何提前 return 都会自动收尾，GDI 对象泄漏不复存在。
pub fn extract_icon_rgba(exe_path: &str) -> Option<IconRgba> {
    // SAFETY：所有句柄均由本函数创建，并在此返回前经守卫释放（无论走哪个分支）；
    // 像素缓冲长度严格等于 32bpp DIBSection 的 w*h*4，读取不会越界。
    unsafe {
        let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hlarge = HICON::default();
        let mut hsmall = HICON::default();
        if ExtractIconExW(PCWSTR(wide.as_ptr()), 0, Some(&mut hlarge), Some(&mut hsmall), 1) == 0 {
            return None;
        }
        // 从这一行起，两个图标句柄交给守卫：后续任何提前 return 都会释放它们。
        // ExtractIconExW 可能只产出其中一个，另一个是无效句柄（DestroyIcon 对无效句柄是安全的空操作）。
        let _large = IconGuard(hlarge);
        let _small = IconGuard(hsmall);
        let icon = if !hlarge.is_invalid() { hlarge } else { hsmall };

        let mut ii = ICONINFO::default();
        if GetIconInfo(icon, &mut ii).is_err() {
            return None;
        }
        // GetIconInfo 产出的掩码/颜色位图必须 DeleteObject，否则每次调用泄漏两个 GDI 对象。
        let _mask = ObjGuard(ii.hbmMask.into());
        let _color = ObjGuard(ii.hbmColor.into());

        let mut bmp = BITMAP::default();
        let bmp_ok = !ii.hbmColor.is_invalid()
            && GetObjectW(
                ii.hbmColor.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bmp as *mut _ as *mut c_void),
            ) != 0;
        if !bmp_ok || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
            return None;
        }
        let (w, h) = (bmp.bmWidth as u32, bmp.bmHeight as u32);
        if w > MAX_ICON_PX || h > MAX_ICON_PX {
            return None;
        }

        let mem_dc = CreateCompatibleDC(None);
        if mem_dc.is_invalid() {
            return None;
        }
        let _dc = DcGuard(mem_dc);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                // 负值 = top-down（首行是图像顶部），省去一次上下翻转
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(hbmp) = CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            return None;
        };
        if hbmp.is_invalid() || bits.is_null() {
            return None;
        }
        let _bmp = ObjGuard(hbmp.into());
        let old = SelectObject(mem_dc, hbmp.into());
        let _ = DrawIconEx(mem_dc, 0, 0, icon, w as i32, h as i32, 0, None, DI_NORMAL);

        // 读 DIB 像素（32bpp = BGRA），交换 R/B 转成 PNG 需要的 RGBA
        let len = (w * h * 4) as usize;
        let mut rgba = vec![0u8; len];
        std::ptr::copy_nonoverlapping(bits as *const u8, rgba.as_mut_ptr(), len);
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        // 位图仍被 DC 选中时 DeleteObject 会失败（GDI 规则），必须先还原再让守卫释放
        let _ = SelectObject(mem_dc, old);

        Some(IconRgba {
            width: w,
            height: h,
            rgba,
        })
    }
}

/// 读 exe 版本信息里的 FileDescription（本地化产品名），失败返回 `None`。
pub fn file_description(exe_path: &str) -> Option<String> {
    // SAFETY：缓冲区长度来自 GetFileVersionInfoSizeW，VerQueryValueW 返回的指针与
    // 长度都指向该缓冲区内；以 null 为界扫描字符串，不会越界。
    unsafe {
        let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(PCWSTR(wide.as_ptr()), Some(0), size, buf.as_mut_ptr() as *mut c_void)
            .is_err()
        {
            return None;
        }
        // 取第一个语言/代码页块
        let mut tlen: u32 = 0;
        let mut tptr: *mut c_void = std::ptr::null_mut();
        let tkey: Vec<u16> = r"\VarFileInfo\Translation"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if !VerQueryValueW(
            buf.as_ptr() as *const c_void,
            PCWSTR(tkey.as_ptr()),
            &mut tptr,
            &mut tlen,
        )
        .as_bool()
            || tlen < 4
        {
            return None;
        }
        let lang = *(tptr as *const u16);
        let cp = *((tptr as *const u16).add(1));
        let key = format!(r"\StringFileInfo\{:04x}{:04x}\FileDescription", lang, cp);
        let mut vlen: u32 = 0;
        let mut vptr: *mut c_void = std::ptr::null_mut();
        let vkey: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        if !VerQueryValueW(
            buf.as_ptr() as *const c_void,
            PCWSTR(vkey.as_ptr()),
            &mut vptr,
            &mut vlen,
        )
        .as_bool()
            || vlen == 0
        {
            return None;
        }
        // 注意：VerQueryValueW 的 vlen 是 u16 字符数（含结尾 null），不是字节数。
        // 不能除以 2，否则长名称会被截断一半（如 "Microsoft Edge" → "Microso"）。
        // 以 null 为界扫描最稳妥（兼容 vlen 两种单位语义），杜绝截断与越界。
        let p = vptr as *const u16;
        let max = vlen as usize;
        let mut n = 0usize;
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        let s = s.trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

/// 枚举默认渲染端点（扬声器 / 耳机）上的全部音频会话。
///
/// 返回值刻意区分两种「空」：
/// - `None` = Core Audio 不可用（无渲染设备 / COM 异常 / 权限问题）——**监控失效**；
/// - `Some(v)` 且 `v` 为空 = 枚举成功，只是此刻确实没有会话——**监控正常**。
///
/// 调用方（audio_usage）据此设置 `WATCH_OK`：两者对用户的含义完全不同。
///
/// 调用线程需已初始化 COM（见 [`ComGuard`]）：返回的峰值计要跨整个采样周期持有。
pub fn audio_meters() -> Option<Vec<AudioMeter>> {
    // SAFETY：整条调用链只使用本模块自己创建并持有的 COM 接口指针；
    // `CoCreateInstance` 的 CLSID 与接口类型、`Activate` 的接口类型均成对匹配；
    // 返回的 AudioMeter 持有各自接口的强引用，不会悬垂。
    unsafe {
        let enumerator =
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .ok()?;
        let mgr = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None).ok()?;
        let sessions = mgr.GetSessionEnumerator().ok()?;
        let count = sessions.GetCount().ok()?;

        let mut out = Vec::new();
        for i in 0..count {
            // 单个会话失败（应用刚好退出）不影响其余会话，跳过即可
            let Ok(session) = sessions.GetSession(i) else {
                continue;
            };
            let Ok(meter) = session.cast::<IAudioMeterInformation>() else {
                continue;
            };
            let Ok(ctrl2) = session.cast::<IAudioSessionControl2>() else {
                continue;
            };
            let Ok(pid) = ctrl2.GetProcessId() else {
                continue;
            };
            out.push(AudioMeter { pid, meter });
        }
        Some(out)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    // 仅测试用到：正式代码里 HID 走 raw_input_len_ok 的 `_` 分支，故不占顶层 import
    use windows::Win32::UI::Input::RIM_TYPEHID;
    use windows::Win32::UI::WindowsAndMessaging::RI_MOUSE_LEFT_BUTTON_DOWN;

    /// 把一块 `RAWINPUT` 按「系统实际填了多少字节」切成字节缓冲。
    /// 键盘事件只有 40 字节（header+RAWKEYBOARD），比 RAWINPUT 的 48 短——
    /// 正是历史上「按键次数恒为 0」的根因，测试必须按真实长度切。
    fn as_bytes(raw: &RAWINPUT, len: usize) -> Vec<u8> {
        unsafe { std::slice::from_raw_parts(raw as *const RAWINPUT as *const u8, len) }.to_vec()
    }

    #[test]
    fn parse_raw_input_accepts_short_keyboard_payload() {
        let mut raw: RAWINPUT = unsafe { std::mem::zeroed() };
        raw.header.dwType = RIM_TYPEKEYBOARD.0;
        raw.header.dwSize = (size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>()) as u32;
        unsafe { raw.data.keyboard = RAWKEYBOARD { VKey: 65, ..std::mem::zeroed() } };
        let bytes = as_bytes(&raw, size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>());
        assert_eq!(
            parse_raw_input(&bytes),
            Some(RawEvent::Keyboard { vk: 65, up: false }),
            "键盘载荷比 RAWINPUT 短，也必须能解析出来"
        );
    }

    #[test]
    fn parse_raw_input_marks_key_break_as_up() {
        let mut raw: RAWINPUT = unsafe { std::mem::zeroed() };
        raw.header.dwType = RIM_TYPEKEYBOARD.0;
        raw.header.dwSize = (size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>()) as u32;
        unsafe {
            raw.data.keyboard = RAWKEYBOARD {
                VKey: 27,
                Flags: RI_KEY_BREAK as u16,
                ..std::mem::zeroed()
            }
        };
        let bytes = as_bytes(&raw, size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>());
        assert_eq!(
            parse_raw_input(&bytes),
            Some(RawEvent::Keyboard { vk: 27, up: true })
        );
    }

    /// union 的鼠标分支：字段必须原样带出，业务侧不碰 union 也能拿到全部数据
    #[test]
    fn parse_raw_input_reads_mouse_union() {
        let mut raw: RAWINPUT = unsafe { std::mem::zeroed() };
        raw.header.dwType = RIM_TYPEMOUSE.0;
        raw.header.dwSize = (size_of::<RAWINPUTHEADER>() + size_of::<RAWMOUSE>()) as u32;
        unsafe {
            let m = &mut raw.data.mouse;
            m.usFlags.0 = 0; // MOVE_RELATIVE
            m.lLastX = 3;
            m.lLastY = -4;
            m.Anonymous.Anonymous.usButtonFlags = RI_MOUSE_LEFT_BUTTON_DOWN as u16;
            m.Anonymous.Anonymous.usButtonData = 120;
        }
        let bytes = as_bytes(&raw, size_of::<RAWINPUT>());
        match parse_raw_input(&bytes) {
            Some(RawEvent::Mouse(m)) => {
                assert_eq!((m.last_x, m.last_y), (3, -4));
                assert_eq!(m.button_flags, RI_MOUSE_LEFT_BUTTON_DOWN as u16);
                assert_eq!(m.button_data, 120);
            }
            other => panic!("期望 Mouse，实际 {other:?}"),
        }
    }

    /// 缓冲被截断时必须返回 None，绝不去读 union
    #[test]
    fn parse_raw_input_rejects_truncated_buffer() {
        let mut raw: RAWINPUT = unsafe { std::mem::zeroed() };
        raw.header.dwType = RIM_TYPEMOUSE.0;
        raw.header.dwSize = (size_of::<RAWINPUTHEADER>() + size_of::<RAWMOUSE>()) as u32;
        let bytes = as_bytes(&raw, size_of::<RAWINPUTHEADER>() + 4);
        assert_eq!(parse_raw_input(&bytes), None);
        // 连 header 都不够
        assert_eq!(parse_raw_input(&[0u8; 4]), None);
    }

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

    /// COM 守卫：新线程上初始化必须成功（否则媒体播放监控会直接不可用）。
    #[test]
    fn com_guard_initializes_on_fresh_thread() {
        // 测试框架为每个用例单独开线程，故这里是「全新线程」的语义。
        assert!(
            ComGuard::init_multithreaded().is_some(),
            "新线程上 COM 初始化应成功"
        );
    }

    /// COM 守卫：析构（CoUninitialize）后应能重新初始化——计数必须真正归零。
    /// 若守卫漏掉反初始化，本用例第二次初始化仍能成功但计数泄漏，
    /// 若守卫多反初始化一次，则会拆掉尚在使用的 COM。
    #[test]
    fn com_guard_can_reinit_after_drop() {
        let first = ComGuard::init_multithreaded();
        assert!(first.is_some());
        drop(first);
        assert!(
            ComGuard::init_multithreaded().is_some(),
            "反初始化后应能重新初始化"
        );
    }

    /// Core Audio 调用链冒烟测试：整条 COM 链路（CoCreateInstance → 默认端点 →
    /// 会话枚举）在**没有渲染设备**的机器上也必须优雅返回 `None`，绝不能 panic。
    /// 有设备时返回 `Some`，两种结果都合法，故只断言「不崩溃」。
    #[test]
    fn audio_meters_never_panics() {
        let _com = ComGuard::init_multithreaded().expect("COM 初始化失败");
        let _ = audio_meters();
    }

    /// 版本信息读取：系统自带的 notepad.exe 一定有 FileDescription。
    /// 这是「应用显示名」的主要来源，读不出来就全靠进程名兜底，用户会看到裸 exe 名。
    #[test]
    fn file_description_of_system_exe_is_nonempty() {
        let fd = file_description(r"C:\Windows\System32\notepad.exe");
        assert!(
            fd.as_deref().is_some_and(|s| !s.is_empty()),
            "notepad.exe 应能读到 FileDescription"
        );
    }

    /// 不存在的路径：必须返回 None，且不能因为提前 return 漏掉任何资源。
    #[test]
    fn file_description_of_missing_file_is_none() {
        assert!(file_description(r"C:\nonexistent\no-such-app.exe").is_none());
    }

    /// 图标提取：不存在的路径返回 None（验证失败路径的守卫不会误释放无效句柄）
    #[test]
    fn extract_icon_rgba_of_missing_file_is_none() {
        assert!(extract_icon_rgba(r"C:\nonexistent\no-such-app.exe").is_none());
    }

    /// 图标提取：拿自身 exe 实测，结果的尺寸与缓冲区长度必须自洽。
    /// 无图标资源的构建返回 None 也合法，故只约束「有结果时一定自洽」。
    #[test]
    fn extract_icon_rgba_of_self_is_self_consistent() {
        let path = std::env::current_exe()
            .expect("无法获取自身路径")
            .to_string_lossy()
            .to_string();
        if let Some(icon) = extract_icon_rgba(&path) {
            assert!(icon.width > 0 && icon.height > 0, "图标尺寸必须为正");
            assert!(
                icon.width <= MAX_ICON_PX && icon.height <= MAX_ICON_PX,
                "图标尺寸不得超过上限 {MAX_ICON_PX}: {}x{}",
                icon.width,
                icon.height
            );
            assert_eq!(
                icon.rgba.len(),
                (icon.width * icon.height * 4) as usize,
                "RGBA 缓冲区长度必须等于 w*h*4"
            );
        }
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

/// 用 exe 内嵌的多尺寸 ico 资源（tauri-build 固定 ID 32512）按窗口实际 DPI
/// 分别加载 ICON_BIG / ICON_SMALL 并 WM_SETICON 覆盖，消除任务栏高 DPI 下图标发糊。
///
/// 底层 tao 默认只挂一张固定 16px 位图，任务栏在高 DPI 下放大必然发糊；
/// 托盘图标是运行时 SDF 动态绘制、资源管理器读的是完整多尺寸 ico，所以那两处清晰。
///
/// SAFETY：内部 FFI 调用（LoadImageW / SendMessageW / GetDpiForWindow 等）均为只读式
/// 系统调用，句柄经 WM_SETICON 后由窗口接管，无需手动释放；本函数对调用方暴露为 safe。
#[cfg(windows)]
pub fn set_window_icons_from_resource(hwnd: HWND) {
    // SAFETY：以下均为只读式系统调用；句柄经 WM_SETICON 后由窗口接管，无需手动释放。
    let hmod = match unsafe { GetModuleHandleW(None) } {
        Ok(h) => h,
        Err(_) => return,
    };
    let hinst = HINSTANCE(hmod.0);
    // MAKEINTRESOURCEW(32512)
    let name = PCWSTR(32512usize as *const u16);
    let mut dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        dpi = 96;
    }

    let set_icon = |wparam: u32, cx: _, cy: _| {
        let (cx, cy) = (
            unsafe { GetSystemMetricsForDpi(cx, dpi) }.max(1),
            unsafe { GetSystemMetricsForDpi(cy, dpi) }.max(1),
        );
        if let Ok(h) = unsafe { LoadImageW(Some(hinst), name, IMAGE_ICON, cx, cy, LR_DEFAULTSIZE) } {
            // SAFETY：WM_SETICON 把图标句柄交给窗口，由窗口负责后续生命周期。
            unsafe {
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(wparam as usize)),
                    Some(LPARAM(h.0 as isize)),
                );
            }
        }
    };
    set_icon(ICON_BIG, SM_CXICON, SM_CYICON);
    set_icon(ICON_SMALL, SM_CXSMICON, SM_CYSMICON);
}

/// 弹一个模态错误对话框，用于构建期 / 启动期致命错误，把「双击无反应」转成可操作的提示。
///
/// SAFETY：MessageBoxW 是只读式模态对话框，无资源需释放，对调用方暴露为 safe。
#[cfg(windows)]
pub fn message_box(title: &str, msg: &str) {
    let wide_msg: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(wide_msg.as_ptr()),
            PCWSTR(wide_title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

