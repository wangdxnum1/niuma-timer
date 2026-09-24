//! 手动暂停监控（v1.3.0「牛马守护」）
//!
//! 全局「暂停」状态机：托盘 / 全局快捷键 / 前端徽章共用。暂停只有一种来源：
//! 手动暂停（[`MANUAL`]）——托盘菜单或 Alt+Shift+P 切换，无限期直到再次恢复。
//!
//! 暂停语义：三类监控（键鼠 / 应用 / 音频）立即停止记账——已入账时长不回滚，
//! 未入账增量不累计（各守卫点直接丢弃，见 activity / app_usage / audio_usage
//! 的 `is_paused` 门控）。恢复瞬间不补记暂停期间的任何时长——「暂停」就是钱先冻结。

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

/// 手动暂停开关（托盘 / 快捷键切换）
static MANUAL: AtomicBool = AtomicBool::new(false);

/// 切换手动暂停（托盘 / 快捷键共用）。
pub fn set_manual(v: bool) {
    MANUAL.store(v, Ordering::SeqCst);
}

/// 当前是否处于暂停（手动）
pub fn is_paused() -> bool {
    MANUAL.load(Ordering::Relaxed)
}

/// 暂停状态视图（pause_monitor 命令返回体）
#[derive(Debug, Clone, Serialize)]
pub struct PauseView {
    pub paused: bool,
}

pub fn view() -> PauseView {
    PauseView {
        paused: is_paused(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 状态机全序列。单用例串行：模块级 static 在 cargo test 并行下共享，
    /// 拆成多个用例会互相污染，必须按顺序走完整条状态链。
    #[test]
    fn pause_state_machine() {
        // 复位到干净起点（前序用例可能留下状态）
        set_manual(false);
        assert!(!is_paused());

        // 手动暂停 / 恢复
        set_manual(true);
        assert!(is_paused(), "手动暂停后应视为暂停");
        set_manual(false);
        assert!(!is_paused());

        // 复位，避免污染其他模块的测试
        set_manual(false);
    }
}
