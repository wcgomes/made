#!/usr/bin/env python3
"""Velopack probe harness (add-release-auto-update tasks 1.3, 1.4, 1.5, 1.7).

Runs on one CI runner. It packs the probe binary at two versions with vpk,
installs v1 per-user, updates it from a local feed, kills the updater at
several points, inspects Velopack's own binaries, and writes every observation
to a JSON results file. It never fails on an observation: the results are
read and recorded by task 1.8. Removed by task 10.3 with the probe workflow.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import stat
import subprocess
import sys
import time
import zipfile
from pathlib import Path

PACK_ID = "Made"
V1, V2 = "0.0.1", "0.0.2"
PORT = 8765
RESULTS: dict = {"checks": {}}


def log(message: str) -> None:
    print(f"probe.py: {message}", flush=True)


def record(name: str, ok: bool | None, **detail) -> None:
    RESULTS["checks"][name] = {"ok": ok, **detail}
    log(f"{'PASS' if ok else 'FAIL' if ok is False else 'INFO'} {name}: {json.dumps(detail)[:600]}")


def run(cmd: list, check: bool = False, timeout: int = 600, **kwargs) -> subprocess.CompletedProcess:
    log("$ " + " ".join(str(part) for part in cmd))
    try:
        result = subprocess.run(
            [str(part) for part in cmd], capture_output=True, text=True, timeout=timeout, **kwargs
        )
    except subprocess.TimeoutExpired as error:
        log(f"timeout: {error}")
        return subprocess.CompletedProcess(cmd, 124, "", "timeout")
    except OSError as error:
        log(f"cannot run: {error}")
        return subprocess.CompletedProcess(cmd, 127, "", str(error))
    if result.stdout:
        print(result.stdout[-6000:], flush=True)
    if result.stderr:
        print(result.stderr[-4000:], file=sys.stderr, flush=True)
    if check and result.returncode != 0:
        raise SystemExit(f"command failed ({result.returncode}): {cmd}")
    return result


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def make_exec(path: Path) -> None:
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def stage(work: Path, os_name: str, exe: Path, version: str) -> Path:
    """Stage one pack input mirroring the REL-03 layout."""
    suffix = ".exe" if os_name == "windows" else ""
    if os_name == "macos":
        bundle = work / f"stage-{version}" / "Made.app"
        bin_dir = bundle / "Contents" / "MacOS"
        res_dir = bundle / "Contents" / "Resources"
        bin_dir.mkdir(parents=True)
        res_dir.mkdir(parents=True)
        (bundle / "Contents" / "Info.plist").write_text(
            f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>made-app</string>
<key>CFBundleIdentifier</key><string>io.github.wcgomes.made.probe</string>
<key>CFBundleName</key><string>Made</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
""",
            encoding="utf-8",
        )
        plugin_root = res_dir
        root = bundle
    else:
        bin_dir = work / f"stage-{version}"
        bin_dir.mkdir(parents=True)
        plugin_root = bin_dir
        root = bin_dir
    shutil.copy2(exe, bin_dir / f"made-app{suffix}")
    shutil.copy2(exe, bin_dir / f"made-askpass{suffix}")
    companion = bin_dir / "remote" / "x86_64-unknown-linux-gnu" / "made-remote"
    companion.parent.mkdir(parents=True)
    companion.write_text("#!/bin/sh\necho companion\n", encoding="utf-8")
    plugin = plugin_root / "plugins" / "opencode-made"
    plugin.mkdir(parents=True)
    (plugin / "package.json").write_text('{"name":"opencode-made","version":"%s"}\n' % version)
    (plugin_root / "LICENSE").write_text("MIT probe\n")
    if version == V2:
        # Large enough that extraction takes long enough to kill mid-copy.
        (bin_dir / "payload.bin").write_bytes(os.urandom(48 * 1024 * 1024))
    for path in [bin_dir / f"made-app{suffix}", bin_dir / f"made-askpass{suffix}", companion]:
        make_exec(path)
    return root


def vpk_pack(stage_root: Path, version: str, channel: str, out: Path, os_name: str) -> subprocess.CompletedProcess:
    main = "made-app.exe" if os_name == "windows" else "made-app"
    cmd = [
        "vpk", "pack", "--packId", PACK_ID, "--packVersion", version, "--packDir", stage_root,
        "--mainExe", main, "--channel", channel, "--outputDir", out, "--packTitle", "Made",
        "--delta", "None", "--yes",
    ]
    if os_name == "macos":
        # 1.3 signature fallback: ad-hoc identity, no certificate.
        cmd += ["--signAppIdentity", "-"]
    return run(cmd, timeout=1200)


def listing(directory: Path) -> list[str]:
    return sorted(str(p.relative_to(directory)).replace("\\", "/") for p in directory.rglob("*") if p.is_file())


def serve(directory: Path) -> subprocess.Popen:
    log(f"serving {directory} on {PORT}")
    process = subprocess.Popen(
        [sys.executable, "-m", "http.server", str(PORT), "--bind", "127.0.0.1"],
        cwd=directory, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(2)
    return process


def launches() -> list[str]:
    path = Path.home() / "made-probe-launch.txt"
    return path.read_text().splitlines() if path.exists() else []


def wait_for_launch(version: str, before: int, timeout: float = 90) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        lines = launches()[before:]
        if any(f"version={version}" in line for line in lines):
            return True
        time.sleep(1)
    return False


def report(exe: Path) -> dict:
    result = run([exe, "report"], timeout=120)
    try:
        start = result.stdout.index("{")
        data = json.loads(result.stdout[start:])
    except ValueError:
        data = {}
    data["exit_code"] = result.returncode
    return data


def updater_names(os_name: str) -> list[str]:
    return {"linux": ["UpdateNix"], "macos": ["UpdateMac"], "windows": ["Update.exe"]}[os_name]


def kill_updater(os_name: str) -> str:
    outputs = []
    for name in updater_names(os_name):
        if os_name == "windows":
            outputs.append(run(["taskkill", "/F", "/IM", name]).stdout.strip())
        else:
            outputs.append(str(run(["pkill", "-9", "-x", name]).returncode))
    return ";".join(outputs)


def updater_running(os_name: str) -> bool:
    for name in updater_names(os_name):
        if os_name == "windows":
            out = subprocess.run(["tasklist", "/FI", f"IMAGENAME eq {name}"], capture_output=True, text=True).stdout
            if name.lower() in out.lower():
                return True
        elif subprocess.run(["pgrep", "-x", name], capture_output=True).returncode == 0:
            return True
    return False


def kill_points(os_name: str, reset, exe_of, feed: str) -> None:
    results = []
    for delay in (0.0, 0.02, 0.05, 0.1, 0.2, 0.4, 0.8):
        reset()
        exe = exe_of()
        before = report(exe)
        process = subprocess.Popen([str(exe), "update", feed, "--apply-norestart"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        seen = False
        deadline = time.time() + 120
        while time.time() < deadline and not seen:
            seen = updater_running(os_name)
            if not seen:
                time.sleep(0.005)
        time.sleep(delay)
        alive = updater_running(os_name)
        killed = kill_updater(os_name) if alive else "already exited"
        process.wait(timeout=120)
        time.sleep(2)
        if updater_running(os_name):
            kill_updater(os_name)
        after = report(exe_of())
        launchable = after.get("exit_code") == 0 and after.get("version") in (V1, V2)
        results.append({
            "delay_s": delay, "seen": seen, "alive_at_kill": alive, "before": before.get("version"),
            "after": after.get("version"), "exit_code": after.get("exit_code"), "killed": killed,
            "launchable": launchable,
        })
    record("1.7 kill points leave a launchable version", all(r["launchable"] for r in results), points=results)


def prerelease_feed(out2: Path, channel: str, work: Path) -> Path:
    pre = work / "feed-pre"
    shutil.copytree(out2, pre)
    feed_path = pre / f"releases.{channel}.json"
    feed = json.loads(feed_path.read_text())
    assets = feed.get("Assets") or feed.get("assets")
    fake = dict(assets[0])
    fake["Version"] = "9.0.0-beta.1"
    fake["FileName"] = f"{PACK_ID}-9.0.0-beta.1-{channel}-full.nupkg"
    assets.append(fake)
    feed_path.write_text(json.dumps(feed))
    return pre


def renamed_feed(out2: Path, channel: str, work: Path, os_label: str) -> tuple[Path, dict]:
    renamed = work / "feed-renamed"
    shutil.copytree(out2, renamed)
    feed_text = (renamed / f"releases.{channel}.json").read_text()
    moves = {}
    for path in sorted(renamed.iterdir()):
        name = path.name
        if name.endswith(".nupkg") or name.startswith(("releases.", "assets.", "RELEASES")):
            continue
        new = f"made-{V2}-{os_label}-{name}"
        path.rename(renamed / new)
        moves[name] = {"to": new, "feed_referenced": name in feed_text}
    return renamed, moves


def inspect_windows(out1: Path, root: Path) -> None:
    vswhere = Path(os.environ.get("ProgramFiles(x86)", "")) / "Microsoft Visual Studio/Installer/vswhere.exe"
    install = run([vswhere, "-latest", "-property", "installationPath"]).stdout.strip()
    tools = sorted(Path(install).glob("VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe"))
    dumpbin = tools[-1] if tools else Path("dumpbin")
    kits = sorted(Path(os.environ.get("ProgramFiles(x86)", "")).glob("Windows Kits/10/bin/*/x64/mt.exe"))
    mt = kits[-1] if kits else Path("mt")
    found = {}
    for label, path in {
        "Setup.exe": next(out1.glob("*-Setup.exe"), None),
        "Update.exe": root / "Update.exe",
        "stub made-app.exe": root / "made-app.exe",
    }.items():
        if path is None or not path.exists():
            found[label] = "missing"
            continue
        deps = run([dumpbin, "/dependents", path]).stdout
        dlls = sorted({line.strip() for line in deps.splitlines() if line.strip().lower().endswith(".dll")})
        manifest = Path(os.environ.get("RUNNER_TEMP", ".")) / f"{label.split()[0]}.manifest"
        run([mt, "-nologo", f"-inputresource:{path};#1", f"-out:{manifest}"])
        level = None
        if manifest.exists():
            text = manifest.read_text(errors="replace")
            marker = 'level="'
            if marker in text:
                level = text.split(marker, 1)[1].split('"', 1)[0]
        found[label] = {"dlls": dlls, "requestedExecutionLevel": level}
    record("1.5 Windows Velopack binaries", None, binaries=found, install_root=str(root))


def inspect_macos(app: Path) -> None:
    update = app / "Contents" / "MacOS" / "UpdateMac"
    libs = run(["otool", "-L", update]).stdout
    loads = run(["otool", "-l", update]).stdout
    minos = [line.split()[-1] for line in loads.splitlines() if line.strip().startswith(("minos", "version"))]
    record("1.5 macOS UpdateMac", None, otool_L=libs.splitlines()[1:], minos=minos[:4])


def inspect_linux(appimage: Path, work: Path) -> None:
    extract = work / "appimage-extract"
    extract.mkdir(exist_ok=True)
    run([appimage, "--appimage-extract"], cwd=extract)
    root = extract / "squashfs-root"
    needed = {}
    for path in [root / "usr/bin/UpdateNix", root / "usr/bin/made-app"]:
        dyn = run(["readelf", "-d", path]).stdout
        needed[path.name] = [line.split("[", 1)[1].rstrip("]") for line in dyn.splitlines() if "(NEEDED)" in line]
    runtime = run(["file", appimage]).stdout.strip()
    record("1.5 Linux AppImage binaries", None, needed=needed, runtime=runtime,
           files=sorted(str(p.relative_to(root)) for p in root.rglob("*") if p.is_file())[:40])


def fuse_absent() -> dict:
    tools = {name: shutil.which(name) for name in ("fusermount", "fusermount3")}
    libs = run(["sh", "-c", "ldconfig -p | grep -i libfuse || true"]).stdout.strip()
    return {"tools": tools, "libfuse": libs}


def replace_runtime(appimage: Path, runtime: Path, out: Path) -> None:
    offset = int(run([appimage, "--appimage-offset"]).stdout.strip())
    with appimage.open("rb") as source, out.open("wb") as target:
        target.write(runtime.read_bytes())
        source.seek(offset)
        shutil.copyfileobj(source, target)
    make_exec(out)


def rewrite_nupkg(feed_dir: Path, channel: str, new_appimage: Path) -> None:
    """Swap the AppImage inside the full nupkg and refresh the feed digests."""
    feed_path = feed_dir / f"releases.{channel}.json"
    feed = json.loads(feed_path.read_text())
    for asset in feed["Assets"]:
        package = feed_dir / asset["FileName"]
        rebuilt = package.with_suffix(".tmp")
        with zipfile.ZipFile(package) as old, zipfile.ZipFile(rebuilt, "w", zipfile.ZIP_DEFLATED) as new:
            for info in old.infolist():
                data = new_appimage.read_bytes() if info.filename.endswith(".AppImage") else old.read(info)
                new.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED)
        rebuilt.replace(package)
        data = package.read_bytes()
        asset["SHA1"] = hashlib.sha1(data).hexdigest().upper()
        asset["SHA256"] = hashlib.sha256(data).hexdigest().upper()
        asset["Size"] = len(data)
    feed_path.write_text(json.dumps(feed))


def linux_flow(args, work: Path, out1: Path, out2: Path) -> None:
    channel = args.channel
    appimage1 = next(out1.glob("*.AppImage"))
    appimage2 = next(out2.glob("*.AppImage"))
    inspect_linux(appimage1, work)
    owners = run(["sh", "-c", "dpkg -S 'libfuse*.so*' 'fusermount*' 2>/dev/null | cut -d: -f1 | sort -u"]).stdout.split()
    for package in sorted(set(owners) | {"fuse", "fuse3", "libfuse2", "libfuse2t64", "libfuse3-3"}):
        run(["sudo", "apt-get", "purge", "-y", package])
    run(["sudo", "sh", "-c", "rm -f /lib/x86_64-linux-gnu/libfuse* /usr/lib/x86_64-linux-gnu/libfuse* && ldconfig"])
    run(["sudo", "sh", "-c", "rm -f /bin/fusermount* /usr/bin/fusermount* /sbin/mount.fuse*"])
    absent = fuse_absent()
    record("1.4 FUSE absent", not any(absent["tools"].values()) and not absent["libfuse"], **absent)

    apps = Path.home() / "Applications"
    apps.mkdir(exist_ok=True)
    installed = apps / "Made.AppImage"

    def place(image: Path) -> None:
        run(["pkill", "-9", "-f", str(installed)])
        staging = installed.with_suffix(".tmp")
        shutil.copy2(image, staging)
        make_exec(staging)
        os.replace(staging, installed)

    def try_runtime(label: str, image1: Path, feed_dir: Path) -> bool:
        place(image1)
        first = report(installed)
        record(f"1.4 {label} launch without FUSE", first.get("exit_code") == 0 and first.get("installed") is True,
               report=first)
        if first.get("exit_code") != 0:
            return False
        server = serve(feed_dir)
        try:
            before = len(launches())
            run([installed, "update", f"http://127.0.0.1:{PORT}", "--apply"], timeout=300)
            relaunched = wait_for_launch(V2, before)
            after = report(installed)
            record(f"1.4 {label} v1->v2 update without FUSE", relaunched and after.get("version") == V2,
                   relaunched=relaunched, after=after.get("version"), launch_lines=launches()[before:])
        finally:
            server.terminate()
        return True

    if not try_runtime("velopack runtime", appimage1, out2):
        runtime = Path(args.fallback_runtime) if args.fallback_runtime else None
        if runtime and runtime.exists():
            fallback1, fallback2 = work / "fallback-v1.AppImage", work / "fallback-v2.AppImage"
            replace_runtime(appimage1, runtime, fallback1)
            replace_runtime(appimage2, runtime, fallback2)
            helptext = run([fallback1, "--appimage-help"])
            variants = {}
            for name, value in (("URUNTIME_EXTRACT", "1"), ("URUNTIME_EXTRACT", "2"), ("URUNTIME_EXTRACT", "3"),
                                ("APPIMAGE_EXTRACT_AND_RUN", "1")):
                result = run([fallback1, "report"], env={**os.environ, name: value})
                variants[f"{name}={value}"] = {"exit_code": result.returncode, "stderr": result.stderr[-300:]}
            record("1.4 fallback runtime options", None, help=(helptext.stdout + helptext.stderr)[-5000:],
                   variants=variants)
            feed = work / "feed-fallback"
            shutil.copytree(out2, feed)
            rewrite_nupkg(feed, channel, fallback2)
            record("1.4 FUSE absent for fallback", not any(fuse_absent()["tools"].values()), **fuse_absent())
            if try_runtime("fallback runtime (A)", fallback1, feed):
                out2 = feed
                appimage1 = fallback1
    pre = prerelease_feed(out2, channel, work)
    common_checks(args, installed_exe=lambda: installed, reset=lambda: place(appimage1),
                  out2=out2, pre=pre, work=work)


def common_checks(args, installed_exe, reset, out2: Path, pre: Path, work: Path) -> None:
    reset()
    server = serve(pre.parent)
    try:
        exe = installed_exe()
        stable = run([exe, "update", f"http://127.0.0.1:{PORT}/{pre.name}", "--download-only", "--stable-only"])
        raw = run([exe, "update", f"http://127.0.0.1:{PORT}/{pre.name}", "--download-only"])
        record("1.7 higher prerelease-style entry", None,
               stable_only_offered=[l for l in stable.stdout.splitlines() if "offered" in l],
               velopack_default_offered=[l for l in raw.stdout.splitlines() if "offered" in l])
        renamed, moves = renamed_feed(out2, args.channel, work, args.os_label)
        reset()
        exe = installed_exe()
        result = run([exe, "update", f"http://127.0.0.1:{PORT}/{renamed.name}", "--download-only"])
        record("1.7 renamed end-user downloads keep updates working", "downloaded" in result.stdout, moves=moves)
        feeds = sorted(p.name for p in out2.iterdir() if p.name.startswith(("releases.", "assets.", "RELEASES")) or p.suffix == ".nupkg")
        record("1.7 channel feed files", None, channel=args.channel, files=feeds)
    finally:
        server.terminate()
    server = serve(out2)
    try:
        kill_points(args.os, reset, installed_exe, f"http://127.0.0.1:{PORT}")
    finally:
        server.terminate()


def macos_flow(args, work: Path, out1: Path, out2: Path) -> None:
    pkg = next(out1.glob("*.pkg"), None)
    zip1 = next(out1.glob("*-Portable.zip"), None)
    apps = Path.home() / "Applications"
    apps.mkdir(exist_ok=True)
    app = apps / "Made.app"
    exe = lambda: app / "Contents" / "MacOS" / "made-app"
    if pkg:
        result = run(["installer", "-pkg", pkg, "-target", "CurrentUserHomeDirectory"], timeout=600)
        located = [str(p) for p in Path.home().glob("Applications/*.app")]
        record("1.3 pkg installs for the current user without admin", result.returncode == 0 and app.exists(),
               exit_code=result.returncode, apps=located, stderr=result.stderr[-800:])
        if app.exists():
            first = report(exe())
            record("1.3 pkg layout resolution", first.get("plugin_in_resources", {}).get("exists") is True
                   and first.get("companion_next_to_exe", {}).get("exists") is True
                   and first.get("askpass_next_to_exe", {}).get("exists") is True, report=first)
            verify = run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", app])
            record("1.3 codesign --verify after pkg install", verify.returncode == 0, output=verify.stderr[-800:])
            server = serve(out2)
            try:
                before = len(launches())
                run([exe(), "update", f"http://127.0.0.1:{PORT}", "--apply"], timeout=300)
                relaunched = wait_for_launch(V2, before)
                after = report(exe())
                verify = run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", app])
                record("1.3 pkg v1->v2 update relaunches", relaunched and after.get("version") == V2,
                       relaunched=relaunched, after=after.get("version"), launch_lines=launches()[before:])
                record("1.3 codesign --verify after update", verify.returncode == 0, output=verify.stderr[-800:])
            finally:
                server.terminate()
        shutil.rmtree(app, ignore_errors=True)
    else:
        record("1.3 pkg produced", False)

    def reset_zip() -> None:
        shutil.rmtree(app, ignore_errors=True)
        run(["ditto", "-x", "-k", zip1, apps], check=True)
        run(["xattr", "-w", "-r", "com.apple.quarantine", "0081;00000000;Safari;", app])

    if zip1:
        reset_zip()
        xattrs = run(["xattr", "-l", app]).stdout
        spctl = run(["spctl", "--assess", "--type", "execute", "-vv", app])
        first = report(exe())
        record("1.3 zip extracted to ~/Applications with quarantine", first.get("exit_code") == 0,
               xattr=xattrs.strip(), spctl=(spctl.stdout + spctl.stderr).strip(), report=first)
        inspect_macos(app)
        pre = prerelease_feed(out2, args.channel, work)
        common_checks(args, installed_exe=exe, reset=reset_zip, out2=out2, pre=pre, work=work)
    else:
        record("1.3 portable zip produced", False)


def windows_flow(args, work: Path, out1: Path, out2: Path) -> None:
    setup = next(out1.glob("*-Setup.exe"))
    local = Path(os.environ["LOCALAPPDATA"]) / PACK_ID
    result = run([setup, "--silent"], timeout=600)
    time.sleep(5)
    exe = lambda: local / "current" / "made-app.exe"
    start_menu = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs"
    uninstall = run(["reg", "query", rf"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{PACK_ID}"])
    record("1.5 Windows per-user install", result.returncode == 0 and exe().exists(),
           root=str(local), start_menu=[p.name for p in start_menu.glob("*.lnk")],
           uninstall_entry=uninstall.returncode == 0, uninstall=uninstall.stdout[-800:])
    inspect_windows(out1, local)
    first = report(exe())
    record("1.3/1.5 Windows layout resolution", first.get("exit_code") == 0, report=first)
    server = serve(out2)
    try:
        before = len(launches())
        run([exe(), "update", f"http://127.0.0.1:{PORT}", "--apply"], timeout=300)
        relaunched = wait_for_launch(V2, before)
        after = report(exe())
        record("Windows v1->v2 update relaunches", relaunched and after.get("version") == V2,
               relaunched=relaunched, after=after.get("version"), launch_lines=launches()[before:])
    finally:
        server.terminate()
    portable = next(out1.glob("*-Portable.zip"))
    root = work / "portable"

    def reset() -> None:
        run(["taskkill", "/F", "/IM", "Update.exe"])
        shutil.rmtree(root, ignore_errors=True)
        with zipfile.ZipFile(portable) as archive:
            archive.extractall(root)

    pre = prerelease_feed(out2, args.channel, work)
    common_checks(args, installed_exe=lambda: root / "current" / "made-app.exe", reset=reset,
                  out2=out2, pre=pre, work=work)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--os", required=True, choices=["linux", "macos", "windows"])
    parser.add_argument("--channel", required=True)
    parser.add_argument("--os-label", required=True)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--fallback-runtime")
    args = parser.parse_args()
    RESULTS.update({"os": args.os, "channel": args.channel, "runner": platform.platform()})
    work = args.work
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    out1, out2 = work / "out1", work / "out2"
    try:
        for version, out in ((V1, out1), (V2, out2)):
            root = stage(work, args.os, args.exe, version)
            packed = vpk_pack(root, version, args.channel, out, args.os)
            record(f"vpk pack {version}", packed.returncode == 0, files=listing(out) if out.exists() else [])
            if packed.returncode != 0:
                raise SystemExit("vpk pack failed")
        {"linux": linux_flow, "macos": macos_flow, "windows": windows_flow}[args.os](args, work, out1, out2)
    except Exception as error:  # noqa: BLE001 - every observation is recorded
        record("harness error", False, error=repr(error))
    finally:
        args.results.write_text(json.dumps(RESULTS, indent=2))
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a", encoding="utf-8") as handle:
                handle.write(f"## Probe {args.channel}\n\n```json\n{json.dumps(RESULTS, indent=2)[:60000]}\n```\n")


if __name__ == "__main__":
    main()
