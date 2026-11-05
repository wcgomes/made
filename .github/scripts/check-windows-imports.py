#!/usr/bin/env python3
"""Fail when a Windows release binary imports a DLL outside Windows 10 1809.

Usage: check-windows-imports.py --dumpbin FILE...   (saved `dumpbin /dependents` output)

The allowlist holds DLLs that ship in-box with Windows 10 version 1809 and
later, plus the `api-ms-win-core-*` API sets they resolve. The C/C++ runtime
redistributable (`VCRUNTIME*`, `MSVCP*`, `api-ms-win-crt-*`, `ucrtbase`) is
refused: release binaries link the CRT statically (REL-04).
"""

from __future__ import annotations

import re
import sys

ALLOWED = {
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "cfgmgr32.dll", "combase.dll", "comctl32.dll",
    "comdlg32.dll", "crypt32.dll", "d3d11.dll", "d3d12.dll", "d3dcompiler_47.dll", "dbghelp.dll",
    "dwmapi.dll", "dwrite.dll", "dxgi.dll", "gdi32.dll", "hid.dll", "imm32.dll", "iphlpapi.dll",
    "kernel32.dll", "msimg32.dll", "mswsock.dll", "ncrypt.dll", "netapi32.dll", "ntdll.dll", "ole32.dll",
    "oleaut32.dll", "opengl32.dll", "powrprof.dll", "propsys.dll", "psapi.dll", "rpcrt4.dll",
    "secur32.dll", "setupapi.dll", "shcore.dll", "shell32.dll", "shlwapi.dll", "uiautomationcore.dll",
    "user32.dll", "userenv.dll", "uxtheme.dll", "version.dll", "winmm.dll", "ws2_32.dll", "wtsapi32.dll",
    "dinput8.dll", "xinput1_4.dll", "winhttp.dll", "wininet.dll", "credui.dll", "msi.dll",
}
REFUSED = re.compile(r"^(vcruntime|msvcp|api-ms-win-crt-|ucrtbase|concrt|vccorlib)", re.IGNORECASE)
DLL = re.compile(r"^\s+(\S+\.dll)\s*$", re.IGNORECASE)


def imports(dumpbin_output: str) -> list[str]:
    return [match.group(1) for line in dumpbin_output.splitlines() if (match := DLL.match(line))]


def violations(dumpbin_output: str) -> list[str]:
    bad = []
    for name in imports(dumpbin_output):
        lower = name.lower()
        if REFUSED.match(lower):
            bad.append(name)
        elif lower not in ALLOWED and not lower.startswith("api-ms-win-core-"):
            bad.append(name)
    return bad


def main(argv: list[str]) -> int:
    if len(argv) < 2 or argv[0] != "--dumpbin":
        print(__doc__, file=sys.stderr)
        return 2
    failed = False
    for path in argv[1:]:
        with open(path, encoding="utf-8", errors="replace") as handle:
            output = handle.read()
        names = imports(output)
        bad = violations(output)
        if not names:
            print(f"{path}: no imports found; refusing unreadable dumpbin output", file=sys.stderr)
            failed = True
        for name in bad:
            print(f"{path}: imports {name}, which does not ship with Windows 10 1809", file=sys.stderr)
        failed = failed or bool(bad)
        if names and not bad:
            print(f"{path}: imports {', '.join(names)}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
