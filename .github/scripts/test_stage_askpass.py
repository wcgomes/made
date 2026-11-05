#!/usr/bin/env python3
"""Tests for stage-askpass.sh with a stub cargo on PATH (task 3.11)."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
TARGET = "x86_64-unknown-linux-gnu"

STUB = """#!/usr/bin/env bash
# Records its arguments and writes a fake askpass where cargo would.
printf '%s\\n' "$*" >> "$STUB_LOG"
profile=debug
for arg in "$@"; do [ "$arg" = "--release" ] && profile=release; done
mkdir -p "target/{target}/$profile"
printf 'fake askpass %s' "$profile" > "target/{target}/$profile/made-askpass"
"""


class StageAskpassTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        tmp = Path(self._dir.name)
        # A scratch repository root keeps the stub's target/ and dist/ output
        # out of the real checkout.
        self.root = tmp / "repo"
        (self.root / ".github/scripts").mkdir(parents=True)
        for name in ("stage-askpass.sh", "hash.sh"):
            shutil.copy2(HERE / name, self.root / ".github/scripts" / name)
        bin_dir = tmp / "bin"
        bin_dir.mkdir()
        cargo = bin_dir / "cargo"
        cargo.write_text(STUB.replace("{target}", TARGET))
        cargo.chmod(0o755)
        self.log = tmp / "cargo.log"
        self.env = {**os.environ, "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                    "STUB_LOG": str(self.log), "TARGET": TARGET}
        self.env.pop("PROFILE", None)

    def tearDown(self) -> None:
        self._dir.cleanup()

    def stage(self, **env: str) -> subprocess.CompletedProcess:
        return subprocess.run(["bash", str(self.root / ".github/scripts/stage-askpass.sh")],
                              env={**self.env, **env}, capture_output=True, text=True)

    def staged(self) -> tuple[str, dict]:
        out = self.root / "dist" / f"ssh-askpass-{TARGET}"
        return (out / "made-askpass").read_text(), json.loads((out / "manifest.json").read_text())

    def test_default_stays_debug(self) -> None:
        result = self.stage()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("--release", self.log.read_text())
        content, manifest = self.staged()
        self.assertEqual(content, "fake askpass debug")
        self.assertEqual(manifest["target"], TARGET)

    def test_release_profile_builds_release(self) -> None:
        result = self.stage(PROFILE="release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--release", self.log.read_text())
        content, manifest = self.staged()
        self.assertEqual(content, "fake askpass release")
        self.assertEqual(manifest["size"], len(content))

    def test_unknown_profile_is_refused(self) -> None:
        self.assertNotEqual(self.stage(PROFILE="fast").returncode, 0)


if __name__ == "__main__":
    unittest.main()
