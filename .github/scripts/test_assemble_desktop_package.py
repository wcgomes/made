#!/usr/bin/env python3
"""Tests for assemble-desktop-package.py with fake build outputs (task 8.7)."""

from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("assemble_desktop_package", HERE / "assemble-desktop-package.py")
assemble = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assemble)

DESKTOP_TARGET = "x86_64-unknown-linux-gnu"
CLAUDE_FILES = [
    "plugins/claude-code-made/.claude-plugin/plugin.json",
    "plugins/claude-code-made/README.md",
    "plugins/claude-code-made/hooks/hooks.json",
    "plugins/claude-code-made/bin/made-claude-status",
]


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def linkage(target: str) -> dict:
    if target.endswith("-unknown-linux-gnu"):
        return {"kind": "glibc", "system": "2.31", "max_symbol": "2.31"}
    return {"kind": "macos", "deployment_target": "12.0", "minos": "12.0"}


class AssembleDesktopPackageTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.tmp = Path(self._dir.name)
        self.stage = self.tmp / "stage"
        self.stage.mkdir()
        for name in ["made-app", "plugins/opencode-made/package.json", "plugins/grok-made/plugin.json"] + CLAUDE_FILES:
            path = self.stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(f"fixture {name}".encode())
        for name in ("made-app", "plugins/claude-code-made/bin/made-claude-status"):
            (self.stage / name).chmod(0o755)

        self.companions = self.tmp / "companions"
        for target in assemble.TARGETS:
            root = self.companions / target
            records = []
            for name in assemble.FILES:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                data = f"{target} {name}".encode()
                path.write_bytes(data)
                records.append({"name": name, "size": len(data), "sha256": digest(data)})
            manifest = {
                "schema": 1,
                "target": target,
                "profile": "release",
                "linkage": linkage(target),
                "files": records,
            }
            (root / "manifest.json").write_text(json.dumps(manifest))
            (root / "SHA256SUMS").write_text("".join(f"{r['sha256']}  {r['name']}\n" for r in records))

        self.askpass_dir = self.tmp / "askpass"
        self.askpass_dir.mkdir()
        self.askpass = self.askpass_dir / "made-askpass"
        self.askpass.write_bytes(b"fake askpass")
        (self.askpass_dir / "manifest.json").write_text(
            json.dumps(
                {
                    "schema": 1,
                    "kind": "askpass",
                    "name": "made-askpass",
                    "target": DESKTOP_TARGET,
                    "size": len(b"fake askpass"),
                    "sha256": digest(b"fake askpass"),
                }
            )
        )

    def tearDown(self) -> None:
        self._dir.cleanup()

    def run_assembly(self) -> None:
        argv = [
            "assemble-desktop-package.py",
            "--stage", str(self.stage),
            "--companions", str(self.companions),
            "--askpass", str(self.askpass),
            "--askpass-manifest", str(self.askpass_dir / "manifest.json"),
            "--desktop-target", DESKTOP_TARGET,
        ]
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
            assemble.main()

    def assert_refuses(self, needle: str) -> None:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit):
            self.run_assembly()
        self.assertIn(needle, stderr.getvalue())

    def test_complete_stage_assembles(self) -> None:
        self.run_assembly()
        for name in CLAUDE_FILES:
            self.assertTrue((self.stage / name).is_file(), name)
        self.assertTrue((self.stage / "remote/manifest.json").is_file())

    def test_missing_claude_plugin_files_fail(self) -> None:
        for name in CLAUDE_FILES:
            with self.subTest(missing=name):
                path = self.stage / name
                data = path.read_bytes()
                mode = path.stat().st_mode
                path.unlink()
                self.assert_refuses("claude-code-made")
                path.write_bytes(data)
                path.chmod(mode)
                self.assertFalse((self.stage / "remote").exists())

    def test_empty_claude_hook_fails(self) -> None:
        (self.stage / CLAUDE_FILES[-1]).write_bytes(b"")
        self.assert_refuses("claude-code-made")

    def test_claude_hook_without_executable_bit_fails(self) -> None:
        if os.name == "nt":
            self.skipTest("no executable bit on Windows")
        hook = self.stage / CLAUDE_FILES[-1]
        hook.chmod(hook.stat().st_mode & ~(stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))
        self.assert_refuses("claude-code-made")

    def test_fails_when_assembly_removes_the_claude_plugin(self) -> None:
        real = assemble.rehash_installed

        def removing(stage: Path, target: str, expected: list) -> None:
            real(stage, target, expected)
            for name in CLAUDE_FILES:
                (stage / name).unlink(missing_ok=True)

        with mock.patch.object(assemble, "rehash_installed", removing):
            self.assert_refuses("claude-code-made")


if __name__ == "__main__":
    unittest.main()
