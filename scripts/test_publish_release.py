"""Offline updater manifest regression tests; never publish or access credentials."""
import contextlib
import os
import pathlib
import shutil
import tempfile
import unittest
import uuid
from unittest import mock
import json
import urllib.parse
import publish_release as release


@contextlib.contextmanager
def tmpdir():
    """Yield a scratch directory a restricted Windows token can actually write to.

    tmpdir() creates its directory with mode 0700. Under a
    lockdown token (the DSH file sandbox on this machine) that mode yields a DACL
    the process itself cannot write through: every file created inside fails with
    EACCES and even cleanup is denied. os.makedirs inherits the parent's
    permissive ACL instead, which the same token can write. Behaviour on a normal
    machine is identical - this only removes a sandbox artefact.
    """
    d = os.path.join(tempfile.gettempdir(), "niuma-uitest-" + uuid.uuid4().hex)
    os.makedirs(d, exist_ok=True)
    try:
        yield d
    finally:
        shutil.rmtree(d, ignore_errors=True)


class ManifestTests(unittest.TestCase):
    def test_notes_generation_replaces_stale_version(self):
        with tmpdir() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")
            (root / "RELEASE_NOTES.md").write_text("# Niuma 1.3.0\nOld notes", encoding="utf-8")
            argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0", "--package", d,
                    "--root", d, "--generate-notes-only"]
            with mock.patch("sys.argv", argv):
                self.assertEqual(release.main(), 0)
            self.assertIn("1.4.0", (root / "RELEASE_NOTES.md").read_text(encoding="utf-8").splitlines()[0])

    def test_api_publishes_only_complete_draft(self):
        for fail_upload in [False, True]:
            with self.subTest(fail_upload=fail_upload), tmpdir() as d:
                root = pathlib.Path(d)
                (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")
                calls, assets = [], []
                published = [False]  # PATCH draft:false 后，tags 端点才可见该 release
                def call(method, url, data=None, content_type=None):
                    calls.append((method, url, data))
                    if method == "POST" and url.endswith("/releases"):
                        self.assertTrue(json.loads(data)["draft"])
                        return 201, {"id": 7, "upload_url": "https://upload.test/assets", "assets": []}
                    if method == "POST":
                        if fail_upload:
                            return 500, {}
                        name = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)["name"][0]
                        assets.append({"name": name, "size": len(data)})
                        return 201, {}
                    if method == "PATCH":
                        self.assertEqual({a["name"] for a in assets},
                                         {"niuma-timer-1.4.0-portable.exe", "latest.json", "SHA256SUMS.txt"})
                        self.assertFalse(json.loads(data)["draft"])
                        published[0] = True
                        return 200, {}
                    if method == "GET":
                        if "/releases/tags/" in url:
                            # 草稿没有 tag 引用 → 404；PATCH 公开后 → 200 且非 draft
                            if published[0]:
                                return 200, {"draft": False}
                            return 404, {}
                        if url.endswith("/releases?per_page=100"):
                            return 200, []  # 无可复用草稿 → 走新建
                        if url.endswith("/releases/7"):
                            return 200, {"assets": assets, "html_url": "https://example.test/release"}
                    return 404, {}
                gh = mock.Mock()
                gh.call.side_effect = call
                argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0", "--package", d, "--root", d]
                with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"), \
                     mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh), \
                     mock.patch.object(release.time, "sleep"):
                    self.assertEqual(release.main(), 1 if fail_upload else 0)
                self.assertEqual(any(c[0] == "PATCH" for c in calls), not fail_upload)
                if not fail_upload:
                    patch_idx = next(i for i, c in enumerate(calls) if c[0] == "PATCH")
                    self.assertTrue(
                        any(c[0] == "GET" and "/releases/tags/" in c[1]
                            for c in calls[patch_idx + 1:]),
                        "PATCH 公开成功后必须做一次 tags 端点验证")

    def test_existing_draft_is_reused_not_duplicated(self):
        # 重跑发布时的真实场景：草稿已存在且部分资产已上传，
        # 必须复用同一份草稿（POST /releases 不得再次出现）。
        with tmpdir() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")
            calls, uploads = [], []
            published = [False]  # PATCH draft:false 后，tags 端点才可见该 release
            draft = {"id": 7, "draft": True, "tag_name": "v1.4.0",
                     "upload_url": "https://upload.test/assets",
                     "assets": [{"name": "niuma-timer-1.4.0-portable.exe", "size": 8}]}
            def call(method, url, data=None, content_type=None):
                calls.append((method, url))
                if method == "GET":
                    if "/releases/tags/" in url:
                        if published[0]:
                            return 200, {"draft": False}
                        return 404, {}
                    if url.endswith("/releases?per_page=100"):
                        return 200, [draft]
                    if url.endswith("/releases/7"):
                        return 200, {"assets": draft["assets"] + uploads,
                                     "html_url": "https://example.test/release"}
                if method == "POST":
                    name = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)["name"][0]
                    uploads.append({"name": name, "size": len(data)})
                    return 201, {}
                if method == "PATCH":
                    self.assertFalse(json.loads(data)["draft"])
                    published[0] = True
                    return 200, {}
                raise AssertionError("unexpected call: %s %s" % (method, url))
            gh = mock.Mock()
            gh.call.side_effect = call
            argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0", "--package", d, "--root", d]
            with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"), \
                 mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh):
                self.assertEqual(release.main(), 0)
            self.assertFalse(any(c[0] == "POST" and c[1].endswith("/releases") for c in calls),
                             "must reuse the existing draft instead of creating a duplicate")
            self.assertEqual({u["name"] for u in uploads}, {"latest.json", "SHA256SUMS.txt"})

    def test_already_published_with_required_assets_is_verified_readonly(self):
        # 发布「成功但未证实」后的重跑：tags 端点已可见且非 draft，旧版一律
        # 「refusing to mutate」返回 1——CI 红字 + 重跑也红，与 post-verify 的
        # HINT「直接重跑确认」自相矛盾。现做只读核验：必需资产齐全 → 返 0。
        with tmpdir() as d:
            root = pathlib.Path(d)
            for name in ("niuma-timer-1.4.0-x64-setup.exe",
                         "niuma-timer-1.4.0-x64-setup.exe.sig"):
                (root / name).write_bytes(b"installer")
            calls = []
            published_assets = [
                {"name": "niuma-timer-1.4.0-x64-setup.exe", "size": 9},
                {"name": "niuma-timer-1.4.0-x64-setup.exe.sig", "size": 9},
                {"name": "latest.json", "size": 5},
                {"name": "SHA256SUMS.txt", "size": 5},
            ]

            def call(method, url, data=None, content_type=None):
                calls.append((method, url))
                if method == "GET" and "/releases/tags/" in url:
                    return 200, {"draft": False, "html_url": "https://example.test/r",
                                 "assets": published_assets}
                raise AssertionError("unexpected call: %s %s" % (method, url))

            gh = mock.Mock()
            gh.call.side_effect = call
            argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0",
                    "--package", d, "--root", d]
            with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"),                  mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh):
                self.assertEqual(release.main(), 0, "已公开且资产齐全必须返 0（幂等自证）")
            self.assertTrue(all(m == "GET" for m, _ in calls),
                            "自证路径只读：不得出现任何 POST/PATCH")

    def test_already_published_missing_assets_fails_without_mutation(self):
        with tmpdir() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-x64-setup.exe").write_bytes(b"installer")
            calls = []

            def call(method, url, data=None, content_type=None):
                calls.append((method, url))
                if method == "GET" and "/releases/tags/" in url:
                    # 远端缺 latest.json 与签名：残缺的已公开 release 必须报失败
                    return 200, {"draft": False, "html_url": "https://example.test/r",
                                 "assets": [{"name": "niuma-timer-1.4.0-x64-setup.exe", "size": 9}]}
                raise AssertionError("unexpected call: %s %s" % (method, url))

            gh = mock.Mock()
            gh.call.side_effect = call
            argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0",
                    "--package", d, "--root", d]
            with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"),                  mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh):
                self.assertEqual(release.main(), 1)
            self.assertTrue(all(m == "GET" for m, _ in calls), "失败路径同样只读")

    def test_already_published_remote_check_without_package_dir(self):
        # 手动重跑核验时本地常常没有 package 目录：退化为远端形态检查
        # （有安装包 + 有签名 + 有 latest.json），不因本地缺文件而误报。
        with tmpdir() as d:
            calls = []

            def call(method, url, data=None, content_type=None):
                calls.append((method, url))
                if method == "GET" and "/releases/tags/" in url:
                    return 200, {"draft": False, "html_url": "https://example.test/r",
                                 "assets": [
                                     {"name": "niuma-timer-1.4.0-x64-setup.exe", "size": 9},
                                     {"name": "niuma-timer-1.4.0-x64-setup.exe.sig", "size": 9},
                                     {"name": "latest.json", "size": 5},
                                 ]}
                raise AssertionError("unexpected call: %s %s" % (method, url))

            gh = mock.Mock()
            gh.call.side_effect = call
            argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0",
                    "--package", d + "-nonexistent", "--root", d]
            with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"),                  mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh):
                self.assertEqual(release.main(), 0)

    def test_post_publish_verify_rejects_still_draft(self):
        # PATCH 返回 200 但公开后的 release 仍是 draft（tags 可查但 draft=true）
        # → 必须退出码 1：v1.4.0 发布正是这类静默失败靠人眼兜底才发现的。
        # 注意 PATCH 前 tags 端点必须 404：主脚本第一步就按 tag 查既有 release，
        # 若提前返回 200 会被当成「已存在的草稿」走复用路径，测的就不是本缺陷。
        with tmpdir() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")
            uploads = []
            published = [False]

            def call(method, url, data=None, content_type=None):
                if method == "POST" and url.endswith("/releases"):
                    return 201, {"id": 7, "upload_url": "https://upload.test/assets", "assets": []}
                if method == "POST":
                    name = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)["name"][0]
                    uploads.append({"name": name, "size": len(data)})
                    return 201, {}
                if method == "PATCH":
                    published[0] = True
                    return 200, {}
                if method == "GET":
                    if "/releases/tags/" in url:
                        # PATCH 前 404（草稿无 tag 引用，正常新建路径）；
                        # PATCH 后 200 但 draft=true → 发布后验证必须拦下
                        if published[0]:
                            return 200, {"draft": True}
                        return 404, {}
                    if url.endswith("/releases?per_page=100"):
                        return 200, []
                    if url.endswith("/releases/7"):
                        return 200, {"assets": uploads, "html_url": "https://example.test/release"}
                return 404, {}

            gh = mock.Mock()
            gh.call.side_effect = call
            argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0", "--package", d, "--root", d]
            with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"), \
                 mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh):
                self.assertEqual(release.main(), 1)

    def test_empty_or_unsigned_installer_is_rejected(self):
        with tmpdir() as d:
            with self.assertRaises(ValueError):
                release.build_latest_json(d, "1.4.0", d, "owner/repo")
            (pathlib.Path(d) / "niuma-timer_1.4.0_x64-setup.exe").write_bytes(b"installer")
            (pathlib.Path(d) / "niuma-timer_1.4.0_x64-setup.exe.sig").write_text("")
            with self.assertRaises(ValueError):
                release.build_latest_json(d, "1.4.0", d, "owner/repo")

    def test_release_gate_precedes_git_and_publication(self):
        # The metadata gate must run before any irreversible git side effect ...
        batch = (pathlib.Path(__file__).parent.parent / "release.bat").read_text(encoding="utf-8")
        generation = batch.index("--generate-notes-only")
        self.assertLess(generation, batch.index('"%GIT%" add -A'))
        # ... and release.bat must NOT publish: since v1.5.1 the Release itself is
        # created by CI (release.yml -> publish_release.py). The old local path
        # (gh release create, --draft transitions, ASSETS block) was deliberately
        # removed, so asserting it here would just re-encode the abandoned design.
        self.assertNotIn("gh release", batch)
        self.assertNotIn("--draft", batch)
        self.assertIn("release.yml", batch)

    def test_release_tag_is_immutable(self):
        batch = (pathlib.Path(__file__).parent.parent / "release.bat").read_text(encoding="utf-8")
        tag_section = batch.split("rem ---------------- 9. tag ----------------", 1)[1].split(
            "rem ---------------- 10. push ----------------", 1)[0]
        self.assertIn('show-ref --verify --quiet "refs/tags/v%VER%"', tag_section)
        self.assertIn('if not "!TAG_COMMIT!"=="!HEAD_COMMIT!"', tag_section)
        self.assertNotIn("tag -d", tag_section)
        self.assertNotIn("push origin \"v%VER%\" --force", batch)
        self.assertIn('if not "%BRANCH%"=="main" (', batch)
        self.assertNotIn("Continue releasing from", batch)
        self.assertLess(batch.index('pushd "%ROOT%"'), batch.index("branch --show-current"))
        self.assertLess(batch.index('if not "%BRANCH%"=="main" ('), batch.index("rem ---------------- 1. version ----------------"))

    def test_cloud_publish_requires_main_and_ci_gates(self):
        # 门禁六步定义在 reusable workflow tests.yml（与 ci.yml 同源，防两份漂移）；
        # release.yml 必须调用它并把 release job 置于 needs 之下——门禁不过不打包。
        # 此前六步在 release.yml 与 ci.yml 各抄一份，本测试只钉 release.yml 自身，
        # 抄错一边测试照绿——正是这次抽 reusable workflow 要消灭的漂移土壤。
        wfdir = pathlib.Path(__file__).parent.parent / ".github" / "workflows"
        workflow = (wfdir / "release.yml").read_text(encoding="utf-8")
        tests_wf = (wfdir / "tests.yml").read_text(encoding="utf-8")
        self.assertIn("fetch-depth: 0", workflow)
        self.assertIn("$env:GITHUB_REF_TYPE -ne 'tag'", workflow)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", workflow)
        self.assertIn("uses: ./.github/workflows/tests.yml", workflow)
        self.assertIn("needs: tests", workflow)
        self.assertLess(workflow.index("needs: tests"), workflow.index("Build bundles"))
        for gate in ("cargo fmt --all -- --check", "cargo clippy --all-targets -- -D warnings",
                     "cargo test", "node scripts/run_all.js",
                     "python scripts/test_publish_release.py", "python scripts/test_push_via_api.py",
                     "cargo deny check advisories"):
            with self.subTest(gate=gate):
                self.assertIn(gate, tests_wf)
                self.assertNotIn(gate, workflow)
        # 两个 workflow 的 rust-cache 必须共享同一命名空间（默认按 job 隔离，
        # release job 永远全冷构建）
        self.assertEqual(tests_wf.count("shared-key: niuma-timer"), 1)
        self.assertIn("shared-key: niuma-timer", workflow)

    def test_release_bat_decouples_pdb_from_publish(self):
        # PDB policy since v1.5.1: the local flow refuses to tag a release whose
        # PDB is missing ("no .pdb in bin\\package"), but uploading the symbols is
        # CI's job (publish_release.py --pdb-only) - release.bat no longer does it.
        batch = (pathlib.Path(__file__).parent.parent / "release.bat").read_text(encoding="utf-8")
        self.assertIn("no .pdb in bin\\package", batch)
        self.assertNotIn("gh release upload", batch)
        self.assertNotIn("--pdb-only", batch)
        publisher = (pathlib.Path(__file__).parent / "publish_release.py").read_text(encoding="utf-8")
        self.assertIn("--pdb-only", publisher)

    def test_installer_and_unsigned_portable_have_distinct_entries(self):
        with tmpdir() as d:
            root = pathlib.Path(d)
            (root / "CHANGELOG.md").write_text("## [1.4.0]\n\nTest notes\n", encoding="utf-8")
            for name in ["niuma-timer_1.4.0_x64-setup.exe", "niuma-timer-1.4.0-portable.exe"]:
                (root / name).write_bytes(b"test artifact")
            (root / "niuma-timer_1.4.0_x64-setup.exe.sig").write_text("signature", encoding="utf-8")
            m = release.build_latest_json(d, "1.4.0", d, "owner/repo")
            self.assertEqual(m["platforms"]["windows-x86_64"], m["platforms"]["windows-x86_64-nsis"])
            self.assertTrue(m["platforms"]["windows-x86_64-portable"]["url"].endswith("portable.exe"))
            self.assertEqual(m["notes"], "Test notes")

    def test_orphan_signature_is_rejected(self):
        with tmpdir() as d:
            (pathlib.Path(d) / "niuma-timer_1.4.0_x64-setup.exe.sig").write_text("signature")
            with self.assertRaises(ValueError):
                release.build_latest_json(d, "1.4.0", d, "owner/repo")

    def test_debug_symbols_not_uploaded_in_default_flow(self):
        # PDB 不是运行时依赖：默认发布只发必需资产，调试符号由 --pdb-only
        # 在发布后 best-effort 补传（见 PdbOnlyTests）。PDB 失败绝不能阻断发布。
        with tmpdir() as d:
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
                 mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh):
                self.assertEqual(release.main(), 0)
            self.assertNotIn("niuma-timer-1.4.0-portable.pdb", uploads)
            sums = (root / "SHA256SUMS.txt").read_text(encoding="utf-8")
            self.assertIn("niuma-timer-1.4.0-portable.exe", sums)
            self.assertNotIn(".pdb", sums)


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
        with tmpdir() as d:
            p = pathlib.Path(d) / "niuma-timer-1.4.1-portable.pdb.zip"
            p.write_bytes(b"x" * 32)
            gh, calls = self._gh(lambda n: (500, {}) if n < 3 else (201, {}))
            with mock.patch.object(release.time, "sleep"):
                ok = release.upload_with_retry(gh, "https://upload.test/assets", str(p))
            self.assertTrue(ok)
            self.assertEqual(len(calls), 3)

    def test_client_error_not_retried(self):
        # 永久错误（422 已存在 / 403 无权限）不得重试，一次即判失败
        with tmpdir() as d:
            p = pathlib.Path(d) / "niuma-timer-1.4.1-portable.pdb.zip"
            p.write_bytes(b"x" * 32)
            gh, calls = self._gh(lambda n: (422, {}))
            with mock.patch.object(release.time, "sleep"):
                ok = release.upload_with_retry(gh, "https://upload.test/assets", str(p))
            self.assertFalse(ok)
            self.assertEqual(len(calls), 1)

    def test_network_error_is_retried(self):
        # 连接被重置（Errno 10054）属瞬时错误，必须被接住并重试
        with tmpdir() as d:
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
        with tmpdir() as d:
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
        with tmpdir() as d:
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
        with tmpdir() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.pdb.zip").write_bytes(b"symbols")

            def call(method, url, data=None, content_type=None):
                if method == "GET" and url.endswith("/releases?per_page=100"):
                    return 200, []
                return 404, {}

            self.assertEqual(self._run(d, call), 1)

    def test_no_pdb_present_is_not_a_failure(self):
        # package 里没有 PDB（例如手工补传已清理）→ 无操作即成功
        with tmpdir() as d:
            root = pathlib.Path(d)
            (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")

            def call(method, url, data=None, content_type=None):
                if method == "GET" and "/releases/tags/" in url:
                    return 200, {"draft": False, "id": 7,
                                 "upload_url": "https://upload.test/assets", "assets": []}
                return 404, {}

            self.assertEqual(self._run(d, call), 0)


class TokenTests(unittest.TestCase):
    """github_token 取值顺序：env GITHUB_TOKEN → env NIUMA_GITHUB_TOKEN → 凭据管理器。"""

    @staticmethod
    def _cred_run(password):
        """构造一个返回 wincred 令牌的 git credential fill mock。"""
        done = mock.Mock()
        done.stdout = password
        return mock.Mock(return_value=done)

    def test_env_github_token_wins(self):
        with mock.patch.dict(os.environ, {"GITHUB_TOKEN": "env-github-tok"}, clear=True), \
             mock.patch("subprocess.run", self._cred_run(b"password=cred-tok\n")):
            self.assertEqual(release.github_token(), "env-github-tok")

    def test_niuma_env_used_when_github_token_empty(self):
        # GITHUB_TOKEN 置空串视为未设置，轮到 NIUMA_GITHUB_TOKEN
        with mock.patch.dict(os.environ, {"GITHUB_TOKEN": "", "NIUMA_GITHUB_TOKEN": "env-niuma-tok"}, clear=True), \
             mock.patch("subprocess.run", self._cred_run(b"password=cred-tok\n")):
            self.assertEqual(release.github_token(), "env-niuma-tok")

    def test_env_beats_credential_store(self):
        # env 命中时凭据管理器不该被碰
        with mock.patch.dict(os.environ, {"GITHUB_TOKEN": "env-github-tok",
                                          "NIUMA_GITHUB_TOKEN": "env-niuma-tok"}, clear=True), \
             mock.patch("subprocess.run", self._cred_run(b"password=cred-tok\n")) as run:
            self.assertEqual(release.github_token(), "env-github-tok")
            run.assert_not_called()

    def test_falls_back_to_credential_store(self):
        with mock.patch.dict(os.environ, {"GITHUB_TOKEN": "", "NIUMA_GITHUB_TOKEN": ""}, clear=True), \
             mock.patch("subprocess.run", self._cred_run(b"password=cred-tok\n")):
            self.assertEqual(release.github_token(), "cred-tok")

    def test_none_when_no_env_and_no_credential(self):
        # 凭据管理器也没令牌 → None（调用方按无 token 处理）
        with mock.patch.dict(os.environ, {"GITHUB_TOKEN": "", "NIUMA_GITHUB_TOKEN": ""}, clear=True), \
             mock.patch("subprocess.run", self._cred_run(b"")):
            self.assertIsNone(release.github_token())


if __name__ == "__main__":
    unittest.main()
