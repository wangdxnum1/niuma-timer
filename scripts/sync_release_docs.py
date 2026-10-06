#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""发版文档同步（release.bat 版本同步段自动调用）：把版本变更覆盖到全部五处。

背景（2026-10-05/06 两次翻车）：release.bat 只同步 Cargo.toml + tauri.conf.json，
README×2 与 CHANGELOG 归段靠人记，test_readme 的五处一致性断言在测试步骤必红，
abort 后还留下半同步的脏工作区。现在 release.bat 在版本同步段调用本脚本，
`release.bat X.Y.Z /y` 一条命令自洽，不再有前置记忆项。

幂等：CHANGELOG 已存在目标版本段、README 已是目标版本时按成功处理（abort 后重跑安全）。
"""

import datetime
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def sync_docs(root: pathlib.Path, version: str, date: str) -> None:
    """README×2 版本行 + CHANGELOG [未发布] 归段。不满足前置时抛 RuntimeError。"""
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise RuntimeError(f"版本号应为 X.Y.Z：{version}")

    # README ×2：版本行「**X.Y.Z** (日期)」→ 目标版本 + 当天日期
    for name, old_line, new_line in [
        ("README.md",
         re.compile(r"> Current version \*\*\d+\.\d+\.\d+\*\* \(\d{4}-\d{2}-\d{2}\) · \[Changelog\]"),
         f"> Current version **{version}** ({date}) · [Changelog]"),
        ("README.zh-CN.md",
         re.compile(r"> 当前版本 \*\*\d+\.\d+\.\d+\*\*（\d{4}-\d{2}-\d{2}）· \[更新日志\]"),
         f"> 当前版本 **{version}**（{date}）· [更新日志]"),
    ]:
        p = root / name
        text = p.read_text(encoding="utf-8")
        if not old_line.search(text):
            raise RuntimeError(f"{name} 缺版本行，无法同步——请手工核对格式")
        p.write_text(old_line.sub(new_line, text, count=1), encoding="utf-8", newline="")
        print(f"  {name} -> {version}")

    # CHANGELOG：「## [未发布]」→「## [V] - 日期」；已归段（重跑）幂等跳过
    p = root / "CHANGELOG.md"
    text = p.read_text(encoding="utf-8")
    if f"## [{version}]" in text:
        print(f"  CHANGELOG.md 已存在 [{version}] 段（幂等跳过）")
    elif "## [未发布]" in text:
        p.write_text(text.replace("## [未发布]", f"## [{version}] - {date}", 1),
                     encoding="utf-8", newline="")
        print(f"  CHANGELOG.md [未发布] -> [{version}] - {date}")
    else:
        raise RuntimeError(
            "CHANGELOG 缺 [未发布] 段且无目标版本段——先写发版条目再发版"
        )


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: python scripts/sync_release_docs.py X.Y.Z")
        return 1
    version = sys.argv[1]
    date = datetime.datetime.now().strftime("%Y-%m-%d")
    try:
        sync_docs(ROOT, version, date)
    except RuntimeError as e:
        print(f"[ERROR] {e}")
        return 1
    print(f"  release docs synced to {version} ({date})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
