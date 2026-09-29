# 设计：自动更新体验优化（更新说明渲染 · 下载进度与重试）

日期：2026-09-29
状态：已与用户逐节确认，待批准执行
关联：

- `docs/superpowers/specs/2026-09-24-v1.4.0-auto-update-design.md`（本设计是它的**增量优化**，不改其任何架构决策）
- 现有实现：`src-tauri/src/update.rs`、`src-tauri/src/main.rs`、`frontend/app.js`、`frontend/index.html`、`frontend/styles.css`
- 前置产物：`scripts/publish_release.py` 的 `latest.json`（`notes` 为 CHANGELOG markdown 原文）、`src-tauri/build.rs` 的 `UPDATE_NOTES` 注入

## 0. 背景与目标

用户在 v1.4.x 实际使用中反馈两个问题（原话）：

> 1. 更新说明里会出现 markdown 原始字符，例如标题的 `###`
> 2. 点击更新按钮，最好出现进度条吧，现在是点击更新了，界面直接消失了，下载慢的话，我都不知道什么情况

目标：

- **G1** 更新说明按 markdown 语义渲染成可读排版，不再出现 `###` / `-` / `**` 原文。
- **G2** 两条更新路径（安装版 / 绿色版）都显示**真实下载百分比**，并能看出「重试中 / 校验中 / 安装中」。
- **G3** 网络中断自动重试（带续传）；彻底失败时**主程序窗口仍在**，给出失败原因，并允许再次点击重试。

## 1. 现状与根因（已查实）

| # | 现象 | 根因（代码位置） |
|---|---|---|
| 1 | 更新说明是 markdown 原文 | `app.js` 用 `textContent` 把 `notes` 直插 `<pre id="updNotes">`；全仓库无任何 markdown 渲染库；原设计文档明写「等宽渲染 Markdown 文本」，即**从未规划渲染** |
| 2 | 无进度、界面直接消失 | 点「立即更新」只 `invoke("start_update")` 后写死文案，**不订阅任何事件**；`start_update` 里安装版 `app.restart()`、绿色版 `app.exit(0)` 直接终止/重启进程；`download_and_install(\|_, _\| {}, \|\| {})` 的进度回调是**空闭包**，进度被丢弃；绿色版助手用 `reqwest::blocking ... bytes()` **一次性读全部字节**，结构上无分块回调 |

## 2. 硬约束（已查实）

| 约束 | 含义 |
|---|---|
| `tauri-plugin-updater` 的 `download()` **不暴露断点续传** | 安装版重试只能从头重下，属插件能力边界 |
| Windows 上 `Updater::install()` 内部即 `std::process::exit(0)` | 交给 NSIS 安装器后重启由安装器负责；`install()` 之后的 `app.restart()` **不可达**（现状是死代码） |
| 运行中的 exe 不可覆盖（可改名） | 绿色版替换仍必须由**独立助手进程**在主程序退出后完成，助手不可移除 |
| 绿色版现有顺序是「主程序先退出 → 助手再下载」 | 主程序一旦退出就再也无法展示下载进度，**必须调换顺序**才能有真进度 |
| `core:event:default` 已在 `capabilities/default.json` | 新增事件通道**不需要**改权限清单 |
| `build.rs` 会按内容指纹自动同步缓存戳 | 前端改动后**不需要**手工改 `?v=` / `FE_VER`，但验收前必须跑过一次真实构建 |

## 3. 决策表

| 决策点 | 结论 |
|---|---|
| 更新说明呈现 | **轻量富文本渲染**（自研约几十行的 `renderMarkdown`），不引入 marked / markdown-it |
| 渲染实现位置 | `frontend/app.js` 顶层纯函数（字符串进、HTML 出，不碰 DOM），便于沿用现有 `vm` 抽取式测试 |
| 链接处理 | 渲染为**不可点击**的 `<span class="upd-link" title="url">`；不让 webview 被导航劫持，且本项目版本段落里目前没有链接 |
| 进度承载方式 | 新增 Tauri 事件 `update-progress`，payload `{ phase, downloaded, total, attempt }`；文案由前端组装 |
| 进度精度 | **两条路径都是真百分比**：安装版接插件 `on_chunk` 回调；绿色版调换顺序为「主程序带进度下载 → 本地交接给助手」 |
| 绿色版字节交接 | 主程序下载并校验后落盘 `<exe>.download`，起助手时传 `--local-file`；**助手替换/回滚/重启状态机一行不改** |
| 重试策略 | 3 次尝试 + 指数退避（1s、3s）+ `Range` 续传（绿色版下载路径） |
| 安装版重试 | 同样 3 次 + 退避，但**无法续传**（插件能力边界），重试即从头下 |
| 校验时机 | 绿色版：主程序**先校验 SHA256 再落盘**，不过关直接返回 Err、进程不退出；助手仍**独立复核一次**（它可能被单独调用） |
| 失败表现 | 安装版/绿色版的下载与校验失败都**在窗口内报错**、重新启用按钮，不再出现「进程已退出但什么都没说」 |
| 实现范围 | 只动上面列出的 5 个文件；不重构助手状态机、不改发布链、不改检查调度 |

## 4. 更新说明渲染（前端）

### 4.1 `renderMarkdown(text) -> html`

在 `frontend/app.js` 顶层新增（紧邻 `paintUpdate()`），**纯字符串处理**：

1. 先整体 HTML 转义（`&` `<` `>` `"`）；
2. 再按行做**块级**识别：`## ` → `<h4>`、`### ` → `<h5>`、`- ` / `* ` → `<ul><li>`、`1. ` → `<ol><li>`、``` 围栏 → `<pre><code>`、空行分段；
3. 最后对行内做替换：`**粗体**` → `<strong>`、`` `代码` `` → `<code>`、`[文字](url)` → `<span class="upd-link" title="url">文字</span>`。

因为转义在前、且**唯一会出现的标签都是自己生成的**，输出可直接 `innerHTML`。输入（CHANGELOG）仍按不可信内容处理。

### 4.2 接入点

- `frontend/index.html`：`<pre id="updNotes">` 改为 `<div id="updNotes" class="upd-notes">`（id 不变）。
- `frontend/app.js`：`paintUpdate()` 里两处 `$("updNotes").textContent = ...` 改为 `innerHTML = renderMarkdown(...)`，覆盖「远端 notes」与「升级后公告 `updateAnnounce`」两条路径。
- `notes` 为空时仍走「暂无更新说明」纯文本分支，不经过渲染器。
- `frontend/styles.css`：`.upd-notes` 增加富文本排版（标题字号与间距、列表缩进、`code` 底色）；`.upd-notes pre` **保留 `white-space: pre-wrap`**，既保证代码块换行，也让 `scripts/test_update.js` 既有断言继续成立。

## 5. 进度事件协议

事件名：**`update-progress`**

```json
{ "phase": "downloading", "downloaded": 4210000, "total": 9300000, "attempt": 1 }
```

- `phase` ∈ `downloading` / `verifying` / `installing` / `restarting` / `retrying`。
- 文案由**前端**组装（Rust 只发数字与 phase），文案映射见 §9.3。
- `update.rs` 新增 `pub fn emit_progress(app: &AppHandle, phase: &str, downloaded: u64, total: Option<u64>, attempt: u32)`，内部 `app.emit("update-progress", payload)`。
- 前端复用现有 `TAURI.event.listen` 通道（与 `win-visibility` 同一条路），由 `core:event:default` 覆盖，**不动 `capabilities/default.json`**。
- **节流**：下载中最多每 120ms 或每变化 ≥1% 发一次（调用方在 `main.rs` 用局部 `Instant` + 上次百分比包一层），避免每 64KB 一次 IPC。

## 6. 下载与重试：`Fetcher` + `download_with_retry`

沿用本项目「窄 IO trait + 单测注入假实现」的既有做法（同 `HelperIo`）。

### 6.1 `trait Fetcher`

```rust
/// 把 url 的内容追加到 buf；内部按 buf.len() 决定是否带 Range 续传；
/// 服务端忽略 Range（200）时自行 clear 后重下。每块回调 on_chunk(已累积长度, 总长度)。
pub trait Fetcher {
    fn fetch(
        &self,
        url: &str,
        buf: &mut Vec<u8>,
        on_chunk: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(), String>;
}
```

`RealFetcher` 用 reqwest blocking 的 `std::io::Read` 逐块读（`reqwest::blocking::Response` 本身就实现 `Read`），**不新增依赖**（沿用现有 `blocking` feature，不加 `stream`）：

- client：`connect_timeout(15s)` + `timeout(300s)`（单次尝试上限；慢链路会被切成多段，由续传接上）；
- `buf` 非空时带 `Range: bytes=<buf.len()>-`；
- 响应 `206` → 续写；响应 `200` → `buf.clear()` 后重下；
- `total = buf.len() + resp.content_length()`（206 时 `Content-Length` 是本段长度，相加才是全量）；
- 分块 64KB，每块 `on_chunk(buf.len(), total)`。

### 6.2 `download_with_retry`

```rust
pub const DOWNLOAD_ATTEMPTS: u32 = 3;

pub fn download_with_retry<F: Fetcher>(
    fetcher: &F,
    url: &str,
    on_progress: &mut dyn FnMut(u64, Option<u64>, u32), // downloaded, total, attempt
    on_retry: &mut dyn FnMut(u32, &str),                // 即将开始的次数, 上次错误
    pause: &mut dyn FnMut(u64),                         // 退避 sleep（可注入，单测秒级跑完）
) -> Result<Vec<u8>, String>
```

- `buf` **跨尝试保留**：中途报错时已读字节仍在 `buf` 里，下次尝试自动从断点续传。
- 退避 1s、3s；每次重试前回调 `on_retry`（供 UI 显示「正在重试（第 N 次）」）。
- 3 次仍失败 → `Err`，错误信息含尝试次数与最后一次原因。
- 读到流结束但字节数少于 `total` 视为失败（进入下一次尝试）。

## 7. 安装版更新流程

`main.rs::start_update` 安装版分支改写为：

1. `update::emit_progress(app, "downloading", 0, None, 1)`；
2. `updater.check()` → 无更新返回 `Err("已是最新版本")`；
3. 用 `download_with_retry` 同款退避循环包裹 `pending.download(on_chunk, on_finish)`：
   - `on_chunk(_, total)` 累加并节流发 `downloading`；
   - `on_finish` 发 `verifying`；
   - 插件不暴露续传，重试即从头下（文档中显式记录为能力边界）；
4. 三次失败 → 返回 `Err`（**窗口还在**，前端显示原因）；
5. 成功 → 发 `installing` → `update.install(bytes)`。

**顺带清理**：删掉 `install()` 之后的 `app.restart()` —— 插件在 Windows 上内部即 `std::process::exit(0)` 交给安装器，重启由安装器负责（[updater.rs:882](https://docs.rs/tauri-plugin-updater)）。因此安装版「安装中」之后窗口消失是**预期行为**，但此时用户已看完整段下载进度，原始抱怨不再成立。

## 8. 绿色版：主程序下载 + 本地交接

### 8.1 顺序调换（核心改动）

```
旧：主程序 fetch 清单 → 起助手(传 url) → app.exit(0) → 助手等退出 → 助手下载 → 替换
新：主程序 fetch 清单 → 带进度下载(重试+续传) → 校验 SHA256 → 落盘 <exe>.download
      → 起助手(传 --local-file) → app.exit(0) → 助手等退出 → 读本地 → 复核 SHA256 → 替换
```

收益：下载这件不可靠的事回到**有 UI 的进程**里；下载失败/校验失败都在窗口内可见（进程不退出）；「无窗口期」从「下载＋替换」缩短为「仅替换」。

### 8.2 数据结构改动（`update.rs`）

```rust
/// 助手取字节的来源
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Url(String),   // 兼容既有路径
    Local(String), // 主程序已下载并校验的暂存文件
}

pub struct HelperArgs {
    pub target: String,
    pub wait_pid: Option<u32>,
    pub source: Source,   // ← 原 `url: String`
    pub sha256: String,
}
```

- `parse_helper_args`：`--url <u>` → `Source::Url`，`--local-file <p>` → `Source::Local`。
- `build_helper_args(target, wait_pid, url, sums)` 保持签名（内部转成 `Source::Url`），为绿色版新流程新增 `build_helper_args_local(target, wait_pid, path, sha256)`。
- 哈希解析复用现有 `find_hash_in_sums(sums, file_name)`，主程序先用它取出期望哈希、下载后自行比对。
- `HelperIo::download(&self, url: &str)` → `read(&self, source: &Source)`；`RealHelperIo` 的 `Local` 分支就是 `std::fs::read(path)`。
- **`run_helper_flow` 的「等退出 → 校验 → 写 .new → 备份 → 顶替 → 拉起 → 回滚 → 清理」状态机一行不改**，只换「取字节」的来源。

### 8.3 主程序侧流程

1. 拉清单 → `platform_url(PORTABLE_KEY)` → 版本比较 → 取 `SHA256SUMS.txt` 里该资产的哈希；
2. `download_with_retry(&RealFetcher, &url, ...)` 带进度下载（`downloading` / `retrying`）；
3. 发 `verifying`，`sha256_hex(&bytes)` 比对；**不过关直接 `Err`，不落盘、进程不退出**；
4. 写暂存文件 `<exe>.download`（与目标同卷，便于助手原子改名）；
5. `build_helper_args_local` → `Command::new(exe).args(["--apply-update", "--target", ..., "--wait-pid", ..., "--local-file", ..., "--sha256", ...])` spawn；
6. 发 `installing` → `app.exit(0)`。

## 9. 前端进度条 UI

### 9.1 DOM（`frontend/index.html`）

保留 `id="updProgress"`（既有断言不破），由纯文本条升级为进度组件：

```html
<div id="updProgress" class="upd-progress hidden" role="status" aria-live="polite">
  <div class="upd-progress-track"><div id="updProgressFill" class="upd-progress-fill"></div></div>
  <p id="updProgressText" class="upd-progress-text"></p>
</div>
```

样式（`frontend/styles.css`）：`.upd-progress-fill` 宽度带 0.2s 过渡；无 `total` 时容器加 `indeterminate` 类走循环扫光动画。

### 9.2 状态与重绘（`frontend/app.js`）

- 模块级 `updateProgress = { phase, downloaded, total, attempt }`；
- 新增 `watchUpdateProgress()`：`TAURI.event.listen("update-progress", ...)`，与 `watchUpdateView()` 同级挂到现有冷启动流程；
- `showView("viewUpdate")` 时按最新状态重绘 → **下载中切走再切回来不丢进度**；
- 有 `total` → `fill.style.width = pct + "%"`；无 `total` → `indeterminate`。

### 9.3 文案映射（前端组装）

| phase | 文案 |
|---|---|
| `downloading` | 正在下载… 45%（4.2 / 9.3 MB） |
| `retrying` | 网络中断，正在重试（第 2 次）…（进度条停在上次位置，不归零） |
| `verifying` | 下载完成，正在校验完整性… |
| `installing` | 安装版：正在安装，安装程序将自动关闭本程序…／绿色版：正在替换程序文件，即将自动重启… |
| `restarting` | 正在重启… |

`installing` 的两套文案由前端依据既有 `updateInfo.installed`（`loadUpdateInfo()` 已带回）选择，**不由 Rust 下发**。

### 9.4 「立即更新」点击处理

`app.js` 的 `updUpdateBtn` 处理改写为：显示进度区（不确定态）→ `invoke("start_update")`；**成功后不再写死文案**（后续全由事件驱动），失败保留现有错误文案并重新启用按钮；进入下载后按钮保持 disabled。

## 10. 测试影响

### 10.1 Rust（`update.rs` 的 `#[cfg(test)]`，沿用现有假 IO）

1. `download_with_retry`：第 1 次中途失败 → 第 2 次以 206 从断点续传 → 字节结果正确且 `on_retry` 被调用一次。
2. 服务端回 200 忽略 Range → 缓冲被清空重下，结果不出现重复前缀。
3. 3 次全失败 → 返回 `Err`，信息含重试次数。
4. `parse_helper_args` 对 `--url` 与 `--local-file` 都解析出正确的 `Source`。
5. `run_helper_flow` 以 `Source::Local` 走通「读本地 → 校验 → 备份 → 替换 → 重启」，退出码 0。
6. 主程序侧 SHA256 比对不符 → 返回 `Err`，且**不写** `<exe>.download`。
7. 助手侧复核不符（`Source::Local` 内容被篡改）→ **不替换**、退出码 1。
8. 既有 `HelperArgs` 构造点随 `source` 字段改名同步更新。

### 10.2 前端（Node 断言脚本，沿用 `test_update_ui.js` 的 `fn()` 抽取 + `vm` 手法）

- 新增：抽取 `renderMarkdown` 跑语义断言 —— `### x` → `<h5>`、`- x` → `<li>`、`**x**` → `<strong>`、`` `x` `` → `<code>`、`<script>` 转义为 `&lt;script&gt;`。
- 新增：`update-progress` 事件 handler 对 `{phase:'downloading',downloaded,total}` 能算出百分比与宽度。
- 保留既有断言：`id="updProgress"`、`id="updNotes"`、`invoke("start_update")`、`white-space: pre-wrap;`、`.upd-version-number` 字号等。
- 汇总入口：`node scripts/run_all.js` + `cargo test`。

## 11. 验收与手测清单

需要真实发布与网络，由用户执行：

- [ ] 安装版：点更新 → 看到 0→100% 与「校验中／安装中」→ 窗口关闭 → 安装完成后自动拉起新版本。
- [ ] 绿色版：点更新 → 真进度；限速/断网时出现「重试中」，恢复后继续；最终替换并重启。
- [ ] 断网到底：3 次失败后**窗口仍在**、显示原因、按钮可再点；程序仍是旧版本且可正常使用。
- [ ] 更新说明：标题／列表／加粗／行内代码正常显示，不再出现 `###`、`-`、`**` 原文。
- [ ] 下载中切到设置页再切回更新页，进度不丢。

## 12. 风险与降级

| 风险 | 应对 |
|---|---|
| GitHub 资产不支持 `Range`，续传退化为重下 | 代码同时处理 200/206；回 200 即从头下，逻辑仍正确，只是浪费已下字节 |
| 300s 单次尝试上限把慢链路切断 | 续传接上，净效果是多分几段；3 次尝试合计约 15 分钟传输预算 |
| 主程序下载后落盘 `<exe>.download` 残留 | 下次更新直接覆盖；助手成功后不在约定的清理范围内，不构成功能问题（如需可在后续迭代清理） |
| 绿色版交接引入回归 | 助手替换状态机**完全不动**；`Source::Url` 分支保留既有行为与测试 |
| 删除不可达的 `app.restart()` 影响他处 | 全仓库仅 `start_update` / `restore_backup` 两处调用 `restart()`，后者不动 |

## 13. 明确不做（范围边界）

- 更新说明**不分页、不折叠、不加摘要**；不引入任何 markdown 库。
- 不重构助手替换状态机、不改发布链、不改「每 6h 检查」调度。
- 不新增更新策略（双通道、差分更新、强制更新等）。
- 不做 Authenticode 代码签名（沿用既有裁决：暂不动）。
- 不清理历史遗留的 `<exe>.download`（见 §12）。

## 14. CHANGELOG（`[未发布]` 区，建议条目）

```
### 修复

- 更新说明不再显示 markdown 原始字符（`###` / `-` / `**` 等），改为正常排版。

### 优化

- 更新时显示真实下载进度与百分比，可看出「重试中 / 校验中 / 安装中」当前阶段。
- 下载中断自动重试并续传；彻底失败时窗口不再消失，可直接看到原因并重试。
```
