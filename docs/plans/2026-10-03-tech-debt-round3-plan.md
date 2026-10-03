# 技术债第三轮清偿 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按 [设计文档](2026-10-03-tech-debt-round3-design.md) 清偿第三轮摸底清单（1 高 + 8 中 + 约 12 低），四批推进，最终发 v1.8.2。

**Architecture:** 批次一 Rust 后端小步修（钳制/登记表/事务/三组收口），批次二发版链路（CI workflow + bat 门禁 + 新守卫脚本），批次三前端（core.js 兜底 + 双份收口 + 七小件），批次四依赖升级（单包小步、独立提交、风险隔离）。

**Tech Stack:** Tauri 2 + Rust（src-tauri），原生 HTML/CSS/JS（无框架、无构建、无 CDN），Node 源码断言风格测试（scripts/test_*.js），Python 发布引擎测试。

**设计文档偏差记录：** 写计划前已对四路摸底报告的全部关键 file:line 人工复核源码，无偏差。D2 有一个既有测试语义变化（见 Task 1.2）。

## Global Constraints

- **四个根 .bat 纯 ASCII、注释写英文、无 chcp、无 BOM**（test_bat_vars.js 自动守；本轮会改 build.bat / release.bat，新增注释一律英文）
- **改前端必须 `cargo build` 一次**，build.rs 回写（FE_VER/capability）的自动改动随当批提交（CI drift guard 会拦）
- 每批一个本地提交，惯例 `fix(tech-debt): 批次N——<主题>`；批次四依赖每包一个 `chore(deps):` 小提交；**push 与发版统一在 Task 5**（release.bat 自会推 tag）
- 前端保持无框架/无构建/无 CDN；**frontend/ 内禁放临时文件**（build.rs 指纹漂移）；目检用 dev.bat 或临时目录+`__TAURI__`桩可视化验证法
- bat 行为验证必须在真实 cmd 终端（Git Bash 复现不出不代表没问题）；PowerShell 一律 `pwsh -NoProfile -Command '...'`
- 测试断言标签用中文（项目风格）；新 JS 测试命名 `scripts/test_*.js`（run_all.js 自动收录）
- **不动清单**（设计文档「非目标」全节）：老跳过项三条、DB Mutex writer 线程、模块拆分、CSP、.gitattributes、win.rs 边界
- 本机若遇 cl 闪断 D8050：按既案 `-j1` 重试（机器态，代码无辜）

## 测试影响盘点

| 测试 | 动作 |
|------|------|
| `scripts/test_commands.js` | +1 条：get_day_timeline 钳制守卫（Task 1.1） |
| `scripts/test_publish_release.py` | +1 条：PDB 步骤 best-effort 结构断言（Task 2.1） |
| `scripts/test_local_gate.js` | **新建**：本地门禁三步 / install-action 钉版 / REPO 一致性（Task 2.2-2.4） |
| `scripts/test_frontend_split.js` | +1 条检查：js/ 目录文件集合 == CHUNKS 双向断言（Task 3.4-⑦） |
| `scripts/test_report_image.js` | +1 条：标题随跨度映射（Task 3.3） |
| `scripts/test_insights.js` / `test_hover_card.js` | 七小件触点复核，视触点补 1–2 条（Task 3.4） |
| Rust 侧（backup.rs / db.rs / calc.rs 等内嵌 #[test]） | backup +3~4 条、TABLE_SINCE 等价 +1 条、to_min 测试改写、其余既有复跑 |
| `scripts/test_bat_vars.js` | 零改动（新 bat 改动被既有 ASCII/变量契约自动覆盖） |
| 其余 ~30 个 test_*.js + 2 个 Python 套件 | 不受影响，各批收尾 run_all 复核 |

---

### Task 1: 批次一——Rust 正确性与兼容 ✅ 已完成（2026-10-03，提交 d9e7cf9，build.bat test 全绿）

> 执行记录：7 项全部落地（clamp 守卫进 test_commands.js；TABLE_SINCE=v1.0.0×7+focus_sessions=1.7.0（git tag 8bc62ff 考证）；还原放行路径安全性已核实——main.rs:298 apply_pending_on_startup 先于 :306 db::conn()；既有测试语义同步：test_day_timeline.js 1 条、test_insights.js 2 条改写为新口径）。

**Files:** `src-tauri/src/cmds_bill.rs`、`src-tauri/src/db.rs`、`src-tauri/src/backup.rs`、`src-tauri/src/maintain.rs`、`src-tauri/src/weekbill.rs`、`src-tauri/src/insights.rs`、`src-tauri/src/focus.rs`、`src-tauri/src/calc.rs`、`src-tauri/src/overtime.rs`、`src-tauri/src/diag.rs`（只读参考）、`scripts/test_commands.js`

**1.1 get_day_timeline offset 钳制（D1，高）**
- [ ] `cmds_bill.rs` get_day_timeline：`let off = offset.max(0);` → `let off = offset.clamp(0, 1200);`，附中文注释说明与 `weekbill::period_bounds` 同口径（时间线按天翻，1200 天封顶；前端 IPC 直达 i64，防 chrono 日期运算 panic——period_bounds 同类漏网收口）
- [ ] `test_commands.js` 加守卫：get_day_timeline 函数体含 `offset.clamp(0, 1200)`、不含 `offset.max(0)`

**1.2 备份校验向前兼容（D2，中）**
- [ ] 用 git log 逐表确认 8 张业务表各自的引入版本，写进 `db.rs` 新增 `pub(crate) const TABLE_SINCE: &[(&str, &str)]`（表名 → "x.y.z"）
- [ ] `db.rs` 既有清单等价性测试处加一条：`TABLE_SINCE` 表集合 == `BUSINESS_TABLES`（双向）
- [ ] `backup.rs` validate_archive 缺表分支改判定：仅当「备份 manifest 的 app_version 可解析 && 该表引入版本可解析 && backup_version >= 引入版本」才返回 Err（真残缺）；老备份合法缺表 / 任一版本解析失败一律放行（fail-open，沿用本文件 version_is_newer 的注释哲学）
- [ ] 核实还原落库流程：放行路径还原后必须 init_tables 补建空表；若还原是整库替换且替换后无建表步骤，在还原完成处补一次 init_tables 调用
- [ ] **既有测试语义更新**：v1.8.1 的「缺任一表即拒」用例按新语义改写（缺已引入表→Err；缺未引入表→Ok），断言注明向前兼容决策；新增 3 条：老备份缺新表放行 / 同代备份缺表拒绝 / app_version 不可解析放行

**1.3 purge 包事务（D3）**
- [ ] `maintain.rs` purge_before_conn：循环体外开 `conn.unchecked_transaction()?`，DELETE 在事务上执行，成功 `commit()`；函数签名 `(&Connection, NaiveDate) -> Result<u32>` 不变；既有 purge 测试复跑零变化

**1.4 WEEKDAYS_CN 收口（D4）**
- [ ] `weekbill.rs` 的 `WEEKDAYS_CN` + `weekday_cn` 改 `pub(crate)`；`insights.rs` / `focus.rs` 删本地副本改 `use crate::weekbill::…`；全库 grep 确认仅剩一份定义

**1.5 to_min 统一（D5）**
- [ ] `overtime.rs:215` 的 `fn to_min(s: &str) -> Option<f64>` 移入 `calc.rs` 作单一实现（overtime 改 use）；`calc.rs:66` 旧 `pub fn to_min(s) -> f64` 删除，调用方（daily_hours 等配置解析路径）显式处理 None：`debug_log` 记坏配置项名与原值 + 回退该配置默认值
- [ ] `calc.rs` to_min_variants 测试改写为 Option 语义 + 新增「坏字符串不走 0.0」断言；grep 全库确认无第二个 to_min 语义分叉

**1.6 db.rs 模块文档（D6）**
- [ ] `db.rs:7` `holiday_cache.json` → `holiday_{year}.json`；「表结构」手抄段替换为指向 `db::BUSINESS_TABLES` 单一真相源的说明

**1.7 开库失败可见反馈（D7）**
- [ ] 先读 `diag.rs` show_fatal 实现确认线程/时机约束，二选一：① db.rs 两处 expect 失败路径先 `diag::show_fatal("无法打开数据库：…（被占用/磁盘满/权限）")` 再 panic（保 panic.log 链路）；② 若 show_fatal 在 setup 前不可用，把 conn() 初始化挪进 tauri setup 转 Err 走现成弹窗路径。**把选择与理由写进代码注释**
- [ ] `init_tables` 的 expect 同样处理

**Task 1 验收**
- [ ] `cargo test` 全绿（新增 ≥5 条）；`cargo clippy --all-targets -- -D warnings` 干净
- [ ] `node scripts/run_all.js` 全绿（含 test_commands 新守卫）
- [ ] 提交：`fix(tech-debt): 批次一——正确性与兼容（timeline 钳制/备份向前兼容/purge 事务/三组收口）`

---

### Task 2: 批次二——发版链路 ✅ 已完成（2026-10-03，提交见 git log 批次二，build.bat test 七步全绿）

> 执行记录：cargo-deny 本地已有 0.20.2（钉同版）；新增 scripts/test_local_gate.js（9 条）；build.bat :do_test 现为 fmt→clippy→deny→test→run_all→双 Python 七步，与 tests.yml 逐字同 flag、cargo-deny 缺失 fail-closed；release.yml PDB 步骤补 continue-on-error + tauri-cli@2 钉版；release.bat GIT 回退拒绝 PortableGit/WorkBuddy；REPO 两处交叉注释；CONVENTIONS.md build.bat test 描述同步为六步对齐版。**注意：clippy 曾红两次——① 收口 weekday_cn 后 focus/insights 的 Datelike 变未使用导入（已删）；② 在 Git Bash 裸跑 cargo clippy 会绕过 common.bat 自持 TMP 触发 cc D8050，必须在 build.bat/cmd 语境跑。**

**Files:** `.github/workflows/release.yml`、`.github/workflows/tests.yml`、`build.bat`、`release.bat`、`scripts/push_via_api.py`、`scripts/test_publish_release.py`、新建 `scripts/test_local_gate.js`

**2.1 PDB best-effort 落地（D8）**
- [ ] `release.yml` 「Upload PDB (best-effort)」步骤加 `continue-on-error: true`（publish_release.py 保持返 1 不动——本地应急路径需要真实退出码）
- [ ] `test_publish_release.py` 加 `test_pdb_upload_is_best_effort`：读取 release.yml 断言该步骤含 `continue-on-error: true`

**2.2 本地门禁补齐（D9）**
- [ ] `build.bat` `:do_test`（约 :171-201）在 cargo test **之前**追加，flag 与 tests.yml rust-checks 逐字一致，errorlevel 走既有 `ERRCODE` 模式：
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo deny check advisories`——先 `cargo deny --version` 探测，缺失则打印安装指引（`cargo install cargo-deny --locked`）并 `exit /b 1`（fail-closed）
- [ ] 头部 echo 文案同步更新；**新增注释一律英文**（纯 ASCII 约束）

**2.3 workflow 工具钉版（D10）**
- [ ] 查 crates.io 取 cargo-deny 当前稳定精确版；`tests.yml` dep-audit `tool: cargo-deny` → `tool: cargo-deny@X.Y.Z`（与本地安装版本一致）
- [ ] `release.yml` `tool: tauri-cli` → `tool: tauri-cli@2`（大版本钉定，对齐本地 `--version "2"` 策略；附近注释顺带更新）

**2.4 REPO/GIT 收口（D11）**
- [ ] `release.bat` GIT 解析：保留系统 Git 优先；`where git` 回退时校验候选路径不含 `PortableGit`/`WorkBuddy` 字样，命中即报错退出并说明（凭据管理器段死，本文件既有注释自述）
- [ ] `release.bat:25` REPO 与 `push_via_api.py:36` REPO 互加交叉注释（单一事实，改一处必改另一处）
- [ ] 新建 `scripts/test_local_gate.js` 四组断言：① build.bat :do_test 含 fmt --check / clippy -D warnings / cargo deny check advisories 三步；② tests.yml 的 cargo-deny、release.yml 的 tauri-cli install-action 均带 `@版本`；③ REPO 两处字面量一致；④ release.yml PDB 步骤含 continue-on-error（与 test_publish_release.py 互补双保险）

**Task 2 验收**
- [ ] 本地首次先 `cargo install cargo-deny --locked`（若缺），然后 `build.bat test` 全绿（六步对齐版）
- [ ] `python scripts/test_publish_release.py` 全绿（含新结构断言）；`node scripts/test_local_gate.js` 绿
- [ ] 提交：`fix(tech-debt): 批次二——发版链路（PDB best-effort/本地门禁补齐/工具钉版/REPO 收口）`

---

### Task 3: 批次三——前端体验

**Files:** `frontend/js/hero.js`、`frontend/js/core.js`、`frontend/js/bill.js`、`frontend/js/monitor.js`、`frontend/js/insights.js`、`frontend/hover_card.html`、`scripts/test_frontend_split.js`、`scripts/test_report_image.js`、`scripts/test_insights.js`、`scripts/test_hover_card.js`

**3.1 hero tick 失败留痕（D12）**
- [ ] `hero.js` tick() 的空 catch 改 `flog("tick ERR: " + (e && e.message ? e.message : String(e)))`（确认 hero.js 可用 core.js 的 flog；若不可用则走既有日志通道）
- [ ] 评估：复用主界面活动条失败 toast 的 2 秒去重模式给徽章降级文案；最小实现只做 flog 也可接受，实施时记录选择

**3.2 全局 rejection 兜底 + saveBillImage（D13）**
- [ ] `core.js` 在 window error 监听旁补 `window.addEventListener("unhandledrejection", …)` → flog（照 hover_card.html:424-427 既有口径）
- [ ] `bill.js` saveBillImage try/finally 补 catch：toast「账单图导出失败」+ flog（带 e.message）

**3.3 报告图标题随跨度（D14）**
- [ ] `bill.js` drawReport：标题按 model 跨度映射「周账单/月账单」（先看 buildReportModel 的跨度字段名）；年跨度按钮已置灰无需第三文案
- [ ] `test_report_image.js` 加断言：drawReport 标题来自跨度映射，不再有写死「周账单」拼接

**3.4 前端低优七小件（D15）**
- [ ] ① px→距离收口：core.js 单一 formatter（米一位小数、≥1000m 显 km），monitor.js `fmtDist` 与 insights.js:328 `fmtMeters` 改引用后删除本地实现
- [ ] ② 时长格式化收口：core.js 统一「分钟」措辞，monitor.js `fmtDurCN` 与 insights.js `tlDur` 改引用；相关既有断言口径复核
- [ ] ③ 金句阈值：SLACK_QUIPS 与 WEEK_BILL_QUIPS 重叠文案收 core.js 共享字面量（两表各自保留 tier 结构）
- [ ] ④ insights.js 四个 `*Data` 死状态变量降局部/删除，误导注释清除
- [ ] ⑤ hover_card.html startQuips 的 3s setTimeout 登记句柄，stopQuips/hideCard 清除
- [ ] ⑥ monitor.js appIconHTML 的 `src` 插值过 escapeHtml（与全项目既有转义用法一致）
- [ ] ⑦ `test_frontend_split.js` 补第 5 项检查：`frontend/js/` 目录 `.js` 文件集合 == CHUNKS（双向：目录多文件 / CHUNKS 指向不存在的文件都红）

**Task 3 验收**
- [ ] `node scripts/run_all.js` 全绿（含四组新断言）
- [ ] `cargo build` 一次，build.rs 回写改动随本批提交（drift guard）
- [ ] 目检：dev.bat 起服务过主界面/账单页/悬停卡（临时目录可视化验证法，frontend/ 内不留文件）
- [ ] 提交：`fix(tech-debt): 批次三——前端体验（hero 留痕/rejection 兜底/标题随跨度/七小件）`

---

### Task 4: 批次四——依赖升级（每小步独立可回退）

**Files:** `src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/src/backup.rs`（zip 使用点）、reqwest 使用点（holiday.rs / update.rs）

**4.1 树内已有大版本对齐（每包：改 Cargo.toml → `cargo update -p <crate>` → `cargo check` → `cargo test` → 单独提交）**
- [ ] png 0.17 → 0.18（grep 全部 `png::` 调用点过 API 变更）
- [ ] dirs 5 → 6（grep 使用点）
- [ ] zip 2 → 4（唯一使用点 backup.rs:79-80,559；SimpleFileOptions / ZipWriter / ZipArchive / CompressionMethod 对照 zip 4 调整）

**4.2 rusqlite 0.32 → 0.38**
- [ ] Cargo.toml `rusqlite = { version = "0.38", features = ["bundled"] }` → `cargo update -p rusqlite` → `cargo check` 过 0.33–0.38 API 变更
- [ ] 全量 `cargo test`（243 个）+ `build.bat test`；bundled SQLite 重编遇 D8050 按 Global Constraints 处理
- [ ] 提交：`chore(deps): rusqlite 0.32 -> 0.38（bundled SQLite 升级）`

**4.3 tauri 系小版本**
- [ ] `cargo update -p tauri -p tauri-plugin-updater` 及各 plugin（Cargo.toml 均为 version="2"，锁内直接到 2.x 最新，含 tauri 2.11.5→2.12.x）→ `cargo check` → 全量测试 → 提交

**4.4 reqwest 0.12 → 0.13（试升，允许失败）**
- [ ] 尝试对齐 0.13（树内 plugin-updater 已用 0.13.4；使用点 holiday.rs:256 与 update.rs 三处，确认 blocking+json 可用）
- [ ] 失败则在 Cargo.toml reqwest 行上注释记档「0.12 与树内 0.13 并存，待 tauri 生态统一」并不硬升；base64 0.22 已是树内新版，不动

**4.5 批次验收**
- [ ] `cargo deny check advisories` 全绿（升级无新告警）；`build.bat test` 全绿
- [ ] 本地 release 构建，bin/package 体积对比记录（进 CHANGELOG 素材）

---

### Task 5: 收尾与发版 v1.8.2

- [ ] `build.bat test`（此时为六步对齐版）全绿
- [ ] CHANGELOG 增 `[1.8.2]` 段：开头一句用户视角摘要（v1.8.1 惯例），四批按「修复/变更」归段，依赖升级记体积变化
- [ ] README.md / README.zh-CN.md 版本对齐（`test_readme.js` 三处同步守卫）
- [ ] `release.bat <版本> /y` 一键发版（本地六步门禁此时已含 fmt/clippy/deny）；云端 success + 8 资产 + latest.json 可拉
- [ ] 更新记忆 `tech-debt-round3-2026-10`：清偿状态、发版结果、仍开放项
- [ ] 提交：`chore(release): v1.8.2 发版材料——CHANGELOG 归段、README 双语版本对齐` + `release: v1.8.2`
