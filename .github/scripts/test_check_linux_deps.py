#!/usr/bin/env python3
"""Tests for check-linux-deps.py using sample `readelf -d` output (task 3.5)."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check-linux-deps.py"
spec = importlib.util.spec_from_file_location("check_linux_deps", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def readelf(*libraries: str) -> str:
    lines = ["", "Dynamic section at offset 0x5c9e8 contains 30 entries:", "  Tag        Type                         Name/Value"]
    for name in libraries:
        lines.append(f" 0x0000000000000001 (NEEDED)             Shared library: [{name}]")
    lines.append(" 0x000000000000000c (INIT)               0x1e000")
    return "\n".join(lines) + "\n"


class CheckLinuxDepsTest(unittest.TestCase):
    def run_cli(self, text: str) -> subprocess.CompletedProcess:
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as handle:
            handle.write(text)
        return subprocess.run([sys.executable, str(SCRIPT), "--readelf", handle.name], capture_output=True, text=True)

    def test_glibc_and_libgcc_are_allowed(self) -> None:
        sample = readelf("libgcc_s.so.1", "libm.so.6", "libc.so.6", "ld-linux-x86-64.so.2", "libpthread.so.0", "libdl.so.2")
        self.assertEqual(check.violations(sample), [])
        self.assertEqual(self.run_cli(sample).returncode, 0)

    def test_any_other_library_fails(self) -> None:
        for name in ("libxkbcommon.so.0", "libssl.so.3", "libfuse.so.2", "libwayland-client.so.0", "libX11.so.6", "libstdc++.so.6"):
            with self.subTest(library=name):
                sample = readelf("libc.so.6", name)
                self.assertEqual(check.violations(sample), [name])
                result = self.run_cli(sample)
                self.assertEqual(result.returncode, 1)
                self.assertIn(name, result.stderr)

    def test_no_needed_entries_fails(self) -> None:
        self.assertEqual(self.run_cli(readelf()).returncode, 1)


if __name__ == "__main__":
    unittest.main()
