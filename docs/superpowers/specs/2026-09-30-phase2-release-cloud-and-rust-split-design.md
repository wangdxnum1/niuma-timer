# 设计：二期——发布流水线上云 + Rust 侧模块化

日期：2026-09-30
状态：实施中（用户 2026-09-30 发话启动二期）
所属迭代：二期（2026-09-29 CI + 前端拆分专项的延续，两项在当期设计文档 §0/§1 中明确留待本期）
前置：`2026-09-29-ci-and-frontend-split-design.md`（CI 安全网 + 前端拆分，已交付）

## 0. 背景与现状（已核实）

**发布链现状**：`release.bat` 在本机按序执行 测试 → cargo build → `cargo tauri build` 打包（NSIS + MSI + 便携版 + .sig + PDB.zip）→ 产物校验 → `publish_release.py --generate-notes-only` 生成元数据（RELEASE_NOTES.md / latest.json / SHA256SUMS.txt）→ commit → tag → push → 本机创建 GitHub Release 并上传。后三步依赖本机环境：Git 凭据管理器里的 GitHub 令牌、7890 端口代理探测、gh CLI 或 Python 回退链、浏览器兜底——换台机器发不了版，代理没开就卡住。

**关键事实**：

1. `scripts/publish_release.py` 本身就是完整的发布引擎：建草稿 → 上传必需资产（bins+sigs+latest.json+SHA256SUMS.txt）→ 缺资产 fail-closed → PATCH 公开 → 发布后 tags 端点验证。幂等（草稿复用、已传资产跳过、拒绝改动已公开 release）。**上云 = 复用它，不重写**；
2. 该脚本取 token 只走 Git 凭据管理器（wincred）——CI 上不存在，需加环境变量取值路径；
3. 更新器签名私钥在本机 `~/.tauri/niuma-timer.key`（`TAURI_SIGNING_PRIVATE_KEY`），缺了会静默产出无签名包、用户侧更新被拒——CI 上必须配成 repo secret；
4. `tauri.conf.json`：`createUpdaterArtifacts: true`，bundle targets `all`（Windows 产 NSIS + MSI）；latest.json 的 updater 端点指向 `releases/latest/download/latest.json`，故发布必须 `make_latest`（脚本已做）；
5. NSIS / WiX 由 tauri CLI 在 Windows 上自动下载，runner 无需预装；
6. main.rs 实测 1241 行（一期设计时 1126）：约 810 行是 `#[tauri::command]` 与可归组业务函数，main() 本体 + setup 接线约 430 行；
7. `build.rs command_names()` 只解析 `src/main.rs`（L176-221），命令移出 main.rs 必须先升级它——这正是留到二期的原因；
8. 11 个前端测试脚本读 `src-tauri/src/main.rs` 做文本断言（test_commands / test_build_info / test_guard / test_update / test_remind / test_shortcut / test_week_bill / test_bill / test_insights 等），拆分后需聚合适配；
9. scheduler.rs（3 处）/ tray.rs（4 处）/ remind.rs（1 处）以 `crate::xxx` 引用 main.rs 的 get_status / AppState / toggle_pause / spawn_holiday_refresh / refresh_tray / maybe_rollover_day / maybe_record_overtime_lock。

## 1. 决策表

| 决策点 | 结论 | 理由 |
|---|---|---|
| 发布引擎 | 复用 `publish_release.py`，CI 与本地共用 | 语义（资产清单、fail-closed、幂等、后验证）已实战验证；重写必引入漂移 |
| CI token | `github_token()` 优先读环境变量（`GITHUB_TOKEN` / `NIUMA_GITHUB_TOKEN`），回落凭据管理器 | CI 用内置 `GITHUB_TOKEN`（contents:write），本机行为不变 |
| 触发方式 | 推送 `v*` tag 触发发布；`workflow_dispatch` + `dry_run` 输入用于演练 | tag 推送是显式动作，等价于现在 release.bat 的确认步骤；dry_run 只上传运行工件不建 Release |
| 公开策略 | 沿用脚本现状：草稿 → 校验 → 自动公开 | 与现 release.bat 行为一致；「推 tag 即发布」门槛够高 |
| release.bat | 删除本地发布段（[5/5] 与 gh/py/browser 三条路径），保留 测试/构建/打包/元数据校验/commit/tag/push | 本机不再需要令牌/代理/gh；应急仍可手动跑 `publish_release.py`（wincred 路径保留） |
| 签名密钥 | GitHub Secrets：`TAURI_SIGNING_PRIVATE_KEY`（+ 可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`） | 无密钥则 .sig 缺失、latest.json 生成直接 fail-closed；经 `gh secret set` 配置，密钥值不落日志 |
| release job runner | `windows-latest` + `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache@v2` | 与 ci.yml rust-checks 同构；tauri-cli 经 `taiki-e/install-action`（预编译秒装，避免 8 分钟 cargo install） |
| release job 测试 | 跑 `cargo test` + `node scripts/run_all.js`（与 release.bat 步骤 0 对齐） | tag 指向的提交理论上已被 ci.yml 验过，但 fail-closed 优先，几分钟成本 |
| 产物收集 | workflow 内用 pwsh 复刻 build.bat `:do_package` 的收集/重命名/压缩逻辑 | 逻辑短（~30 行）；抽共享脚本反而牵动本机已验证的 bat 链 |
| Rust 拆分粒度 | main.rs → 保留入口+接线（约 430 行）；切出 diag / state / cmds_core / cmds_bill / cmds_monitor / cmds_storage / cmds_update / cmds_debug 8 个模块 | 按域聚簇；不追求更碎（cmds_monitor 只有 3 个命令，但域边界清晰） |
| crate::xxx 引用兼容 | main.rs 根模块 `pub(crate) use` re-export 共享项，scheduler/tray/remind **零改动** | 跨模块调用点是「crate 根命名」语义，re-export 等价；少动 3 个文件的 8 处调用 |
| build.rs 升级 | `command_names()` 递归扫描 `src/**/*.rs`；新增 `cargo:rerun-if-changed=src` | 单一真相源扩大到全 crate；rerun 保证新命令在本地也及时补 capabilities |
| 测试适配 | 新增 `scripts/lib/rs_sources.js`（RS_FILES + rsSource() 聚合 + rsRead()），沿用 fe_sources.js 模式 | 与前端拆分同一套方法论；presence 断言在聚合文本上语义不变 |
| 命令注册守卫 | test_commands.js 扩展：`#[tauri::command]` 全量 ↔ generate_handler 名单 ↔ capabilities 三方互查 | 拆分后「定义了但忘注册」成为新的事故面，CI 直接红 |

## 2. 分项设计

### 2.1 发布流水线（`.github/workflows/release.yml`，PR-A）

```yaml
on:
  push:
    tags: ["v*"]
  workflow_dispatch:
    inputs:
      dry_run: { description: 只构建并上传运行工件，不创建 Release, type: boolean, default: true }
permissions: { contents: write }
concurrency: { group: release-${{ github.ref }}, cancel-in-progress: false }
jobs:
  release:
    runs-on: windows-latest
    timeout-minutes: 60
```

步骤序（全部 fail-closed）：

1. checkout（tag 触发时自动落在 tag 指向的提交）；
2. setup-node@v4（前端断言测试）+ dtolnay/rust-toolchain@stable + Swatinem/rust-cache@v2（workspaces: src-tauri）+ setup-python@v5（publish_release.py）；
3. `taiki-e/install-action@v2` 装 tauri-cli；
4. **版本一致性**（tag 触发时）：tag `v{X}` == Cargo.toml == tauri.conf.json == `{X}`，不符即失败（本机 release.bat 负责同步，这里兜底防「tag 与代码版本错位」）；
5. `cargo test` + `node scripts/run_all.js`；
6. `cargo tauri build`，环境注入 `TAURI_SIGNING_PRIVATE_KEY`（secrets）与可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；密钥缺失时构建照样成功但无 .sig → 下一步 latest.json 生成 fail-closed 拦住；
7. **收集产物 → bin/package**（pwsh）：bundle/nsis/*VER*.exe(.sig)、bundle/msi/*VER*.msi(.sig)、裸 exe 重命名 `niuma-timer-VER-portable.exe` + 其 .sig、`niuma_timer.pdb` 压成 `niuma-timer-VER-portable.pdb.zip`（tar -a）；
8. `publish_release.py --generate-notes-only` → 生成 RELEASE_NOTES.md / latest.json / SHA256SUMS.txt 并硬校验（latest.json 空 platforms 即失败，等价本地 :verify_latest）；
9. `dry_run=true`（dispatch 默认）：`actions/upload-artifact` 上传 bin/package 全量 → 结束，**不碰 Release**；
10. 正式路径：`publish_release.py --tag --version --package bin/package --repo ${{ github.repository }}`，token 经 env `GITHUB_TOKEN` → 草稿 → 资产 → 校验 → 公开 → 后验证（脚本既有语义）；
11. PDB best-effort：`publish_release.py --pdb-only`（失败只告警不失败，与本地语义一致）。

并发组不取消进行中的发布（cancel-in-progress: false——发布跑到一半被取消比慢更糟）。

**互斥说明**：release.bat 本机流程在推 tag 前已完整跑过 测试/构建/打包/元数据校验，推 tag 后 CI 重建是同提交的重复算力，换来的是「任何机器推 tag 都能发版」+「发布产物全部来自云端构建」。本机构建段保留作为 pre-flight（tag 推出去之前就把编译问题拦住），后续如嫌重复可再瘦身，本期不动。

### 2.2 `publish_release.py` 改动（PR-A）

`github_token()` 取值顺序改为：环境变量 `GITHUB_TOKEN` → `NIUMA_GITHUB_TOKEN` → Git 凭据管理器（原逻辑原样保留为回落）。其余零改动。`test_publish_release.py` 补环境变量取值路径的用例（monkeypatch 环境变量，断言优先级与回落）。

### 2.3 release.bat 瘦身（PR-A）

删除：[5/5] GitHub Release 段（`:publish_gh` / `:publish_py` / `:publish_browser` / `:upload_pdb` / gh 探测 / ASSETS 收集）。保留：版本同步、tauri-cli 与签名密钥前置检查（本地打包仍需要）、测试/构建/打包/产物校验/`--generate-notes-only` + `:verify_latest`（推 tag 前的 fail-closed 预演）、commit、tag、push。summary 段改为指向 Actions 运行页。README 双语的发布说明同步改写。

### 2.4 main.rs 模块化（PR-B）

新模块与内容（纯移动，不改逻辑）：

| 新文件 | 内容（自 main.rs 迁出） |
|---|---|
| `diag.rs` | panic_log_path / trace_startup / install_crash_log / build_info / show_fatal |
| `state.rs` | AppState + Default、current_monthly_workdays、get_status（字段 pub(crate)） |
| `cmds_core.rs` | load_config / save_config / refresh_holidays / get_status_cmd / hide_window / show_window / focus_window / write_debug_log / get_autostart / set_autostart / export_csv / apply_monitor_switches / apply_shortcuts |
| `cmds_bill.rs` | get_overtime_records / save_overtime_record / delete_overtime_record / get_bill / get_heatmap / get_trend / get_body_bill |
| `cmds_monitor.rs` | get_activity_summary / get_app_usage_summary / get_audio_usage_summary |
| `cmds_storage.rs` | get_storage_info / run_maintenance / backup_now / list_backups / restore_backup |
| `cmds_update.rs` | check_update / start_update / start_update_installed / start_update_portable / skip_update_version / take_update_announcement |
| `cmds_debug.rs` | test_offwork_notify / test_sedentary_notify / reset_remind_state / run_remind_tick / test_sedentary_trigger |

main.rs 保留：mod 声明、INVOKE_SHIM、apply_holiday_cache / spawn_holiday_refresh / refresh_tray / maybe_rollover_day / maybe_record_overtime_lock / toggle_pause（scheduler/tray 的接线面）、main() + setup 闭包 + generate_handler + run loop（约 430 行）。

兼容要点：

- main.rs 根模块 `pub(crate) use state::{AppState, get_status};` —— remind.rs / tray.rs 的 `crate::get_status`、`crate::AppState` 原样解析，**零调用点改动**；scheduler.rs 引用的三个 fn 本就留在 main.rs；
- generate_handler 里命令保持裸名（`use cmds_x::{...};` 引入），handler 名单文本不变，test_update.js 的注册断言不动；
- `#[tauri::command]` 的 invoke 路由按函数名注册（`generate_handler![fn_name]` 与来源模块无关），迁移不影响 IPC 契约与 capabilities。

### 2.5 build.rs 解析器升级（PR-B）

`command_names()` 拆为 `parse_commands_in(&str)`（现单文件逻辑原样抽出）+ 目录递归收集 `src/**/*.rs`（排序保证确定性）逐个解析合并；空名单 panic 的文案同步。新增 `println!("cargo:rerun-if-changed=src");`：本地改任意 .rs 后 build script 重跑，capabilities 及时补齐（CI 的漂移守卫本就覆盖此面，这条让本地也闭环）。build.rs 头注释与 L330 注释同步措辞。

### 2.6 测试适配（PR-B）

- 新增 `scripts/lib/rs_sources.js`：`RS_FILES`（src 顶层 .rs 排序清单）、`rsSource()`（按序拼接）、`rsRead(rel)`；风格对齐 fe_sources.js；
- `test_commands.js`：命令提取改扫 `rsSource()`；新增两条守卫——「每个 #[tauri::command] 都在 generate_handler 注册」（从 main.rs 解析 handler 名单）与既有「commands ↔ capabilities」互查并列；
- `test_build_info.js`：build_info() 定义断言改走 rsSource()；main.rs 落日志调用点断言仍读 main.rs（调用点留在 main()）；
- `test_guard.js`：pause_monitor 缺席断言扩到 rsSource()（更强）；
- `test_remind.js` / `test_shortcut.js`：mainSrc 换 rsSource()；
- `test_update.js`：main() 拦截顺序断言仍读 main.rs（顺序对在同一个 main() 里），命令定义断言换 rsSource()；
- `test_week_bill.js` / `test_insights.js` / `test_bill.js`：断言 mod 声明与 handler 登记，均在 main.rs 保留区，不动（实施时核实）。

### 2.7 文档（两个 PR 各自携带）

CHANGELOG.md 顶部加 `[未发布]` 段（publish_release.py 的 changelog_section 已有该兜底，发版时归位）；README 双语：发布说明（tag 推送 → CI 发布 + Secrets 前置）、项目结构树（src-tauri/src 新模块 + scripts/lib/rs_sources.js）、技术栈表 CI 行补 Release workflow。

## 3. 验证方式

- **PR-A**：`python scripts/test_publish_release.py` 全绿；workflow_dispatch dry_run 在 main 上跑绿，运行工件里能看到与本地 bin/package 同构的全套产物（含 .sig 与 pdb.zip）；不产生任何 Release。
- **PR-B**：`node scripts/run_all.js` 全绿；`cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` 全过；本地 `cargo build` 后 `git status` 干净（漂移守卫语义）；冒烟走查 debug 构建可启动。
- **合入后**：main 上两个 job 绿；发布 workflow 的 dry_run 绿。**不裁真 tag、不发真 Release**——首次真实发布留给下一次版本迭代，由用户执行。

## 4. 风险与回退

| 风险 | 缓解 |
|---|---|
| CI 构建环境（工具链漂移）与本地产物差异 | 同为 stable 工具链；产物名只含版本号不含工具链指纹；dry_run 先行演练 |
| Secrets 未配/密钥加密需密码 | dry_run 同样消耗 secrets，跑一次即暴露；缺失时 fail 点在 latest.json 生成（错误信息明确指向签名） |
| `taiki-e/install-action` 上游不可用 | 回退 `cargo install tauri-cli --locked`（workflow 注释给出替代行） |
| 双重发布竞态（本机误发 + CI 同时发） | release.bat 发布段删除后仅 CI 发；publish_release.py 幂等且拒绝改动已公开 release |
| 拆分引入行为差异 | 纯移动不改逻辑；编译器 + clippy + cargo test + run_all 四道网；单 PR 可整体 revert |
| 命令漏注册（新事故面） | test_commands.js 三方互查守卫 + build.rs 全目录解析 + rerun-if-changed=src |
| 测试断言误放行（聚合源弱化「在 main.rs」语义） | 顺序敏感与调用点断言保留 per-file 读取（main.rs），仅 presence 断言用聚合源 |
