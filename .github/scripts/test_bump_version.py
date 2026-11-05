#!/usr/bin/env python3
"""Tests for scripts/bump-version.sh on a fixture copy of the repository (task 2.4)."""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SOURCE_SITES = {
    "Cargo.toml",
    "Cargo.lock",
    "plugins/opencode-made/package.json",
    "plugins/opencode-made/package-lock.json",
    "plugins/opencode-made/src/protocol.ts",
    "plugins/opencode-made/README.md",
    "plugins/grok-made/plugin.json",
}


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=root, check=True, capture_output=True, text=True
    ).stdout


def copy_repository(destination: Path) -> None:
    files = git(REPO, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0")
    for name in filter(None, files):
        source = REPO / name
        if not source.is_file():
            continue
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    env = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
    for args in (["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "fixture"]):
        subprocess.run(["git", *args], cwd=destination, check=True, env={**os.environ, **env},
                       capture_output=True)


def workspace_version() -> str:
    spec = importlib.util.spec_from_file_location("check_versions", REPO / ".github/scripts/check-versions.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.workspace_version(REPO)


CURRENT = workspace_version()
_major, _minor, _ = (int(part) for part in CURRENT.split("."))
# The next minor version, so the fixture always moves forward from the repository.
TARGET = f"{_major}.{_minor + 1}.0"


class BumpVersionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls._dir = tempfile.TemporaryDirectory()
        cls.root = Path(cls._dir.name) / "repo"
        copy_repository(cls.root)
        cls.before = (cls.root / "plugins/opencode-made/tests/lifecycle.test.ts").read_text()
        cls.result = subprocess.run(
            ["bash", "scripts/bump-version.sh", TARGET], cwd=cls.root, capture_output=True, text=True
        )

    @classmethod
    def tearDownClass(cls) -> None:
        cls._dir.cleanup()

    def changed(self) -> set[str]:
        return {line[3:] for line in git(self.root, "status", "--porcelain").splitlines()}

    def test_bump_succeeds_and_the_version_check_passes(self) -> None:
        self.assertEqual(self.result.returncode, 0, self.result.stderr + self.result.stdout)
        check = subprocess.run(
            [sys.executable, ".github/scripts/check-versions.py"], cwd=self.root, capture_output=True, text=True
        )
        self.assertEqual(check.returncode, 0, check.stderr)
        self.assertIn(TARGET, check.stdout)

    def test_rewrites_exactly_the_source_sites(self) -> None:
        self.assertEqual(self.changed(), SOURCE_SITES)
        readme = (self.root / "plugins/opencode-made/README.md").read_text()
        self.assertIn(f'"appVersion": "{TARGET}"', readme)
        protocol = (self.root / "plugins/opencode-made/src/protocol.ts").read_text()
        self.assertIn(f'export const PLUGIN_VERSION = "{TARGET}";', protocol)

    def test_test_fixtures_are_untouched(self) -> None:
        after = (self.root / "plugins/opencode-made/tests/lifecycle.test.ts").read_text()
        self.assertEqual(after, self.before)
        self.assertIn("0.1.0-test", after)

    def test_cargo_lock_changes_only_workspace_member_versions(self) -> None:
        diff = git(self.root, "diff", "-U0", "--", "Cargo.lock").splitlines()
        edits = [line for line in diff if line[:1] in "+-" and not line.startswith(("+++", "---"))]
        self.assertTrue(edits)
        for line in edits:
            self.assertIn(line, (f'-version = "{CURRENT}"', f'+version = "{TARGET}"'))

    def test_npm_lock_changed_through_npm_and_npm_ci_passes(self) -> None:
        diff = git(self.root, "diff", "-U0", "--", "plugins/opencode-made/package-lock.json")
        self.assertIn(f'+  "version": "{TARGET}"', diff)
        self.assertIn(f'+      "version": "{TARGET}"', diff)
        if shutil.which("npm") is None:
            self.skipTest("npm is not installed")
        ci = subprocess.run(
            ["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund"],
            cwd=self.root / "plugins/opencode-made", capture_output=True, text=True,
        )
        self.assertEqual(ci.returncode, 0, ci.stderr)

    def test_invalid_and_prerelease_versions_are_refused(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw) / "repo"
            copy_repository(root)
            for bad in ("0.2", "v0.2.0", "0.2.0-beta.1", "0.2.0+build.1", "01.2.0", ""):
                with self.subTest(version=bad):
                    result = subprocess.run(
                        ["bash", "scripts/bump-version.sh", bad], cwd=root, capture_output=True, text=True
                    )
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertEqual(git(root, "status", "--porcelain"), "")


if __name__ == "__main__":
    unittest.main()
