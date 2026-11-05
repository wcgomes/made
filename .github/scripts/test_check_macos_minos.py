#!/usr/bin/env python3
"""Tests for check-macos-minos.py using sample `otool -l` output (task 3.9)."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check-macos-minos.py"
spec = importlib.util.spec_from_file_location("check_macos_minos", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def build_version(minos: str) -> str:
    return f"""Load command 9
      cmd LC_BUILD_VERSION
  cmdsize 32
 platform 1
    minos {minos}
      sdk 15.2
   ntools 1
     tool 3
  version 1115.7.3
Load command 10
      cmd LC_SOURCE_VERSION
  cmdsize 16
  version 0.0
"""


LEGACY = """Load command 8
      cmd LC_VERSION_MIN_MACOSX
  cmdsize 16
  version 10.12
      sdk 10.15
"""


class CheckMacosMinosTest(unittest.TestCase):
    def run_cli(self, text: str) -> subprocess.CompletedProcess:
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as handle:
            handle.write(text)
        return subprocess.run([sys.executable, str(SCRIPT), "--otool", handle.name], capture_output=True, text=True)

    def test_minos_12_or_lower_passes(self) -> None:
        for minos in ("12.0", "11.0", "10.13"):
            with self.subTest(minos=minos):
                self.assertTrue(check.acceptable(build_version(minos)))
                self.assertEqual(self.run_cli(build_version(minos)).returncode, 0)
        self.assertTrue(check.acceptable(LEGACY))

    def test_higher_minos_fails(self) -> None:
        for minos in ("12.1", "13.0", "15.0"):
            with self.subTest(minos=minos):
                self.assertFalse(check.acceptable(build_version(minos)))
                result = self.run_cli(build_version(minos))
                self.assertEqual(result.returncode, 1)
                self.assertIn(minos, result.stderr)

    def test_tool_version_lines_are_not_minimums(self) -> None:
        self.assertEqual(check.minimum_versions(build_version("12.0")), [(12, 0)])

    def test_missing_declaration_fails(self) -> None:
        self.assertFalse(check.acceptable("Load command 1\n      cmd LC_SEGMENT_64\n"))


if __name__ == "__main__":
    unittest.main()
