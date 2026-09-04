# 牛马计时器（Niuma Timer）项目记忆

## 定位
Windows 托盘常驻工具（Tauri v2 + Rust 后端 + 原生 HTML/CSS/JS 前端，无前端打包链）。
核心：实时显示今日已赚 ¥、已工作时长、距下班、¥/分速率、距发薪日。

## 技术栈
- 后端：Rust stable + Tauri v2（tray-icon / custom-protocol feature）
- 持久化：SQLite（rusqlite bundled，WAL，库文件 `%APPDATA%/niuma-timer/niuma.db`）；config.json / holiday 缓存仍为 JSON
- Windows 专属：windows crate 0.61（锁屏监听 WTS、音频会话轮询、窗口图标按 DPI 设置、Raw Input 键鼠采集；**低级钩子 WH_*_LL 已于 2026-08 废弃**，详见下）
- 单例：tauri-plugin-single-instance
- 前端：单 index.html + app.js + styles.css，6 个视图（主界面/设置/加班明细/活动明细/应用使用/媒体播放），自带 splash 防白闪；通过注入 INVOKE_SHIM 暴露 `window.__TAURI__` 省去 @tauri-apps/api

## 模块（src-tauri/src/，14 个）
main / config / calc / holiday / tray / icon_render / db / overtime / activity / app_usage / audio_usage / lock_monitor
（README 仅描述前 6 个，已落后于实现）

## 已实现（超出 2026-08-18 设计文档）
- 工资计时 + 爽感三件套（动态图标文字、¥/分、距发薪）
- 加班追踪：监听锁屏（WTS_SESSION_LOCK）→ 按下班离开时刻计算加班费/饭补，SQLite 落盘，明细页可增删改
- 鼠标键盘活动监控（**Raw Input / WM_INPUT**，2026-08 起替换掉全局低级钩子以根治输入法延迟，逐小时桶 + 按键 Top 榜）
- 应用使用时长监控（前台窗口钩子 + 白名单，微信等）
- 媒体播放监控（音频会话峰值轮询）
- 托盘悬停彩色卡片（可开关，含防抖/看门狗/点击冷却）

## 重要约定 / 注意点
- README.md / README.zh-CN.md 已于 2026-08-31 重写对齐现状（14 模块、加班/监控套件）；后续改功能仍需同步 README
- 前端改动已能自动重编：build.rs 用 frontend 目录内容指纹注入 `cargo:rustc-env=TAURI_FRONTEND_FP`，内容一变即重编 crate 并重嵌；main.rs 末尾原 `[frontend-v3]` 注释已于 2026-08-31 删除（过时、会误导人手工 bump），改 frontend/ 无需任何手工操作
- 根目录调试脚本（activate_wechat.py / diag_*.py / sim_input.py / logs/ 等）已于 2026-08-31 清理删除，不再存在于仓库
- 构建产物在 bin/debug、bin/release（由 build.bat 产出）
- **改 frontend/ 后必须手工 bump 四处缓存戳**，否则 WebView2 继续吃旧缓存、改动看不见（build.rs 的指纹只保证 cargo 重嵌资源，管不了 WebView2 运行期缓存）：`index.html` 的 `styles.css?v=N`、`index.html` 的 `app.js?v=N`、`src-tauri/tauri.conf.json` 的 `index.html?v=N`、`app.js` 里的 `FE_VER`（同时兼作日志版本标记，排查旧缓存第一证据）
- **db 无迁移逻辑（2026-09-04 整体拆除，提交 12bca8c）**：程序未发布、无旧库旧 JSON，`db.rs` 只有 `init_tables()` 幂等建表（`conn()` 首次调用触发），**没有**版本化迁移 / user_version / JSON 导入 / normalize_app_names。schema 变更直接改 DDL，开发期删 `%APPDATA%/niuma-timer/niuma.db` 重建即可。历史教训：版本化迁移曾在全新库上「duplicate column name: source」首跑 panic（dev 机库已是最新版所以从不触发），不要再加回来
- 部署诊断：`main.rs` 有 panic hook + setup 阶段追踪写 `%APPDATA%/niuma-timer/panic.log`，build 失败弹 MessageBox（show_fatal）；发布 exe 已静态链接 CRT（`.cargo/config.toml` 的 `+crt-static`），唯一外部依赖是 WebView2
- 涉及 DB 写入逻辑的单测用 `Connection::open_in_memory()` + `db::CREATE_OT_RECORDS` 等单表 DDL 常量建表，**不要碰真实库** `%APPDATA%/niuma-timer/niuma.db`
