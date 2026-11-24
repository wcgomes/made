# 0010: Bundled DejaVu Terminal Font

**Status:** Accepted 2026-11-24; implemented, not yet merged.

## Context

The terminal grid used egui's bundled Hack. Hack lacks many Claude Code TUI symbols (⏵ ⏺ ⎿ ✢ ✳ ✶ ✻ ✽ ✓ ✗ ✘ ⚠ ★ …). egui 0.37's system font lookup searches only generic families and never reaches Menlo or Apple Symbols, so those symbols rendered as tofu, color emoji, or misaligned STIX glyphs. Hack also looked heavy.

## Decision

Bundle a **DejaVu Sans Mono** subset (2.37) as the terminal font on every OS, backed by a bundled **Iosevka Term Extended Regular** symbols-only subset (v34.9.1, OFL) for the three Claude Code symbols DejaVu lacks (⏵ ⏺ ⎿) and Powerline. UI fonts stay Ubuntu-Light and Hack. Font choice is not user-configurable (deferred).

- The user chose DejaVu after a rendered comparison of Iosevka Term Extended Light/Regular, JetBrains Mono Light/Regular, DejaVu Sans Mono, Hack, and Menlo.
- DejaVu covers 24 of 27 Claude Code symbols natively; the Iosevka subset covers the rest, so nothing reaches Hack or system fonts.
- DejaVu is under the Bitstream Vera license plus public-domain changes; the subset may keep its name because it contains neither "Bitstream" nor "Vera".
- Implementation and metrics: [Design system › Typography](../ui/design-system.md#typography).

## Rejected alternatives

- **Iosevka Term Light** (tried first, live): the only surveyed font covering all 27 symbols, but its 0.5 em cells were too narrow and less legible in use.
- **Iosevka Term Extended:** wider, but keeps Iosevka's narrow, tall letterforms.
- **JetBrains Mono:** the user preferred DejaVu's look; it also has taller 17.16pt rows.
- **System font per OS:** inconsistent across OSes; Menlo cannot be bundled.
- **Changing egui's app-wide text alpha curve:** would also thin UI text.

## Consequences

- Same glyph coverage and the same 7.83 × 15.125pt cell as Hack at 13pt on every OS.
- Adds ~256 KB of font assets with both licenses in `crates/made-app/assets/terminal-font/`; regenerate the subsets with `scripts/subset-terminal-fonts.sh` when coverage changes.
- Braille is drawn as cell geometry because DejaVu has none.
- Residuals: DejaVu's ✻ ✳ ✽ are slightly heavier than its text; Iosevka's ⏺ is slightly smaller and 1px higher than DejaVu's ●; edges still render darker than CoreText.
