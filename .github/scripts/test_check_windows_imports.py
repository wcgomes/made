#!/usr/bin/env python3
"""Tests for check-windows-imports.py using sample dumpbin output (task 3.7)."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "check-windows-imports.py"
spec = importlib.util.spec_from_file_location("check_windows_imports", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def dumpbin(binary: str, *dlls: str) -> str:
    body = "\n".join(f"    {name}" for name in dlls)
    return f"""Microsoft (R) COFF/PE Dumper Version 14.44.35211.0
Copyright (C) Microsoft Corporation.  All rights reserved.


Dump of file target\\release\\{binary}

File Type: EXECUTABLE IMAGE

  Image has the following dependencies:

{body}

  Summary

        5000 .data
"""


STATIC_APP = ("kernel32.dll", "user32.dll", "gdi32.dll", "ntdll.dll", "advapi32.dll", "ws2_32.dll",
              "bcryptprimitives.dll", "ole32.dll", "shell32.dll", "api-ms-win-core-synch-l1-2-0.dll")


class CheckWindowsImportsTest(unittest.TestCase):
    def run_cli(self, text: str) -> subprocess.CompletedProcess:
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as handle:
            handle.write(text)
        return subprocess.run([sys.executable, str(SCRIPT), "--dumpbin", handle.name], capture_output=True, text=True)

    def test_in_box_imports_pass_for_each_binary(self) -> None:
        for binary in ("made-app.exe", "made-askpass.exe", "made-grok-status.exe"):
            with self.subTest(binary=binary):
                sample = dumpbin(binary, *STATIC_APP)
                self.assertEqual(check.imports(sample), list(STATIC_APP))
                self.assertEqual(check.violations(sample), [])
                self.assertEqual(self.run_cli(sample).returncode, 0)

    def test_msvc_runtime_imports_fail(self) -> None:
        for dll in ("VCRUNTIME140.dll", "VCRUNTIME140_1.dll", "MSVCP140.dll",
                    "api-ms-win-crt-runtime-l1-1-0.dll", "ucrtbase.dll"):
            with self.subTest(dll=dll):
                sample = dumpbin("made-grok-status.exe", "kernel32.dll", dll)
                self.assertEqual(check.violations(sample), [dll])
                result = self.run_cli(sample)
                self.assertEqual(result.returncode, 1)
                self.assertIn(dll, result.stderr)

    def test_unknown_dll_fails(self) -> None:
        sample = dumpbin("made-askpass.exe", "kernel32.dll", "libssl-3-x64.dll")
        self.assertEqual(check.violations(sample), ["libssl-3-x64.dll"])

    def test_empty_output_fails(self) -> None:
        self.assertEqual(self.run_cli("nothing here\n").returncode, 1)


if __name__ == "__main__":
    unittest.main()
