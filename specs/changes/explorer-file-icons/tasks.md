# Tasks: explorer-file-icons

> **Scope:** Explorer file and folder icons only. Row geometry, fills, label colors, selection, focus, keyboard behavior, picker rows, and the tree model are unchanged. The `made-ui-ux-v1` cross-reference is applied with this proposal and is not an implementation task.

## 0. Contract alignment

- [x] 0.1 Cross-reference this change from the `made-ui-ux-v1` "Explorer folder and row presentation" scenario so entry icons defer here while v1 keeps row geometry, selection, focus, and label ownership; do not renumber or rewrite unrelated v1 content (path: `specs/changes/made-ui-ux-v1/spec.md`)

## 1. Vendored assets and provenance

- [x] 1.1 Write the failing asset-inventory and provenance test: expects the mapping JSON at the asset root, all 1,256 SVGs under `icons/`, every mapping icon path normalizing into that directory, the MIT license text, attribution, an update note, and pinned v5.40.0 provenance; confirm failure before vendoring (path: `crates/made-app/tests/explorer_icons.rs`)
  - Evidence (red, then green): before vendoring, `cargo test -p made-app --test explorer_icons vendored_assets_match_pinned_release_inventory` failed with `the asset root must hold only the vendored release` (the root held only `icons`). It passes after 1.2–1.4.
- [x] 1.2 Vendor the complete Material Icon Theme v5.40.0 package: the mapping JSON at the asset root and all 1,256 SVGs under `icons/`, adding no curated subset; upstream `./../icons/<name>.svg` mapping paths are normalized by the build script (path: `crates/made-app/assets/material-icons/material-icons.json`)
  - Evidence (1.2–1.4): `cmp` and `diff -r` show `material-icons.json`, `LICENSE`, and `icons/` byte-identical to the extracted v5.40.0 package. The asset root holds exactly those entries plus `README.md`, and `icons/` holds 1,256 `.svg` files.
- [x] 1.3 Add the upstream MIT license text (path: `crates/made-app/assets/material-icons/LICENSE`)
- [x] 1.4 Add attribution and the update/readme note: upstream project, source/tag, pinned version, complete-set invariant, and replacement procedure (path: `crates/made-app/assets/material-icons/README.md`)
  - Evidence (m5, review fix): step 3 of the update procedure now states every count the tests hard-code (1,256 SVGs and definitions; 2,156 file names; 1,391 file extensions; 4,674 folder names in the closed and expanded tables) and the constants that change together (`ICON_COUNT`, `PINNED_TARBALL`, `PINNED_INTEGRITY`, and the literal table counts in `generated_index_is_sorted_and_complete`). A README-to-test check matches all 8 items; before the edit, 6 of the 8 were missing.

## Checkpoint — Phase 1

- [x] Verify "Pinned release with complete assets and MIT attribution" via `cargo test -p made-app --test explorer_icons vendored_assets_match_pinned_release_inventory`
- [x] Inventory check reports 1,256 SVGs under `icons/`, one mapping JSON at the asset root with all icon paths resolving inside `icons/`, one license, and the pinned v5.40.0 note

## 2. Build-time index and resolution

- [x] 2.1 Write failing resolution tests for a known file type, exact-name precedence over extension, compound `d.ts`, case-insensitive file names, extensions, and folder names, unknown file default, named folder closed/open, default folder closed/open, special-row bypass, and embedded-byte resolution; confirm failure before implementation (path: `crates/made-app/tests/explorer_icons.rs`)
  - Evidence (red): before `made_app::explorer_icons` existed, `cargo test -p made-app --test explorer_icons --locked` failed to compile with `E0432 unresolved import made_app::explorer_icons`. All 10 Phase 2 checkpoint tests pass after 2.3–2.6.
- [x] 2.2 Write the failing generated-index audit test: file and folder tables are sorted with no duplicate or case-fold collisions, every key maps to a vendored definition, and every icon id exposes non-empty embedded bytes; confirm failure (path: `crates/made-app/tests/explorer_icons.rs`)
- [x] 2.3 Add `serde_json` as a build dependency (path: `crates/made-app/Cargo.toml`)
- [x] 2.4 Add the build script: parse the mapping, normalize each `./../icons/<name>.svg` icon path into the vendored `icons/` directory, reject absolute, traversal, or escaping paths, validate every definition's SVG and reject orphan SVGs, reject duplicate object keys at any depth, missing, empty, or case-fold-colliding entries, and stray `icons/` entries except a regular file named exactly `.DS_Store` or `Thumbs.db`, sort all tables, emit rerun directives, and write the generated index into `OUT_DIR` (path: `crates/made-app/build.rs`)
  - Evidence (2.4): the build generates 1,256 `Icon` entries and tables of 2,156 file names, 1,391 extensions, 4,674 folder names, and 4,674 expanded folder names. Scratch copies each with one violation exit 101 with a specific message: missing SVG, orphan SVG, `..` traversal, absolute path, empty SVG, non-SVG content, case-fold collision, empty key, dangling target, missing default, invalid JSON. The unmodified copy exits 0.
  - Evidence (review fix M1, m1): probe harness re-run, 45 cases, 0 unexpected results. Red first: the nine M1 and m1 defect cases exited 0 on the prior build. After the fix, a repeated object key at any depth exits 101 with `… is not a valid mapping: duplicate object key "ts" at line 1 column N` (top-level, nested depth 2, and an escaped `\u0074s` spelling included). A stray `notes.txt`, an empty `sub/`, and a `sub/` containing `x.svg` each exit 101 with `icons/<name> is not an .svg file` or `icons/<name> is a directory`, followed by `icons/ may contain only regular .svg files, so remove it or move it out of the vendored directory`. The unmodified mapping still builds (exit 0), and a repeated key in sibling array objects is not flagged.
  - Evidence (OS metadata exception): a regular file named exactly `.DS_Store` or `Thumbs.db` directly in `icons/` is ignored. The build exits 0 and the generated index is byte-identical to the baseline; four probe cases cover this (`.DS_Store`, `Thumbs.db`, a zero-byte `.DS_Store`, and both together), all identical. Near misses (`.DS_Store.bak`, `thumbs.db`, `THUMBS.DB`, `.ds_store`, `.DS_Store` with a trailing space, `._.DS_Store`), directories and symlinks with those names, an orphan SVG beside `.DS_Store`, and an empty SVG beside `Thumbs.db` still exit 101 with the specific message, each checked against its message text. Probe run after the change: 60 cases, 0 unexpected results, all earlier M1 and m1 cases included.
- [x] 2.5 Declare the public explorer icons module (path: `crates/made-app/src/lib.rs`)
- [x] 2.6 Implement the module: generated-index access, `include_bytes!` SVG embedding, exact-name then longest-suffix then default file resolution, named then default folder resolution with closed/expanded variants, special-row bypass, embedded-byte access, and index audit stats (path: `crates/made-app/src/explorer_icons.rs`)

## Checkpoint — Phase 2

- [x] Verify "Known file-type icon" via `file_icon_resolves_known_extension`
- [x] Verify "Exact-name precedence over extension" via `file_icon_prefers_exact_name_over_extension`
- [x] Verify "Compound extension" via `file_icon_prefers_longest_compound_extension`
- [x] Verify "Case-insensitive matching" via `file_name_extension_and_folder_matching_are_case_insensitive`
- [x] Verify "Unknown file uses the default icon" via `unknown_file_uses_default_file_icon`
- [x] Verify "Named folder uses closed and open variants" via `named_folder_uses_closed_and_open_variants`
- [x] Verify "Default folder uses closed and open variants" via `default_folder_uses_closed_and_open_icons`
- [x] Verify "Special rows keep explicit markers" via `special_rows_do_not_resolve_theme_icons`
- [x] Verify the generated index via `generated_index_is_sorted_and_complete`
- [x] Verify embedded-byte resolution via `icons_resolve_from_embedded_bytes`

## 3. Rendering

- [x] 3.1 Add `egui_extras` 0.37 with the `svg` feature (path: `crates/made-app/Cargo.toml`)
- [x] 3.2 Write the failing all-icons rasterize test: decode every vendored icon to a non-empty image and confirm at least one known icon retains multiple distinct colors; confirm failure before rendering (path: `crates/made-app/tests/explorer_icons.rs`)
  - Evidence (red, then green): before 3.3 the test failed with `NoImageLoaders`. After 3.3 all 12 `explorer_icons` tests passed. Review fix m4: the test was split into `all_vendored_icons_rasterize_to_visible_pixels` (every icon) and `known_icon_keeps_multiple_colors` (`typescript.svg`). The traceability check (every test named in the checkpoints and coverage table exists in source) was red on the new names before the split and is green after it. 13 `explorer_icons` tests pass.
- [x] 3.3 Install only the SVG loader once in theme setup so `egui::Image` can decode the embedded SVGs, deliberately omitting the file, HTTP, image, GIF, and WebP loaders (path: `crates/made-app/src/theme.rs`)
- [x] 3.4 Update the existing explorer-row headless assertions: replace the app-drawn triangle and bullet expectations with textured image expectations while preserving row geometry, depth, selection, hover, and non-file-marker coverage; confirm failure (path: `crates/made-app/tests/ui_model.rs`)
- [x] 3.5 Add failing rendering tests: an image mesh occupies the 16px slot, open and closed folder textures differ, text stays 24px from the row edge plus 14px per depth with unchanged centering, the former bullet and triangle are absent, hover and selection do not recolor icons, unknown file and folder fallbacks render, and `@`, `?`, and `!` remain with their current marker colors (path: `crates/made-app/tests/ui_model.rs`)
  - Evidence (red, then green): before 3.6, the modified 3.4 test and the seven new 3.5 `ui_model` tests all failed on the missing textured icon in the 16px slot. All eight pass after 3.6–3.7.
- [x] 3.6 Precompute each row's icon id when rows rebuild and render the resolved full-color image with a distinct embedded URI per icon in the existing 16px slot without changing row geometry, fills, label colors, or focus behavior (path: `crates/made-app/src/explorer_view.rs`)
- [x] 3.7 Remove the now-unused `paint_bullet`; keep `paint_disclosure` for the picker (path: `crates/made-app/src/theme.rs`)

## Checkpoint — Phase 3

- [x] Verify "Known file-type icon" via `explorer_rows_render_material_icons_in_the_icon_slot`
- [x] Verify "Named folder uses closed and open variants" via `explorer_folder_icon_changes_between_closed_and_open`
- [x] Verify "Default folder uses closed and open variants" via `explorer_default_folder_icon_changes_between_closed_and_open`
- [x] Verify "Unknown file uses the default icon" via `explorer_unknown_file_uses_default_icon`
- [x] Verify "Explorer row layout is unchanged" via `explorer_rows_keep_text_offsets_and_vertical_centering`
- [x] Verify "Hover and selection do not tint icons" via `explorer_icons_keep_full_color_on_hover_and_selection`
- [x] Verify "Full-color icon rendering" via `known_icon_keeps_multiple_colors` and `explorer_icons_keep_full_color_on_hover_and_selection`
- [x] Verify "Special rows keep explicit markers" via `explorer_special_rows_keep_explicit_markers`
- [x] Verify every vendored icon rasterizes via `all_vendored_icons_rasterize_to_visible_pixels`

## 4. Verification

- [x] 4.1 Run `cargo fmt --all -- --check` and record the result (path: `specs/changes/explorer-file-icons/tasks.md`)
  - Evidence: `cargo fmt --all -- --check` → exit 0, no output (latest run). The first check after the review fix flagged two long lines in `build.rs`; `cargo fmt --all` fixed them and the check then passed.
- [x] 4.2 Run `cargo clippy --workspace --all-targets --locked -- -D warnings` and record the result (path: `specs/changes/explorer-file-icons/tasks.md`)
  - Evidence: `cargo clippy --workspace --all-targets --locked -- -D warnings` → exit 0, zero warning or error lines (latest run, after formatting). The build script is linted too.
- [x] 4.3 Run `cargo test --workspace --locked --no-fail-fast` and record the exact counts and result (path: `specs/changes/explorer-file-icons/tasks.md`)
  - Evidence: `cargo test --workspace --locked --no-fail-fast` → exit 101 (latest run). Totals: 709 passed, 1 failed, 2 ignored. Sole failing test: `real_zsh_zdotdir_unset_and_reassignment_follow_user_startup_files` (`made-platform --test shell_integration`), which is deterministic and predates this change. The `explorer_view::tests` keyboard test passed in this run; it is intermittent under parallel execution and also fails at HEAD.
  - Also run: `cargo test -p made-app --test ui_model --locked` → exit 0, 60 passed, 0 failed.
- [x] 4.4 Run `CARGO_NET_OFFLINE=true cargo test -p made-app --test explorer_icons --locked` and confirm the icon path resolves and rasterizes from embedded bytes with no fetch, filesystem asset read, or update path (path: `specs/changes/explorer-file-icons/tasks.md`)
  - Evidence: `CARGO_NET_OFFLINE=true cargo test -p made-app --test explorer_icons --locked` → exit 0, `test result: ok. 13 passed; 0 failed; 0 ignored` (latest run, finished in 0.51s).

## Checkpoint — Phase 4

- [x] Verify "Offline build and runtime" via `icons_resolve_from_embedded_bytes`, `all_vendored_icons_rasterize_to_visible_pixels`, and `CARGO_NET_OFFLINE=true cargo test -p made-app --test explorer_icons --locked`
- [x] Every scenario in `spec.md` maps to a passing test in the scenario coverage table below; fmt, clippy, and the workspace test run pass
  - Evidence: all 13 scenarios map to passing tests (names verified in the latest run). `cargo fmt --all -- --check` → exit 0; `cargo clippy --workspace --all-targets --locked -- -D warnings` → exit 0, zero warning or error lines; `cargo test --workspace --locked --no-fail-fast` → exit 101 with 709 passed, 1 failed, 2 ignored, the sole failure being the known zsh test recorded in 4.3. The workspace run has one pre-existing environmental failure in `made-platform` `shell_integration` that reproduces on a clean `HEAD` export and is unrelated to this change; the macOS arm64 `keyboard_moves_selection_and_activates_only_on_enter_at_both_sizes` test is intermittently timing-sensitive under parallel load and is tracked as a separate follow-up.

## Coverage summary

| Requirement | Task coverage | Checkpoint |
|---|---|---|
| Explorer Entry Icon Resolution | 2.1–2.6 | Phase 2 |
| Explorer Entry Icon Presentation | 3.1–3.7 | Phase 3 |
| Vendored Offline Icon Assets | 1.1–1.4, 2.4, 2.6, 4.4 | Phase 1, Phase 2, Phase 4 |

## Scenario coverage

| Spec scenario | Test or command |
|---|---|
| Known file-type icon | `file_icon_resolves_known_extension`; `explorer_rows_render_material_icons_in_the_icon_slot` |
| Exact-name precedence over extension | `file_icon_prefers_exact_name_over_extension` |
| Compound extension | `file_icon_prefers_longest_compound_extension` |
| Case-insensitive matching | `file_name_extension_and_folder_matching_are_case_insensitive` |
| Unknown file uses the default icon | `unknown_file_uses_default_file_icon`; `explorer_unknown_file_uses_default_icon` |
| Named folder uses closed and open variants | `named_folder_uses_closed_and_open_variants`; `explorer_folder_icon_changes_between_closed_and_open` |
| Default folder uses closed and open variants | `default_folder_uses_closed_and_open_icons`; `explorer_default_folder_icon_changes_between_closed_and_open` |
| Special rows keep explicit markers | `special_rows_do_not_resolve_theme_icons`; `explorer_special_rows_keep_explicit_markers` |
| Explorer row layout is unchanged | `explorer_rows_keep_text_offsets_and_vertical_centering` |
| Hover and selection do not tint icons | `explorer_icons_keep_full_color_on_hover_and_selection` |
| Full-color icon rendering | `known_icon_keeps_multiple_colors`; `all_vendored_icons_rasterize_to_visible_pixels` |
| Pinned release with complete assets and MIT attribution | `vendored_assets_match_pinned_release_inventory` |
| Offline build and runtime | `icons_resolve_from_embedded_bytes`; `CARGO_NET_OFFLINE=true cargo test -p made-app --test explorer_icons --locked` |

## Analyze — explorer file icons (2026-10-29)

**Readiness assessment:** PASS for implementation planning; no implementation has been performed, so all delivery checkpoints remain open until the test-first tasks run.

| Analyze gate | Result | Basis |
|---|---|---|
| Scope and live-contract consistency | PASS | New active change for Explorer entry icons; `made-ui-ux-v1` keeps row layout, selection, focus, and label ownership and cross-references this change; nothing archived or version-bumped |
| Outcome versus implementation separation | PASS | `spec.md` states observable icon resolution, presentation, and offline/licensing outcomes; `design.md` and `tasks.md` carry `build.rs`, `include_bytes!`, `egui_extras`, loader installation, and per-row caching |
| Scenario/test-first coverage | PASS | 13 scenarios across 3 requirements; every scenario maps to a failing-test task, implementation task, and named checkpoint |
| Unresolved clarification markers | PASS | No unresolved clarification marker, TODO, or unresolved placeholder in proposal, spec, tasks, or design |
| Duplicated normative contract | PASS | v1 retains row geometry, text offsets, fills, label colors, selection, and focus; icons appear once in this change and are only cross-referenced from v1 |
| Dependency and ownership | PASS | All changes stay in `made-app`; `made-explorer` remains UI-free; no OS branch or network path is added |
| Historical completion integrity | PASS | Only the v1 cross-reference is pre-applied; no implementation, evidence, or delivery claim is marked complete |
| Offline and security | PASS | Complete pinned v5.40.0 assets, build-time validation, embedded bytes, MIT attribution, no runtime fetch or update path, trusted SVG input only |
