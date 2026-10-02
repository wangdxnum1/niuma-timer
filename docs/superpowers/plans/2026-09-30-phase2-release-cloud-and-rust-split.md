# 实施计划：二期——发布流水线上云 + Rust 侧模块化

> **状态（2026-10-02 补记）：已完成并合入 main**（PR-A 发布上云 / PR-B Rust 模块化，各自 CI 绿后合并）。勾选框保留原样作为执行记录；build.bat / common.bat 的后续加固属 v1.8.0 范围。

设计：`docs/superpowers/specs/2026-09-30-phase2-release-cloud-and-rust-split-design.md`
顺序：两个 PR（A 上云 → B 拆分），各自 CI 绿后合并；docs 先行提交到 main。

## Task 1: publish_release.py 环境变量 token（PR-A）

**Files**: scripts/publish_release.py、scripts/test_publish_release.py

- [ ] `github_token()` 取值顺序改为 env `GITHUB_TOKEN` → env `NIUMA_GITHUB_TOKEN` → Git 凭据管理器（原逻辑不动）
- [ ] test_publish_release.py 补用例：env 命中优先、`GITHUB_TOKEN` 优先于 `NIUMA_GITHUB_TOKEN`、两个 env 都缺时回落凭据管理器（monkeypatch 隔离）
- [ ] `python scripts/test_publish_release.py` 全绿（该文件不在 run_all.js 覆盖内，手动跑）

**Done**: python 测试全绿，本机无 env 时行为不变。

## Task 2: release.yml（PR-A）

**Files**: .github/workflows/release.yml（新建）

- [ ] 触发：push tags `v*`；workflow_dispatch + `dry_run` 输入（默认 true）
- [ ] `permissions: contents: write`；`concurrency: release-<ref>`，cancel-in-progress: false；`timeout-minutes: 60`
- [ ] job 步骤按设计 §2.1 步骤序 1-11 落地：checkout / setup-node / rust-toolchain / rust-cache / setup-python / taiki-e/install-action(tauri-cli) / 版本一致性（仅 tag 触发）/ cargo test / run_all.js / cargo tauri build（secrets 注入签名）/ pwsh 收集产物 / --generate-notes-only + latest.json 校验 / dry_run→upload-artifact 或 正式发布 + --pdb-only
- [ ] 版本一致性：tag 形如 `vX.Y.Z` 且 X.Y.Z == Cargo.toml == tauri.conf.json；dispatch 时跳过
- [ ] pwsh 收集逻辑对齐 build.bat `:do_package`（版本号过滤、portable 重命名、.sig 复制、PDB tar -a 压缩、triple 两种路径形态）
- [ ] 正式发布步骤 env 注入 `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`、`NIUMA_PROXY: off`、`--repo ${{ github.repository }}`
- [ ] YAML 语法本地校验（python -c yaml.safe_load 或等价）

**Done**: release.yml 语法通过、步骤与设计一致。

## Task 3: release.bat 瘦身（PR-A）

**Files**: release.bat

- [ ] 删除 [5/5] 段：gh 探测、ASSETS 收集、`:publish_gh` / `:publish_py` / `:publish_browser` / `:upload_pdb` / `:release_fail`
- [ ] 步骤条改为 0-4（测试/构建/打包/commit/tag+push）；summary 指向 `https://github.com/wangdxnum1/niuma-timer/actions`（CI 发布进度）
- [ ] 保留：签名密钥前置检查、tauri-cli 安装、元数据 `--generate-notes-only` + `:verify_latest`（推 tag 前的 fail-closed 预演）
- [ ] 头部注释补一句：tag 推送后由 GitHub Actions 构建并发布；应急手动发布命令一行示例

**Done**: release.bat 语法完整（bat 无本地解析器，以逐段核对 + 实跑 --help 级路径为准），发布职责全部移交 CI。

## Task 4: 文档（PR-A）

**Files**: CHANGELOG.md、README.md、README.zh-CN.md

- [ ] CHANGELOG.md 顶部新增 `[未发布]` 段：发布流水线上云 + release.bat 瘦身
- [ ] README 双语：`release.bat` 说明改为「构建+打包+打 tag+推送，发布由 CI 完成」；技术栈/CI 部分补 Release workflow；新增前置：GitHub Secrets 配 TAURI_SIGNING_PRIVATE_KEY（+可选 PASSWORD）

**Done**: test_readme.js 全绿，双语一致。

## Task 5: PR-A 交付（secrets → push → CI → merge → dry-run）

- [ ] 用 Git 凭据管理器的令牌（`git credential fill`，不回显）+ `gh secret set` 配置 `TAURI_SIGNING_PRIVATE_KEY`（读 `~/.tauri/niuma-timer.key`；密钥首行注释含 encrypted 时再配 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 并向用户确认密码来源）
- [ ] docs 提交推 main（spec + plan，避免混入 PR diff）
- [ ] `git checkout -b ci/release-cloud`；提交 release.yml + publish_release.py + test_publish_release.py + release.bat + CHANGELOG + README；push
- [ ] 开 PR（gh 用 GH_TOKEN=credential-fill 的令牌，或 compare 页手动）；CI 两 job 绿
- [ ] 合并 PR-A；main 拉平
- [ ] **dry-run 验证**：API dispatch release.yml（workflow_dispatch, dry_run=true, ref=main），watch 至绿；核对运行工件包含 setup exe / msi / portable exe / 各 .sig / pdb.zip / latest.json / SHA256SUMS.txt
- [ ] 确认未创建任何 Release（releases 页无新条目）

**Done**: dry-run 绿、工件齐全、零 Release 副作用。

## Task 6: 测试基建适配（PR-B）

**Files**: scripts/lib/rs_sources.js（新建）、scripts/test_commands.js、test_build_info.js、test_guard.js、test_remind.js、test_shortcut.js、test_update.js

- [ ] rs_sources.js：RS_FILES（src-tauri/src 顶层 .rs 排序清单）、rsSource()（拼接）、rsRead(rel)；风格对齐 fe_sources.js
- [ ] test_commands.js：命令提取换 rsSource()；新增「每个命令都在 main.rs 的 generate_handler 注册」守卫（从 main.rs 解析 handler 名单）
- [ ] test_build_info.js：build_info() 定义断言换 rsSource()；main() 落日志调用点断言保留 main.rs 读取
- [ ] test_guard.js：pause_monitor 缺席断言换 rsSource()
- [ ] test_remind.js / test_shortcut.js：mainSrc 换 rsSource()
- [ ] test_update.js：main() 拦截顺序断言保留 main.rs；命令定义断言换 rsSource()
- [ ] 核实 test_week_bill.js / test_insights.js / test_bill.js 断言全部落在 main.rs 保留区（mod 声明 + handler 登记），不动
- [ ] 此时（拆分前）run_all.js 全绿——纯换读源，语义不变

**Done**: 拆分前全部测试先绿（读源适配零风险落地）。

## Task 7: build.rs 解析器升级（PR-B）

**Files**: src-tauri/build.rs

- [ ] `command_names()` 拆出 `parse_commands_in(&str)`（现逻辑原样），外层递归收集 src/**/*.rs（含未来子目录，排序确定序）逐文件解析合并
- [ ] 新增 `println!("cargo:rerun-if-changed=src");`
- [ ] 空名单 panic 文案与头注释、L330 注释同步措辞
- [ ] `cargo build` 触发 build script 重跑，capabilities 无 diff（git status 干净）

**Done**: cargo build 通过、无回写漂移。

## Task 8: main.rs 模块化（PR-B）

**Files**: src-tauri/src/main.rs + 新建 diag.rs / state.rs / cmds_core.rs / cmds_bill.rs / cmds_monitor.rs / cmds_storage.rs / cmds_update.rs / cmds_debug.rs

- [ ] 按设计 §2.4 表格逐块移动（纯移动不改逻辑；每步 `cargo check`）
- [ ] state.rs 字段 pub(crate)；跨模块被调 fn pub(crate)
- [ ] main.rs：mod 声明 + `pub(crate) use state::{AppState, get_status};`（scheduler/tray/remind 零改动）+ `use cmds_x::{...};` 保持 generate_handler 裸名
- [ ] 拆完后 main.rs ≤ 500 行（目标 ~430），8 个新模块各自单一域
- [ ] `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + `cargo test` 全过
- [ ] `node scripts/run_all.js` 全绿（Task 6 已适配）
- [ ] 本地 `cargo build` 后 `git status` 干净（漂移守卫语义）

**Done**: 四道网全绿，行为等价（纯移动）。

## Task 9: 文档 + PR-B 交付

- [ ] CHANGELOG `[未发布]` 段补：main.rs 模块化 + build.rs 全目录解析
- [ ] README 双语项目结构树：src-tauri/src 新模块清单 + scripts/lib/rs_sources.js
- [ ] `node scripts/test_readme.js` 绿
- [ ] `git checkout -b refactor/rust-modularize`；提交；push；开 PR；CI 两 job 绿
- [ ] 合并 PR-B；main 拉平；本地 run_all + cargo 三件套终验
- [ ] 收尾汇报：两个 PR 链接、dry-run 结果、Secrets 配置状态、首次真实发布的操作方式

**Done**: main 处于 CI 全绿 + 全部测试绿 + 发布流水线就绪状态。
