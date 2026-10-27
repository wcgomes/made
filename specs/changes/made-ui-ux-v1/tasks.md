# Tasks: made-ui-ux-v1

> **Checkbox scope:** checked items reflect implementation and automated Linux/headless/Xvfb evidence in this tree, not native platform or manual visual validation. Moved task IDs retain their original core numbering in parentheses for traceability. Native macOS/Windows, native Linux X11/Wayland, native pointer/keyboard, native compositor/backdrop, accessibility-tree, and hardware-GPU checkpoints remain unchecked. See `wiki/records/20261026-v1-evidence.md`.

> **Contract split:** this change owns UI/UX/visual tasks only. Product/runtime/integration/security/performance tasks remain in `specs/changes/opencode-session-follow-v1/tasks.md`. No code behavior is changed by this split; no archive or version bump.

## 1. Visual system, brand, header and picker

- [x] 1.1 (moved core 8.7) Adjust layout bounds only as required for usable full-height explorer/left surfaces at both sizes; preserve Calm Editor tokens, contrast, opaque terminal/preview and focus styling (path: `crates/made-app/src/theme.rs`)
- [x] 1.2 (moved core 11.2) Add/rework the icon-action primitive with monochrome glyph, 24×24 minimum target, tooltip/accessibility metadata hook and focus styling (path: `crates/made-app/src/theme.rs`)
- [x] 1.3 (moved core 11.1) Extend UI tests for icon-only header actions (matching tooltip/accessibility name, visible focus, no text labels, workspace tab close affordance, 1280×800/720×480 reachability); replace existing text-label assertions without weakening control-reachability coverage; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 1.4 (moved core 11.3) Implement icon-only Open folder, Explorer toggle, Diagnostics and optional Sessions actions; remove header text action labels; preserve focus order and workspace-tab close affordances (path: `crates/made-app/src/ui.rs`)
- [x] 1.5 (moved core 1.20) Write a failing folder-picker test for no-workspace launch and picker selection (path: `crates/made-app/tests/workspace_picker.rs`)
- [x] 1.6 (moved core 1.21) Implement the in-app folder picker and selected-workspace open (path: `crates/made-app/src/workspace_picker.rs`)
- [x] 1.7 Verify Calm Editor glass alphas, clear-color/native fallback, two-tone wordmark, tab/no-dot treatment, compact overflow and picker selection/Close/Escape polish through headless tests and Xvfb captures (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 1.8 Record native macOS HudWindow vibrancy, Windows 11 Mica, compositor alpha/glass and native contrast at both window sizes; keep compile-only checks marked as non-evidence (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 1.9 Record native header/tab/picker keyboard, tooltip, pointer and focus behavior on macOS, Linux X11/Wayland and Windows (path: `wiki/records/20261026-v1-evidence.md`)

## Checkpoint — Phase 1

- [ ] Verify "Calm Editor tokens and typography", "Explorer glass and transparent header", "Floating header controls", "Native backdrop auto and fallback", "Reading-surface contrast"
- [ ] Verify "MADE wordmark identity", "Workspace tab presentation", "Workspace tab overflow and close"
- [ ] Verify "Picker selection and close", "Picker tree expansion and ancestors", "Picker keyboard tree navigation", "Picker refresh and re-root state", "Picker preserves live workspace"
- [x] Headless/Xvfb checks cover the implemented Linux UI; native backdrop/keyboard/pointer rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 2. Layout, explorer and preview presentation

- [x] 2.1 (moved core 8.1) Extend UI tests for quiet default shell, separate workspace/session tabs, initially hidden session map, full-height tree, explorer hide/restore/resize, compact layout, keyboard controls and per-tab widget ids; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 2.2 (moved core 8.2) Extend preview UI tests for full-left file-open, no default placeholder, both close/return paths to live terminal, output continuity, input isolation, root-change clearing and stale reads, while retaining every binary/size/encoding/no-write guard; confirm failures (path: `crates/made-app/tests/read_only_ui.rs`)
- [x] 2.3 (moved core 8.5) Replace default status/banner/empty-session chrome with workspace tab controls and discoverable Explorer/Diagnostics/optional Sessions controls; implement new-folder, switch and close actions, full-height hideable/resizable right tree and mutually exclusive full-left terminal/preview, workspace-scoped ids and explicit focus transfer (path: `crates/made-app/src/ui.rs`)
- [x] 2.4 (moved core 8.6) Separate tree and preview rendering, remove embedded/right-column preview and watcher status chrome, reconcile hidden-tree changes and reject stale root/preview deliveries; retain all guarded preview states and actions (path: `crates/made-app/src/explorer_view.rs`)
- [x] 2.5 (moved core 11.4) Extend UI tests for explorer header action removal, hide/restore through the header icon, hover-only divider with no at-rest line/grip, pointer-drag width, no keyboard width stop, per-workspace width retention, and no Reload action; replace the existing visible-at-rest splitter assertion without weakening drag coverage; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 2.6 (moved core 11.5) Extend live-update regression to prove create/delete/rename still update within 1 second with no manual Reload control present; confirm failures (path: `crates/made-app/tests/ui_model.rs`)
- [x] 2.7 (moved core 11.6) Remove explorer header Wider/Narrower/Reload/Hide actions while retaining root identity and contextual tree errors [P] (path: `crates/made-app/src/explorer_view.rs`)
- [x] 2.8 (moved core 11.7) Implement header Explorer toggle hide/restore wiring and pointer-drag-only divider geometry/painting (transparent at rest, hover/drag line/grip, no keyboard stop) while retaining stored per-workspace width and usable left pane (path: `crates/made-app/src/ui.rs`)
- [x] 2.9 (moved core 11.8) Extend the end-to-end UX validation checklist with the moved UI scenarios, including wordmark, glass, picker, icon tooltip/focus, explorer action removal, hide/restore via header, hover-only divider, no manual Reload, responsive and motion checks; confirm each has an evidence row (path: `tests/e2e/ux_validation.md`)
- [x] 2.10 (moved core 2.12) Implement the explorer tree and preview panels with keyboard navigation (path: `crates/made-app/src/explorer_view.rs`)
- [ ] 2.11 Record native pointer drag/hover, keyboard focus, explorer animation smoothness, and 1280×800/720×480 visual review on native desktops (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 2.12 (moved core 2.13) Write a failing syntax-highlighting fallback test (path: `crates/made-explorer/tests/highlight.rs`)
- [x] 2.13 (moved core 2.14) Implement optional syntax highlighting with plain-text fallback (path: `crates/made-explorer/src/highlight.rs`)

## Checkpoint — Phase 2

- [ ] Verify "Quiet shell workspace", "Icon-only header controls", "Explorer hide and restore", "Explorer resize and compact layout", "Explorer header action removal", "Hover-only explorer divider", "No manual refresh control", "Quiet integration chrome"
- [ ] Verify "Text file preview", "Preview terminal roundtrip", "Syntax highlighting"
- [ ] Verify "Responsive breakpoints preserve panes and controls", "Explorer hide-show animation", "Motion does not carry essential information"
- [x] Headless/Xvfb evidence covers the implemented Linux layout, animation, preview roundtrip and compact captures; native pointer/keyboard/animation rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 3. Session map and diagnostics presentation

- [x] 3.1 (moved core 8.8) Extend optional session-map tests for selected-workspace ownership and no empty default strip; verify same/different worktree, metadata-unavailable and all focus outcomes remain available without default chrome; confirm failures before any necessary map changes (path: `crates/made-app/tests/session_map.rs`)
- [x] 3.2 (moved core 8.9) Make only optional-map ownership/presentation changes required by 3.1; retain every root session and supported activity indicator (path: `crates/made-app/src/session_map.rs`)
- [x] 3.3 Confirm the diagnostics floating card uses the documented opaque presentation, sectioned values, bounded log, copy actions and responsive clamps; cross-check the data/redaction coverage retained in core tasks 5.1–5.4 and 8.3–8.4 (path: `crates/made-app/tests/ui_model.rs`)
- [ ] 3.4 Record native diagnostics card focus, copy actions, contrast, resize and 1280×800/720×480 presentation (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 3.5 (moved core 3.12) Implement the session map UI (path: `crates/made-app/src/session_map.rs`)

## Checkpoint — Phase 3

- [ ] Verify "Session map hidden by default", "Session activity indicators", "Session map overflow and long labels"
- [ ] Verify "Shell follow diagnostics without chrome noise", "Diagnostics panel presentation"
- [x] Headless/Xvfb evidence covers the implemented Linux map/diagnostics presentation; native session-map focus and diagnostics panel rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 4. Responsive, motion and accessibility

- [x] 4.1 Extend/add headless UI tests for the 900–1099 and <900 breakpoints, compact labeled overflow, no clipped controls, explorer animation intermediate/reversal/closed-input behavior, focus rings, icon accessible names and non-color status cues (path: `crates/made-app/tests/ui_model.rs`)
- [x] 4.2 Implement/adjust focus rings, minimum target sizes, accessible metadata, responsive spacing/overflow and the documented motion while retaining the existing widget ids and terminal input invariants (path: `crates/made-app/src/theme.rs`)
- [ ] 4.3 Record a native keyboard-only walkthrough at 1280×800 and 720×480, including header actions, tabs, tree, preview return, picker and diagnostics, plus the documented contrast/colorblind spot-check (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 4.4 Record native hover/drag pointer behavior, visible focus, tooltip/accessible-name metadata and (if enabled) platform accessibility-tree behavior; do not claim screen-reader exposure while accesskit is absent (path: `wiki/records/20261026-v1-evidence.md`)

## Checkpoint — Phase 4

- [ ] Verify "Visible focus on persistent controls", "Icon actions expose accessible names", "Keyboard workspace and session navigation", "Status cues do not rely on color"
- [x] Headless/Xvfb checks cover focus, targets, overflow and non-color cues on Linux; native keyboard/pointer/screen-reader rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 5. Evidence and readiness

- [x] 5.1 Record Linux/Xvfb/headless UI evidence for 1280×800/720×480, glass/backdrop fallback, wordmark/tabs, picker, explorer animation/divider, preview and diagnostics; distinguish software from native/hardware evidence (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 5.2 Record native macOS arm64/x64, Linux X11/Wayland and Windows x64 UI evidence, including compositor/backdrop, pointer/keyboard, animation and hardware-GPU visual checks (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] 5.3 Reassess UI-contract delivery against `spec.md` after native evidence; keep unverified platforms/scenarios open rather than inferring a pass (path: `wiki/testing/platform-matrix.md`, `wiki/records/20261026-v1-evidence.md`)

## 6. Surgical visual refinement (2026-10-26)

- [x] 6.1 Update the normative contract and design reference before code: floating header with no band; selected tab treatment (later finalized in 7.1 as `BG_CARD_GLASS` + 1px `BORDER`, superseded by the 8.1 `ACCENT` border and then the 10.x fill-only treatment); unified radius-6 borderless hover/selection; borderless open-folder marking and row metrics; borderless picker selected row and quiet secondary action; hover-only floating scrollbars and terminal fade; then refresh the task coverage and UX checklist with final Linux evidence (path: `specs/changes/made-ui-ux-v1/spec.md`, `design.md`, `tasks.md`, `tests/e2e/ux_validation.md`)
- [x] 6.2 Add headless/unit coverage for the transparent header (no band fill/border/shadow/highlight), no tab underline, selected tab treatment/13px label cue (the final glass/border fill is tracked in 7.1/7.3), inactive opacity/no divider, unified hover/selection and picker secondary hover/pressed states, explorer text/icon-slot/depth metrics and no fill for an expanded-unselected folder, borderless picker secondary action, floating scroll style and the terminal scrollbar’s hidden/thumb-on-pane-hover/track-on-bar-hover/8px geometry; the assertions encode the refined contract and are green (path: `crates/made-app/tests/ui_model.rs`, `crates/made-app/src/theme.rs`, `crates/made-app/src/ui.rs`, `crates/made-app/src/terminal_view.rs`)
- [x] 6.3 Implement the floating header, unified interaction states, selected tab/icon treatment including the 13px vs 12.5px non-color label cue, explorer rows and picker actions while retaining focus rings, tooltips, accessible names, minimum targets, glass explorer card and existing layout metrics (path: `crates/made-app/src/theme.rs`, `ui.rs`, `explorer_view.rs`)
- [x] 6.4 Implement floating overlay scrollbars for explorer/picker/preview/diagnostics and the fading terminal scrollback bar; pane hover paints only the thumb, bar hover/drag may reveal a faint track, and zero allocated width/no always-on track are preserved (path: `crates/made-app/src/theme.rs`, `terminal_view.rs`)
- [x] 6.5 Run `cargo test --workspace` (608 passed, 0 failed, 2 ignored), `cargo fmt --all -- --check` (PASS), and `cargo clippy --workspace --all-targets -- -D warnings` (PASS); record the exact test count (path: `specs/changes/made-ui-ux-v1/tasks.md`)
- [x] 6.6 Perform temporary Linux/Xvfb validation captures (not committed): two workspace tabs with an expanded folder, picker/initial tree, preview, scrollbar hover, splitter, mid-animation, and terminal rest/pane/bar states. Record claim mapping and limits as Linux-only evidence in `tests/e2e/ux_validation.md`; captures are ad hoc validation artifacts and are not stored. (path: `tests/e2e/ux_validation.md`)
- [ ] 6.7 Record native pointer hover/fade, tab/icon selection, scrollbar drag, and visual review on macOS, Linux X11/Wayland and Windows (path: `wiki/records/20261026-v1-evidence.md`)

## 7. Workspace-tab glass, row focus, and picker navigation (2026-10-26)

- [x] 7.1 Update the normative contract and design reference before code: selected workspace tab uses `BG_CARD_GLASS` + 1px `BORDER` (superseded by the 8.1 `ACCENT` border and then the 10.x fill-only treatment); inactive tab uses a translucent black idle token; hover is border-only without `BG_HOVER`; close hover removes its fill, turns only the icon `ERROR`, and activates the parent tab hover; explorer/picker rows lose focus outlines while other controls keep rings; app-drawn disclosure/bullet markers share the label's optical center; picker single-click entry/current-target/Enter/double-click semantics and the deliberate focus exception are documented (superseded by the 9.x inline tree) (path: `specs/changes/made-ui-ux-v1/spec.md`, `design.md`, `tasks.md`, `tests/e2e/ux_validation.md`)
- [x] 7.2 Add failing headless/unit coverage for the glass selected tab and `BORDER`, translucent idle fill, idle-tab border-only hover, close-icon `ERROR` with parent hover, no explorer/picker row or tree-container focus outlines, app-drawn marker/text optical alignment, single-click picker navigation/path/subfolder/no-child-highlight/no-workspace-process, Enter-on-current confirmation, child selection open/enter, and the double-click guard (superseded by the 9.x inline tree; tab assertions superseded by 10.x) (path: `crates/made-app/tests/ui_model.rs`, `crates/made-app/tests/workspace_picker.rs`, local `theme`/`ui` tests)
- [x] 7.3 Implement the tab tokens/states and close interaction: selected `BG_CARD_GLASS` + `BORDER`, idle `BG_TAB_IDLE`, border-only combined pointer hover, red/icon-only close hover, while leaving session tabs and header icon actions unchanged (tab fill/stroke superseded by 10.x) (path: `crates/made-app/src/theme.rs`, `ui.rs`)
- [x] 7.4 Implement app-drawn disclosure triangles/bullets and vertically centered labels in explorer and picker rows; remove the selected-row/container focus rings while preserving focus ownership, selection fill, primary text, icon slot/text offsets, and depth indent (path: `crates/made-app/src/theme.rs`, `explorer_view.rs`, `ui.rs`)
- [x] 7.5 Implement picker single-click entry, cleared child highlight after navigation, first/last keyboard selection from none, Enter semantics, child open/enter, and the double-click guard; update the help text without starting a workspace before confirmation (superseded by the 9.x inline tree) (path: `crates/made-app/src/ui.rs`, `workspace_picker.rs`)
- [x] 7.6 Run `cargo test --workspace`, `cargo fmt --all -- --check`, and `cargo clippy --workspace --all-targets -- -D warnings`; record exact counts and results: 616 passed at that checkpoint (619 after the review follow-up under 7.13; 625 after Phase 8; 630 after the Phase 9 review fix), 0 failed, 2 ignored; fmt/clippy PASS (path: `specs/changes/made-ui-ux-v1/tasks.md`)
- [x] 7.7 Perform temporary Linux/Xvfb validation captures (not committed) at 1280×800 and 720×480: selected/idle tabs, close hover, explorer selected folder, picker states, and compact equivalents; distinguish headless from Xvfb evidence. (path: `tests/e2e/ux_validation.md`)
- [ ] 7.8 Record native tab/close hover, tree/picker focus, picker click/Enter, and visual review on macOS, Linux X11/Wayland, and Windows (path: `wiki/records/20261026-v1-evidence.md`)
- [x] 7.9 Render the picker path as a compact inset `BG_SELECTED` radius-4 chip with primary mono text and no border; keep the full-path tooltip and fit/truncate inside the toolbar at 720×480 and 1280×800; document that it carries the current/last-clicked target while child-row selection is separate (superseded by the 9.x single selected-row tree target) (path: `crates/made-app/src/ui.rs`, `theme.rs`, `crates/made-app/tests/ui_model.rs`, `spec.md`, `design.md`, `tests/e2e/ux_validation.md`)
- [x] 7.10 When the tree container gains focus with no selected row, select the first visible row using only the existing fill/primary-text cue, without opening a preview or changing pointer row activation; add the no-click focus test (path: `crates/made-app/src/explorer_view.rs`, `crates/made-app/tests/ui_model.rs`)
- [x] 7.11 Reset the picker list scroll to the top on instance/revision changes (navigation/refresh) while preserving keyboard scroll-to-selection; add a long-list single-click test (superseded by the 9.x tree/re-root tests) (path: `crates/made-app/src/ui.rs`, `crates/made-app/tests/ui_model.rs`)
- [x] 7.12 Suppress the parent workspace-tab path tooltip while the close X is hovered so the “Close workspace” tooltip is the one shown; keep the parent border/label hover (path: `crates/made-app/src/ui.rs`, `crates/made-app/tests/ui_model.rs`)
- [x] 7.13 Clean up the proposal/tasks/checklist counts and stale tab wording, then rerun tests/fmt/clippy and perform temporary 1280/720 Xvfb validation captures (not committed) (path: `specs/changes/made-ui-ux-v1/proposal.md`, `tasks.md`, `tests/e2e/ux_validation.md`)

## Checkpoint — Phase 7

- [ ] Verify "Workspace tab presentation", "Workspace tab hover and close", "Explorer folder and row presentation", "Picker selection and close", "Picker tree expansion and ancestors", "Picker keyboard tree navigation", "Picker refresh and re-root state", "Visible focus on persistent controls"
- [x] Headless/Xvfb evidence covers the Linux workspace-tab glass/idle/hover/close, tree/picker focus exception with first-row selection, app-drawn marker alignment, picker tree expansion/keyboard/double-click behavior (the later Phase 9 tree supersedes the old enter semantics), path chip, scroll/re-root reset, and close tooltip; native pointer/focus/picker rows remain pending in `wiki/records/20261026-v1-evidence.md`

## 8. Mint accent, brand spacing, and header refinement (2026-10-27)

- [x] 8.1 Update the normative contract and design reference before code: mint-green accent/focus/cursor tokens; 28px wide/20px compact wordmark-to-tab gap (historical Phase 8 value; superseded by Phase 11 right-side 24/18 breathing gaps); app-drawn `+` Open folder after the last visible tab and before overflow with reserved tab budget; right icon order Sessions (when available), Diagnostics, Explorer (rightmost); close hover/focus treatment without layout growth (historical; superseded by 14.1); splitter ~140 ms fade with discreet `BORDER_CONTROL` hover and restrained `ACCENT` drag; refresh expected/evidence rows (path: `specs/changes/made-ui-ux-v1/spec.md`, `design.md`, `tasks.md`, `tests/e2e/ux_validation.md`)
- [x] 8.2 Add failing headless/unit coverage for the mint wordmark/accent/focus/cursor tokens, status-tone distinction from the accent, 28/20 brand gap (historical; superseded by Phase 11), `+` immediately after visible tabs and before overflow with both reachable, Explorer-rightmost right-icon order, close-hover target with unchanged 24×24 hit rect and tab width (historical; superseded by 14.1), splitter fade helper/partial-opacity discreet colors/rest-none/fade-out, and no stale amber/tab-`BORDER` expectations; confirm failures before implementation where practical (path: `crates/made-app/tests/ui_model.rs`, local `theme`/`ui` tests)
- [x] 8.3 Implement the mint accent tokens and semantic success/focus/cursor updates; apply the accent to wordmark `M`, primary actions, focus rings, spinners, terminal cursor, workspace-tab selected/hover outlines, and splitter drag while keeping card borders neutral and close-X hover `ERROR` (superseded by the 10.x fill-only tab treatment) (path: `crates/made-app/src/theme.rs`, `ui.rs`, `terminal_view.rs` presentation)
- [x] 8.4 Implement the 28/20 wordmark gap (historical; superseded by Phase 11), app-drawn `+` Open folder after the last visible tab before overflow, reserved tab-fitting budget, and right icon order Sessions/Diagnostics/Explorer with Explorer rightmost; retain ids, tooltips, keyboard activation, focus, and 24px targets (path: `crates/made-app/src/theme.rs`, `ui.rs`)
- [x] 8.5 Implement the splitter fade-in/out with the documented duration, colors, opacities, hit area, cursor, tooltip, and pointer-only keyboard exception; close visibility is owned by 13.1; no layout, width, or allocated-size change (path: `crates/made-app/src/theme.rs`, `ui.rs`)
- [x] 8.6 Run `cargo test --workspace` (625 passed, 0 failed, 2 ignored), `cargo fmt --all -- --check` (PASS), and `cargo clippy --workspace --all-targets -- -D warnings` (PASS); record exact final counts/results (path: `specs/changes/made-ui-ux-v1/tasks.md`)
- [x] 8.7 Perform temporary Linux/Xvfb validation captures (not committed) at 1280×800 and 720×480: mint wordmark, selected/idle tab, `+` after tabs, Explorer rightmost, close hover, and faded splitter, with compact equivalents; record observations/limits. Do not include the Phase B picker tree. (path: `tests/e2e/ux_validation.md`)
- [x] 8.8 Sanity-check that no current-token documentation or test still expects the amber accent, neutral workspace-tab outline, right-group Open folder, or immediate splitter paint; report any remaining historical/wiki references as out of scope (path: `specs/changes/made-ui-ux-v1/`, `crates/made-app/`)

## Checkpoint — Phase 8

- [ ] Verify "Calm Editor tokens and typography", "MADE wordmark identity", "Workspace tab presentation", "Workspace tab close visibility", "Header placement and order", "Explorer splitter fade", "Hover-only explorer divider", "Motion does not carry essential information"
- [x] Headless/Xvfb evidence covers the mint accent tokens, status distinction, 28/20 gap (Phase 8; superseded by Phase 11), `+`-after-tabs ordering/overflow reachability, Explorer-rightmost order, close-hover geometry (historical; superseded by 14.1), and splitter fade/opacity/colors on Linux; native pointer/focus/compositor motion remains pending in `wiki/records/20261026-v1-evidence.md`

## 9. Inline folder-picker tree (2026-10-27)

- [x] 9.1 Update the normative contract and design reference before code: replace picker enter-and-replace navigation with an inline expandable tree rooted at the start directory; document depth-0 root rows, ancestor-preserving expansion, 14px per-depth indentation, 32px rows, disclosure markers, one selected row/path target, keyboard Right/Left/Enter/Ctrl+Enter/Backspace semantics, Up re-root reset, Refresh expansion/selection preservation, 1000-entry per-directory cap, double-click guard, and error behavior; refresh expected/evidence rows (path: `specs/changes/made-ui-ux-v1/spec.md`, `design.md`, `tasks.md`, `tests/e2e/ux_validation.md`)
- [x] 9.2 Add failing model/UI coverage for root depth-0 rows, ancestor-preserving expand/collapse, flattened selection/path target, keyboard expand/collapse/parent/Enter/Ctrl+Enter/Backspace, Up re-root state reset, Refresh expansion/valid-selection preservation, per-directory caps, unreadable-child error, click/double-click semantics, path chip following the selected row, scroll-to-selected, row geometry/depth/markers, help text, and no workspace start; confirm failures before implementation where practical (path: `crates/made-app/tests/workspace_picker.rs`, `crates/made-app/tests/ui_model.rs`)
- [x] 9.3 Implement the flattened `PickerRow` tree model, expansion state and per-directory child caches, single visible-row selection/path target, toggle/expand/collapse/confirm-row semantics, refresh preservation/pruning, Up re-root reset, and safe invalid/unreadable handling; remove enter/navigate-to-replacement paths (path: `crates/made-app/src/workspace_picker.rs`)
- [x] 9.4 Update `WorkspaceLaunch`/`AppModel` picker delegation and UI command names to toggle/expand/collapse semantics; no dead enter-the-selected-directory paths (path: `crates/made-app/src/workspace.rs`, `src/app.rs`, `src/ui.rs`)
- [x] 9.5 Implement the picker tree UI: depth indentation, disclosure triangles, 32px rows/4px inset/radius-6 states, path chip selected-row/root target, both footer actions, tree keyboard commands, scroll-to-selected, re-root scroll reset, preserved refresh state, and expanded help text (path: `crates/made-app/src/ui.rs`)
- [x] 9.6 Run `cargo test --workspace` (630 passed, 0 failed, 2 ignored), `cargo fmt --all -- --check` (PASS), and `cargo clippy --workspace --all-targets -- -D warnings` (PASS); record exact final counts/results (path: `specs/changes/made-ui-ux-v1/tasks.md`)
- [x] 9.7 Perform temporary Linux/Xvfb validation captures (not committed) at 1280×800 and 720×480: nested expanded tree, deep selected row/path chip, collapsed parent, keyboard child selection, no outline, and mint primary action; record observations/limits. (path: `tests/e2e/ux_validation.md`)
- [x] 9.8 Sanity-check that current picker docs/tests no longer claim click-to-enter/replace, separate hidden child selection, Enter-on-current, or Left-as-Up; keep Phase A green/header/close/splitter behavior intact (path: `specs/changes/made-ui-ux-v1/`, `crates/made-app/`)
- [x] 9.9 Review fix: replace centered picker scroll with minimal bottom-edge reveal that keeps the selected row plus visible ancestors, add a 720 deep-leaf keyboard test, correct the compact initial-state and keyboard validation captures, and rerun tests/fmt/clippy (path: `crates/made-app/src/ui.rs`, `crates/made-app/tests/ui_model.rs`, `tests/e2e/ux_validation.md`)

## Checkpoint — Phase 9

- [ ] Verify "Picker selection and close", "Picker tree expansion and ancestors", "Picker keyboard tree navigation", "Picker refresh and re-root state", "Picker preserves live workspace"
- [x] Headless/Xvfb evidence covers the flattened tree, ancestor-preserving expansion/collapse, depth geometry, path target/chip, keyboard tree semantics, minimal ancestor-preserving scroll, double-click guard, refresh/Up state, per-directory cap, error handling, and no-process guarantee on Linux; native pointer/focus remains pending in `wiki/records/20261026-v1-evidence.md`

## 10. Darker fallback clear and fill-only tabs (2026-10-27)

- [x] 10.1 Update the normative contract and design reference before code: fallback `BG_CLEAR` alpha 225 with native 150 unchanged; opaque `BG_TAB_SELECTED #1B231F` selected, `BG_TAB_HOVER #151C18` fill-only hover, translucent `BG_TAB_IDLE`; remove all selected/hover workspace-tab border claims; document the 13px/luminance non-color cue; update interaction/alpha/contrast tables and expected evidence rows (path: `specs/changes/made-ui-ux-v1/spec.md`, `design.md`, `tasks.md`, `tests/e2e/ux_validation.md`)
- [x] 10.2 Update/add failing token and UI coverage for fallback alpha 225/native 150, exact tab fills, selected tab `BG_TAB_SELECTED` with no stroke, idle `BG_TAB_IDLE` with no rest stroke, hover `BG_TAB_HOVER` with no stroke/primary label, selected-tab hover keeping `BG_TAB_SELECTED`, close hover applying the parent fill (including selected-tab close hover keeping `BG_TAB_SELECTED`), no `ACCENT`/`BORDER` tab outline, and the 13px selected cue; retain activation/close/focus/tooltip/overflow/plus/Explorer-rightmost/splitter/picker-tree coverage; confirm failures before implementation where practical (path: `crates/made-app/tests/ui_model.rs`, local `theme`/`ui` tests)
- [x] 10.3 Implement the darker fallback clear and fill-only tab states; keep native alpha 150, all other accent uses, tab widths, activation, close hover, focus rings, plus/header/Explorer ordering, and picker tree unchanged (path: `crates/made-app/src/theme.rs`, `ui.rs`)
- [x] 10.4 Run `cargo test --workspace` (630 passed, 0 failed, 2 ignored), `cargo fmt --all -- --check` (PASS), and `cargo clippy --workspace --all-targets -- -D warnings` (PASS); record exact final counts/results (path: `specs/changes/made-ui-ux-v1/tasks.md`)
- [x] 10.5 Perform temporary Linux/Xvfb validation captures (not committed) at 1280×800 and 720×480: selected/idle tabs, idle hover fill, close hover with parent fill/red scaled glyph, plus-after-tabs, and Explorer rightmost; attempt a light-background fallback observation or report the alpha-token limit. (path: `tests/e2e/ux_validation.md`)
- [x] 10.6 Sanity-check current docs/tests for stale selected/hover `ACCENT`/`BORDER` tab strokes or glass selected-tab claims; keep picker tree/header/green accent behavior intact (path: `specs/changes/made-ui-ux-v1/`, `crates/made-app/`)

## Checkpoint — Phase 10

- [ ] Verify "Calm Editor tokens and typography", "Native backdrop auto and fallback", "Workspace tab presentation", "Workspace tab hover and close", "Workspace tab close visibility", "Motion does not carry essential information"
- [x] Headless/Xvfb evidence covers fallback alpha 225/native 150, opaque selected/hover tab fills with no strokes, parent close hover fill, 13px/luminance cue, and unchanged plus/Explorer/splitter/picker behavior on Linux; native pointer/focus/compositor evidence remains pending in `wiki/records/20261026-v1-evidence.md`

## 11. Right-side wordmark and compact header (2026-10-27)

- [x] 11.1 Update the normative contract/design before code: workspace tabs start at the header content left edge; the wordmark sits between the tab strip and right icon group with a reserved 24px wide / 18px compact strip-side gap and an exact 24/18 wordmark-to-icons gap, no fill/hover/selection; reserve the whole right cluster in the tab-fitting budget; reduce `top_height` to 34/30 (Phase 11; superseded by Phase 12) and workspace `tab_height` to 26/24 while keeping session-map strip/tab metrics at 36/34 and 28/26; update geometry tables/diagrams, tests, and UX checklist (path: `specs/changes/made-ui-ux-v1/spec.md`, `design.md`, `tasks.md`, `tests/e2e/ux_validation.md`)
- [x] 11.2 Add/update failing headless coverage for the first tab at the header content left, wordmark/right-icon cluster position and 24/18 gaps, no wordmark tab fill/hover/selection, right-cluster reservation and no overlap/clipping at 1280/720, plus/overflow reachability, Explorer-rightmost order, `top_height` 34/30 (Phase 11; superseded by Phase 12), workspace tab height 26/24, session-map height 28/26 unchanged, card top 52/44 (Phase 11; superseded by Phase 12), and unchanged activation/focus/close targets; retain all current picker/tab-fill/accent/close-hover/splitter/scrollbar/overflow/accessibility coverage; confirm failures before implementation where practical (path: `crates/made-app/tests/ui_model.rs`, local `theme`/`ui` tests)
- [x] 11.3 Implement the layout metrics split: top strip 34/30 (Phase 11; superseded by Phase 12), workspace tabs 26/24, session-map tabs preserved at 28/26, and 24/18 wordmark breathing-gap tokens (path: `crates/made-app/src/theme.rs`, `ui.rs`)
- [x] 11.4 Implement the header layout: tabs from the content left, right-side wordmark before the icon group, right-cluster budget reservation, plus immediately after visible tabs, overflow after plus, and unchanged icon order/focus/tooltips (path: `crates/made-app/src/ui.rs`)
- [x] 11.5 Run `cargo test --workspace` (630 passed, 0 failed, 2 ignored), `cargo fmt --all -- --check` (PASS), and `cargo clippy --workspace --all-targets -- -D warnings` (PASS); record exact final counts/results (path: `specs/changes/made-ui-ux-v1/tasks.md`)
- [x] 11.6 Perform temporary Linux/Xvfb validation captures (not committed) at 1280×800 and 720×480: tabs starting left, right-side wordmark with 24/18 gaps, plus after tabs, Explorer rightmost, reduced header/card top; report observations/limits. Result: card tops 52/44 and first tab fill at 22/18 (Phase 11; superseded by Phase 12), wordmark background plain with no tab fill; Linux/Xvfb only, 720 live-resized, overflow headless-only. (path: `tests/e2e/ux_validation.md`)
- [x] 11.7 Sanity-check current docs/tests for stale pre-Phase-11 wordmark-before-tabs spacing claims or 40/36 header/28/26 workspace-tab geometry; keep fill-only tabs, green accent, picker tree, close hover, splitter fade, and plus-after-tabs intact (path: `specs/changes/made-ui-ux-v1/`, `crates/made-app/`)

## Checkpoint — Phase 11

- [ ] Verify "Calm Editor tokens and typography", "Floating header controls", "Header placement and order", "MADE wordmark identity", "Workspace tab presentation", "Explorer resize and compact layout"
- [x] Headless/Xvfb evidence covers tabs at the header left, right-side wordmark/gaps/no tab treatment, reserved right cluster, compact reachability, 34/30 and 26/24 geometry and card top 52/44 (Phase 11; superseded by Phase 12), session-map height preservation, plus/overflow and Explorer order on Linux; native pointer/focus/compositor evidence remains pending in `wiki/records/20261026-v1-evidence.md`

## 12. Header alignment and compact strip (2026-10-27)

- [x] 12.1 Update the normative contract/design before code: tabs start at the window gutter/card left edge; the header row spans gutter..width-gutter with no extra horizontal padding and the last right icon ends at the card right edge; top strip and workspace tabs are both 26/24; header top is one 8/6 pane gap below the window edge; card content starts at 42/36 with one 8/6 gap below the strip; keep 24/18 wordmark gaps, right-cluster reservation, and session-map metrics; update geometry tables/diagrams, tests, and UX checklist (path: `specs/changes/made-ui-ux-v1/spec.md`, `design.md`, `tasks.md`, `tests/e2e/ux_validation.md`)
- [x] 12.2 Add/update failing headless coverage for first tab `x == gutter`, last icon right `== width - gutter`, 26/24 strip/tab, header top `== gap`, card top 42/36, equal outer top/bottom gaps, no clipping/overlap at 1280/720, plus/overflow reachability, wordmark position/gaps/no fill, Explorer rightmost, and unchanged tab/close/focus/min-target behavior; retain picker/tree/green/splitter/fill-only coverage; confirm failures before implementation where practical (path: `crates/made-app/tests/ui_model.rs`, local `theme`/`ui` tests)
- [x] 12.3 Implement the layout metrics: `top_height == tab_height` at 26/24, no workspace-header horizontal padding, header top at `gap`, panel/content top at `2*gap + top_height` (42/36), and session-map/preview metrics unchanged (path: `crates/made-app/src/theme.rs`, `ui.rs`)
- [x] 12.4 Implement the gutter-to-gutter header row, card-edge-aligned last icon, unchanged right-cluster budget/wordmark gaps, plus-after-tabs, overflow after plus, and Explorer-rightmost order (path: `crates/made-app/src/ui.rs`)
- [x] 12.5 Run `cargo test --workspace` (630 passed, 0 failed, 2 ignored), `cargo fmt --all -- --check` (PASS), and `cargo clippy --workspace --all-targets -- -D warnings` (PASS); record exact final counts/results (path: `specs/changes/made-ui-ux-v1/tasks.md`)
- [x] 12.6 Perform temporary Linux/Xvfb validation captures (not committed) at 1280×800 and 720×480: first tab aligned to the terminal/card left, last icon aligned to the explorer/card right, 26/24 strip and 42/36 card top, wordmark before icons; report observations/limits. Result: tab fill starts at x=10/8, selected Explorer fill reaches x=1269/711 against card edge 1270/712 (1px antialias), card tops 42/36, wordmark ink 1149..1187 / 603..631 with 3 exact accent pixels; checks pass. (path: `tests/e2e/ux_validation.md`)
- [x] 12.7 Sanity-check current docs/tests for stale 34/30 strip, 52/44 card-top, or 12/10 workspace-header-padding claims; keep fill-only tabs, green accent, picker tree, close hover, splitter fade, session-map metrics, and plus-after-tabs intact (path: `specs/changes/made-ui-ux-v1/`, `crates/made-app/`)
- [x] 12.8 (superseded by 12.9) First optical attempt shifted the right cluster by the 6/5px glyph inset so the glyph aligned to the card border; the hit target and selected/hover fill then overhung the gutter. Historical result: glyph bbox right 1270/712, target right 1276/717. Temporary Xvfb validation (not committed). (path: `crates/made-app/src/theme.rs`, `ui.rs`, `crates/made-app/tests/ui_model.rs`)
- [x] 12.9 Final flush correction: restore the last 26/24 target to end exactly at `content.right`, right-align only the 14px glyph inside that target, keep the wordmark/gaps/tab reservation, and assert target right, glyph right, selected fill right, and hover fill right all equal `content.right` with no target/fill gutter overhang or clipping at 1280/720; temporary Xvfb validation (not committed). (path: `crates/made-app/src/theme.rs`, `ui.rs`, `crates/made-app/tests/ui_model.rs`)

- [x] 12.10 Restore the normal centered glyph for every right icon: keep the last 26/24 target, selected/hover fill, and focus ring ending at `content.right`; center the 14px glyph with equal optical padding (right edge inset ~6/5px); remove the right-aligned helper/inset test; assert target/fill right == `content.right`, glyph center == target center and equal padding, target inside window, no clipping/overlap at 1280/720; temporary `icon-centered-1280.png` / `icon-centered-720.png` validation (not committed) (path: `crates/made-app/src/theme.rs`, `ui.rs`, `crates/made-app/tests/ui_model.rs`)

- [x] 12.11 (historical; superseded by 12.12) Reduce the wide default explorer card width from 440 to 400 while keeping the compact default at 288: use named constants, keep 900–1099 at 35% clamped to 288–400, initialize the workbench at 400, retain the explicit 440 legacy migration plus the current 400 constructor value as responsive defaults, preserve deliberate 300/460 stored widths with normal clamping, update the 1280 geometry (left 852, column 408, card 400), and align spec/design/wiki/UX text and tests (path: `crates/made-app/src/theme.rs`, `workbench.rs`, `tests/ui_model.rs`, `specs/changes/made-ui-ux-v1/`, `tests/e2e/ux_validation.md`, `wiki/ui/layout-and-controls.md`)

- [x] 12.12 Reduce the wide explorer default from 400 to 360 while keeping the compact default at 288: update the wide constant and 288–360 mid-width clamp; treat 440, 400 and 360 as unstored defaults in migration while preserving deliberate 300/320/380/420 widths; keep the workbench on the named wide constant; update 1280 geometry to left 892, column 368, card 360 at x=910; keep 720 at 410/294/288; align spec/design/tasks/proposal/UX/wiki text and tests (path: `crates/made-app/src/theme.rs`, `workbench.rs`, `tests/ui_model.rs`, `specs/changes/made-ui-ux-v1/`, `tests/e2e/ux_validation.md`, `wiki/ui/layout-and-controls.md`)

## Checkpoint — Phase 12

- [ ] Verify "Floating header controls", "Header placement and order", "MADE wordmark identity", "Workspace tab presentation", "Explorer resize and compact layout"
- [x] Headless/Xvfb evidence covers card-edge-aligned tabs; last target, selected/hover fill, and focus ring ending at the card right border with no gutter overhang, and the 14px glyph centered with normal optical padding; no header horizontal padding, 26/24 strip/tab, card top 42/36, equal outer gaps, reserved right cluster, compact reachability, plus/overflow and Explorer order on Linux; native pointer/focus/compositor evidence remains pending in `wiki/records/20261026-v1-evidence.md`

## 13. Diagnostics previous-log presentation (2026-10-27)

Presentation-only follow-up to the core retention contract in `opencode-session-follow-v1` Phase 12. Data fields, limits, redaction, and rotation are owned there.

- [x] 13.1 Add a failing headless UI test for the Previous log row, disabled “not yet rotated” action, enabled copy after the file appears, full-path tooltip, 720×480 warning/card/footer/path/copy containment, a degraded Diagnostics state plus unavailable data state, and warning-versus-error tone coverage (path: `crates/made-app/tests/ui_model.rs`, `crates/made-app/src/ui.rs`, `crates/made-app/tests/diagnostics.rs`)
- [x] 13.2 Implement the compact Previous log path row with full-path tooltip, disabled/enabled “Copy previous log path” action using `Diagnostics::copy_previous_log_path`, quiet not-yet-rotated state, `.1` cache invalidation, and warning/error notices only for issues present while the card renders, without adding routine chrome outside the card (path: `crates/made-app/src/ui.rs`, `crates/made-app/src/diagnostics.rs`)
- [x] 13.3 Update the UI spec/design/proposal/tasks and UX checklist row; run 1280×800/720×480 headless Diagnostics checks. A temporary Xvfb capture was attempted but produced no usable frame under Xvfb without a window manager, so no screenshot evidence is claimed. (path: `specs/changes/made-ui-ux-v1/`, `tests/e2e/ux_validation.md`)

## Checkpoint — Phase 13

- [x] Headless evidence covers previous-log absent/disabled, present/copy, full-path tooltip, 720×480 containment with a warning issue, and warning/error tones on Linux
- [ ] Record native Diagnostics focus, copy-action, tooltip, and previous-log presentation on macOS, Linux X11/Wayland, and Windows (path: `wiki/records/20261026-v1-evidence.md`)
- [ ] Final-exit write/rotation failures cannot be shown in the card; stderr is the only report. This visibility gap is documented, not claimed as passing in-app feedback.

## 14. Workspace-tab close visibility (2026-10-27)

- [x] 14.1 Test-first close visibility refinement: keep the 24×24 close target and keyboard focusability reserved but hide the `×` glyph at rest for idle and selected tabs; reveal it on tab hover or keyboard focus on the tab/close; keep fixed `CLOSE_GLYPH_SIZE` 14, `ERROR` only on direct X hover, no X background fill, parent hover/selected fill/label, close tooltip suppression, and unchanged tab width/label layout; remove `CLOSE_HOVER_SCALE`, `CLOSE_HOVER_SCALE_TIME`, `close_glyph_scale`, and the per-tab scale animation; replace scale tests with rest/hover/focus/constant-size/focus-ring/close-activation coverage; update spec/design/tasks/proposal/UX/wiki and temporary Xvfb checks (path: `crates/made-app/src/ui.rs`, `theme.rs`, `crates/made-app/tests/ui_model.rs`, `specs/changes/made-ui-ux-v1/`, `tests/e2e/ux_validation.md`, `wiki/ui/`, `wiki/records/20261026-v1-evidence.md`)

## Checkpoint — Phase 14

- [ ] Verify "Workspace tab close visibility" and "Motion does not carry essential information"
- [x] Headless/Xvfb evidence covers hidden-at-rest close glyphs for idle/selected tabs, hover/focus reveal, fixed 14px size, 24×24 target/focus ring, ERROR/no-fill X hover, unchanged tab/label layout, and close activation on Linux; native pointer/focus/compositor evidence remains pending in `wiki/records/20261026-v1-evidence.md`


## Coverage summary

| Requirement | Task coverage | Checkpoint |
|---|---|---|
| Calm Editor Visual System | 1.1–1.2, 1.7–1.8, 4.3, 6.1–6.6, 7.1–7.7; 8.1–8.8; 10.1–10.6; 11.1–11.7; 12.1–12.12; 14.1; retained theme tests | Phase 1, Phase 4, Phase 6, Phase 7, Phase 8, Phase 10, Phase 11, Phase 12, Phase 14 |
| Minimal Workspace Layout | 2.1, 2.3–2.11; 1.3–1.4; 4.1–4.2; 6.1–6.6; 7.1–7.13; 8.1–8.8; 11.1–11.7; 12.1–12.12 | Phase 1, Phase 2, Phase 4, Phase 6, Phase 7, Phase 8, Phase 11, Phase 12 |
| Brand and Workspace Tabs | 1.4, 1.7, 2.3; retained tab tests; 6.1–6.6; 7.1–7.13; 8.1–8.8; 10.1–10.6; 11.1–11.7; 12.1–12.12; 14.1 | Phase 1, Phase 2, Phase 6, Phase 7, Phase 8, Phase 10, Phase 11, Phase 12, Phase 14 |
| Optional Session Map Presentation | 3.1–3.2, 3.5; core data/focus tasks 3.11, 3.13–3.14; 6.1–6.6; 11.1–11.7 (preserved session-tab metrics); 12.1–12.12 (metrics unchanged) | Phase 3, Phase 6, Phase 11, Phase 12 |
| Explicit Folder Picker | 1.5–1.7; core launch tasks 10.1–10.6; 6.1–6.6; 7.1–7.13; 9.1–9.9 | Phase 1, Phase 6, Phase 7, Phase 9 |
| Full-Left Preview Presentation | 2.2–2.4, 2.10, 2.12–2.13; core preview/safety tasks 2.7–2.11, 2.15–2.16; 6.4 | Phase 2, Phase 6 |
| Diagnostics Presentation | 3.3–3.4; 13.1–13.3; core data/redaction tasks 5.1–5.4, 8.3–8.4, 12.1–12.7; 6.4 | Phase 3, Phase 6, Phase 13 |
| Responsive and Motion | 2.5, 2.8, 4.1–4.2; 6.1–6.6; 7.6–7.7; 8.1–8.8; 11.1–11.7; 12.1–12.12; 14.1 | Phase 2, Phase 4, Phase 6, Phase 7, Phase 8, Phase 11, Phase 12, Phase 14 |
| Keyboard Focus and Accessibility | 1.3, 4.1–4.4; 6.2–6.3; 7.1–7.13; 8.1–8.8; 9.1–9.9; 14.1 | Phase 1, Phase 4, Phase 6, Phase 7, Phase 8, Phase 9, Phase 14 |

## Scenario disposition from the core change

Every UI scenario removed from the core spec is retained here under the same title unless noted.

| Core requirement/scenario | Disposition |
|---|---|
| Minimal Workspace Layout — Quiet shell workspace | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Icon-only header controls | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Explorer hide and restore | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Explorer resize and compact layout | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Explorer header action removal | Moved to Minimal Workspace Layout |
| Minimal Workspace Layout — Hover-only explorer divider | Moved to Minimal Workspace Layout |
| Session Map — Session activity indicators | Moved to Optional Session Map Presentation |
| Read-Only File Preview — Text file preview | Moved to Full-Left Preview Presentation |
| Read-Only File Preview — Syntax highlighting | Moved to Full-Left Preview Presentation |
| Read-Only File Preview — Preview terminal roundtrip | Moved to Full-Left Preview Presentation; the hidden-input/process-survival clause is retained in core as "Preview keeps terminal process and input isolated" |
| Explorer Live Updates — No manual refresh control | Moved to Minimal Workspace Layout |
| Diagnostics — Shell follow diagnostics without chrome noise | Moved to Diagnostics Presentation |
| Read-Only File Preview — (no exact predecessor) | Core retains safety/process scenario "Preview keeps terminal process and input isolated" |
| Diagnostics — (no exact predecessor) | Core retains data scenario "Shell-only diagnostics" |

Derived UI scenarios make previously implicit presentation guarantees explicit: Calm Editor tokens/contrast/glass/backdrop, wordmark/tab/overflow, session-map hidden/overflow, picker selection/keyboard/preserve-live, diagnostics card layout, responsive breakpoints, motion, focus/accessibility, and quiet integration chrome.

## Analyze — UI/core split (current)

**Readiness assessment:** PASS for the split contract, the workspace-tab close visibility refinement, the 2026-10-26 Linux visual polish and workspace-tab/row-focus/picker refinement, the 2026-10-27 mint-accent/header refinement, the 2026-10-27 inline picker tree, and the 2026-10-27 fallback-clear/fill-only-tab correction, the right-side-wordmark/compact-header refinement, and the header card-edge/26/24-strip alignment refinement; delivery remains Linux-only with native/manual UI evidence pending.

| Analyze gate | Result | Basis |
|---|---|---|
| Scope and live-contract consistency | PASS | Both changes remain active; core retains product/runtime/integration/security/performance; UI owns visual/presentation; no version/archive |
| Outcome versus implementation separation | PASS | Normative UI outcomes in `spec.md`; tokens/layout/components/motion/evidence rationale in non-normative `design.md` |
| Scenario traceability and no loss | PASS | 97 original scenarios accounted for: 85 retained in core, 12 moved here, plus 2 derived core and 33 derived UI scenarios (45 UI scenarios / 9 requirements); disposition table above |
| Unresolved clarification markers | PASS | No unresolved clarification markers, TODO, or unresolved placeholder in proposal/spec/tasks/design |
| Duplicated normative contract | PASS | UI layout/chrome/presentation removed from core requirements and replaced with cross-references; core safety/data/security guarantees retained once |
| Dependency/task ownership | PASS | UI tasks own `ui.rs`, `theme.rs`, `explorer_view.rs`, `terminal_view.rs` presentation, `session_map.rs` presentation, picker UI and UI evidence; core tasks own runtime/integration/security/preview safety/diagnostics data |
| Historical completion integrity | PASS | Moved checked items retain their original checked state and Linux evidence; no historical item was rewritten as checked without evidence; new 6.x, 7.x, 8.x, 9.x, 10.x, 11.x, 12.x, 13.x and 14.x items carry fresh Linux-only checks |
| Honest delivery status | PASS — Linux UI evidence; native pending | 654 tests, headless/Xvfb checks and evidence rows cover the Linux UI including the 360px wide explorer default, card-edge-aligned 26/24 floating header (42/36 card top) with the last visible icon glyph on the card right border, mint accent/focus/cursor tokens, right-side wordmark with 24/18 breathing gaps and reserved right cluster, `+`-after-tabs header placement with reserved overflow budget, Explorer-rightmost order, opaque `BG_TAB_SELECTED`/`BG_TAB_HOVER` workspace tabs with no tab outline, translucent `BG_TAB_IDLE`, fill-only hover, and red/no-fill hover/focus-only close with the close-specific tooltip, plus fallback clear alpha 225/native 150, tree/picker focus exception with first-row selection on focus, app-drawn marker alignment, the inline picker tree with ancestor-preserving expansion, depth indentation, path target/chip, keyboard tree navigation, minimal ancestor-preserving scroll, refresh/Up state, double-click guard, per-directory cap and unreadable-child error, top-reset re-rooting, borderless picker secondary action, explorer scrollbar rest/hover, faded explorer splitter hover/drag, a mid-hide animation check, preview, terminal scrollbar checks, and the bounded-log Previous log row; native backdrop/compositor, native pointer/keyboard, native scrollbar drag, native animation and hardware-GPU remain unchecked. Picker tree behavior, path chip, focus-ring, scroll/re-root, refresh and cap assertions are headless plus 1280/720 Xvfb checks; close visibility is headless UI-tested, splitter-fade timing/opacity are token/helper-tested, and the splitter opacity is sampled from Xvfb pixels |
| Cross-spec coherence | PASS | Core spec/tasks/design link to this change; this design links back; wiki/architecture.md and this change's design updated where required |

Notes: The split intentionally adds derived UI scenarios for presentation behavior already implemented/discussed; it adds no new product scope. The 2026-10-26 polish adds five derived scenarios (floating header, unified hover/selection, explorer folder/row presentation, hover-only scrollbars, fade layout stability). The second 2026-10-26 refinement adds two derived scenarios (workspace tab hover/close and picker single-click navigation) and updates existing wording for the glass selected tab, translucent idle tab, border-only hover, close hover, tree/picker focus exception, marker alignment, and Enter/double-click semantics without adding runtime scope. The independent review follow-up adds the picker path chip, first-row tree selection on focus, picker scroll reset, close-tooltip suppression, and documentation cleanup; it adds three headless tests and no runtime scope. The 2026-10-27 header refinement adds three derived scenarios (header placement and order, workspace tab close hover, explorer splitter fade), replaces the amber accent with mint green, adds the app-drawn `+` after tabs and Explorer-rightmost order, and adds close-hover visibility/splitter-fade motion without changing runtime or picker-tree scope. The 2026-10-27 picker-tree refinement replaces the old single-click enter-and-replace scenario and keyboard scenario with tree expansion/keyboard scenarios and adds the refresh/re-root scenario, for a net one-scenario increase (44 UI scenarios / 9 requirements); it changes picker model/UI behavior only, keeps runtime/bridge/security/terminal/explorer semantics, and does not persist tree state. The final Phase 9 review fix replaces centered scroll-to-selected with minimal bottom-edge reveal that keeps the selected row plus visible ancestors, adds one 720 deep-leaf keyboard test (630 total), corrects the compact initial-state capture, and recaptures the compact keyboard state. The 2026-10-27 tab-fill correction darkens the fallback clear to alpha 225, replaces the selected glass/accent-border tab with opaque `BG_TAB_SELECTED`, makes hover a fill-only `BG_TAB_HOVER`, and keeps native clear alpha 150 and all other accent uses; it updates existing token/UI tests and captures without adding UI scenarios. The final 2026-10-27 refinement moves tabs to the header left edge, moves the wordmark before the right icon group with 24/18 breathing gaps and no tab treatment, reserves the whole right cluster in the tab-fitting budget, and reduces the top strip/workspace tabs to 34/30 and 26/24 while keeping session-map metrics at 36/34 and 28/26; it changes geometry and existing headless coverage without adding UI scenarios. The Phase 12 default-width refinement first reduced the wide explorer default from 440 to 400 (historical), then reduced it again to 360, keeps the 900–1099 default at 35% clamped to 288–360 and the compact default at 288, and resolves the legacy 440, interim 400 and current 360 constructor values to the responsive default while preserving deliberate dragged widths. The Phase 12 alignment refinement then removes the workspace-header horizontal inset so tabs start at the gutter/card left edge, reduces the strip and tabs to a common 26/24 with header top at the 8/6 pane gap, and moves the card top to 42/36 with equal outer gaps. Its final correction restores normal centered glyph painting for every right icon: the last 26/24 target, selected/hover fill, and focus ring end at `content.right` with no gutter overhang, while the 14px glyph keeps equal optical padding on both sides. It removes the temporary right-aligned helper/test and changes geometry/coverage without adding UI scenarios. Phase 14 then hides the workspace-tab close × at rest, reveals it on tab hover or keyboard focus without any scale animation, keeps the 24×24 target/focusability reserved, and removes the old scale tokens/state/tests without adding UI scenarios. `accesskit` remains disabled, so platform screen-reader exposure is not claimed. Native UI evidence must be recorded before any archive request for either change.

## Analyze — Diagnostics previous-log presentation (2026-10-27)

**Readiness assessment:** PASS for implementation planning and Linux automated presentation; native Diagnostics evidence remains pending.

| Analyze gate | Result | Basis |
|---|---|---|
| Scope and live-contract consistency | PASS | Presentation-only addition inside the existing Diagnostics card; retention data/limits/rotation remain in the core change; no new requirement or routine chrome |
| Outcome versus implementation separation | PASS | Spec states the row/copy/disabled/tone outcomes; exact row placement, truncation, and action primitive remain in design/tasks |
| Scenario/test-first coverage | PASS | One added Diagnostics Presentation scenario maps to 13.1–13.3; core scenarios 12.1–12.7 cover data, bounds, and rotation |
| Historical completion integrity | PASS | Prior checked tasks/checkpoints are unchanged; Phase 13 is new and checked only for Linux automated evidence |
| Security and redaction | PASS | The row displays the already-sanitized `.1` path; copying uses `Diagnostics::copy_previous_log_path`; 64-hex runs in issue notices are redacted; no new filesystem access from the UI |
| Responsive and accessibility | PASS | 720×480 headless card/footer/path/copy containment, degraded status rendering, truncated label with full-path tooltip, stable action id, disabled state, and warning/error semantic tones are covered; native focus/tooltip remains pending |
| Failure visibility | PASS with documented gap | Only issues present while the card renders (notably the startup legacy warning) can be shown; final-exit write/rotation failures are stderr-only because the window closes first |
| Delivery/verification readiness | PASS in Linux scope / native pending | Headless tests and isolated workspace verification pass; native macOS/Linux desktop/Windows Diagnostics evidence remains open |

| New/modified scenario (exact spec title) | Test-first tasks → implementation | Evidence checkpoint |
|---|---|---|
| Previous log path and retention notice | 13.1 → 13.2, 13.3 | 13 |

The Phase 13 addition brings the UI contract to 45 scenarios across 9 requirements (33 derived UI scenarios). No runtime or data behavior is added here; the core change remains authoritative for retention.

## Implementation handoff and ownership

Spec ref: `specs/changes/made-ui-ux-v1/`

| Bounded implementation scope | Owned files | Dependencies |
|---|---|---|
| Theme, tokens, glass, native backdrop fallback, focus/accessible primitives | `crates/made-app/src/theme.rs`, `crates/made-platform/src/native_backdrop.rs`, `crates/made-app/tests/ui_model.rs` | Core runtime/workspace state; no OS branch outside `made-platform` |
| Header, wordmark, workspace tabs/overflow, picker composition and layout/motion | `crates/made-app/src/ui.rs`, `crates/made-app/src/workspace_picker.rs`, `crates/made-app/tests/ui_model.rs`, `crates/made-app/tests/workspace_picker.rs` | Core launch semantics and workspace ownership; serialized `ui.rs`/`theme.rs` writes |
| Explorer presentation, full-left preview presentation | `crates/made-app/src/explorer_view.rs`, `crates/made-explorer/src/highlight.rs`, `crates/made-explorer/tests/highlight.rs`, `crates/made-app/tests/read_only_ui.rs`, `crates/made-app/tests/ui_model.rs` | Core explorer/preview safety, root generation, and terminal visibility state; `read_only_ui.rs` is shared with core task 2.11, so writes are serialized |
| Terminal grid presentation and fading scrollback bar | `crates/made-app/src/terminal_view.rs` | Core terminal input/IME/selection/PTY semantics must not change |
| Session map and diagnostics presentation | `crates/made-app/src/session_map.rs`, `crates/made-app/src/diagnostics.rs`, `crates/made-app/tests/session_map.rs`, `crates/made-app/tests/ui_model.rs` | Core session-map data/focus and diagnostics data/redaction; `session_map.rs` tests are shared with core task 3.11, so writes are serialized |
| Evidence and platform UI validation | `tests/e2e/ux_validation.md`, `wiki/records/20261026-v1-evidence.md`, `wiki/testing/platform-matrix.md` | Phases 1–4 and 6–9, and available native platforms; report unavailable evidence as pending |

Do not change terminal input/IME/selection behavior, preview write guards, root arbitration, bridge/security, or runtime ownership as part of UI work. If a UI change requires a runtime contract change, return to the core change first. Do not mark native/manual checkpoints complete from Xvfb/headless or compile-only evidence.
