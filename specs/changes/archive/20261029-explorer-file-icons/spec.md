# Change Spec: explorer-file-icons

> **Contract relationship:** This active change is authoritative for Explorer entry icons. The active `made-ui-ux-v1` "Explorer folder and row presentation" scenario cross-references this change for file and folder icons and retains ownership of row geometry, selection, focus, and other Explorer presentation. This is a new bounded change, not an archive or version bump of v1.

## ADDED Requirements

### Requirement: Explorer Entry Icon Resolution

Made MUST show a full-color Material Icon Theme icon for every regular file and directory row in the Explorer, resolved from the vendored v5.40.0 mapping. File resolution MUST match the exact file name case-insensitively first, then the longest matching extension suffix case-insensitively (so a compound suffix such as `d.ts` wins over `ts`), then the theme's default file icon. Folder resolution MUST match the folder name case-insensitively, MUST use the theme's expanded variant for an expanded folder and closed variant for a collapsed folder, and MUST fall back to the theme's default expanded or closed folder icon. Symlink rows MUST keep the explicit `@` marker, other/special rows MUST keep `?`, and error rows MUST keep `!`; these rows MUST NOT be replaced by theme icons and MUST keep their current marker colors. Resolution MUST be deterministic, MUST be independent of locale, and MUST depend only on the entry name, kind, and a folder's expanded state.

#### Scenario: Known file-type icon

- Given an Explorer file row whose exact name or extension maps to an icon in the vendored Material Icon Theme
- When the row renders
- Then the mapped theme icon is shown in the row's 16px icon slot

#### Scenario: Exact-name precedence over extension

- Given a file whose exact name maps to one theme icon and whose extension maps to a different theme icon
- When the row renders
- Then the exact-name icon is shown

#### Scenario: Compound extension

- Given a file with a compound extension such as `types.d.ts` whose `d.ts` suffix and `ts` suffix both map to theme icons
- When the row renders
- Then the longest matching suffix (`d.ts`) icon is shown

#### Scenario: Case-insensitive matching

- Given file names, file extensions, and folder names that differ from the theme mapping only by letter case
- When the rows render
- Then the same icons are shown as for the canonical-case names

#### Scenario: Unknown file uses the default icon

- Given a file with no exact-name or extension match in the theme
- When the row renders
- Then the theme's default file icon is shown

#### Scenario: Named folder uses closed and open variants

- Given a folder whose name maps to a theme folder icon
- When the row is rendered collapsed and then expanded
- Then the closed variant is shown while collapsed and the expanded variant while expanded

#### Scenario: Default folder uses closed and open variants

- Given a folder with no name match in the theme
- When the row is rendered collapsed and then expanded
- Then the theme's default closed folder icon is shown while collapsed and the default expanded folder icon while expanded

#### Scenario: Special rows keep explicit markers

- Given symlink, other/special, and error rows in the Explorer
- When the Explorer renders them
- Then they show `@`, `?`, and `!` respectively with their current marker colors and no theme icon

### Requirement: Explorer Entry Icon Presentation

Explorer entry icons MUST render full-color exactly as shipped by the vendored theme, without tinting, desaturation, monochrome conversion, or alpha recoloring. Their artwork colors MUST NOT change at rest, hover, selection, or keyboard focus; hover and selection MUST be conveyed only by the existing row fill and label treatment. Icons MUST occupy the existing 16px icon slot and share the row label's vertical optical center. File and folder rows in the Explorer MUST NOT paint the former app-drawn file bullet or folder disclosure triangle. Row height (24px), the 4px horizontal inset, 6px radius, the text start at 24px from the row's left edge plus 14px per depth, hover/selection fills, label colors, hit areas, and focus treatment MUST remain unchanged. An icon MUST NOT add a fill, border, focus outline, accent bar, or extra indent.

#### Scenario: Explorer row layout is unchanged

- Given Explorer rows with file and folder icons at multiple depths
- When the tree renders at 1280×800 or 720×480
- Then rows keep 24px height, a 4px horizontal inset, 6px radius, a 16px icon slot, text 24px from the row's left edge plus 14px per depth, and unchanged hit areas; each icon and its label share one vertical optical center; the former app-drawn file bullet and folder disclosure triangle are not painted; and no icon adds a fill, border, focus outline, accent bar, or extra indent

#### Scenario: Hover and selection do not tint icons

- Given an icon-bearing row at rest, then hovered, then selected, then keyboard-focused
- When the row renders in each state
- Then the icon's artwork colors are unchanged and hover and selection remain conveyed only by the existing row fill and label treatment

#### Scenario: Full-color icon rendering

- Given any theme icon shown in the Explorer
- When it is rendered
- Then its upstream artwork and colors are preserved with no tint, desaturation, monochrome conversion, or alpha recoloring

### Requirement: Vendored Offline Icon Assets

Made MUST vendor and ship the complete Material Icon Theme v5.40.0 icon set, all 1,256 SVGs and its mapping, with the upstream MIT license text and attribution to the Material Icon Theme project. The vendored release MUST be documented by pinned version and update path. Building and testing the icon assets MUST NOT require npm, an icon package download, or icon-specific network access. At runtime, resolving and rendering Explorer icons MUST NOT require npm, a package download, or network access, and MUST work with the network unavailable. Made MUST NOT fetch, download, or update icons at runtime.

#### Scenario: Pinned release with complete assets and MIT attribution

- Given the application source and its vendored icon assets
- When the assets are inspected
- Then the complete v5.40.0 set is present with all 1,256 SVGs, the mapping, the MIT license text, attribution, and a documented pinned version and update path

#### Scenario: Offline build and runtime

- Given a machine with no network access, no npm registry, and the existing Cargo dependency cache
- When Made is built, tested, and run
- Then the icon assets come from the vendored source, every Explorer icon resolves and renders, and no icon asset is downloaded, fetched, or updated at runtime
