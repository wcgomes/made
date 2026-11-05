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
embedded `APPIMAGE_TARGET_DIR=${HOME}/.cache/made/appimage-<version>-<hash>`
keeps the target in a per-user location, named by the image's content hash so
two builds of one version never share an extraction. The image root is mode
0700 so a fresh extraction is private. uruntime 0.8.1 expands only plain
`${NAME}` references in embedded settings, with no default value, so the
target cannot prefer `$XDG_RUNTIME_DIR` and fall back to `$HOME/.cache`; a
target under an unset `$XDG_RUNTIME_DIR` would leave Made unable to start, so
the target is always under `$HOME/.cache`.

The image's `AppRun` is a guard, kept as defence in depth: it refuses to start
unless the extraction directory is a real directory owned by the user with no
group or other permission bits and its parent is owned by the user (which it
then makes 0700). uruntime reuses any non-empty target, so an extraction cut
short (killed, or a full disk) would otherwise be run as is: the guard checks
every file against the SHA-256 manifest built into the image, and on a
mismatch removes the extraction and starts the AppImage once more to extract
afresh. Then it runs Velopack's `AppRun`.
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

MANIFEST = ".made-manifest.sha256"

GUARD = """#!/bin/sh
# Made: start only from a complete extraction in a directory private to this user.
dir="${APPDIR:-$(dirname "$0")}"
owned() {
  [ ! -L "$1" ] && [ -d "$1" ] && [ "$(stat -c %u "$1" 2>/dev/null)" = "$(id -u)" ]
}
mode="$(stat -c %a "$dir" 2>/dev/null)"
if ! owned "$dir" || [ -z "$mode" ] || [ $((0$mode & 077)) -ne 0 ]; then
  echo "made: refusing to start from $dir: it is not a private directory owned by this user" >&2
  exit 1
fi
parent="$(dirname "$dir")"
if ! owned "$parent"; then
  echo "made: refusing to start from $dir: its parent $parent is not owned by this user" >&2
  exit 1
fi
chmod 700 "$parent" 2>/dev/null
if ! (cd "$dir" && sha256sum --quiet --strict -c MANIFEST_NAME >/dev/null 2>&1); then
  rm -rf "$dir"
  if [ -n "$MADE_APPIMAGE_REEXTRACTED" ] || [ ! -f "$APPIMAGE" ]; then
    echo "made: the extracted copy of Made in $dir was incomplete and has been removed; start Made again" >&2
    exit 1
  fi
  export MADE_APPIMAGE_REEXTRACTED=1
  exec "$APPIMAGE" "$@"
fi
unset APPIMAGE_EXTRACT_AND_RUN MADE_APPIMAGE_REEXTRACTED
exec "$dir/AppRun.velopack" "$@"
""".replace("MANIFEST_NAME", MANIFEST)


def embedded_envs(version: str, content_hash: str) -> str:
    return (
        "APPIMAGE_EXTRACT_AND_RUN=1\n"
        f"APPIMAGE_TARGET_DIR=${{HOME}}/.cache/made/appimage-{version}-{content_hash}"
    )


def write_manifest(root: Path) -> None:
    """SHA-256 of every file in the image, in `sha256sum -c` format."""
    lines = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink() or not (path.is_dir() or path.is_file()):
            fail(f"{relative} is not a plain file or directory; the guard checks only those")
        if "\n" in relative or "\\" in relative:
            fail(f"{relative!r} cannot be listed in the manifest")
        if path.is_file():
            lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  ./{relative}\n")
    if not lines:
        fail("the image holds no files")
    (root / MANIFEST).write_text("".join(lines), encoding="utf-8")


def content_hash(image: Path) -> str:
    return hashlib.sha256(image.read_bytes()).hexdigest()[:16]


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
    write_manifest(root)
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
    envs = embedded_envs(version, content_hash(rebuilt_image))
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
