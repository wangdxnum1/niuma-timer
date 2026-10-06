#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""sync_release_docs.py 的离线单测：五处同步 / 幂等 / fail-closed，不碰真实仓库。"""

import os
import pathlib
import sys
import tempfile
import unittest
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sync_release_docs import sync_docs  # noqa: E402

README_EN = "> Current version **1.0.0** (2026-01-01) · [Changelog](./CHANGELOG.md)\n"
README_ZH = "> 当前版本 **1.0.0**（2026-01-01）· [更新日志](./CHANGELOG.md)\n"
CHANGELOG = "# 更新日志\n\n## [未发布]\n\n### 新增\n\n- 条目\n\n## [1.0.0] - 2026-01-01\n"


def make_root():
    """临时仓库根（os.makedirs 而非 TemporaryDirectory：受限令牌下 0700 ACL 不可写）。"""
    d = os.path.join(tempfile.gettempdir(), "niuma-syncdocs-" + uuid.uuid4().hex)
    os.makedirs(d)
    root = pathlib.Path(d)
    (root / "README.md").write_text(README_EN, encoding="utf-8", newline="")
    (root / "README.zh-CN.md").write_text(README_ZH, encoding="utf-8", newline="")
    (root / "CHANGELOG.md").write_text(CHANGELOG, encoding="utf-8", newline="")
    return root


class SyncDocsTests(unittest.TestCase):
    def test_bumps_all_docs(self):
        root = make_root()
        sync_docs(root, "2.0.0", "2026-10-06")
        en = (root / "README.md").read_text(encoding="utf-8")
        zh = (root / "README.zh-CN.md").read_text(encoding="utf-8")
        log = (root / "CHANGELOG.md").read_text(encoding="utf-8")
        self.assertIn("**2.0.0** (2026-10-06)", en)
        self.assertIn("**2.0.0**（2026-10-06）", zh)
        self.assertIn("## [2.0.0] - 2026-10-06", log)
        self.assertNotIn("## [未发布]", log)

    def test_rerun_is_idempotent(self):
        root = make_root()
        sync_docs(root, "2.0.0", "2026-10-06")
        sync_docs(root, "2.0.0", "2026-10-06")  # abort 后重跑：不得报错、不得重复归段
        log = (root / "CHANGELOG.md").read_text(encoding="utf-8")
        self.assertEqual(log.count("## [2.0.0]"), 1)

    def test_missing_unpublished_section_fails_closed(self):
        root = make_root()
        (root / "CHANGELOG.md").write_text(
            "# 更新日志\n\n## [1.0.0] - 2026-01-01\n", encoding="utf-8", newline=""
        )
        with self.assertRaises(RuntimeError):
            sync_docs(root, "2.0.0", "2026-10-06")

    def test_bad_version_rejected(self):
        root = make_root()
        with self.assertRaises(RuntimeError):
            sync_docs(root, "abc", "2026-10-06")


if __name__ == "__main__":
    unittest.main()
