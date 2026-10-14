# 托盘悬停堆损坏修复

## 问题与原因

2026-10-14 07:59:41，1.10.0 的 niuma-timer.exe 崩溃。CDB 与本机 mcp-windbg-rs 分析 `niuma-timer.exe.49068.dmp`，匹配 release PDB 后确认调用链为悬停工作线程 → pointer_sample → tray_rect → TrayIcon Drop → Rc::drop_slow → RtlFreeHeap。

异常码 0xC0000374，堆诊断为 HEAP_FAILURE_BLOCK_NOT_BUSY：释放操作触及空闲块。Tauri 托盘对象内部为 Rc<RefCell<...>>；后台调用 tray_by_id/rect 时克隆并释放句柄，rect 内部还会克隆句柄投递主线程。跨线程 Rc 生命周期竞争与重复释放证据高度吻合。小型转储无法还原首次释放或完整竞争次序，不能声称所有其他堆损坏来源都已排除。

## 修复方案

- 悬停位置查询：主线程闭包内获取托盘句柄、查询矩形、换算坐标并释放句柄；只向后台传普通坐标结果。
- tooltip 更新：后台先计算普通字符串，再投递主线程完成句柄获取、更新和释放。
- 后台查询等待最多 500ms，调度失败、通道关闭或超时返回未知位置；晚到的主线程任务仍在主线程完成，不会将托盘资源带回后台。现有悬停状态机保持其未知采样处理方式。
- 未清理配置、统计数据库或用户崩溃记录；没有自动替换正在运行的应用。

## 验证方法

新增线程生命周期测试使用真实任务通道和独立 UI 模拟线程，在查询闭包内创建、克隆和释放非 Send 的 Rc 资源，确认释放线程与结果传递。另测调度失败不执行查询，以及超时后主线程仍能安全完成任务。

执行项目标准 `build.bat test`，覆盖 Rust 单测、fmt、clippy、依赖审核、前端回归与发布脚本测试；执行 `build.bat release` 生成修复程序。结果以本次日志 `logs/tray-thread-fix-gate.log` 与 `logs/tray-thread-fix-release.log` 为准。

本次完整门禁退出码 0：266 项 Rust 测试、38 个前端脚本及 29/11/4 项 Python 测试通过，fmt、clippy、在线依赖审核通过。Release 构建退出码 0，产物为 `bin/release/niuma-timer.exe`。

独立审查确认匹配版本的 Tauri runtime 在主线程上直接执行 `run_on_main_thread` 任务，因此 UI 闭包内 `rect()` 的内部调度与接收同步完成，不会等待自己。两处 `tray_by_id` 的获取、克隆和释放均留在 UI 线程，审查未发现阻断问题。

人工验收：完全退出旧程序后启动新程序，反复悬停/离开托盘、右键菜单、双击主界面、切换彩色卡片与原生 tooltip、切换不同 DPI 显示器；保持运行超过本次约 37 小时，检查 Windows Application Error 1000 与新转储。需要进一步证明首次非法释放时，可在独立测试环境结合 PageHeap/Application Verifier。自动化结果不代表已完成长时人工验收。
