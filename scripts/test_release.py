"""Release contract tests; no network, user credentials, commits, or tags."""

import datetime
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import version
import homebrew


class VersionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="stt-release-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "scripts").mkdir()
        for name in ("version.py", "headatever.sh"):
            shutil.copy2(version.ROOT / "scripts" / name, self.root / "scripts" / name)
        (self.root / "VERSION").write_text("1.260818.0\n")
        (self.root / "Cargo.toml").write_text('[package]\nname = "stt-cli"\nversion = "0.1.0"\n')
        (self.root / "Cargo.lock").write_text(
            'version = 4\n[[package]]\nname = "other"\nversion = "0.1.0"\n'
            '[[package]]\nname = "stt-cli"\nversion = "0.1.0"\n'
        )

    def headatever(self, *args):
        return subprocess.run(
            ["bash", "scripts/headatever.sh", *args], cwd=self.root,
            capture_output=True, text=True,
        )

    def test_sync_updates_only_the_application_package(self):
        version.sync(self.root)
        self.assertEqual(version.check(self.root, tag="v1.260818.0"), "1.260818.0")
        self.assertIn('name = "other"\nversion = "0.1.0"', (self.root / "Cargo.lock").read_text())
        with self.assertRaises(ValueError):
            version.check(self.root, tag="vv1.260818.0")

    def test_invalid_versions_and_impossible_dates_are_rejected(self):
        for invalid in ("v1.260818.0", "01.260818.0", "1.260818.01", "1.260231.0", "1.261301.0"):
            with self.subTest(version=invalid), self.assertRaises(ValueError):
                version.validate(invalid)

    def test_legacy_prefix_is_normalized_by_headatever(self):
        (self.root / "VERSION").write_text("v1.260818.0")
        result = self.headatever("set", "1.260818.0", "--no-git")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(version.check(self.root), "1.260818.0")

    def test_patch_dry_run_does_not_change_files(self):
        before = {name: (self.root / name).read_bytes() for name in ("VERSION", "Cargo.toml", "Cargo.lock")}
        result = self.headatever("patch", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(before, {name: (self.root / name).read_bytes() for name in before})

    def test_same_day_patch_syncs_all_version_consumers(self):
        today = datetime.date.today().strftime("%y%m%d")
        (self.root / "VERSION").write_text(f"1.{today}.9\n")
        result = self.headatever("patch", "--no-git")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(version.check(self.root), f"1.{today}.10")

    def test_invalid_set_leaves_version_untouched(self):
        before = (self.root / "VERSION").read_bytes()
        result = self.headatever("set", "1.260231.0", "--no-git")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.root / "VERSION").read_bytes(), before)


class HomebrewTests(unittest.TestCase):
    def test_formula_pins_both_tag_and_commit_and_keeps_head_available(self):
        revision = "a" * 40
        formula = homebrew.render("v1.260906.0", revision)
        self.assertIn('tag:      "v1.260906.0"', formula)
        self.assertIn(f'revision: "{revision}"', formula)
        self.assertIn('head "https://github.com/channprj/stt-cli.git"', formula)
        self.assertNotIn("TO_BE_REPLACED", formula)
        self.assertNotIn("releases/download", formula)

    def test_untrusted_tag_and_revision_cannot_be_inserted_as_ruby(self):
        for tag, revision in [
            ('v1.260906.0"; system("bad")', "a" * 40),
            ("1.260906.0", "a" * 40),
            ("v1.260906.0", "main"),
            ("v1.260906.0", 'a"; system("bad")'),
        ]:
            with self.subTest(tag=tag, revision=revision), self.assertRaises(ValueError):
                homebrew.render(tag, revision)


if __name__ == "__main__":
    unittest.main()
