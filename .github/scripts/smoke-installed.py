#!/usr/bin/env python3
"""Install one packaged target per-user and smoke-test the installed layout (REL-03).

Usage: smoke-installed.py --form setup|pkg|zip|appimage --assets DIR --version X.Y.Z

Installs the end-user download from DIR the way a user does (Windows setup,
macOS `.pkg` for the current user or `.zip` into ~/Applications, Linux
AppImage marked executable), then fails on any miss:

- `--version` reports the release version;
- `--smoke-test` passes and its markers report the installed OpenCode plugin,
  a present `made-grok-status`, all four companions, askpass, updates disabled,
  `update_repo=wcgomes/made` and no test-only update seam;
- the four `remote/<target>` trees match `remote/manifest.json` hashes;
- `plugins/grok-made` holds an executable `made-grok-status` and no Python file;
- executables keep their executable bit on Unix.

The Linux smoke runs under `xvfb-run` with software rendering.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPOSITORY = "wcgomes/made"
TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin")
FAILURES: list[str] = []


def check(condition: bool, message: str) -> None:
    print(f"{'ok  ' if condition else 'FAIL'} {message}", flush=True)
    if not condition:
        FAILURES.append(message)


def run(cmd: list, timeout: int = 600, **kwargs) -> subprocess.CompletedProcess:
    print("$ " + " ".join(str(c) for c in cmd), flush=True)
    result = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, timeout=timeout, **kwargs)
    if result.stdout:
        print(result.stdout[-8000:], flush=True)
    if result.stderr:
        print(result.stderr[-4000:], file=sys.stderr, flush=True)
    return result


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def one(assets: Path, pattern: str) -> Path:
    found = sorted(assets.glob(pattern))
    if len(found) != 1:
        raise SystemExit(f"expected one {pattern} in {assets}, found {[p.name for p in found]}")
    return found[0]


def install(form: str, assets: Path, version: str) -> tuple[list[str], Path, Path]:
    """Install; return (command to run Made, executable dir, install root)."""
    home = Path.home()
    if form == "setup":
        setup = one(assets, f"made-{version}-windows-x64-setup.exe")
        result = run([setup, "--silent"])
        check(result.returncode == 0, "the Windows setup installs for the current user")
        # Setup starts Made once after installing; close that first-run window.
        time.sleep(8)
        run(["taskkill", "/F", "/T", "/IM", "made-app.exe"])
        root = Path(os.environ["LOCALAPPDATA"]) / "Made"
        exe = root / "current" / "made-app.exe"
        start_menu = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs"
        check(any(p.name == "Made.lnk" for p in start_menu.glob("*.lnk")), "a Start menu shortcut exists")
        uninstall = run(["reg", "query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Made"])
        check(uninstall.returncode == 0, "Made is listed in the current user's installed apps")
        return [str(exe)], exe.parent, exe.parent
    if form in ("pkg", "zip"):
        apps = home / "Applications"
        apps.mkdir(exist_ok=True)
        app = apps / "Made.app"
        shutil.rmtree(app, ignore_errors=True)
        if form == "pkg":
            pkg = one(assets, f"made-{version}-macos-*.pkg")
            result = run(["installer", "-pkg", pkg, "-target", "CurrentUserHomeDirectory"])
            check(result.returncode == 0, "the .pkg installs for the current user without administrator rights")
            # The package starts Made once after installing; close that window.
            time.sleep(8)
            run(["pkill", "-x", "made-app"])
        else:
            archive = one(assets, f"made-{version}-macos-*.zip")
            result = run(["ditto", "-x", "-k", archive, apps])
            check(result.returncode == 0, "the .zip extracts into ~/Applications")
        check(app.is_dir(), "Made.app is installed in ~/Applications")
        verify = run(["codesign", "--verify", "--deep", "--strict", app])
        check(verify.returncode == 0, "codesign --verify accepts the installed bundle")
        exe = app / "Contents" / "MacOS" / "made-app"
        return [str(exe)], exe.parent, app / "Contents" / "Resources"
    if form == "appimage":
        image = one(assets, f"made-{version}-linux-x64.AppImage")
        apps = home / "Applications"
        apps.mkdir(exist_ok=True)
        installed = apps / image.name
        shutil.copy2(image, installed)
        installed.chmod(installed.stat().st_mode | stat.S_IXUSR)
        extract = Path(tempfile.mkdtemp(prefix="made-appimage-"))
        result = run([installed, "--appimage-extract"], cwd=extract)
        check(result.returncode == 0, "the AppImage contents can be listed")
        bin_dir = extract / "squashfs-root" / "usr" / "bin"
        return [str(installed)], bin_dir, bin_dir
    raise SystemExit(f"unknown form {form}")


def check_layout(exe_dir: Path, install_root: Path) -> None:
    hook = "made-grok-status.exe" if os.name == "nt" else "made-grok-status"
    askpass = "made-askpass.exe" if os.name == "nt" else "made-askpass"
    check((install_root / "plugins/opencode-made/package.json").is_file(), "plugins/opencode-made/package.json is installed")
    check(not (install_root / "plugins/opencode-made/node_modules").exists(), "no node_modules is shipped")
    grok = install_root / "plugins/grok-made"
    check((grok / "hooks/bin" / hook).is_file(), "plugins/grok-made carries made-grok-status")
    check(not list(grok.rglob("*.py")), "plugins/grok-made contains no Python file")
    check((install_root / "LICENSE").is_file(), "LICENSE is installed")
    check((exe_dir / askpass).is_file(), "made-askpass sits beside made-app")
    manifest_path = exe_dir / "remote" / "manifest.json"
    check(manifest_path.is_file(), "remote/manifest.json is installed")
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        for target in TARGETS:
            records = manifest["targets"][target]["files"]
            check(len(records) == 10, f"{target} lists the 10-artifact companion set")
            for record in records:
                path = exe_dir / "remote" / target / record["name"]
                check(path.is_file() and sha256(path) == record["sha256"], f"{target}/{record['name']} matches the manifest")
    if os.name != "nt":
        executables = [exe_dir / "made-app", exe_dir / askpass, grok / "hooks/bin" / hook]
        executables += [exe_dir / "remote" / t / "made-remote" for t in TARGETS]
        executables += [exe_dir / "remote" / t / "plugins/grok-made/hooks/bin/made-grok-status" for t in TARGETS]
        for path in executables:
            check(path.is_file() and bool(path.stat().st_mode & stat.S_IXUSR), f"{path.name} in {path.parent.name} is executable")


def check_extraction_guard(image: Path, version: str) -> None:
    """Security M2: the AppImage extracts only into a private per-user directory.

    Runs the installed AppImage normally, then with the extraction target
    pre-planted as a directory owned by another user, a directory with group
    and other permission bits, and a symlink; each planted case must refuse.
    An interrupted extraction (a file cut short, or most files missing) is
    never run: Made extracts afresh and starts.
    """
    envs = run([image, "--appimage-envs"]).stdout.splitlines()
    setting = next((line for line in envs if line.startswith("APPIMAGE_TARGET_DIR=")), "")
    target = Path(setting.partition("=")[2].replace("${HOME}", str(Path.home())))
    check(target.parent == Path.home() / ".cache" / "made"
          and target.name.startswith(f"appimage-{version}-") and len(target.name) == len(f"appimage-{version}-") + 16,
          f"the extraction target is per user, version and build: {target}")
    real = target.with_name(f"{target.name}-real")
    for path in (target, real):
        run(["sudo", "rm", "-rf", path])
    keep = dict(os.environ, NO_CLEANUP="1")
    result = run([image, "--version"], env=keep)
    check(result.returncode == 0 and result.stdout.strip() == f"Made {version}", "the AppImage starts from its private target")
    check(target.is_dir() and not target.is_symlink(), f"extraction goes to {target}")
    if target.is_dir():
        info = target.stat()
        check(info.st_uid == os.getuid() and info.st_mode & 0o077 == 0, "the extraction directory is owned by the user with mode 0700")
    check(not list(Path(tempfile.gettempdir()).glob("appimage_extracted_*")), "nothing is extracted under the shared temp directory")
    if not target.is_dir():
        return
    target.rename(real)

    def refused(label: str) -> None:
        result = run([image, "--version"])
        check(result.returncode != 0 and f"Made {version}" not in result.stdout, f"a pre-planted target ({label}) is refused")

    run(["sudo", "cp", "-a", real, target])
    run(["sudo", "chown", "-R", "root:root", target])
    run(["sudo", "chmod", "755", target])
    refused("owned by another user")
    run(["sudo", "rm", "-rf", target])
    shutil.copytree(real, target, symlinks=True)
    target.chmod(0o777)
    refused("group and other writable")
    shutil.rmtree(target)
    target.symlink_to(real)
    refused("a symlink")
    target.unlink()

    def recovers(label: str) -> None:
        result = run([image, "--version"])
        check(result.returncode == 0 and result.stdout.strip() == f"Made {version}",
              f"an interrupted extraction ({label}) is replaced and Made starts")

    shutil.copytree(real, target, symlinks=True)
    largest = max((path for path in target.rglob("*") if path.is_file()), key=lambda path: path.stat().st_size)
    with largest.open("r+b") as handle:
        handle.truncate(largest.stat().st_size // 2)
    recovers("a file cut short")
    shutil.rmtree(target, ignore_errors=True)
    target.mkdir(mode=0o700)
    shutil.copy2(real / "AppRun", target / "AppRun")
    recovers("most files missing")
    shutil.rmtree(target, ignore_errors=True)
    shutil.rmtree(real)
    result = run([image, "--version"])
    check(result.returncode == 0 and result.stdout.strip() == f"Made {version}", "the AppImage starts again once the planted target is gone")


def smoke(command: list[str], version: str) -> None:
    result = run(command + ["--version"])
    check(result.returncode == 0 and result.stdout.strip() == f"Made {version}", f"--version reports Made {version}")
    env = dict(os.environ, MADE_NATIVE_BACKDROP="off")
    smoke_cmd = command + ["--smoke-test", "3000"]
    if sys.platform.startswith("linux"):
        env.pop("WAYLAND_DISPLAY", None)
        env["WINIT_UNIX_BACKEND"] = "x11"
        smoke_cmd = ["xvfb-run", "-a", "-s", "-screen 0 1280x800x24"] + smoke_cmd
    result = run(smoke_cmd, timeout=300, env=env)
    lines = [line for line in result.stdout.splitlines() if line.startswith("made smoke:")]
    check(result.returncode == 0 and "made smoke: OK" in lines, "the installed smoke test passes")
    marker = next((line for line in lines if "window_ready=1" in line), "")
    for expected in (
        f"version={version}",
        "opencode_plugin=installed",
        "grok_status=present",
        "companions=4/4",
        "askpass=found",
        "updates=disabled",
        f"update_repo={REPOSITORY}",
        "update_seam=0",
    ):
        check(f" {expected}" in f" {marker}", f"the smoke line reports {expected}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--form", required=True, choices=["setup", "pkg", "zip", "appimage"])
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    command, exe_dir, install_root = install(args.form, args.assets, args.version)
    check_layout(exe_dir, install_root)
    if args.form == "appimage":
        check_extraction_guard(Path(command[0]), args.version)
    smoke(command, args.version)
    if FAILURES:
        print(f"{len(FAILURES)} installed-layout check(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print("installed layout passed every check")
    return 0


if __name__ == "__main__":
    sys.exit(main())
