# 设计：PDB 上传健壮性（发布链解耦 + 上传加固）

日期：2026-09-29
状态：待用户批准
所属迭代：无（迭代已停，本次为回顾式排查后的一次定点加固）
关联：
- 发布链：`release.bat` / `scripts/publish_release.py` / `build.bat`
- 测试：`scripts/test_publish_release.py`
- 前置设计：`docs/superpowers/specs/2026-09-28-v1.4.1-polish-design.md`（发布后验证已落地）

## 0. 背景与定位

迭代已停。回顾发布与分发链后，聚焦到一个具体痛点：**PDB（调试符号）上传的健壮性**。

现状（已核实）：

1. `build.bat` 把 release PDB（原始 **146.8 MB**）压成 `niuma-timer-<ver>-portable.pdb.zip`（**32.2 MB**，Release 最大资产）放入 `bin\package`；
2. `publish_release.py` 把 PDB 并入 `assets`（L449-455、L457），上传失败即 `failed=True`，进而**阻断 `PATCH draft:false`**——一个**非运行时依赖**能卡死整次发布；
3. `release.bat` 有 **gh CLI / python / browser 三条发布路径**，gh 为首选（L334-338），PDB 同样被塞进 gh 的 `!ASSETS!`（L329）：**两条自动化路径都存在同样的耦合**；
4. 上传为**一次性整包 POST**（`f.read()` 全量入内存 + 单请求），官方 API **不支持分块/断点续传**；本地代理下偶发远端重置（Errno 10054）。

定位：以**最小改动**让「PDB 失败绝不阻断发布」，并让 PDB 上传本身更耐受瞬时网络错误。不做代码签名、不做信任链、不做 CI 化。

### 成功标准

- **G1** PDB 上传失败/缺失时，release 仍能正常公开（`PATCH draft:false` 成功）；
- **G2** PDB 上传对瞬时网络错误具备重试/退避能力，且能区分「瞬时」与「永久」错误；
- **G3** 运行时必需资产（安装包 / `.sig` / `latest.json` / `SHA256SUMS.txt`）缺失或上传失败时，**仍必须 fail-closed**（退出码 1、不公开）。

## 1. 决策表

| 决策点 | 结论 |
|---|---|
| 方案范围 | 只做 **A（解耦关键路径）+ B（上传加固）**；不做 C（改 Cargo 调试信息级别） |
| PDB 在流程中的位置 | **整体移出主发布流程**：默认发布只处理必需资产，PDB 一律走 `--pdb-only` |
| 流式上传 | **放弃**。改动 `gh.call` body 类型会波及现有测试断言并改变 chunked/Content-Length 语义，且离线 CI 无法验证 GitHub 端点真实行为；32 MB 入内存非失败主因 |
| 加固内容（B） | 重试次数 3→4、指数退避 + 抖动、错误分类（瞬时重试 / 永久不重试） |
| 补传方式 | **发布后自动再跑一次**（用户选定）：`release.bat` 在发布成功后 best-effort 调 `--pdb-only` |
| gh 路径的补传 | 统一走 python `--pdb-only`（同一个加固上传器）；无 `PYEXE` 时退化为 `gh release upload --clobber` |
| 预发布 PDB 存在性门禁 | **保留** `release.bat` L217-223：它约束的是「构建是否产出 PDB」，与「上传」职责不同 |
| `build.bat` | **不改**：PDB 仍照旧压缩入包 |

## 2. 分项设计

### 2.1 `publish_release.py` 默认模式：只发必需资产

- 仍**收集** PDB 到独立列表 `pdbs`（供 `--pdb-only` 复用筛选逻辑），但**不再并入 `assets`**：`assets = bins + sigs`（+ `latest.json` + `SHA256SUMS.txt`）。
- 原 L449-455 的 `.pdb/.pdb.zip` 分支保留为独立收集；把「为何不在主流程」写成注释（PDB 不是运行时依赖，失败不得阻断发布）。
- 主流程其余逻辑（草稿复用、按 id 取终态核对、`PATCH draft:false`、发布后 tags 验证）**不变**；因 PDB 已不在 `assets`，L505 的「必需资产子集」校验天然不含 PDB。

### 2.2 新增 `--pdb-only` 模式

新增 `argparse` 开关 `--pdb-only`（`action="store_true"`），语义为「仅补传调试符号」：

1. **跳过** 备注生成、`build_latest_json`、SHA256SUMS 写入、草稿创建、`PATCH`、发布后验证；
2. 复用 token 与 `build_opener()`；
3. 查找目标 release：`GET /releases/tags/{tag}`
   - 200 → **允许操作已公开的 release**（这是与默认模式的关键差异；默认模式 L386-388 会拒绝已公开 release）；
   - 404 → 退化为扫描 release 列表按 `tag_name` 复用草稿；
   - 都找不到 → 报错退出码 1（没有可挂载 PDB 的 release）；
4. 仅对 `pdbs` 执行上传，复用 `existing` 幂等跳过（已上传则 `[skip]`）；
5. 退出码：全部成功 0；任一 PDB 永久失败 1（供调用方告警，**但调用方不得据此判发布失败**）。

### 2.3 上传加固（B）

抽出统一上传 helper（默认模式与 `--pdb-only` 共用）；新增 `import random`：

```python
def upload_with_retry(gh, upload_base, path, attempts=4):
    """上传单个资产，成功返回 True。仅在瞬时错误上重试。"""
    name = os.path.basename(path)
    with open(path, "rb") as f:
        data = f.read()
    ctype = mimetypes.guess_type(name)[0] or "application/octet-stream"
    url = "%s?name=%s" % (upload_base, urllib.parse.quote(name))
    mb = len(data) / 1024.0 / 1024.0
    st, res = None, None
    for attempt in range(1, attempts + 1):
        try:
            st, res = gh.call("POST", url, data, ctype)
        except (urllib.error.URLError, ssl.SSLError, OSError) as e:
            st, res = None, "%s: %s" % (type(e).__name__, e)   # 错误分类：可读类别
        if st in (200, 201):
            log("  [ok] %s  %.1f MB" % (name, mb))
            return True
        # 永久错误（4xx，如 422 已存在 / 403 无权限）不重试，快速暴露
        if isinstance(st, int) and 400 <= st < 500:
            break
        if attempt < attempts:
            delay = min(2 ** (attempt - 1) * 2, 30) + random.uniform(0, 1)  # 退避 + 抖动
            log("  [retry] %s  %.1f MB attempt %d/%d failed (%s), retry in %.1fs ..."
                % (name, mb, attempt, attempts, st, delay))
            time.sleep(delay)
    log("  [FAIL] %s HTTP %s: %s" % (name, st, res))
    return False
```

要点：
- **重试次数 3 → 4**；退避 `min(2^, 30)` 秒 + `uniform(0,1)` 抖动；
- **只重试瞬时错误**：网络异常（含 `ConnectionResetError`/10054）、超时、HTTP 5xx；
- **4xx 不重试**（避免在「资产已存在 / 无权限」上空转）；
- `data` 仍为整包 bytes（**不做流式**）；
- 错误文本带异常类名，便于区分「网络抖动」与「权限/配额」。

### 2.4 `release.bat` 接线

1. **L329** 从 `!ASSETS!` 移除 `*.pdb.zip` / `*.pdb`（gh 路径不再耦合 PDB）；保留注释说明 PDB 改为发布后补传。
2. 新增子过程 `:upload_pdb`，在 **gh 路径发布成功后**（L354-358 之间，`verify_latest` 之后）与 **python 路径发布成功后**（L369-372 之间）各 `call` 一次：

```bat
rem PDB 是发布后的 best-effort 补传：失败只告警，绝不回退已完成的发布。
:upload_pdb
if defined PYEXE (
  %PYEXE% "%ROOT%scripts\publish_release.py" --tag "v%VER%" --version "%VER%" ^
      --package "%BIN%\package" --repo "%REPO%" --pdb-only
  if errorlevel 1 echo   [WARN] PDB upload failed; rerun with --pdb-only to backfill.
) else (
  for %%f in ("%BIN%\package\*.pdb.zip" "%BIN%\package\*.pdb") do (
    if exist "%%f" (
      gh release upload "v%VER%" "%%f" --repo "%REPO%" --clobber
      if errorlevel 1 echo   [WARN] PDB upload failed: %%f
    )
  )
)
exit /b 0
```

   - 子过程**恒返回 0**，调用方不得 `goto :release_fail`；
   - 浏览器路径（`:publish_browser`）**不变**，仍提示手工拖入 PDB（L378-379）。

### 2.5 测试（`scripts/test_publish_release.py`）

沿用现有 unittest + mock `gh.call` 风格。新增/调整：

| 用例 | 断言 |
|---|---|
| 默认发布不含 PDB | 上传名列表无 `.pdb*`；`main()==0`；`PATCH` 照常发生 |
| 必需资产 fail-closed | `.sig` 缺失 / exe 上传失败 → `main()==1` 且**无 `PATCH`**（G3 不回退） |
| `--pdb-only` 上传 | 目标 release 已公开（tags 返回 200 且 `draft:false`）时仍上传 PDB；返回 0 |
| `--pdb-only` 幂等 | PDB 已在 `existing` → `[skip]`，不上传、不 `PATCH` |
| `--pdb-only` 无 release | tags 404 且列表无草稿 → 退出码 1 |
| 重试策略 | 瞬时错误（`URLError` / 500）第 k 次成功 → 成功且重试 k-1 次；4xx → **只调一次** |
| batch 接线守卫 | 文本断言 `release.bat`：`ASSETS` 段不含 `.pdb`；存在 `--pdb-only` / `gh release upload` |

**既有用例调整**：`test_debug_symbols_uploaded_but_not_checksummed`（L189-227）语义反转——默认发布**不再**上传 PDB，改为断言默认不含 PDB，并把「PDB 会上传」搬到 `--pdb-only` 用例。

### 2.6 手工端到端（离线 CI 覆盖不到）

1. 真跑一次 `release.bat` 到 Publish 阶段：观察**主发布先成功**（`PATCH` + tags 验证），PDB 补传**独立打印**；
2. 单跑 `publish_release.py --pdb-only`：已上传的 PDB 打印 `[skip]`，验证幂等；
3. 断网/关代理跑 `--pdb-only`：观察重试退避日志与最终 `[WARN]`，确认**不回退已发布状态**。

## 3. 非目标（明确不做）

- 流式上传（§3.1 已否决，理由见决策表）；
- 方案 C：调 Cargo release profile 缩小 PDB；
- Authenticode 代码签名 / 便携版签名信任链 / 私钥加密；
- CI/CD 自动化；
- `build.bat` 的任何改动；
- 浏览器发布路径的改动。

## 4. 风险与回退

| 风险 | 对策 |
|---|---|
| 向**已公开** release 追加资产的行为需实测确认 | 手工端到端 2.6 先验证；若不可行，退化为「发布前先传 PDB、但仍不参与 fail 判定」 |
| 重试 4 次拉长失败耗时 | 退避上限 `cap=30s`，最坏有界（约 ≤ 60s + 请求耗时） |
| gh 路径在无 python 时补传能力弱化 | 退化为 `gh release upload --clobber`，仍只告警 |
| `--pdb-only` 与默认模式对「已公开 release」策略不一致被误用 | 该差异写入参数 help 与代码注释；默认模式仍拒绝改已公开 release |

## 5. 未决问题

无——方案 A/B、PDB 移出主流程、放弃流式、发布后自动补传均经用户逐项确认；「已公开 release 可追加资产」作为实测项列入验收。
