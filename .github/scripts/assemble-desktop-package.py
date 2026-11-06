#!/usr/bin/env python3
"""Assemble four remote companions, the hash manifest, and native askpass.

Missing, empty, oversized, or hash-mismatched artifacts fail this process.
Nothing here downloads a companion. Callers must already have the build
artifacts on disk.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import stat
import sys
from pathlib import Path

TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
)
FILES = (
    "made-remote",
    "plugins/opencode-made/package.json",
    "plugins/opencode-made/tui.ts",
    "plugins/opencode-made/index.ts",
    "plugins/opencode-made/src/protocol.ts",
    "plugins/opencode-made/src/integration.ts",
    "plugins/grok-made/plugin.json",
    "plugins/grok-made/README.md",
    "plugins/grok-made/hooks/hooks.json",
    "plugins/grok-made/hooks/bin/made-grok-status",
)
MAX_BYTES = 128 * 1024 * 1024


def fail(message: str) -> None:
    print(f"packaging failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def child(root: Path, name: str) -> Path:
    path = root
    for part in name.split("/"):
        if part in ("", ".", ".."):
            fail(f"refusing unsafe artifact path {name}")
        path = path / part
    return path


def load_json(path: Path) -> dict:
    if not path.is_file() or path.is_symlink():
        fail(f"manifest missing: {path}")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        fail(f"manifest is not JSON: {path}: {error}")
    if not isinstance(data, dict):
        fail(f"manifest is not an object: {path}")
    return data


def verify_file(root: Path, record: dict) -> dict:
    if not isinstance(record, dict):
        fail(f"bad artifact record under {root}")
    name = record.get("name")
    if not isinstance(name, str) or not name:
        fail(f"artifact record missing a name under {root}")
    path = child(root, name)
    if not path.is_file() or path.is_symlink():
        fail(f"artifact missing: {name}")
    data = path.read_bytes()
    size = len(data)
    digest = hashlib.sha256(data).hexdigest()
    expected_size = record.get("size")
    expected_hash = record.get("sha256")
    if size <= 0 or size > MAX_BYTES:
        fail(f"artifact empty or over 128 MiB: {name} ({size} bytes)")
    if expected_size != size:
        fail(f"size mismatch for {name}: manifest {expected_size}, file {size}")
    if not isinstance(expected_hash, str) or expected_hash != digest or len(digest) != 64:
        fail(f"hash mismatch for {name}")
    return {"name": name, "size": size, "sha256": digest}


def verify_companion(directory: Path, target: str) -> dict:
    if not directory.is_dir():
        fail(f"companion artifact missing for {target}")
    manifest = load_json(directory / "manifest.json")
    if manifest.get("schema") != 1 or manifest.get("target") != target:
        fail(f"companion manifest target/schema mismatch for {target}")
    if manifest.get("profile") != "release":
        fail(f"companion profile must be release for {target}")
    linkage = manifest.get("linkage")
    if not isinstance(linkage, dict):
        fail(f"companion linkage missing for {target}")
    if target.endswith("-unknown-linux-gnu"):
        if linkage.get("kind") != "glibc" or linkage.get("system") != "2.31" or linkage.get("max_symbol") != "2.31":
            fail(f"Linux companion for {target} is not pinned to glibc 2.31")
    elif target.endswith("-apple-darwin"):
        if (
            linkage.get("kind") != "macos"
            or linkage.get("deployment_target") != "12.0"
            or linkage.get("minos") != "12.0"
        ):
            fail(f"macOS companion for {target} is not pinned to deployment target 12.0")
    else:
        fail(f"unsupported companion target {target}")
    records = manifest.get("files")
    if not isinstance(records, list) or [item.get("name") for item in records] != list(FILES):
        fail(f"companion manifest files do not match the stage allowlist for {target}")
    verified = [verify_file(directory, record) for record in records]
    sums = directory / "SHA256SUMS"
    if not sums.is_file():
        fail(f"SHA256SUMS missing for {target}")
    lines = [line for line in sums.read_text(encoding="utf-8").splitlines() if line]
    expected_lines = [f"{item['sha256']}  {item['name']}" for item in verified]
    if lines != expected_lines:
        fail(f"SHA256SUMS diverges from manifest for {target}")
    return {"linkage": linkage, "files": verified}


def copy_tree(source: Path, destination: Path) -> None:
    if destination.exists():
        fail(f"refusing to overwrite {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination, symlinks=False)
    for path in destination.rglob("*"):
        if path.is_symlink():
            fail(f"refusing a symlinked companion file: {path}")
    made = destination / "made-remote"
    if not made.is_file() or made.stat().st_size <= 0:
        fail(f"copied companion is missing: {made}")
    made.chmod(made.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    hook = destination / "plugins/grok-made/hooks/bin/made-grok-status"
    if not hook.is_file() or hook.stat().st_size <= 0:
        fail(f"copied native hook is missing or empty: {hook}")
    # Workflow artifact download does not preserve modes, so the executable bit
    # is restored here exactly like the companion's.
    hook.chmod(hook.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def verify_askpass(path: Path, manifest_path: Path, expected_target: str) -> dict:
    if not path.is_file() or path.is_symlink() or path.stat().st_size <= 0:
        fail(f"native askpass missing or empty: {path}")
    manifest = load_json(manifest_path)
    if manifest.get("schema") != 1 or manifest.get("kind") != "askpass":
        fail("askpass manifest is not a schema-1 askpass record")
    if manifest.get("name") != path.name:
        fail(f"askpass name mismatch: manifest {manifest.get('name')}, file {path.name}")
    if manifest.get("target") != expected_target:
        fail(f"askpass target {manifest.get('target')} does not match desktop {expected_target}")
    digest = sha256(path)
    size = path.stat().st_size
    if manifest.get("size") != size or manifest.get("sha256") != digest:
        fail("askpass hash or size does not match its manifest")
    return {"name": path.name, "size": size, "sha256": digest}


def rehash_installed(stage: Path, target: str, expected: list[dict]) -> None:
    root = stage / "remote" / target
    for record in expected:
        path = child(root, record["name"])
        if sha256(path) != record["sha256"] or path.stat().st_size != record["size"]:
            fail(f"copied artifact diverged for {target}/{record['name']}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stage", type=Path, required=True)
    parser.add_argument("--companions", type=Path, required=True)
    parser.add_argument("--askpass", type=Path, required=True)
    parser.add_argument("--askpass-manifest", type=Path, required=True)
    parser.add_argument("--desktop-target", required=True)
    args = parser.parse_args()

    stage = args.stage
    if not stage.is_dir():
        fail(f"desktop stage does not exist: {stage}")
    app = stage / "made-app"
    app_exe = stage / "made-app.exe"
    if not app.is_file() and not app_exe.is_file():
        fail("desktop stage is missing made-app; refusing to replace the development artifact")
    plugin = stage / "plugins" / "opencode-made" / "package.json"
    if not plugin.is_file() or plugin.stat().st_size <= 0:
        fail("desktop stage is missing plugins/opencode-made")
    grok_plugin = stage / "plugins" / "grok-made" / "plugin.json"
    if not grok_plugin.is_file() or grok_plugin.stat().st_size <= 0:
        fail("desktop stage is missing plugins/grok-made")
    if (stage / "plugin").exists():
        fail("desktop stage contains the obsolete plugin/ path")

    verified = {target: verify_companion(args.companions / target, target) for target in TARGETS}
    askpass = verify_askpass(args.askpass, args.askpass_manifest, args.desktop_target)

    remote = stage / "remote"
    if remote.exists():
        fail(f"refusing to overwrite an existing remote directory: {remote}")
    remote.mkdir()
    for target in TARGETS:
        copy_tree(args.companions / target, remote / target)
        # The per-target manifest and checksum sidecar stay out of the
        # directory the app reads. The package manifest is remote/manifest.json.
        for extra in ("manifest.json", "SHA256SUMS"):
            leftover = remote / target / extra
            if leftover.exists():
                leftover.unlink()
        rehash_installed(stage, target, verified[target]["files"])

    destination_askpass = stage / askpass["name"]
    if destination_askpass.exists():
        fail(f"refusing to overwrite {destination_askpass}")
    shutil.copy2(args.askpass, destination_askpass)
    if not askpass["name"].endswith(".exe"):
        destination_askpass.chmod(destination_askpass.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    if sha256(destination_askpass) != askpass["sha256"]:
        fail("copied askpass hash diverged")

    package_manifest = {
        "schema": 1,
        "kind": "ssh-remote-companions",
        "profile": "release",
        "runtime_download": "none",
        "targets": verified,
        "askpass": askpass,
    }
    manifest_path = remote / "manifest.json"
    manifest_path.write_text(json.dumps(package_manifest, indent=2) + "\n", encoding="utf-8")
    reread = load_json(manifest_path)
    if reread != package_manifest:
        fail("written hash manifest does not round-trip")
    for target in TARGETS:
        rehash_installed(stage, target, reread["targets"][target]["files"])
    if sha256(destination_askpass) != reread["askpass"]["sha256"]:
        fail("askpass diverged from the written manifest")
    if not plugin.is_file():
        fail("assembly removed the desktop plugin")
    if not grok_plugin.is_file():
        fail("assembly removed the shipped grok-made plugin")
    print(f"assembled companions and {askpass['name']} into {stage}")


if __name__ == "__main__":
    main()
