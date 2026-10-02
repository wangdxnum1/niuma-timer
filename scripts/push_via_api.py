#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Emergency push via the GitHub Git Data API (github.com blocked, api.github.com reachable).

2026-09-30 起本机常态：github.com 直连被墙（git push/fetch 全挂），api.github.com
可达。代理在运行时一切正常；代理不在时用本工具把本地提交推上去：

    python scripts/push_via_api.py <local-branch> [--base origin/main] [--force]

行为：找出 local-branch 相对 --base 的未推提交（rev-list --reverse），逐个经
Git Data API 重建（blob 上传 → base_tree 建树 → 建提交），最后创建/强推远端分支
引用。blob sha 用 `git rev-parse <commit>:<path>` 校验一致性。

注意：
- API 重建的提交 sha 与本地不同（committer 元数据），推完后本地分支与远端
  分叉属预期；代理恢复后 `git fetch origin && git reset --hard origin/<branch>`
  对齐（先核对两侧 tree sha 一致再 reset）。
- 大 payload 走临时文件（Windows 命令行 32K 上限）。
- 需要 Git 凭据管理器中已存的 github.com 令牌（与 publish_release.py 同源）。
"""
import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile

REPO = "wangdxnum1/niuma-timer"
API = "https://api.github.com/repos/" + REPO


def token():
    out = subprocess.run(
        ["git", "-c", "credential.helper=wincred", "credential", "fill"],
        input=b"protocol=https\nhost=github.com\n\n", capture_output=True,
    ).stdout.decode("utf-8", "replace")
    for line in out.splitlines():
        if line.startswith("password="):
            return line[9:].strip()
    return None


TOK = token()
assert TOK, "no GitHub token in git credential store"


def api(method, url, payload=None):
    cmd = ["curl", "-s", "-X", method, API + url,
           "-H", "Authorization: token " + TOK,
           "-H", "Accept: application/vnd.github+json", "--max-time", "60"]
    tmp = None
    if payload is not None:
        tmp = tempfile.NamedTemporaryFile(delete=False, suffix=".json")
        tmp.write(json.dumps(payload, ensure_ascii=False).encode("utf-8"))
        tmp.close()
        cmd += ["-H", "Content-Type: application/json", "-d", "@" + tmp.name]
    r = subprocess.run(cmd, capture_output=True)
    if tmp:
        os.unlink(tmp.name)
    return json.loads(r.stdout.decode("utf-8", "replace") or "{}")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True).stdout.decode("utf-8", "replace")


def push_branch(branch, base_ref):
    commits = git("rev-list", "--reverse", f"{base_ref}..{branch}").split()
    assert commits, "nothing to push"

    # 远端基线：main tip 即首个新提交的父
    parent = api("GET", "/git/ref/heads/main")["object"]["sha"]
    parent_tree = api("GET", "/commits/" + parent)["commit"]["tree"]["sha"]

    # 基线校验：本地基线引用的树必须与远端 main 一致（内容等同的前提）。
    # 本地 origin/main 过期时这里会失败——先 fetch 对齐再推。
    local_base_tree = git("rev-parse", f"{base_ref}^{{tree}}").strip()
    print("base:", parent[:10], "| local tree:", local_base_tree[:10],
          "| remote tree:", parent_tree[:10], "| equal:", local_base_tree == parent_tree)
    if local_base_tree != parent_tree:
        print("FATAL: base tree mismatch - 本地基线与远端内容不一致，先对齐再推")
        return 1

    for c in commits:
        msg = git("log", "-1", "--format=%B", c)
        changes = [l.split("	") for l in
                   git("diff-tree", "--no-commit-id", "--name-status", "-r", c).splitlines()]
        entries = []
        for st, path in changes:
            path = path.strip('"')
            if st == "D":
                entries.append({"path": path, "mode": "100644", "type": "blob", "sha": None})
                continue
            content = git("show", f"{c}:{path}")
            want = git("rev-parse", f"{c}:{path}").strip()
            up = api("POST", "/git/blobs",
                     {"content": base64.b64encode(
                         content.encode("utf-8", "surrogateescape")).decode(),
                      "encoding": "base64"})
            got = up.get("sha")
            print(f"  blob {path} [{'ok' if got == want else 'SHA-DIFFERS'}]")
            if not got:
                print("BLOB FAIL", up)
                return 1
            entries.append({"path": path, "mode": "100644", "type": "blob", "sha": got})
        tree = api("POST", "/git/trees", {"base_tree": parent_tree, "tree": entries})
        if "sha" not in tree:
            print("TREE FAIL", tree)
            return 1
        commit = api("POST", "/git/commits",
                     {"message": msg, "tree": tree["sha"], "parents": [parent]})
        if "sha" not in commit:
            print("COMMIT FAIL", commit)
            return 1
        print(f"commit {c[:8]} -> {commit['sha'][:8]}  {msg.splitlines()[0][:50]}")
        parent = commit["sha"]
        parent_tree = tree["sha"]

    r = api("POST", "/git/refs", {"ref": "refs/heads/" + branch, "sha": parent})
    if "object" not in r:
        r = api("PATCH", "/git/refs/heads/" + branch, {"sha": parent, "force": True})
    ok = isinstance(r.get("object"), dict)
    print("ref:", ("-> " + r["object"]["sha"][:10]) if ok else r)
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("branch", help="本地分支名（其相对基线的提交将被推送）")
    ap.add_argument("--base", default="origin/main", help="基线引用（默认 origin/main）")
    ap.add_argument("--force", action="store_true", help="远端分支已存在时强制覆盖")
    a = ap.parse_args()
    sys.exit(push_branch(a.branch, a.base))


if __name__ == "__main__":
    main()
