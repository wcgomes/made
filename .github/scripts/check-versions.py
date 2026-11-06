#!/usr/bin/env python3
"""Check that every Made version site equals `[workspace.package].version`.

Usage: check-versions.py [--root DIR] [--tag vX.Y.Z]

The workspace `Cargo.toml` is the single version source (REL-02). Every other
site must equal it; each disagreement is reported with its file, its version
and the workspace version. With `--tag`, the tag must be `v<version>` and a
stable semantic version: prerelease and build-metadata tags are refused.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)
CARGO = "Cargo.toml"
PACKAGE = "plugins/opencode-made/package.json"
LOCK = "plugins/opencode-made/package-lock.json"
PROTOCOL = "plugins/opencode-made/src/protocol.ts"
GROK = "plugins/grok-made/plugin.json"


class VersionError(Exception):
    """A version site is missing, unreadable or invalid."""


def workspace_version(root: Path) -> str:
    in_section = False
    for line in (root / CARGO).read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_section = stripped == "[workspace.package]"
            continue
        if in_section:
            match = re.match(r'^version\s*=\s*"([^"]*)"\s*$', stripped)
            if match:
                return match.group(1)
    raise VersionError(f"{CARGO}: [workspace.package].version not found")


def json_file(root: Path, name: str) -> dict:
    try:
        return json.loads((root / name).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise VersionError(f"{name}: cannot read: {error}") from error


def protocol_version(root: Path) -> str | None:
    text = (root / PROTOCOL).read_text(encoding="utf-8")
    matches = re.findall(r'^export const PLUGIN_VERSION = "([^"]*)";$', text, flags=re.MULTILINE)
    return matches[0] if len(matches) == 1 else None


def sites(root: Path) -> list[tuple[str, str | None]]:
    """Every version site other than the workspace, as (label, version)."""
    package = json_file(root, PACKAGE)
    lock = json_file(root, LOCK)
    return [
        (PACKAGE, package.get("version")),
        (f"{LOCK} (version)", lock.get("version")),
        (f'{LOCK} (packages[""].version)', lock.get("packages", {}).get("", {}).get("version")),
        (f"{PROTOCOL} (PLUGIN_VERSION)", protocol_version(root)),
        (GROK, json_file(root, GROK).get("version")),
    ]


def check(root: Path, tag: str | None = None) -> list[str]:
    """Return every problem found; an empty list means the versions agree."""
    try:
        version = workspace_version(root)
        found = sites(root)
    except (OSError, VersionError) as error:
        return [str(error)]
    problems = []
    if not SEMVER.match(version):
        problems.append(f"{CARGO}: workspace version '{version}' is not a valid semantic version")
    for label, value in found:
        if value != version:
            problems.append(f"{label} version {value} differs from the workspace version {version}")
    if tag is not None:
        if not tag.startswith("v") or not SEMVER.match(tag[1:]):
            problems.append(f"tag {tag} is not v<semantic version>")
        else:
            tag_version = tag[1:]
            parsed = SEMVER.match(tag_version)
            if parsed.group(4) or parsed.group(5):
                problems.append(f"tag {tag} is a prerelease or build version; only stable releases are published")
            if tag_version != version:
                problems.append(f"tag {tag} version {tag_version} differs from the workspace version {version}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--tag")
    args = parser.parse_args()
    problems = check(args.root, args.tag)
    for problem in problems:
        print(f"version check failed: {problem}", file=sys.stderr)
    if not problems:
        print(f"versions agree: {workspace_version(args.root)}" + (f" (tag {args.tag})" if args.tag else ""))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
