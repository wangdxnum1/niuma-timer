#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Publish a GitHub release for Niuma Timer without the gh CLI.

The OAuth token that Git Credential Manager already stores is reused, so there
is no interactive login step. Everything is idempotent: an existing release is
reused and assets that are already uploaded are skipped.

Usage:
    python scripts/publish_release.py --tag v1.0.0 --version 1.0.0 \
        --package bin/package [--repo owner/repo] [--title "..."] \
        [--generate-notes-only]

Exit codes: 0 = ok, 1 = failed (caller should fall back to the browser).
"""

import argparse
import hashlib
import json
import mimetypes
import os
import re
import socket
import ssl
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_REPO = "wangdxnum1/niuma-timer"
API = "https://api.github.com"
UPLOADS = "https://uploads.github.com"
# Env overrides so these no longer require editing the script:
#   NIUMA_PROXY           "host:port" of the local proxy to probe (default
#                         127.0.0.1:7890); "off" skips the probe, always direct
#   NIUMA_DEFAULT_BRANCH  branch new releases target (default main)
PROXY = os.environ.get("NIUMA_PROXY", "127.0.0.1:7890")
DEFAULT_BRANCH = os.environ.get("NIUMA_DEFAULT_BRANCH", "main")


def log(msg):
    print(msg, flush=True)


# --------------------------------------------------------------------------
# network helpers
# --------------------------------------------------------------------------
def build_opener():
    # GitHub is unreachable directly from some networks, so probe the local
    # Clash mixed port and route through it when it is up. TLS verification is
    # relaxed ONLY on the proxy path (Clash MITM presents a self-signed cert);
    # direct connections keep strict certificate validation.
    if PROXY.lower() == "off":
        log("  network: direct (NIUMA_PROXY=off)")
        return urllib.request.build_opener()
    host, _, port_s = PROXY.rpartition(":")
    if not host or not port_s.isdigit():
        log("  network: NIUMA_PROXY=%r is not host:port, going direct" % PROXY)
        return urllib.request.build_opener()
    if not proxy_alive(host, int(port_s)):
        log("  network: direct")
        return urllib.request.build_opener()
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    proxy_url = "http://%s:%s" % (host, port_s)
    op = urllib.request.build_opener(
        urllib.request.HTTPSHandler(context=ctx),
        urllib.request.ProxyHandler({"https": proxy_url, "http": proxy_url}))
    log("  network: using local proxy %s (TLS check relaxed for its self-signed cert)" % proxy_url)
    return op


def proxy_alive(host, port, timeout=2):
    try:
        s = socket.create_connection((host, port), timeout=timeout)
        s.close()
        return True
    except OSError:
        return False


class GitHub:
    def __init__(self, token, opener):
        self.opener = opener
        self.headers = {
            "Authorization": "token %s" % token,
            "User-Agent": "niuma-release",
            "Accept": "application/vnd.github+json",
        }

    def call(self, method, url, data=None, content_type=None, timeout=300):
        headers = dict(self.headers)
        if content_type:
            headers["Content-Type"] = content_type
        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with self.opener.open(req, timeout=timeout) as r:
                raw = r.read()
                return r.status, (json.loads(raw) if raw else {})
        except urllib.error.HTTPError as e:
            return e.code, {"error": e.read().decode("utf-8", "replace")[:600]}


# --------------------------------------------------------------------------
# token
# --------------------------------------------------------------------------
def github_token():
    """Reuse the credential Git Credential Manager already stored for github.com."""
    candidates = [r"C:\Program Files\Git\cmd\git.exe", "git"]
    for git in candidates:
        try:
            p = subprocess.run([git, "-c", "credential.helper=wincred",
                                "credential", "fill"],
                               input=b"protocol=https\nhost=github.com\n\n",
                               capture_output=True, timeout=30)
        except (OSError, subprocess.SubprocessError):
            continue
        for line in p.stdout.decode("utf-8", "replace").splitlines():
            if line.startswith("password="):
                tok = line[len("password="):].strip()
                if tok:
                    log("  token: reused from git credential store (%s...)" % tok[:3])
                    return tok
    return None


# --------------------------------------------------------------------------
# release notes
# --------------------------------------------------------------------------
def human_size(n):
    return "%.1f MB" % (n / 1024.0 / 1024.0)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def describe(fname):
    low = fname.lower()
    if low.endswith(".msi"):
        return "MSI 包，适合企业分发 / 静默安装"
    if "setup" in low:
        return "**推荐**。NSIS 安装向导（中文界面），装到开始菜单，可设开机自启"
    if "portable" in low:
        return "便携版，双击即用，不安装、不写注册表"
    return ""


def changelog_section(root, version):
    """Pull the section for `version` out of CHANGELOG.md (if present).

    Falls back to the [未发布] section with a warning when the version has no
    section of its own, so the "what changed" block never disappears from the
    release notes silently.
    """
    path = os.path.join(root, "CHANGELOG.md")
    if not os.path.isfile(path):
        return ""
    try:
        text = open(path, encoding="utf-8").read()
    except OSError:
        return ""

    def grab(pat):
        m = re.search(pat, text, re.S | re.M)
        if not m:
            return ""
        body = m.group(1).strip()
        # drop the "release link" definition line that sits at the very end
        return re.sub(r"\n\[[^\]]+\]:\s*\S+\s*$", "", body).strip()

    body = grab(r"^##\s+\[?%s\]?[^\n]*\n(.*?)(?=^##\s|\Z)" % re.escape(version))
    if body:
        return body
    body = grab(r"^##\s+\[?未发布\]?[^\n]*\n(.*?)(?=^##\s|\Z)")
    if body:
        log("  [WARN] CHANGELOG.md has no section for %s; using the [未发布] section" % version)
        return body
    log("  [WARN] CHANGELOG.md has no section for %s (and no [未发布] fallback)" % version)
    return ""


def build_notes(root, version, package_dir):
    files = []
    if os.path.isdir(package_dir):
        for name in sorted(os.listdir(package_dir)):
            p = os.path.join(package_dir, name)
            if os.path.isfile(p) and name.lower().endswith((".exe", ".msi")):
                files.append((name, os.path.getsize(p), sha256_file(p)))
    files.sort(key=lambda kv: (".msi" in kv[0].lower(), "portable" in kv[0].lower()))

    lines = []
    lines.append("# 牛马计时器 %s" % version)
    lines.append("")
    lines.append("常驻 Windows 托盘的工资计时器：实时显示**今天赚了多少钱**，"
                 "以及已工作时长、距下班、每分钟赚多少、距发薪日。")
    lines.append("")
    if files:
        lines.append("## 下载")
        lines.append("")
        lines.append("| 文件 | 大小 | SHA-256 | 说明 |")
        lines.append("| --- | --- | --- | --- |")
        for name, size, digest in files:
            lines.append("| `%s` | %s | `%s` | %s |"
                         % (name, human_size(size), digest[:8], describe(name)))
        lines.append("")
        lines.append("完整 SHA-256 校验和见附件 `SHA256SUMS.txt`。")
        lines.append("")
    lines.append("> 需要 Windows 10/11（64 位），依赖 WebView2 运行时。"
                 "Windows 11 已自带；Windows 10 若缺失，安装包会自动引导安装。")
    lines.append("> 便携版不自带 WebView2，如遇白窗请先手动安装 "
                 "[WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/)。")
    lines.append("")

    section = changelog_section(root, version)
    if section:
        lines.append("## 本次变更")
        lines.append("")
        lines.append(section)
        lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("完整变更记录见 "
                 "[CHANGELOG](https://github.com/%s/blob/main/CHANGELOG.md)。" % DEFAULT_REPO)
    return "\n".join(lines)


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--package", required=True)
    ap.add_argument("--repo", default=DEFAULT_REPO)
    ap.add_argument("--title", default=None)
    ap.add_argument("--root", default=None)
    ap.add_argument("--generate-notes-only", action="store_true")
    args = ap.parse_args()

    root = args.root or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    package_dir = args.package
    if not os.path.isabs(package_dir):
        package_dir = os.path.join(root, package_dir)

    notes_path = os.path.join(package_dir, "RELEASE_NOTES.md")
    if os.path.isfile(notes_path):
        log("  notes: reusing %s" % notes_path)
    else:
        notes = build_notes(root, args.version, package_dir)
        os.makedirs(package_dir, exist_ok=True)
        with open(notes_path, "w", encoding="utf-8", newline="\n") as f:
            f.write(notes)
        log("  notes: generated %s" % notes_path)

    if args.generate_notes_only:
        return 0

    token = github_token()
    if not token:
        log("  [ERROR] no GitHub token available from the git credential store")
        return 1

    gh = GitHub(token, build_opener())
    api = "%s/repos/%s" % (API, args.repo)

    st, rel = gh.call("GET", "%s/releases/tags/%s" % (api, args.tag))
    if st == 200:
        log("  release: already exists, reusing id=%s" % rel.get("id"))
    else:
        body = open(notes_path, encoding="utf-8").read().splitlines()
        if body and body[0].startswith("# "):
            body = body[1:]
            while body and not body[0].strip():
                body = body[1:]
        payload = {
            "tag_name": args.tag,
            "name": args.title or ("Niuma Timer %s" % args.version),
            "body": "\n".join(body).strip(),
            "draft": False,
            "prerelease": False,
            "target_commitish": DEFAULT_BRANCH,
        }
        data = json.dumps(payload).encode("utf-8")
        st, rel = gh.call("POST", "%s/releases" % api, data,
                          "application/json; charset=utf-8")
        if st not in (200, 201):
            log("  [ERROR] create release failed HTTP %s: %s" % (st, rel))
            return 1
        log("  release: created %s" % rel.get("html_url"))

    upload_base = (rel.get("upload_url") or "").split("{")[0]
    if not upload_base:
        log("  [ERROR] no upload_url in the release payload")
        return 1

    existing = {a["name"] for a in rel.get("assets", [])}
    assets = []
    if os.path.isdir(package_dir):
        for name in sorted(os.listdir(package_dir)):
            p = os.path.join(package_dir, name)
            if os.path.isfile(p) and name.lower().endswith((".exe", ".msi")):
                assets.append(p)

    if not assets:
        log("  [WARN] no .exe/.msi found in %s" % package_dir)
    else:
        # Digest sheet shipped as an asset; the notes table links to it.
        sums_path = os.path.join(package_dir, "SHA256SUMS.txt")
        with open(sums_path, "w", encoding="utf-8", newline="\n") as f:
            for path in assets:
                f.write("%s  %s\n" % (sha256_file(path), os.path.basename(path)))
        assets.append(sums_path)

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
        for attempt in (1, 2):  # one retry: uploads through the proxy hiccup
            st, res = gh.call("POST", url, data, ctype)
            if st in (200, 201):
                break
            if attempt == 1:
                log("  [retry] %s failed HTTP %s, retrying once ..." % (name, st))
        if st in (200, 201):
            log("  [ok] %s  %.1f MB" % (name, len(data) / 1024.0 / 1024.0))
        else:
            log("  [FAIL] %s HTTP %s: %s" % (name, st, res))
            failed = True

    st, final = gh.call("GET", "%s/releases/tags/%s" % (api, args.tag))
    if st == 200:
        log("")
        log("RELEASE_URL=%s" % final.get("html_url"))
        for a in final.get("assets", []):
            log("  - %s  %.1f MB" % (a["name"], a["size"] / 1024.0 / 1024.0))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
