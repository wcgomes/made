#!/usr/bin/env python3
"""Fail when a Linux release binary needs a library outside the REL-04 baseline.

Usage: check-linux-deps.py BINARY...           (runs `readelf -d`)
       check-linux-deps.py --readelf FILE...   (reads saved `readelf -d` output)

Only glibc's own libraries and `libgcc_s.so.1` may appear as ELF `NEEDED`
entries. The glibc symbol-version floor stays with check-glibc-max.sh, and
libraries loaded at runtime by name (libxkbcommon, Wayland, X11, EGL/GL,
Vulkan) are covered by fresh-distribution acceptance, not by this check.
"""

from __future__ import annotations

import re
import subprocess
import sys

ALLOWED = {
    "libc.so.6",
    "libm.so.6",
    "libpthread.so.0",
    "libdl.so.2",
    "librt.so.1",
    "libutil.so.1",
    "libresolv.so.2",
    "ld-linux-x86-64.so.2",
    "libgcc_s.so.1",
}
NEEDED = re.compile(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]")


def needed(readelf_output: str) -> list[str]:
    return NEEDED.findall(readelf_output)


def violations(readelf_output: str) -> list[str]:
    return [name for name in needed(readelf_output) if name not in ALLOWED]


def main(argv: list[str]) -> int:
    saved = bool(argv) and argv[0] == "--readelf"
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
            output = subprocess.run(["readelf", "-d", path], capture_output=True, text=True, check=True).stdout
        names = needed(output)
        if not names:
            print(f"{path}: no NEEDED entries; refusing an unexpected static or non-ELF binary", file=sys.stderr)
            failed = True
            continue
        bad = violations(output)
        for name in bad:
            print(f"{path}: needs {name}, which is outside the Linux baseline", file=sys.stderr)
        failed = failed or bool(bad)
        if not bad:
            print(f"{path}: NEEDED {', '.join(names)}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
