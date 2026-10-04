#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Emergency push via the GitHub Git Data API (github.com blocked, api.github.com reachable).

2026-09-30 起本机常态：github.com 直连被墙（git push/fetch 全挂），api.github.com
可达。代理在运行时一切正常；代理不在时用本工具把本地提交推上去：

    python scripts/push_via_api.py <local-branch> [--base origin/main] [--force]

行为：找出 local-branch 相对 --base 的未推提交（rev-list --reverse），逐个经
Git Data API 重建（blob 上传 → base_tree 建树 → 建提交），最后创建/更新远端分支
引用。blob sha 用 `git rev-parse <commit>:<path>` 校验一致性，**不符立即中止**
（带病提交上了远端，比对修复还贵）。

注意：
- API 重建的提交 sha 与本地不同（committer 元数据），推完后本地分支与远端
  分叉属预期；代理恢复后 `git fetch origin && git reset --hard origin/<branch>`
  对齐（先核对两侧 tree sha 一致再 reset）。
- blob 内容走原始字节（git show 的 stdout 不解码）：二进制文件（png/zip/图标）
  原样上传。旧版按 utf-8/replace 解码再回编，二进制会被静默换成损坏内容且
  SHA-DIFFERS 只打印不中止——两个坑都已堵上。
- 大 payload 走临时文件（Windows 命令行 32K 上限）。
- 令牌：环境变量 NIUMA_GITHUB_TOKEN / GITHUB_TOKEN 优先（与 publish_release.py
  同源），否则读 Git 凭据管理器中已存的 github.com 令牌。离线单测（本目录
  test_push_via_api.py）依赖「import 不取令牌」，别把凭据读取挪回模块级。
"""
import argparse
import base64
import hashlib
import json
import os
import subprocess
import sys
import tempfile

# Single source of repo slug: keep in sync with release.bat REPO
# (scripts/test_local_gate.js asserts both literals match).
REPO = "wangdxnum1/niuma-timer"
API = "https://api.github.com/repos/" + REPO

# main() 里注入；模块级不取凭据——无令牌的机器上 import 即崩，且离线单测没法跑
TOK = None


def token():
    for var in ("NIUMA_GITHUB_TOKEN", "GITHUB_TOKEN"):
        v = os.environ.get(var)
        if v and v.strip():
            return v.strip()
    out = subprocess.run(
        ["git", "-c", "credential.helper=wincred", "credential", "fill"],
        input=b"protocol=https\nhost=github.com\n\n", capture_output=True,
    ).stdout.decode("utf-8", "replace")
    for line in out.splitlines():
        if line.startswith("password="):
            return line[9:].strip()
    return None


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


def git_bytes(*args):
    """git 子进程的原始字节输出。blob 内容必须走这里：utf-8 解码再回编会
    把二进制文件（png/zip）静默换成 'replace' 后的损坏内容。"""
    return subprocess.run(["git", *args], capture_output=True).stdout


def git(*args):
    return git_bytes(*args).decode("utf-8", "replace")


def blob_sha1(raw: bytes) -> str:
    """Git blob sha = sha1("blob <size>\\0" + content)。测试用假 API 据此返回
    真校验值，交叉验证上传内容与 git rev-parse 的一致性。"""
    h = hashlib.sha1()
    h.update(b"blob " + str(len(raw)).encode("ascii") + b"\0" + raw)
    return h.hexdigest()


def push_branch(branch, base_ref, force=False, tag=None):
    commits = git("rev-list", "--reverse", f"{base_ref}..{branch}").split()
    if not commits:
        print("nothing to push")
        return 1

    # 远端基线：基线引用的远端 tip 即首个新提交的父。引用名随 --base 走
    # （origin/main -> main），不再写死 main——否则 --base 指向别的分支时
    # 基线树比对必 FATAL。
    remote_branch = base_ref.split("/")[-1]
    ref = api("GET", "/git/ref/heads/" + remote_branch)
    parent = ref.get("object", {}).get("sha")
    if not parent:
        print("REF FAIL: 远端 heads/" + remote_branch + " 不存在或读取失败", ref)
        return 1
    commit_info = api("GET", "/commits/" + parent)
    parent_tree = commit_info.get("commit", {}).get("tree", {}).get("sha")
    if not parent_tree:
        print("COMMIT READ FAIL", commit_info)
        return 1

    # 基线校验：本地基线引用的树必须与远端一致（内容等同的前提）。
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
            content = git_bytes("show", f"{c}:{path}")
            want = git("rev-parse", f"{c}:{path}").strip()
            up = api("POST", "/git/blobs",
                     {"content": base64.b64encode(content).decode(),
                      "encoding": "base64"})
            got = up.get("sha")
            print(f"  blob {path} [{'ok' if got == want else 'SHA-DIFFERS'}]")
            if got != want:
                # 校验不符必须中止：坏 blob 进 tree 会产出内容损坏的远端提交，
                # 而流程继续会照样「成功」。旧版只打印 SHA-DIFFERS 不停下。
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
    if not isinstance(r.get("object"), dict):
        # 分支已存在 → PATCH。force 默认 False：非快进会被 GitHub 拒绝并如实报错，
        # 不再无条件强推（旧版恒 force:true，--force 是个假旗标）。
        r = api("PATCH", "/git/refs/heads/" + branch, {"sha": parent, "force": bool(force)})
    ok = isinstance(r.get("object"), dict)
    print("ref:", ("-> " + r["object"]["sha"][:10]) if ok else r)
    if not ok:
        return 1
    # 机读行：release.bat 的 API 备援路径据此确认远端 head（人读的 ref: 行保留）
    print("REMOTE_SHA: " + parent)
    if tag:
        # tag 必须与分支同一次调用创建：远端 head sha 是 API 重建的，与本地不同，
        # 回去用 git push 推本地 tag 会指向一个远端不存在的提交并触发 CI 构建它。
        # 轻量 tag（ref 直指提交）足以触发 release.yml 的 on: push: tags。
        tr = api("POST", "/git/refs", {"ref": "refs/tags/" + tag, "sha": parent})
        if isinstance(tr.get("object"), dict):
            print("tag:", tag, "->", parent[:10])
        else:
            # 已存在 → 只读核验：指向同一提交才算幂等成功；发布 tag 不可变，
            # 指向不一致时如实报错并要求换新版本号，绝不强改。
            cur = api("GET", "/git/ref/tags/" + tag)
            cur_sha = (cur.get("object") or {}).get("sha")
            if cur_sha == parent:
                print("tag:", tag, "already at", parent[:10], "(idempotent)")
            else:
                print("TAG FAIL: refs/tags/" + tag, "already exists at", cur_sha,
                      "but this push head is", parent,
                      "- release tags are immutable, use a new version")
                return 1
    return 0


def main():
    global TOK
    ap = argparse.ArgumentParser()
    ap.add_argument("branch", help="本地分支名（其相对基线的提交将被推送）")
    ap.add_argument("--base", default="origin/main", help="基线引用（默认 origin/main）")
    ap.add_argument("--force", action="store_true",
                    help="远端分支已存在且非快进时强制覆盖（默认拒绝）")
    ap.add_argument("--tag", metavar="NAME",
                    help="分支推送成功后在远端 head 上创建轻量 tag 引用"
                         "（refs/tags/NAME，触发 release.yml 的 on: push: tags）；"
                         "已存在且指向一致视为幂等成功，指向不一致则报错拒绝")
    a = ap.parse_args()
    TOK = token()
    if not TOK:
        print("no GitHub token: set NIUMA_GITHUB_TOKEN / GITHUB_TOKEN, "
              "or store one for github.com in the git credential manager")
        return 1
    return push_branch(a.branch, a.base, a.force, a.tag)


if __name__ == "__main__":
    sys.exit(main())
