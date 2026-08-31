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
//! - 钩子 / 事件句柄必须在退出前卸载（见各模块的 `Unhook*`），否则系统范围内残留。
//!
//! 注意：本项目监控仅支持 Windows，跨平台不在范围内。

use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};

/// 在调用线程上运行标准 Windows 消息循环，直到收到 `WM_QUIT`。
///
/// 低级钩子线程、前台窗口事件线程、隐藏窗口消息线程都依赖它——
/// 此前三处各自内联了同一段循环，现统一为单一实现，消除漂移风险。
///
/// # Safety
/// 调用线程必须是可以安全处理消息的线程；本函数只转发消息，不处理任何窗口
/// 过程，也不安装 / 卸载任何钩子。调用方仍需自行保证钩子句柄的生命周期。
pub unsafe fn run_message_loop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}
