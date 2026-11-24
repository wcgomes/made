# Terminal grid fonts

Made embeds both files with `include_bytes!` and registers them, in this order, as the `terminal` font family, ahead of egui's Monospace chain (Hack, Ubuntu-Light, egui icons) and the system font fallback:

1. `DejaVuSansMono.ttf`: the primary face. Its cell is 1233/2048 em (7.83 pt at 13 pt).
2. `IosevkaTermExtended-Symbols.ttf`: the first fallback, for symbols DejaVu Sans Mono lacks, such as U+23F5, U+23FA, and U+23BF. Its 600/1000 em cell matches DejaVu's to within 0.3%.

## Provenance

| Field | DejaVu Sans Mono | Iosevka Term Extended |
|---|---|---|
| Upstream project | [DejaVu Fonts](https://github.com/dejavu-fonts/dejavu-fonts) | [Iosevka](https://github.com/be5invis/Iosevka) by Renzhi Li (Belleve Invis) |
| Pinned version | `2.37` (tag `version_2_37`) | `34.9.1` |
| Release asset | https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.zip | https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-IosevkaTerm-34.9.1.zip |
| Asset sha256 | `7576310b219e04159d35ff61dd4a4ec4cdba4f35c00e002a136f00e96a908b0a` | `3bd57a66a7f1e429a131d3bc35f4d0f3a8eb722f2fe8f59f595e424bda9a33c9` |
| Source face | `dejavu-fonts-ttf-2.37/ttf/DejaVuSansMono.ttf` | `IosevkaTerm-Extended.ttf` (Term Extended Regular) |
| License | Bitstream Vera Fonts license, with DejaVu changes in the public domain and Arev glyphs under their own notice; see [`LICENSE-DejaVu`](LICENSE-DejaVu), the upstream `LICENSE` unchanged. The subset keeps the name DejaVu Sans Mono, which contains neither "Bitstream" nor "Vera". | SIL Open Font License 1.1, see [`LICENSE-Iosevka`](LICENSE-Iosevka) (upstream `LICENSE.md` at tag `v34.9.1`). No Reserved Font Name. |

## Contents

- `DejaVuSansMono.ttf`: a subset of the source face with 2,305 code points and 2,356 glyphs. It keeps Latin, Greek, Cyrillic, punctuation, currency, letterlike, arrows, math, technical, box drawing, block, geometric, symbol, and dingbat code points.
- `IosevkaTermExtended-Symbols.ttf`: a subset with 609 code points and 677 glyphs. It keeps code points from General Punctuation, Arrows, Miscellaneous Technical, Geometric Shapes (and Extended), Miscellaneous Symbols, Dingbats, Supplemental Arrows A to C, Miscellaneous Symbols and Arrows, and Powerline, minus the code points the DejaVu subset maps. fontTools also keeps U+221F in it, so that one code point is in both subsets; DejaVu's glyph wins. Box drawing, block elements, and braille are drawn as cell geometry by the app, so neither subset needs braille.
- In both subsets every mapped code point has the face's one-cell advance; double-width and zero-width glyphs are left out. The other glyphs are `.notdef` and outline components that mapped glyphs reference. Layout features, hinting, and DejaVu's FontForge timestamp table are removed. All name records, including the copyright and license fields, are kept.
- `LICENSE-DejaVu` and `LICENSE-Iosevka`: the upstream license texts.

## Update procedure

Edit the pinned URLs, sha256s, and block lists in `scripts/subset-terminal-fonts.sh`, run it, and update this README. With fontTools 4.60.2 (installed by the script from a hash-pinned wheel) and the same source archives, the script reproduces the committed files byte for byte.
