#!/usr/bin/env python3
"""Compose the macOS Made.app from an assembled desktop stage (probe 1.3).

Usage: compose-macos-bundle.py --stage DIR --bundle OUT/Made.app --version X.Y.Z

Velopack packs a prebuilt `.app` as is. Mach-O executables (`made-app`,
`made-askpass`) go to `Contents/MacOS`; everything else (plugins, LICENSE,
README, version.json, the companion tree) goes to `Contents/Resources`,
because files that are not Mach-O lose their code signature in Velopack's
update package when they live in `Contents/MacOS`. `Contents/MacOS/remote` is a
relative symlink to `../Resources/remote`, so companions still resolve beside
the executable while `AppPaths` resolves plugins from `Contents/Resources`.
"""

from __future__ import annotations

import argparse
import os
import shutil
import stat
import sys
from pathlib import Path

EXECUTABLES = ("made-app", "made-askpass")
BUNDLE_ID = "io.github.wcgomes.made"


def fail(message: str) -> None:
    print(f"bundle failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def info_plist(version: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>made-app</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Made</string>
  <key>CFBundleDisplayName</key><string>Made</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"""


def compose(stage: Path, bundle: Path, version: str) -> None:
    if bundle.exists():
        fail(f"refusing to overwrite {bundle}")
    for name in EXECUTABLES:
        if not (stage / name).is_file():
            fail(f"stage is missing {name}")
    macos = bundle / "Contents" / "MacOS"
    resources = bundle / "Contents" / "Resources"
    macos.mkdir(parents=True)
    resources.mkdir(parents=True)
    (bundle / "Contents" / "Info.plist").write_text(info_plist(version), encoding="utf-8")
    for entry in sorted(stage.iterdir()):
        destination = (macos if entry.name in EXECUTABLES else resources) / entry.name
        if entry.is_dir():
            shutil.copytree(entry, destination, symlinks=False)
        else:
            shutil.copy2(entry, destination)
    if not (resources / "remote").is_dir():
        fail("stage has no remote/ companion tree")
    os.symlink("../Resources/remote", macos / "remote")
    for path in [macos / name for name in EXECUTABLES] + list((resources / "remote").rglob("made-remote")) + list(
        (resources / "remote").rglob("made-grok-status")
    ) + list((resources / "plugins").rglob("made-grok-status")):
        path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    for required in ("plugins/opencode-made/package.json", "plugins/grok-made/plugin.json", "LICENSE"):
        if not (resources / required).is_file():
            fail(f"bundle is missing Contents/Resources/{required}")
    print(f"composed {bundle}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--stage", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    compose(args.stage, args.bundle, args.version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
