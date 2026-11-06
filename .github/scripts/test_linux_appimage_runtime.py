#!/usr/bin/env python3
"""Unit tests for the AppImage extraction guard and embedded settings (task 11.7).

The guard is the image's AppRun: it must run Velopack's AppRun only from a
real directory owned by the user with no group or other permission bits, whose
files all match the image's manifest. An incomplete extraction is removed and
the AppImage is started once more. The AppImage-level runtime test with a
directory owned by another user runs in release.yml (`smoke-installed.py`),
which can use sudo.
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
        self.cache = self.base / ".cache" / "made"
        self.cache.mkdir(parents=True, mode=0o700)

    def tearDown(self) -> None:
        self._dir.cleanup()

    def appdir(self, name: str, mode: int, parent: Path | None = None) -> Path:
        root = (parent or self.cache) / f"appimage-{name}"
        root.mkdir()
        (root / "AppRun").write_text(runtime.GUARD)
        (root / "AppRun").chmod(0o755)
        (root / "AppRun.velopack").write_text('#!/bin/sh\necho "STARTED $APPIMAGE_EXTRACT_AND_RUN" "$@"\n')
        (root / "AppRun.velopack").chmod(0o755)
        (root / "usr").mkdir()
        (root / "usr" / "made-app").write_bytes(b"\x7fELF" + b"x" * 4096)
        runtime.write_manifest(root)
        root.chmod(mode)
        return root

    def launch(self, appdir: Path, **extra: str) -> subprocess.CompletedProcess:
        env = dict(os.environ, APPDIR=str(appdir), APPIMAGE_EXTRACT_AND_RUN="1", HOME=str(self.base))
        env.pop("APPIMAGE", None)
        env.pop("MADE_APPIMAGE_REEXTRACTED", None)
        env.update(extra)
        return subprocess.run(["sh", str(appdir / "AppRun"), "--version"], env=env, capture_output=True, text=True)

    def fake_appimage(self) -> Path:
        image = self.base / "Made.AppImage"
        image.write_text('#!/bin/sh\necho "REEXTRACT $MADE_APPIMAGE_REEXTRACTED" "$@"\n')
        image.chmod(0o755)
        return image

    def test_an_incomplete_extraction_is_removed_and_the_appimage_started_again(self) -> None:
        for damage in ("truncated", "missing", "manifest"):
            with self.subTest(damage=damage):
                root = self.appdir(f"partial-{damage}", 0o700)
                if damage == "truncated":
                    (root / "usr" / "made-app").write_bytes(b"\x7fELF")
                elif damage == "missing":
                    (root / "usr" / "made-app").unlink()
                else:
                    (root / runtime.MANIFEST).unlink()
                result = self.launch(root, APPIMAGE=str(self.fake_appimage()))
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), "REEXTRACT 1 --version")
                self.assertFalse(root.exists(), "the incomplete extraction is removed")

    def test_a_second_incomplete_extraction_stops_with_a_clear_message(self) -> None:
        root = self.appdir("partial-again", 0o700)
        (root / "usr" / "made-app").unlink()
        result = self.launch(root, APPIMAGE=str(self.fake_appimage()), MADE_APPIMAGE_REEXTRACTED="1")
        self.assertEqual(result.returncode, 1)
        self.assertIn("was incomplete and has been removed; start Made again", result.stderr)
        self.assertNotIn("REEXTRACT", result.stdout)
        self.assertFalse(root.exists())

    def test_an_incomplete_extraction_without_an_appimage_stops_with_a_clear_message(self) -> None:
        root = self.appdir("partial-no-image", 0o700)
        (root / "usr" / "made-app").write_bytes(b"")
        result = self.launch(root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("start Made again", result.stderr)
        self.assertNotIn("STARTED", result.stdout)

    def test_the_parent_directory_is_made_private(self) -> None:
        parent = self.cache
        parent.chmod(0o775)
        root = parent / "appimage"
        root.mkdir()
        (root / "AppRun").write_text(runtime.GUARD)
        (root / "AppRun.velopack").write_text('#!/bin/sh\necho STARTED\n')
        (root / "AppRun.velopack").chmod(0o755)
        runtime.write_manifest(root)
        root.chmod(0o700)
        result = self.launch(root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(parent.stat().st_mode & 0o777, 0o700)

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
        env = dict(os.environ, APPDIR=str(link), HOME=str(self.base))
        result = subprocess.run(["sh", str(link / "AppRun")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertNotIn("STARTED", result.stdout)

    def test_a_missing_directory_is_refused(self) -> None:
        real = self.appdir("gone", 0o700)
        env = dict(os.environ, APPDIR=str(self.base / "absent"), HOME=str(self.base))
        result = subprocess.run(["sh", str(real / "AppRun")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)


    def test_appdir_naming_another_directory_is_refused_and_nothing_is_deleted(self) -> None:
        victim = self.appdir("victim", 0o700)
        (victim / "usr" / "made-app").unlink()
        own = self.appdir("own", 0o700)
        env = dict(os.environ, APPDIR=str(victim), HOME=str(self.base), APPIMAGE=str(self.fake_appimage()))
        result = subprocess.run(["sh", str(own / "AppRun")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("not this AppImage's extraction directory", result.stderr)
        self.assertTrue(victim.exists(), "a directory named only by APPDIR is never deleted")
        self.assertTrue(own.exists())

    def test_an_incomplete_directory_outside_the_cache_is_never_deleted(self) -> None:
        elsewhere = self.base / "elsewhere"
        elsewhere.mkdir(mode=0o700)
        root = self.appdir("outside", 0o700, parent=elsewhere)
        (root / "usr" / "made-app").unlink()
        result = self.launch(root, APPIMAGE=str(self.fake_appimage()))
        self.assertEqual(result.returncode, 1)
        self.assertIn("remove it and start Made again", result.stderr)
        self.assertTrue(root.exists())
        self.assertNotIn("REEXTRACT", result.stdout)

    def test_an_incomplete_cache_directory_not_named_appimage_is_never_deleted(self) -> None:
        root = self.cache / "other"
        root.mkdir()
        (root / "AppRun").write_text(runtime.GUARD)
        (root / "AppRun.velopack").write_text("#!/bin/sh\necho STARTED\n")
        runtime.write_manifest(root)
        (root / "AppRun.velopack").write_text("changed")
        root.chmod(0o700)
        result = self.launch(root, APPIMAGE=str(self.fake_appimage()))
        self.assertEqual(result.returncode, 1)
        self.assertTrue(root.exists())

class EmbeddedSettingsTest(unittest.TestCase):
    def test_the_extraction_target_is_per_user_per_version_and_per_build(self) -> None:
        envs = runtime.embedded_envs("0.2.0", "0123456789abcdef").splitlines()
        self.assertEqual(envs[0], "APPIMAGE_EXTRACT_AND_RUN=1")
        self.assertEqual(envs[1], "APPIMAGE_TARGET_DIR=${HOME}/.cache/made/appimage-0.2.0-0123456789abcdef")

    def test_two_builds_of_one_version_get_different_targets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            first, second = Path(tmp) / "a", Path(tmp) / "b"
            first.write_bytes(b"image one")
            second.write_bytes(b"image two")
            self.assertNotEqual(runtime.content_hash(first), runtime.content_hash(second))
            self.assertRegex(runtime.content_hash(first), r"^[0-9a-f]{16}$")


class ManifestTest(unittest.TestCase):
    def test_every_file_is_listed_for_sha256sum(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "usr").mkdir()
            (root / "usr" / "bin").write_bytes(b"binary")
            (root / "AppRun").write_text("guard")
            runtime.write_manifest(root)
            check = subprocess.run(["sha256sum", "--strict", "-c", runtime.MANIFEST], cwd=root, capture_output=True, text=True)
            self.assertEqual(check.returncode, 0, check.stdout + check.stderr)
            self.assertEqual(len((root / runtime.MANIFEST).read_text().splitlines()), 2)

    def test_a_symlink_in_the_image_fails_the_build(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "file").write_text("x")
            (root / "link").symlink_to("file")
            with self.assertRaises(SystemExit):
                runtime.write_manifest(root)


if __name__ == "__main__":
    unittest.main()
