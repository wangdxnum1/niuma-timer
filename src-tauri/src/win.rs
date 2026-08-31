//! Windows 平台 Win32 原语集中地。
//!
//! 监控套件（锁屏监听、全局键鼠钩子、前台窗口事件）都依赖 Windows 专属 API，
//! 这些 `unsafe` 样板此前散落在各模块。这里把它们收敛到一处，给未来新增的
//! Win32 代码一个明确的归属，既避免 unsafe 样板继续扩散，也便于统一维护
//! 关键安全不变量。
//!
//! 关键不变量（违反会导致输入中断 / 钩子失效 / 事件丢失）：
//! - 低级钩子（`WH_MOUSE_LL` / `WH_KEYBOARD_LL`）与 `SetWinEventHook` 的回调线程
//!   必须跑消息循环，否则收不到钩子 / 事件消息；
//! - 钩子句柄必须在退出前卸载，否则系统范围内残留——`LowLevelHook` RAII 守卫
//!   已统一保证「安装即持句柄、作用域结束自动 `Unhook`」，调用方无需手写卸载分支。
//!
//! 注意：本项目监控仅支持 Windows，跨平台不在范围内。

use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, HHOOK, HOOKPROC, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, WINDOWS_HOOK_ID, MSG,
};

/// 在调用线程上运行标准 Windows 消息循环，直到收到 `WM_QUIT`。
///
/// 低级钩子线程、前台窗口事件线程、隐藏窗口消息线程都依赖它——
/// 此前三处各自内联了同一段循环，现统一为单一实现，消除漂移风险。
///
/// # Safety
/// 调用线程必须是可以安全处理消息的线程；本函数只转发消息，不处理任何窗口
/// 过程。钩子句柄的生命周期由 `LowLevelHook` 守卫负责，调用方只需保证守卫
/// 在消息循环运行期间保持存活（见 `activity` 模块用法）。
pub unsafe fn run_message_loop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

/// 低级钩子句柄的 RAII 守卫。
///
/// 安装成功后持有 `HHOOK`，`Drop` 时自动调用 `UnhookWindowsHookEx`，
/// 保证钩子在线程 / 函数退出时一定被卸载，不会在系统范围内残留。
///
/// 这正是手工 `match` 卸载时最容易漏掉的分支：例如「装鼠标钩子成功、
/// 装键盘钩子失败」的错误分支，若忘记卸载已成功的鼠标钩子，就会留下
/// 一个永远不消失的全局钩子。用守卫后该分支由编译器自动兜底——
/// 提前 `return` 时，`mouse` 守卫离开作用域即触发卸载。
///
/// # Safety
/// `install` 的 `proc` 必须是能在钩子线程上安全调用的回调；调用线程需运行
/// 消息循环（见 `run_message_loop`）。守卫本身不要求 `unsafe` 上下文使用，
/// 仅 `install` 是 `unsafe` 的。
pub struct LowLevelHook {
    hook: Option<HHOOK>,
    label: &'static str,
}

impl LowLevelHook {
    /// 安装一个低级钩子；失败返回 `Err`（此时不会持有任何句柄，无泄漏）。
    ///
    /// # Safety
    /// `proc` 必须是能在钩子线程上安全调用的回调；调用线程需运行消息循环。
    pub unsafe fn install(
        id: WINDOWS_HOOK_ID,
        proc: HOOKPROC,
        label: &'static str,
    ) -> windows::core::Result<Self> {
        let hook = SetWindowsHookExW(id, proc, None, 0)?;
        Ok(Self {
            hook: Some(hook),
            label,
        })
    }

    /// 手动卸载（Drop 前提前释放时用）。重复调用安全。
    pub fn uninstall(&mut self) {
        if let Some(h) = self.hook.take() {
            // UnhookWindowsHookEx 是 unsafe fn：此处安全，因为 h 是 install 成功返回的
            // 有效句柄，且 take() 后本守卫不再持有它，绝不会重复卸载。
            if let Err(e) = unsafe { UnhookWindowsHookEx(h) } {
                eprintln!("[win] 卸载钩子 {} 失败: {e}", self.label);
            }
        }
    }
}

impl Drop for LowLevelHook {
    fn drop(&mut self) {
        self.uninstall();
    }
}
