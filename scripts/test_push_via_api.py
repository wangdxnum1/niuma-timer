"""Offline tests for the emergency Git Data API push tool; no network, no credentials.

push_via_api.py 的历史坑都钉在这里：
1) blob 内容按原始字节上传——旧版按 utf-8/replace 解码再回编，二进制文件
   （png/zip/图标）被静默换成损坏内容且照样「成功」；
2) blob sha 校验不符立即中止——旧版只打印 SHA-DIFFERS 照推不误；
3) import 不取凭据——旧版模块级 assert TOK，无令牌机器 import 即崩、离线没法测。
"""
import base64
import contextlib
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import push_via_api as push


def run(*args, cwd, stdin=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, input=stdin)
    if r.returncode != 0:
        raise AssertionError(
            "git {} failed: {}".format(args, r.stderr.decode("utf-8", "replace")))
    return r.stdout


@contextlib.contextmanager
def git_repo():
    """Two-commit scratch repo: base (pushed baseline) -> main (one new commit).

    Uses os.makedirs (not tempfile.TemporaryDirectory): the 0700 ACL of the latter
    is unwritable under a restricted token - see test_publish_release.py tmpdir().
    Also chdirs into the repo: push_branch() shells out to git in the process cwd.
    """
    d = os.path.join(tempfile.gettempdir(), "niuma-pushapi-" + uuid.uuid4().hex)
    os.makedirs(d, exist_ok=True)
    old = os.getcwd()
    try:
        run("init", "-q", "-b", "main", cwd=d)
        run("config", "user.name", "t", cwd=d)
        run("config", "user.email", "t@example.com", cwd=d)
        # autocrlf=false：否则 git 在 add 时把 CRLF 归一化成 LF，字节级保真断言失真
        run("config", "core.autocrlf", "false", cwd=d)
        (pathlib.Path(d) / "base.txt").write_bytes(b"base\n")
        run("add", ".", cwd=d)
        run("commit", "-q", "-m", "base", cwd=d)
        base = run("rev-parse", "HEAD", cwd=d).decode().strip()
        run("branch", "baseline", cwd=d)  # 本地基线引用（push_branch --base 的远端名取最后一段）
        os.chdir(d)
        yield d, base
    finally:
        os.chdir(old)
        shutil.rmtree(d, ignore_errors=True)


class FakeApi:
    """Stand-in for push.api(): blobs are answered with the real git blob sha of
    the uploaded content (so only byte-exact uploads pass), everything else is
    recorded. POST /git/refs fails like an existing branch, forcing the PATCH path."""

    def __init__(self, parent, tree, blob_sha_override=None):
        self.parent = parent
        self.tree = tree
        self.blob_sha_override = blob_sha_override
        self.blob_payloads = []
        self.calls = []
        self.patch_force = None

    def __call__(self, method, url, payload=None):
        self.calls.append(method + " " + url)
        if method == "GET" and url.startswith("/git/ref/heads/"):
            return {"object": {"sha": self.parent}}
        if method == "GET" and url.startswith("/commits/"):
            return {"commit": {"tree": {"sha": self.tree}}}
        if method == "POST" and url == "/git/blobs":
            raw = base64.b64decode(payload["content"])
            self.blob_payloads.append(raw)
            return {"sha": self.blob_sha_override or push.blob_sha1(raw)}
        if method == "POST" and url == "/git/trees":
            return {"sha": "fake-tree-" + str(len(self.calls))}
        if method == "POST" and url == "/git/commits":
            return {"sha": "fake-commit-" + str(len(self.calls))}
        if method == "POST" and url.startswith("/git/refs"):
            return {"message": "Reference already exists"}  # 触发 PATCH 路径
        if method == "PATCH" and url.startswith("/git/refs/heads/"):
            self.patch_force = payload.get("force")
            return {"object": {"sha": payload["sha"]}}
        raise AssertionError("unexpected api call: {} {}".format(method, url))


class PushViaApiTests(unittest.TestCase):
    def _add_second_commit(self, d, binary=b"\x00\xff\xfePNG\x89\xd9\r\n\x80\x81", note=None):
        """Second commit on main carrying a binary file and a CRLF+中文 text file."""
        note = note if note is not None else "line1\r\nline2 中文\r\n"
        root = pathlib.Path(d)
        (root / "bin.dat").write_bytes(binary)
        (root / "note.txt").write_bytes(note.encode("utf-8"))
        run("add", ".", cwd=d)
        run("commit", "-q", "-m", "add binary + text", cwd=d)

    def _fake_for_baseline(self, d, base, **kw):
        tree = run("rev-parse", base + "^{tree}", cwd=d).decode().strip()
        return FakeApi(base, tree, **kw)

    def test_blob_sha1_matches_git_hash_object(self):
        with git_repo() as (d, _):
            raw = b"\x00\xff\xfe\x89PNG-cross-check"
            ours = push.blob_sha1(raw)
            theirs = run("hash-object", "--stdin", cwd=d, stdin=raw).decode().strip()
            self.assertEqual(ours, theirs, "blob_sha1 必须与 git hash-object 一致")

    def test_import_does_not_fetch_token(self):
        """子进程干净导入：TOK 保持 None，不读环境变量也不碰凭据管理器。"""
        env = dict(os.environ)
        env.pop("NIUMA_GITHUB_TOKEN", None)
        env.pop("GITHUB_TOKEN", None)
        scripts = os.path.dirname(os.path.abspath(__file__))
        r = subprocess.run(
            [sys.executable, "-c",
             "import push_via_api; assert push_via_api.TOK is None, 'token fetched at import'"],
            capture_output=True, cwd=scripts, env=env)
        self.assertEqual(r.returncode, 0, r.stderr.decode("utf-8", "replace"))

    def test_binary_and_text_blobs_upload_byte_exact(self):
        with git_repo() as (d, base):
            self._add_second_commit(d)
            fake = self._fake_for_baseline(d, base)
            with mock.patch.object(push, "api", fake):
                self.assertEqual(push.push_branch("main", "baseline"), 0)
            self.assertEqual(len(fake.blob_payloads), 2, "应上传两个 blob")
            expected_bin = b"\x00\xff\xfePNG\x89\xd9\r\n\x80\x81"
            self.assertIn(expected_bin, fake.blob_payloads,
                          "二进制内容必须逐字节一致（旧版 utf-8/replace 会损坏）")
            expected_note = "line1\r\nline2 中文\r\n".encode("utf-8")
            self.assertIn(expected_note, fake.blob_payloads, "文本（含 CRLF）必须逐字节一致")
            self.assertIs(fake.patch_force, False, "不带 --force 时 PATCH 不得强推")

    def test_sha_mismatch_aborts_before_tree(self):
        with git_repo() as (d, base):
            self._add_second_commit(d)
            fake = self._fake_for_baseline(d, base, blob_sha_override="deadbeef" * 5)
            with mock.patch.object(push, "api", fake):
                self.assertEqual(push.push_branch("main", "baseline"), 1,
                                 "blob sha 校验不符必须返回非零")
            self.assertFalse(any("/git/trees" in c for c in fake.calls),
                             "sha 不符后不得继续建树/建提交/推引用")

    def test_force_flag_reaches_patch(self):
        with git_repo() as (d, base):
            self._add_second_commit(d)
            fake = self._fake_for_baseline(d, base)
            with mock.patch.object(push, "api", fake):
                self.assertEqual(push.push_branch("main", "baseline", force=True), 0)
            self.assertIs(fake.patch_force, True, "--force 必须真实传递给 PATCH")


if __name__ == "__main__":
    unittest.main()
