# 技术债第三轮清偿 设计文档（spec）

> 状态：**执行中**——批次一已完成（提交 d9e7cf9），批次二待做。实施计划见 [2026-10-03-tech-debt-round3-plan.md](2026-10-03-tech-debt-round3-plan.md)。

## 背景

技术债排查第三轮（2026-10-03，接第一/二轮，二轮四批已随 v1.8.0/v1.8.1 清完）。本轮由四路 Explore 摸底产出候选清单（Rust 后端 / 前端 / 构建与 CI / 数据层），**全部关键条目已人工对源码逐条复核行号**，无偏差后才入本 spec。总体结论：代码库健康度高，仅 1 条高危（可达 panic），其余为鲁棒性、一致性与依赖陈旧问题。

## 范围总览（四批）

| 批次 | 主题 | 条目 |
|------|------|------|
| 一 | Rust 正确性与兼容 | timeline 钳制（高）、备份校验向前兼容（中）、purge 事务、三组收口、文档、开库反馈 |
| 二 | 发版链路 | PDB best-effort 落地、本地门禁补齐云端六步、workflow 工具钉版、REPO/GIT 收口 |
| 三 | 前端体验 | hero tick 留痕（中）、全局 rejection 兜底（中）、报告图标题随跨度（中）、前端低优七小件 |
| 四 | 依赖升级 | rusqlite 0.32→0.38、tauri 系小版本、png/dirs/zip 对齐树内大版本、reqwest 0.13 试升 |

## 条目明细（问题 / 证据 / 决策）

### 批次一：Rust 正确性与兼容

**D1（高）`get_day_timeline` offset 未设上限** — `cmds_bill.rs:107-109` 只做 `offset.max(0)`，offset 是前端 IPC 直达的 i64；传 ≥1e8 时 `today - Duration::days(off)` 越界令 chrono panic（异步命令 panic 被任务边界吞掉，前端 invoke 永不 resolve，时间线静默死亡且零日志）。与 v1.8.0 修掉的 `weekbill::period_bounds` 极值 panic 是同类漏网，也是最后一条。
**决策**：`offset.clamp(0, 1200)`（时间线按天翻，1200 天封顶，与 period_bounds 同量级），并加 JS 守卫断言防回归。

**D2（中）备份校验的向前兼容陷阱** — `backup.rs` validate_archive 要求解包库含 `db::BUSINESS_TABLES` 全部表，缺一即拒；而旧版本备份**合法地**没有后来新增的表（VACUUM INTO 快照）。后果：未来任何一次加表发版，升级后的用户还原任何旧备份都报「备份缺少业务表」且无绕过路径——回滚通道整体失效。
**决策**：表登记「引入版本」（`TABLE_SINCE`，与 BUSINESS_TABLES 做等价性测试钉住）。缺表仅当**能确定该备份本应含此表**（备份 app_version 可解析 ≥ 引入版本）才拒绝；老备份合法缺表、版本解析失败一律放行（沿用 backup.rs 既有注释哲学：「不做比做错（误拒合法备份）更糟」）。放行路径必须保证还原后 init_tables 补建空表。

**D3（低）`purge_before_conn` 多步写未包事务** — `maintain.rs:237-245` 八张表逐条 DELETE 各自 autocommit，中途失败留半截状态。
**决策**：`unchecked_transaction` 包整体，签名不变，失败零副作用等下轮重试。

**D4（低）`WEEKDAYS_CN` 三份逐字拷贝** — weekbill.rs:104 / insights.rs:18 / focus.rs:34，同 days_in_month 三份拷贝款。
**决策**：收口到 weekbill.rs 单一 `pub(crate)` 实现，insights/focus 改引用。

**D5（低）两个语义相反的 `to_min`** — `calc.rs:66` 解析失败静默返 0.0（坏配置如 `pm_end` 写坏时算钱路径全线失真且无日志）；`overtime.rs:215` 返 `Option` 显式报错。
**决策**：overtime 的 Option 版为单一实现（移入 calc.rs），overtime 引用；calc 侧调用方显式处理 None——记 debug.log 并回退配置默认值。行为变化：坏配置从「静默算错钱」变「回退默认 + 留痕」。

**D6（低）`db.rs:7` 模块文档失真** — 仍写 `holiday_cache.json`（按年缓存改造后已是 `holiday_{year}.json`），表清单手抄仅 3 张（实际 8 张）。
**决策**：文档改为按年缓存文件名；表清单段落指向 `BUSINESS_TABLES` 单一真相源，不再手抄。

**D7（低）开库失败零可见反馈** — `db.rs` 的 `Connection::open(...).expect` 与 `init_tables` 的 expect：库被杀软锁住/磁盘满/权限异常时进程静默退出，panic hook 只写 panic.log 不弹窗，用户侧表现为「双击无反应」。
**决策**：失败路径先 `diag::show_fatal`（现成机制，diag.rs:73）给人话弹窗再走 panic 留 panic.log；若 show_fatal 在 setup 前不可用，则初始化挪进 tauri setup 转 Err 走现成弹窗路径。二选一在实施时按 show_fatal 线程约束定，记录选择理由。

### 批次二：发版链路

**D8（中）PDB 上传失败把已成功的发布 run 标红** — `release.yml` 「Upload PDB (best-effort)」步骤注释自述「失败只告警不回退发布」（publish_release.py:612 同述），但步骤没有 `continue-on-error: true`，`publish_pdb_only` 失败返 1 会把整个发布 run 判红，诱导人去「补救」一个没坏的发布。
**决策**：步骤加 `continue-on-error: true`；`publish_release.py` 保持返 1 不动（本地应急路径需要真实退出码）。结构断言进发布引擎测试。

**D9（中）本地发版门禁只有云端六步的 4/6** — `release.bat` 第 0 步只跑 `build.bat test`（cargo test + 前端断言 + 两个 Python 套件）；`cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo deny check advisories` 只在 tag 推上去后的云端跑。tag 不可变——云端红=版本号已消耗，只能人肉 bump 重打，「零手动发版」承诺的破口。
**决策**：三步按 tests.yml **逐字同 flag** 补进 `build.bat :do_test`；cargo-deny 本地缺失时 fail-closed 报错并给安装指引（CI 每次都跑，本地缺就装一次）。新增 `scripts/test_local_gate.js` 守结构。

**D10（中）workflow 工具未钉版** — tests.yml 的 `cargo-deny` 与 release.yml 的 `tauri-cli` 都经 `taiki-e/install-action@v2` 装**最新版**，而本地钉 `--version "2"`。上游发大版本（cargo-deny 改 deny.toml schema / tauri-cli 3 转正）会让 CI 隔夜变红、本地依旧绿。
**决策**：install-action 支持版本钉定——cargo-deny 钉当前稳定精确版，tauri-cli 钉大版本 `@2`，与本地策略一致。

**D11（低）REPO/GIT 硬编码多处真相源** — release.bat:25 `REPO`、:35 `GIT` 写死系统 Git 路径（回退 PATH 时可能命中注释自述凭据管理器段死的 WorkBuddy PortableGit）、scripts/push_via_api.py:36 REPO 再写一份。
**决策**：GIT 解析加 PortableGit 校验（命中即报错说明原因）；REPO 两处加交叉注释并由 test_local_gate.js 断言字面量一致。

### 批次三：前端体验

**D12（中）`hero.js:48` tick() 的 catch 全静默** — 连 flog 都没有。`get_status_cmd` 持续失败时主界面大数字/徽章/时间轴/副标题全部冻结在旧值、日志零痕迹。是 v1.8.0 修的四处 invoke 静默失败之后漏网的**第五处**。
**决策**：catch 内至少 flog；视实施复用活动条失败 toast 的 2 秒去重模式给可见反馈（最小实现只做 flog）。

**D13（中）主窗口缺 `unhandledrejection` 兜底** — core.js:35 只挂了 `window error`；hover_card.html:424 反而两个都有。具体暴露点：`saveBillImage`（bill.js:405-452）try/finally 无 catch，`buildReportModel`/`drawReport` 抛错即成无人处理的 rejection——按钮复位但无 toast 无日志。
**决策**：core.js 补 unhandledrejection → flog（对齐 hover_card 既有实现）；saveBillImage 补 catch 给 toast + flog。

**D14（中）月跨度报告图标题穿帮** — `bill.js:302` 标题写死「牛马计时器 · 周账单」，而月跨度允许导出图片（仅年跨度置灰）；图内金句已按跨度换「本月」，标题不改晒图即矛盾。
**决策**：标题按 model 跨度映射「周账单/月账单」；test_report_image.js 补断言。

**D15（低）前端低优七小件**：
1. px→距离换算两套（monitor.js `fmtDist` vs insights.js:328 `fmtMeters`，单位与精度口径分裂）→ 收口 core.js 单一 formatter；
2. 时长格式化双份（monitor.js `fmtDurCN` vs insights.js `tlDur`，措辞「分钟/分」不一）→ 收口 core.js 统一「分钟」；
3. 摸鱼金句阈值表双份（monitor.js `SLACK_QUIPS` vs bill.js `WEEK_BILL_QUIPS` 同名文案两处字面量）→ 重叠文案收 core.js 共享；
4. insights.js 四个 `*Data` 「缓存」变量是死状态+误导注释 → 降局部/清除；
5. hover_card.html `startQuips` 每次悬停新增未登记的 3s setTimeout，短悬停反复进出堆积到期回调 → 登记句柄并在 stopQuips/hideCard 清除；
6. monitor.js `appIconHTML` 的 `src` 插值是全项目唯一未转义的后端字符串拼进 HTML 属性（当前是本地 data URL 无实险）→ 过 escapeHtml，堵住未来接入非受控来源时的属性注入点；
7. `test_frontend_split.js` 契约盲区：不校验 index.html 是否多出清单外 `<script>`、js/ 目录是否有未入 CHUNKS 的新文件 → 补「js/ 目录 .js 集合 == CHUNKS」双向断言。

## 非目标（维持不动，勿顺手做）

- **老跳过项三条**（用户点名才启动，本轮核实均仍在且有小恶化，记录在案）：金额格式化约 17 处内联（实证口径漂移：主界面 `¥123.45` vs 悬停卡 `¥123`）；hero/hover_card 时间轴双份渲染（已变三份 `ov2`，且配置生效时机分叉）；设置页绑定仍在 monitor.js（加班明细按钮还寄宿 storage.js）。
- P5 全局 DB Mutex 改 writer 线程（中期项，现状未恶化）。
- update.rs/win.rs 模块拆分（二期裁决：仅用户明确发话后启动）。
- CSP / unsafe-inline（前端内联 script 依赖，紧不了）；`.gitattributes`（用户已否决，\r 归一化兜底已覆盖）。
- reqwest 0.12/0.13 双栈**合并**（受 tauri-plugin-updater 制约；本轮只做自家直接依赖的版本对齐与记档）。
- 前端引入构建器/框架/TypeScript（架构既定）。

## 发版策略

- 四批攒一个版本 **v1.8.2**，批次间无耦合、各自可独立发版；依赖升级批次刻意放最后（风险隔离），其受阻不阻塞前三批发版。
- CHANGELOG v1.8.2 段开头放用户视角摘要（v1.8.1 起的惯例）；README 双语对齐走 test_readme.js 守卫。
- 用户可见的行为变化仅 D5（坏配置回退+留痕）与 D14（标题修复），其余为鲁棒性与链路加固——适合 patch 版。

## 风险与规避

| 风险 | 规避 |
|------|------|
| rusqlite 0.38 bundled SQLite C 代码在本机 VS 18 链路编译（历史 D8050 机器态） | build.bat 自持 TMP 已根治；仍闪断按既案 `-j1` 重试 |
| zip 2→4 API 变更（backup.rs 是唯一使用点） | 单独小步升，cargo check 驱动调整；失败可回退单包 |
| build.bat test 加 clippy 后本地门禁变慢 | 仅发版前跑，可接受；CI 有缓存不受影响 |
| D2 放行语义与既有「缺表即拒」测试冲突 | 按新语义更新既有测试并在断言注明向前兼容决策 |
| bat 改动在 Git Bash 复现不出问题 | 必须在真实 cmd 终端验证（common.bat 中文注释事故的教训） |
