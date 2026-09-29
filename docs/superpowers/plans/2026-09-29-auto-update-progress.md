# 自动更新体验优化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** G1 更新说明按 markdown 正常排版（不再出现 `###` 等原始字符）；G2 安装版与绿色版两条更新路径都显示真实下载进度与百分比；G3 下载中断自动重试（指数退避），彻底失败时窗口不再消失、能直接看到原因。

**Architecture:** 前端自研轻量 `renderMarkdown`（先整体 HTML 转义再还原有限标记，不引入库），并新增进度条组件、订阅后端 `update-progress` 事件。后端拆两条路径：安装版改用官方 updater 的 `download` / `install` 分步 API，包一层重试循环并回调进度；绿色版把「下载」从助手提前到主程序（带进度 + Range 断点续传 + 下载后 SHA256 校验），落盘 `<exe>.download` 后以 `--local-file` 交接给内置助手独立复核替换。助手替换状态机不动，仅把入参从 `url` 抽象为 `Source` 枚举。

**Tech Stack:** Tauri v2（`tauri-plugin-updater` 2.x、`tauri::async_runtime::spawn_blocking`）、reqwest blocking（现有 `["json", "blocking"]`，不加 `stream`——blocking `Response` 本身实现 `std::io::Read`）、原生 HTML/CSS/JS、Node `vm`/`assert` 前端测试。

**设计依据:** `docs/superpowers/specs/2026-09-29-auto-update-progress-design.md`（已批准）。

## Global Constraints

- 只改这些文件：`src-tauri/src/update.rs`、`src-tauri/src/main.rs`、`frontend/app.js`、`frontend/index.html`、`frontend/styles.css`、`scripts/test_update_ui.js`、`CHANGELOG.md`；`cargo build` 后 `build.rs` 回写缓存戳的既有文件（`frontend/index.html`、`frontend/app.js`、`tauri.conf.json`、`src/tray.rs`）如有变动一并提交。
- **不新增任何依赖**；reqwest features 保持 `["json", "blocking"]`（不加 `stream`）。
- **不改** `src-tauri/capabilities/*.json`（已有 `core:event:default`，emit 无需新权限）。
- `update.rs` 新增的辅助函数一律**不带** `#[tauri::command]`（命令面保持最小；`build.rs::command_names()` 只扫 main.rs）。
- `main.rs` **不新增 use**——`Duration` / `Instant` 等一律写全限定路径（如 `std::time::Duration::from_secs(1)`）。
- 全仓库唯一允许新增的 `#[allow(...)]`：Task 6 给 `build_helper_args` 加 `#[allow(dead_code)]`。Task 3-5 期间 `cargo build` 出现临时 dead_code 警告属预期（函数先落地、后接线），以 Task 7 的 0 warning 收口。
- 前端新增顶层函数（`renderMarkdown` / `paintUpdateProgress`）内部**不得出现行首 `}`**——`scripts/test_update_ui.js` 的 `fn()` 抽取器按 `"\n}"` 截取函数体；MB 格式化用内联箭头函数，不新增被抽取的依赖函数。
- 节流判据固定为 `pct != last_pct || elapsed >= 120ms`，**不得**加 `|| pct == -1`（total 未知时每 64KB 都会 emit）。
- `retrying` / `error` 阶段前端不重设进度条宽度、不动 `indeterminate` 类（保持上次位置）。
- 代码注释用简体中文，解释「为什么」。
- 每个 Task 收尾在仓库根目录跑 `node scripts/run_all.js`；Rust 相关 Task 再在 `src-tauri` 下跑 `cargo test`。

---

### Task 1: 更新说明富文本渲染（前端）

**Files:**
- Modify: `frontend/app.js`（新增顶层 `renderMarkdown`；`paintUpdate` 两处 `textContent` 改 `innerHTML`）
- Modify: `frontend/index.html`（`updNotes` 从 `<pre>` 改 `<div>`）
- Modify: `frontend/styles.css`（`.upd-notes` 去掉 `white-space: pre-wrap;`，新增排版规则）
- Test: `scripts/test_update_ui.js`（补抽取与语义断言）

**Interfaces:**
- Consumes: `paintUpdate()` 现有的 `info.notes` 与 `updateAnnounce` 字符串。
- Produces: 顶层 `renderMarkdown(text) -> string`（空/无 → `"暂无更新说明"`；`## `→`<h4>`、`### `→`<h5>`、`- `/`* `→`<ul><li>`、`N. `→`<ol><li>`、``` 围栏→`<pre><code>`、`**x**`→`<strong>`、`` `x` ``→`<code>`、`[文字](url)`→不可点击的 `<span class="upd-link" title="url">`；先整体转义 `& < > "` 再还原标记，杜绝注入）。

- [ ] **Step 1: 写失败测试**

在 `scripts/test_update_ui.js` 中：

① L32 的抽取列表补 `renderMarkdown`（放最前，供 `paintUpdate` 调用）：

```js
  vm.runInContext(['renderMarkdown','paintUpdate','loadUpdateInfo','showView','returnFromUpdate'].map(fn).join('\n'), ctx);
```

② 在 `vm.runInContext(...)` 之后、`$('viewSettings').scrollTop = 480;` 之前插入语义断言：

```js
  assert.equal(ctx.renderMarkdown(''), '暂无更新说明');
  const md = ctx.renderMarkdown('## 更新日志\n\n### 修复\n\n- 修复**崩溃**问题，见 `main.rs`\n1. 第一步\n\n<script>alert(1)</script>');
  assert(md.includes('<h4>更新日志</h4>'), 'h4 heading');
  assert(md.includes('<h5>修复</h5>'), 'h5 heading');
  assert(md.includes('<li>修复<strong>崩溃</strong>问题，见 <code>main.rs</code></li>'), 'ul li + bold + code');
  assert(md.includes('<li>第一步</li>'), 'ol li');
  assert(md.includes('&lt;script&gt;'), 'raw html must be escaped');
  const link = ctx.renderMarkdown('[首页](https://example.com)');
  assert(link.includes('<span class="upd-link" title="https://example.com">首页</span>'), 'link as plain span');
```

- [ ] **Step 2: 运行测试确认失败**

Run: `node scripts/test_update_ui.js`
Expected: FAIL —— `AssertionError [ERR_ASSERTION]: renderMarkdown`（fn() 抽取不到该函数）

- [ ] **Step 3: 实现 renderMarkdown 并接入 paintUpdate**

`frontend/app.js` 在 `let updateSettingsScroll = 0;` 行之后插入顶层函数（注意：所有闭合括号都缩进，行首不能出现 `}`）：

```js
// 把 CHANGELOG 片段的 markdown 源码转成安全的 HTML：先整体转义再还原有限标记，
// 支持 标题/列表/围栏代码块/粗体/行内代码；链接只展示文字 + title，不可点击，
// 避免更新说明里的外链把用户带离应用。
function renderMarkdown(text) {
  if (!text || !text.trim()) return "暂无更新说明";
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  const inline = (s) => s
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<span class="upd-link" title="$2">$1</span>');
  const html = [];
  let list = null;
  let code = false;
  const closeList = () => {
    if (list) { html.push("</" + list + ">"); list = null; }
  };
  for (const raw of String(text).split(/\r?\n/)) {
    if (raw.trim().startsWith("```")) {
      if (code) { html.push("</code></pre>"); code = false; }
      else { closeList(); html.push("<pre><code>"); code = true; }
      continue;
    }
    if (code) { html.push(raw); continue; }
    const t = raw.trim();
    if (!t) { closeList(); continue; }
    if (t.startsWith("### ")) { closeList(); html.push("<h5>" + inline(t.slice(4)) + "</h5>"); continue; }
    if (t.startsWith("## ")) { closeList(); html.push("<h4>" + inline(t.slice(3)) + "</h4>"); continue; }
    const ul = t.match(/^[-*] (.*)$/);
    if (ul) {
      if (list !== "ul") { closeList(); html.push("<ul>"); list = "ul"; }
      html.push("<li>" + inline(ul[1]) + "</li>");
      continue;
    }
    const ol = t.match(/^\d+\. (.*)$/);
    if (ol) {
      if (list !== "ol") { closeList(); html.push("<ol>"); list = "ol"; }
      html.push("<li>" + inline(ol[1]) + "</li>");
      continue;
    }
    closeList();
    html.push("<p>" + inline(t) + "</p>");
  }
  closeList();
  if (code) html.push("</code></pre>");
  return html.join("\n");
}
```

`paintUpdate()` 两处替换：

```js
  // 原：$("updNotes").textContent = info && info.notes ? info.notes : "暂无更新说明";
  $("updNotes").innerHTML = renderMarkdown(info && info.notes ? info.notes : "");
```

```js
  // 原：$("updNotes").textContent = updateAnnounce;
  $("updNotes").innerHTML = renderMarkdown(updateAnnounce);
```

`frontend/index.html` L771：

```html
        <div id="updNotes" class="upd-notes">暂无</div>
```

`frontend/styles.css`：`.upd-notes` 规则删掉 `white-space: pre-wrap;` 这一行，并在其后追加排版规则（`.upd-notes pre` 里的 `white-space: pre-wrap;` 同时保住 `scripts/test_update.js` 的子串断言）：

```css
.upd-notes h4 { margin: 10px 0 4px; font-size: 15px; color: #ececf0; }
.upd-notes h5 { margin: 8px 0 2px; font-size: 13.5px; color: #ddd; }
.upd-notes p { margin: 4px 0; }
.upd-notes ul,
.upd-notes ol { margin: 4px 0; padding-left: 20px; }
.upd-notes li { margin: 2px 0; }
.upd-notes code {
  padding: 1px 5px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.08);
  font-size: 12px;
}
.upd-notes pre {
  margin: 6px 0;
  padding: 8px 10px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.06);
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
}
.upd-notes pre code { padding: 0; background: none; font-size: 12px; }
.upd-link { color: #ffd650; text-decoration: underline dotted; cursor: help; }
```

- [ ] **Step 4: 验证**

Run: `node scripts/test_update_ui.js && node scripts/test_update.js && node scripts/run_all.js`
Expected: 全绿（run_all 打印 `[run_all] N test scripts, all passed`）

---

### Task 2: 进度条 UI 与事件订阅（前端）

**Files:**
- Modify: `frontend/index.html`（`updProgress` 从一行 `<p>` 升级为进度组件）
- Modify: `frontend/app.js`（`updateProgress` 状态、`paintUpdateProgress`、`watchUpdateProgress`、`showView` 补画、更新按钮改写、启动接线）
- Modify: `frontend/styles.css`（进度条样式）
- Test: `scripts/test_update_ui.js`

**Interfaces:**
- Consumes: 后端 `update-progress` 事件，payload `{ phase, downloaded, total, attempt }`，`phase ∈ downloading / verifying / installing / restarting / retrying / error`（error 额外带 `message`）；Task 3 才开始发事件，本任务先做空态容错。
- Produces: 顶层 `paintUpdateProgress()`（读全局 `updateProgress` 画 `#updProgress`）与 `watchUpdateProgress()`（订阅事件）。

- [ ] **Step 1: 写失败测试**

`scripts/test_update_ui.js`：

① `element()` 返回对象补 `style: {}`（进度条要写 `fill.style.width`）：

```js
  return {textContent: '', disabled: false, dataset: {}, scrollTop: 0, style: {},
```

② ctx 补 `updateProgress: null,`：

```js
  const ctx = {$, updateInfo:null, updateAnnounce:null, updateChecking:false, updateSettingsScroll:0, updateProgress:null,
```

③ L32 抽取列表补 `'paintUpdateProgress'`（放 `paintUpdate` 之后即可）。

④ 在 Task 1 加的语义断言之后插入进度断言：

```js
  ctx.updateProgress = {phase:'downloading', downloaded: 5*1048576, total: 10*1048576, attempt: 1};
  ctx.paintUpdateProgress();
  assert.equal($('updProgressFill').style.width, '50%');
  assert($('updProgressText').textContent.includes('50%'), 'percent in text');
  assert(!$('updProgress').classList.contains('hidden'));
  assert(!$('updProgress').classList.contains('indeterminate'));
  ctx.updateProgress = {phase:'downloading', downloaded: 3*1048576, total: 0, attempt: 1};
  ctx.paintUpdateProgress();
  assert($('updProgress').classList.contains('indeterminate'), 'unknown total -> sweep');
  ctx.updateProgress = null;
  ctx.paintUpdateProgress();
  assert($('updProgress').classList.contains('hidden'), 'no progress -> hidden');
```

- [ ] **Step 2: 运行测试确认失败**

Run: `node scripts/test_update_ui.js`
Expected: FAIL —— `AssertionError [ERR_ASSERTION]: paintUpdateProgress`

- [ ] **Step 3: 实现组件与接线**

`frontend/index.html` L777 一行替换为：

```html
      <div id="updProgress" class="upd-progress hidden" role="status" aria-live="polite">
        <div class="upd-progress-track"><div id="updProgressFill" class="upd-progress-fill"></div></div>
        <p id="updProgressText" class="upd-progress-text"></p>
      </div>
```

`frontend/app.js`：

① 在 `let updateSettingsScroll = 0;` 行之后加状态（Task 1 插入的 renderMarkdown 在其后，不冲突）：

```js
// 最近一次 update-progress 事件；null = 无进行中的更新
let updateProgress = null;
```

② 在 `paintUpdate()` 之后加画函数（内部括号全部缩进，行首不能出现 `}`）：

```js
// 画下载进度：由 update-progress 事件驱动；retrying/error 保持上次的进度条位置
function paintUpdateProgress() {
  const box = $("updProgress");
  const fill = $("updProgressFill");
  const text = $("updProgressText");
  if (!updateProgress || !updateProgress.phase) {
    box.classList.add("hidden");
    return;
  }
  box.classList.remove("hidden");
  const mb = (n) => (n / 1048576).toFixed(1);
  const p = updateProgress;
  if (p.phase === "downloading") {
    if (p.total) {
      const pct = Math.min(100, Math.round((p.downloaded / p.total) * 100));
      box.classList.remove("indeterminate");
      fill.style.width = pct + "%";
      text.textContent = "正在下载… " + pct + "%（" + mb(p.downloaded) + " / " + mb(p.total) + " MB）";
    } else {
      box.classList.add("indeterminate");
      fill.style.width = "";
      text.textContent = "正在下载… " + mb(p.downloaded) + " MB";
    }
  } else if (p.phase === "retrying") {
    text.textContent = "网络中断，正在重试（第 " + p.attempt + " 次）…";
  } else if (p.phase === "verifying") {
    box.classList.remove("indeterminate");
    fill.style.width = "100%";
    text.textContent = "下载完成，正在校验完整性…";
  } else if (p.phase === "installing") {
    box.classList.remove("indeterminate");
    fill.style.width = "100%";
    text.textContent = updateInfo && updateInfo.installed
      ? "正在安装，安装程序将自动关闭本程序…"
      : "正在替换程序文件，即将自动重启…";
  } else if (p.phase === "restarting") {
    fill.style.width = "100%";
    text.textContent = "正在重启…";
  } else if (p.phase === "error") {
    text.textContent = "更新失败：" + (p.message || "未知错误");
  }
}
```

③ `showView()` 的 viewUpdate 分支补一行（原有 paintUpdate/loadUpdateInfo 逻辑不动）：

```js
      if (id === "viewUpdate") {
        if (updateAnnounce) paintUpdate();
        else loadUpdateInfo();
        paintUpdateProgress();
      }
```

④ 在 `watchUpdateView()` 之后加订阅函数：

```js
// 订阅后端下载进度；事件系统若不通则进度条停留在最近一次画面——不阻塞更新本身
async function watchUpdateProgress() {
  try {
    await TAURI.event.listen("update-progress", (evt) => {
      updateProgress = evt && evt.payload ? evt.payload : null;
      if (curView === "viewUpdate") paintUpdateProgress();
    });
  } catch (err) {
    flog("update progress listen failed: " + (err && err.message ? err.message : String(err)));
  }
}
```

⑤ 更新按钮点击处理整段替换（进度交给事件驱动，命令返回只代表交接/安装流程结束）：

```js
// 立即更新：进度由 update-progress 事件驱动；失败时窗口不再消失，可直接看到原因并重试
$("updUpdateBtn").addEventListener("click", async () => {
  const button = $("updUpdateBtn");
  if (button.disabled) return;
  button.disabled = true;
  updateProgress = { phase: "downloading", downloaded: 0, total: 0, attempt: 1 };
  paintUpdateProgress();
  try {
    await invoke("start_update");
  } catch (e) {
    updateProgress = { phase: "error", message: e && e.message ? e.message : String(e) };
    paintUpdateProgress();
    button.disabled = false;
  }
});
```

⑥ 文件底部 `watchUpdateView();` 之后补一行：

```js
watchUpdateProgress();
```

`frontend/styles.css` 追加（`.upd-progress` 用 flex，必须显式写 `.hidden` 覆盖）：

```css
.upd-progress { display: flex; flex-direction: column; gap: 6px; }
.upd-progress.hidden { display: none; }
.upd-progress-track {
  height: 8px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.1);
  overflow: hidden;
}
.upd-progress-fill {
  height: 100%;
  width: 0;
  border-radius: 999px;
  background: #ffd650;
  transition: width 0.2s linear;
}
.upd-progress.indeterminate .upd-progress-fill {
  width: 40%;
  animation: upd-sweep 1.2s infinite linear;
}
@keyframes upd-sweep {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(250%); }
}
.upd-progress-text { margin: 0; font-size: 13px; color: #b8b8be; overflow-wrap: anywhere; }
```

- [ ] **Step 4: 验证**

Run: `node scripts/test_update_ui.js && node scripts/test_update.js && node scripts/run_all.js`
Expected: 全绿（`test_update.js` 的 `id="updProgress"` 与 `invoke("start_update")` 断言仍命中）

---

### Task 3: 下载核心（Rust）：Fetcher / 断点续传 / 带重试下载 / 进度事件

**Files:**
- Modify: `src-tauri/src/update.rs`（import 调整；`find_hash_in_sums` 之后插入新代码；`mod tests` 末尾追加测试）

**Interfaces:**
- Produces:
  - `pub const PROGRESS_EVENT: &str = "update-progress"`
  - `pub const DOWNLOAD_ATTEMPTS: u32 = 3`
  - `pub fn emit_progress(app: &tauri::AppHandle, phase: &str, downloaded: u64, total: Option<u64>, attempt: u32)`
  - `pub trait Fetcher { fn fetch(&self, url: &str, buf: &mut Vec<u8>, on_chunk: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String>; }` + `pub struct RealFetcher`
  - `pub fn download_with_retry<F: Fetcher>(fetcher: &F, url: &str, on_progress: &mut dyn FnMut(u64, Option<u64>, u32), on_retry: &mut dyn FnMut(u32, &str), pause: &mut dyn FnMut(u32)) -> Result<Vec<u8>, String>`
- 本 Task 尚无生产调用者（Task 5/6 接线），`cargo build` 出现 dead_code 警告属预期。

- [ ] **Step 1: 写失败测试**

`src-tauri/src/update.rs` 的 `mod tests` 末尾（最后一个 `}` 之前）追加：

```rust
    // ---- download_with_retry：假 Fetcher 驱动断点续传 / 全量重来 / 重试耗尽 ----

    struct FakeFetcher {
        /// 完整的「服务器内容」
        full: Vec<u8>,
        /// 第 N 次 fetch 允许下发的字节数；None = 一次性给完
        caps: Vec<Option<usize>>,
        /// true = 无视 Range 从头发全量（模拟不支持续传的服务器）
        ignore_range: bool,
        fetch_calls: Cell<u32>,
        /// 每次 fetch 时 buf 的既有长度（= 续传起点）
        starts: RefCell<Vec<usize>>,
    }

    impl Fetcher for FakeFetcher {
        fn fetch(&self, _url: &str, buf: &mut Vec<u8>, on_chunk: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
            let n = self.fetch_calls.get() + 1;
            self.fetch_calls.set(n);
            if self.ignore_range {
                buf.clear();
            }
            self.starts.borrow_mut().push(buf.len());
            let start = buf.len();
            let cap = self.caps.get((n - 1) as usize).copied().flatten();
            let end = match cap {
                Some(k) => (start + k).min(self.full.len()),
                None => self.full.len(),
            };
            buf.extend_from_slice(&self.full[start..end]);
            on_chunk(buf.len() as u64, Some(self.full.len() as u64));
            // 恒 Ok：把「传输不完整」的重试路径留给 download_with_retry 自己判
            Ok(())
        }
    }

    /// 跑一遍 download_with_retry，回收三类回调的调用记录
    fn run_download(
        fetcher: &FakeFetcher,
    ) -> (
        Result<Vec<u8>, String>,
        Vec<(u64, Option<u64>, u32)>,
        Vec<u32>,
        Vec<u32>,
    ) {
        let mut progress = Vec::new();
        let mut retries = Vec::new();
        let mut pauses = Vec::new();
        let out = download_with_retry(
            fetcher,
            "https://example.com/new.exe",
            &mut |d, t, a| progress.push((d, t, a)),
            &mut |a, _e| retries.push(a),
            &mut |a| pauses.push(a),
        );
        (out, progress, retries, pauses)
    }

    #[test]
    fn download_with_retry_resumes_from_breakpoint() {
        // 第 1 次断在 4 字节，第 2 次从 4 续到 10：起点即证明带了 Range
        let fetcher = FakeFetcher {
            full: (0u8..10).collect(),
            caps: vec![Some(4), None],
            ignore_range: false,
            fetch_calls: Cell::new(0),
            starts: RefCell::new(Vec::new()),
        };
        let (out, progress, retries, pauses) = run_download(&fetcher);
        assert_eq!(out.unwrap(), (0u8..10).collect::<Vec<u8>>());
        assert_eq!(*fetcher.starts.borrow(), vec![0, 4]);
        assert_eq!(fetcher.fetch_calls.get(), 2);
        assert_eq!(retries, vec![2]);
        assert_eq!(pauses, vec![1]);
        assert_eq!(progress.last(), Some(&(10, Some(10), 2)));
    }

    #[test]
    fn download_with_retry_restarts_when_range_ignored() {
        // 服务端无视 Range 回全量：fetcher 内部清空重来，两次起点都是 0，结果仍完整
        let fetcher = FakeFetcher {
            full: (0u8..10).collect(),
            caps: vec![Some(4), None],
            ignore_range: true,
            fetch_calls: Cell::new(0),
            starts: RefCell::new(Vec::new()),
        };
        let (out, _progress, _retries, _pauses) = run_download(&fetcher);
        assert_eq!(out.unwrap(), (0u8..10).collect::<Vec<u8>>());
        assert_eq!(*fetcher.starts.borrow(), vec![0, 0]);
    }

    #[test]
    fn download_with_retry_exhausts_attempts() {
        // 每次都断在 3 字节：3 次全用完后报「已重试 2 次」
        let fetcher = FakeFetcher {
            full: (0u8..10).collect(),
            caps: vec![Some(3), Some(3), Some(3)],
            ignore_range: false,
            fetch_calls: Cell::new(0),
            starts: RefCell::new(Vec::new()),
        };
        let (out, _progress, retries, pauses) = run_download(&fetcher);
        assert!(out.unwrap_err().contains("已重试 2 次"));
        assert_eq!(fetcher.fetch_calls.get(), 3);
        assert_eq!(retries, vec![2, 3]);
        assert_eq!(pauses, vec![1, 2]);
    }
```

- [ ] **Step 2: 编译确认失败**

Run: `cd src-tauri; cargo test update::tests::download_with_retry`
Expected: 编译错误 —— `Fetcher` / `download_with_retry` 不存在

- [ ] **Step 3: 实现**

`src-tauri/src/update.rs`：

① import 区两行调整（L7 / L10）：

```rust
use std::io::{Read, Write};
```

```rust
use tauri::{Emitter, Manager};
```

② 在 `find_hash_in_sums` 函数之后、`build_helper_args` 的文档注释之前插入：

```rust
/// 前端进度事件名（payload 见 emit_progress）。
pub const PROGRESS_EVENT: &str = "update-progress";

/// 下载最大尝试次数（首次 + 2 次重试）。
pub const DOWNLOAD_ATTEMPTS: u32 = 3;

/// 向前端广播下载进度。事件发送失败直接忽略——进度只是锦上添花，
/// 绝不能因为它打断更新。
pub fn emit_progress(
    app: &tauri::AppHandle,
    phase: &str,
    downloaded: u64,
    total: Option<u64>,
    attempt: u32,
) {
    let payload = serde_json::json!({
        "phase": phase,
        "downloaded": downloaded,
        "total": total,
        "attempt": attempt,
    });
    let _ = app.emit(PROGRESS_EVENT, payload);
}

/// 下载能力抽象：把 url 内容取到 buf（从 buf 现有长度续传），
/// 每收到一块数据回调 on_chunk(已下载, 总长)。真实实现走 reqwest blocking。
pub trait Fetcher {
    fn fetch(
        &self,
        url: &str,
        buf: &mut Vec<u8>,
        on_chunk: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(), String>;
}

/// 真实下载：15 秒连接超时 + 300 秒总超时 + 显式 UA（GitHub 对无 UA 请求限流）。
/// buf 非空时带 Range 续传；服务端不支持（回 200 全量）则清空重来。
pub struct RealFetcher;

impl Fetcher for RealFetcher {
    fn fetch(
        &self,
        url: &str,
        buf: &mut Vec<u8>,
        on_chunk: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<(), String> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .user_agent(concat!("niuma-timer/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let mut req = client.get(url);
        if !buf.is_empty() {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-", buf.len()));
        }
        let resp = req.send().map_err(|e| e.to_string())?;
        let status = resp.status();
        if status == reqwest::StatusCode::OK {
            // 服务器不支持 Range（回了 200 全量）：丢掉半截，从头收
            buf.clear();
        } else if !status.is_success() {
            return Err(format!("HTTP {status}"));
        }
        let total = resp.content_length().map(|n| buf.len() as u64 + n);
        on_chunk(buf.len() as u64, total);
        let mut chunk = [0u8; 65536];
        loop {
            let n = resp.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..n]);
            on_chunk(buf.len() as u64, total);
        }
    }
}

/// 带重试的下载：最多 DOWNLOAD_ATTEMPTS 次；退避由 pause 回调执行（单测注入零等待）。
/// buf 跨尝试保留——配合支持 Range 的 Fetcher 即为断点续传，服务端不支持时 fetch 内部清空重来。
/// on_progress(已下载, 总长, 第几次尝试) 驱动进度；on_retry(下次尝试序号, 失败原因) 驱动提示。
pub fn download_with_retry<F: Fetcher>(
    fetcher: &F,
    url: &str,
    on_progress: &mut dyn FnMut(u64, Option<u64>, u32),
    on_retry: &mut dyn FnMut(u32, &str),
    pause: &mut dyn FnMut(u32),
) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut last_err = String::from("未知错误");
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        let mut seen_total: Option<u64> = None;
        // 闭包同时可变借用 seen_total 与 on_progress，把 fetch 调用包进独立作用域，
        // 让闭包在块末 drop，避免借用冲突
        let result = {
            let mut on_chunk = |downloaded: u64, total: Option<u64>| {
                seen_total = total;
                on_progress(downloaded, total, attempt);
            };
            fetcher.fetch(url, &mut buf, &mut on_chunk)
        };
        match result {
            Ok(()) => {
                // 流正常返回也可能被中途掐断：有总长就比长度，无总长只要非空即算完整
                let complete = match seen_total {
                    Some(len) => buf.len() as u64 >= len,
                    None => !buf.is_empty(),
                };
                if complete {
                    return Ok(buf);
                }
                last_err = if seen_total.is_some() {
                    "传输提前中断".to_string()
                } else {
                    "响应内容为空".to_string()
                };
            }
            Err(e) => last_err = e,
        }
        if attempt < DOWNLOAD_ATTEMPTS {
            on_retry(attempt + 1, &last_err);
            pause(attempt);
        }
    }
    Err(format!(
        "下载失败（已重试 {} 次）：{last_err}",
        DOWNLOAD_ATTEMPTS - 1
    ))
}
```

- [ ] **Step 4: 验证**

Run: `cd src-tauri; cargo test`
Expected: 全部通过（含新增 3 个用例；`cargo build` 会有 dead_code 警告——接线在 Task 5/6，暂不管）

Run: `node scripts/run_all.js`
Expected: 全绿

---

### Task 4: `Source` 抽象：助手支持 url 与本地文件两种来源

**Files:**
- Modify: `src-tauri/src/update.rs`（`Source` 枚举；`HelperArgs.url` → `source`；`HelperIo::download` → `read`；`parse_helper_args` / `RealHelperIo` / `run_helper_flow` / `build_helper_args` 同步；新增 `build_helper_args_local`；测试同步）
- Modify: `src-tauri/src/main.rs`（`start_update` 绿色版段argv 从 `args.url` 改为解构 `args.source`，保编译）

**Interfaces:**
- Produces: `pub enum Source { Url(String), Local(String) }`；`HelperArgs { target, wait_pid, source, sha256 }`；`pub fn build_helper_args_local(target: &str, wait_pid: u32, path: &str, sha256: &str) -> HelperArgs`；助手新增 argv 开关 `--local-file <path>`。
- 助手流程在本地来源成功后顺带清理交接文件（`.download`），失败时保留（下次更新覆盖写）。

- [ ] **Step 1: 写失败测试**

`src-tauri/src/update.rs` 测试区同步改造：

① `FakeIo`（L703 起）：字段 `download_result: Result<Vec<u8>, String>,` 改名 `read_result`，并新增 `read_sources: RefCell<Vec<Source>>,`；`Default` 里 `download_result: Ok(vec![1, 2, 3]),` 改 `read_result: Ok(vec![1, 2, 3]),`，并补 `read_sources: RefCell::new(Vec::new()),`；impl 里的下载方法替换为：

```rust
        fn read(&self, source: &Source) -> Result<Vec<u8>, String> {
            self.read_sources.borrow_mut().push(source.clone());
            self.read_result.clone()
        }
```

② `helper_args_for_test()`（L778）：`url: "https://example.com/new.exe".to_string(),` 改：

```rust
            source: Source::Url("https://example.com/new.exe".to_string()),
```

③ `run_helper_flow_hash_mismatch`：`download_result: Ok(vec![9, 9, 9]),` 改 `read_result: Ok(vec![9, 9, 9]),`

④ `run_helper_flow_download_failure_restarts_old` 整体改名并改字段：

```rust
    #[test]
    fn run_helper_flow_read_failure_restarts_old() {
        let io = FakeIo { read_result: Err("offline".to_string()), ..FakeIo::default() };
        assert_eq!(run_helper_flow(&io, &helper_args_for_test()), 1);
        assert_eq!(io.spawned.borrow().len(), 1);
        assert_eq!(io.rename_calls.get(), 0);
    }
```

⑤ `parse_helper_args_full` 的 L982 断言改：

```rust
        assert_eq!(h.source, Source::Url("https://example.com/new.exe".to_string()));
```

⑥ 在 `parse_helper_args_missing_required` 之后追加三个用例：

```rust
    #[test]
    fn parse_helper_args_local_file() {
        let argv: Vec<String> = [
            "niuma-timer.exe",
            "--apply-update",
            "--target",
            "C:\\app\\niuma-timer.exe",
            "--wait-pid",
            "1234",
            "--local-file",
            "C:\\app\\niuma-timer.exe.download",
            "--sha256",
            "ABC",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let h = parse_helper_args(&argv).unwrap();
        assert_eq!(h.source, Source::Local("C:\\app\\niuma-timer.exe.download".to_string()));
        assert_eq!(h.sha256, "abc");
    }

    #[test]
    fn run_helper_flow_from_local_source() {
        let io = FakeIo::default();
        let args = HelperArgs {
            source: Source::Local("C:\\app\\niuma-timer.exe.download".to_string()),
            ..helper_args_for_test()
        };
        assert_eq!(run_helper_flow(&io, &args), 0);
        assert_eq!(
            *io.read_sources.borrow(),
            vec![Source::Local("C:\\app\\niuma-timer.exe.download".to_string())]
        );
        assert_eq!(io.rename_calls.get(), 2);
        // 成功后清理：备份 + 本地交接文件
        assert_eq!(io.removed.borrow().len(), 2);
    }

    #[test]
    fn run_helper_flow_local_content_mismatch() {
        let io = FakeIo {
            read_result: Ok(vec![7, 7, 7]),
            ..FakeIo::default()
        };
        let args = HelperArgs {
            source: Source::Local("C:\\app\\niuma-timer.exe.download".to_string()),
            ..helper_args_for_test()
        };
        assert_eq!(run_helper_flow(&io, &args), 1);
        assert!(io.written.borrow().is_empty());
        assert_eq!(io.rename_calls.get(), 0);
        assert_eq!(io.read_sources.borrow().len(), 1);
    }
```

- [ ] **Step 2: 编译确认失败**

Run: `cd src-tauri; cargo test update::tests::parse_helper_args`
Expected: 编译错误 —— `Source` 不存在 / `HelperArgs` 缺 `source` 字段

- [ ] **Step 3: 实现 update.rs**

① `HelperArgs` 定义（L112）之前加：

```rust
/// 助手取更新包的两个来源：远程 url（旧路径保留）或主程序已下好的本地文件（新流程）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Url(String),
    Local(String),
}
```

② `HelperArgs` 的字段 `pub url: String,`（doc「新 exe 下载地址」）改为：

```rust
    /// 新包来源（url 或本地交接文件）
    pub source: Source,
```

③ `parse_helper_args` 的 `Some(HelperArgs { ... })` 段替换为：

```rust
    let source = match (value_of("--url"), value_of("--local-file")) {
        (Some(url), _) => Source::Url(url),
        (None, Some(path)) => Source::Local(path),
        (None, None) => return None,
    };
    Some(HelperArgs {
        target: value_of("--target")?,
        wait_pid: value_of("--wait-pid").and_then(|s| s.parse::<u32>().ok()),
        source,
        sha256: value_of("--sha256")?.to_lowercase(),
    })
```

④ `HelperIo` trait 的下载方法（doc「把 url 的内容下载到内存」+ `fn download(&self, url: &str) -> Result<Vec<u8>, String>;`）替换为：

```rust
    /// 按来源取更新包内容（url 下载 / 本地文件读盘）
    fn read(&self, source: &Source) -> Result<Vec<u8>, String>;
```

⑤ `run_helper_flow` 的「② 下载」段（原 `let bytes = match io.download(&args.url) { ... }`）替换为：

```rust
    // ② 取更新包内容
    let bytes = match io.read(&args.source) {
        Ok(b) => b,
        Err(e) => {
            io.log(&format!("读取新版本失败：{e}"));
            let _ = io.spawn(target);
            return 1;
        }
    };
```

同步把该函数文档首行的「等旧进程退出 → 下载 → 校验」改为「等旧进程退出 → 取更新包 → 校验」。

⑥ 步骤 ⑧ 清理段（原只有 `let _ = io.remove(&old_path);`）替换为：

```rust
    // ⑧ 清理备份与本地交接文件（失败路径不清理，下次更新会覆盖写）
    let _ = io.remove(&old_path);
    if let Source::Local(path) = &args.source {
        let _ = io.remove(Path::new(path));
    }
```

⑦ `impl HelperIo for RealHelperIo` 的下载方法（原 `fn download(&self, url: &str)`）替换为：

```rust
    fn read(&self, source: &Source) -> Result<Vec<u8>, String> {
        match source {
            // 主程序已下好并校验过的本地交接文件：直接读盘
            Source::Local(path) => std::fs::read(path).map_err(|e| e.to_string()),
            Source::Url(url) => {
                let resp = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(120))
                    .build()
                    .map_err(|e| e.to_string())?
                    .get(url)
                    .send()
                    .map_err(|e| e.to_string())?;
                if !resp.status().is_success() {
                    return Err(format!("HTTP {}", resp.status()));
                }
                resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
            }
        }
    }
```

⑧ `build_helper_args` 内 `Ok(HelperArgs { ... url: url.to_string(), ... })` 的字段改：

```rust
        source: Source::Url(url.to_string()),
```

⑨ 在 `build_helper_args` 之后新增：

```rust
/// 组装本地交接的助手参数：主程序已下载并校验过，助手只独立复核一次 SHA256。
pub fn build_helper_args_local(target: &str, wait_pid: u32, path: &str, sha256: &str) -> HelperArgs {
    HelperArgs {
        target: target.to_string(),
        wait_pid: Some(wait_pid),
        source: Source::Local(path.to_string()),
        sha256: sha256.to_lowercase(),
    }
}
```

- [ ] **Step 4: 保编译：main.rs 绿色版 argv 段**

`src-tauri/src/main.rs` 的 `start_update` 里（原 `argv.push("--url".to_string()); argv.push(args.url.clone());`）：

```rust
    let update::Source::Url(url) = &args.source else {
        return Err("内部错误：助手参数不是 url 来源".to_string());
    };
    argv.push("--url".to_string());
    argv.push(url.clone());
```

（Task 6 会整体重写这段。）

- [ ] **Step 5: 验证**

Run: `cd src-tauri; cargo test`
Expected: 全部通过

Run: `node scripts/run_all.js`
Expected: 全绿（`scripts/test_update.js` 无任何断言依赖 `download` 方法名或 `url` 字段名）

---

### Task 5: 安装版：真进度 + 重试 + 删除不可达 restart

**Files:**
- Modify: `src-tauri/src/main.rs`（`start_update` 拆为 dispatcher + `start_update_installed` + 过渡版 `start_update_portable`）

**Interfaces:**
- Consumes: Task 3 的 `emit_progress` / `DOWNLOAD_ATTEMPTS`；`tauri_plugin_updater` 的 `pending.download(on_chunk, on_finish)`（`on_chunk: FnMut(usize, Option<u64>)`）与 `pending.install(&bytes)`（Windows 上成功后进程由安装器接管）。
- Produces: `start_update` 变为按 `update::installed_kind()` 分发；绿色版行为暂不变（Task 6 重写）。

- [ ] **Step 1: 实现**

`src-tauri/src/main.rs` 把整个 `start_update`（L729-788）替换为：

```rust
/// 立即更新：按安装形态分发；两条路径都把进度经 `update-progress` 事件推给前端。
#[tauri::command]
async fn start_update(app: tauri::AppHandle) -> Result<(), String> {
    let _operation = update::UpdateOperation::begin()?;
    if update::installed_kind().is_some() {
        start_update_installed(app).await
    } else {
        start_update_portable(app).await
    }
}

/// 安装版：官方 updater 分步下载（带进度与重试）→ 交给安装器静默安装。
/// 成功后进程由安装器接管，`install` 返回即到头——这里不也不该再 restart。
async fn start_update_installed(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let Some(target) = update::installed_kind() else {
        return Err("当前不是安装版".to_string());
    };
    let updater = app
        .updater_builder()
        .target(target)
        .build()
        .map_err(|e| e.to_string())?;
    let Some(pending) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("已是最新版本".to_string());
    };
    update::emit_progress(&app, "downloading", 0, None, 1);
    let mut bytes: Option<Vec<u8>> = None;
    let mut last_err = String::from("未知错误");
    for attempt in 1..=update::DOWNLOAD_ATTEMPTS {
        // Cell 不是 Send：跨 await 的闭包只能捕获 owned 局部量
        let mut seen: u64 = 0;
        let mut last_pct: i64 = -2;
        let mut last_at = std::time::Instant::now();
        let chunk_app = app.clone();
        let on_chunk = move |len: usize, total: Option<u64>| {
            seen += len as u64;
            let pct = match total {
                Some(t) if t > 0 => (seen as f64 / t as f64 * 100.0) as i64,
                _ => -1,
            };
            let now = std::time::Instant::now();
            // 节流：百分比变化或距上次 ≥120ms 才发；total 未知时只按时间节流
            if pct != last_pct || now.duration_since(last_at) >= std::time::Duration::from_millis(120) {
                last_pct = pct;
                last_at = now;
                update::emit_progress(&chunk_app, "downloading", seen, total, attempt);
            }
        };
        let finish_app = app.clone();
        let on_finish = move || update::emit_progress(&finish_app, "verifying", 0, None, attempt);
        match pending.download(on_chunk, on_finish).await {
            Ok(b) => {
                bytes = Some(b);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
        if attempt < update::DOWNLOAD_ATTEMPTS {
            update::emit_progress(&app, "retrying", 0, None, attempt + 1);
            // async 环境里的退避等待：丢到阻塞线程再 sleep
            let secs = if attempt <= 1 { 1u64 } else { 3u64 };
            let _ = tauri::async_runtime::spawn_blocking(move || {
                std::thread::sleep(std::time::Duration::from_secs(secs))
            })
            .await;
        }
    }
    let Some(bytes) = bytes else {
        update::emit_progress(&app, "error", 0, None, update::DOWNLOAD_ATTEMPTS);
        return Err(format!(
            "下载失败（已重试 {} 次）：{last_err}",
            update::DOWNLOAD_ATTEMPTS - 1
        ));
    };
    update::emit_progress(
        &app,
        "installing",
        bytes.len() as u64,
        Some(bytes.len() as u64),
        1,
    );
    pending.install(&bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// 绿色版（Task 6 前的过渡）：保持原有「把 url 交给助手下载」的行为。
async fn start_update_portable(app: tauri::AppHandle) -> Result<(), String> {
    let rel = tauri::async_runtime::spawn_blocking(update::fetch_remote)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let url = rel
        .platform_url(update::PORTABLE_KEY)
        .ok_or_else(|| "远端清单里没有便携包条目".to_string())?
        .to_string();
    if update::compare_versions(&rel.version, env!("CARGO_PKG_VERSION")) != std::cmp::Ordering::Greater {
        return Err("已是最新版本".to_string());
    }
    let sums_url = update::sums_url_for(&url);
    let sums = tauri::async_runtime::spawn_blocking(move || update::fetch_text(&sums_url))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let args = update::build_helper_args(
        &exe.to_string_lossy(),
        std::process::id(),
        &url,
        &sums,
    )?;
    let mut argv = vec![
        "--apply-update".to_string(),
        "--target".to_string(),
        args.target.clone(),
    ];
    if let Some(pid) = args.wait_pid {
        argv.push("--wait-pid".to_string());
        argv.push(pid.to_string());
    }
    let update::Source::Url(url) = &args.source else {
        return Err("内部错误：助手参数不是 url 来源".to_string());
    };
    argv.push("--url".to_string());
    argv.push(url.clone());
    argv.push("--sha256".to_string());
    argv.push(args.sha256.clone());
    std::process::Command::new(&exe)
        .args(&argv)
        .spawn()
        .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}
```

- [ ] **Step 2: 验证**

Run: `cd src-tauri; cargo test`
Expected: 全部通过（`scripts/test_update.js` 对 `start_update` 的断言只是函数存在 + 命令注册，仍命中）

Run: `cd src-tauri; cargo build`
Expected: 编译成功；Task 3 遗留的 dead_code 警告仍在（`RealFetcher` / `download_with_retry` / `build_helper_args_local` 等），Task 6/7 收口

Run: `node scripts/run_all.js`
Expected: 全绿

---

### Task 6: 绿色版：主程序下载 + 校验 + 本地交接

**Files:**
- Modify: `src-tauri/src/main.rs`（整体重写 `start_update_portable`）
- Modify: `src-tauri/src/update.rs`（`build_helper_args` 失去最后调用者，加 `#[allow(dead_code)]`）

**Interfaces:**
- Consumes: Task 3 的 `RealFetcher` / `download_with_retry` / `emit_progress`；Task 4 的 `build_helper_args_local` / `Source::Local`。
- Produces: 绿色版新流程——主程序带进度下载（Range 续传 + 3 次重试）→ SHA256 校验 → 落盘 `<exe>.download` → 起助手 `--local-file` 复核替换 → `app.exit(0)`。

- [ ] **Step 1: 重写 start_update_portable**

`src-tauri/src/main.rs` 把 Task 5 的过渡版 `start_update_portable` 整体替换为：

```rust
/// 绿色版：主程序带进度下载 → SHA256 校验 → 落盘交接文件 → 起内置助手复核替换。
/// 下载从助手提前到主程序，才能把真实进度发给前端；助手只做「复核 + 原子替换」。
async fn start_update_portable(app: tauri::AppHandle) -> Result<(), String> {
    let rel = tauri::async_runtime::spawn_blocking(update::fetch_remote)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let url = rel
        .platform_url(update::PORTABLE_KEY)
        .ok_or_else(|| "远端清单里没有便携包条目".to_string())?
        .to_string();
    if update::compare_versions(&rel.version, env!("CARGO_PKG_VERSION")) != std::cmp::Ordering::Greater {
        return Err("已是最新版本".to_string());
    }
    let sums_url = update::sums_url_for(&url);
    let sums = tauri::async_runtime::spawn_blocking(move || update::fetch_text(&sums_url))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let file_name = url.rsplit('/').next().unwrap_or("");
    if file_name.is_empty() {
        return Err("下载地址里没有文件名".to_string());
    }
    let expected = update::find_hash_in_sums(&sums, file_name)
        .ok_or_else(|| format!("SHA256SUMS.txt 里没有 {file_name} 的哈希"))?;

    // 下载 + 重试都在阻塞线程里跑（不卡 async 运行时）；AppHandle 先克隆好分给各闭包
    let progress_app = app.clone();
    let download_url = url.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let downloading_app = progress_app.clone();
        let retry_app = progress_app;
        let mut last_pct: i64 = -2;
        let mut last_at = std::time::Instant::now();
        let fetcher = update::RealFetcher;
        update::download_with_retry(
            &fetcher,
            &download_url,
            &mut |done: u64, total: Option<u64>, attempt: u32| {
                let pct = match total {
                    Some(t) if t > 0 => (done as f64 / t as f64 * 100.0) as i64,
                    _ => -1,
                };
                let now = std::time::Instant::now();
                if pct != last_pct || now.duration_since(last_at) >= std::time::Duration::from_millis(120) {
                    last_pct = pct;
                    last_at = now;
                    update::emit_progress(&downloading_app, "downloading", done, total, attempt);
                }
            },
            &mut |attempt: u32, _err: &str| {
                update::emit_progress(&retry_app, "retrying", 0, None, attempt);
            },
            &mut |attempt: u32| {
                let secs = if attempt <= 1 { 1u64 } else { 3u64 };
                std::thread::sleep(std::time::Duration::from_secs(secs));
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| {
        update::emit_progress(&app, "error", 0, None, update::DOWNLOAD_ATTEMPTS);
        e
    })?;

    update::emit_progress(
        &app,
        "verifying",
        bytes.len() as u64,
        Some(bytes.len() as u64),
        1,
    );
    if !update::sha256_hex(&bytes).eq_ignore_ascii_case(&expected) {
        update::emit_progress(&app, "error", 0, None, 1);
        return Err(format!("SHA256 校验失败：期望 {expected}"));
    }

    // 校验通过才落盘交接文件（与 exe 同目录同卷，助手改名是原子操作）
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let staging = format!("{}.download", exe.to_string_lossy());
    std::fs::write(&staging, &bytes).map_err(|e| e.to_string())?;

    let args = update::build_helper_args_local(
        &exe.to_string_lossy(),
        std::process::id(),
        &staging,
        &expected,
    );
    let mut argv = vec![
        "--apply-update".to_string(),
        "--target".to_string(),
        args.target.clone(),
    ];
    if let Some(pid) = args.wait_pid {
        argv.push("--wait-pid".to_string());
        argv.push(pid.to_string());
    }
    let update::Source::Local(staging_path) = &args.source else {
        return Err("内部错误：交接参数不是本地文件来源".to_string());
    };
    argv.push("--local-file".to_string());
    argv.push(staging_path.clone());
    argv.push("--sha256".to_string());
    argv.push(args.sha256.clone());
    update::emit_progress(
        &app,
        "installing",
        bytes.len() as u64,
        Some(bytes.len() as u64),
        1,
    );
    std::process::Command::new(&exe)
        .args(&argv)
        .spawn()
        .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}
```

- [ ] **Step 2: build_helper_args 加 allow**

`src-tauri/src/update.rs` 的 `build_helper_args` 文档注释与 `pub fn` 之间加一行（此刻它失去最后调用者，属刻意的保留实现）：

```rust
#[allow(dead_code)]
```

- [ ] **Step 3: 验证**

Run: `cd src-tauri; cargo test`
Expected: 全部通过

Run: `cd src-tauri; cargo build`
Expected: 编译成功、**0 warning**（`build_helper_args_local` / `RealFetcher` / `download_with_retry` / `emit_progress` 均已有调用者）

Run: `node scripts/run_all.js`
Expected: 全绿

---

### Task 7: CHANGELOG + 全量验证 + 提交

**Files:**
- Modify: `CHANGELOG.md`（新增 `## [未发布]` 区）

- [ ] **Step 1: 写 CHANGELOG**

`CHANGELOG.md` 在 `## [1.4.1] - 2026-09-28` 之前插入（原文照抄 spec §14）：

```markdown
## [未发布]

### 修复

- 更新说明不再显示 markdown 原始字符（`###` / `-` / `**` 等），改为正常排版。

### 优化

- 更新时显示真实下载进度与百分比，可看出「重试中 / 校验中 / 安装中」当前阶段。
- 下载中断自动重试并续传；彻底失败时窗口不再消失，可直接看到原因并重试。

```

- [ ] **Step 2: 全量验证**

Run: `node scripts/run_all.js`
Expected: `[run_all] N test scripts, all passed`

Run: `cd src-tauri; cargo test`
Expected: 全部通过

Run: `cd src-tauri; cargo build`
Expected: 0 warning；注意 `build.rs` 可能回写缓存戳到 `frontend/index.html`、`frontend/app.js`、`tauri.conf.json`、`src/tray.rs`——这些文件如有变动属正常，随本次一并提交。

- [ ] **Step 3: 手测清单（用户执行，需真实发布环境）**

1. 安装版：点「立即更新」→ 进度条按百分比推进，文案依次出现「下载 → 校验 → 安装」，安装器接管后程序自动关闭。
2. 绿色版：点「立即更新」→ 同样有真实百分比；更新完成后自动重启为新版本，exe 同目录不留 `.download` 残留。
3. 更新说明：`###` / `-` / `**` 均渲染为标题 / 列表 / 粗体，无原始字符。
4. 断网重试：下载中途断网 → 文案切「网络中断，正在重试（第 N 次）…」；彻底失败 → 进度条停在原位并显示「更新失败：原因」，按钮恢复可点。
5. 无更新时点「立即更新」（需构造）→ 显示「更新失败：已是最新版本」，不崩。

- [ ] **Step 4: 提交**

```powershell
git add src-tauri/src/update.rs src-tauri/src/main.rs frontend/app.js frontend/index.html frontend/styles.css scripts/test_update_ui.js CHANGELOG.md
git add -u
git commit -m "feat: 自动更新显示真实下载进度并支持断网重试，更新说明改为 markdown 排版" -m "安装版改用 updater 分步 download/install 包重试循环；绿色版把下载从助手提前到主程序（Range 续传 + SHA256 校验）后以 --local-file 交接；前端新增 renderMarkdown 与进度条组件，订阅 update-progress 事件。"
```

---

## Self-Review 记录

- **Spec 覆盖**：§4 渲染规则 → Task 1；§5 事件协议与节流 → Task 2/3/5/6；§6 Fetcher/重试 → Task 3；§7 安装版流程（含删除不可达 `app.restart()`）→ Task 5；§8 绿色版顺序调换与本地交接 → Task 4/6；§9 前端 UI → Task 2；§10 测试 → 各 Task 的 Test 文件；§14 CHANGELOG → Task 7。
- **占位符扫描**：无 `TODO` / `...` / 「略」；所有代码块可直接粘贴。
- **类型一致性**：`emit_progress(&AppHandle, &str, u64, Option<u64>, u32)` 与两处调用一致；`download(on_chunk: FnMut(usize, Option<u64>), on_finish: FnOnce())` 与安装版闭包签名一致；`install(&self, impl AsRef<[u8]>)` 传 `&bytes` 合法；`pending.download` 为 `&self`，重试循环内可重复调用；前端 `paintUpdateProgress` 读的全局（`updateProgress` / `updateInfo`）均已进入测试 ctx。
