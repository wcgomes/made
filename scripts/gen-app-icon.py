#!/usr/bin/env python3
"""Regenerate the binary app icons from crates/made-app/assets/app-icon/made-icon.svg.

Usage: scripts/gen-app-icon.py [--chrome PATH]

Needs Google Chrome (headless, to rasterize the SVG with alpha) and Pillow.
Outputs, next to the SVG:
  made-icon.ico           Windows: 16/24/32/48/64/128/256, full-bleed tile
  made-icon.icns          macOS: Apple icon grid, 824px body inside a 1024px canvas
  made-icon-512.png       Linux AppImage icon (vpk --icon), full-bleed tile
  made-icon-256.png       window icon on Windows and Linux (embedded in made-app)
  made-icon-macos-512.png window/Dock icon on macOS (embedded in made-app)
"""

from __future__ import annotations

import argparse
import io
import os
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "crates" / "made-app" / "assets" / "app-icon"
CHROME_CANDIDATES = (
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chrome",
)
CANVAS = 1024
MACOS_BODY = 824  # Apple's icon grid: rounded-rect body inside the 1024 canvas
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
# (icns type, pixel size): 1x and 2x slots from 16 to 512 points.
ICNS_TYPES = (
    ("icp4", 16), ("ic11", 32), ("icp5", 32), ("ic12", 64), ("ic07", 128),
    ("ic13", 256), ("ic08", 256), ("ic14", 512), ("ic09", 512), ("ic10", 1024),
)


def find_chrome(explicit: str | None) -> str:
    for candidate in ([explicit] if explicit else CHROME_CANDIDATES):
        if candidate and (os.path.isfile(candidate) or shutil.which(candidate)):
            return candidate
    sys.exit("gen-app-icon: Chrome not found; pass --chrome PATH")


def rasterize(chrome: str, svg: Path, body: int, work: Path) -> Image.Image:
    """Render the SVG at `body` px, centered on a transparent CANVAS x CANVAS page."""
    offset = (CANVAS - body) // 2
    page = work / f"page-{body}.html"
    page.write_text(
        '<!doctype html><html><body style="margin:0;background:transparent">'
        f'<img src="{svg.as_uri()}" width="{body}" height="{body}" '
        f'style="position:absolute;left:{offset}px;top:{offset}px"></body></html>',
        encoding="utf-8",
    )
    out = work / f"page-{body}.png"
    subprocess.run(
        [chrome, "--headless", "--disable-gpu", "--hide-scrollbars",
         "--default-background-color=00000000", f"--window-size={CANVAS},{CANVAS}",
         f"--screenshot={out}", page.as_uri()],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    return Image.open(out).convert("RGBA").crop((0, 0, CANVAS, CANVAS))


def png_bytes(image: Image.Image, size: int) -> bytes:
    buffer = io.BytesIO()
    image.resize((size, size), Image.HAMMING).save(buffer, "PNG", optimize=True)
    return buffer.getvalue()


def write_icns(image: Image.Image, path: Path) -> None:
    chunks = b"".join(
        kind.encode("ascii") + struct.pack(">I", 8 + len(data)) + data
        for kind, size in ICNS_TYPES
        for data in [png_bytes(image, size)]
    )
    path.write_bytes(b"icns" + struct.pack(">I", 8 + len(chunks)) + chunks)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--chrome")
    chrome = find_chrome(parser.parse_args().chrome)
    svg = ASSETS / "made-icon.svg"
    with tempfile.TemporaryDirectory() as tmp:
        full = rasterize(chrome, svg, CANVAS, Path(tmp))
        mac = rasterize(chrome, svg, MACOS_BODY, Path(tmp))
    (ASSETS / "made-icon-512.png").write_bytes(png_bytes(full, 512))
    (ASSETS / "made-icon-256.png").write_bytes(png_bytes(full, 256))
    (ASSETS / "made-icon-macos-512.png").write_bytes(png_bytes(mac, 512))
    write_icns(mac, ASSETS / "made-icon.icns")
    full.save(ASSETS / "made-icon.ico", sizes=[(s, s) for s in ICO_SIZES])


if __name__ == "__main__":
    main()
