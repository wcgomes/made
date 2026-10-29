# Layout and Controls

Made uses one native window with workspace tabs above a content area, a selected left surface (terminal or full-left preview), and a full-height Explorer in the right column. The terminal card owns the only glass layer in the terminal surface (`BG_TERMINAL_GLASS`, 160/255), switching to opaque `BG_TERMINAL` in alternate-screen TUI mode; its body/chrome add no second glass, and the optional session strip stays opaque `BG_HEADER`. The normative layout and interaction contract is [`specs/changes/archive/20261029-made-ui-ux-v1/spec.md`](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md). This page maps the implemented composition in [`ui.rs`](../../crates/made-app/src/ui.rs) and [`explorer_view.rs`](../../crates/made-app/src/explorer_view.rs).

## Window geometry

| Region | 1280×800 | 720×480 |
|---|---|---|
| Window gutter | 10 | 8 |
| Floating header | 1260×26 at `(10,8)` | 704×24 at `(8,6)` |
| Gap below strip | 8 | 6 |
| Main content | 1260×748 at `y=42` | 704×436 at `y=36` |
| Explorer column | 368 total: 8 gap + 360 card | 294 total: 6 gap + 288 card |
| Left terminal/preview | 892×748 | 410×436 |
| Explorer hidden | Left surface gets full 1260×748 | Full 704×436 |

Panels have no separator lines; cards and gaps do the grouping. The Explorer is always full content height, never reduced by preview. At 1100+ width the Explorer defaults to 360px. At 900–1099 it defaults to 35% of usable content width, clamped to 288–360px; below 900 it uses compact gutter/header/radii and a 288px default. `min_left_width` is unchanged. Essential controls must fit or move to labeled overflow, never clip.

## Header and workspace tabs

- Floating header: no persistent band fill, border, shadow, or top highlight. The 26/24 strip equals the workspace tab height; the header row spans gutter..width-gutter with no extra horizontal padding, tabs start at the card/window content left edge, and the card content top is 42/36 with one 8/6 pane gap above and below the strip.
- Left: editor-like workspace tabs start at the content left edge. The cached two-tone monospace `MADE` wordmark (mint `ACCENT` `M`, primary `ADE`, 14px, 1.8px tracking, tooltip “Minimal Agentic Development Environment”) sits between the tab strip and the right icon group, with a reserved 24px wide / 18px compact strip-side gap and an exact 24/18 icon-side gap; it has no tab fill/hover/selection and no divider.
- Selected-tab stroke follows the [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md). Selected uses opaque `BG_TAB_SELECTED` with a `TEXT_PRIMARY` 13.0px label; idle uses `BG_TAB_IDLE` with secondary 12.5px text at 0.72 opacity; hover uses `BG_TAB_HOVER` with a primary label, and a selected tab keeps its own fill on hover. There is no accent outline, underline, dot/hollow marker, or width animation. Tabs use stable workspace-scoped ids and an overflow list when needed.
- Close “×”: hidden at rest for every tab, including the selected tab, while its 24×24 target and keyboard focusability stay reserved. It appears at the fixed 14px size on tab hover or when the tab/close has keyboard focus, with no scale animation. Hovering the X paints no close background and turns only the glyph `ERROR`; tab width, label truncation, and layout do not change. The parent tab uses `BG_TAB_HOVER` (or keeps `BG_TAB_SELECTED` when selected) and a primary label. The parent path tooltip is suppressed so “Close workspace” is the tooltip shown.
- Tab-strip tail: the app-drawn `+` action sits immediately after the last visible workspace tab and before the overflow trigger. It keeps the same stable ID, 24×24 minimum target, and keyboard activation as the other icon actions, and is reserved in the tab-fitting budget. Its tooltip and accessibility name stay “Open folder” whether or not a compatible TUI is detected. With no detected compatible TUI, activating it opens the folder picker directly. When a TUI is detected on `PATH`, it opens a menu anchored to the control with `Terminal` first and `OpenCode` second; `Terminal` runs the existing add-workspace picker flow, while `OpenCode` starts the TUI directly in a new home-directory tab without opening the picker or stopping the current tab.
- Right: icon-only Sessions (when shown), Diagnostics, Explorer, left-to-right so Explorer is rightmost. The last icon’s 26/24 target, selected/hover fill, and focus ring all end exactly at the card right border (`content.right`) with no gutter overhang; the 14px glyph stays centered in its target with normal optical padding. Each has a monochrome app-drawn glyph at 0.80 opacity when inactive, at least 24×24 target, matching tooltip, accessible name, and visible focus; hover is `BG_HOVER` and selection is `BG_SELECTED` without a border or accent bar. No persistent action text labels; no version row. Session-map strip/tabs remain 36/34 and 28/26.
- The Explorer toggle is the only hide/restore action and remains keyboard reachable.

## Explorer

- Full-height right card using `BG_CARD_GLASS` at 188/255 without a native backdrop (off/unsupported/failed), or `BG_CARD_GLASS_NATIVE` at 24/255 when macOS HudWindow or Windows 11 Mica is actually applied; same RGB pair, 1px border, radius 10/8, painted glass shadow.
- Card header 34/30 uses `BG_HEADER_GLASS` at 194/255 without a native backdrop, or `BG_HEADER_GLASS_NATIVE` at 32/255 when the native effect is applied; same RGB pair, 1px top highlight. It shows only `EXPLORER` and the root name (full-path tooltip). There are no Wider, Narrower, Reload, or Hide text actions and no routine refresh control.
- Hiding/restoring animates the panel width over about 150 ms with quadratic ease-out. The tree stays laid out at its stored width and is clipped to the sliding panel, so content does not reflow. Reversal settles at the stored width; closed state removes tree/splitter input.
- Resize is pointer-drag only on an 8px boundary that is transparent at rest. Hover fades in a 1px line and 6×32 grip at partial-opacity `BORDER_CONTROL` over about 0.14 s; dragging uses the restrained mint `ACCENT` with the same geometry, and leaving fades both out. Hover alone does not change width. It keeps a resize cursor and “Drag to resize explorer” tooltip and is not a keyboard Tab stop.
- Width, visibility, expansion, and valid selection survive tab switches and hide/restore, not app restart.
- Tree rows are 24px high with a 4px horizontal inset, 6px radius, a 16px icon slot inset 4px within the painted row fill (slot starts 8px from the row's left edge), and text starting 28px from the row's left edge plus 14px per depth. Directories and files use distinct glyphs; symlinks and errors are explicit.
- Hover is a borderless radius-6 `BG_HOVER` fill; selected/opened rows use only the inset radius-6 `BG_SELECTED` fill plus primary text, with no row or tree-container focus outline, border, or accent side bar.
- File and directory rows show full-color Material Icon Theme icons in the existing 16px slot. Folders use the theme's closed and expanded variants and keep the brighter expanded label; file icons follow the theme mapping. Symlink, other/special, and error rows keep `@`, `?`, and `!`. Row height, inset, radius, text start at 28px plus 14px per depth, fills, label colors, and the icon/label optical center are unchanged. The authoritative change is [`explorer-file-icons`](../../specs/explorer-file-icons.md). Live refresh is watcher-driven; there is no routine Reload action.
- Keyboard focus entering the tree with no prior selection selects the first visible row without opening a preview.
- The tree scrolls with a floating overlay bar hidden at rest; it allocates zero width and never reflows content.
- Hiding gives the reclaimed width to the left surface without stopping the shell or losing preview/tree state. Restore reconciles changes that occurred while hidden.

## Full-left preview

- Opening a file replaces the entire left terminal surface visually; it is never stacked below the Explorer or split beside the terminal. No preview placeholder exists before a file is opened.
- Card header 34/30: filename, full-path tooltip, understated read-only label, `Copy path`, `Refresh`, a return-to-terminal action, and `Close`. Both Close and return restore the same live terminal and its focus.
- Content uses a 44px right-aligned line-number gutter and 13px monospace code at 18px lines with horizontal scrolling. Syntax highlighting comes from syntect with a plain-text fallback; low-contrast spans are blended toward primary at document build time, not per frame.
- Meta line shows size/language/lines/updated age; compact mode hides some metadata below 600 height.
- Binary, invalid UTF-8, over-2-MiB, missing, unreadable, and empty states use safe cards/messages; no raw binary is rendered and there is no write path.
- The hidden shell keeps running and consuming bounded output. Hidden preview input never reaches it. A root change clears the preview, invalidates stale reads, and returns to terminal.

## Picker

- The picker is an explicit card surface: `MADE` brand and version, icon-only Close X with tooltip, `Back` for add-workspace, path/toolbar, inline tree, and primary/secondary folder actions.
- Toolbar: Up/Refresh, then a borderless inset `BG_SELECTED` path chip at radius 4 with primary mono 10.5 text and a full-path tooltip. The chip follows the selected tree row, or the root when nothing is selected.
- The list is an inline expandable tree rooted at the start directory. Root children are depth 0; expanding inserts children below the parent and keeps every ancestor visible; collapsing removes only that subtree. Rows are 32px high with a 4px horizontal inset, radius-6 fills, and 14px depth; app-drawn triangles share the label’s optical center.
- There is one flattened selected row. Hover uses a borderless radius-6 `BG_HOVER` fill; selection uses only the inset `BG_SELECTED` fill plus primary text, with no row/container focus outline, border, or accent side bar. The tree scrollbar is hidden at rest. The picker card and its buttons keep their focus rings.
- Clicking a row selects it and toggles expansion; an expanded folder collapses and stays selected, and a collapsed folder expands. The path chip and “Use current folder” target follow the selected row path, or the root when nothing is selected. A double-click toggles once and never starts a workspace.
- Keyboard: Up/Down move visible rows; Right expands the selected folder or selects its first child when already expanded; Left collapses the selected folder or selects its parent when collapsed; Enter toggles expansion; Cmd/Ctrl+Enter confirms; Backspace or Up re-roots to the parent directory and clears expansion/selection. Refresh re-reads the root and expanded directories, preserving valid expansion and selection by path; each directory keeps the 1000-entry cap, and an unreadable expanded child stays collapsed with a non-blocking error.
- Expanding or moving selection reveals the row with minimal scrolling: it bottom-aligns only when needed, keeps the selected row and visible ancestor chain on screen when they fit, and does not center the row.
- Both footer actions remain: “Open selected folder” is a borderless secondary action (`BG_ELEVATED` rest, `BG_HOVER` hover, `BG_SELECTED` pressed) and “Use current folder” stays accent-filled. Escape/Close returns to the live workspace when one exists, otherwise to the default home/terminal fallback; Back preserves the add-workspace return flow.
- No shell or explorer process starts before confirmation. Confirming an add-workspace flow opens a new tab without disturbing the existing shell.

## Scrollbars

- Explorer tree, picker list, preview code, diagnostics body/log, and session-map overflow use floating overlay scrollbars: hidden at rest, thumb-only fade-in over about 120–160 ms when the area is hovered, expand to the 8px bar with a faint track only on bar hover/drag, and fade out on leave. They allocate zero width and never shift layout.
- The terminal scrollback bar is custom-painted with the same rest/hover/fade behavior and the same `SCROLL_THUMB`/`SCROLL_THUMB_HOVER` colors. It keeps its full 8px hit area, paints only the thumb on pane hover, and paints the faint track only on bar hover/drag.
- Fade duration and states are asserted from the scroll style/helper state; temporary Xvfb checks cover rest, pane-hover, and bar-hover states, but the captures are not stored.

## Diagnostics and optional session map

- Diagnostics is an opaque floating card with a status pill, reason, copy actions, sectioned connection/session/worktree/version/metrics/log detail, a bounded log block, and a path/copy footer.
- Technical state belongs here: workspace identity, latest shell cwd, cwd reporting capability/reason, effective root/source, integration status/reason, versions, revisions, heartbeat, metrics, and redacted logs.
- Inactive optional integration is not a degraded workspace. There is no routine status bar, banner, or empty session strip.
- The optional OpenCode session map is initially hidden, scoped to the selected workspace, and visually distinct from workspace tabs. It lists open root sessions with title, path, and busy/attention/unread/renaming indicators using shape plus text. It never reserves empty space by default.

## Interaction and accessibility

- Every persistent keyboard-reachable control has a visible focus state; the pointer-only Explorer divider is skipped. Picker card/buttons and other controls keep focus rings. Deliberate exception: Explorer tree rows/container and picker rows paint no outline; their focus/selection is the inset `BG_SELECTED` fill plus primary text.
- The `+` menu is keyboard reachable: Enter/Space opens it, Up/Down moves focus between items with the standard focus ring, and Enter/Space activates the focused item. Items use stable ids, at least 24×24 targets, and accessible button names.
- Workspace/session tabs support Tab entry, Left/Right movement, and Enter/Space activation. The terminal is one focus stop.
- A workspace tab is selected by its opaque `BG_TAB_SELECTED` fill and the 13.0px label versus 12.5px idle; selection never relies on color alone. The close X is hidden at rest, appears on tab/close hover or keyboard focus, keeps its inside focus ring, and uses the red glyph only as its direct-hover cue, with tab width unchanged.
- Keyboard focus entering the Explorer tree with no prior selection selects the first visible row without opening a preview. The picker tree uses Up/Down, Right/Left, Enter, Cmd/Ctrl+Enter, and Backspace/Up as described above; rows have no outline and use the selection fill plus primary text.
- Targets are at least 24×24, text at least 10px, and status never relies on color alone.
- Motion is limited to spinners, hover-triggered scrollbar fades, the splitter fade (0.14 s), the Explorer width transition, and standard control animation; no essential information exists only in motion. Close visibility is state-driven and never scales; splitter motion stays inside the existing 8px hit area.
- `accesskit` is not enabled, so platform screen-reader exposure is not claimed. Tooltips and `Response::widget_info` accessible names are set for icon actions.

## Implementation state

The Calm Editor layout is implemented and verified on Linux with headless/UI-model tests plus temporary Xvfb checks at 1280×800 and 720×480. These checks cover the mint header accent, `+` placement, Explorer-rightmost icon order, workspace-tab states per the [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md), tab close, faded splitter, expanded/selected Explorer folder, inline picker tree expansion/selection/collapse/keyboard states, preview, explorer scrollbar, terminal scrollback, primary/alternate terminal surfaces, and conditional Explorer glass. Visual captures are ad hoc validation artifacts and are not stored in the repository. The no-native fallback clear uses alpha 215/255; native clear remains 135/255, and Xvfb without a compositor cannot show blending over a light root. Native Linux/Windows compositor behavior and native pointer/keyboard/screen-reader behavior remain unvalidated. A macOS arm64 HudWindow vibrancy/UI rendering check on 2026-10-28 covered windowed, resized, and fullscreen states; Phase 15's exact glass alpha values and Phase 16's conditional Explorer pairs are headless/token-verified, and the 1280×800 native Explorer tint was visually approved by the user on 2026-10-28. Post-fix native terminal pixel parity and native contrast certification remain pending (the display was locked for the post-fix capture). Windows and macOS x64 remain compile-only. See [Platform matrix](../testing/platform-matrix.md) and [v1 evidence](../records/20261026-v1-evidence.md).

Related: [Design system](design-system.md), [UI spec](../../specs/changes/archive/20261029-made-ui-ux-v1/spec.md), [Runtime](../runtime/terminal-first.md).
