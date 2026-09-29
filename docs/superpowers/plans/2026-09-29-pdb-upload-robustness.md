# PDB 上传健壮性 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 PDB（调试符号）上传失败不再阻断 GitHub Release 发布，并让 PDB 上传对瞬时网络错误更耐受。

**Architecture:** 把 PDB 从主发布流程中整体移出，新增 `--pdb-only` 补传入口；发布后由 `release.bat` best-effort 触发补传。同时把上传重试抽成 `upload_with_retry` 并加固（4 次、指数退避 + 抖动、区分瞬时/永久错误）。必需资产仍走原 fail-closed 路径。

**Tech Stack:** Python 3（仅标准库：`urllib`/`ssl`/`urllib.error`/`random`/`time`）、Windows 批处理（cmd）、`unittest` + `unittest.mock`。

## Global Constraints

- 仅使用 Python 标准库，**不新增任何依赖**。
- **不实现流式上传**：`gh.call` 的 `data` 保持为整包 `bytes`。
- 必需资产（`.exe`/`.msi` + `.sig` + `latest.json` + `SHA256SUMS.txt`）缺失或上传失败时**必须 fail-closed**（`main()` 返回 1 且不发 `PATCH`）。
- **不修改 `build.bat`**；**不修改**浏览器发布路径（`:publish_browser`）。
- 代码注释用简体中文，与仓库既有风格一致（解释「为什么」）。
- 测试**离线**运行：绝不访问网络、绝不读取真实凭据（沿用现有 `mock.patch.object(release, "github_token", ...)` 风格）。
- 测试运行方式：在仓库根目录执行 `python scripts/test_publish_release.py -v`。
- 版本/标签参数沿用 `--tag vX.Y.Z` 与 `--version X.Y.Z`。

---

### Task 1: 抽出并加固 `upload_with_retry`

**Files:**
- Modify: `scripts/publish_release.py`（顶部 `import` 区；新增 helper；替换 L471-499 的上传循环）
- Test: `scripts/test_publish_release.py`（新增 `UploadRetryTests`；给会触发重试的既有用例打 `sleep` 补丁）

**Interfaces:**
- Consumes: 现有 `GitHub.call(self, method, url, data=None, content_type=None, timeout=300)`、模块级 `log(msg)`。
- Produces: `upload_with_retry(gh, upload_base, path, attempts=4) -> bool`（成功 True；失败 False；仅在瞬时错误上重试）。

- [ ] **Step 1: 写失败测试**

在 `scripts/test_publish_release.py` 的 `ManifestTests` 之后新增一个测试类：

```python
class UploadRetryTests(unittest.TestCase):
    def _gh(self, responder):
        gh = mock.Mock()
        calls = []

        def call(method, url, data=None, content_type=None):
            calls.append(url)
            return responder(len(calls))

        gh.call.side_effect = call
        return gh, calls

    def test_retries_transient_then_succeeds(self):
        # 瞬时错误（500）应重试，第 3 次成功则整体成功
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d) / "niuma-timer-1.4.1-portable.pdb.zip"
            p.write_bytes(b"x" * 32)
            gh, calls = self._gh(lambda n: (500, {}) if n < 3 else (201, {}))
            with mock.patch.object(release.time, "sleep"):
                ok = release.upload_with_retry(gh, "https://upload.test/assets", str(p))
            self.assertTrue(ok)
            self.assertEqual(len(calls), 3)

    def test_client_error_not_retried(self):
        # 永久错误（422 已存在 / 403 无权限）不得重试，一次即判失败
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d) / "niuma-timer-1.4.1-portable.pdb.zip"
            p.write_bytes(b"x" * 32)
            gh, calls = self._gh(lambda n: (422, {}))
            with mock.patch.object(release.time, "sleep"):
                ok = release.upload_with_retry(gh, "https://upload.test/assets", str(p))
            self.assertFalse(ok)
            self.assertEqual(len(calls), 1)

    def test_network_error_is_retried(self):
        # 连接被重置（Errno 10054）属瞬时错误，必须被接住并重试
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d) / "niuma-timer-1.4.1-portable.pdb.zip"
            p.write_bytes(b"x" * 32)

            def responder(n):
                if n < 4:
                    raise ConnectionResetError(10054, "Connection reset by peer")
                return 201, {}

            gh, calls = self._gh(responder)
            with mock.patch.object(release.time, "sleep"):
                ok = release.upload_with_retry(gh, "https://upload.test/assets", str(p))
            self.assertTrue(ok)
            self.assertEqual(len(calls), 4)
```

- [ ] **Step 2: 运行测试确认失败**

Run: `python scripts/test_publish_release.py -v`
Expected: FAIL —— `AttributeError: module 'publish_release' has no attribute 'upload_with_retry'`

- [ ] **Step 3: 实现 `upload_with_retry`**

在 `scripts/publish_release.py` 顶部 import 区加入 `random`（保持字母序，放在 `os` 与 `re` 之间）：

```python
import random
```

在 `def log(msg):` 之后新增 helper：

```python
def upload_with_retry(gh, upload_base, path, attempts=4):
    """上传单个资产；成功返回 True，仅在瞬时错误上重试。

    大文件（PDB 可达数十 MB）经本地代理上传时偶发连接被远端重置（Errno 10054），
    属瞬时网络错误，重试几次再判失败；永久错误（4xx，如 422 已存在 / 403 无权限）
    立即放弃，避免在无意义的重试上空转。网络异常必须在这里接住，否则未捕获的
    URLError 会让整个脚本崩掉，连 fail-closed 都走不到。
    """
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
            st, res = None, "%s: %s" % (type(e).__name__, e)
        if st in (200, 201):
            log("  [ok] %s  %.1f MB" % (name, mb))
            return True
        if isinstance(st, int) and 400 <= st < 500:
            break
        if attempt < attempts:
            delay = min(2 ** (attempt - 1) * 2, 30) + random.uniform(0, 1)
            log("  [retry] %s  %.1f MB attempt %d/%d failed (%s), retry in %.1fs ..."
                % (name, mb, attempt, attempts, st, delay))
            time.sleep(delay)
    log("  [FAIL] %s HTTP %s: %s" % (name, st, res))
    return False
```

- [ ] **Step 4: 用 helper 替换 main() 中的内联上传循环**

把 `scripts/publish_release.py` 中这段（原 L471-499）：

```python
    failed = False
    for path in assets:
        name = os.path.basename(path)
        if name in existing:
            log("  [skip] %s (already uploaded)" % name)
            continue
        with open(path, "rb") as f:
            data = f.read()
        ctype = mimetypes.guess_type(name)[0] or "application/octet-stream"
        url = "%s?name=%s" % (upload_base, urllib.parse.quote(name))
        st, res = None, None
        # 大文件（PDB 上百 MB）经本地代理上传时偶发连接被远端重置，
        # 属瞬时网络错误，多试几次再判失败；网络异常必须在这里接住，
        # 否则未捕获的 URLError 会让整个脚本崩掉，连 fail-closed 都走不到。
        for attempt in (1, 2, 3):
            try:
                st, res = gh.call("POST", url, data, ctype)
            except (urllib.error.URLError, ssl.SSLError, OSError) as e:
                st, res = None, e
            if st in (200, 201):
                break
            if attempt < 3:
                log("  [retry] %s failed (%s), retrying %d/3 ..." % (name, st, attempt))
                time.sleep(2 * attempt)
        if st in (200, 201):
            log("  [ok] %s  %.1f MB" % (name, len(data) / 1024.0 / 1024.0))
        else:
            log("  [FAIL] %s HTTP %s: %s" % (name, st, res))
            failed = True
```

替换为：

```python
    failed = False
    for path in assets:
        name = os.path.basename(path)
        if name in existing:
            log("  [skip] %s (already uploaded)" % name)
            continue
        if not upload_with_retry(gh, upload_base, path):
            failed = True
```

- [ ] **Step 5: 给触发重试的既有用例打 `sleep` 补丁**

`test_api_publishes_only_complete_draft` 的 `fail_upload=True` 分支现在会重试 4 次并 `sleep`，会导致测试变慢。修改该用例的 `with` 块（原 L61-62），加入对 `sleep` 的补丁：

```python
                with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"), \
                     mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh), \
                     mock.patch.object(release.time, "sleep"):
                    self.assertEqual(release.main(), 1 if fail_upload else 0)
```

- [ ] **Step 6: 运行全部测试确认通过**

Run: `python scripts/test_publish_release.py -v`
Expected: PASS（新增 3 个 `UploadRetryTests` + 既有全部用例，且整体运行时间无显著增加）

- [ ] **Step 7: 提交**

```bash
git add scripts/publish_release.py scripts/test_publish_release.py
git commit -m "refactor(release): 抽出 upload_with_retry，重试 4 次加指数退避与抖动"
```

---

### Task 2: 抽出 `collect_package_assets` 并把 PDB 移出默认发布流程

**Files:**
- Modify: `scripts/publish_release.py`（新增 `collect_package_assets`；改写 main() 中的资产收集与 `assets` 组装）
- Test: `scripts/test_publish_release.py`（改写 `test_debug_symbols_uploaded_but_not_checksummed`）

**Interfaces:**
- Consumes: Task 1 的 `upload_with_retry`；模块级 `log`。
- Produces: `collect_package_assets(package_dir, version) -> (bins, sigs, pdbs)`（三个 `list[str]` 绝对路径；`bins` 为 `.exe`/`.msi`，`sigs` 为 `.sig`，`pdbs` 为 `.pdb`/`.pdb.zip`；均按版本号过滤）。

- [ ] **Step 1: 改写测试为「默认发布不含 PDB」**

把 `scripts/test_publish_release.py` 中的 `test_debug_symbols_uploaded_but_not_checksummed`（原 L189-227）整体替换为：

```python
    def test_debug_symbols_not_uploaded_in_default_flow(self):
        # PDB 不是运行时依赖：默认发布只发必需资产，调试符号由 --pdb-only
        # 在发布后 best-effort 补传（见 PdbOnlyTests）。PDB 失败绝不能阻断发布。
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")
            (root / "niuma-timer-1.4.0-portable.pdb").write_bytes(b"symbols")
            uploads, published = [], [False]

            def call(method, url, data=None, content_type=None):
                if method == "POST" and url.endswith("/releases"):
                    return 201, {"id": 7, "upload_url": "https://upload.test/assets", "assets": []}
                if method == "POST":
                    uploads.append(urllib.parse.parse_qs(
                        urllib.parse.urlparse(url).query)["name"][0])
                    return 201, {}
                if method == "PATCH":
                    published[0] = True
                    return 200, {}
                if method == "GET":
                    if "/releases/tags/" in url:
                        return (200, {"draft": False}) if published[0] else (404, {})
                    if url.endswith("/releases?per_page=100"):
                        return 200, []
                    if url.endswith("/releases/7"):
                        return 200, {"assets": [{"name": n, "size": 1} for n in uploads],
                                     "html_url": "https://example.test/release"}
                return 404, {}

            gh = mock.Mock()
            gh.call.side_effect = call
            argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0", "--package", d, "--root", d]
            with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"), \
                 mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh), \
                 mock.patch.object(release.time, "sleep"):
                self.assertEqual(release.main(), 0)
            self.assertNotIn("niuma-timer-1.4.0-portable.pdb", uploads)
            sums = (root / "SHA256SUMS.txt").read_text(encoding="utf-8")
            self.assertIn("niuma-timer-1.4.0-portable.exe", sums)
            self.assertNotIn(".pdb", sums)
```

- [ ] **Step 2: 运行测试确认失败**

Run: `python scripts/test_publish_release.py -v`
Expected: FAIL —— 断言 `NotIn` 失败（当前默认流程**会**上传 PDB）

- [ ] **Step 3: 新增 `collect_package_assets`**

在 `scripts/publish_release.py` 的 `upload_with_retry` 之后新增：

```python
def collect_package_assets(package_dir, version):
    """按版本筛选 package 目录下的资产，返回 (bins, sigs, pdbs)。

    cargo tauri build 不会清理旧版本的 bundle，所以一律按版本号过滤，
    避免把上一版的安装包/签名/符号混进本次 Release。PDB 单独成列，
    因为它不是运行时依赖，不参与主发布的必需资产集合。
    """
    bins, sigs, pdbs = [], [], []
    if os.path.isdir(package_dir):
        for name in sorted(os.listdir(package_dir)):
            p = os.path.join(package_dir, name)
            if not os.path.isfile(p):
                continue
            low = name.lower()
            if low.endswith((".exe", ".msi")):
                if version not in name:
                    log("  [skip] %s: not version %s" % (name, version))
                    continue
                bins.append(p)
            elif low.endswith(".sig"):
                if version in name:
                    sigs.append(p)
            elif low.endswith((".pdb", ".pdb.zip")):
                if version in name:
                    pdbs.append(p)
    return bins, sigs, pdbs
```

- [ ] **Step 4: 改写 main() 的资产收集与组装**

把 `scripts/publish_release.py` 中这段（原 L428-459）：

```python
    existing = {a["name"] for a in rel.get("assets", [])}
    bins = []
    sigs = []
    pdbs = []
    if os.path.isdir(package_dir):
        for name in sorted(os.listdir(package_dir)):
            p = os.path.join(package_dir, name)
            if not os.path.isfile(p):
                continue
            low = name.lower()
            if low.endswith((".exe", ".msi")):
                if args.version not in name:
                    # stale bundle from an older release: never upload or checksum it
                    log("  [skip] %s: not version %s" % (name, args.version))
                    continue
                bins.append(p)
            elif low.endswith(".sig"):
                # 签名与安装包必须同批上传：latest.json 引用的就是这些文件，
                # 少一个就等价于给客户端一个 404 的更新源
                if args.version in name:
                    sigs.append(p)
            elif low.endswith((".pdb", ".pdb.zip")):
                # 调试符号与安装包同批上传：崩溃 dump 只有配上这一版 exe 的
                # PDB（GUID+Age 匹配）才能精确符号化，漏发即该版本不可查。
                # 打包产出的是压缩包（PDB 原样上百 MB，经代理上传会被重置），
                # 原始 .pdb 也接受，便于手工补传。
                if args.version in name:
                    pdbs.append(p)

    assets = bins + sigs + pdbs
    if manifest:
        assets.append(os.path.join(package_dir, "latest.json"))
```

替换为：

```python
    existing = {a["name"] for a in rel.get("assets", [])}
    bins, sigs, pdbs = collect_package_assets(package_dir, args.version)

    # 主发布只发必需资产：安装包 + 签名（latest.json 引用的就是它们，
    # 少一个就等价于给客户端一个 404 的更新源）。PDB（符号）不是运行时依赖，
    # 一律走 --pdb-only 在发布后补传，绝不允许它阻断发布。
    assets = bins + sigs
    if manifest:
        assets.append(os.path.join(package_dir, "latest.json"))
```

- [ ] **Step 5: 运行全部测试确认通过**

Run: `python scripts/test_publish_release.py -v`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add scripts/publish_release.py scripts/test_publish_release.py
git commit -m "refactor(release): PDB 移出主发布流程，只发必需运行时资产"
```

---

### Task 3: 新增 `--pdb-only` 补传模式

**Files:**
- Modify: `scripts/publish_release.py`（argparse 新增开关；新增 `publish_pdb_only`；在 main() 入口分发）
- Test: `scripts/test_publish_release.py`（新增 `PdbOnlyTests`）

**Interfaces:**
- Consumes: Task 1 的 `upload_with_retry`、Task 2 的 `collect_package_assets`；`github_token()`、`build_opener()`、`GitHub`、`API`、`DEFAULT_REPO`。
- Produces: `publish_pdb_only(args, root, package_dir) -> int`（0 成功或无需补传；1 失败），以及 CLI 开关 `--pdb-only`。

- [ ] **Step 1: 写失败测试**

在 `scripts/test_publish_release.py` 新增测试类：

```python
class PdbOnlyTests(unittest.TestCase):
    def _run(self, d, call):
        gh = mock.Mock()
        gh.call.side_effect = call
        argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0",
                "--package", d, "--root", d, "--pdb-only"]
        with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"), \
             mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh), \
             mock.patch.object(release.time, "sleep"):
            return release.main()

    def test_uploads_pdb_to_published_release(self):
        # 目标 Release 已公开（draft=false）时仍能补传 PDB，且不改动发布状态
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.pdb.zip").write_bytes(b"symbols")
            uploads, patched = [], []

            def call(method, url, data=None, content_type=None):
                if method == "GET" and "/releases/tags/" in url:
                    return 200, {"draft": False, "id": 7,
                                 "upload_url": "https://upload.test/assets", "assets": []}
                if method == "POST":
                    uploads.append(urllib.parse.parse_qs(
                        urllib.parse.urlparse(url).query)["name"][0])
                    return 201, {}
                if method == "PATCH":
                    patched.append(url)
                    return 200, {}
                return 404, {}

            self.assertEqual(self._run(d, call), 0)
            self.assertEqual(uploads, ["niuma-timer-1.4.0-portable.pdb.zip"])
            self.assertEqual(patched, [], "--pdb-only 绝不能 PATCH 发布状态")

    def test_skips_already_uploaded(self):
        # 已上传的 PDB 幂等跳过，不重复 POST
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.pdb.zip").write_bytes(b"symbols")
            posts = []

            def call(method, url, data=None, content_type=None):
                if method == "GET" and "/releases/tags/" in url:
                    return 200, {"draft": False, "id": 7, "upload_url": "https://upload.test/assets",
                                 "assets": [{"name": "niuma-timer-1.4.0-portable.pdb.zip", "size": 7}]}
                if method == "POST":
                    posts.append(url)
                    return 201, {}
                return 404, {}

            self.assertEqual(self._run(d, call), 0)
            self.assertEqual(posts, [])

    def test_fails_when_no_release_found(self):
        # tags 查不到、草稿列表也没有 → 没有可挂载 PDB 的 Release，返回 1
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.pdb.zip").write_bytes(b"symbols")

            def call(method, url, data=None, content_type=None):
                if method == "GET" and url.endswith("/releases?per_page=100"):
                    return 200, []
                return 404, {}

            self.assertEqual(self._run(d, call), 1)

    def test_no_pdb_present_is_not_a_failure(self):
        # package 里没有 PDB（例如手工补传已清理）→ 无操作即成功
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")

            def call(method, url, data=None, content_type=None):
                if method == "GET" and "/releases/tags/" in url:
                    return 200, {"draft": False, "id": 7,
                                 "upload_url": "https://upload.test/assets", "assets": []}
                return 404, {}

            self.assertEqual(self._run(d, call), 0)
```

- [ ] **Step 2: 运行测试确认失败**

Run: `python scripts/test_publish_release.py -v`
Expected: FAIL —— `--pdb-only` 未被识别（`SystemExit: 2`，argparse unknown argument）

- [ ] **Step 3: 新增 argparse 开关与分发**

在 `scripts/publish_release.py` 的 `main()` 中，`ap.add_argument("--generate-notes-only", action="store_true")` 之后新增：

```python
    ap.add_argument("--pdb-only", action="store_true",
                    help="只补传调试符号（PDB），允许操作已公开的 release；"
                         "不建草稿、不 PATCH、不校验；失败返回 1（调用方只应告警）")
```

在 `args = ap.parse_args()` 之后、`root = ...` 之后（即在 `notes_path = ...` 之前）插入分发：

```python
    if args.pdb_only:
        return publish_pdb_only(args, root, package_dir)
```

- [ ] **Step 4: 实现 `publish_pdb_only`**

在 `scripts/publish_release.py` 的 `main()` 定义**之前**新增：

```python
def publish_pdb_only(args, root, package_dir):
    """仅补传调试符号。

    与默认发布的根本差异：这里允许操作**已公开**的 release（默认模式会拒绝
    改动已发布内容）。PDB 不是运行时依赖，缺失只影响该版本崩溃的可符号化能力，
    所以本模式失败返回 1 仅供调用方告警，调用方不得据此判定发布失败。
    """
    token = github_token()
    if not token:
        log("  [ERROR] no GitHub token available from the git credential store")
        return 1

    gh = GitHub(token, build_opener())
    api = "%s/repos/%s" % (API, args.repo)

    st, rel = gh.call("GET", "%s/releases/tags/%s" % (api, args.tag))
    if st != 200:
        # tags 端点对草稿与「尚未公开」的 tag 可能 404，退回列表按 tag_name 找
        rel = None
        lst, rels = gh.call("GET", "%s/releases?per_page=100" % api)
        if lst == 200:
            for r in rels:
                if r.get("tag_name") == args.tag:
                    rel = r
                    break
    if not rel:
        log("  [ERROR] no release found for tag %s" % args.tag)
        return 1

    upload_base = (rel.get("upload_url") or "").split("{")[0]
    if not upload_base:
        log("  [ERROR] no upload_url in the release payload")
        return 1

    existing = {a["name"] for a in rel.get("assets", [])}
    _, _, pdbs = collect_package_assets(package_dir, args.version)
    if not pdbs:
        log("  [WARN] no PDB assets matching %s in %s" % (args.version, package_dir))
        return 0

    failed = False
    for path in pdbs:
        name = os.path.basename(path)
        if name in existing:
            log("  [skip] %s (already uploaded)" % name)
            continue
        if not upload_with_retry(gh, upload_base, path):
            failed = True
    return 1 if failed else 0
```

- [ ] **Step 5: 运行全部测试确认通过**

Run: `python scripts/test_publish_release.py -v`
Expected: PASS（新增 4 个 `PdbOnlyTests` + 既有全部用例）

- [ ] **Step 6: 提交**

```bash
git add scripts/publish_release.py scripts/test_publish_release.py
git commit -m "feat(release): 新增 --pdb-only 补传入口，允许操作已公开 release"
```

---

### Task 4: `release.bat` 接线（发布后 best-effort 补传 PDB）

**Files:**
- Modify: `release.bat`（L329 处 `!ASSETS!` 移除 PDB；gh 与 python 两条路径发布成功后 `call :upload_pdb`；新增子过程）
- Test: `scripts/test_publish_release.py`（新增接线守卫文本断言）

**Interfaces:**
- Consumes: Task 3 的 `--pdb-only` CLI；`PYEXE`、`%VER%`、`%REPO%`、`%BIN%`、`%ROOT%`（release.bat 内既有变量）。
- Produces: 批处理子过程 `:upload_pdb`（恒 `exit /b 0`）。

- [ ] **Step 1: 写失败测试（接线守卫）**

在 `scripts/test_publish_release.py` 新增：

```python
    def test_release_bat_decouples_pdb_from_publish(self):
        # PDB 必须在发布后 best-effort 补传：ASSETS 段不得含 .pdb，
        # 且脚本必须提供 --pdb-only 与 gh release upload 两条补传路径。
        batch = (pathlib.Path(__file__).parent.parent / "release.bat").read_text(encoding="utf-8")
        assets_block = batch[batch.index('set "ASSETS="'):batch.index("where gh >nul")]
        self.assertNotIn(".pdb", assets_block)
        self.assertIn("--pdb-only", batch)
        self.assertIn("gh release upload", batch)
        self.assertIn("call :upload_pdb", batch)
```

（放在 `ManifestTests` 内，与 `test_release_gate_precedes_git_and_publication` 相邻。）

- [ ] **Step 2: 运行测试确认失败**

Run: `python scripts/test_publish_release.py -v`
Expected: FAIL —— `assets_block` 仍含 `.pdb`

- [ ] **Step 3: 从 `!ASSETS!` 移除 PDB**

把 `release.bat` 中原 L326-329：

```bat
rem PDB（调试符号）同样是 Release 资产：客户端崩溃 dump 必须用「与那个 exe
rem 同批构建」的 PDB 才能精确符号化，漏发这一版就永久查不了。
rem 打包产出的是 .pdb.zip（压缩后才经得起代理上传），原始 .pdb 一并兼容。
for %%f in ("%BIN%\package\*.pdb.zip" "%BIN%\package\*.pdb") do set "ASSETS=!ASSETS! "%%f""
```

替换为：

```bat
rem PDB（调试符号）不在发布关键路径上：它是非运行时依赖，上传失败/被网络重置
rem 都不该拖垮整次发布。改由发布成功后 best-effort 补传（见 :upload_pdb）。
```

- [ ] **Step 4: 在 gh 路径发布成功后调用补传**

在 `release.bat` 的 gh 路径中，`call :verify_latest` 与 `if errorlevel 1 goto :release_fail` 之后、`popd` 之前（原 L357-359）插入：

```bat
call :upload_pdb
```

即该段变为：

```bat
echo     release v%VER% published
call :verify_latest
if errorlevel 1 goto :release_fail
call :upload_pdb
popd
goto :summary
```

- [ ] **Step 5: 在 python 路径发布成功后调用补传**

在 `release.bat` 的 python 路径中，同样在 `popd` 之前（原 L369-372）插入 `call :upload_pdb`：

```bat
echo     release v%VER% published
call :verify_latest
if errorlevel 1 goto :release_fail
call :upload_pdb
popd
goto :summary
```

- [ ] **Step 6: 新增 `:upload_pdb` 子过程**

在 `release.bat` 中 `:verify_latest` 子过程之后、`:release_fail` 之前插入：

```bat
:upload_pdb
rem PDB 是发布后的 best-effort 补传：失败只告警，绝不回退已完成的发布。
rem 优先用 python 脚本（带重试/退避，允许操作已公开 release）；无 python 时
rem 退化为 gh release upload。两条路径都不影响本子过程的返回值。
if defined PYEXE (
  %PYEXE% "%ROOT%scripts\publish_release.py" --tag "v%VER%" --version "%VER%" --package "%BIN%\package" --repo "%REPO%" --pdb-only
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

- [ ] **Step 7: 运行全部测试确认通过**

Run: `python scripts/test_publish_release.py -v`
Expected: PASS

- [ ] **Step 8: 语法自检（不发布）**

Run: `cmd /c "findstr /n /c:\":upload_pdb\" release.bat"`
Expected: 输出 3 行（子过程定义 1 处 + `call :upload_pdb` 2 处），确认接线到位。

- [ ] **Step 9: 提交**

```bash
git add release.bat scripts/test_publish_release.py
git commit -m "build(release): PDB 改为发布后 best-effort 补传，不再阻断发布"
```

---

### Task 5: 手工端到端验证（离线 CI 覆盖不到）

**Files:** 无代码改动；仅按步骤观察真实发布行为。

**Interfaces:**
- Consumes: Task 1-4 的全部产物。
- Produces: 验收证据（日志片段），用于确认 G1/G3 与「已公开 release 可追加资产」这一实测项。

- [ ] **Step 1: 准备一次真实发布**

Run: `release.bat`（走完构建 → tag → push → 发布）
Expected:
- 主发布先完成：日志出现 `RELEASE_URL=...` 与 `PATCH` 成功后打印的资产清单，**不含** `.pdb.zip`；
- 随后独立出现 `:upload_pdb` 的输出：要么 PDB 上传 `[ok]`，要么 `[WARN] PDB upload failed; rerun ...`；
- `Done.` 摘要正常输出，脚本退出码 0。

- [ ] **Step 2: 验证 G1（PDB 失败不阻断发布）**

在 `bin\package` 中把 `*.pdb.zip` 临时改名为不可识别的后缀，重跑发布（或对已有草稿重跑）：

Run: `python scripts\publish_release.py --tag v<VER> --version <VER> --package bin\package --pdb-only`
Expected: 打印 `[WARN] no PDB assets matching ...`，退出码 0；已发布的 Release 不受影响。

（恢复文件名后继续。）

- [ ] **Step 3: 验证「已公开 release 可追加资产」+ 幂等**

首次运行：

Run: `python scripts\publish_release.py --tag v<VER> --version <VER> --package bin\package --pdb-only`
Expected: 打印 `[ok] niuma-timer-<VER>-portable.pdb.zip  XX.X MB`，退出码 0；GitHub Release 页出现该资产。

再次运行同一命令：

Run: `python scripts\publish_release.py --tag v<VER> --version <VER> --package bin\package --pdb-only`
Expected: 打印 `[skip] niuma-timer-<VER>-portable.pdb.zip (already uploaded)`，退出码 0。

> 若首次运行报错表明 GitHub 不允许向已公开 release 追加资产，则记录证据并回退到 spec §4 的对策（「发布前先传 PDB、但仍不参与 fail 判定」），并在本计划中追加一个补救任务。

- [ ] **Step 4: 验证 G3（必需资产仍 fail-closed）**

在一个临时副本目录中，仅放入安装包但缺少 `.sig`，运行默认发布：

Run: `python scripts\publish_release.py --tag v<VER> --version <VER> --package <临时目录> --repo <owner/repo>`
Expected: 退出码 1，且**不**发 `PATCH`（日志无 `RELEASE_URL=`），确认必需资产缺失仍 fail-closed。

- [ ] **Step 5: 记录验收结论**

在本次发布对应的 `docs/v1.4.x-acceptance.md`（若存在）或提交信息中记录：
- 主发布成功与 PDB 补传独立完成（G1）；
- 「已公开 release 可追加资产」实测结论（可行 / 需回退）；
- 必需资产缺失时退出码 1 且不公开（G3）。

---

## Self-Review

**1. Spec coverage**

| Spec 章节 | 覆盖任务 |
|---|---|
| §2.1 默认只发必需资产 | Task 2 |
| §2.2 `--pdb-only` | Task 3 |
| §2.3 上传加固（重试/退避/抖动/错误分类） | Task 1 |
| §2.4 `release.bat` 接线 | Task 4 |
| §2.5 测试（默认不含 PDB / fail-closed / pdb-only ×3 / 重试 ×2 / batch 守卫） | Task 1-4 |
| §2.6 手工端到端 | Task 5 |
| 决策表「保留 L217-223 门禁」「build.bat 不改」 | 无改动（保持） |
| 决策表「浏览器路径不改」 | 无改动（保持） |

无遗漏。

**2. Placeholder scan**

无 TBD/TODO；所有代码步骤均给出完整可粘贴代码；命令均为可执行的具体命令。Task 5 Step 4 的 `<临时目录>`/`<owner/repo>` 属操作占位，非代码占位。

**3. Type consistency**

- `upload_with_retry(gh, upload_base, path, attempts=4) -> bool`：Task 1 定义，Task 2/3 调用一致。
- `collect_package_assets(package_dir, version) -> (bins, sigs, pdbs)`：Task 2 定义，Task 3 用 `_, _, pdbs = ...` 解包一致。
- `publish_pdb_only(args, root, package_dir) -> int`：Task 3 定义与 `main()` 内 `return publish_pdb_only(args, root, package_dir)` 一致。
- `gh.call(method, url, data=None, content_type=None)`：所有 fake 的签名与该 4 个位置参数一致。
