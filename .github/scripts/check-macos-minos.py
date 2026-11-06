#!/usr/bin/env python3
"""Fail when a macOS release binary declares a minimum OS newer than 12.0.

Usage: check-macos-minos.py BINARY...           (runs `otool -l`)
       check-macos-minos.py --otool FILE...     (reads saved `otool -l` output)

Reads LC_BUILD_VERSION `minos` or, for older binaries, LC_VERSION_MIN_MACOSX
`version`. A binary with neither load command fails.
"""

from __future__ import annotations

import re
import subprocess
import sys

MAXIMUM = (12, 0)


def minimum_versions(otool_output: str) -> list[tuple[int, ...]]:
    found = []
    command = None
    for raw in otool_output.splitlines():
        line = raw.strip()
        if line.startswith("cmd "):
            command = line.split()[1]
            continue
        key = {"LC_BUILD_VERSION": "minos", "LC_VERSION_MIN_MACOSX": "version"}.get(command or "")
        if key and re.match(rf"^{key}\s", line):
            found.append(tuple(int(part) for part in line.split()[1].split(".")))
    return found


def acceptable(otool_output: str) -> bool:
    versions = minimum_versions(otool_output)
    return bool(versions) and all(version[:2] <= MAXIMUM for version in versions)


def main(argv: list[str]) -> int:
    saved = bool(argv) and argv[0] == "--otool"
    paths = argv[1:] if saved else argv
    if not paths:
        print(__doc__, file=sys.stderr)
        return 2
    failed = False
    for path in paths:
        if saved:
            with open(path, encoding="utf-8") as handle:
                output = handle.read()
        else:
            output = subprocess.run(["otool", "-l", path], capture_output=True, text=True, check=True).stdout
        versions = [".".join(map(str, v)) for v in minimum_versions(output)]
        if acceptable(output):
            print(f"{path}: minimum macOS {', '.join(versions)}")
        else:
            failed = True
            shown = ", ".join(versions) or "none declared"
            print(f"{path}: minimum macOS {shown} is not 12.0 or lower", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
