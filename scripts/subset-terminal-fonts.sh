#!/usr/bin/env bash
# Rebuild the terminal grid fonts in crates/made-app/assets/terminal-font/:
#
#   DejaVuSansMono.ttf               primary face: text and the symbols it has
#   IosevkaTermExtended-Symbols.ttf  first fallback: symbols DejaVu Sans Mono lacks
#
# Usage: scripts/subset-terminal-fonts.sh
#
# Sources, each checked against its sha256 before it is read:
#   DejaVu Fonts 2.37, dejavu-fonts-ttf-2.37/ttf/DejaVuSansMono.ttf from
#     https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.zip
#   Iosevka v34.9.1, IosevkaTerm-Extended.ttf (Term Extended Regular) from
#     https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-IosevkaTerm-34.9.1.zip
# Set DEJAVU_ZIP or IOSEVKA_ZIP to a local copy of an archive to skip its
# download.
#
# DejaVu Sans Mono's cell is 1233/2048 em (0.602) and Iosevka Term Extended's is
# 600/1000 em. Each subset keeps only code points the face maps at exactly one
# cell, so double-width and zero-width glyphs are left to later fallbacks.
# DejaVu Sans Mono keeps the text and symbol blocks below. The Iosevka subset
# keeps only the symbol blocks, minus the code points the DejaVu subset already
# maps (fontTools still keeps U+221F in both; DejaVu's glyph wins). Box drawing,
# block elements, and braille are drawn as cell geometry by the app and are not
# taken from it. Layout features and hinting are dropped: the grid draws one
# glyph per cell, and egui does not hint. DejaVu's FontForge timestamp table
# (FFTM) is dropped. Every name record, including the copyright and license
# fields, is kept. For each face this command runs:
#
#   pyftsubset <face>.ttf --unicodes-file=<cell code points> \
#     --layout-features= --no-hinting --drop-tables+=FFTM --name-IDs='*' \
#     --name-languages='*' --name-legacy --notdef-outline --output-file=<asset>
#
# fontTools is pinned to 4.60.2, installed from a hash-pinned wheel, so the
# output is byte-identical. Set FONTTOOLS_PYTHON to a Python that already has
# that version to skip creating a virtual environment.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dejavu_url="https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.zip"
dejavu_sha256="7576310b219e04159d35ff61dd4a4ec4cdba4f35c00e002a136f00e96a908b0a"
iosevka_url="https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-IosevkaTerm-34.9.1.zip"
iosevka_sha256="3bd57a66a7f1e429a131d3bc35f4d0f3a8eb722f2fe8f59f595e424bda9a33c9"
fonttools_version="4.60.2"
# sha256 of fonttools-4.60.2-py3-none-any.whl as published on PyPI.
fonttools_wheel_sha256="73cf92eeda67cf6ff10c8af56fc8f4f07c1647d989a979be9e388a49be26552a"
assets="$root/crates/made-app/assets/terminal-font"

for tool in curl unzip shasum python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "subset-terminal-fonts: $tool is required" >&2; exit 1; }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/src"

# fetch <local copy or empty> <url> <sha256> <archive member>
fetch() {
  local zip="$1"
  if [ -z "$zip" ]; then
    zip="$work/$(basename "$2")"
    curl -fsSL -o "$zip" "$2"
  fi
  local actual
  actual="$(shasum -a 256 "$zip" | awk '{ print $1 }')"
  if [ "$actual" != "$3" ]; then
    echo "subset-terminal-fonts: $zip has sha256 $actual, expected $3" >&2
    exit 1
  fi
  unzip -q -j "$zip" "$4" -d "$work/src"
}
fetch "${DEJAVU_ZIP:-}" "$dejavu_url" "$dejavu_sha256" dejavu-fonts-ttf-2.37/ttf/DejaVuSansMono.ttf
fetch "${IOSEVKA_ZIP:-}" "$iosevka_url" "$iosevka_sha256" IosevkaTerm-Extended.ttf

python="${FONTTOOLS_PYTHON:-}"
if [ -z "$python" ]; then
  python3 -m venv "$work/venv"
  # Hash-pinned to the pure-Python wheel on PyPI; pip installs only that file.
  printf 'fonttools==%s --hash=sha256:%s\n' "$fonttools_version" "$fonttools_wheel_sha256" >"$work/requirements.txt"
  "$work/venv/bin/python" -I -m pip install --quiet --disable-pip-version-check --no-deps --require-hashes -r "$work/requirements.txt"
  python="$work/venv/bin/python"
fi
installed="$("$python" -I -c 'import fontTools; print(fontTools.version)')"
if [ "$installed" != "$fonttools_version" ]; then
  echo "subset-terminal-fonts: fontTools $installed found, $fonttools_version required" >&2
  exit 1
fi

# Code points for each face: only those it maps at one cell.
"$python" -I - "$work/src/DejaVuSansMono.ttf" "$work/src/IosevkaTerm-Extended.ttf" \
  "$work/text.txt" "$work/symbols.txt" <<'EOF'
import sys
from fontTools.ttLib import TTFont

SYMBOL_BLOCKS = [
    (0x2000, 0x206F),  # General Punctuation
    (0x2190, 0x21FF),  # Arrows
    (0x2300, 0x23FF),  # Miscellaneous Technical
    (0x25A0, 0x25FF),  # Geometric Shapes
    (0x2600, 0x26FF),  # Miscellaneous Symbols
    (0x2700, 0x27BF),  # Dingbats
    (0x27F0, 0x27FF),  # Supplemental Arrows-A
    (0x2900, 0x297F),  # Supplemental Arrows-B
    (0x2B00, 0x2BFF),  # Miscellaneous Symbols and Arrows
    (0xE0A0, 0xE0A3),  # Powerline
    (0xE0B0, 0xE0B3),  # Powerline
    (0x1F780, 0x1F7FF),  # Geometric Shapes Extended
    (0x1F800, 0x1F8FF),  # Supplemental Arrows-C
]
TEXT_BLOCKS = [
    (0x0020, 0x007E),  # Basic Latin
    (0x00A0, 0x00FF),  # Latin-1 Supplement
    (0x0100, 0x024F),  # Latin Extended-A and -B
    (0x0250, 0x02FF),  # IPA Extensions, Spacing Modifier Letters
    (0x0370, 0x03FF),  # Greek and Coptic
    (0x0400, 0x052F),  # Cyrillic, Cyrillic Supplement
    (0x1E00, 0x1EFF),  # Latin Extended Additional
    (0x2070, 0x209F),  # Superscripts and Subscripts
    (0x20A0, 0x20CF),  # Currency Symbols
    (0x2100, 0x214F),  # Letterlike Symbols
    (0x2150, 0x218F),  # Number Forms
    (0x2200, 0x22FF),  # Mathematical Operators
    (0x2400, 0x243F),  # Control Pictures
    (0x2460, 0x24FF),  # Enclosed Alphanumerics
    (0x2500, 0x257F),  # Box Drawing
    (0x2580, 0x259F),  # Block Elements
    (0x27C0, 0x27EF),  # Miscellaneous Mathematical Symbols-A
    (0x2800, 0x28FF),  # Braille Patterns
    (0x2980, 0x2AFF),  # Miscellaneous Mathematical Symbols-B, Supplemental Operators
    (0xFFFD, 0xFFFD),  # Replacement Character
] + SYMBOL_BLOCKS


def one_cell(path, blocks):
    font = TTFont(path)
    cmap = font.getBestCmap()
    advances = font["hmtx"].metrics
    cell = advances[cmap[ord("M")]][0]
    return [
        cp
        for first, last in blocks
        for cp in range(first, last + 1)
        if cp in cmap and advances[cmap[cp]][0] == cell
    ]


text = one_cell(sys.argv[1], TEXT_BLOCKS)
taken = set(text)
symbols = [cp for cp in one_cell(sys.argv[2], SYMBOL_BLOCKS) if cp not in taken]
for path, cps in [(sys.argv[3], text), (sys.argv[4], symbols)]:
    with open(path, "w") as out:
        out.write("".join(f"U+{cp:04X}\n" for cp in cps))
EOF

# subset <source> <code points> <asset>: written to $work/out first.
mkdir "$work/out"
subset() {
  "$python" -I -m fontTools.subset "$work/src/$1" \
    --unicodes-file="$work/$2" \
    --layout-features= --no-hinting --drop-tables+=FFTM \
    --name-IDs='*' --name-languages='*' --name-legacy \
    --notdef-outline \
    --output-file="$work/out/$3"
  echo "$3: $(wc -c <"$work/out/$3" | tr -d ' ') bytes, $(wc -l <"$work/$2" | tr -d ' ') code points"
}
subset DejaVuSansMono.ttf text.txt DejaVuSansMono.ttf
subset IosevkaTerm-Extended.ttf symbols.txt IosevkaTermExtended-Symbols.ttf

# Both subsets succeeded: replace the assets together.
mkdir -p "$assets"
mv "$work/out/DejaVuSansMono.ttf" "$work/out/IosevkaTermExtended-Symbols.ttf" "$assets/"
echo "wrote both subsets to $assets"
