#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""本地假更新源：自动更新功能手测用，不发布、不碰生产。

运行（仓库根目录）：
    python serve/update_server.py                       # 端口 8000，限速 2 MB/s
    python serve/update_server.py --rate 1048576        # 限速 1 MB/s，进度条更慢
    python serve/update_server.py --fail-after 2097152  # 每次下载 2 MB 后掐断，模拟中断

行为：
- 以本脚本所在目录为根提供静态文件，把「假新版本」文件直接放这里即可；
- 每次请求 latest.json / SHA256SUMS.txt 时现场扫描目录重新生成：
    *portable*.exe          -> windows-x86_64-portable（signature 留空，绿色版只认 SHA256）
    *setup*.exe + 同名 .sig -> windows-x86_64-nsis + windows-x86_64 别名
    *.msi + 同名 .sig       -> windows-x86_64-msi（无 nsis 时兼任 windows-x86_64）
- .exe/.msi 下载默认限速（本地回环太快，进度条会一闪而过）；
- 支持 Range: bytes=N- 断点续传（绿色版重试时客户端会带这个头，服务端必须回 206）；
- --fail-after N：发出 N 字节后直接掐断连接，模拟「下载中断」——先带着它点更新，
  看到重试文案后，换成不带它的命令重启服务，绿色版应从断点续传；
- version 默认 99.0.0（比本地 1.4.1 大即触发更新），notes 为 markdown 样例，
  正好验证前端 ### 标题 / - 列表 / **粗体** 渲染。

测试完恢复生产配置：git checkout -- src-tauri
"""

import argparse
import hashlib
import json
import os
import re
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote

ROOT = os.path.dirname(os.path.abspath(__file__))
CHUNK = 64 * 1024
EXE_RE = re.compile(r".*\.(exe|msi)$", re.I)

RATE = 2 * 1024 * 1024      # 字节/秒，--rate 覆盖
FAIL_AFTER = 0              # >0 时每个下载发出这么多字节后掐断，--fail-after 覆盖
FAKE_VERSION = os.environ.get("NIUMA_FAKE_VERSION", "99.0.0")
HOST = "127.0.0.1"

NOTES = (
    "### 本地假更新源 99.0.0\n"
    "\n"
    "这是 **测试说明**，专门用来验证更新页的 markdown 渲染：\n"
    "\n"
    "- 一级条目：进度条应平滑推进\n"
    "- 二级条目：断点续传只对绿色版生效\n"
    "  - 嵌套条目：`inline code` 与 [链接](https://example.com) 也应正常\n"
    "\n"
    "**粗体行**：如果你能看到井号被渲染成标题，说明 renderMarkdown 生效。\n"
)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def scan():
    """扫描目录，返回 (manifest, sums_text)。每次请求都重新算，保证改文件即时生效。"""
    files = sorted(f for f in os.listdir(ROOT) if os.path.isfile(os.path.join(ROOT, f)))
    platforms = {}

    def entry(name, sig=""):
        return {"signature": sig, "url": "http://%s:%s/%s" % (HOST, PORT, name)}

    portable = next((f for f in files if EXE_RE.match(f) and "portable" in f.lower()), None)
    if portable:
        platforms["windows-x86_64-portable"] = entry(portable)

    setup = next((f for f in files if f.lower().endswith(".exe") and "setup" in f.lower()), None)
    if setup and setup + ".sig" in files:
        with open(os.path.join(ROOT, setup + ".sig"), encoding="utf-8") as fh:
            sig = fh.read().strip()
        if sig:
            platforms["windows-x86_64-nsis"] = entry(setup, sig)
            platforms["windows-x86_64"] = entry(setup, sig)

    msi = next((f for f in files if f.lower().endswith(".msi")), None)
    if msi and msi + ".sig" in files:
        with open(os.path.join(ROOT, msi + ".sig"), encoding="utf-8") as fh:
            sig = fh.read().strip()
        if sig:
            platforms["windows-x86_64-msi"] = entry(msi, sig)
            platforms.setdefault("windows-x86_64", entry(msi, sig))

    manifest = {
        "version": FAKE_VERSION,
        "notes": NOTES,
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    sums = "".join(
        "%s  %s\n" % (sha256_file(os.path.join(ROOT, f)), f)
        for f in files if EXE_RE.match(f)
    )
    return manifest, sums


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print("  [%s] %s" % (self.command, fmt % args), flush=True)

    def do_GET(self):
        path = unquote(self.path.split("?", 1)[0]).lstrip("/")
        try:
            if path in ("", "latest.json"):
                manifest, _ = scan()
                body = json.dumps(manifest, ensure_ascii=False, indent=2).encode("utf-8")
                self.send_body(200, body, "application/json; charset=utf-8")
            elif path == "SHA256SUMS.txt":
                _, sums = scan()
                self.send_body(200, sums.encode("utf-8"), "text/plain; charset=utf-8")
            else:
                self.send_file(path)
        except ConnectionAbortedError:
            print("  [cut] 客户端中断（或 --fail-after 掐断）", flush=True)

    def send_body(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def send_file(self, rel):
        # 只允许本目录内的文件，拒绝任何路径穿越
        fp = os.path.normpath(os.path.join(ROOT, rel))
        if not fp.startswith(ROOT) or not os.path.isfile(fp):
            self.send_body(404, b"not found", "text/plain")
            return
        size = os.path.getsize(fp)
        start, partial = 0, False
        rng = self.headers.get("Range", "")
        m = re.match(r"bytes=(\d+)-", rng)
        if m:
            start, partial = int(m.group(1)), True
            if start >= size:
                self.send_response(416)
                self.send_header("Content-Range", "bytes */%d" % size)
                self.end_headers()
                return
        self.send_response(206 if partial else 200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(size - start))
        if partial:
            self.send_header("Content-Range", "bytes %d-%d/%d" % (start, size - 1, size))
        self.end_headers()

        slow = EXE_RE.match(rel) and RATE > 0
        fail_after = FAIL_AFTER if slow else 0
        sent = 0
        with open(fp, "rb") as f:
            f.seek(start)
            t0 = time.monotonic()
            while True:
                chunk = f.read(CHUNK)
                if not chunk:
                    break
                self.wfile.write(chunk)
                sent += len(chunk)
                if fail_after and sent >= fail_after:
                    self.close_connection = True  # 模拟远端重置
                    print("  [cut] --fail-after 命中，发送 %d 字节后掐断" % sent, flush=True)
                    return
                if slow:
                    lag = sent / RATE - (time.monotonic() - t0)
                    if lag > 0:
                        time.sleep(lag)


def main():
    global RATE, FAIL_AFTER, PORT
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--rate", type=int, default=RATE, help="下载限速 字节/秒，0 不限速")
    ap.add_argument("--fail-after", type=int, default=0, help="下载 N 字节后掐断（模拟中断）")
    args = ap.parse_args()
    RATE, FAIL_AFTER, PORT = args.rate, args.fail_after, args.port

    manifest, sums = scan()
    print("假更新源根目录: %s" % ROOT)
    print("version=%s  平台键: %s" % (FAKE_VERSION, ", ".join(sorted(manifest["platforms"])) or "（无）"))
    if not any(k.startswith("windows-x86_64-portable") for k in manifest["platforms"]):
        print("[提示] 目录里没有 *portable*.exe，绿色版测不了——把构建产物拷一份进来")
    if "windows-x86_64-portable" in manifest["platforms"] and not sums:
        print("[警告] 有 portable exe 但哈希列表为空？")
    print("SHA256SUMS 条目: %d" % len(sums.splitlines()))
    print("监听 http://%s:%d  (Ctrl+C 退出)" % (HOST, PORT))
    ThreadingHTTPServer((HOST, PORT), Handler).serve_forever()


if __name__ == "__main__":
    main()
