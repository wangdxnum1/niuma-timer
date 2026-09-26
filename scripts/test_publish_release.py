"""Offline updater manifest regression tests; never publish or access credentials."""
import pathlib
import tempfile
import unittest
from unittest import mock
import json
import urllib.parse
import publish_release as release


class ManifestTests(unittest.TestCase):
    def test_notes_generation_replaces_stale_version(self):
        with tempfile.TemporaryDirectory() as d:
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
            with self.subTest(fail_upload=fail_upload), tempfile.TemporaryDirectory() as d:
                root = pathlib.Path(d)
                (root / "niuma-timer-1.4.0-portable.exe").write_bytes(b"portable")
                calls, assets = [], []
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
                        return 200, {}
                    if not any(c[0] == "POST" for c in calls):
                        return 404, {}
                    return 200, {"assets": assets, "html_url": "https://example.test/release"}
                gh = mock.Mock()
                gh.call.side_effect = call
                argv = ["publish_release.py", "--tag", "v1.4.0", "--version", "1.4.0", "--package", d, "--root", d]
                with mock.patch("sys.argv", argv), mock.patch.object(release, "github_token", return_value="dummy"), \
                     mock.patch.object(release, "build_opener"), mock.patch.object(release, "GitHub", return_value=gh):
                    self.assertEqual(release.main(), 1 if fail_upload else 0)
                self.assertEqual(any(c[0] == "PATCH" for c in calls), not fail_upload)

    def test_empty_or_unsigned_installer_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                release.build_latest_json(d, "1.4.0", d, "owner/repo")
            (pathlib.Path(d) / "niuma-timer_1.4.0_x64-setup.exe").write_bytes(b"installer")
            (pathlib.Path(d) / "niuma-timer_1.4.0_x64-setup.exe.sig").write_text("")
            with self.assertRaises(ValueError):
                release.build_latest_json(d, "1.4.0", d, "owner/repo")

    def test_release_gate_precedes_git_and_publication(self):
        batch = (pathlib.Path(__file__).parent.parent / "release.bat").read_text(encoding="utf-8")
        generation = batch.index("--generate-notes-only")
        self.assertLess(generation, batch.index('"%GIT%" add -A'))
        self.assertIn("--draft", batch)
        self.assertIn("--draft=false", batch)

    def test_installer_and_unsigned_portable_have_distinct_entries(self):
        with tempfile.TemporaryDirectory() as d:
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
        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / "niuma-timer_1.4.0_x64-setup.exe.sig").write_text("signature")
            with self.assertRaises(ValueError):
                release.build_latest_json(d, "1.4.0", d, "owner/repo")


if __name__ == "__main__":
    unittest.main()
