#!/usr/bin/env python3
"""Stage one desktop package tree for a development artifact or a release.

Usage: stage-desktop-package.py --profile debug|release --target TRIPLE
       --app PATH --hook PATH --stage DIR

The tree holds made-app, the app-owned `plugins/opencode-made` package without
`node_modules`, the shipped `plugins/grok-made` package staged only through
`stage-grok-plugin.sh`, README.md, LICENSE and `version.json`. Debug artifacts
also carry the Cargo metadata and toolchain files they always carried.
Companions and askpass are added afterwards by assemble-desktop-package.py.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import stat
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UPDATE_REPO = "wcgomes/made"
COMPANION_TARGETS = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]


def fail(message: str) -> None:
    print(f"staging failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def workspace_version(root: Path) -> str:
    in_section = False
    for line in (root / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_section = stripped == "[workspace.package]"
        elif in_section:
            match = re.match(r'^version\s*=\s*"([^"]+)"', stripped)
            if match:
                return match.group(1)
    return "unknown"


def tool_version(command: list[str]) -> str:
    try:
        return subprocess.run(command, capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def make_executable(path: Path) -> None:
    if os.name != "nt":
        path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def metadata(root: Path, profile: str, target: str, windows: bool) -> dict:
    plugin = json.loads((root / "plugins/opencode-made/package.json").read_text(encoding="utf-8"))
    data = {
        "name": "made",
        "version": workspace_version(root),
        "profile": profile,
        "target": target,
        "runner_os": os.environ.get("RUNNER_OS"),
        "runner_arch": os.environ.get("RUNNER_ARCH"),
        "commit": os.environ.get("GITHUB_SHA"),
        "ref": os.environ.get("GITHUB_REF_NAME"),
        "rustc": tool_version(["rustc", "--version"]),
        "cargo": tool_version(["cargo", "--version"]),
        "plugin": {"name": plugin.get("name"), "version": plugin.get("version")},
        "signing": "none",
        "notarization": "none",
        "companions": {
            "layout": "remote/<target>/made-remote",
            "targets": COMPANION_TARGETS,
            "askpass": "made-askpass.exe" if windows else "made-askpass",
            "runtime_download": "none",
        },
    }
    if profile == "release":
        data["installer"] = "velopack"
        data["auto_update"] = {"source": "github-releases", "repository": UPDATE_REPO}
    else:
        data["installer"] = "none"
        data["auto_update"] = "none"
    return data


def stage(args: argparse.Namespace, root: Path = ROOT) -> Path:
    windows = args.target.endswith("-windows-msvc")
    exe = ".exe" if windows else ""
    out: Path = args.stage
    if out.exists():
        fail(f"refusing to overwrite an existing stage: {out}")
    if not args.app.is_file() or args.app.stat().st_size <= 0:
        fail(f"made-app binary missing or empty: {args.app}")
    out.mkdir(parents=True)
    app = out / f"made-app{exe}"
    shutil.copy2(args.app, app)
    make_executable(app)

    extras = ["README.md", "LICENSE"]
    if args.profile == "debug":
        extras += ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]
    for name in extras:
        shutil.copy2(root / name, out / name)

    # Release AppPaths resolves the app-owned plugin at
    # <install_root>/plugins/opencode-made, never the obsolete singular directory.
    shutil.copytree(
        root / "plugins/opencode-made",
        out / "plugins/opencode-made",
        ignore=shutil.ignore_patterns("node_modules"),
    )
    if not (out / "plugins/opencode-made/package.json").is_file():
        fail("plugins/opencode-made/package.json was not staged")

    # The shipped grok-made package is staged only by the shared script, so the
    # local copy and the remote copies are byte-for-byte the same package.
    bash = shutil.which("bash") or "bash"
    subprocess.run(
        [bash, str(root / ".github/scripts/stage-grok-plugin.sh"), args.target, str(args.hook),
         str(out / "plugins/grok-made")],
        check=True,
    )
    hook = out / "plugins/grok-made/hooks/bin" / f"made-grok-status{exe}"
    for required in ("plugin.json", "hooks/hooks.json"):
        if not (out / "plugins/grok-made" / required).is_file():
            fail(f"plugins/grok-made/{required} was not staged")
    if not hook.is_file() or hook.stat().st_size <= 0:
        fail(f"native hook missing or empty: {hook}")
    if any((out / "plugins/grok-made").rglob("*.py")):
        fail("plugins/grok-made must not contain a Python file")
    if (out / "plugin").exists():
        fail("stage contains the obsolete plugin/ path")

    (out / "version.json").write_text(
        json.dumps(metadata(root, args.profile, args.target, windows), indent=2) + "\n", encoding="utf-8"
    )
    for path in sorted(p for p in out.rglob("*") if p.is_file()):
        print(path.relative_to(out).as_posix())
    return out


def parse(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--profile", choices=["debug", "release"], required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--hook", type=Path, required=True)
    parser.add_argument("--stage", type=Path, required=True)
    return parser.parse_args(argv)


if __name__ == "__main__":
    stage(parse())
