#!/usr/bin/env python3
"""ui/bill-toolbar 分支的 API 推送：单提交经 Git Data API 重建（github.com 直连被墙）。"""
import base64
import json
import os
import subprocess
import sys
import tempfile

REPO = "wangdxnum1/niuma-timer"
API = "https://api.github.com/repos/" + REPO
BRANCH = "ui/bill-toolbar"


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
assert TOK


def api(method, url, payload=None):
    cmd = ["curl", "-s", "-X", method, API + url,
           "-H", "Authorization: token " + TOK,
           "-H", "Accept: application/vnd.github+json", "--max-time", "40"]
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


remote_main = api("GET", "/git/ref/heads/main")["object"]["sha"]
print("remote main:", remote_main[:10])
base_tree = api("GET", "/commits/" + remote_main)["commit"]["tree"]["sha"]
local_tree = git("rev-parse", "HEAD^{tree}").strip()
print("base tree:", base_tree[:10], "| local tree:", local_tree[:10])
if base_tree != local_tree:
    print("FATAL: local HEAD tree != remote main tree —— 内容基线不一致，中止")
    sys.exit(1)

changes = [l.split("\t") for l in
           git("diff-tree", "--no-commit-id", "--name-status", "-r", "HEAD").splitlines()]
entries = []
for st, path in changes:
    path = path.strip('"')
    content = git("show", "HEAD:" + path)
    want = git("rev-parse", "HEAD:" + path).strip()
    up = api("POST", "/git/blobs",
             {"content": base64.b64encode(content.encode("utf-8", "surrogateescape")).decode(),
              "encoding": "base64"})
    got = up.get("sha")
    print(f"  blob {path} -> {str(got)[:8]} [{'ok' if got == want else 'sha-differs'}]")
    if not got:
        sys.exit(1)
    entries.append({"path": path, "mode": "100644", "type": "blob", "sha": got})

tree = api("POST", "/git/trees", {"base_tree": base_tree, "tree": entries})
msg = git("log", "-1", "--format=%B", "HEAD")
commit = api("POST", "/git/commits", {"message": msg, "tree": tree["sha"], "parents": [remote_main]})
if "sha" not in commit:
    print("COMMIT FAIL", commit)
    sys.exit(1)
print("commit:", commit["sha"][:10])
r = api("POST", "/git/refs", {"ref": "refs/heads/" + BRANCH, "sha": commit["sha"]})
if "object" not in r:
    r = api("PATCH", "/git/refs/heads/" + BRANCH, {"sha": commit["sha"], "force": True})
print("ref:", r.get("object", {}).get("sha", "FAIL")[:10])
print("DONE")
