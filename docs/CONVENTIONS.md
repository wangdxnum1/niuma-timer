# 工程约定（Conventions）

本文件是牛马计时器**工程约定的跟踪版真相源**：改了行为就同步这里，评审、CI 与 AI 会话都以它为准。
`.workbuddy/memory/MEMORY.md` 是**本机草稿本**（已被 `.gitignore` 忽略），里面的数字随时过期，**不要当参考**。
每条都写了「为什么」；有事故背景的附一句当时怎么翻的车。路径相对仓库根，`src/` 指 `src-tauri/src/`。

## 代码地图（现状）
- 后端 33 个 `.rs`：核心域模块 + 命令层 `cmds_core/cmds_bill/cmds_monitor/cmds_storage/cmds_update/cmds_debug`（二期自 `main.rs` 切出，纯移动）+ `tray/hover_state.rs`。
- 前端无打包链：`frontend/index.html` 里 **8 个视图容器**（`viewMain/viewBill/viewSettings/viewOt/viewAct/viewApp/viewAudio/viewUpdate`）+ `frontend/js/` **10 个块**（`app.js` 已删除，禁止复活）+ `styles.css` + `hover_card.html`（独立窗口）。
- 测试规模：Rust 侧 241 个 `#[test]`；`scripts/test_*.js` 35 个 + `scripts/test_publish_release.py` + `scripts/test_push_via_api.py`。
- 用户数据在 `%APPDATA%/niuma-timer/`：`niuma.db`（SQLite + WAL）、`config.json`、`holiday_{year}.json`、`debug.log` / `panic.log`、`icons/`。

## 构建入口与脚本纪律
- `build.bat [debug|release|all|package|test]` 是唯一构建入口；`release.bat [版本] [/y]` 一键发版，第 0 步就是 `call build.bat test`；`dev.bat` 起静态服务器 + `cargo tauri dev` 做前端热迭代（Rust 只编一次）。
- 共享初始化全在 `common.bat`：`ROOT/SRC/BIN`、cargo 兜底、`vcvars64`。它**故意不写 `setlocal`**（变量必须活过 `call`）；三个调用方都必须 `call "%~dp0common.bat"` 后立刻 `if errorlevel 1 exit /b 1`。
- **四个根 .bat 必须纯 ASCII、注释写英文、绝不加 `chcp`**。事故两次：① `chcp 65001` 让 cc-rs / embed_resource 误读 vswhere 的 GBK 输出，release 构建报 `RC.EXE not set`（2026-10-02）；② 中文 `rem` 在部分终端被 cmd 错位解析，注释片段当命令执行（`'报' is not recognized`）。另：**不要加 UTF-8 BOM**（cmd 会让 `@echo off` 失效）。
- 验收方式：逐字节扫描确认无 >0x7F 字节（现在四个文件都是 0）。
- `build.bat test` 必跑四个套件：`cargo test --quiet` → `node scripts/run_all.js` → `python scripts/test_publish_release.py` → `python scripts/test_push_via_api.py`。CI 的四个质量 job（`frontend-tests` / `release-engine-tests` / `dep-audit`（cargo-deny advisories） / `rust-checks`（fmt / clippy `-D warnings` / test / 环境契约 / drift guard））统一定义在 **reusable workflow `.github/workflows/tests.yml`**，`ci.yml`（push/PR）与 `release.yml`（发布，`needs: tests` 作前置门禁）都经 `uses: ./.github/workflows/tests.yml` 调用——**六步检查只此一份**，改门禁只改 tests.yml；两 workflow 的 rust-cache 用同一 `shared-key`（默认按 job 隔离会让发布 job 永远全冷构建）。`test_cloud_publish_requires_main_and_ci_gates` 钉住这套结构。
- ci.yml 的 `paths-ignore` 只允许 `docs/**`：根目录 `**.md` 不可忽略——test_readme.js 守的 README/CHANGELOG 恰是被改对象，忽略了守卫就只在下次碰代码的提交上补跑。
- **没有任何入口跑的测试一定会烂掉**：发布引擎套件就是这样在 v1.5.1 上云后静默失效的，直到 commit 8951e22 才接回 `build.bat test` 与 CI。
- `scripts/test_build_env.js` 把 `build.bat` / `common.bat` 复制到系统临时目录运行；cargo 桩生成的假 exe 只能留在该隔离目录，禁止写入真实 `src-tauri/target`。测试 runner 设置 `VSCMD_SKIP_SENDTELEMETRY=1`：否则 VS 开发命令行会启动后台 `vctip`，持有临时目录导致 Windows CI 清理时报 `EBUSY`。
- CI 的 drift guard 会因 `git status --porcelain -- frontend src-tauri` 非空而失败：`build.rs` 自动回写缓存戳与 capability，所以本地改完前端必须 `cargo build` 一次并把自动改动一并提交。

## 构建环境陷阱
- **TMP/TEMP 由构建自己持有**：`common.bat` 创建并钉住 `%ROOT%.tmp`（`.gitignore` 已忽略）。只要开着调试信息（`-Z7`，cc-rs 对 debug/release **都**传），cl.exe 会把命令行「debug record」写进 `%TMP%`；TMP 缺失/不可写/未设置时 cl 在跑 c1.dll 之前就以 **D8050（exit 2）** 退出，于是 ring / libsqlite3-sys / vswhom-sys 同时失败，看起来像工具链坏了。**别把这里的 TMP 改回继承调用方**。
- cargo 必须直连 rsproxy：`common.bat` 设 `CARGO_NET_RETRY/CARGO_HTTP_TIMEOUT`，但**不要**给 cargo 挂全局代理（Clash 的全局 SOCKS5 会掐断 cargo TLS 握手）。
- cargo 找不到时先看 `common.bat` 的兜底：`CARGO_BIN` 未定义 + `where cargo` 失败时会临时把 `%USERPROFILE%\.cargo\bin` 塞进 PATH（老终端看不到新装的 Rust）。要指定具体 cargo，先 `set "CARGO_BIN=..."`。

## 发版门禁（`release.bat`）
- 发版走 CI：push tag `vX.Y.Z` 触发 `.github/workflows/release.yml` 重建并发布。`release.bat` 只做版本同步 → 打包 → tag → push；`scripts/publish_release.py` 是 CI 不可用时的应急本地发布路径，`scripts/push_via_api.py` 是 github.com 被墙时的应急推送。
- 发布流水线在构建前必须确认正式发布使用版本匹配的 tag、tag 提交位于 `origin/main`，并重跑 fmt / clippy / Rust 测试 / 前端测试 / 发布引擎测试 / cargo-deny；手动分支运行只允许 `dry_run`。
- Windows 发布 runner 必须设置 `PYTHONIOENCODING=utf-8`：默认重定向输出是 `cp1252`，发布引擎测试里的中文诊断会报 `UnicodeEncodeError`，即使业务逻辑本身正确。
- 已存在的 `vX.Y.Z` tag 只在指向当前 HEAD 时复用；指向其他提交必须报错并改用新版本。禁止删除或强推已经使用的发布 tag。
- 版本号必须同时落 `src-tauri/Cargo.toml` 与 `src-tauri/tauri.conf.json`，且**回读两个文件校验**同步成功。
- 硬门禁（缺一个就中止）：`TAURI_SIGNING_PRIVATE_KEY` 未设不打包——少了它只会静默发出未签名包，用户装更新时才发现被拒；发布前校验 `bin/package` 里每个安装包**文件名带目标版本**（否则是从旧版本漏进来的陈旧产物）、存在 `*.exe.sig`（没有它就没有 `latest.json`，这个版本永远收不到更新）、存在 `*.pdb.zip` 或 `*.pdb`（没有 PDB 的版本，线上崩溃永远无法符号化）。
- 所有模式都拒绝在非 `main` 分支发版（否则 tag 指向不在 main 上的提交，云端一定拒绝）；push 用 `-c credential.helper=wincred -c http.sslBackend=openssl` 并按 `127.0.0.1:7890` 是否监听决定走代理还是**显式清空**代理（全局 gitconfig 里的残留 socks5 会在代理关闭时静默接管连接）。

## 沙箱 / agent 会话陷阱（本机 DSH 文件沙箱以低完整性运行 shell）
- **git 凭据助手不可用**：Git for Windows 一律经 `sh -c` 调助手，而沙箱禁止 MSYS signal pipe（Win32 error 5）。改用代理 + `-c http.sslBackend=openssl`（Schannel 会以 `SEC_E_NO_CREDENTIALS` 失败）；一次性 push 可把凭据写进 URL。
- **Python 别用 `tempfile.TemporaryDirectory()` / `mkdtemp()`**：它们以 0700 建目录，受限令牌写不进去，里面每个文件都 EACCES、连清理都被拒。用 `os.makedirs`（继承父目录宽松 ACL），见 `scripts/test_publish_release.py` 的 `tmpdir()`。
- **HKCU 不可写**：写注册表的测试必须先探测再跳过，否则沙箱里 `cargo test` 恒红，真回归被噪声淹没。范式见 `src/win.rs` 的 `hkcu_writable()` + `register_aumid_writes_display_name_and_icon`。

## Rust 代码约定
- **取锁一律 `sync::lock(&m, "模块::锁名")`**（如 `"app_usage::CUR"`，AppState 用 `"state.config"`），锁中毒时记日志并 `into_inner()` 自愈。禁裸 `.lock().unwrap()`（仅 `#[cfg(test)]` 与 `sync.rs` 自身的自愈回归用例例外）：常驻托盘里一个线程持锁 panic 会让每个碰它的线程跟着 panic，而崩溃现场离真正的根因极远。
- **SQLite 一律 `db::with_db`**（统一处理锁中毒 + 失败写 `debug.log` + 返回 `Result` 供调用方降级）。`main.rs` 里 `db::conn();` 的空调用是唯一例外——它负责启动建库。
- **schema 变更走 `db::ensure_column` 幂等补列**（先查 `pragma_table_info` 再 `ALTER`）。不能无条件 ALTER（全新库报 duplicate column name，历史上就是这么翻的车），更不能删库重建：v1.0.0 起已有真实用户库。待补列清单在 `db.rs` 的 `EXTRA_COLUMNS`。
- **新增周期任务挂 `scheduler.rs`**（1s 一拍：托盘刷新 + focus 状态机；5 拍：跨天 + 锁屏加班；10 拍：活动落盘 + 应用结算；60 拍：按「日期变了」跑每日维护 + 提醒）。不要自己 spawn sleep 循环。
- **不可合并进调度器的线程**：`activity::raw_thread` / `activity::keyq_worker`（键盘队列排空）/ `app_usage::watch_thread` / `lock_monitor`（都要跑 Win32 消息循环）、`audio_usage::tick_loop`（COM 亲和）、`tray` 悬停卡 actor（自带超时驱动）、`update::spawn_update_checker`（阻塞网络 IO，套 catch_unwind）。调度器每个任务都套 `catch_unwind`，单任务 panic 只丢一拍。
- **Win32 代码一律放 `win.rs` 并用 RAII 守卫**（`MessageWindow` / `WinEventHookGuard` / `ComGuard` / `IconGuard` / `ObjGuard` / `DcGuard`），不手工配对释放；其它模块只该在回调签名上留 unsafe。
- **键鼠统计用 Raw Input，不要回到低级钩子**（`WH_MOUSE_LL/WH_KEYBOARD_LL` 是「系统拦截」模型，每次按键先跨线程派发到钩子再投递给目标程序，会拖慢输入法；Raw Input 是旁路投递，热路径只做原子 `+1` / 入队，开关监控 = 真正注销设备）。
- windows-rs 0.61 的几个易错点：`GetRawInputDeviceInfoW` 的首参是 `Option<HANDLE>`（要 `Some(hdevice)`）；`HANDLE` 包的是裸指针（`HANDLE(p as *mut c_void)`）而非 isize；`RAWINPUTHEADER` 里的 `hDevice` 在 x64 下位于偏移 8（`win.rs::raw_input_hdevice` 手工按字节取，别改成按字段读）。
- **慢命令必须标 `#[tauri::command(async)]`**：同步命令默认在**主线程**执行，dbstat 全库扫描、WAL TRUNCATE + 清数据这类活会冻结托盘 1s 刷新与窗口事件（见 `cmds_storage.rs` 注释）。现状：`cmds_bill/cmds_monitor/cmds_storage/cmds_update` 的重活都是 async。
- **`main()` 最开头必须先分流更新助手**：`update::parse_helper_args`（`--apply-update`，命中即跑完文件替换并退出；带旗标但参数不全时 `exit 2`）与紧随其后的 `update::resume_after_helper()`，都**早于建库、建窗、装插件**。助手模式复用同一个 exe 在被替换前完成自我更新，把任何初始化搬到它前面都可能让更新卡死。
- **`[profile.release]` 不得加 `panic = "abort"`**：scheduler 的 `catch_unwind` 自愈依赖 unwind 语义，abort 会让「单任务 panic 只丢一拍」的隔离失效，一个任务崩掉整个托盘。
- Rust 侧 `emit` 要 `use tauri::Emitter`（`scheduler.rs` / `update.rs` / `tray.rs`）。
- **新增 Tauri 命令**：写 `#[tauri::command]` → 登记进 `main.rs` 的 `generate_handler!` → 前端 `invoke`。**不要手工配权限**：`build.rs` 从 `src/**/*.rs` 解析命令名单，自动补 `capabilities/default.json` 的 `allow-*` 并生成 `permissions/autogenerated/*.toml`。事故：手工名单漏登记时编译照过、**调度器内部调用也正常**（内部调用不走 IPC、不受 ACL 限制），只有前端点按钮报 `not allowed by ACL`。`scripts/test_commands.js` 守命令↔capabilities↔handler 三方一致（当前 38 命令 / 46 权限项）。
- `src-tauri/permissions/autogenerated/` 与 `src-tauri/gen/` **已 gitignore**（tauri-build 每次重生成，跟踪只制造行尾噪音）。tauri-build 只会**新增**权限文件，删掉的命令会留下永远的墓碑——处理办法是删掉 `permissions/autogenerated/` 整个目录重新构建（build.rs 按现存命令精确重生成）。

## 跨端字段契约（最容易静默失效的一类）
- **命令参数名：Rust snake_case → JS 传 camelCase**（`known_icons` ← `knownIcons`，见 `frontend/js/monitor.js`；Tauri 只转这一层）。
- **嵌套 serde 结构与所有返回结构体都是 snake_case**，Tauri 不转驼峰：`ManualOvertimeInput.cross_midnight`、`StorageInfo.total_bytes`。写错不报错，只取到 `undefined`——存储卡片「共占用 0 B、占比全 0%」就是这么活到发布后的。
- `maintain.rs::storage_info_json_keys_match_field_names` 用序列化键名把上面这条钉死；改结构体字段名就必须同步 `frontend/js/` 的读取点。
- **前端显隐一律 `classList` 切 `hidden` 类**，`frontend/styles.css` 必须保留通用 `.hidden { display: none !important; }`——自带 `display:flex` 的容器（`.bill-dash` / `.bill-empty`）会盖过普通规则。事故（2026-09-14）：账单三面板都写了 `hidden` 却没规则，三块同时渲染。`scripts/test_hidden.js` 守这条契约。
- **导出必须给明确反馈**：统一走后端 `export_csv` / `export_image` 落盘到「下载」目录（`safe_download_path` 只取 basename 并剔除 `/\:*?"<>|`，CSV 由后端加 BOM），前端 `showToast` + 并发守卫（`exportingCsv`），别让用户以为没反应而反复点刷出一堆同名文件。
- 前端靠 `main.rs` 的 `INVOKE_SHIM` 注入 `window.__TAURI__`，**没有 `@tauri-apps/api`、没有打包链**；新增前端能力优先复用垫片已暴露的 invoke / window / event。

## 数据与业务约定
- **WAL 必须显式 `PRAGMA wal_checkpoint(TRUNCATE)` 才收缩**（`maintain.rs::checkpoint_wal`）：自动 checkpoint 只复用空间；托盘常驻使「最后一个连接关闭」永不发生，实测 WAL 停在 4.15MB 而主库只有 245KB。
- `run_daily` 顺序是**收缩 → 删数据 → 再收缩**：DELETE 自己会写 WAL，只在开头收缩的话用户点「立即整理」看不到效果。
- `retention_days` 默认 **0 = 永久保留**，唯一落点是 `cutoff_for`（有单测）。清理表清单是 `DATED_TABLES`（当前 8 张，含 `focus_sessions`）——新增按日期分桶的表要同时改这里。
- **新增业务表必须登记两处**：`db.rs` 的 `TABLE_DDL`（建表）与 `db::BUSINESS_TABLES`（名单真相源，备份完整性校验 `backup::stage_restore` 与保留期清理都从它取表集合）。「名单 ↔ 实际建表」「DATED_TABLES ↔ BUSINESS_TABLES」两条等价性测试让漏登记立刻红——backup 手抄清单漏登 focus_sessions 的既成事实就是这么来的。
- **加班归属日由锁屏时刻自己决定**（`resolve_overtime_day`：早于 06:00 归前一天且 +24h）。`calc_record_auto` 故意**不接收日期参数**：调用方传「检测时刻的日期」会把凌晨锁屏记到第二天，通宵加班直接丢失。
- `DayKind` 三档（补班日算 Workday）；休息日/节假日用独立起算 `weekend_ot_start`（默认 `DEFAULT_REST_OT_START = "09:00"`，**不能沿用 pm_end**，否则上午来下午走的人一分钱算不到）与独立费率（法定节假日 → 休息日 → 工作日逐级回退）。
- **远程会话判定不能用「进程是否存在」**：第三方远程软件常驻（开着 UU 没连）会被误判成远程，自动加班被永久关掉。现判定 = `remote::is_remote_active` = `SM_REMOTESESSION` ∪ 最近 `REMOTE_WINDOW`(60s) 内有远程虚拟设备输入，接入点 `main.rs` 的 `overtime_exclude_remote` 闸门与 `focus.rs`。
- 远程设备特征串只放 `RDP*/SUNLOGIN/TODESK/UU`，**不要加裸 `VIRTUAL`/`MIRROR`**——物理设备也会命中，误伤反噬（`remote.rs` 有对应单测）。
- 备份快照用 `VACUUM INTO`（库开着 WAL，直接 `fs::copy` 会静默丢掉还在 `-wal` 里的最近写入）。还原是**两段式**：命令只铺 `*.pending` 文件 + 重启，真正换库发生在下一个进程的最开头（`main.rs` setup 里的 `backup::apply_pending_on_startup()`，必须先于任何 `db::conn()`）——持着永不释放的 WAL 连接原地覆盖 `niuma.db`，会新旧页缓存混用，属最难排查的一类损坏。
- 活动统计落盘是**增量语义**：只写脏的小时桶与脏键码，脏标记**事务提交成功后才清**（`activity.rs::save_day`），失败保留脏标记下轮重试；禁止 `let _ =` 吞错误——半截数据会重复累加。
- 存储细分用 dbstat 虚拟表精确统计，不可用时按行数估算并把 `approx` 标真；`build_storage_info` 是**纯函数**（可直接单测）；`other = db_bytes - 表总和` 必须 `saturating_sub`，否则估算路径会下溢成天文数字。
- 节假日内置兜底表在 `holiday.rs` 的 `BUILTIN`（当前 2025 / 2026）。**每年 11 月国务院公布次年安排后要补这一年**；缺表时工作日数退化成自然周几计数（2026-10 实测 18 天被算成 22 天，费率偏低约 18%），用户看到的钱是错的。
- 历史数据只允许补录，**写操作只挡未来日期**；主界面卡片永远只反映今天（历史走 SQLite，今天走内存含实时增量）。

## 前端约定
- 前端是 10 个块按**加载序**拼装（`scripts/lib/fe_sources.js` 的 `CHUNKS`，`boot.js` 必须最后），`frontend/app.js` 已删除且禁止复活；`FE_VER` 只在 `frontend/js/core.js` 声明一次。`scripts/test_frontend_split.js` 守这四条。
- **缓存戳全自动**：`build.rs` 用 frontend 内容指纹回写 `TARGETS` = `frontend/index.html`（1 处 CSS + 10 处 JS）、`frontend/js/core.js`（`FE_VER`）、`src-tauri/tauri.conf.json`、`src/tray.rs`（悬停卡 URL），8 位 hex。
- 哈希前必须先归一化剔除 `?v=` 与 `\r`，否则「回写版本号 → 内容变 → 指纹变 → 再回写」会死循环、每次编译都重编整个 crate；回写**必须在 `tauri_build::try_build()` 之前**，晚一步本次嵌入 exe 的还是旧资源、要编两次才生效。
- **悬停卡窗口尺寸两侧必须同步**：`src/tray.rs` 的 `HOVER_CARD_H = 352` / `HOVER_CARD_W = 380` == `frontend/hover_card.html` 的 `body{height/width}`（body 自带上下各 8px padding，卡片实际高 336）。`scripts/test_hover_card.js` 守这条 + 内容余量下限（窗口 ≥344）。
- **flex 列容器里别让「唯一可压缩的子项」吸收内容溢出**：`.card` 是 flex 列容器，`.hero` 是唯一带 `overflow:hidden` 的子项，按 Flexbox 规范其自动最小尺寸退化为 0，内容一超高就把 34px 大字金额压到 ~1px（2026-09-14 二次返工查出的真根因，比颜色问题更底层）。已加 `.hero{flex:0 0 auto}`，不要删。
- 透明 WebView（`.transparent(true)`）上的金额用纯 `color` 实心金，别用 `background-clip:text` + 透明填充裁剪渐变文字；悬停卡「窗口不消失 / 不显示」两个 bug 同源（过度依赖 `cursor_position()` 裁决），现两侧都不靠它 + 20s 硬上限。
- 前端测试**日期与时刻都要固定**（`FakeDate` + `static now()`，见 `scripts/test_tagline.js`）：`dynamicTagline` 读 `getHours()`，不固定时刻时 12:00–13:00 跑测试会让午休用例全误判。
- 顶层 `eval("function f(){}")` 与同名 `const` 冲突，须包进函数；**扫描代码模式一律用跨行正则**（或先 `\s+` 归一），按行 grep 会漏 rustfmt/prettier 拆行的写法（收口锁时实测漏 5 处）。

## 测试约定
- `scripts/run_all.js` 逐个 `spawnSync` 独立进程并首错即停——测试会改全局 `Date` / `$`，共进程必然互相污染。
- **测实现，别测复刻**：碰全局资源的函数要拆出吃参数的 `*_conn` 内层函数（`purge_before_conn` / `checkpoint_wal_conn` / `table_usage_conn`）或用纯函数（`build_storage_info`）直测。反例：把同样的 SQL 抄一遍跑一遍，实现改错测试照样绿。
- **前端桩数据不许手写字段名**：必须与 Rust 结构体字段逐字比对，并反向扫描「JS 读的每个字段在结构体里存在吗」（见 `scripts/test_storage.js`）。否则桩数据跟着写错也能全绿。
- **Rust 源一律经 `scripts/lib/rs_sources.js` 聚合读取**：它**递归**扫 `src-tauri/src/**/*.rs`（只扫顶层时 `tray/hover_state.rs` 对其断言完全隐形，那 19 条测试白写）。presence 类断言在聚合文本上语义不变；**「代码在哪个文件」「同文件内先后顺序」必须用 `rsRead("main.rs")`** 单文件读（参照 `scripts/test_commands.js` 的 handler 注册断言）。
- 文档也算契约：`scripts/test_readme.js` 守两份 README 与 `Cargo.toml` 版本一致、二级标题数量对齐；`build.rs` 从 `CHANGELOG.md` 抽 `## [x.y.z]` 段落编译进 exe 当更新公告（缺段落只是「暂无更新说明」，但不能因此忘写）。
- 修 bug 时补一条固定旧行为的回归用例（如 `old_behavior_cross_midnight_yielded_nothing`），把「这次改了什么」写进代码；跨端字段契约类 bug 用「与源码比对」的测试锁死，别靠驼峰/下划线自觉。
- 功能「看起来实现了」要顺调用链确认上游没提前 return（周末加班开关曾是死代码：`if !is_workday { return }`）；周期检测类任务不要用「检测时刻」当业务归属时间，一律以事件时刻为准（时间戳 / mtime / 日志时间）。

## 仓库与环境陷阱
- **绝不跑 `git stash`**：本机删除劫持会把 gc 的删除转成搬回收站，曾导致仓库损坏；`.git/config` 已设 `gc.auto = 0` 与 `autoDetach = false`。验证改动用 `git diff` 或临时 commit。
- **打补丁脚本必须行尾无关**：当前仓库检出统一为 CRLF（`index.html` / `styles.css` / `js/*.js` / `*.rs` / `*.bat` / `*.md` 均无裸 LF），但仍**不要用裸 `\n` 锚点**——用 `splitlines(keepends=True)` 按行内容定位，插入/替换时沿用文件原有换行符。
- 验证/还原文件一律二进制读写（文本模式会静默改换行符，`git diff` 看不出来）；改完用 `git diff --stat` 复核只剩预期行。
- **写脚本用 UTF-8**：`scripts/test_shutdown_hooks.js` 的头部中文注释已经因为编码往返变成乱码（`茅聙聙…`），是活样本——别重蹈。
- 删整块代码别靠 brace 匹配（`strip()=="}}"` 极易截到内层 `}` 而非函数闭合，留下孤儿/缺括号）：新文件或重写文件直接全量写盘最稳。
- 生成物与本地产物不进库：`.tmp/`、`bin/`、`target/`、`src-tauri/gen/`、`src-tauri/permissions/autogenerated/`、`logs/`、`*.key`、`.workbuddy/`、`.superpowers/`。
- 根目录不留调试脚本，日志统一进 `logs/`；改功能要同步 README 双语 + CHANGELOG（前者由 `test_readme.js` 强制执行）。
