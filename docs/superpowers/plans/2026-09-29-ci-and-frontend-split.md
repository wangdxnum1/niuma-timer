# 实施计划：CI 自动化 + 前端代码拆分

> **状态（2026-10-02 补记）：已完成**；但本文档描述的是当时的形态，其中两处此后被推翻——① 导航从「3 个顶部标签」改为 4 项竖排侧栏（`nav.rail`）；② `scripts/lib/split_check.js` 已删除。勾选框保留原样作为执行记录。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkboxes so progress can be tracked while executing. Do not skip verification steps. Each task ends with a commit.

- **Spec**: docs/superpowers/specs/2026-09-29-ci-and-frontend-split-design.md（已批准；commit 4790cdf + 修订 b3ac04d）
- **仓库**: github.com/wangdxnum1/niuma-timer（公开，Windows-only Tauri v2 托盘应用）
- **日期**: 2026-09-29

## Goal

1. 建立 GitHub Actions CI 安全网：前端测试（ubuntu）+ Rust fmt/clippy/test（windows）+ build.rs 回写漂移守卫。
2. 将 frontend/app.js（2972 行 / 115 KB）按「纯连续切割」拆为 10 个经典 script 块，字节级还原，行为零变化。

## Architecture / Tech Stack

- 前端：原生 HTML/CSS/JS，**无打包器**。经典多 `<script>` 共享全局词法环境——跨块函数调用运行时解析，无顺序问题；唯一硬约束是「加载序满足声明期求值」。
- 拆分方式：**纯连续切割，不移动任何行**。已穷举核验全部顶层语句（62 处列 0 挂载 + 33 处顶层 let/var + 1 处顶层 for），跨块引用全部指向更早加载的块。
- CI：ubuntu 跑前端测试（全部为纯 Node 文件断言，无 cargo 依赖）；windows-latest 跑 Rust 三件套；构建后漂移守卫检测 build.rs 回写。
- Rust 侧本期不动（main.rs 拆分、发布流水线上云为二期）。

## 全局约束（每个任务执行者必读）

1. **PowerShell**：`git commit` 只用单行 `-m`（heredoc 不可用）；多条命令用 `;` 链接。
2. **缓存戳纪律**：`?v=` 值与 `const FE_VER = "v<hex>"` 由 src-tauri/build.rs 在 cargo build 时自动回写，**禁止手改值**；改前端后必须 cargo build 同步并提交回写结果。
3. **经典多 script**：10 块按 CHUNKS 加载序在 index.html 依次加载，**boot.js 必须最后**。
4. **纯连续切割**：只切行、不移动行；每块文件 = app.js 对应行区间逐行一致（CRLF 归一为 LF，文件以单个换行结尾）。
5. **run_all.js**：只扫 scripts/ 顶层 `test_*.js`、首败即停（scripts/lib/ 不会被发现）。过渡期（Task 5-13）契约测试预期红 → **run_all 全量验证推迟到 Task 14**。
6. **PR-2 分支在 Task 14 完成前不 push**（避免 GitHub 上出现预期红 CI）。
7. 提交信息遵循仓库惯例：`type(scope): 中文描述`。

## 前置事实（已核实，直接使用，无需重新采集）

### app.js 物理行号 → 块映射（1 基闭区间）

| 块 | app.js 行区间 | 行数 | 备注 |
|---|---|---|---|
| core.js | [1, 46] | 46 | `$`、TAURI、FE_VER、状态 let——必须最先加载 |
| settings.js | [47, 316] | 270 | |
| hero.js | [317, 559] | 243 | |
| overtime.js | [560, 901] | 342 | 含 showConfirm/hideConfirm/confirmResolve(L867) |
| monitor.js | [902, 1553] | 652 | settings 域绑定物理在此段，但引用更早加载的 settings——已验证安全 |
| storage.js | [1554, 1786] | 233 | overtime 域绑定物理在此段，但引用更早加载的 overtime——已验证安全 |
| boot.js | [1787, 1888] + [2507, 2727] + [2947, 2972] | 349 | 三段按出现顺序拼接；含顶层 for(L2627) 与启动序列(L2948-2963)——必须最后加载 |
| bill.js | [1889, 2132] | 244 | 含 fmtMoney、QUIPS |
| insights.js | [2133, 2506] | 374 | |
| update.js | [2728, 2946] | 219 | 自带 6 个绑定(L2915-2933)，同块自洽 |

合计 2972 行；12 个物理区间无缝平铺 [1, 2972]，连续性已验证。

### 两种顺序的刻意差异（不要混淆）

- **CHUNKS（加载序）** = index.html `<script>` 标签序 = feSource() 拼接序：
  `core, settings, hero, overtime, monitor, bill, insights, storage, update, boot`
- **RANGES（物理序）** = app.js 自上而下的切割顺序：
  `core, settings, hero, overtime, monitor, storage, [boot①], bill, insights, [boot②], update, [boot③]`
- 切割执行按**物理序**自上而下（顺序切割自然）；**加载序**由依赖分析确定（bill/insights 物理在 storage 之后但先加载，其挂载依赖已全部验证）。
- split_check 按 RANGES（物理）比对；fe_sources 按 CHUNKS（加载）聚合。两者键序不同是刻意设计。

### 顶层语句盘点（已穷举核验）

- 列 0 挂载 62 处（`$(...)` / `document.*` / `window.*`），引用目标全部指向更早加载的块。
- 顶层 let/var 33 处，全部随所在区间自然归位，**无需移动任何行**。
- 顶层控制流仅 1 处：L2627 `for (const k of Object.keys(DAY_NAV)) updateDayNav(k);`，位于 boot 区间 [2507, 2727] 内。
- 列 0 无 if/while/switch/try/return/裸调用/括号开头行（grep 已核验）。

### 测试口径

- scripts/ 顶层共 27 个测试；**22 个**读 app.js 源码文本（Task 14 换读源），**5 个豁免**不读 app.js：test_build_info / test_readme / test_hover_card / test_hover_interaction / test_lock_order。
- 全部测试为纯 Node 文件断言（readFileSync + 正则），无 execSync/spawnSync cargo → ubuntu CI 可跑。
- 22 个测试的 4 种 app.js 读源模式（Task 14 以 `git grep -n "app\.js" -- scripts/` 实际定位）：
  1. `fs.readFileSync(path.join(ROOT, "frontend", "app.js"), "utf8")`
  2. `read("frontend/app.js")` 之类局部封装
  3. `path.join(__dirname, "../frontend/app.js")`
  4. `path.join(root, "frontend/app.js")`

### 关键文件行号

- src-tauri/build.rs **L25**：`const TARGETS: &[&str] = &["../frontend/index.html", "../frontend/app.js", "tauri.conf.json", "src/tray.rs"];`
- src-tauri/build.rs **L285**：`println!("cargo:rerun-if-changed=../frontend");`（目录级，已覆盖 js/ 子目录）
- src-tauri/build.rs **L288**：`println!("cargo:rerun-if-changed=../frontend/app.js");`（Task 13 删除此行）
- frontend/index.html **L817**：`<script src="app.js?v=3c970508"></script>`（当前唯一 script 标签）
- 构建产物：`src-tauri/target/debug/niuma-timer.exe`（无 `[[bin]]` 覆盖，冒烟用）
- 当前 git：main 领先 origin/main 2 个 docs 提交，工作树干净。

## Interfaces 约定

- `require("./lib/fe_sources")` → `{ CHUNKS, feSource, readIndexHtml }`。
- `node scripts/lib/split_check.js [chunk.js...]`：exit 0 全过；exit 1 有差异；未切出的块输出 `[skip]` 不计失败；app.js 已删除时提示退役并 exit 0。
- `node scripts/test_frontend_split.js`：4 项契约检查，exit 0/1。
- **过渡期契约测试预期红速查表**（只允许这些红，出现其他红 = 真问题，停下修复）：

| 阶段 | 预期红项 |
|---|---|
| Task 5（基建后） | ①index.html 缺 10 标签 ②app.js 存在 ③core.js 不存在 |
| Task 6-12（逐块切） | ①缺标签 ②app.js 存在（FE_VER 与白名单项必须绿） |
| Task 13（接线后） | ②app.js 存在 |
| Task 14（删 app.js 后） | 无——全绿 |

---

# PR-1：分支 `ci/safety-net`——CI 安全网

## Task 1: cargo fmt 一次性格式化

**Files**: src-tauri/**/*.rs（fmt 自动改写，无手工编辑）

**Steps**:

- [ ] `git push origin main`（发布 2 个 docs 提交，避免 PR 混入无关 diff）
- [ ] `git checkout -b ci/safety-net`
- [ ] `cd src-tauri` 后执行 `cargo fmt`
- [ ] `cargo fmt --all -- --check` 退出码 0
- [ ] `cargo test` 全过（fmt 不改语义；若失败停下排查，不得跳过）
- [ ] `git status` 确认改动仅在 src-tauri/ 的 .rs 文件
- [ ] 提交：`git add src-tauri ; git commit -m "style: cargo fmt 一次性格式化（CI fmt --check 前置）"`

**Done**: fmt --check 绿、cargo test 全过、提交完成。

## Task 2: clippy 存量清零

**Files**: src-tauri/**/*.rs（按 warning 修复）

**Steps**:

- [ ] `cd src-tauri` 后执行 `cargo clippy --all-targets` 并统计 warning 数
- [ ] ≤15 条：全部修复，目标 0 豁免
- [ ] \>15 条：先修机械易修项；剩余逐条加局部 `#[allow(...)]` + 行尾注释说明原因，并在 Task 3 的 ci.yml clippy 步骤上方以注释列出豁免清单
- [ ] `cargo clippy --all-targets -- -D warnings` 退出码 0（这正是 CI 判据）
- [ ] `cargo test` 仍全过
- [ ] 提交：`git commit -am "fix: clippy 存量清零（CI -D warnings 前置）"`

**Done**: `cargo clippy --all-targets -- -D warnings` 通过。

## Task 3: 写 CI workflow + push + 开 PR

**Files**: .github/workflows/ci.yml（新增）

完整内容（若 Task 2 有豁免，在 clippy 步骤上方加注释清单）：

```yaml
name: CI

on:
  push:
    branches: [main]
    paths-ignore:
      - "**.md"
      - "docs/**"
  pull_request:
    paths-ignore:
      - "**.md"
      - "docs/**"

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  frontend-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
      - name: Run frontend tests
        run: node scripts/run_all.js

  rust-checks:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      - name: cargo fmt
        run: cargo fmt --all -- --check
        working-directory: src-tauri
      - name: cargo clippy
        run: cargo clippy --all-targets -- -D warnings
        working-directory: src-tauri
      - name: cargo test
        run: cargo test
        working-directory: src-tauri
      - name: Drift guard (build.rs 回写检测)
        shell: pwsh
        run: |
          $status = git status --porcelain -- frontend src-tauri
          if ($status) {
            Write-Host $status
            throw "build.rs 回写了未提交的自动改动：本地执行 cargo build，将 frontend/ 与 src-tauri/ 的自动变更一并提交后再推送"
          }
```

**Steps**:

- [ ] 将上面 YAML 原样落盘为 .github/workflows/ci.yml
- [ ] `git add .github/workflows/ci.yml ; git commit -m "ci: GitHub Actions——前端测试 + Rust 三件套 + build.rs 漂移守卫"`
- [ ] `git push -u origin ci/safety-net`
- [ ] 开 PR：若 gh 可用 `gh pr create --base main --title "ci: CI 安全网（fmt/clippy/test + 漂移守卫）" --body "fmt 一次性格式化 + clippy 清零 + CI workflow + 漂移守卫"`；否则打开 https://github.com/wangdxnum1/niuma-timer/compare/main...ci/safety-net 手动建 PR
- [ ] 等 CI 完成：`gh run watch`（或 PR 页轮询），frontend-tests 与 rust-checks 两 job 都必须绿

**Done**: PR 存在且两 job 绿。

## Task 4: 漂移守卫负向自测 + 合并 PR-1

**Steps**:

- [ ] 在 PR 分支上：给 frontend/app.js 顶部加一行注释 `// drift-guard-self-test`（**不做**本地缓存戳同步）
- [ ] `git commit -am "test: 漂移守卫负向自测（预期 CI 红）" ; git push`
- [ ] 等 CI：rust-checks 必须红，失败步骤为 Drift guard，输出含 index.html / app.js / tauri.conf.json / src/tray.rs 的回写清单（build.rs 检测到指纹变化回写了缓存戳）
- [ ] `git revert HEAD ; git push`（revert 该提交，恢复字节一致）
- [ ] 等 CI：全绿
- [ ] 合并 PR-1（`gh pr merge ci/safety-net --merge` 或网页 merge）
- [ ] `git checkout main ; git pull ; git branch -d ci/safety-net`

**Done**: PR-1 合并；漂移守卫被证实能抓 build.rs 回写。

---

# PR-2：分支 `refactor/frontend-split`——10 块拆分

前置：PR-1 已合并，本地 main 已 `git pull`。

## 通用切块流程（Task 6-12 每一块都必须完整走一遍）

对每个块 `<name>`（区间见前置事实表）：

1. **边界核对**：Read app.js 区间首尾 ±3 行（offset = 区间起点 - 3，limit = 区间行数 + 6），确认起点行是上一块末尾语句的结束、终点行是本块完整语句的结束。若与 RANGES 不符（例如中途跑过 cargo build 改动了行），先修正前置事实表与 split_check.js 中的 RANGES 再继续。
2. **整段读取**：Read 精确区间（offset = 起点，limit = 区间行数）。
3. **落盘**：Write `frontend/js/<name>.js`，内容逐行等于所读区间，文件以单个换行结尾。
4. **语法检查**：`node --check frontend/js/<name>.js` 退出码 0。
5. **还原校验**：`node scripts/lib/split_check.js <name>` 输出 `[ok]`。
6. **契约测试**：`node scripts/test_frontend_split.js`——必然红，但只允许「缺标签 / app.js 存在」两类（见预期红速查表）；若出现 FE_VER 或白名单类失败 = 真问题，停下修复（白名单误报 → 按实际顶层语句扩充 test_frontend_split.js 的 TOP_OK 并在提交说明中记录）。
7. **提交**：`git add frontend/js/<name>.js ; git commit -m "refactor(frontend): 切出 js/<name>.js（app.js L<a>-L<b>）"`。

## Task 5: 契约基建（聚合器 + 还原校验 + 契约测试）

**Files**: scripts/lib/fe_sources.js（新增）、scripts/lib/split_check.js（新增）、scripts/test_frontend_split.js（新增）

三个文件全文如下，原样落盘。

**scripts/lib/fe_sources.js**：

```js
// 前端源码聚合器：替代原先对单文件 frontend/app.js 的读取。
// CHUNKS 为「加载序」（与 frontend/index.html 中 <script> 标签顺序一致）。
// 注意：与 app.js 的物理行序不同——storage 物理在 bill/insights 之前，
// 但按加载序在其后加载（依赖分析结论见 spec 边界原则）。
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..", "..");
const FRONTEND = path.join(ROOT, "frontend");

// 加载序：boot.js 必须最后
const CHUNKS = [
  "core.js", "settings.js", "hero.js", "overtime.js", "monitor.js",
  "bill.js", "insights.js", "storage.js", "update.js", "boot.js",
];

// 按加载序拼接全部前端块。各块文件均以单个换行结尾，
// 用空串拼接即可完整保留每块字节内容（不引入多余空行）。
function feSource() {
  return CHUNKS.map((n) => fs.readFileSync(path.join(FRONTEND, "js", n), "utf8")).join("");
}

function readIndexHtml() {
  return fs.readFileSync(path.join(FRONTEND, "index.html"), "utf8");
}

module.exports = { CHUNKS, feSource, readIndexHtml };
```

**scripts/lib/split_check.js**：

```js
// 过渡期字节级还原校验：按 RANGES 从 frontend/app.js 切出期望行，
// 与 frontend/js/<name>.js 逐行比对（CRLF 归一化 + 忽略 FE_VER 值行）。
// app.js 删除后本工具退役（契约由 scripts/test_frontend_split.js 接管）。
//
// 用法：node scripts/lib/split_check.js [chunk.js ...]
//   不带参数 = 校验全部块；带参数 = 只校验指定块。
// 退出码：0 全部通过；1 存在差异或参数非法。
const fs = require("fs");
const path = require("path");

const FRONTEND = path.join(__dirname, "..", "..", "frontend");

// 物理行号区间（1 基闭区间）。键顺序无意义，区间值是 app.js 的物理行号。
// boot.js 由三段非连续区间按出现顺序拼接。
const RANGES = {
  "core.js": [[1, 46]],
  "settings.js": [[47, 316]],
  "hero.js": [[317, 559]],
  "overtime.js": [[560, 901]],
  "monitor.js": [[902, 1553]],
  "bill.js": [[1889, 2132]],
  "insights.js": [[2133, 2506]],
  "storage.js": [[1554, 1786]],
  "update.js": [[2728, 2946]],
  "boot.js": [[1787, 1888], [2507, 2727], [2947, 2972]],
};

// 构建回写会改 FE_VER 的值但不改结构，比对时忽略该值行。
const isFeVerLine = (l) => /^const FE_VER = /.test(l);

function toLines(text) {
  return text.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n");
}

function main() {
  const appPath = path.join(FRONTEND, "app.js");
  if (!fs.existsSync(appPath)) {
    console.log("frontend/app.js 不存在——拆分已完成，split_check 退役。");
    process.exit(0);
  }
  const appLines = toLines(fs.readFileSync(appPath, "utf8"));

  const requested = process.argv.slice(2);
  const targets = requested.length ? requested : Object.keys(RANGES);
  let fail = false;

  for (const name of targets) {
    const ranges = RANGES[name];
    if (!ranges) {
      console.error(`[error] 未知块名：${name}（合法值：${Object.keys(RANGES).join(", ")}）`);
      fail = true;
      continue;
    }
    const chunkPath = path.join(FRONTEND, "js", name);
    if (!fs.existsSync(chunkPath)) {
      console.log(`[skip] ${name}（尚未切出）`);
      continue;
    }
    const expected = ranges
      .flatMap(([a, b]) => appLines.slice(a - 1, b))
      .filter((l) => !isFeVerLine(l));
    const actual = toLines(fs.readFileSync(chunkPath, "utf8")).filter((l) => !isFeVerLine(l));

    let diffAt = -1;
    for (let i = 0; i < Math.max(expected.length, actual.length); i++) {
      if (expected[i] !== actual[i]) { diffAt = i; break; }
    }
    if (diffAt === -1) {
      console.log(`[ok] ${name}（${actual.length} 行）`);
    } else {
      fail = true;
      console.error(`[FAIL] ${name} 第 ${diffAt + 1} 行不一致`);
      console.error(`  期望: ${JSON.stringify(expected[diffAt])}`);
      console.error(`  实际: ${JSON.stringify(actual[diffAt])}`);
    }
  }
  process.exit(fail ? 1 : 0);
}

main();
```

**scripts/test_frontend_split.js**：

```js
// 前端拆分契约测试（多 script 结构守卫），4 项检查：
//  1. index.html 按 CHUNKS 加载序引用 js/<name>?v=
//  2. app.js 已删除且 frontend/ 内无残留引用
//  3. const FE_VER 仅在 core.js 声明一次
//  4. 非 boot 块顶层仅允许声明与纯挂载（声明-挂载白名单，正则近似）
const fs = require("fs");
const path = require("path");
const { CHUNKS, readIndexHtml } = require("./lib/fe_sources");

const FRONTEND = path.join(__dirname, "..", "frontend");
const JS_DIR = path.join(FRONTEND, "js");

function walk(dir, exts, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p, exts, out);
    else if (exts.includes(path.extname(e.name))) out.push(p);
  }
  return out;
}

const failures = [];

// -- 1. index.html 标签按加载序排列 ----------------------------------------
const html = readIndexHtml();
let pos = -1;
for (const name of CHUNKS) {
  const needle = `js/${name}?v=`;
  const at = html.indexOf(needle);
  if (at === -1) {
    failures.push(`index.html 缺少 <script src="${needle}...">（CHUNKS 加载序要求全部 10 个标签）`);
  } else if (at < pos) {
    failures.push(`index.html 中 ${needle} 出现在前一个标签之前（违反加载序）`);
  } else {
    pos = at;
  }
}

// -- 2. app.js 已删除且无残留引用 ------------------------------------------
if (fs.existsSync(path.join(FRONTEND, "app.js"))) {
  failures.push("frontend/app.js 仍存在——拆分契约要求删除单文件入口");
}
for (const f of walk(FRONTEND, [".html", ".css", ".js"])) {
  if (f.replace(/\\/g, "/").endsWith("frontend/app.js")) continue;
  const lines = fs.readFileSync(f, "utf8").split("\n");
  lines.forEach((line, i) => {
    if (/app\.js/.test(line)) {
      failures.push(`${path.relative(FRONTEND, f)}:${i + 1} 残留 app.js 引用：${line.trim()}`);
    }
  });
}

// -- 3. FE_VER 仅在 core.js ------------------------------------------------
const corePath = path.join(JS_DIR, "core.js");
if (!fs.existsSync(corePath)) {
  failures.push("frontend/js/core.js 不存在（FE_VER 必须在 core.js 声明）");
} else {
  for (const name of CHUNKS) {
    const p = path.join(JS_DIR, name);
    if (!fs.existsSync(p)) continue;
    const hits = fs.readFileSync(p, "utf8").split("\n")
      .filter((l) => /^const FE_VER = /.test(l)).length;
    if (name === "core.js") {
      if (hits !== 1) failures.push(`core.js 应声明且仅声明一次 const FE_VER（当前 ${hits} 处）`);
    } else if (hits > 0) {
      failures.push(`${name} 不允许声明 const FE_VER（仅 core.js 可声明）`);
    }
  }
}

// -- 4. 非 boot 块顶层白名单（声明-挂载） ------------------------------------
// 顶层（列 0）非空行仅允许：声明（function/async function/const/let/var）、
// 纯挂载（$、document.*、window.*）、块结尾（}/)）、注释。裸业务调用即红。
const TOP_OK = /^(function\b|async function\b|const\b|let\b|var\b|\$|document\.|window\.|\}|\)|\/\/|\/\*| \*|\*\/)/;
for (const name of CHUNKS.slice(0, -1)) {
  const p = path.join(JS_DIR, name);
  if (!fs.existsSync(p)) continue;
  const bad = [];
  fs.readFileSync(p, "utf8").split("\n").forEach((line, i) => {
    if (line.length > 0 && !TOP_OK.test(line)) bad.push(`  L${i + 1}: ${line.trim().slice(0, 80)}`);
  });
  if (bad.length) failures.push(`${name} 顶层出现白名单之外的语句：\n${bad.join("\n")}`);
}

if (failures.length) {
  console.error(`前端拆分契约检查失败（${failures.length} 项）：`);
  for (const f of failures) console.error(`- ${f}`);
  process.exit(1);
}
console.log(`前端拆分契约检查通过（${CHUNKS.length} 块）`);
```

**Steps**:

- [ ] `git checkout -b refactor/frontend-split`（自最新 main）
- [ ] 落盘三个文件（上面全文原样）
- [ ] `node scripts/lib/split_check.js` → 10 个 `[skip]`，exit 0
- [ ] `node scripts/test_frontend_split.js` → exit 1，失败项恰为：缺 10 标签、app.js 存在、core.js 不存在（预期红，记录输出）
- [ ] 过渡期不跑 `node scripts/run_all.js`（首败即停会撞预期红的契约测试，Task 14 才全量跑）
- [ ] 提交：`git add scripts/lib/fe_sources.js scripts/lib/split_check.js scripts/test_frontend_split.js ; git commit -m "test(frontend): 拆分契约基建——fe_sources 聚合器 + split_check 还原校验 + 契约测试"`

**Done**: 三文件就位，契约测试处于预期红，工作树干净。

## Task 6: 切出 core.js + settings.js

按通用切块流程切 core.js（[1, 46]）与 settings.js（[47, 316]），各一次提交。

注意：core.js 含 FE_VER 声明行，切出后契约测试第 3 项（FE_VER 检查）应转绿。

## Task 7: 切出 hero.js + overtime.js

按通用切块流程切 hero.js（[317, 559]）与 overtime.js（[560, 901]），各一次提交。

## Task 8: 切出 monitor.js

按通用切块流程切 monitor.js（[902, 1553]），一次提交。

## Task 9: 切出 storage.js

按通用切块流程切 storage.js（[1554, 1786]），一次提交。

注意：storage 物理序在此处，但加载序在 bill/insights 之后——只影响 Task 13 的标签顺序，不影响本任务。

## Task 10: 切出 bill.js + insights.js

按通用切块流程切 bill.js（[1889, 2132]）与 insights.js（[2133, 2506]），各一次提交。

## Task 11: 切出 update.js

按通用切块流程切 update.js（[2728, 2946]），一次提交。

## Task 12: 切出 boot.js（三段拼接）

按通用切块流程切 boot.js，区别：区间为三段 [1787, 1888] + [2507, 2727] + [2947, 2972]，三次 Read 按出现顺序拼接为一个文件后落盘。split_check 对多区间 flatMap 比对。

- 提交信息：`refactor(frontend): 切出 js/boot.js（app.js L1787-1888 + L2507-2727 + L2947-2972，最后加载）`

**Task 12 完成后自查**:

- [ ] 10 个块文件全部存在于 frontend/js/
- [ ] `node scripts/lib/split_check.js` 全部 `[ok]`
- [ ] `node scripts/test_frontend_split.js` 只剩两类预期红（缺 10 标签 + app.js 存在），FE_VER 与白名单项全绿

## Task 13: index.html 多标签 + build.rs TARGETS + 缓存戳同步 + 冒烟

**Files**: frontend/index.html（L817）、src-tauri/build.rs（L25、L288）；构建回写：tauri.conf.json、src/tray.rs、js/core.js

**Steps**:

- [ ] index.html L817 的 `<script src="app.js?v=3c970508"></script>` 替换为以下 10 个标签（按 CHUNKS 加载序；hex 占位即可，构建后由 build.rs 统一回写）：

```html
<script src="js/core.js?v=3c970508"></script>
<script src="js/settings.js?v=3c970508"></script>
<script src="js/hero.js?v=3c970508"></script>
<script src="js/overtime.js?v=3c970508"></script>
<script src="js/monitor.js?v=3c970508"></script>
<script src="js/bill.js?v=3c970508"></script>
<script src="js/insights.js?v=3c970508"></script>
<script src="js/storage.js?v=3c970508"></script>
<script src="js/update.js?v=3c970508"></script>
<script src="js/boot.js?v=3c970508"></script>
```

- [ ] build.rs L25：`"../frontend/app.js"` 改为 `"../frontend/js/core.js"`（FE_VER 落点移到 core.js）
- [ ] build.rs L288：删除 `println!("cargo:rerun-if-changed=../frontend/app.js");`（L285 目录级条目已覆盖 js/）
- [ ] `git grep -n "app\.js" -- src-tauri/` 无剩余真实引用（注释提及可顺手改写）
- [ ] `cd src-tauri` 后 `cargo build`——build.rs 重算指纹并回写缓存戳
- [ ] `git status` 确认回写文件集 ⊆ {frontend/index.html, frontend/js/core.js, src-tauri/tauri.conf.json, src-tauri/src/tray.rs}；抽查 index.html 10 个标签 hex 一致，且 core.js `const FE_VER = "v<hex>"` 与之一致
- [ ] `node scripts/lib/split_check.js` 仍全 `[ok]`（FE_VER 值行被忽略）
- [ ] 冒烟（先退出正在运行的牛马计时器实例——single-instance 会拒起新实例）：运行 `src-tauri/target/debug/niuma-timer.exe`，逐项确认：
  - 主界面金额/工时数字 tick 跳动
  - 托盘菜单可打开
  - 设置页保存生效（toast 出现）
  - 账单页 4 个 tab 可切换
  - 监控页今日/本周/月三视图可切换
  - 更新页可打开
  - 备份/还原 UI 可见
  确认后退出进程。任何一项异常 → 停下排查（优先怀疑块顺序/缺块）
- [ ] 单跑豁免测试确认不受影响：`node scripts/test_lock_order.js ; node scripts/test_build_info.js` 全过
- [ ] 提交（先 `git status` 审查范围，将手改文件与回写文件一并 add）：`git commit -m "refactor(frontend): index.html 接入 10 块多 script，build.rs TARGETS 切到 js/core.js 并同步缓存戳"`

**Done**: 缓存戳一致、冒烟清单全过、豁免测试绿。

## Task 14: 22 测试换读源 + 删除 app.js + run_all 全绿 + push + 开 PR

**Files**: scripts/test_*.js（22 个）、frontend/app.js（删除）、scripts/lib/split_check.js（删除）、缓存戳回写集

**Steps**:

- [ ] `git grep -ln "app\.js" -- scripts/` 列出全部待改文件（预期 22 个测试 + scripts/lib/split_check.js）
- [ ] 逐个把 app.js 读源替换为 `require("./lib/fe_sources").feSource()`（按各文件现有风格内联；4 种读源模式见前置事实）。只改读源处，断言逻辑不动
- [ ] 契约测试第 2 项若报 frontend/ 内残留 "app.js" 字样（多为注释），按输出逐行清理
- [ ] 删除 `frontend/app.js` 与 `scripts/lib/split_check.js`（用 DeleteFile 工具）
- [ ] **关键**：`cd src-tauri` 后 `cargo build`——app.js 删除使指纹变化，build.rs 回写缓存戳（跳过此步 CI 漂移守卫必红）
- [ ] `node scripts/test_frontend_split.js` → 全绿 exit 0
- [ ] 逐个单跑被改的 22 个测试（`node scripts/test_xxx.js`），全过；若个别测试因块序（feSource 按加载序拼接）断言失败：该断言若是跨块物理序敏感的 indexOf 比较，改用 per-chunk 读取重写断言并在提交说明记录
- [ ] `node scripts/run_all.js` → 28 个脚本一次性全绿
- [ ] `git status` 审查范围（应仅含 22 个测试、frontend/app.js 删除、split_check.js 删除、缓存戳回写集），然后 `git add -A ; git commit -m "refactor(frontend): 22 个测试换 feSource() 读源，删除单文件 app.js 与过渡期 split_check"`
- [ ] `git push -u origin refactor/frontend-split`；开 PR：`gh pr create --base main --title "refactor(frontend): app.js 拆分为 10 个 script 块" --body "纯连续切割，split_check 字节级还原校验，契约测试守卫"`（或 compare 网页手动建）
- [ ] 等 CI：两 job 全绿（契约已绿、漂移已同步）

**Done**: run_all 28 全绿、CI 绿、PR 开出。

## Task 15: README 双语同步 + 最终验证 + 合并 PR-2

**Files**: README.md、README.zh-CN.md

**Steps**:

- [ ] `git grep -n "app\.js" -- README.md README.zh-CN.md` 定位全部提及（项目结构树、测试说明等）
- [ ] 结构树：`frontend/app.js` 条目改为 `frontend/js/` 下 10 个块（按加载序列出文件名）；测试说明补一句「前端源码经 scripts/lib/fe_sources.js 按加载序聚合后断言」
- [ ] `node scripts/test_readme.js` 全过
- [ ] `git commit -am "docs: README 项目结构与测试说明同步前端拆分" ; git push`
- [ ] 最终验证四件套：
  - `node scripts/run_all.js` 全绿
  - `cd src-tauri` 后 `cargo fmt --all -- --check ; cargo clippy --all-targets -- -D warnings ; cargo test` 全过
  - `git status` 干净
  - PR 页 CI 全绿
- [ ] 合并 PR-2；`git checkout main ; git pull ; git branch -d refactor/frontend-split`
- [ ] 收尾汇报：两个 PR 链接 + run_all/CI 结果

**Done**: PR-2 合并，main 处于 CI 全绿 + 全部测试绿状态。
