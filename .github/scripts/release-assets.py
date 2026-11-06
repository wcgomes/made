#!/usr/bin/env python3
"""Finish one target's Velopack output into release assets, and write SHA256SUMS.

Usage:
  release-assets.py finish --out VPK_OUT --version X.Y.Z --channel CHANNEL --dest DIR
  release-assets.py sums --dir DIR
  release-assets.py check-sums --dir DIR

`finish` copies the end-user downloads under `made-<version>-<os>-<arch>`
names, keeps every feed-referenced file (the channel feed and the packages
it lists) under its Velopack name, and leaves out Velopack's legacy
`RELEASES-*` and upload-only `assets.*.json` files. Any other file fails.
`sums` lists the SHA-256 of every other file in DIR in `SHA256SUMS`; it
detects corruption only and does not prove who published the release.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path

# channel -> (os label, {Velopack name suffix: release name suffix})
DOWNLOADS = {
    "win-x64": ("windows-x64", {"-Setup.exe": "-setup.exe"}),
    "osx-arm64": ("macos-arm64", {"-Setup.pkg": ".pkg", "-Portable.zip": ".zip"}),
    "osx-x64": ("macos-x64", {"-Setup.pkg": ".pkg", "-Portable.zip": ".zip"}),
    "linux-x64": ("linux-x64", {".AppImage": ".AppImage"}),
}
SUMS = "SHA256SUMS"


def fail(message: str) -> None:
    print(f"release assets: {message}", file=sys.stderr)
    raise SystemExit(1)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def feed_files(out: Path, channel: str, version: str) -> set[str]:
    feed_path = out / f"releases.{channel}.json"
    if not feed_path.is_file():
        fail(f"feed {feed_path.name} is missing")
    feed = json.loads(feed_path.read_text(encoding="utf-8"))
    assets = feed.get("Assets")
    if not isinstance(assets, list) or not assets:
        fail(f"feed {feed_path.name} lists no packages")
    names = {feed_path.name}
    for asset in assets:
        if asset.get("Version") != version:
            fail(f"feed entry {asset.get('FileName')} has version {asset.get('Version')}, expected {version}")
        name = asset.get("FileName", "")
        if not (out / name).is_file() or "/" in name or "\\" in name:
            fail(f"feed-referenced package {name!r} is missing")
        names.add(name)
    return names


def finish(out: Path, version: str, channel: str, dest: Path) -> list[str]:
    if channel not in DOWNLOADS:
        fail(f"unknown channel {channel}")
    label, suffixes = DOWNLOADS[channel]
    dest.mkdir(parents=True, exist_ok=True)
    keep = feed_files(out, channel, version)
    written: list[str] = []
    found_downloads = set()
    for path in sorted(p for p in out.iterdir() if p.is_file()):
        name = path.name
        if name in keep:
            target = name
        elif name.startswith("RELEASES-") or (name.startswith("assets.") and name.endswith(".json")):
            continue
        else:
            match = next((suffix for suffix in suffixes if name.endswith(suffix)), None)
            if match is None:
                fail(f"unexpected Velopack output {name}")
            found_downloads.add(match)
            target = f"made-{version}-{label}{suffixes[match]}"
        if (dest / target).exists():
            fail(f"refusing to overwrite {target}")
        shutil.copy2(path, dest / target)
        written.append(target)
    missing = set(suffixes) - found_downloads
    if missing:
        fail(f"missing end-user downloads for {channel}: {sorted(missing)}")
    return written


def write_sums(directory: Path) -> Path:
    lines = [
        f"{sha256(path)}  {path.name}"
        for path in sorted(directory.iterdir(), key=lambda p: p.name)
        if path.is_file() and path.name != SUMS
    ]
    if not lines:
        fail(f"no assets in {directory}")
    sums = directory / SUMS
    sums.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return sums


def check_sums(directory: Path) -> None:
    sums = directory / SUMS
    listed = {}
    for line in sums.read_text(encoding="utf-8").splitlines():
        digest, name = line.split("  ", 1)
        listed[name] = digest
    present = {p.name for p in directory.iterdir() if p.is_file() and p.name != SUMS}
    if set(listed) != present:
        fail(f"SHA256SUMS lists {sorted(listed)} but the assets are {sorted(present)}")
    for name, digest in listed.items():
        if sha256(directory / name) != digest:
            fail(f"digest mismatch for {name}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    finish_cmd = sub.add_parser("finish")
    finish_cmd.add_argument("--out", type=Path, required=True)
    finish_cmd.add_argument("--version", required=True)
    finish_cmd.add_argument("--channel", required=True)
    finish_cmd.add_argument("--dest", type=Path, required=True)
    for name in ("sums", "check-sums"):
        sub.add_parser(name).add_argument("--dir", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.command == "finish":
        for name in finish(args.out, args.version, args.channel, args.dest):
            print(name)
    elif args.command == "sums":
        print(write_sums(args.dir).read_text(encoding="utf-8"), end="")
    else:
        check_sums(args.dir)
        print("SHA256SUMS matches every asset")
    return 0


if __name__ == "__main__":
    sys.exit(main())
