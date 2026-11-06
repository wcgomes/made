#!/usr/bin/env python3
"""Tests for release-assets.py (add-release-auto-update task 7.1)."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("release_assets", HERE / "release-assets.py")
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)

VELOPACK_OUTPUT = {
    "win-x64": ["Made-win-x64-Setup.exe"],
    "osx-arm64": ["Made-osx-arm64-Setup.pkg", "Made-osx-arm64-Portable.zip"],
    "osx-x64": ["Made-osx-x64-Setup.pkg", "Made-osx-x64-Portable.zip"],
    "linux-x64": ["Made-linux-x64.AppImage"],
}
EXPECTED = {
    "win-x64": ["made-0.2.0-windows-x64-setup.exe"],
    "osx-arm64": ["made-0.2.0-macos-arm64.pkg", "made-0.2.0-macos-arm64.zip"],
    "osx-x64": ["made-0.2.0-macos-x64.pkg", "made-0.2.0-macos-x64.zip"],
    "linux-x64": ["made-0.2.0-linux-x64.AppImage"],
}


def velopack_out(root: Path, channel: str, version: str = "0.2.0") -> Path:
    out = root / f"out-{channel}"
    out.mkdir()
    package = f"Made-{version}-{channel}-full.nupkg"
    (out / package).write_bytes(f"package {channel}".encode())
    feed = {"Assets": [{"PackageId": "Made", "Version": version, "Type": "Full", "FileName": package}]}
    (out / f"releases.{channel}.json").write_text(json.dumps(feed))
    (out / f"RELEASES-{channel}").write_text("legacy")
    (out / f"assets.{channel}.json").write_text("[]")
    for name in VELOPACK_OUTPUT[channel]:
        (out / name).write_bytes(f"download {name}".encode())
    return out


class ReleaseAssetsTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.root = Path(self._dir.name)

    def tearDown(self) -> None:
        self._dir.cleanup()

    def test_downloads_are_renamed_per_target_and_feed_files_are_unchanged(self) -> None:
        dest = self.root / "release"
        for channel in VELOPACK_OUTPUT:
            with self.subTest(channel=channel):
                out = velopack_out(self.root, channel)
                written = assets.finish(out, "0.2.0", channel, dest)
                feed_files = {f"releases.{channel}.json", f"Made-0.2.0-{channel}-full.nupkg"}
                self.assertEqual(set(written), feed_files | set(EXPECTED[channel]))
                for name in feed_files:
                    self.assertEqual((dest / name).read_bytes(), (out / name).read_bytes())
                self.assertFalse((dest / f"RELEASES-{channel}").exists())
                self.assertFalse((dest / f"assets.{channel}.json").exists())
                for name in EXPECTED[channel]:
                    self.assertIn("0.2.0", name)
        # The four channels never collide in one release folder.
        self.assertEqual(len(list(dest.iterdir())), sum(2 + len(v) for v in EXPECTED.values()))

    def test_sums_list_every_other_asset_with_a_correct_digest(self) -> None:
        dest = self.root / "release"
        for channel in VELOPACK_OUTPUT:
            assets.finish(velopack_out(self.root, channel), "0.2.0", channel, dest)
        sums = assets.write_sums(dest)
        lines = sums.read_text().splitlines()
        names = sorted(p.name for p in dest.iterdir() if p.name != "SHA256SUMS")
        self.assertEqual([line.split("  ", 1)[1] for line in lines], names)
        for line in lines:
            digest, name = line.split("  ", 1)
            self.assertEqual(digest, hashlib.sha256((dest / name).read_bytes()).hexdigest())
        assets.check_sums(dest)
        (dest / names[0]).write_bytes(b"corrupt")
        with self.assertRaises(SystemExit):
            assets.check_sums(dest)

    def test_unknown_output_and_missing_downloads_fail(self) -> None:
        out = velopack_out(self.root, "win-x64")
        (out / "Made-win-x64-Portable.zip").write_bytes(b"not shipped on Windows")
        with self.assertRaises(SystemExit):
            assets.finish(out, "0.2.0", "win-x64", self.root / "a")
        out = velopack_out(self.root, "osx-x64")
        (out / "Made-osx-x64-Portable.zip").unlink()
        with self.assertRaises(SystemExit):
            assets.finish(out, "0.2.0", "osx-x64", self.root / "b")

    def test_a_feed_entry_with_another_version_fails(self) -> None:
        out = velopack_out(self.root, "linux-x64", version="0.1.9")
        with self.assertRaises(SystemExit):
            assets.finish(out, "0.2.0", "linux-x64", self.root / "c")


if __name__ == "__main__":
    unittest.main()
