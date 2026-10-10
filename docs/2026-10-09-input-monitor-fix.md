# 键盘和鼠标活动无法记录：原因与修复

## 问题与证据

用户反馈键盘敲击、鼠标点击、移动距离均停止增长。本机 `monitor_activity` 配置为 true，但 2026-10-09 运行日志记录：

```text
[activity] Raw Input 窗口创建失败: 找不到窗口类。 (0x8007057F)
```

相同错误在旧日志的 1.8.0 至 1.10.0 构建中已存在。错误发生在 Raw Input 设备注册之前，活动采集线程退出，因此键鼠事件都无法进入计数器。

## 根因与修复方案

### 不同监控窗口共用了一个注册标记

`win::MessageWindow` 同时服务锁屏监控（`NiumaLockMonitor`）和键鼠采集（`NiumaRawInputWnd`），但原实现用一个 `RAW_CLASS_READY` 标记表示所有窗口类已注册。任意一个类先注册后，另一个类的注册被跳过；随后创建第二种窗口时 Windows 报找不到窗口类。

修复移除跨类共享的标记，每次创建前按实际类名调用 `RegisterClassW`。同名已注册接受 `ERROR_CLASS_ALREADY_EXISTS`，其他注册错误直接返回给调用方，不继续创建未注册的窗口。

### 活动监控注销设备时参数不合法

原实现设置 `RIDEV_REMOVE` 时仍传目标窗口 HWND，且忽略 API 失败并把内存状态清成已注销。Windows 要求此时 HWND 必须为 NULL，因此实际设备注册未被移除。

修复传入空 HWND，仅在注销成功后清除注册状态，失败记日志。该修复保证关闭活动监控时确实注销设备，再开启时可正常注册。

参考：[Windows 窗口类创建要求](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw)、[Raw Input 注册与注销要求](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerrawinputdevices)。

## 自动验证

新增真实 Win32 测试，不使用模拟 Windows API，不启动应用，不注入键鼠操作：

- `message_window_registers_each_class_and_can_recreate`：在同一测试进程创建两个不同类的隐藏窗口，并销毁后按逆序重建。修复前失败，错误码与用户日志同为 `0x8007057F`。
- `raw_input_unregisters_devices_and_can_register_again`：实际注册键盘与鼠标，通过 `GetRegisteredRawInputDevices` 检查目标 HWND，注销后检查设备已移除，重复两次注册/注销。修复前失败于设备仍在注册列表。
- 修复后 `win::tests` 21 项通过；完整 `build.bat test` 退出码 0，Rust 263 项、前端 38 个脚本、Python 29+11+4 项全部通过，fmt、Clippy 和在线依赖审计通过。
- `build.bat all` 退出码 0，已生成 `bin/debug/niuma-timer.exe` 与 `bin/release/niuma-timer.exe`。独立代码审查未发现重要或阻塞问题。

原来的按键去重、鼠标计数、暂停及锁屏过滤、数据库持久化规则保持不变。本次没有清空或修改用户配置、数据库或历史统计。

## 实机验收

1. 从托盘完全退出旧进程，再启动修复版；确认活动监控已开启且未暂停。
2. 打开今日键鼠活动页，敲击若干按键、左右点击并移动鼠标；等待页面刷新，确认三类计数持续增加。
3. 关闭活动监控，观察计数停止；重新开启后重复输入，确认恢复增加。
4. 锁屏期间不应计数，解锁后应继续增加；重启应用后已落盘的今日统计应保留。

自动验证覆盖真实窗口和设备生命周期；尚未替换当前运行的应用或完成上述实机页面验收。先前窗口未创建期间的键鼠事件没有被采集，无法通过本次修复补回。
