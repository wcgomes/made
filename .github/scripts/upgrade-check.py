#!/usr/bin/env python3
"""Upgrade an installed previous release through the in-app path (REL-12).

Usage: upgrade-check.py --form setup|pkg|zip|appimage --assets DIR
                        --previous X.Y.Z --latest X.Y.Z --automatic on|off

Installs the previous release from DIR per-user (as smoke-installed.py does),
seeds app data (an SSH server record and the REL-10 choice), runs the
installed build's `--update` (check and download only), then
`--update --apply` with no Made window open, and fails unless the installed
build then reports the latest version, the seeded app data is byte-for-byte
unchanged, the REL-03 layout is complete and the smoke line shows the
production update location.
"""

from __future__ import annotations

import argparse
import getpass
import hashlib
import importlib.util
import json
import os
import platform
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("smoke_installed", HERE / "smoke-installed.py")
smoke = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(smoke)


def data_dir() -> Path:
    home = Path.home()
    if sys.platform == "win32":
        return Path(os.environ["APPDATA"]) / "Made"
    if sys.platform == "darwin":
        return home / "Library" / "Application Support" / "Made"
    return Path(os.environ.get("XDG_DATA_HOME") or home / ".local" / "share") / "made"


def make_private(path: Path, directory: bool) -> None:
    if sys.platform == "win32":
        user = getpass.getuser()
        subprocess.run(["icacls", str(path), "/inheritance:r", "/grant:r", f"{user}:(OI)(CI)F" if directory else f"{user}:F"],
                       check=True, capture_output=True)
    else:
        path.chmod(0o700 if directory else 0o600)


def seed(automatic: bool) -> dict[str, str]:
    directory = data_dir()
    directory.mkdir(parents=True, exist_ok=True)
    make_private(directory, True)
    files = {
        "preferences.json": {"schema": 1, "automatic_update_checks": automatic},
        "ssh-servers.json": {
            "schema_version": 1,
            "servers": [
                {
                    "id": list(range(1, 17)),
                    "metadata": {
                        "display_name": "Upgrade check server",
                        "host": "upgrade-check.invalid",
                        "user": "made",
                        "port": 2222,
                        "key_file": None,
                        "initial_directory": None,
                    },
                    "consent": None,
                    "grok_consent": None,
                }
            ],
        },
    }
    digests = {}
    for name, document in files.items():
        path = directory / name
        path.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
        make_private(path, False)
        digests[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return digests


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):  # noqa: ANN002, ANN003 - urllib signature
        return None


def channel(form: str) -> str:
    if form == "setup":
        return "win-x64"
    if form == "appimage":
        return "linux-x64"
    return "osx-arm64" if platform.machine() == "arm64" else "osx-x64"


def wait_for_latest(form: str, latest: str, timeout: float = 600) -> None:
    """Bounded wait until GitHub's `latest` feed redirect names the new tag.

    Right after publishing, `releases/latest/download/` can still redirect
    to the previous release for a short while (observed for ~20 s), so a
    check started immediately would correctly find nothing newer.
    """
    url = f"https://github.com/wcgomes/made/releases/latest/download/releases.{channel(form)}.json"
    expected = f"/releases/download/v{latest}/"
    opener = urllib.request.build_opener(_NoRedirect)
    deadline = time.time() + timeout
    location = ""
    while time.time() < deadline:
        try:
            opener.open(urllib.request.Request(url, method="HEAD"), timeout=30)
        except urllib.error.HTTPError as error:
            location = error.headers.get("Location", "")
            if expected in location:
                print(f"latest feed now resolves to v{latest}", flush=True)
                return
        except urllib.error.URLError as error:
            location = str(error)
        time.sleep(10)
    smoke.check(False, f"GitHub's latest feed resolves to v{latest} (last: {location})")


def wait_for_exit(pid: int, timeout: float = 600) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        listing = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], capture_output=True, text=True).stdout
        if str(pid) not in listing:
            return
        time.sleep(2)
    smoke.check(False, f"the apply helper {pid} finished within {timeout:.0f}s")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--form", required=True, choices=["setup", "pkg", "zip", "appimage"])
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--previous", required=True)
    parser.add_argument("--latest", required=True)
    parser.add_argument("--automatic", required=True, choices=["on", "off"])
    args = parser.parse_args()

    command, _, _ = smoke.install(args.form, args.assets, args.previous)
    version = smoke.run(command + ["--version"])
    smoke.check(version.stdout.strip() == f"Made {args.previous}", f"the previous release {args.previous} is installed")
    digests = seed(args.automatic == "on")
    wait_for_latest(args.form, args.latest)

    download = smoke.run(command + ["--update"], timeout=900)
    smoke.check(download.returncode == 0, "the command-line entry checks and downloads")
    smoke.check(f"Made {args.latest} is ready to install." in download.stdout, f"{args.latest} is downloaded and verified")
    after_download = smoke.run(command + ["--version"])
    smoke.check(after_download.stdout.strip() == f"Made {args.previous}", "a download alone changes nothing installed")

    apply = smoke.run(command + ["--update", "--apply"], timeout=900)
    smoke.check(apply.returncode == 0, "the command-line apply succeeds with no Made window open")
    helper = re.search(r"helper process (\d+)", apply.stdout)
    if helper:
        wait_for_exit(int(helper.group(1)))

    updated = smoke.run(command + ["--version"])
    smoke.check(updated.stdout.strip() == f"Made {args.latest}", f"the installed build now reports {args.latest}")
    for name, digest in digests.items():
        path = data_dir() / name
        smoke.check(path.is_file() and hashlib.sha256(path.read_bytes()).hexdigest() == digest, f"{name} is preserved")

    # The updated install must be the complete REL-03 layout.
    if args.form == "appimage":
        image = Path(command[0])
        extract = Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "made-upgraded"
        extract.mkdir(parents=True, exist_ok=True)
        smoke.run([image, "--appimage-extract"], cwd=extract)
        bin_dir = extract / "squashfs-root" / "usr" / "bin"
        smoke.check_layout(bin_dir, bin_dir)
    elif args.form in ("pkg", "zip"):
        exe = Path(command[0])
        smoke.check_layout(exe.parent, exe.parent.parent / "Resources")
    else:
        exe = Path(command[0])
        smoke.check_layout(exe.parent, exe.parent)
    smoke.smoke(command, args.latest)

    if smoke.FAILURES:
        print(f"{len(smoke.FAILURES)} upgrade check(s) failed:", file=sys.stderr)
        for failure in smoke.FAILURES:
            print(f"  - {failure}", file=sys.stderr)
        return 1
    print(f"upgraded {args.previous} -> {args.latest} with app data preserved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
