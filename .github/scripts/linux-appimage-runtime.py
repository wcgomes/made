#!/usr/bin/env python3
"""Replace Velopack's AppImage runtime with a FUSE-less one (probe 1.4, fallback A).

Usage: linux-appimage-runtime.py --out VPK_OUT --channel linux-x64 --runtime URUNTIME

Velopack's type-2 runtime refuses to start without `fusermount`. The AppImage
in VPK_OUT, and the copy inside each full package of the channel feed, is
rebuilt as the static uruntime followed by the unchanged squashfs image,
with `APPIMAGE_EXTRACT_AND_RUN=1` embedded so it never needs FUSE or user
namespaces. The feed's size, SHA-1 and SHA-256 are rewritten to match.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import stat
import subprocess
import sys
import zipfile
from pathlib import Path

EMBEDDED_ENV = "APPIMAGE_EXTRACT_AND_RUN=1"


def fail(message: str) -> None:
    print(f"appimage runtime failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def rebuild(appimage: Path, runtime: Path) -> None:
    offset = int(subprocess.run([str(appimage), "--appimage-offset"], check=True,
                                capture_output=True, text=True).stdout.strip())
    rebuilt = appimage.with_suffix(".rebuilt")
    with appimage.open("rb") as source, rebuilt.open("wb") as target:
        target.write(runtime.read_bytes())
        source.seek(offset)
        shutil.copyfileobj(source, target)
    rebuilt.chmod(rebuilt.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    subprocess.run([str(rebuilt), "--appimage-addenvs", EMBEDDED_ENV], check=True)
    envs = subprocess.run([str(rebuilt), "--appimage-envs"], check=True, capture_output=True, text=True).stdout
    if EMBEDDED_ENV not in envs:
        fail(f"{EMBEDDED_ENV} was not embedded: {envs!r}")
    rebuilt.replace(appimage)


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
    args = parser.parse_args()
    images = list(args.out.glob("*.AppImage"))
    if len(images) != 1:
        fail(f"expected one AppImage in {args.out}, found {len(images)}")
    rebuild(images[0], args.runtime)
    rewrite_packages(args.out, args.channel, images[0])
    print(f"rebuilt {images[0].name} with the FUSE-less runtime")
    return 0


if __name__ == "__main__":
    sys.exit(main())
