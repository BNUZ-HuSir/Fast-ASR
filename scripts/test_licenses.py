"""Regression checks for retained notices; test files stay inside .build/."""
import json
from pathlib import Path
import shutil
import time
import unittest
from unittest.mock import patch

import check_licenses
from release_common import ROOT, digest


class NoticeIntegrityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = ROOT / ".build" / ("license-tests-" + str(time.time_ns()))
        cls.workspace.mkdir(parents=True)

    def setUp(self):
        self.directory = self.workspace / self._testMethodName
        self.source = self.directory / "source"
        for source, _ in check_licenses.license_materials():
            target = self.source / source.relative_to(ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        for name in ("xasr-manifest.json", "whisper-manifest.json"):
            target = self.source / "assets" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / "assets" / name, target)

    def test_unchanged_snapshot_passes(self):
        self.assertEqual(len(check_licenses.license_materials(self.source)), 16)

    def test_changed_or_missing_license_fails(self):
        target = self.source / "vendor/licenses/X-ASR-LICENSE"
        target.rename(target.with_suffix(".preserved"))
        with self.assertRaisesRegex(RuntimeError, "Missing or altered"):
            check_licenses.license_materials(self.source)
        target.write_text("different license text", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "Missing or altered"):
            check_licenses.license_materials(self.source)

    def test_model_update_requires_review(self):
        target = self.source / "assets/xasr-manifest.json"
        target.write_bytes(target.read_bytes() + b"\n")
        with self.assertRaisesRegex(RuntimeError, "review licenses"):
            check_licenses.license_materials(self.source)

    def test_unlisted_notice_and_unsafe_path_fail(self):
        manifest = self.source / "vendor/licenses/SOURCES.json"
        value = json.loads(manifest.read_text(encoding="utf-8"))
        value["files"][0]["name"] = "../outside"
        manifest.write_text(json.dumps(value), encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "Unsafe"):
            check_licenses.license_materials(self.source)
        shutil.copy2(ROOT / "vendor/licenses/SOURCES.json", manifest)
        (manifest.parent / "extra-NOTICE").write_text("unreviewed", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "inventory does not match"):
            check_licenses.license_materials(self.source)

    def test_install_verify_and_preserve_existing_files(self):
        destination = self.directory / "download-target"
        with patch("check_licenses.license_materials",
                   side_effect=lambda: check_licenses_original(self.source)):
            with self.assertRaisesRegex(RuntimeError, "Missing or outdated"):
                check_licenses.install_license_materials(destination, verify_only=True)
            self.assertFalse(destination.exists())
            check_licenses.install_license_materials(destination)
            check_licenses.install_license_materials(destination, verify_only=True)
            target = destination / "licenses/X-ASR-LICENSE"
            target.write_text("old local note", encoding="utf-8")
            before = {p for p in destination.rglob("*")}
            with self.assertRaisesRegex(RuntimeError, "Missing or outdated"):
                check_licenses.install_license_materials(destination, verify_only=True)
            self.assertEqual(before, {p for p in destination.rglob("*")})
            check_licenses.install_license_materials(destination)
            backups = list(target.parent.glob(target.name + ".damaged-*"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_text(encoding="utf-8"), "old local note")
            self.assertEqual(digest(target), digest(self.source / "vendor/licenses/X-ASR-LICENSE"))


check_licenses_original = check_licenses.license_materials

if __name__ == "__main__":
    unittest.main()
