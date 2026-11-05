#!/usr/bin/env python3
"""Tests for check-versions.py (add-release-auto-update task 2.1)."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check-versions.py"
spec = importlib.util.spec_from_file_location("check_versions", SCRIPT)
check_versions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_versions)


def write_fixture(root: Path, version: str = "0.2.0", **overrides: str) -> None:
    def pick(key: str) -> str:
        return overrides.get(key, version)

    (root / "plugins/opencode-made/src").mkdir(parents=True)
    (root / "plugins/grok-made").mkdir(parents=True)
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "%s"\nedition = "2021"\n'
        '\n[dependencies]\nversion = "9.9.9"\n' % pick("cargo")
    )
    (root / "plugins/opencode-made/package.json").write_text(
        json.dumps({"name": "opencode-made", "version": pick("package")}, indent=2)
    )
    (root / "plugins/opencode-made/package-lock.json").write_text(
        json.dumps(
            {
                "name": "opencode-made",
                "version": pick("lock"),
                "lockfileVersion": 3,
                "packages": {"": {"name": "opencode-made", "version": pick("lock_root")}},
            },
            indent=2,
        )
    )
    (root / "plugins/opencode-made/src/protocol.ts").write_text(
        'export const PROTOCOL = 1;\nexport const PLUGIN_VERSION = "%s";\n' % pick("protocol")
    )
    (root / "plugins/grok-made/plugin.json").write_text(
        json.dumps({"name": "grok-made", "version": pick("grok")}, indent=2)
    )


class CheckVersionsTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.root = Path(self._dir.name)

    def tearDown(self) -> None:
        self._dir.cleanup()

    def run_cli(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root), *args],
            capture_output=True,
            text=True,
        )

    def test_all_sites_equal_passes(self) -> None:
        write_fixture(self.root)
        self.assertEqual(check_versions.check(self.root), [])
        result = self.run_cli()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("0.2.0", result.stdout)

    def test_each_site_mismatch_names_site_and_versions(self) -> None:
        cases = {
            "package": "plugins/opencode-made/package.json",
            "lock": "plugins/opencode-made/package-lock.json (version)",
            "lock_root": 'plugins/opencode-made/package-lock.json (packages[""].version)',
            "protocol": "plugins/opencode-made/src/protocol.ts (PLUGIN_VERSION)",
            "grok": "plugins/grok-made/plugin.json",
        }
        for key, label in cases.items():
            with self.subTest(site=key), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                write_fixture(root, **{key: "0.2.1"})
                problems = check_versions.check(root)
                self.assertEqual(len(problems), 1, problems)
                self.assertIn(label, problems[0])
                self.assertIn("0.2.1", problems[0])
                self.assertIn("workspace version 0.2.0", problems[0])
                result = subprocess.run(
                    [sys.executable, str(SCRIPT), "--root", raw], capture_output=True, text=True
                )
                self.assertEqual(result.returncode, 1)
                self.assertIn(label, result.stderr)

    def test_hand_edit_of_only_the_workspace_fails_for_every_site(self) -> None:
        write_fixture(self.root, version="0.1.0", cargo="0.2.0")
        problems = check_versions.check(self.root)
        self.assertEqual(len(problems), 5, problems)
        for problem in problems:
            self.assertIn("version 0.1.0 differs from the workspace version 0.2.0", problem)

    def test_tag_mismatch_fails_naming_both_versions(self) -> None:
        write_fixture(self.root, version="0.1.0")
        result = self.run_cli("--tag", "v0.2.0")
        self.assertEqual(result.returncode, 1)
        self.assertIn("0.2.0", result.stderr)
        self.assertIn("0.1.0", result.stderr)

    def test_matching_tag_passes(self) -> None:
        write_fixture(self.root)
        result = self.run_cli("--tag", "v0.2.0")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_invalid_semver_fails(self) -> None:
        for bad in ("0.2", "01.2.0", "0.2.0.1", "x.y.z", ""):
            with self.subTest(version=bad), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                write_fixture(root, version=bad)
                problems = check_versions.check(root)
                self.assertTrue(any("not a valid semantic version" in p for p in problems), problems)
        write_fixture(self.root)
        self.assertNotEqual(self.run_cli("--tag", "v0.2").returncode, 0)
        self.assertNotEqual(self.run_cli("--tag", "0.2.0").returncode, 0)

    def test_prerelease_tag_is_refused(self) -> None:
        write_fixture(self.root, version="0.3.0-beta.1")
        result = self.run_cli("--tag", "v0.3.0-beta.1")
        self.assertEqual(result.returncode, 1)
        self.assertIn("prerelease", result.stderr)

    def test_repository_versions_agree(self) -> None:
        self.assertEqual(check_versions.check(HERE.parents[1]), [])


if __name__ == "__main__":
    unittest.main()
