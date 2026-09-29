# 设计：CI 自动化 + 前端代码拆分

日期：2026-09-29
状态：已批准（2026-09-29）
所属迭代：无（方向探索后的工程健康专项）
关联：
- 测试：`scripts/run_all.js` + 27 个 `scripts/test_*.js`
- 构建：`src-tauri/build.rs`（指纹 / 缓存戳 / capabilities / command_names）
- 拆分对象：`frontend/app.js`（2972 行 / 115 KB，非空行 2773）、`frontend/index.html`（817 行内联脚本 + 1 个 script 标签）

## 0. 背景与定位

现状（已核实）：

1. **无 CI**：仓库无 `.github/` 目录，27 个前端断言脚本与 Rust 单测全靠手动执行，回归依赖自觉；
2. 仓库公开（`github.com/wangdxnum1/niuma-timer`），GitHub Actions 对公开仓库免费且不限时长；
3. `app.js` 单文件持续膨胀（2972 行），内部按视图域自然聚簇（函数清单见 §2.2），但文件级边界为零；
4. `build.rs` 三个关键机制：指纹**递归扫描 `frontend/` 全目录**（新增子目录自动纳入）；`?v=` 缓存戳回写为**循环替换**（同一文件多处自动全量同步）；`command_names()` **只解析 `src/main.rs`**（命令移出 main.rs 需升级解析器）；
5. 27 个前端测试中 **22 个**为 `app.js` 源码文本断言（`readFileSync` 后做 `includes` 匹配）——拆分后需聚合适配；其余 5 个（build_info / readme / hover_card / hover_interaction / lock_order）不读 `app.js`，无需适配；
6. `cargo fmt --check` 当前不过（build.rs 等存在格式 diff）；
7. `build.rs` 编译期会自动回写缓存戳与 capabilities——**提交里忘了带这些自动改动**是一类真实事故（CHANGELOG 2026-09-11 的 ACL 事故即属此类），目前无任何机制拦截。

定位：**先建安全网（CI），再做行为等价的前端拆分**。不做发布流水线上云（二期，`release.bat` 的代理/凭据链路需单独调试）；不动 Rust 侧模块结构（main.rs 1126 行只是接线层，模块边界本已清晰，且动它必须升级 build.rs 解析器，收益配不上风险）。

### 成功标准

- **G1** push / PR 自动执行全部前端断言 + `cargo fmt/clippy/test`，回归不再依赖手动；
- **G2** 前端拆分后**行为 100% 等价**：22 个既有测试换读源方式后全绿（其余 5 个不受影响），新增契约测试锁住拆分结构；
- **G3** 「改前端忘同步缓存戳 / 加命令忘带 capabilities」在 CI 直接红（指纹漂移守卫）；
- **G4** 拆分后新增测试脚本无需关心分块细节（聚合器兜底，`run_all.js` 自动发现新测试）。

## 1. 决策表

| 决策点 | 结论 | 理由 |
|---|---|---|
| CI 范围 | 前端测试 + fmt/clippy/test + 指纹漂移守卫；**不含**发布流水线 | 发布上云未选，代理/凭据坑需二期单独调 |
| rust job 的 runner | `windows-latest` | windows crate + Win32 链接，交叉平台 `cargo check` 不可靠；公开仓库免费 |
| 前端测试 runner | `ubuntu-latest` + Node 20 | 纯 fs 文本断言，与 OS 无关，更快 |
| rustfmt | 一次性 `cargo fmt` 提交后启用 `--check` | 当前不过 check，先清后锁 |
| clippy 策略 | 首期 `-D warnings`；存量告警少则直接修，多则建豁免清单（记在 workflow 注释，二期清零） | 实施时按实际数量定，目标 0 豁免 |
| 前端拆分方式 | **经典多 `<script>` 顺序加载**；不用 ES modules、不用构建期拼接 | 见 §2.2 备选方案对比 |
| Rust 侧拆分 | 本期不动（含 `command_names()` 单文件解析） | 风险/收益比不划算 |
| 测试适配 | 新增 `scripts/lib/fe_sources.js` 聚合器，各测试改 1 行读源 | `includes` 断言在聚合文本上语义不变 |
| app.js 去留 | **删除**，由 `frontend/js/` 分块取代 | 防止双源漂移 |
| FE_VER 常量位置 | 移入 `js/core.js`，build.rs `TARGETS` 同步调整 | 常量必须存在于被回写的文件中 |

## 2. 分项设计

### 2.1 CI 工作流（`.github/workflows/ci.yml`）

触发与并发：

```yaml
on:
  push:
    branches: [main]
    paths-ignore: ["**.md", "docs/**"]
  pull_request:
    paths-ignore: ["**.md", "docs/**"]
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true
```

**Job A：`frontend-tests`**（ubuntu-latest，秒级）
1. `actions/checkout@v4`；
2. `actions/setup-node@v4`（node 20）；
3. `node scripts/run_all.js`——27 个脚本在子进程中逐个跑，失败即停（run_all 现有语义保持）。

**Job B：`rust-checks`**（windows-latest）
1. `actions/checkout@v4` + `dtolnay/rust-toolchain@stable`（components: rustfmt, clippy）+ `Swatinem/rust-cache@v2`（按 `Cargo.lock` 哈希缓存）；
2. `cargo fmt --all -- --check`；
3. `cargo clippy --all-targets -- -D warnings`；
4. `cargo test`（既有单测均为纯函数测试，无窗口/文件依赖，headless 可跑）；
5. **指纹漂移守卫**：构建完成后 `git diff --exit-code`。原理：build.rs 在编译期间会自动回写缓存戳（`?v=` / FE_VER）与 capabilities 的 `allow-*`；一个干净的提交在 CI 上构建后应当零 diff。若非零，输出提示「本地 `cargo build` 一次，把自动改动的文件一并提交」并以退出码 1 失败。`BUILD_TIME` 等构建信息仅走 `rustc-env`，不落盘，不会误报。

两个 job 均为必须通过项（个人仓库是否启用 branch protection 强制，由用户自行决定，workflow 本身不改行为）。

### 2.2 前端拆分

**分块与加载序**（保持 index.html 现有 body 尾部位置与普通 `<script>` 标签，按文档序执行；经典脚本共享全局词法环境，顶层函数声明即 `window` 全局——内联 HTML 事件处理器不受影响，与单文件语义等价）：

| 序 | 文件 | 内容（按 app.js 函数聚簇划定） |
|---|---|---|
| 1 | `js/core.js` | TAURI/invoke 垫片、`FE_VER`、`flog`、全局 error 兜底、`$`、全局状态（winVisible/curView/viewData/monitors）、通用格式化（fmtShortH/fmtDurCN/fmtBytes/fmtWan/fmtMeters/fmtDist/escapeHtml/sleep）、日期工具（todayStr/fmtYMD/addDays/isToday/dateLabel） |
| 2 | `js/settings.js` | load/save/doSave/readCfg、白名单 chips、monitor 状态同步、showToast |
| 3 | `js/hero.js` | refresh/silentRefresh/tick、renderBadge/renderSparkline/renderTimeline/renderTagline/dynamicTagline |
| 4 | `js/overtime.js` | 加班视图全家桶、表单/确认（showConfirm/hideConfirm/confirmResolve）、CSV（csvCell/csvRows/downloadCsv/exportOvertimeCsv/exportWeekBillCsv） |
| 5 | `js/monitor.js` | 键鼠/应用/媒体三视图、renderChart/renderTopKeys/renderHourChart、分类（CAT_CYCLE/catKey/renderCategories/cycleAppCategory）、图标缓存与合并、日期导航（histDate/DAY_NAV/shiftHist） |
| 6 | `js/bill.js` | 周账单 receipt/dash 双风格、quips（QUIPS）、fmtMoney、span 切换 |
| 7 | `js/insights.js` | 时段热力/趋势/身体账单三视图、SVG 工具（svgEl） |
| 8 | `js/storage.js` | 存储占用、清理、备份/还原 |
| 9 | `js/update.js` | 更新页、renderMarkdown、下载进度、watcher |
| 10 | `js/boot.js` | showView/repaintCurrentView/DETAIL_VIEWS、视图控制器绑定（mon-seg/rail/pager/keydown/day-nav）、通用翻页器（pgStep/billStep）、调试卡与彩蛋、splash/showWindow/closeWindow、boot/refreshAll/watchVisibility、bindDebugBtn、**启动序列**（boot()、watch*、轮询 setInterval/setTimeout） |

边界原则：**纯连续切割、不移动任何行**——经典脚本共享全局词法环境，跨块函数调用在运行时解析，无顺序问题；唯一硬约束是**加载序满足声明期求值**（某块顶层语句引用的标识符须已在更早加载的块或本块声明）。已核定全部 71 处顶层挂载与启动序列：其引用全部落在更早加载的块（settings 域绑定物理落在 monitor 段、overtime 域绑定物理落在 storage 段，均指向更早块），当前分块序安全；`boot.js` 必须最后加载。行号地图与逐项核验见实施计划，上表为聚簇基线。

**build.rs 改动（最小）**：
- `TARGETS`：`"../frontend/app.js"` → `"../frontend/js/core.js"`（FE_VER 所在块）；
- `rerun-if-changed=../frontend`（目录级条目已覆盖 js/ 子目录，删除单文件级 app.js 条目）；
- `normalize` / `apply_ver` / 指纹逻辑**零改动**：FE_VER_PREFIX 在 core.js 命中；`?v=` 多处命中由循环替换覆盖；指纹递归扫描自动纳入新目录。

**删除**：`frontend/app.js`。

**备选方案对比**（为何不选）：

| 方案 | 否决理由 |
|---|---|
| ES modules | 子模块 URL 写死在 import 语句中，`?v=` 缓存戳只作用于入口，WebView2 缓存穿透需另造机制（版本化目录/导入映射）；`type=module` 改变执行语义（strict/延迟/顶层 this），对 817 行内联脚本的真实风险 |
| 源码目录 + 构建期拼接 | 引入「先拼再编」构建步骤与两套文件，违背 no-bundler 哲学，编辑体验分裂 |

### 2.3 测试适配与契约

**`scripts/lib/fe_sources.js`（新增）**：导出 `CHUNKS`（有序文件名常量）与聚合读取函数——按加载序拼接全部块的文本（`\n` 分隔），并同供 index.html / styles.css 文本。22 个测试脚本唯一改动：`readFileSync(.../app.js)` 换为聚合源 `feSource()`（每文件 1-2 行 diff），`includes` 断言语义不变；其余 5 个测试零改动。

**`scripts/test_frontend_split.js`（新增契约测试）**：
1. index.html 按序引用全部 10 块且每处带 `?v=`；
2. `frontend/app.js` 不存在，全仓前端引用无 `app.js?` 残留；
3. `const FE_VER` 位于 core.js；
4. 声明-挂载白名单守卫：非 boot 块顶层（列 0）仅允许声明（function/const/let/var）与纯挂载（`$(...)` / `document.*` / `window.*`），出现裸业务调用即红（正则近似）；
5. 命名 `test_` 前缀即可被 `run_all.js` 自动发现，聚合器无需注册。

### 2.4 实施顺序（两个 PR）

**PR-1：CI 安全网**（不碰业务代码）
1. 一次性 `cargo fmt` 提交；
2. 清 clippy 存量（少→修；多→豁免清单）；
3. `ci.yml` 落地，GitHub 上验证两 job 全绿；
4. 漂移守卫负向自测：故意改前端不提交缓存戳 → CI 红 → 还原（与 pdb spec 的验证做法一致）。

**PR-2：前端拆分**（行为等价手术）
1. 先落聚合器与契约测试（此时应红：app.js 仍在）；
2. 按「逐块切割（每块经 `node --check` + `scripts/lib/split_check.js` 逐行还原校验）→ index.html 多标签 + build.rs TARGETS → 本地构建同步缓存戳 + 冒烟 → 删 app.js → 22 测试换读源 → `run_all.js` 全绿」顺序执行；
3. 本地 `cargo build` 冒烟：主界面 tick、设置保存、账单四 tab、三监控视图、更新页、备份还原 UI；
4. README 双语同步（项目结构树 + 测试说明）。

## 3. 验证方式

- PR-1：Actions 两 job 全绿 + 漂移守卫负向自测通过；
- PR-2：`run_all.js` 全绿（27+1 脚本）+ `cargo test` 全绿 + 指纹守卫过 + 冒烟清单走查 + 悬停卡（独立 HTML，不受影响）确认。

## 4. 风险与回退

| 风险 | 缓解 |
|---|---|
| 拆分引入加载序 bug | 契约测试锁结构 + 逐块切换逐步验证 + 冒烟清单；单 PR 可整体 revert |
| clippy 存量过大拖慢 PR-1 | 豁免清单记录于 workflow 注释，二期清零，不阻塞 CI 上线 |
| CI 与本地构建产物差异（build.rs 回写） | 漂移守卫正是为此设计；负向自测验证其有效性 |
| windows runner 偶发抖动 | rust-cache 热身后耗时可控；必要时对 job 加一层重试，首期不加 |
