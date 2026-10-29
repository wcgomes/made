# Design: explorer-file-icons

## Approach

Vendor the complete Material Icon Theme v5.40.0 package under `crates/made-app/assets/material-icons/`: the 1,256 SVGs under `icons/`, `material-icons.json` at the asset root, and the upstream MIT license. The JSON exposes 1,256 icon definitions, 2,156 file-name keys, 1,391 extension keys, and 4,674 folder-name keys with matching expanded variants; keeping the whole package avoids a curated subset with silent coverage gaps.

At build time, `crates/made-app/build.rs` parses the mapping, validates it against the vendored SVG inventory, rejects malformed JSON, duplicate keys, empty keys, colliding keys, stray `icons/` entries, and orphan SVGs, sorts each table, and emits a generated Rust index into `OUT_DIR`. Each SVG is embedded with `include_bytes!` using `CARGO_MANIFEST_DIR`-relative paths so the build is portable and the binary needs no runtime asset directory. `crates/made-app/src/explorer_icons.rs` owns resolution and exposes an icon id plus its embedded bytes.

`ExplorerPane` precomputes one icon id for every row of the flattened tree when `rows_dirty` rebuilds `self.rows`, so rendering does no name parsing, table lookup, or JSON work per frame. `explorer_view.rs` draws the icon as an `egui::Image` sized to 16×16 in the existing slot using a distinct `bytes://material-icons/<file>.svg` URI per icon, keyed by the SVG file name, so egui caches each texture separately; `theme::install` installs only the `egui_extras` 0.37 SVG (resvg) loader, once per context. Only the `svg` feature is enabled, so the file, HTTP, image, GIF, and WebP loaders are deliberately absent; no runtime asset-path or network loader exists. `theme::paint_bullet` becomes unused and is removed; `theme::paint_disclosure` stays for the picker.

Alternatives considered:

- Runtime JSON parsing and filesystem reads: adds startup I/O, failure modes, and per-row work for data that never changes at runtime.
- Curated icon subset: smaller, but unknown types fall back more often and every upstream update needs manual curation.
- Build-time PNG rasterization: loses vector crispness across DPI scales and adds generated binary artifacts to review.
- Extension-only resolution: misses exact-name icons such as `package.json`, `Dockerfile`, and dotfiles.

## Significant decisions

### D1: Complete pinned package, vendored offline

Use the full upstream v5.40.0 package instead of a subset. The accepted cost is about 1.49 MB of source assets — 1,033,822 bytes of SVG across 1,256 files plus a 453,305-byte mapping JSON — and matching binary growth; the benefit is complete type coverage, reproducible builds, and no npm or network step. The vendored README records the upstream project, version/tag, source, license, complete-set invariant, and update procedure so the package can be audited and refreshed deliberately.

### D2: Full-color artwork, no tinting

Material icons carry type identity through their shipped colors. Tinting them to `TEXT_TERTIARY` or desaturating them on rest would erase that identity and add a recolor pipeline that must track selection, hover, focus, and expansion. The existing row fill and label treatment already communicate interaction and expansion, so icons keep their upstream colors in every state; the image is drawn with the neutral white tint so egui applies no color multiply.

### D3: Folder icon replaces the Explorer disclosure triangle

The folder icon occupies the same 16px slot as the old triangle, uses the theme's closed variant while collapsed and expanded variant while expanded, and keeps the brighter expanded label. The picker keeps `paint_disclosure` because it is a separate 32px-row surface with its own navigation contract and was not requested to change. Keeping one visual element in the slot avoids crowding and preserves the 24px text origin.

### D4: Exact name, then longest extension suffix, then default

File lookup lowercases the name and checks `fileNames` first. If absent, it walks dot-separated suffixes from longest to shortest (`types.d.ts` → `d.ts` → `ts`) and checks `fileExtensions`, so compound suffixes win. If neither matches, it uses the theme default `file` icon. Folder lookup lowercases the name and checks `folderNamesExpanded` or `folderNames` depending on state, falling back to `folder-open` or `folder`.

Case folding is locale-independent; matching does not use the OS locale or a case-sensitive filesystem assumption. The precedence mirrors the upstream theme semantics and handles exact-name icons, compound extensions, dotfiles, and unknown types without special cases.

### D5: Special rows keep explicit markers

Symlinks, other/special entries, and error rows have no trustworthy theme icon and their state matters more than their type. They keep the current `@`, `?`, and `!` monospace markers, colors, and positions. Only `EntryKind::File` and `EntryKind::Directory` enter icon resolution.

### D6: Build-time index and embedded bytes

`build.rs` is the only component that reads `material-icons.json`. Upstream mapping paths use the `./../icons/<name>.svg` form; the build strips that prefix, joins the file name to the vendored `icons/` directory, and rejects absolute paths, traversal, or any resolved path outside that directory. Every `iconDefinitions` entry must name an existing vendored SVG, and every vendored SVG must be named by some definition, so an unreferenced (orphan) SVG fails the build. `icons/` accepts only regular `.svg` files with plain ASCII names; a stray file, subdirectory, or symlink fails, except a regular file named exactly `.DS_Store` or `Thumbs.db` directly in `icons/`, which is ignored as OS metadata. The build also rejects malformed JSON, duplicate JSON object keys at any depth, empty keys, dangling definition targets, missing defaults, and keys that collide after case folding, then emits one embedded entry per vendored SVG plus sorted lookup arrays for binary search. Compilation fails on any violation, so a broken vendor update cannot ship.

`explorer_icons.rs` reads the generated index and embedded bytes only. `egui_extras` 0.37 with the `svg` feature supplies resvg decoding through `SvgLoader`, the only loader `theme.rs` registers; `egui::Image::from_bytes` uses the embedded bytes and egui caches the decoded textures. No runtime filesystem or network access exists on the icon path.

### D7: New active change, not a v1 archive or version bump

`made-ui-ux-v1` remains active and owns Explorer row height, inset, radius, text offsets, fills, label colors, selection, and focus. This change owns file and folder icon resolution and rendering. The v1 scenario now defers entry icons to this change instead of naming a triangle and bullet as the only markers. Nothing is archived, renumbered, or declared delivered.

## Flow

```text
crates/made-app/assets/material-icons/
  material-icons.json + icons/*.svg (v5.40.0, MIT; mapping paths normalized at build)
        │
        ├─ build.rs: parse → validate SVG inventory and definitions
        │             → reject collisions, duplicate keys, stray entries, orphans
        │             → sort tables → OUT_DIR/explorer_icons_index.rs
        ▼
  include_bytes!(CARGO_MANIFEST_DIR/assets/material-icons/icons/*.svg)
        │
        ▼
explorer_icons::resolve(name, kind, expanded) -> Option<IconId>
        │  called for every flattened row when ExplorerPane rows rebuild
        ▼
ExplorerPane row icon id
        │
        ▼
egui::Image::from_bytes("bytes://material-icons/<file>.svg") in the 16px slot
        │  SvgLoader installed by theme::install, once per context
        ▼
full-color 16×16 textured mesh at the row's optical center
```

## Architectural boundaries

- `made-app` owns the vendored data, build-time generation, resolution, and rendering. `made-explorer` remains UI-free and unchanged; it continues to provide `TreeRow` name, kind, depth, and expanded state.
- `build.rs` validates and generates only; it holds no product decisions and does not render.
- `theme.rs` owns one-time loader installation and stops exposing the file bullet; it does not resolve icons.
- `explorer_view.rs` owns the per-row cache and the 16px slot geometry; row metrics, fills, label colors, and focus behavior stay there unchanged.
- No OS branch, network listener, or new dependency is added outside `made-app`.

## Migration and rollback

There is no persisted state or data migration. The change is a presentation swap: rows that previously drew a triangle or bullet now draw an embedded image. Rolling back means removing the icon rendering and vendored assets, restoring `paint_bullet` and the triangle call in `explorer_view.rs`, and reverting the `made-ui-ux-v1` cross-reference. Updating the vendored version requires replacing the complete inventory, refreshing the license/attribution/update note, and rerunning the inventory, index, and rasterization tests; a partial replacement is rejected by the build.

## Security considerations

Only trusted bytes from the pinned vendored release reach the SVG decoder; there is no runtime asset path, user-supplied SVG, URL loading, download, or update mechanism. `build.rs` treats the mapping as untrusted input: it normalizes `iconPath` values into the vendored `icons/` directory and rejects duplicate keys at any depth, missing definitions, empty or case-folding-colliding keys, stray `icons/` entries, orphan SVGs, path traversal, and absolute paths, and it only reads files under the vendored asset directory. Decoding is bounded by 1,256 small, fixed SVGs, and egui caches decoded textures. The MIT license text and attribution ship with the assets.
