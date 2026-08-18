# 🐎 牛马计时器 (NiuMa Timer)

Windows 托盘常驻工具，实时显示**今天赚了多少钱、上了多少小时班、距下班还有多久**。

- 只统计**当天**，零点自动重置
- 配置：月薪 + 上午/下午四个时间点（午休不计时）+ 发薪日
- 时薪 = 月薪 ÷ 当月实际上班天数 ÷ 每日工时
- 托盘图标直接画出「今天已赚 ¥XX」，悬停看完整信息
- 右键菜单：设置 / 刷新工作日 / 退出
- 爽感三件套：实时赚钱速率（¥/分）、距发薪日倒计时、托盘图标金额

## 环境要求

- Windows 10/11
- Rust stable（>= 1.77）：<https://rustup.rs>
- 系统已安装 **WebView2 Runtime**（Win10/11 通常自带；没有则到微软官网下载）
- 联网（首次拉取节假日数据；断网会用本地缓存）

## 构建

一键编译并导出产物到 `bin/` 目录（区分 debug / release）：

```bat
build.bat            # 同时构建 debug 和 release
build.bat debug     # 仅 debug
build.bat release   # 仅 release
```

- debug 产物：`bin/debug/niuma-timer.exe`
- release 产物：`bin/release/niuma-timer.exe`

手动编译（cargo 默认按 host triple 输出到 `target/<triple>/<flavor>/`）：

```bash
cd src-tauri
cargo build --release
# 产物：src-tauri/target/x86_64-pc-windows-msvc/release/niuma-timer.exe
#       （不同机器 triple 可能不同，用 build.bat 会自动定位）
```

如需打包成安装包（.msi）：

```bash
cargo install tauri-cli
cargo tauri build
```

> 直接 `cargo build --release` 也能用，前端页面会被嵌入到 exe 中。

## 使用

1. 运行 `niuma-timer.exe`，托盘出现图标（显示 ¥0）。
2. **右键托盘 → 设置**：填写月薪、上下班时间、发薪日，点「保存」。
3. 点「刷新工作日数据」拉取当年节假日，自动算出当月实际上班天数（也可手动覆盖）。
4. 之后托盘图标每秒刷新「今天已赚 ¥XX」，悬停查看：已工作 / 距下班 / ¥每分 / 距发薪天数。

## 数据存放

- 配置：`%APPDATA%\niuma-timer\config.json`
- 节假日缓存：`%APPDATA%\niuma-timer\holiday_{年份}.json`
- 节假日数据源：[timor.tech 节假日 API](https://timor.tech/api/holiday)（每年国务院放假安排，含调休补班）

## 已知边界

- 加班功能本期未做（下班即封顶）。
- 非工作日显示「今天休息」，不累计。
- 首次启动若没网，会用「当月自然工作日（周一到周五）」兜底，联网后刷新更准。
