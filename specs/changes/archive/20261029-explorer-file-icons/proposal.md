# Proposal: Explorer file and folder icons

> **Change status:** This is a new active change for Explorer entry icons. It does not archive, version-bump, or supersede `made-ui-ux-v1`.

## Intent

The Explorer currently distinguishes entries with a folder disclosure triangle and a generic file bullet, which show kind and expansion but not file type. Replacing those markers with the Material Icon Theme gives file and folder names a familiar, scannable visual identity while preserving the existing Calm Editor row geometry, selection treatment, and offline build.

## Scope

In scope:

- Full-color file and folder icons for Explorer tree rows, resolved from the complete pinned Material Icon Theme v5.40.0 package.
- Exact-name, longest-extension-suffix, case-insensitive, and default fallback resolution for files; named and default folder icons with closed and expanded variants.
- Replacing the app-drawn Explorer folder disclosure triangle and file bullet in the existing 16px icon slot without changing row metrics, text offsets, hover/selection fills, or label colors.
- Vendoring all 1,256 SVGs and the mapping JSON with MIT attribution and a documented pinned update path; no runtime npm, network, or download dependency.
- A minimal cross-reference in the active `made-ui-ux-v1` Explorer row scenario; this change is authoritative for Explorer entry icons.

## Non-goals

- Picker, header, toolbar, tab, status, or diagnostics icons.
- Tinting, desaturation, monochrome conversion, theme switching, user icon overrides, or icon settings.
- Row height, inset, radius, text offsets, fills, label colors, focus treatment, keyboard behavior, or tree model changes.
- Replacing the explicit `@`, `?`, and `!` markers for symlink, other/special, and error rows.
- A runtime icon download, update service, or network dependency.
- Archiving `made-ui-ux-v1`, bumping a version, or declaring v1 delivered.

## Approach

Vendor the complete Material Icon Theme v5.40.0 package as application assets and generate a deterministic sorted lookup table at build time; embed the SVGs in the binary and resolve one icon per visible row when the row list rebuilds. Render the resolved full-color SVG in the existing 16px slot through `egui_extras` 0.37 with its `svg`/resvg support and `egui::Image`, leaving all row layout and interaction treatment in `explorer_view.rs` unchanged. Non-normative mechanics and alternatives are in `design.md`.

## Decision index

- **Complete pinned package, vendored offline:** use all 1,256 SVGs and `material-icons.json` from Material Icon Theme v5.40.0 (MIT) with no curated subset; the ~1 MB asset cost buys complete type coverage and deterministic offline builds (design: D1, D6).
- **Full-color, no tinting:** preserve the upstream artwork exactly; hover and selection stay row-fill and label cues rather than icon recoloring (design: D2).
- **Folder icon replaces the Explorer disclosure triangle:** use the theme's closed and expanded variants in the existing 16px slot; keep `paint_disclosure` for the picker and keep all row geometry and label colors (design: D3).
- **File precedence exact name → longest extension suffix → default:** match case-insensitively, prefer exact names such as `package.json`, and prefer compound suffixes such as `d.ts` over `ts`; fall back to the theme default file icon (design: D4).
- **Non-file rows keep explicit markers:** symlink, other/special, and error rows remain `@`, `?`, and `!` rather than being misrepresented by theme icons (design: D5).
- **Build-time index and embedded bytes:** validate the mapping and vendor inventory during the build, emit sorted tables, and resolve from embedded bytes so no runtime filesystem or network access is needed (design: D6).
- **New active change, not a v1 archive or version bump:** `made-ui-ux-v1` remains active and authoritative for Explorer row layout, selection, and focus; this change owns entry icons and only cross-references the v1 scenario (design: D7).

## Clarifications

- **Q:** Should this be folded into `made-ui-ux-v1` instead of a new change?
  **A:** No. It is a new active change with a single bounded outcome, Explorer entry icons; `made-ui-ux-v1` keeps its row presentation contract and cross-references this change for icons. Nothing is archived and no version is bumped.
- **Q:** Should the picker's disclosure triangles change too?
  **A:** No. Only Explorer tree file and folder rows get Material icons; the folder picker keeps its app-drawn disclosure triangles.
- **Q:** Should icons be tinted to match the Calm Editor palette?
  **A:** No. Icons render full-color exactly as shipped; row fill and label color changes remain the hover, selection, and expansion cues.
- **Q:** What happens to unknown file types and special filesystem entries?
  **A:** Unknown files and folders use the theme's default file or folder icon; symlink, other/special, and error rows keep their explicit `@`, `?`, and `!` markers.
