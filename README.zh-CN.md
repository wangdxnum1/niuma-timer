# 🐎 牛马计时器

> 实时看看今天赚了多少钱，精确到秒。
> [English](./README.md)

一个轻量级 Windows 托盘常驻工具，给打工人用。它蹲在系统托盘里，实时显示：

- 今天**已经赚了多少**（¥）
- 已经**上了多少小时班**（小时/分钟/秒）
- **距下班还有多久**
- **赚钱速率**（¥/分）和**距发薪日天数**

托盘图标直接画出已赚金额——瞄一眼任务栏，就能感受到每秒入账的快感。

## 功能特性

- **只统计当天**——零点自动重置，不累计前一天
- **托盘图标画金额**——`¥328` 直接渲染在图标上，不用开窗口
- **多行 tooltip 对齐排版**——用全角空格对齐标签列，数值起点严格对齐：

  ```
  今日已赚　¥328.50
  已工作　　5小时10分30秒
  距下班　　1小时50分30秒
  ────────
  赚钱速率　¥5.48/分
  距发薪　　23天
  ```

- **时长格式可配置**——`hms`（几小时几分几秒）/ `hm`（几小时几分）/ `h`（小数小时）
- **单例模式**——第二次启动会聚焦已有窗口，而不是开新进程
- **双击托盘显示主窗口**——双击托盘图标打开设置界面
- **节假日自动获取**——从 CDN 拉取国务院放假安排（含调休补班），本地缓存；断网时按周一到周五兜底
- **无运行时依赖**——release 产物自包含（系统只需 WebView2 Runtime，Win10/11 一般自带）

## 计算原理

```
时薪 = 月薪 ÷ 当月实际工作日 ÷ 每日工时

今日已赚 = 已工作小时 × 时薪
赚钱速率（¥/分）= 时薪 ÷ 60
```

午休自动排除（上午段 + 下午段分别配置）。下班即封顶，非工作日显示"今天休息"。

## 环境要求

- **Windows 10 / 11**
- **WebView2 Runtime**——通常已预装；没有的话到[微软官网](https://developer.microsoft.com/microsoft-edge/webview2/)下载
- 联网（仅首次拉取节假日数据需要；断网用本地缓存）

## 构建

### 前置条件

- [Rust 工具链](https://rustup.rs)（stable，≥ 1.77）
- 项目基于 **Tauri v2**（Rust 后端 + 纯 HTML/CSS/JS 前端，无需 npm 打包）

### 一键编译（推荐）

```bat
build.bat            :: 同时构建 debug 和 release
build.bat debug      :: 仅 debug
build.bat release    :: 仅 release
```

产物输出到：

- `bin\debug\niuma-timer.exe`
- `bin\release\niuma-timer.exe`

脚本会自动探测 `rustc` 的 host triple，定位 `target\<triple>\<flavor>\` 下的产物。

### 手动编译

```bash
cd src-tauri
cargo build --release
# 产物：src-tauri/target/<triple>/release/niuma-timer.exe
```

### 打包成安装包（.msi）

```bash
cargo install tauri-cli --version "^2"
cargo tauri build
```

## 使用方法

1. 运行 `niuma-timer.exe`——托盘出现图标（显示 `¥0`）。
2. **右键托盘 → 设置**：填写月薪、上午/下午上下班时间、发薪日，点「保存」。
3. 点「刷新工作日数据」拉取当年节假日，自动算出当月实际上班天数（也可手动覆盖）。
4. 托盘图标每秒刷新「今天已赚 ¥XX」。悬停查看完整对齐信息；**双击托盘图标**打开主窗口。

### 配置项

| 字段 | 说明 |
|---|---|
| `monthly_salary` | 月薪（元） |
| `am_start` / `am_end` | 上午上班/下班时间（如 `09:00` / `12:00`） |
| `pm_start` / `pm_end` | 下午上班/下班时间（如 `13:30` / `18:00`） |
| `payday` | 发薪日（每月几号，1–31） |
| `workdays_override` | 手动覆盖自动计算的上班天数（可选） |
| `duration_format` | 时长显示格式：`hms` / `hm` / `h`（默认 `hms`） |

## 数据存放

- 配置文件：`%APPDATA%\niuma-timer\config.json`
- 节假日缓存：`%APPDATA%\niuma-timer\holiday_{年份}.json`
- 节假日数据源：[`NateScarlet/holiday-cn`](https://github.com/NateScarlet/holiday-cn)（经 jsDelivr CDN 分发，国务院放假安排，含调休补班）

## 技术栈

| 层 | 技术 |
|---|---|
| 后端 | Rust + Tauri v2 |
| 前端 | 原生 HTML / CSS / JS（无打包工具） |
| 单例模式 | `tauri-plugin-single-instance` |
| 托盘图标 | 运行时像素绘制，内置 5×7 点阵字体 |
| 节假日数据 | GitHub 托管 JSON 经 jsDelivr CDN，本地缓存兜底 |

## 已知边界

- 无加班功能——下班即封顶，不继续累计。
- 非工作日显示"今天休息"，不累计。
- tooltip 是系统渲染的纯文本（无颜色/图标），对齐依赖全角空格（U+3000）列宽计算。

## 项目结构

```
niuma-timer/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs        # 应用入口、invoke 垫片、定时循环
│   │   ├── config.rs      # 配置结构体 + 读写
│   │   ├── calc.rs        # 赚钱计算 + 时长格式化 + tooltip
│   │   ├── holiday.rs     # 节假日数据拉取/解析/缓存
│   │   ├── icon_render.rs # 托盘图标像素绘制
│   │   └── tray.rs        # 托盘图标、菜单、双击处理
│   ├── Cargo.toml
│   ├── build.rs           # Tauri 构建（注册命令）
│   ├── tauri.conf.json
│   └── capabilities/default.json
├── frontend/
│   ├── index.html
│   ├── app.js
│   └── styles.css
├── build.bat              # 一键编译（debug + release → bin/）
└── bin/                   # 编译产物（已 gitignore）
```

---

用 🦀 做的，献给每一个牛马。
