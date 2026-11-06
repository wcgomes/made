#!/usr/bin/env python3
"""Refuse a release tag before anything is built or published (REL-01, REL-02).

Usage:
  release-gate.py --tag vX.Y.Z [--repo OWNER/REPO]
  release-gate.py --tag vX.Y.Z [--repo OWNER/REPO] --delete-drafts
  release-gate.py --tag vX.Y.Z [--repo OWNER/REPO] --check-newest

The gate refuses a tag that is not `v<stable semantic version>` (prerelease
and build versions included), disagrees with any version site, is already a
published release, or is not greater than the newest published stable
release. `--delete-drafts` removes unpublished drafts for the tag left by a
failed run; it needs a token that can write releases and never touches a
published release. The full gate runs again in the publish job right before
publishing. `--check-newest`, after publishing, fails unless the tag is the
newest published stable release. GitHub is reached through the `gh` CLI.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("check_versions", HERE / "check-versions.py")
check_versions = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_versions)


def parse(version: str) -> tuple[int, int, int] | None:
    match = check_versions.SEMVER.match(version)
    if not match or match.group(4) or match.group(5):
        return None
    return int(match.group(1)), int(match.group(2)), int(match.group(3))


class GitHub:
    """The release API the gate needs, through `gh api`."""

    def __init__(self, repo: str) -> None:
        self.repo = repo

    def releases(self) -> list[dict]:
        output = subprocess.run(
            ["gh", "api", "--paginate", "--jq", ".[]", f"repos/{self.repo}/releases?per_page=100"],
            check=True, capture_output=True, text=True,
        ).stdout
        # One compact JSON release per line across every page.
        return [json.loads(line) for line in output.splitlines() if line.strip()]

    def delete_release(self, release_id: int) -> None:
        subprocess.run(
            ["gh", "api", "-X", "DELETE", f"repos/{self.repo}/releases/{release_id}"],
            check=True, capture_output=True, text=True,
        )


def flatten(pages) -> list[dict]:
    if pages and isinstance(pages[0], list):
        return [release for page in pages for release in page]
    return list(pages)


def evaluate(tag: str, releases: list[dict], root: Path) -> list[str]:
    """Every reason to refuse `tag`; empty means it may be released."""
    problems = check_versions.check(root, tag)
    version = parse(tag[1:]) if tag.startswith("v") else None
    published = [r for r in releases if not r.get("draft")]
    if any(r.get("tag_name") == tag for r in published):
        problems.append(f"release {tag} is already published; published versions are never replaced")
    stable = [
        (parse(r.get("tag_name", "")[1:]), r.get("tag_name"))
        for r in published
        if not r.get("prerelease") and r.get("tag_name", "").startswith("v")
    ]
    stable = [(v, name) for v, name in stable if v is not None]
    if version is not None and stable:
        newest, newest_tag = max(stable)
        if version <= newest:
            problems.append(f"{tag} is not greater than the newest published stable release {newest_tag}")
    return problems


def newest_problems(tag: str, releases: list[dict]) -> list[str]:
    """After publishing: `tag` must be published and the newest stable release."""
    published = [r for r in releases if not r.get("draft") and not r.get("prerelease")]
    if not any(r.get("tag_name") == tag for r in published):
        return [f"{tag} is not a published stable release"]
    mine = parse(tag[1:]) if tag.startswith("v") else None
    newer = [
        r["tag_name"]
        for r in published
        if r.get("tag_name", "").startswith("v")
        and (version := parse(r["tag_name"][1:])) is not None
        and mine is not None
        and version > mine
    ]
    return [f"{name} is newer than {tag}" for name in newer]


def delete_drafts(tag: str, github: GitHub) -> list[int]:
    """Delete unpublished drafts for `tag`; returns the deleted ids."""
    deleted = []
    for release in flatten(github.releases()):
        if release.get("tag_name") == tag and release.get("draft"):
            github.delete_release(release["id"])
            deleted.append(release["id"])
    return deleted


def main(argv: list[str] | None = None, github: GitHub | None = None, root: Path | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "wcgomes/made"))
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--delete-drafts", action="store_true")
    mode.add_argument("--check-newest", action="store_true")
    args = parser.parse_args(argv)
    github = github or GitHub(args.repo)
    root = root or HERE.parents[1]
    if args.delete_drafts:
        for release_id in delete_drafts(args.tag, github):
            print(f"deleted unpublished draft {release_id} for {args.tag}")
        return 0
    releases = flatten(github.releases())
    if args.check_newest:
        problems = newest_problems(args.tag, releases)
        for problem in problems:
            print(f"publish check failed: {problem}", file=sys.stderr)
        if not problems:
            print(f"{args.tag} is the newest published stable release")
        return 1 if problems else 0
    problems = evaluate(args.tag, releases, root)
    for problem in problems:
        print(f"release refused: {problem}", file=sys.stderr)
    if not problems:
        print(f"release gate passed for {args.tag}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
