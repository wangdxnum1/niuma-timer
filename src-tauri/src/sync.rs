//! 锁访问收口。
//!
//! 全项目的 `Mutex` 一律通过 [`lock`] 获取，禁止再写裸的 `.lock().unwrap()`。
//!
//! 原因：常驻托盘程序里，任何一个线程在持锁期间 panic，这把锁就会永久中毒，
//! 此后**每个**碰它的线程都会跟着 panic——一个模块的小错会连锁拖垮整个托盘，
//! 而崩溃现场往往离真正的根因很远，极难排查。
//!
//! 本项目的临界区都只是读写状态（配置副本、当日累计、哈希表），不会留下
//! 半截结构，因此中毒后取回数据继续使用是安全的。真正需要担心的原始 panic
//! 已被记录到 `panic.log` / `debug.log`，自愈只是不让故障扩散。

use std::sync::{Mutex, MutexGuard};

/// 获取互斥锁。锁已中毒时记日志并自愈（取回其中的数据继续用）。
///
/// `who` 用于定位是哪把锁中毒，直接写模块与锁名，如 `"app_usage::CUR"`。
pub fn lock<'a, T>(m: &'a Mutex<T>, who: &str) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            crate::db::debug_log(&format!(
                "[lock] {who} 检测到锁中毒，已自愈（此前有线程在持锁期间 panic）"
            ));
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::lock;
    use std::panic;
    use std::sync::Mutex;

    /// 回归：持锁期间 panic 导致锁中毒后，自愈能取回数据且值仍是 panic 前写入的。
    /// （这条用例同时证明本项目所有 `lock()` 调用都不会因中毒而连锁 panic。）
    #[test]
    fn lock_recovers_from_poison() {
        let m = Mutex::new(1i32);
        let r = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let mut g = m.lock().unwrap();
            *g = 2;
            panic!("故意在持锁期间 panic");
        }));
        assert!(r.is_err(), "前面的闭包应当 panic");
        assert!(m.is_poisoned(), "持锁期间 panic 后锁应处于中毒状态");
        // 自愈：不 panic，且数据完好（临界区只做了赋值，不会留下半截结构）
        assert_eq!(*lock(&m, "test"), 2);
    }

    /// 正常路径：不中毒时不产生任何额外行为
    #[test]
    fn lock_works_when_clean() {
        let m = Mutex::new("ok".to_string());
        assert_eq!(*lock(&m, "test"), "ok");
        assert!(!m.is_poisoned());
    }
}
