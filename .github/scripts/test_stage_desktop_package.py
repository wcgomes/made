#!/usr/bin/env python3
"""Tests for stage-desktop-package.py with fake build outputs (task 3.1)."""

from __future__ import annotations

import importlib.util
import json
import os
import re
import stat
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
spec = importlib.util.spec_from_file_location("stage_desktop_package", HERE / "stage-desktop-package.py")
stage_desktop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage_desktop)

GROK_FILES = ["README.md", "hooks/bin/made-grok-status", "hooks/hooks.json", "plugin.json"]
CLAUDE_FILES = [".claude-plugin/plugin.json", "README.md", "bin/made-claude-status", "hooks/hooks.json"]


def workspace_version() -> str:
    return stage_desktop.workspace_version(REPO)


class StageDesktopPackageTest(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.tmp = Path(self._dir.name)
        self.app = self.tmp / "build" / "made-app"
        self.hook = self.tmp / "build" / "made-grok-status"
        self.claude_hook = self.tmp / "build" / "made-claude-status"
        self.app.parent.mkdir()
        self.app.write_bytes(b"\x7fELF fake app")
        self.hook.write_bytes(b"\x7fELF fake hook")
        self.claude_hook.write_bytes(b"\x7fELF fake claude hook")
        # A plugin source whose version differs proves the stamped version.
        source = self.tmp / "grok-source"
        (source / "hooks").mkdir(parents=True)
        (source / "plugin.json").write_text('{\n  "name": "grok-made",\n  "version": "9.9.9"\n}\n')
        (source / "README.md").write_text("fixture\n")
        (source / "hooks/hooks.json").write_text('{"command": "bin/made-grok-status"}\n')
        (source / "hooks/stale.py").write_text("print('never shipped')\n")
        os.environ["MADE_GROK_PLUGIN_SOURCE"] = str(source)
        self.claude_source = self.tmp / "claude-source"
        (self.claude_source / ".claude-plugin").mkdir(parents=True)
        (self.claude_source / "hooks").mkdir()
        (self.claude_source / ".claude-plugin/plugin.json").write_text(
            '{\n  "name": "claude-code-made",\n  "version": "9.9.9"\n}\n'
        )
        (self.claude_source / "README.md").write_text("fixture\n")
        (self.claude_source / "hooks/hooks.json").write_text('{"command": "bin/made-claude-status"}\n')
        os.environ["MADE_CLAUDE_PLUGIN_SOURCE"] = str(self.claude_source)
        node_modules = REPO / "plugins/opencode-made/node_modules"
        self.made_node_modules = not node_modules.exists()
        if self.made_node_modules:
            (node_modules / "fake").mkdir(parents=True)

    def tearDown(self) -> None:
        os.environ.pop("MADE_GROK_PLUGIN_SOURCE", None)
        os.environ.pop("MADE_CLAUDE_PLUGIN_SOURCE", None)
        if self.made_node_modules:
            import shutil

            shutil.rmtree(REPO / "plugins/opencode-made/node_modules")
        self._dir.cleanup()

    def staged(self, profile: str) -> Path:
        out = self.tmp / f"stage-{profile}"
        stage_desktop.stage(
            stage_desktop.parse(
                ["--profile", profile, "--target", "x86_64-unknown-linux-gnu", "--app", str(self.app),
                 "--hook", str(self.hook), "--claude-hook", str(self.claude_hook), "--stage", str(out)]
            )
        )
        return out

    def test_both_profiles_stage_the_complete_tree(self) -> None:
        for profile in ("debug", "release"):
            with self.subTest(profile=profile):
                out = self.staged(profile)
                self.assertEqual((out / "made-app").read_bytes(), self.app.read_bytes())
                self.assertTrue((out / "plugins/opencode-made/package.json").is_file())
                self.assertFalse((out / "plugins/opencode-made/node_modules").exists())
                self.assertTrue((out / "LICENSE").is_file())
                self.assertEqual((out / "LICENSE").read_text(), (REPO / "LICENSE").read_text())
                grok = out / "plugins/grok-made"
                files = sorted(p.relative_to(grok).as_posix() for p in grok.rglob("*") if p.is_file())
                self.assertEqual(files, GROK_FILES)
                manifest = json.loads((grok / "plugin.json").read_text())
                self.assertEqual(manifest["version"], workspace_version())
                self.assertEqual((grok / "hooks/bin/made-grok-status").read_bytes(), self.hook.read_bytes())
                claude = out / "plugins/claude-code-made"
                files = sorted(p.relative_to(claude).as_posix() for p in claude.rglob("*") if p.is_file())
                self.assertEqual(files, CLAUDE_FILES)
                manifest = json.loads((claude / ".claude-plugin/plugin.json").read_text())
                self.assertEqual(manifest["version"], workspace_version())
                self.assertEqual((claude / "bin/made-claude-status").read_bytes(), self.claude_hook.read_bytes())
                self.assertFalse((out / "plugin").exists())

    def test_executable_bits_on_unix(self) -> None:
        if os.name == "nt":
            self.skipTest("no executable bit on Windows")
        out = self.staged("release")
        for path in (
            out / "made-app",
            out / "plugins/grok-made/hooks/bin/made-grok-status",
            out / "plugins/claude-code-made/bin/made-claude-status",
        ):
            self.assertTrue(path.stat().st_mode & stat.S_IXUSR, path)

    def test_fails_when_a_claude_plugin_source_file_is_missing(self) -> None:
        for name in (".claude-plugin/plugin.json", "README.md", "hooks/hooks.json"):
            with self.subTest(missing=name):
                (self.claude_source / name).rename(self.claude_source / (name + ".off"))
                with self.assertRaises((SystemExit, subprocess.CalledProcessError)):
                    self.staged(name.replace("/", "-"))
                (self.claude_source / (name + ".off")).rename(self.claude_source / name)

    def test_fails_when_the_claude_hook_is_missing_or_empty(self) -> None:
        for label, content in (("missing", None), ("empty", b"")):
            with self.subTest(hook=label):
                if content is None:
                    self.claude_hook.unlink()
                else:
                    self.claude_hook.write_bytes(content)
                with self.assertRaises((SystemExit, subprocess.CalledProcessError)):
                    self.staged(label)

    def test_fails_when_the_claude_hook_lacks_its_executable_bit(self) -> None:
        if os.name == "nt":
            self.skipTest("no executable bit on Windows")
        real_run = subprocess.run

        def strip_bit(command, *args, **kwargs):
            result = real_run(command, *args, **kwargs)
            if "stage-claude-plugin.sh" in str(command[1]):
                (Path(command[-1]) / "bin/made-claude-status").chmod(0o644)
            return result

        with mock.patch.object(stage_desktop.subprocess, "run", strip_bit):
            with self.assertRaises(SystemExit):
                self.staged("release")

    def test_release_metadata(self) -> None:
        out = self.staged("release")
        data = json.loads((out / "version.json").read_text())
        self.assertEqual(data["profile"], "release")
        self.assertEqual(data["version"], workspace_version())
        self.assertNotEqual(data.get("installer"), "none")
        self.assertNotEqual(data.get("auto_update"), "none")
        self.assertEqual(data["auto_update"]["repository"], "wcgomes/made")
        self.assertFalse((out / "Cargo.toml").exists())

    def test_debug_metadata_keeps_the_development_contents(self) -> None:
        out = self.staged("debug")
        data = json.loads((out / "version.json").read_text())
        self.assertEqual(data["profile"], "debug")
        self.assertEqual(data["installer"], "none")
        self.assertEqual(data["auto_update"], "none")
        for name in ("README.md", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
            self.assertTrue((out / name).is_file(), name)

    def test_refuses_to_overwrite_a_stage(self) -> None:
        self.staged("debug")
        with self.assertRaises(SystemExit):
            self.staged("debug")

    def test_workspace_version_is_semver(self) -> None:
        self.assertRegex(workspace_version(), re.compile(r"^\d+\.\d+\.\d+$"))


if __name__ == "__main__":
    unittest.main()
