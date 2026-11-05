#!/usr/bin/env python3
"""Replace Velopack's AppImage runtime with a FUSE-less one (probe 1.4, fallback A).

Usage: linux-appimage-runtime.py --out VPK_OUT --channel linux-x64 --runtime URUNTIME --version X.Y.Z

Velopack's type-2 runtime refuses to start without `fusermount`. The AppImage
in VPK_OUT, and the copy inside each full package of the channel feed, is
rebuilt as the static uruntime followed by a rebuilt squashfs image, with
`APPIMAGE_EXTRACT_AND_RUN=1` embedded so it never needs FUSE or user
namespaces. The feed's size, SHA-1 and SHA-256 are rewritten to match.

Extraction is private to the user (security M2): uruntime's default target is
a predictable shared `$TMPDIR/appimage_extracted_*` directory that it reuses
without checking its owner, so another local user could plant code there. The
embedded `APPIMAGE_TARGET_DIR=${HOME}/.cache/made/appimage-<version>` keeps
the target inside the user's home, the image root is mode 0700 so a fresh
extraction is private, and the image's `AppRun` is a guard that refuses to
start unless the extraction directory is a real directory owned by the user
with no group or other permission bits, then runs Velopack's `AppRun`.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import json
import shutil
import stat
import subprocess
import sys
import zipfile
from pathlib import Path

GUARD = """#!/bin/sh
# Made: start only from an extraction directory private to this user.
dir="${APPDIR:-$(dirname "$0")}"
owner="$(stat -c %u "$dir" 2>/dev/null)"
mode="$(stat -c %a "$dir" 2>/dev/null)"
if [ -L "$dir" ] || [ ! -d "$dir" ] || [ "$owner" != "$(id -u)" ] || [ -z "$mode" ] \\
  || [ $((0$mode & 077)) -ne 0 ]; then
  echo "made: refusing to start from $dir: it is not a private directory owned by this user" >&2
  exit 1
fi
unset APPIMAGE_EXTRACT_AND_RUN
exec "$dir/AppRun.velopack" "$@"
"""


def embedded_envs(version: str) -> str:
    return f"APPIMAGE_EXTRACT_AND_RUN=1\nAPPIMAGE_TARGET_DIR=${{HOME}}/.cache/made/appimage-{version}"


def fail(message: str) -> None:
    print(f"appimage runtime failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def rebuild(appimage: Path, runtime: Path, version: str) -> None:
    offset = int(subprocess.run([str(appimage), "--appimage-offset"], check=True,
                                capture_output=True, text=True).stdout.strip())
    work = appimage.parent / "appimage-rebuild"
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    image = work / "original.squashfs"
    with appimage.open("rb") as source, image.open("wb") as target:
        source.seek(offset)
        shutil.copyfileobj(source, target)
    root = work / "root"
    subprocess.run(["unsquashfs", "-no-progress", "-d", str(root), str(image)], check=True)
    velopack_apprun = root / "AppRun"
    if not velopack_apprun.is_file():
        fail("the Velopack AppImage has no AppRun")
    velopack_apprun.rename(root / "AppRun.velopack")
    guard = root / "AppRun"
    guard.write_text(GUARD, encoding="utf-8")
    guard.chmod(0o755)
    root.chmod(0o700)
    rebuilt_image = work / "rebuilt.squashfs"
    env = dict(os.environ)
    env.pop("SOURCE_DATE_EPOCH", None)
    subprocess.run(["mksquashfs", str(root), str(rebuilt_image), "-comp", "gzip", "-root-owned",
                    "-noappend", "-mkfs-time", "0", "-no-progress"], check=True, env=env)
    rebuilt = appimage.with_suffix(".rebuilt")
    with rebuilt.open("wb") as target:
        target.write(runtime.read_bytes())
        with rebuilt_image.open("rb") as source:
            shutil.copyfileobj(source, target)
    rebuilt.chmod(rebuilt.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    envs = embedded_envs(version)
    subprocess.run([str(rebuilt), "--appimage-addenvs", envs], check=True)
    shown = subprocess.run([str(rebuilt), "--appimage-envs"], check=True, capture_output=True, text=True).stdout
    for line in envs.splitlines():
        if line not in shown:
            fail(f"{line} was not embedded: {shown!r}")
    rebuilt.replace(appimage)
    shutil.rmtree(work, ignore_errors=True)


def rewrite_packages(out: Path, channel: str, appimage: Path) -> None:
    feed_path = out / f"releases.{channel}.json"
    feed = json.loads(feed_path.read_text(encoding="utf-8"))
    data = appimage.read_bytes()
    for asset in feed["Assets"]:
        package = out / asset["FileName"]
        rebuilt = package.with_suffix(".rebuilt")
        replaced = 0
        with zipfile.ZipFile(package) as old, zipfile.ZipFile(rebuilt, "w", zipfile.ZIP_DEFLATED) as new:
            for info in old.infolist():
                if info.filename.endswith(".AppImage"):
                    new.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED)
                    replaced += 1
                else:
                    new.writestr(info, old.read(info), compress_type=info.compress_type)
        if replaced != 1:
            fail(f"{package.name} holds {replaced} AppImages, expected 1")
        rebuilt.replace(package)
        payload = package.read_bytes()
        asset["SHA1"] = hashlib.sha1(payload).hexdigest().upper()
        asset["SHA256"] = hashlib.sha256(payload).hexdigest().upper()
        asset["Size"] = len(payload)
    feed_path.write_text(json.dumps(feed, indent=2), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    images = list(args.out.glob("*.AppImage"))
    if len(images) != 1:
        fail(f"expected one AppImage in {args.out}, found {len(images)}")
    rebuild(images[0], args.runtime, args.version)
    rewrite_packages(args.out, args.channel, images[0])
    print(f"rebuilt {images[0].name} with the FUSE-less runtime")
    return 0


if __name__ == "__main__":
    sys.exit(main())
