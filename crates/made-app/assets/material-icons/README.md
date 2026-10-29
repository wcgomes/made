# Material Icon Theme assets

Offline icon set for Made's Explorer file and folder icons.

## Provenance

| Field | Value |
|---|---|
| Upstream project | [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme) by Material Extensions |
| Pinned version | `5.40.0` (npm package `material-icon-theme@5.40.0`) |
| Upstream tag and commit | `v5.40.0` at `045291ff033062a25441dd297fd51546326866f2` (npm `gitHead`) |
| npm tarball | https://registry.npmjs.org/material-icon-theme/-/material-icon-theme-5.40.0.tgz |
| Registry `dist.integrity` | `sha512-IrvgusbwtqxhUtVgUy0nToJcfN7CYVRdbBxKMfbo0vab9flVBHA5INZ9UiE5SgRj+PM6opCl5nn5TZdpm2UaxA==` |
| Registry `dist.shasum` | `3ffad32f5a2afeefc243b38799ed526189ccbae3` |
| License | MIT, see [`LICENSE`](LICENSE) |

The tarball was checked against the registry values above before vendoring.

## Contents

- `material-icons.json`: the package's `dist/material-icons.json`, unmodified. Made reads only the default (dark) mapping: `iconDefinitions`, `fileNames`, `fileExtensions`, `folderNames`, `folderNamesExpanded`, `file`, `folder`, and `folderExpanded`. The `light`, `highContrast`, `languageIds`, and `hidesExplorerArrows` sections are not used.
- `icons/`: all 1,256 SVG files from the package's `icons/` directory, unmodified.
- `LICENSE`: the upstream MIT license text.

## Complete-set invariant

This directory holds the complete v5.40.0 set and nothing else. `icons/` contains exactly 1,256 SVG files. Every `iconDefinitions` entry's `iconPath` has the form `./../icons/<name>.svg` and names a file in `icons/`, and every SVG in `icons/` is referenced by at least one definition. No icon is curated, substituted, or omitted. `crates/made-app/build.rs` rejects any violation, so a partial or mismatched replacement fails the build. The only exceptions are regular files named exactly `.DS_Store` or `Thumbs.db`: directly in `icons/`, where `build.rs` ignores them as OS metadata, and directly in this asset root, where the inventory test ignores them; every other stray entry in either location still fails. `crates/made-app/tests/explorer_icons.rs` checks the same inventory.

Nothing is downloaded at build or run time. Made embeds every SVG with `include_bytes!`.

## Update procedure

1. Read the new version's `dist.integrity` from `https://registry.npmjs.org/material-icon-theme/<version>`. Download `https://registry.npmjs.org/material-icon-theme/-/material-icon-theme-<version>.tgz` and confirm its sha512 matches before extracting.
2. Extract the tarball. Replace this directory's contents as a whole:
   - `material-icons.json` from `package/dist/material-icons.json`;
   - `icons/` from the complete `package/icons/` directory;
   - `LICENSE` from `package/LICENSE`.
   Copy nothing else: no npm metadata, `dist/` siblings, type declarations, or modules.
3. Update the pinned version, tag and commit, tarball URL, and `dist.integrity` in this README, and the matching constants `PINNED_TARBALL` and `PINNED_INTEGRITY` in `crates/made-app/tests/explorer_icons.rs`. The tests also hard-code the vendored counts. Change them in the same change as the replaced files, and restate every changed count in this README:
   - `ICON_COUNT` (currently 1,256) in `crates/made-app/tests/explorer_icons.rs` is the SVG count in `icons/`, the `iconDefinitions` count, and the icon count the tests check.
   - In `generated_index_is_sorted_and_complete`, the literal table sizes must match the new mapping: `stats.file_names` for 2,156 file names, `stats.file_extensions` for 1,391 file extensions (including compound suffixes such as `d.ts`), and `stats.folder_names` and `stats.folder_names_expanded` for 4,674 folder names each, in the closed and expanded tables.
4. Run `cargo test -p made-app --test explorer_icons --locked`. The build and the inventory, index, and rasterization tests must all pass.
5. Review the diff for unexpected icon or license changes before committing.
