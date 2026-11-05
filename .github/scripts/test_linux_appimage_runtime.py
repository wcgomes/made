#!/usr/bin/env python3
"""Unit tests for the AppImage extraction guard and embedded settings (task 11.7).

The guard is the image's AppRun: it must run Velopack's AppRun only from a
real directory owned by the user with no group or other permission bits. The
AppImage-level runtime test with a directory owned by another user runs in
release.yml (`smoke-installed.py`), which can use sudo.
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("linux_appimage_runtime", HERE / "linux-appimage-runtime.py")
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


@unittest.skipUnless(os.name == "posix", "the guard is a POSIX shell script")
class ExtractionGuardTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.base = Path(self._dir.name)

    def tearDown(self) -> None:
        self._dir.cleanup()

    def appdir(self, name: str, mode: int) -> Path:
        root = self.base / name
        root.mkdir()
        (root / "AppRun").write_text(runtime.GUARD)
        (root / "AppRun").chmod(0o755)
        (root / "AppRun.velopack").write_text('#!/bin/sh\necho "STARTED $APPIMAGE_EXTRACT_AND_RUN" "$@"\n')
        (root / "AppRun.velopack").chmod(0o755)
        root.chmod(mode)
        return root

    def launch(self, appdir: Path) -> subprocess.CompletedProcess:
        env = dict(os.environ, APPDIR=str(appdir), APPIMAGE_EXTRACT_AND_RUN="1")
        return subprocess.run(["sh", str(appdir / "AppRun"), "--version"], env=env, capture_output=True, text=True)

    def test_a_private_directory_starts_velopack_apprun_without_leaking_the_extract_flag(self) -> None:
        result = self.launch(self.appdir("private", 0o700))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "STARTED  --version")

    def test_group_or_other_bits_are_refused(self) -> None:
        for mode in (0o750, 0o705, 0o777, 0o755):
            with self.subTest(mode=oct(mode)):
                result = self.launch(self.appdir(f"mode-{mode:o}", mode))
                self.assertEqual(result.returncode, 1)
                self.assertIn("not a private directory owned by this user", result.stderr)
                self.assertNotIn("STARTED", result.stdout)

    def test_a_symlinked_directory_is_refused(self) -> None:
        real = self.appdir("real", 0o700)
        link = self.base / "link"
        link.symlink_to(real)
        env = dict(os.environ, APPDIR=str(link))
        result = subprocess.run(["sh", str(link / "AppRun")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("STARTED", result.stdout)

    def test_a_missing_directory_is_refused(self) -> None:
        real = self.appdir("gone", 0o700)
        env = dict(os.environ, APPDIR=str(self.base / "absent"))
        result = subprocess.run(["sh", str(real / "AppRun")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)


class EmbeddedSettingsTest(unittest.TestCase):
    def test_the_extraction_target_is_per_user_and_per_version(self) -> None:
        envs = runtime.embedded_envs("0.2.0").splitlines()
        self.assertEqual(envs[0], "APPIMAGE_EXTRACT_AND_RUN=1")
        self.assertEqual(envs[1], "APPIMAGE_TARGET_DIR=${HOME}/.cache/made/appimage-0.2.0")


if __name__ == "__main__":
    unittest.main()
